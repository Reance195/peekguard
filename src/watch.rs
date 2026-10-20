//! Watch state machine: bind a target window, follow the cursor, hide/restore.
#![allow(dead_code)]

use crate::config::{Config, HideMode};
use crate::plat::hide::Hidden;
use crate::plat::*;

/// The platform operations the watch state machine needs.
///
/// Keeping them behind a trait lets the state machine run against a fake in
/// unit tests. Without it the only coverage was the end-to-end Python suite,
/// which needs a real desktop session and cannot aim at one transition.
pub trait Env {
    fn is_valid(&self, hwnd: HWND) -> bool;
    fn process_id(&self, hwnd: HWND) -> u32;
    fn process_name(&self, pid: u32) -> String;
    fn class_name(&self, hwnd: HWND) -> String;
    fn window_title(&self, hwnd: HWND) -> String;
    fn window_rect(&self, hwnd: HWND) -> RECT;
    fn cursor_pos(&self) -> POINT;
    fn is_minimized(&self, hwnd: HWND) -> bool;
    fn foreground_window(&self) -> HWND;
    fn hide(&self, hwnd: HWND, mode: HideMode, alpha: u8) -> Option<Hidden>;
}

/// The real implementation, forwarding to the active platform layer.
pub struct PlatformEnv;

impl Env for PlatformEnv {
    fn is_valid(&self, hwnd: HWND) -> bool {
        crate::plat::is_valid(hwnd)
    }
    fn process_id(&self, hwnd: HWND) -> u32 {
        crate::plat::process_id(hwnd)
    }
    fn process_name(&self, pid: u32) -> String {
        crate::plat::process_name(pid)
    }
    fn class_name(&self, hwnd: HWND) -> String {
        crate::plat::class_name(hwnd)
    }
    fn window_title(&self, hwnd: HWND) -> String {
        crate::plat::window_title(hwnd)
    }
    fn window_rect(&self, hwnd: HWND) -> RECT {
        crate::plat::window_rect(hwnd)
    }
    fn cursor_pos(&self) -> POINT {
        crate::plat::cursor_pos()
    }
    fn is_minimized(&self, hwnd: HWND) -> bool {
        crate::plat::is_minimized(hwnd)
    }
    fn foreground_window(&self) -> HWND {
        crate::plat::foreground_window()
    }
    fn hide(&self, hwnd: HWND, mode: HideMode, alpha: u8) -> Option<Hidden> {
        crate::plat::hide::hide(hwnd, mode, alpha)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    /// Nothing bound.
    Idle,
    /// Bound and visible; cursor is inside (or window isn't eligible yet).
    Armed,
    /// Cursor has left and the delay is ticking down.
    Pending { since: u64 },
    /// Hidden by us.
    Hidden,
    /// Hidden and latched: only an explicit restore brings it back.
    Latched,
}

impl State {
    pub fn is_hidden(&self) -> bool {
        matches!(self, State::Hidden | State::Latched)
    }
    pub fn label(&self) -> &'static str {
        match self {
            State::Idle => "未绑定",
            State::Armed => "守护中",
            State::Pending { .. } => "即将隐身",
            State::Hidden => "已隐身",
            State::Latched => "已隐身(锁定)",
        }
    }
}

/// What the watch loop decided to do this tick. The caller performs the work so
/// that I/O and state stay in one place.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    None,
    Hide,
    Restore,
    /// Target vanished or became unusable.
    Drop,
    /// We tried to hide and the window refused (elevated target, exotic window).
    /// The binding is dropped in response, so we stop retrying forever.
    Refused,
}

#[derive(Clone, Debug)]
pub struct Target {
    pub hwnd: HWND,
    pub process: String,
    pub class: String,
    pub title: String,
    /// Rect captured when we hid; the "come back here" zone.
    pub home_rect: RECT,
}

