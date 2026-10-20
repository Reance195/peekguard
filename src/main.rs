//! PeekGuard — bind any window, hide it when the mouse leaves, restore on return.
//!
//! Zero dependencies. Single binary: Windows (Win32), Linux (X11) and
//! macOS (CoreGraphics + Accessibility).

#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

// Windows-only subsystems. The Linux/macOS equivalents live in `x11/` and
// `macos/` and reach the shared code through the `plat` adapter.
#[cfg(windows)]
mod about;
#[cfg(windows)]
mod autostart;
mod config;
#[cfg(windows)]
mod hide;
#[cfg(windows)]
mod icon;
#[cfg(windows)]
mod ipc;
#[cfg(windows)]
mod picker;
mod plat;
mod rules;
mod session;
#[cfg(windows)]
mod tray;
mod update;
mod watch;
#[cfg(windows)]
mod win;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(not(windows))]
mod uds_ipc;
#[cfg(target_os = "linux")]
mod x11;

use config::{Config, HideMode, Rule};
#[cfg(target_os = "macos")]
use macos::*;
#[cfg(windows)]
use tray::*;
use watch::{State, Watcher};
#[cfg(windows)]
use win::*;
#[cfg(target_os = "linux")]
use x11::*;

// Both flavors answer to the same name for the JSON helpers; on Windows `ipc`
// is the named-pipe module declared above, here it aliases the shared Unix
// domain socket transport (Linux and macOS). What differs is only the plumb
// job of moving lines across the process boundary.
#[cfg(not(windows))]
use crate::uds_ipc as ipc;

#[cfg(windows)]
const CLASS_NAME: &str = "PeekGuard.MessageWindow.v1";
const TICK_MS: u32 = 16; // 62 Hz cursor polling: invisible CPU cost, no felt lag

#[cfg(windows)]
const HOTKEY_BIND: i32 = 1;
#[cfg(windows)]
const HOTKEY_TOGGLE: i32 = 2;
#[cfg(windows)]
const HOTKEY_LATCH: i32 = 3;

/// Posted by the background update-check thread. `wParam` = manual (non-zero when
/// the user asked), `lParam` = a `Box<update::CheckResult>` as a raw pointer.
#[cfg(windows)]
const WM_UPDATE_RESULT: u32 = WM_APP + 2;

/// Global app state, owned by the message loop and reached from the window proc.
struct App {
    watcher: Watcher,
    /// Finds rule-matched windows and hands them to the watcher.
    scanner: rules::Scanner,
    /// Tray icon. Windows only; the Linux build has no tray.
    #[cfg(windows)]
    tray: Option<Tray>,
    /// Message window (Windows). Zero on Linux, where there is no window.
    hwnd: HWND,
    /// Crosshair window picker, while the user is choosing a window to bind.
    picker: Option<plat::picker::Picker>,
    /// A newer release the last check found, if any.
    update_available: Option<update::UpdateInfo>,
    /// A background update check is in flight; don't start a second one.
    update_checking: bool,
}

static mut APP: *mut App = std::ptr::null_mut();

fn app() -> &'static mut App {
    unsafe { &mut *APP }
}

/// Diagnostic log. Enable with PEEKGUARD_DEBUG=1; writes to stderr and to
/// `debug.log` in the config dir so a GUI build can be troubleshot too.
pub fn dbg_log(args: std::fmt::Arguments) {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    let on = *ENABLED.get_or_init(|| {
        std::env::var("PEEKGUARD_DEBUG")
            .map(|v| v == "1")
            .unwrap_or(false)
    });
    if !on {
        return;
    }
    let line = format!(
        "[{:>8}ms] {}
",
        tick_ms(),
        args
    );
    eprint!("{line}");
    let mut p = Config::config_dir();
    let _ = std::fs::create_dir_all(&p);
    p.push("debug.log");
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(p)
    {
        let _ = f.write_all(line.as_bytes());
    }
}

#[macro_export]
macro_rules! dbgmsg {
    ($($arg:tt)*) => { $crate::dbg_log(format_args!($($arg)*)) };
}

fn main() {
    #[cfg(windows)]
    run_windows();
    #[cfg(not(windows))]
    run_desktop();
}

