//! Watch state machine: bind a target window, follow the cursor, hide/restore.
#![allow(dead_code)]

use crate::config::Config;
use crate::hide::{hide, Hidden};
use crate::win::*;

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
    pub fn capture(hwnd: HWND) -> Option<Target> {
        if !is_valid(hwnd) {
            return None;
        }
        let pid = process_id(hwnd);
        Some(Target {
            hwnd,
            process: process_name(pid),
            class: class_name(hwnd),
            title: window_title(hwnd),
            home_rect: window_rect(hwnd),
        })
    }

    pub fn describe(&self) -> String {
        let t = if self.title.is_empty() { "(无标题)".to_string() } else { truncate(&self.title, 28) };
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
    /// Cursor position the last time we evaluated.
    last_cursor: POINT,
    /// Set when we hid because the user asked explicitly (hotkey/tray) rather than the mouse leaving.
    manual: bool,
}

impl Watcher {
    pub fn new(cfg: Config) -> Watcher {
        Watcher {
            state: State::Idle,
            target: None,
            hidden: None,
            cfg,
            last_cursor: POINT::default(),
            manual: false,
        }
    }

    pub fn bind(&mut self, hwnd: HWND) -> bool {
        self.unbind();
        match Target::capture(hwnd) {
            Some(t) => {
                self.target = Some(t);
                self.state = State::Armed;
                self.manual = false;
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
    }

    /// Force-hide (hotkey/tray). Only valid with a live target.
    pub fn hide_now(&mut self) -> bool {
        let Some(t) = &self.target else { return false };
        if self.hidden.is_some() {
            return true;
        }
        match hide(t.hwnd, self.cfg.mode, self.cfg.alpha) {
            Some(h) => {
                self.hidden = Some(h);
                self.state = if self.cfg.latch { State::Latched } else { State::Hidden };
                self.manual = true;
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
            let r = window_rect(t.hwnd);
            let r = if r.is_empty() { t.home_rect } else { r };
            r.contains_with_margin(p.x, p.y, margin)
        }
    }

    /// One evaluation tick. `now_ms` is a monotonic millisecond clock.
    pub fn tick(&mut self, now_ms: u64) -> Action {
        let Some(target) = self.target.clone() else {
            self.state = State::Idle;
            return Action::None;
        };

        if !is_valid(target.hwnd) {
            self.restore();
            self.target = None;
            self.state = State::Idle;
            return Action::Drop;
        }

        let cursor = cursor_pos();
        self.last_cursor = cursor;
        let hidden = self.hidden.is_some();

        // Latched: nothing but an explicit call gets it back.
        if self.state == State::Latched {
            return Action::None;
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
        if self.cfg.skip_minimized && is_minimized(target.hwnd) {
            self.state = State::Armed;
            return Action::None;
        }
        if self.cfg.hide_only_when_focused {
            let fg = unsafe { GetForegroundWindow() };
            if fg != target.hwnd {
                self.state = State::Armed;
                return Action::None;
            }
        }

        let inside = self.cursor_inside(cursor, false);
        match self.state {
            State::Armed => {
                if !inside {
                    self.state = State::Pending { since: now_ms };
                }
                Action::None
            }
            State::Pending { since } => {
                if inside {
                    self.state = State::Armed;
                    return Action::None;
                }
                if now_ms.saturating_sub(since) >= self.cfg.delay_ms {
                    match hide(target.hwnd, self.cfg.mode, self.cfg.alpha) {
                        Some(h) => {
                            self.hidden = Some(h);
                            self.state = if self.cfg.latch { State::Latched } else { State::Hidden };
                            Action::Hide
                        }
                        None => {
                            // Couldn't hide (elevated target, exotic window). Back off
                            // so we don't spin on it.
                            self.state = State::Armed;
                            Action::Drop
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
            if is_valid(t.hwnd) {
                t.title = window_title(t.hwnd);
                if !self.hidden.is_some() {
                    let r = window_rect(t.hwnd);
                    if !r.is_empty() {
                        t.home_rect = r;
                    }
                }
            }
        }
    }
}