impl Target {
    pub fn capture(env: &dyn Env, hwnd: HWND) -> Option<Target> {
        if !env.is_valid(hwnd) {
            return None;
        }
        let pid = env.process_id(hwnd);
        Some(Target {
            hwnd,
            process: env.process_name(pid),
            class: env.class_name(hwnd),
            title: env.window_title(hwnd),
            home_rect: env.window_rect(hwnd),
        })
    }

    pub fn describe(&self) -> String {
        let t = if self.title.is_empty() {
            "(无标题)".to_string()
        } else {
            truncate(&self.title, 28)
        };
        format!("{} — {}", self.process, t)
    }
}

fn truncate(s: &str, max_chars: usize) -> String {
    let mut out: String = s.chars().take(max_chars).collect();
    if s.chars().count() > max_chars {
        out.push('…');
    }
    out
}

pub struct Watcher {
    pub state: State,
    pub target: Option<Target>,
    pub hidden: Option<Hidden>,
    pub cfg: Config,
    env: Box<dyn Env>,
    /// Cursor position the last time we evaluated.
    last_cursor: POINT,
    /// Last tick on which the cursor was confirmed inside the guard zone. The
    /// debounce countdown starts from here rather than from the tick that first
    /// noticed the cursor had left, which would otherwise throw away up to one
    /// whole poll interval of the configured delay.
    last_inside_ms: Option<u64>,
    /// Set when we hid because the user asked explicitly (hotkey/tray) rather than the mouse leaving.
    manual: bool,
    /// Set by a restore while the cursor is away from the window: auto-hide is
    /// suspended until the cursor has been inside once. Without it, restoring by
    /// hotkey with the cursor parked elsewhere would hide the window again within
    /// a tick or two and the hotkey would look broken.
    awaiting_entry: bool,
}

impl Watcher {
    pub fn new(cfg: Config) -> Watcher {
        Watcher::with_env(cfg, Box::new(PlatformEnv))
    }

    /// Build a watcher against a custom environment. Tests use this to drive
    /// the state machine without touching a real desktop.
    pub(crate) fn with_env(cfg: Config, env: Box<dyn Env>) -> Watcher {
        Watcher {
            state: State::Idle,
            target: None,
            hidden: None,
            cfg,
            env,
            last_cursor: POINT::default(),
            last_inside_ms: None,
            manual: false,
            awaiting_entry: false,
        }
    }

    pub fn bind(&mut self, hwnd: HWND) -> bool {
        self.unbind();
        match Target::capture(self.env.as_ref(), hwnd) {
            Some(t) => {
                self.target = Some(t);
                self.state = State::Armed;
                self.manual = false;
                self.awaiting_entry = false;
                self.last_inside_ms = None;
                true
            }
            None => false,
        }
    }

    pub fn unbind(&mut self) {
        self.restore();
        self.target = None;
        self.state = State::Idle;
        self.manual = false;
        self.awaiting_entry = false;
        self.last_inside_ms = None;
    }

    /// Restore the window if we are holding it hidden.
    pub fn restore(&mut self) {
        if let Some(h) = self.hidden.take() {
            h.revert();
        }
        if self.state.is_hidden() {
            self.state = State::Armed;
        }
        self.manual = false;
        // Auto-hide stays suspended until the cursor comes back inside once;
        // see the field docs. The auto-restore path (cursor already inside)
        // clears this on the very next tick.
        self.awaiting_entry = true;
    }

    /// Force-hide (hotkey/tray). Only valid with a live target.
    pub fn hide_now(&mut self) -> bool {
        let Some(t) = &self.target else { return false };
        if self.hidden.is_some() {
            return true;
        }
        match self.env.hide(t.hwnd, self.cfg.mode, self.cfg.alpha) {
            Some(h) => {
                self.hidden = Some(h);
                self.state = if self.cfg.latch {
                    State::Latched
                } else {
                    State::Hidden
                };
                self.manual = true;
                self.awaiting_entry = false;
                true
            }
            None => false,
        }
    }