/// Windows entry point: message window, tray icon, hotkeys, message loop.
#[cfg(windows)]
fn run_windows() {
    unsafe {
        SetProcessDPIAware();
    }

    // If we panic, the first thing that must happen is putting any hidden window
    // back. Without this a crash would leave the user staring at a desktop with an
    // invisible window on it and no clue what happened.
    install_panic_guard();

    // Single instance: a second copy would fight over the same windows.
    //
    // A losing copy must never be mistaken for a working one. The check is a
    // global mutex, so a *test* instance collides with the user's real, running
    // PeekGuard through no fault of its own; `PEEKGUARD_MUTEX` lets a test name
    // its own mutex and get a genuinely independent instance. The warning box is
    // skipped under that variable too: a modal box is right for a double-clicked
    // shortcut, but in a test it would hang the run while looking like a guard
    // bug (the process stays alive and simply never hides anything).
    let mutex_name = std::env::var("PEEKGUARD_MUTEX")
        .map(|n| format!("Global\\{n}"))
        .unwrap_or_else(|_| "Global\\PeekGuard-SingleInstance".to_string());
    let mutex_name = to_wide(&mutex_name);
    unsafe {
        let h = CreateMutexW(std::ptr::null(), 0, mutex_name.as_ptr());
        if h.is_null() || error_already_exists() {
            if std::env::var("PEEKGUARD_MUTEX").is_err() {
                message_box("PeekGuard 已经在运行了（看托盘图标）。", "PeekGuard", 0x40);
            }
            return;
        }
    }

    let recovered = session::replay();
    let (cfg, note) = Config::load();

    let mut state = App {
        watcher: Watcher::new(cfg),
        scanner: rules::Scanner::new(),
        tray: None,
        hwnd: std::ptr::null_mut(),
        picker: None,
        update_available: None,
        update_checking: false,
    };
    unsafe {
        APP = &mut state;
    }

    let hwnd = create_message_window();
    if hwnd.is_null() {
        message_box("创建窗口失败，无法启动。", "PeekGuard", 0x10);
        return;
    }
    unsafe {
        (*APP).hwnd = hwnd;
    }

    let hicon = icon::create_icon(32)
        .or_else(|| icon::create_icon(16))
        .unwrap_or_else(icon::stock_icon);
    app().tray = Some(Tray::add(hwnd, hicon, "PeekGuard — 未绑定"));

    register_hotkeys(hwnd);
    // Serve local IPC for scripts and the uTools plugin. Best-effort: if the
    // pipe cannot be created the tray keeps working with hotkeys alone.
    ipc::spawn(hwnd);
    unsafe {
        SetTimer(hwnd, 1, TICK_MS, std::ptr::null());
    }

    maybe_arm_test_panic();

    apply_autobind();
    update_tray();
    scan_rules();

    // Announce the launch. A normal run has no main window, so this balloon is the
    // only thing telling the user PeekGuard came up. Skipped under PEEKGUARD_MUTEX
    // (the end-to-end suites) where it would just be noise. The crash-recovery count
    // and any config note ride along in the same balloon: the tray shows one
    // `szInfo` at a time, so a second notify would only overwrite the first.
    if std::env::var("PEEKGUARD_MUTEX").is_err() {
        let mut msg = String::from("PeekGuard 已启动，正在托盘运行。");
        if recovered > 0 {
            msg.push_str(&format!("已恢复上次异常退出时隐藏的 {recovered} 个窗口。"));
        }
        if let Some(n) = note {
            if !n.is_empty() {
                msg.push(' ');
                msg.push_str(&n);
            }
        }
        notify(&msg);
    }

    // Look for a newer release once at startup (opt-out via `check_update`). It
    // runs on a background thread and reports back through the message loop, so
    // a slow or absent network never delays the tray coming up. Skipped under
    // PEEKGUARD_MUTEX so the end-to-end suites stay offline and deterministic.
    if app().watcher.cfg.check_update && std::env::var("PEEKGUARD_MUTEX").is_err() {
        start_update_check(false);
    }

    message_loop();
}

/// macOS/Linux entry point: no window, no tray. The tool is driven by the
/// global hotkeys and by IPC (scripts, the uTools plugin), and reports through
/// desktop notifications.
#[cfg(not(windows))]
fn run_desktop() {
    // The connection to the window server must come up before anything else:
    // every helper in `x11`/`macos` talks through that handle.
    if let Err(e) = crate::plat::connect() {
        eprintln!("[peekguard] {e}");
        std::process::exit(1);
    }

    // If we panic, the first thing that must happen is putting any hidden
    // window back (same contract as Windows; see `run_windows`).
    install_panic_guard();

    // Single instance: a second copy would fight over the same windows.
    // `PEEKGUARD_MUTEX` lets the end-to-end suites name their own lock file
    // and get a genuinely independent instance.
    let _lock = match crate::plat::acquire_single_instance() {
        Ok(lock) => lock,
        Err(e) => {
            if std::env::var("PEEKGUARD_MUTEX").is_err() {
                notify(&e);
            }
            return;
        }
    };

    let recovered = session::replay();
    let (cfg, note) = Config::load();

    let mut state = App {
        watcher: Watcher::new(cfg),
        scanner: rules::Scanner::new(),
        hwnd: 0,
        picker: None,
        update_available: None,
        update_checking: false,
    };
    unsafe {
        APP = &mut state;
    }

    // Serve local IPC for scripts and the uTools plugin. Best-effort: a
    // failure to bind the socket leaves hotkeys working on their own.
    ipc::start();
    crate::plat::install_signal_handlers();

    maybe_arm_test_panic();

    apply_autobind();
    update_tray();
    scan_rules();

    // Announce the launch (notification; there is no tray). Skipped under
    // PEEKGUARD_MUTEX (the end-to-end suites) where it would just be noise.
    if std::env::var("PEEKGUARD_MUTEX").is_err() {
        let mut msg = String::from("PeekGuard 已启动，正在后台运行。");
        if recovered > 0 {
            msg.push_str(&format!("已恢复上次异常退出时隐藏的 {recovered} 个窗口。"));
        }
        if let Some(n) = note {
            if !n.is_empty() {
                msg.push(' ');
                msg.push_str(&n);
            }
        }
        notify(&msg);
    }

    // Look for a newer release once at startup (opt-out via `check_update`).
    // Skipped under PEEKGUARD_MUTEX so the suites stay offline and
    // deterministic.
    if app().watcher.cfg.check_update && std::env::var("PEEKGUARD_MUTEX").is_err() {
        start_update_check(false);
    }

    main_loop();
}

/// The macOS/Linux main loop: a fixed 16 ms tick plus three polled inputs --
/// IPC requests, global hotkeys and the pointer (via `on_tick`). On Linux the
/// hotkeys come from polling XQueryKeymap and the pointer from XQueryPointer;
/// on macOS a Carbon event-handler thread sets hotkey flags and CGEvent
/// queries read the pointer. Either way an input is queried, never pushed.
#[cfg(not(windows))]
fn main_loop() {
    let mut hotkeys = crate::plat::Hotkeys::new();
    let mut next = crate::plat::tick_ms();
    loop {
        // Serve waiting IPC clients first: a script or the plugin should
        // never wait a full tick for an answer.
        ipc::drain(ipc_command);
        poll_update_result();

        match hotkeys.poll() {
            Some(crate::plat::Hotkey::Bind) => toggle_bind(),
            Some(crate::plat::Hotkey::Toggle) => toggle_hide(),
            Some(crate::plat::Hotkey::Latch) => toggle_latch(),
            None => {}
        }

        on_tick();

        let sig = crate::plat::pending_signal();
        if sig != 0 {
            dbgmsg!("signal {} received, shutting down", sig);
            shutdown();
            return;
        }

        // Fixed cadence with drift correction. Falling behind (a slow tick,
        // a burst of IPC) resyncs instead of bursting to catch up.
        next += TICK_MS as u64;
        let now = crate::plat::tick_ms();
        if next > now {
            std::thread::sleep(std::time::Duration::from_millis(next - now));
        } else {
            next = now + TICK_MS as u64;
        }
    }
}