    /// Toggle: hidden -> restore, visible -> hide.
    pub fn toggle(&mut self) -> bool {
        if self.hidden.is_some() {
            self.restore();
            false
        } else {
            self.hide_now()
        }
    }

    /// Whether the cursor counts as "inside" the guard zone right now.
    fn cursor_inside(&self, p: POINT, hidden: bool) -> bool {
        let Some(t) = &self.target else { return false };
        let margin = self.cfg.margin_px;
        if hidden {
            // Coming back: use the rect the window had when we hid it. The window
            // itself may have been moved off-screen, so this is the only reliable zone.
            t.home_rect.contains_with_margin(p.x, p.y, margin)
        } else {
            let r = self.env.window_rect(t.hwnd);
            let r = if r.is_empty() { t.home_rect } else { r };
            r.contains_with_margin(p.x, p.y, margin)
        }
    }

    /// One evaluation tick. `now_ms` is a monotonic millisecond clock.
    pub fn tick(&mut self, now_ms: u64) -> Action {
        let Some(hwnd) = self.target.as_ref().map(|t| t.hwnd) else {
            self.state = State::Idle;
            return Action::None;
        };

        if !self.env.is_valid(hwnd) {
            self.restore();
            self.target = None;
            self.state = State::Idle;
            return Action::Drop;
        }

        let cursor = self.env.cursor_pos();
        self.last_cursor = cursor;
        let hidden = self.hidden.is_some();

        // Latched: nothing but an explicit call gets it back. If the latch was
        // lifted while the window stayed hidden, fall through to the normal
        // hidden logic so returning to the window brings it back again.
        if self.state == State::Latched {
            if self.cfg.latch {
                return Action::None;
            }
            self.state = State::Hidden;
        }

        if hidden {
            let inside = self.cursor_inside(cursor, true);
            if self.manual {
                // The user asked for this hide explicitly, and the cursor is still
                // sitting on the window. Honour it until the mouse actually leaves;
                // only then does the mouse policy take over again. Without this the
                // very next tick would restore it and the hotkey would look broken.
                if inside {
                    return Action::None;
                }
                self.manual = false;
                return Action::None;
            }
            // Auto-hidden: coming back to the old spot brings the window back.
            if inside {
                self.restore();
                return Action::Restore;
            }
            return Action::None;
        }

        // Not hidden.
        //
        // A minimized window is skipped unconditionally, whatever `skip_minimized`
        // says. `hide()` cannot act on one (there is nothing on screen to hide), and
        // because that also comes back as `None` it must never be mistaken for a
        // window *refusing* to be hidden — that path suppresses the window and tells
        // the user about admin rights, which would be both wrong and sticky. Stay
        // `Armed` and keep the binding, so the moment the window is restored it gets
        // protected as normal.
        if self.env.is_minimized(hwnd) {
            self.state = State::Armed;
            return Action::None;
        }
        if self.cfg.hide_only_when_focused {
            let fg = self.env.foreground_window();
            if fg != hwnd {
                self.state = State::Armed;
                return Action::None;
            }
        }

        let inside = self.cursor_inside(cursor, false);
        if inside {
            self.awaiting_entry = false;
            self.last_inside_ms = Some(now_ms);
        }
        match self.state {
            State::Armed => {
                // `awaiting_entry` suspends the countdown after a restore with the
                // cursor away: the window must get one visit before leaving can
                // hide it again.
                if !inside && !self.awaiting_entry {
                    // Start the countdown from the last tick the cursor really was
                    // inside, not from this one. Using now_ms here would throw away
                    // up to a whole poll interval before the configured delay even
                    // begins, which is most of why hiding felt sluggish.
                    //
                    // Only trust a timestamp from within the last `delay_ms`: if the
                    // cursor was last seen inside a long time ago the window sat in
                    // Armed without ever hiding (minimized, or not focused, with
                    // hide_only_when_focused), and counting from back then would hide
                    // it the instant it became eligible.
                    let since = match self.last_inside_ms {
                        Some(t) if now_ms.saturating_sub(t) <= self.cfg.delay_ms => t,
                        _ => now_ms,
                    };
                    self.state = State::Pending { since };
                }
                Action::None
            }
            State::Pending { since } => {
                if inside {
                    self.state = State::Armed;
                    return Action::None;
                }
                if now_ms.saturating_sub(since) >= self.cfg.delay_ms {
                    match self.env.hide(hwnd, self.cfg.mode, self.cfg.alpha) {
                        Some(h) => {
                            self.hidden = Some(h);
                            self.state = if self.cfg.latch {
                                State::Latched
                            } else {
                                State::Hidden
                            };
                            Action::Hide
                        }
                        None => {
                            // The window refuses to be hidden (elevated target, an
                            // exotic shell surface). Retrying every debounce would
                            // spin forever and never work, so stop guarding it and
                            // tell the user once.
                            self.target = None;
                            self.state = State::Idle;
                            Action::Refused
                        }
                    }
                } else {
                    Action::None
                }
            }
            _ => Action::None,
        }
    }