/// Do the one thing that matters when we die unexpectedly: give the window back.
fn install_panic_guard() {
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        unsafe {
            if !APP.is_null() {
                let a = &mut *APP;
                a.watcher.restore();
                session::write(&[]);
                #[cfg(windows)]
                if let Some(t) = a.tray.as_mut() {
                    t.remove();
                }
            }
        }
        prev(info);
    }));
}

/// Pick up rule-matched windows. Runs at the top of the tick, cheap enough that
/// the 1 Hz rate limit is the only throttling it needs.
fn scan_rules() {
    let a = app();
    if a.watcher.target.is_some() || a.picker.is_some() {
        return;
    }
    if !a.scanner.due(tick_ms()) {
        return;
    }
    let Some(hwnd) = a.scanner.find(&a.watcher.cfg) else {
        return;
    };
    if a.watcher.bind(hwnd) {
        a.scanner.note_bound(hwnd);
        dbgmsg!(
            "rule scan bound {:?}",
            a.watcher.target.as_ref().map(|t| t.describe())
        );
        notify(&format!(
            "已按规则保护: {}",
            a.watcher
                .target
                .as_ref()
                .map(|t| t.describe())
                .unwrap_or_default()
        ));
        update_tray();
    }
}

/// Test-only: `PEEKGUARD_PANIC_AFTER=<ms>` panics asynchronously, to verify that a
/// crash mid-hide leaves no window stranded. Documented in tests/README.md.
/// Never set this in normal use — it exists so the recovery path can be tested
/// on the real binary rather than assumed to work.
fn maybe_arm_test_panic() {
    let Ok(ms) = std::env::var("PEEKGUARD_PANIC_AFTER")
        .unwrap_or_default()
        .parse::<u64>()
    else {
        return;
    };
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(ms));
        panic!("PEEKGUARD_PANIC_AFTER test trigger");
    });
}

fn apply_autobind() {
    let a = app();
    let Some(rule) = a.watcher.cfg.autobind.clone() else {
        return;
    };
    for hwnd in top_level_windows() {
        let pid = process_id(hwnd);
        let proc = process_name(pid);
        let cls = class_name(hwnd);
        let title = window_title(hwnd);
        if rule.matches(&proc, &cls, &title) {
            a.watcher.bind(hwnd);
            return;
        }
    }
}

#[cfg(windows)]
fn register_hotkeys(hwnd: HWND) {
    unsafe {
        let a = RegisterHotKey(
            hwnd,
            HOTKEY_BIND,
            MOD_CONTROL | MOD_ALT | MOD_NOREPEAT,
            'P' as u32,
        );
        let b = RegisterHotKey(
            hwnd,
            HOTKEY_TOGGLE,
            MOD_CONTROL | MOD_ALT | MOD_NOREPEAT,
            'H' as u32,
        );
        let c = RegisterHotKey(
            hwnd,
            HOTKEY_LATCH,
            MOD_CONTROL | MOD_ALT | MOD_NOREPEAT,
            'L' as u32,
        );
        dbgmsg!(
            "hotkeys registered: bind={} toggle={} latch={} err={}",
            a,
            b,
            c,
            GetLastError()
        );
    }
}

#[cfg(windows)]
fn message_loop() {
    let mut msg = MSG::default();
    loop {
        let r = unsafe { GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) };
        if r <= 0 {
            break;
        }
        unsafe {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    shutdown();
}

fn shutdown() {
    let a = app();
    // Critical: never leave a window invisible behind us.
    a.watcher.unbind();
    session::write(&[]);
    #[cfg(windows)]
    {
        if let Some(t) = a.tray.as_mut() {
            t.remove();
        }
        unsafe {
            UnregisterHotKey(a.hwnd, HOTKEY_BIND);
            UnregisterHotKey(a.hwnd, HOTKEY_TOGGLE);
            UnregisterHotKey(a.hwnd, HOTKEY_LATCH);
        }
    }
}

// ---------------------------------------------------------------- window proc

#[cfg(windows)]
unsafe extern "system" fn wnd_proc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    match msg {
        WM_TIMER => {
            on_tick();
            0
        }
        WM_HOTKEY => {
            dbgmsg!("WM_HOTKEY id={}", w as i32);
            match w as i32 {
                HOTKEY_BIND => toggle_bind(),
                HOTKEY_TOGGLE => toggle_hide(),
                HOTKEY_LATCH => toggle_latch(),
                _ => {}
            }
            0
        }
        WM_COMMAND => {
            on_menu(w);
            0
        }
        m if m == WM_APP + 1 => {
            // tray callback
            let event = l as u32;
            if event == WM_LBUTTONDBLCLK {
                toggle_bind();
            } else if event == WM_RBUTTONUP || event == WM_CONTEXTMENU {
                on_menu(show_menu(hwnd, &menu_state()));
            } else if event == NIN_BALLOONUSERCLICK {
                // Clicking the "new version" balloon opens the download page.
                if app().update_available.is_some() {
                    open_update_page();
                }
            }
            0
        }
        m if m == WM_UPDATE_RESULT => {
            // Reclaim the box the worker thread posted, then act on it.
            let result = unsafe { *Box::from_raw(l as *mut update::CheckResult) };
            on_update_result(w != 0, result);
            0
        }
        m if m == ipc::WM_IPC_REQUEST => {
            // A pipe client is waiting; the pipe thread owns the box and
            // reclaims it once we signal the event inside on_request.
            unsafe { ipc::on_request(l) };
            0
        }
        WM_CLOSE => {
            // Closing means "go to tray", not "quit".
            0
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            0
        }
        _ => DefWindowProcW(hwnd, msg, w, l),
    }
}