    /// Update a bound window's recorded identity (title changes over time).
    pub fn refresh_identity(&mut self) {
        if let Some(t) = &mut self.target {
            if self.env.is_valid(t.hwnd) {
                t.title = self.env.window_title(t.hwnd);
                if self.hidden.is_none() {
                    let r = self.env.window_rect(t.hwnd);
                    if !r.is_empty() {
                        t.home_rect = r;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    const TARGET: usize = 0x2a;

    fn target_hwnd() -> HWND {
        TARGET as HWND
    }

    fn other_hwnd() -> HWND {
        (TARGET + 1) as HWND
    }

    fn rect(l: i32, t: i32, r: i32, b: i32) -> RECT {
        RECT {
            left: l,
            top: t,
            right: r,
            bottom: b,
        }
    }

    fn point(x: i32, y: i32) -> POINT {
        POINT { x, y }
    }

    struct Inner {
        valid: Cell<bool>,
        minimized: Cell<bool>,
        focused: Cell<bool>,
        hide_ok: Cell<bool>,
        rect: Cell<RECT>,
        cursor: Cell<POINT>,
        title: RefCell<String>,
        hide_calls: Cell<u32>,
    }

    #[derive(Clone)]
    struct FakeEnv(Rc<Inner>);

    impl FakeEnv {
        fn new() -> FakeEnv {
            FakeEnv(Rc::new(Inner {
                valid: Cell::new(true),
                minimized: Cell::new(false),
                focused: Cell::new(true),
                hide_ok: Cell::new(true),
                rect: Cell::new(rect(0, 0, 100, 100)),
                cursor: Cell::new(point(50, 50)),
                title: RefCell::new("Fake".into()),
                hide_calls: Cell::new(0),
            }))
        }

        fn watcher(&self) -> Watcher {
            self.watcher_with(test_cfg())
        }

        fn watcher_with(&self, cfg: Config) -> Watcher {
            Watcher::with_env(cfg, Box::new(self.clone()))
        }

        fn cursor(&self, x: i32, y: i32) {
            self.0.cursor.set(point(x, y));
        }

        fn window_rect(&self, r: RECT) {
            self.0.rect.set(r);
        }

        fn title(&self, s: &str) {
            *self.0.title.borrow_mut() = s.to_string();
        }

        fn hide_ok(&self, ok: bool) {
            self.0.hide_ok.set(ok);
        }

        fn hidden_calls(&self) -> u32 {
            self.0.hide_calls.get()
        }
    }

    impl Env for FakeEnv {
        fn is_valid(&self, _: HWND) -> bool {
            self.0.valid.get()
        }
        fn process_id(&self, _: HWND) -> u32 {
            1234
        }
        fn process_name(&self, _: u32) -> String {
            "fake.exe".into()
        }
        fn class_name(&self, _: HWND) -> String {
            "FakeWindow".into()
        }
        fn window_title(&self, _: HWND) -> String {
            self.0.title.borrow().clone()
        }
        fn window_rect(&self, _: HWND) -> RECT {
            self.0.rect.get()
        }
        fn cursor_pos(&self) -> POINT {
            self.0.cursor.get()
        }
        fn is_minimized(&self, _: HWND) -> bool {
            self.0.minimized.get()
        }
        fn foreground_window(&self) -> HWND {
            if self.0.focused.get() {
                target_hwnd()
            } else {
                other_hwnd()
            }
        }
        fn hide(&self, _: HWND, _: HideMode, _: u8) -> Option<Hidden> {
            self.0.hide_calls.set(self.0.hide_calls.get() + 1);
            if self.0.hide_ok.get() {
                Some(fake_hidden())
            } else {
                None
            }
        }
    }

    /// A `Hidden` that is safe to revert in tests: window 0 is invalid on every
    /// platform, so `revert()` returns immediately.
    #[cfg(windows)]
    fn fake_hidden() -> Hidden {
        Hidden {
            snap: crate::plat::hide::Snapshot {
                hwnd: 0 as HWND,
                rect: RECT::default(),
                alpha: None,
                was_layered: false,
                ex_style: 0,
                was_minimized: false,
            },
            mode: HideMode::Hide,
        }
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn fake_hidden() -> Hidden {
        Hidden {
            snap: crate::plat::hide::Snapshot {
                hwnd: 0 as HWND,
                rect: RECT::default(),
                opacity: None,
            },
            mode: HideMode::Hide,
        }
    }

    fn test_cfg() -> Config {
        Config {
            delay_ms: 100,
            margin_px: 0,
            check_update: false,
            ..Config::default()
        }
    }

    #[test]
    fn bind_captures_identity_and_home_rect() {
        let env = FakeEnv::new();
        let mut w = env.watcher();

        assert!(w.bind(target_hwnd()));
        assert_eq!(w.state, State::Armed);
        let t = w.target.as_ref().unwrap();
        assert_eq!(t.process, "fake.exe");
        assert_eq!(t.class, "FakeWindow");
        assert_eq!(t.title, "Fake");
        assert_eq!(t.home_rect, rect(0, 0, 100, 100));
    }

    #[test]
    fn bind_rejects_invalid_window() {
        let env = FakeEnv::new();
        env.0.valid.set(false);
        let mut w = env.watcher();

        assert!(!w.bind(target_hwnd()));
        assert_eq!(w.state, State::Idle);
        assert!(w.target.is_none());
    }

    #[test]
    fn stays_armed_while_cursor_is_inside() {
        let env = FakeEnv::new();
        let mut w = env.watcher();
        w.bind(target_hwnd());

        for now in [0, 16, 90, 5_000] {
            assert_eq!(w.tick(now), Action::None);
            assert_eq!(w.state, State::Armed);
        }
        assert_eq!(env.hidden_calls(), 0);
    }

    #[test]
    fn hides_after_delay_counted_from_last_inside_tick() {
        let env = FakeEnv::new();
        let mut w = env.watcher();
        w.bind(target_hwnd());
        w.tick(0);

        env.cursor(500, 500);
        assert_eq!(w.tick(50), Action::None);
        assert_eq!(w.state, State::Pending { since: 0 });
        assert_eq!(w.tick(99), Action::None);
        assert_eq!(env.hidden_calls(), 0);

        assert_eq!(w.tick(100), Action::Hide);
        assert_eq!(w.state, State::Hidden);
        assert!(w.hidden.is_some());
        assert_eq!(env.hidden_calls(), 1);
    }

    #[test]
    fn returning_during_pending_cancels_the_countdown() {
        let env = FakeEnv::new();
        let mut w = env.watcher();
        w.bind(target_hwnd());
        w.tick(0);

        env.cursor(500, 500);
        w.tick(10);
        assert!(matches!(w.state, State::Pending { .. }));

        env.cursor(50, 50);
        assert_eq!(w.tick(20), Action::None);
        assert_eq!(w.state, State::Armed);
        assert_eq!(env.hidden_calls(), 0);
    }

    #[test]
    fn reentry_into_home_rect_restores_even_if_window_moved() {
        let env = FakeEnv::new();
        let mut w = env.watcher();
        w.bind(target_hwnd());
        env.cursor(500, 500);
        w.tick(0);
        assert_eq!(w.tick(100), Action::Hide);

        // The hide strategy may have parked the window off-screen; the cursor
        // still has to come back to where the window used to be.
        env.window_rect(rect(-3_000, -3_000, -2_900, -2_900));
        env.cursor(50, 50);
        assert_eq!(w.tick(200), Action::Restore);
        assert_eq!(w.state, State::Armed);
        assert!(w.hidden.is_none());
    }

    #[test]
    fn restore_with_cursor_away_requires_one_entry_before_auto_hide() {
        let env = FakeEnv::new();
        let mut w = env.watcher();
        w.bind(target_hwnd());
        assert!(w.hide_now());
        assert_eq!(w.state, State::Hidden);

        env.cursor(500, 500);
        assert!(!w.toggle());
        assert_eq!(w.state, State::Armed);

        // Staying outside must not start the countdown until the window has
        // been visited once after the restore.
        assert_eq!(w.tick(0), Action::None);
        assert_eq!(w.state, State::Armed);

        env.cursor(50, 50);
        w.tick(10);
        env.cursor(500, 500);
        w.tick(20);
        assert!(matches!(w.state, State::Pending { .. }));
    }

    #[test]
    fn manual_hide_is_honoured_until_the_cursor_leaves() {
        let env = FakeEnv::new();
        let mut w = env.watcher();
        w.bind(target_hwnd());
        assert!(w.hide_now());

        // Cursor still on the window: do not undo the explicit hide.
        assert_eq!(w.tick(0), Action::None);
        assert_eq!(w.state, State::Hidden);

        env.cursor(500, 500);
        assert_eq!(w.tick(10), Action::None);
        assert_eq!(w.state, State::Hidden);

        // Mouse policy takes over again: coming back restores.
        env.cursor(50, 50);
        assert_eq!(w.tick(20), Action::Restore);
        assert_eq!(w.state, State::Armed);
    }

    #[test]
    fn latch_holds_hidden_until_latch_is_lifted() {
        let mut cfg = test_cfg();
        cfg.latch = true;
        let env = FakeEnv::new();
        let mut w = env.watcher_with(cfg);
        w.bind(target_hwnd());

        env.cursor(500, 500);
        w.tick(0);
        assert_eq!(w.tick(100), Action::Hide);
        assert_eq!(w.state, State::Latched);

        env.cursor(50, 50);
        assert_eq!(w.tick(200), Action::None);
        assert_eq!(w.state, State::Latched);

        w.cfg.latch = false;
        assert_eq!(w.tick(210), Action::Restore);
        assert_eq!(w.state, State::Armed);
    }

    #[test]
    fn minimized_window_is_never_hidden() {
        let env = FakeEnv::new();
        env.0.minimized.set(true);
        let mut w = env.watcher();
        w.bind(target_hwnd());

        env.cursor(500, 500);
        for now in [0, 100, 500] {
            assert_eq!(w.tick(now), Action::None);
            assert_eq!(w.state, State::Armed);
        }
        assert_eq!(env.hidden_calls(), 0);
        assert!(w.target.is_some());

        // Restored: protection resumes without hiding on a stale timestamp.
        env.0.minimized.set(false);
        assert_eq!(w.tick(600), Action::None);
        assert_eq!(w.state, State::Pending { since: 600 });
    }

    #[test]
    fn hide_only_when_focused_gates_the_countdown() {
        let mut cfg = test_cfg();
        cfg.hide_only_when_focused = true;
        let env = FakeEnv::new();
        let mut w = env.watcher_with(cfg);
        w.bind(target_hwnd());

        env.cursor(500, 500);
        env.0.focused.set(false);
        assert_eq!(w.tick(0), Action::None);
        assert_eq!(w.tick(500), Action::None);
        assert_eq!(w.state, State::Armed);

        env.0.focused.set(true);
        assert_eq!(w.tick(600), Action::None);
        assert_eq!(w.state, State::Pending { since: 600 });
        assert_eq!(w.tick(700), Action::Hide);
    }

    #[test]
    fn vanished_target_drops_the_binding() {
        let env = FakeEnv::new();
        let mut w = env.watcher();
        w.bind(target_hwnd());
        env.0.valid.set(false);

        assert_eq!(w.tick(0), Action::Drop);
        assert_eq!(w.state, State::Idle);
        assert!(w.target.is_none());
    }

    #[test]
    fn refused_hide_drops_the_binding_instead_of_retrying() {
        let env = FakeEnv::new();
        env.hide_ok(false);
        let mut w = env.watcher();
        w.bind(target_hwnd());

        env.cursor(500, 500);
        w.tick(0);
        assert_eq!(w.tick(100), Action::Refused);
        assert_eq!(w.state, State::Idle);
        assert!(w.target.is_none());
        assert!(w.hidden.is_none());
        assert_eq!(env.hidden_calls(), 1);

        // No target left: further ticks are quiet, not retries.
        assert_eq!(w.tick(200), Action::None);
        assert_eq!(env.hidden_calls(), 1);
    }

    #[test]
    fn refresh_identity_updates_title_but_not_home_rect_while_hidden() {
        let env = FakeEnv::new();
        let mut w = env.watcher();
        w.bind(target_hwnd());

        env.title("renamed");
        env.window_rect(rect(10, 10, 200, 200));
        w.refresh_identity();
        let t = w.target.as_ref().unwrap();
        assert_eq!(t.title, "renamed");
        assert_eq!(t.home_rect, rect(10, 10, 200, 200));

        assert!(w.hide_now());
        env.title("renamed again");
        env.window_rect(rect(300, 300, 400, 400));
        w.refresh_identity();
        let t = w.target.as_ref().unwrap();
        assert_eq!(t.title, "renamed again");
        assert_eq!(t.home_rect, rect(10, 10, 200, 200));
    }

    #[test]
    fn stale_inside_stamp_does_not_hide_immediately() {
        let env = FakeEnv::new();
        let mut w = env.watcher();
        w.bind(target_hwnd());
        w.tick(0);

        env.0.minimized.set(true);
        env.cursor(500, 500);
        w.tick(1_000);

        env.0.minimized.set(false);
        w.tick(1_100);
        assert_eq!(w.state, State::Pending { since: 1_100 });
        assert_eq!(w.tick(1_150), Action::None);
        assert_eq!(w.tick(1_200), Action::Hide);
    }

    #[test]
    fn describe_truncates_long_titles() {
        let env = FakeEnv::new();
        env.title("abcdefghijklmnopqrstuvwxyz0123456789");
        let mut w = env.watcher();
        w.bind(target_hwnd());
        assert_eq!(
            w.target.as_ref().unwrap().describe(),
            "fake.exe — abcdefghijklmnopqrstuvwxyz01…"
        );
    }

    #[test]
    fn truncate_counts_characters_not_bytes() {
        assert_eq!(truncate("中文标题很长", 2), "中文…");
        assert_eq!(truncate("ab", 2), "ab");
    }
}