#[cfg(windows)]
fn create_message_window() -> HWND {
    unsafe {
        let hinst = GetModuleHandleW(std::ptr::null());
        let class_name = to_wide(CLASS_NAME);
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: 0,
            lpfnWndProc: Some(wnd_proc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: hinst,
            hIcon: std::ptr::null_mut(),
            hCursor: std::ptr::null_mut(),
            hbrBackground: std::ptr::null_mut(),
            lpszMenuName: std::ptr::null(),
            lpszClassName: class_name.as_ptr(),
            hIconSm: std::ptr::null_mut(),
        };
        if RegisterClassExW(&wc) == 0 {
            // Already registered from a previous instance in-process; that's fine.
        }
        let title = to_wide("PeekGuard");
        CreateWindowExW(
            0,
            class_name.as_ptr(),
            title.as_ptr(),
            0,
            0,
            0,
            0,
            0,
            HWND_MESSAGE,
            std::ptr::null_mut(),
            hinst,
            std::ptr::null_mut(),
        )
    }
}

// ---------------------------------------------------------------- actions

static TICK_COUNT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn on_tick() {
    let a = app();
    if a.picker.is_some() {
        // While picking, the tick belongs to the picker: no rule scanning, no
        // watching — the user is mid-gesture and the bound window is untouched.
        let outcome = a.picker.as_mut().unwrap().tick(tick_ms());
        match outcome {
            plat::picker::Outcome::Continue => {}
            plat::picker::Outcome::Cancel => {
                a.picker = None; // dropping the picker tears its frame down
                notify("已取消绑定。");
            }
            plat::picker::Outcome::Pick(h) => {
                a.picker = None;
                bind_window(h);
            }
        }
        return;
    }
    scan_rules();
    let before = a.watcher.state;
    // Capture the handle: a refused hide clears the binding as part of the action,
    // so afterwards there is nothing left to read it from.
    let watched = a.watcher.target.as_ref().map(|t| t.hwnd);
    let action = a.watcher.tick(tick_ms());
    let after = a.watcher.state;
    let n = TICK_COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    if n.is_multiple_of(25) {
        let c = cursor_pos();
        let r = a
            .watcher
            .target
            .as_ref()
            .map(|t| window_rect(t.hwnd))
            .unwrap_or_default();
        dbgmsg!(
            "tick bind={} state={:?} cursor=({},{}) rect=({},{},{},{}) hidden={} action={:?}",
            a.watcher.target.is_some(),
            a.watcher.state,
            c.x,
            c.y,
            r.left,
            r.top,
            r.right,
            r.bottom,
            a.watcher.hidden.is_some(),
            action
        );
    }
    if action != watch::Action::None {
        dbgmsg!("action={:?} -> state={:?}", action, after);
    }
    if before != after || action != watch::Action::None {
        session::write(&a.watcher.hidden.iter().cloned().collect::<Vec<_>>());
        update_tray();
    }
    if action == watch::Action::Drop {
        update_tray();
    }
    if action == watch::Action::Refused {
        // The window would not accept the hide. Remember which one so the rule
        // scanner does not pick it straight back up and loop on it.
        if let Some(h) = watched {
            a.scanner.suppress(h);
        }
        #[cfg(windows)]
        notify("这个窗口拒绝被隐身（可能是管理员权限窗口）——已停止保护它。");
        #[cfg(not(windows))]
        notify("无法隐身这个窗口（试试换一种隐身方式）——已停止保护它。");
        session::write(&[]);
        update_tray();
    }
    a.watcher.refresh_identity();
}

fn toggle_bind() {
    let a = app();
    dbgmsg!(
        "toggle_bind: currently bound={} picking={}",
        a.watcher.target.is_some(),
        a.picker.is_some()
    );
    // The hotkey doubles as a cancel while the picker is up.
    if a.picker.is_some() {
        a.picker = None;
        notify("已取消绑定。");
        return;
    }
    if a.watcher.target.is_some() {
        // Release just this window; the rules keep working for others.
        if let Some(t) = &a.watcher.target {
            let h = t.hwnd;
            a.scanner.suppress(h);
        }
        a.watcher.unbind();
        session::write(&[]);
        notify("已解绑（这条规则不再接管这个窗口）。");
        update_tray();
    } else {
        open_picker();
    }
}

/// Open the crosshair picker, so the user chooses which window to bind.
fn open_picker() {
    let a = app();
    if a.picker.is_some() {
        return;
    }
    match plat::picker::Picker::open() {
        Some(p) => {
            a.picker = Some(p);
            notify("移动到要保护的窗口上，点击绑定；右键或 Esc 取消。");
        }
        None => notify("无法打开窗口选择器。"),
    }
}

/// Bind a specific window (used once the picker has a selection).
fn bind_window(h: HWND) {
    let a = app();
    dbgmsg!(
        "bind_window: hwnd={:?} title={:?} proc={:?}",
        h,
        window_title(h),
        process_name(process_id(h))
    );
    if h == a.hwnd || !is_valid(h) {
        notify("找不到可以绑定的窗口。");
    } else if a.watcher.bind(h) {
        // Binding by hand overrides any earlier suppression of this window.
        a.scanner.note_bound(h);
        let desc = a
            .watcher
            .target
            .as_ref()
            .map(|t| t.describe())
            .unwrap_or_default();
        notify(&format!("已绑定: {desc}"));
    } else {
        notify("这个窗口不支持绑定。");
    }
    update_tray();
}

fn toggle_hide() {
    let a = app();
    if a.watcher.target.is_none() {
        notify("先绑定一个窗口（Ctrl+Alt+P）。");
        return;
    }
    if a.watcher.hidden.is_some() {
        a.watcher.restore();
    } else if !a.watcher.hide_now() {
        let elevated = a
            .watcher
            .target
            .as_ref()
            .map(|t| plat::hide::owner_elevated(t.hwnd))
            .unwrap_or(false);
        if elevated {
            notify("无法操作该窗口：它可能以管理员身份运行，请也用管理员身份启动 PeekGuard。");
        } else {
            notify("隐藏失败，试试换一种隐身方式。");
        }
    }
    session::write(&a.watcher.hidden.iter().cloned().collect::<Vec<_>>());
    update_tray();
}

fn toggle_latch() {
    let a = app();
    a.watcher.cfg.latch = !a.watcher.cfg.latch;
    if a.watcher.cfg.latch && a.watcher.hidden.is_some() {
        a.watcher.state = State::Latched;
    }
    let on = a.watcher.cfg.latch;
    let _ = a.watcher.cfg.save();
    notify(if on {
        "锁存模式已开启：隐身之后只能手动恢复。"
    } else {
        "锁存模式已关闭。"
    });
    update_tray();
}

#[cfg(windows)]
fn menu_state() -> MenuState {
    let a = app();
    MenuState {
        bound: a.watcher.target.as_ref().map(|t| t.describe()),
        hidden: a.watcher.hidden.is_some(),
        mode: a.watcher.cfg.mode,
        latch: a.watcher.cfg.latch,
        delay_ms: a.watcher.cfg.delay_ms,
        is_elevated: is_elevated(),
        rules: a.watcher.cfg.rules.len(),
        scan_enabled: !a.scanner.muted(),
        autostart: plat::autostart::enabled(),
        check_update: a.watcher.cfg.check_update,
        update_available: a.update_available.as_ref().map(|i| i.version.clone()),
    }
}

#[cfg(windows)]
fn on_menu(cmd: usize) {
    match cmd {
        CMD_BIND => toggle_bind(),
        CMD_TOGGLE => toggle_hide(),
        CMD_MODE_ALPHA => set_mode(HideMode::Alpha),
        CMD_MODE_MOVE => set_mode(HideMode::Move),
        CMD_MODE_HIDE => set_mode(HideMode::Hide),
        CMD_LATCH => toggle_latch(),
        CMD_SCAN => toggle_scan(),
        CMD_DELAY_DOWN => adjust_delay(-50),
        CMD_DELAY_UP => adjust_delay(50),
        CMD_SAVE_RULE => save_rule(),
        CMD_UNBIND_ALL => unbind_all(),
        CMD_AUTOSTART => toggle_autostart(),
        CMD_CHECK_UPDATE => start_update_check(true),
        CMD_UPDATE_TOGGLE => toggle_update_check(),
        CMD_OPEN_UPDATE => open_update_page(),
        CMD_RELOAD => reload_config(),
        CMD_OPEN_CONFIG => open_config(),
        CMD_ABOUT => about(),
        CMD_QUIT => {
            let a = app();
            unsafe {
                DestroyWindow(a.hwnd);
            }
        }
        _ => {}
    }
}

/// Pause / resume rule-driven protection.
fn toggle_scan() {
    let a = app();
    let now_muted = !a.scanner.muted();
    a.scanner.set_muted(now_muted);
    notify(if now_muted {
        "已暂停规则自动保护（手动绑定不受影响）。"
    } else {
        "规则自动保护已恢复。"
    });
    update_tray();
}

fn toggle_autostart() {
    let enable = !plat::autostart::enabled();
    match plat::autostart::set(enable) {
        Ok(_) => notify(if enable {
            "已设置开机自动启动。"
        } else {
            "已取消开机自动启动。"
        }),
        Err(e) => notify(&format!("设置失败: {e}")),
    }
    update_tray();
}

fn set_mode(mode: HideMode) {
    let a = app();
    if a.watcher.cfg.mode == mode {
        return;
    }
    // Refuse to switch when the bound target already rejects outside style edits,
    // so the user doesn't discover a broken mode when the boss walks in.
    if let Some(t) = &a.watcher.target {
        if plat::hide::owner_elevated(t.hwnd) {
            notify("这个窗口不接受外部样式修改（可能以管理员身份运行）。");
            update_tray();
            return;
        }
    }
    a.watcher.cfg.mode = mode;
    let _ = a.watcher.cfg.save();
    notify(&format!("隐身方式: {}", plat::mode_name(mode)));
    update_tray();
}

#[cfg_attr(not(windows), allow(dead_code))]
fn adjust_delay(delta: i64) {
    let a = app();
    let v = (a.watcher.cfg.delay_ms as i64 + delta).clamp(0, 5_000) as u64;
    a.watcher.cfg.delay_ms = v;
    let _ = a.watcher.cfg.save();
    notify(&format!("防抖延迟: {v} ms"));
    update_tray();
}

#[cfg_attr(not(windows), allow(dead_code))]
fn save_rule() {
    let a = app();
    let Some(t) = a.watcher.target.clone() else {
        notify("先绑定一个窗口。");
        return;
    };
    // Prefer process+class: titles change constantly, but a title fragment is what
    // usually distinguishes "the chat window" from "the settings window".
    let rule = Rule {
        process: Some(t.process.clone()),
        class: Some(t.class.clone()),
        title: None,
    };
    a.watcher.cfg.add_rule(rule.clone());
    // Saving a rule is an explicit request for automatic protection.
    a.scanner.set_muted(false);
    match a.watcher.cfg.save() {
        Ok(_) => notify(&format!("已存为常驻规则: {}", rule.describe())),
        Err(e) => notify(&format!("保存配置失败: {e}")),
    }
    update_tray();
}
/// Drop the active binding, restoring whatever was hidden. Rule protection keeps
/// running for every other window — only the released one is held back.
fn unbind_all() {
    let a = app();
    let had = a.watcher.target.is_some();
    if let Some(t) = &a.watcher.target {
        let h = t.hwnd;
        a.scanner.suppress(h);
    }
    a.watcher.unbind();
    session::write(&[]);
    notify(if had {
        "已解绑并恢复显示。"
    } else {
        "当前没有绑定任何窗口。"
    });
    update_tray();
}

fn reload_config() {
    let a = app();
    let (cfg, note) = Config::load();
    let was_hidden = a.watcher.hidden.is_some();
    if was_hidden {
        // never re-read config with a window hidden — restore first
        a.watcher.restore();
    }
    a.watcher.cfg = cfg;
    // New rules may legitimately match windows the old set rejected.
    a.scanner.set_muted(false);
    let msg = note
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| "配置已重载。".to_string());
    notify(&msg);
    update_tray();
}

#[cfg_attr(not(windows), allow(dead_code))]
fn open_config() {
    let path = Config::config_path();
    let _ = Config::load();
    let p = path.to_string_lossy().to_string();
    #[cfg(windows)]
    {
        std::process::Command::new("cmd")
            .args(["/C", "start", "", &p])
            .spawn()
            .ok();
    }
    #[cfg(not(windows))]
    crate::plat::open_external(&p);
    notify(&format!("配置文件: {p}"));
}

#[cfg(windows)]
fn about() {
    let a = app();
    about::show(a.hwnd);
}

// ---------------------------------------------------------------- update check

/// Kick off a background check. `manual` runs show a "checking…" notification
/// and report "up to date" / failure; the startup run stays quiet unless a new
/// release turns up. One check runs at a time.
#[cfg(windows)]
fn start_update_check(manual: bool) {
    let a = app();
    if a.update_checking {
        if manual {
            notify("正在检查更新…");
        }
        return;
    }
    a.update_checking = true;
    if manual {
        notify("正在检查更新…");
    }
    let hwnd = a.hwnd as usize;
    // The worker only knows the window handle; it never touches APP, so the
    // single-threaded main loop stays the sole owner of application state.
    std::thread::spawn(move || {
        let result = update::check();
        let ptr = Box::into_raw(Box::new(result));
        unsafe {
            if PostMessageW(
                hwnd as HWND,
                WM_UPDATE_RESULT,
                manual as usize,
                ptr as isize,
            ) == 0
            {
                // No loop to hand the box to; take it back so it is not leaked.
                drop(Box::from_raw(ptr));
            }
        }
    });
}

#[cfg(not(windows))]
static UPDATE_RX: std::sync::Mutex<Option<std::sync::mpsc::Receiver<(bool, update::CheckResult)>>> =
    std::sync::Mutex::new(None);

/// Unix flavor: the worker hands its result back over a channel that the
/// main loop drains every tick.
#[cfg(not(windows))]
fn start_update_check(manual: bool) {
    let a = app();
    if a.update_checking {
        if manual {
            notify("正在检查更新…");
        }
        return;
    }
    a.update_checking = true;
    if manual {
        notify("正在检查更新…");
    }
    let (tx, rx) = std::sync::mpsc::channel::<(bool, update::CheckResult)>();
    *UPDATE_RX.lock().unwrap_or_else(|e| e.into_inner()) = Some(rx);
    // The worker never touches APP; the main loop stays the sole owner of
    // application state and lands the result on its next tick.
    std::thread::spawn(move || {
        let result = update::check();
        let _ = tx.send((manual, result));
    });
}

/// Drain finished checks queued by the update thread (Unix main loop).
#[cfg(not(windows))]
fn poll_update_result() {
    let guard = UPDATE_RX.lock().unwrap_or_else(|e| e.into_inner());
    let Some(rx) = guard.as_ref() else { return };
    while let Ok((manual, result)) = rx.try_recv() {
        on_update_result(manual, result);
    }
}

/// Land a finished check on the UI thread.
fn on_update_result(manual: bool, result: update::CheckResult) {
    let a = app();
    a.update_checking = false;
    match result {
        update::CheckResult::Available(info) => {
            dbgmsg!("update available: {} {}", info.version, info.url);
            a.update_available = Some(info.clone());
            #[cfg(windows)]
            notify(&format!(
                "发现新版本 {} —— 右键托盘图标选“下载新版本”，或点一下本提示。",
                info.version
            ));
            #[cfg(not(windows))]
            notify(&format!(
                "发现新版本 {} —— 请到 GitHub Releases 页面下载更新。",
                info.version
            ));
        }
        update::CheckResult::UpToDate => {
            dbgmsg!("update check: already current");
            if manual {
                notify(&format!("已是最新版本 v{}。", update::VERSION));
            }
        }
        update::CheckResult::Failed(e) => {
            dbgmsg!("update check failed: {e}");
            if manual {
                notify(&format!("检查更新失败: {e}"));
            }
        }
    }
    update_tray();
}

/// Turn the startup check on or off, and persist the choice.
#[cfg_attr(not(windows), allow(dead_code))]
fn toggle_update_check() {
    let a = app();
    a.watcher.cfg.check_update = !a.watcher.cfg.check_update;
    let on = a.watcher.cfg.check_update;
    let _ = a.watcher.cfg.save();
    notify(if on {
        "已开启：启动时自动检查更新。"
    } else {
        "已关闭：不再联网检查更新。"
    });
    update_tray();
}

/// Open the release page for the newer version. If none is known yet, run a
/// check instead of opening something arbitrary.
#[cfg_attr(not(windows), allow(dead_code))]
fn open_update_page() {
    let a = app();
    match a.update_available.as_ref() {
        Some(info) => open_url(&info.url),
        None => {
            notify("还没有可用的新版本，先检查一下。");
            start_update_check(true);
        }
    }
}

#[cfg(windows)]
fn open_url(url: &str) {
    let a = app();
    let op = to_wide("open");
    let file = to_wide(url);
    unsafe {
        ShellExecuteW(
            a.hwnd,
            op.as_ptr(),
            file.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        );
    }
}

/// macOS/Linux open links through the desktop's default handler.
#[cfg(not(windows))]
fn open_url(url: &str) {
    crate::plat::open_external(url);
}

#[cfg(windows)]
fn update_tray() {
    let a = app();
    let tip = match (&a.watcher.target, a.watcher.state) {
        (None, _) => "PeekGuard — 未绑定".to_string(),
        (Some(t), s) => {
            let base = format!("PeekGuard — {} · {}", s.label(), t.process);
            let _ = t;
            base
        }
    };
    if let Some(tr) = a.tray.as_mut() {
        tr.set_tip(&tip);
    }
}

/// macOS/Linux have no tray, so there is no tooltip to keep current; state
/// gets out through notifications and the IPC `status` command.
#[cfg(not(windows))]
fn update_tray() {}

// ---------------------------------------------------------------- helpers

#[cfg(windows)]
fn message_box(text: &str, caption: &str, ty: u32) {
    let t = to_wide(text);
    let c = to_wide(caption);
    unsafe {
        MessageBoxW(std::ptr::null_mut(), t.as_ptr(), c.as_ptr(), ty);
    }
}

/// Transient balloon notification via the tray icon.
#[cfg(windows)]
fn notify(text: &str) {
    let a = app();
    let Some(tr) = a.tray.as_mut() else { return };
    tr.data.uFlags = NIF_INFO;
    let wide = to_wide(text);
    let n = wide.len().min(tr.data.szInfo.len());
    tr.data.szInfo[..n].copy_from_slice(&wide[..n]);
    let title = to_wide("PeekGuard");
    let tn = title.len().min(tr.data.szInfoTitle.len());
    tr.data.szInfoTitle[..tn].copy_from_slice(&title[..tn]);
    tr.data.dwInfoFlags = 1; // NIIF_INFO
    tr.data.uVersionOrTimeout = 5000;
    unsafe {
        Shell_NotifyIconW(NIM_MODIFY, &tr.data);
    }
    tr.data.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
    eprintln!("[peekguard] {text}");
}

/// macOS/Linux raise a desktop notification through the platform helper
/// (`osascript` on macOS, `notify-send` on Linux) plus stderr.
#[cfg(not(windows))]
fn notify(text: &str) {
    crate::plat::notify(text);
}

/// All visible top-level windows with a title, used for rule autobind.
fn top_level_windows() -> Vec<HWND> {
    crate::plat::top_level_windows()
}

// ---------------------------------------------------------------- ipc (uTools & scripts)

/// Execute one command arriving over the named pipe (see `ipc.rs`) and return
/// a single-line JSON response. Runs on the UI thread, where all state lives.
///
/// Commands: `status`, `hide`, `show`, `toggle`, `bind`, `unbind`,
/// `latch on|off|toggle`, `mode alpha|move|hide`, `scan on|off|toggle`,
/// `reload`, `autostart on|off|toggle`. There is deliberately no `quit`.
pub(crate) fn ipc_command(line: &str) -> String {
    let mut parts = line.split_whitespace();
    let verb = parts.next().unwrap_or("").to_ascii_lowercase();
    let arg = parts.next().unwrap_or("").to_ascii_lowercase();
    match verb.as_str() {
        "status" | "ping" => ipc_ok(""),
        "hide" => ipc_quiet(force_hide()),
        "show" => ipc_quiet(force_show()),
        "toggle" => {
            if app().watcher.hidden.is_some() {
                ipc_quiet(force_show())
            } else {
                ipc_quiet(force_hide())
            }
        }
        "bind" => {
            if app().picker.is_some() {
                return ipc_ok("窗口选择器已经打开");
            }
            open_picker();
            if app().picker.is_some() {
                ipc_ok("窗口选择器已打开：把光标移到目标窗口上点击即可")
            } else {
                ipc_err("无法打开窗口选择器")
            }
        }
        "unbind" => {
            if app().watcher.target.is_none() {
                ipc_ok("当前没有绑定窗口")
            } else {
                unbind_all();
                ipc_ok("已解绑并恢复显示")
            }
        }
        "latch" => {
            let a = app();
            match parse_toggle_arg(&arg, a.watcher.cfg.latch) {
                Ok(want) if a.watcher.cfg.latch == want => ipc_ok(if want {
                    "锁存模式已经开启"
                } else {
                    "锁存模式已经关闭"
                }),
                Ok(want) => {
                    toggle_latch();
                    ipc_ok(if want {
                        "锁存模式已开启"
                    } else {
                        "锁存模式已关闭"
                    })
                }
                Err(e) => ipc_err(&e),
            }
        }
        "mode" => match HideMode::parse(&arg) {
            Some(m) if app().watcher.cfg.mode == m => {
                ipc_ok(&format!("隐身方式已经是 {}", plat::mode_name(m)))
            }
            Some(m) => {
                set_mode(m);
                // set_mode refuses on windows that reject outside style edits.
                if app().watcher.cfg.mode == m {
                    ipc_ok(&format!("隐身方式已切换为 {}", plat::mode_name(m)))
                } else {
                    ipc_err("这个窗口不接受外部样式修改（可能以管理员身份运行）")
                }
            }
            None => ipc_err("用法: mode alpha|move|hide"),
        },
        "scan" => {
            let a = app();
            let enabled = !a.scanner.muted();
            match parse_toggle_arg(&arg, enabled) {
                Ok(want) if enabled == want => ipc_ok(if want {
                    "规则自动保护已经开启"
                } else {
                    "规则自动保护已经暂停"
                }),
                Ok(want) => {
                    toggle_scan();
                    ipc_ok(if want {
                        "规则自动保护已开启"
                    } else {
                        "规则自动保护已暂停"
                    })
                }
                Err(e) => ipc_err(&e),
            }
        }
        "reload" => {
            reload_config();
            ipc_ok("配置已重载")
        }
        "autostart" => {
            let current = plat::autostart::enabled();
            match parse_toggle_arg(&arg, current) {
                Ok(want) if current == want => ipc_ok(if want {
                    "开机自启已经开启"
                } else {
                    "开机自启已经关闭"
                }),
                Ok(want) => {
                    toggle_autostart();
                    if plat::autostart::enabled() == want {
                        ipc_ok(if want {
                            "已设置开机自动启动"
                        } else {
                            "已取消开机自动启动"
                        })
                    } else {
                        ipc_err("设置开机自启失败")
                    }
                }
                Err(e) => ipc_err(&e),
            }
        }
        "" => ipc_err("空命令"),
        other => ipc_err(&format!("未知命令: {other}")),
    }
}

/// Hide the bound window right now. Same behavior as the hotkey path, but the
/// outcome is returned instead of ballooned; the IPC layer hands it back in
/// the JSON response.
fn force_hide() -> Result<String, String> {
    let a = app();
    if a.watcher.target.is_none() {
        return Err("先绑定一个窗口（Ctrl+Alt+P）".to_string());
    }
    if a.watcher.hidden.is_some() {
        return Ok("窗口已经处于隐身状态".to_string());
    }
    if a.watcher.hide_now() {
        session::write(&a.watcher.hidden.iter().cloned().collect::<Vec<_>>());
        update_tray();
        Ok("已隐身".to_string())
    } else {
        session::write(&a.watcher.hidden.iter().cloned().collect::<Vec<_>>());
        update_tray();
        let elevated = a
            .watcher
            .target
            .as_ref()
            .map(|t| plat::hide::owner_elevated(t.hwnd))
            .unwrap_or(false);
        Err(if elevated {
            "无法操作该窗口：它可能以管理员身份运行，请也用管理员身份启动 PeekGuard".to_string()
        } else {
            "隐藏失败，试试换一种隐身方式".to_string()
        })
    }
}

/// Bring the bound window back, if it is hidden.
fn force_show() -> Result<String, String> {
    let a = app();
    if a.watcher.target.is_none() {
        return Err("先绑定一个窗口（Ctrl+Alt+P）".to_string());
    }
    if a.watcher.hidden.is_none() {
        return Ok("窗口当前可见".to_string());
    }
    a.watcher.restore();
    session::write(&[]);
    update_tray();
    Ok("已恢复显示".to_string())
}

/// Parse `on` / `off` / `toggle` (empty means toggle). `current` is the state
/// a bare `toggle` flips.
fn parse_toggle_arg(arg: &str, current: bool) -> Result<bool, String> {
    match arg {
        "" | "toggle" => Ok(!current),
        "on" | "true" | "1" => Ok(true),
        "off" | "false" | "0" => Ok(false),
        other => Err(format!("无法识别的参数 \"{other}\"（可用 on|off|toggle）")),
    }
}

/// Outcome helper for IPC commands: the message travels back in the JSON only,
/// so hiding or restoring a window never fires a notification.
fn ipc_quiet(outcome: Result<String, String>) -> String {
    match outcome {
        Ok(msg) => ipc_ok(&msg),
        Err(msg) => ipc_err(&msg),
    }
}

fn ipc_ok(msg: &str) -> String {
    format!(
        r#"{{"ok":true,"msg":"{}","state":{}}}"#,
        ipc::escape_json(msg),
        ipc_state_json()
    )
}

fn ipc_err(msg: &str) -> String {
    format!(
        r#"{{"ok":false,"err":"{}","state":{}}}"#,
        ipc::escape_json(msg),
        ipc_state_json()
    )
}

/// Snapshot of everything a panel or script wants to know. Machine-readable
/// names (`state` is english, numbers stay numbers); the plugin maps them to
/// labels.
fn ipc_state_json() -> String {
    let a = app();
    let (bound, desc, proc) = match &a.watcher.target {
        Some(t) => (true, t.describe(), t.process.clone()),
        None => (false, String::new(), String::new()),
    };
    format!(
        concat!(
            r#"{{"version":"{}","bound":{},"bound_desc":"{}","bound_process":"{}","#,
            r#""state":"{}","hidden":{},"mode":"{}","latch":{},"delay_ms":{},"rules":{},"#,
            r#""scan":{},"picking":{},"elevated":{},"autostart":{},"config":"{}"}}"#,
        ),
        update::VERSION,
        bound,
        ipc::escape_json(&desc),
        ipc::escape_json(&proc),
        watcher_state_name(a.watcher.state),
        a.watcher.hidden.is_some(),
        a.watcher.cfg.mode.as_str(),
        a.watcher.cfg.latch,
        a.watcher.cfg.delay_ms,
        a.watcher.cfg.rules.len(),
        !a.scanner.muted(),
        a.picker.is_some(),
        is_elevated(),
        plat::autostart::enabled(),
        ipc::escape_json(&Config::config_path().to_string_lossy()),
    )
}

fn watcher_state_name(state: State) -> &'static str {
    match state {
        State::Idle => "idle",
        State::Armed => "armed",
        State::Pending { .. } => "pending",
        State::Hidden => "hidden",
        State::Latched => "latched",
    }
}
