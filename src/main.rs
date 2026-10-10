//! PeekGuard — bind any window, hide it when the mouse leaves, restore on return.
//!
//! Zero dependencies. Single-binary Windows tool.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod config;
mod hide;
mod icon;
mod session;
mod tray;
mod watch;
mod win;

use config::{Config, Rule};
use std::ffi::c_void;
use tray::*;
use watch::{State, Watcher};
use win::*;

const CLASS_NAME: &str = "PeekGuard.MessageWindow.v1";
const TICK_MS: u32 = 40; // 25 Hz cursor polling: invisible CPU cost, snappy feel

const HOTKEY_BIND: i32 = 1;
const HOTKEY_TOGGLE: i32 = 2;
const HOTKEY_LATCH: i32 = 3;

/// Global app state, owned by the message loop and reached from the window proc.
struct App {
    watcher: Watcher,
    tray: Option<Tray>,
    hwnd: HWND,
}

static mut APP: *mut App = std::ptr::null_mut();

fn app() -> &'static mut App {
    unsafe { &mut *APP }
}

/// Diagnostic log. Enable with PEEKGUARD_DEBUG=1; writes to stderr and to
/// %APPDATA%\PeekGuard\debug.log so a GUI build can be troubleshot too.
pub fn dbg_log(args: std::fmt::Arguments) {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    let on = *ENABLED.get_or_init(|| std::env::var("PEEKGUARD_DEBUG").map(|v| v == "1").unwrap_or(false));
    if !on {
        return;
    }
    let line = format!("[{:>8}ms] {}
", tick_ms(), args);
    eprint!("{line}");
    let mut p = Config::config_dir();
    let _ = std::fs::create_dir_all(&p);
    p.push("debug.log");
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(p) {
        let _ = f.write_all(line.as_bytes());
    }
}

#[macro_export]
macro_rules! dbgmsg {
    ($($arg:tt)*) => { $crate::dbg_log(format_args!($($arg)*)) };
}

fn main() {
    unsafe {
        SetProcessDPIAware();
    }

    // Single instance: a second copy would fight over the same windows.
    let mutex_name = to_wide("Global\\PeekGuard-SingleInstance");
    unsafe {
        let h = CreateMutexW(std::ptr::null(), 0, mutex_name.as_ptr());
        if h.is_null() || error_already_exists() {
            message_box("PeekGuard 已经在运行了（看托盘图标）。", "PeekGuard", 0x40);
            return;
        }
    }

    let recovered = session::replay();
    let (cfg, note) = Config::load();

    let mut state = App { watcher: Watcher::new(cfg), tray: None, hwnd: std::ptr::null_mut() };
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

    let hicon = icon::create_icon(32).or_else(|| icon::create_icon(16)).unwrap_or_else(icon::stock_icon);
    app().tray = Some(Tray::add(hwnd, hicon, "PeekGuard — 未绑定"));

    register_hotkeys(hwnd);
    unsafe {
        SetTimer(hwnd, 1, TICK_MS, std::ptr::null());
    }

    apply_autobind();
    if recovered > 0 {
        notify(&format!("已恢复上次异常退出时隐藏的 {recovered} 个窗口。"));
    }
    if let Some(n) = note {
        if !n.is_empty() {
            notify(&n);
        }
    }
    update_tray();

    message_loop();
}

fn apply_autobind() {
    let a = app();
    let Some(rule) = a.watcher.cfg.autobind.clone() else { return };
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

fn register_hotkeys(hwnd: HWND) {
    unsafe {
        let a = RegisterHotKey(hwnd, HOTKEY_BIND, MOD_CONTROL | MOD_ALT | MOD_NOREPEAT, 'P' as u32);
        let b = RegisterHotKey(hwnd, HOTKEY_TOGGLE, MOD_CONTROL | MOD_ALT | MOD_NOREPEAT, 'H' as u32);
        let c = RegisterHotKey(hwnd, HOTKEY_LATCH, MOD_CONTROL | MOD_ALT | MOD_NOREPEAT, 'L' as u32);
        dbgmsg!(
            "hotkeys registered: bind={} toggle={} latch={} err={}",
            a,
            b,
            c,
            GetLastError()
        );
    }
}

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
    if let Some(t) = a.tray.as_mut() {
        t.remove();
    }
    unsafe {
        UnregisterHotKey(a.hwnd, HOTKEY_BIND);
        UnregisterHotKey(a.hwnd, HOTKEY_TOGGLE);
        UnregisterHotKey(a.hwnd, HOTKEY_LATCH);
    }
}

// ---------------------------------------------------------------- window proc

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
            on_menu(w as usize);
            0
        }
        m if m == WM_APP + 1 => {
            // tray callback
            let event = l as u32;
            if event == WM_LBUTTONDBLCLK {
                toggle_bind();
            } else if event == WM_RBUTTONUP || event == WM_CONTEXTMENU {
                on_menu(show_menu(hwnd, &menu_state()));
            }
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
            std::ptr::null_mut() as *mut c_void,
        )
    }
}

// ---------------------------------------------------------------- actions

static TICK_COUNT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn on_tick() {
    let a = app();
    let before = a.watcher.state;
    let action = a.watcher.tick(tick_ms());
    let after = a.watcher.state;
    let n = TICK_COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    if n % 25 == 0 {
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
    a.watcher.refresh_identity();
}

fn toggle_bind() {
    let a = app();
    dbgmsg!("toggle_bind: currently bound={}", a.watcher.target.is_some());
    if a.watcher.target.is_some() {
        a.watcher.unbind();
        session::write(&[]);
        notify("已解绑。");
    } else {
        let fg = unsafe { GetForegroundWindow() };
        dbgmsg!(
            "toggle_bind: fg={:?} title={:?} proc={:?} self={:?}",
            fg,
            window_title(fg),
            process_name(process_id(fg)),
            a.hwnd
        );
        if fg == a.hwnd || !is_valid(fg) {
            notify("找不到可以绑定的前台窗口。");
        } else if a.watcher.bind(fg) {
            let desc = a.watcher.target.as_ref().map(|t| t.describe()).unwrap_or_default();
            notify(&format!("已绑定: {desc}"));
        } else {
            notify("这个窗口不支持绑定。");
        }
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
            .map(|t| hide::owner_elevated(t.hwnd))
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
    notify(if on { "锁存模式已开启：隐身之后只能手动恢复。" } else { "锁存模式已关闭。" });
    update_tray();
}

fn menu_state() -> MenuState {
    let a = app();
    MenuState {
        bound: a.watcher.target.as_ref().map(|t| t.describe()),
        hidden: a.watcher.hidden.is_some(),
        mode: a.watcher.cfg.mode,
        latch: a.watcher.cfg.latch,
        delay_ms: a.watcher.cfg.delay_ms,
        is_elevated: is_elevated(),
    }
}

fn on_menu(cmd: usize) {
    match cmd {
        CMD_BIND => toggle_bind(),
        CMD_TOGGLE => toggle_hide(),
        CMD_MODE => cycle_mode(),
        CMD_LATCH => toggle_latch(),
        CMD_DELAY_DOWN => adjust_delay(-50),
        CMD_DELAY_UP => adjust_delay(50),
        CMD_SAVE_RULE => save_rule(),
        CMD_UNBIND_ALL => unbind_all(),
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

fn cycle_mode() {
    let a = app();
    // Refuse to switch when the bound target already rejects outside style edits,
    // so the user doesn't discover a broken mode when the boss walks in.
    if let Some(t) = &a.watcher.target {
        if hide::owner_elevated(t.hwnd) {
            notify("这个窗口不接受外部样式修改（可能以管理员身份运行）。");
            update_tray();
            return;
        }
    }
    a.watcher.cfg.mode = a.watcher.cfg.mode.next();
    let _ = a.watcher.cfg.save();
    notify(&format!("隐身方式: {}", tray::mode_name(a.watcher.cfg.mode)));
    update_tray();
}

fn adjust_delay(delta: i64) {
    let a = app();
    let v = (a.watcher.cfg.delay_ms as i64 + delta).clamp(0, 5_000) as u64;
    a.watcher.cfg.delay_ms = v;
    let _ = a.watcher.cfg.save();
    notify(&format!("防抖延迟: {v} ms"));
    update_tray();
}

fn save_rule() {
    let a = app();
    let Some(t) = a.watcher.target.clone() else {
        notify("先绑定一个窗口。");
        return;
    };
    // Prefer process+class: titles change constantly, but a title fragment is what
    // usually distinguishes "the chat window" from "the settings window".
    let rule = Rule { process: Some(t.process.clone()), class: Some(t.class.clone()), title: None };
    a.watcher.cfg.add_rule(rule.clone());
    match a.watcher.cfg.save() {
        Ok(_) => notify(&format!("已存为常驻规则: {}", rule.describe())),
        Err(e) => notify(&format!("保存配置失败: {e}")),
    }
}

/// Drop every active binding, restoring whatever was hidden.
fn unbind_all() {
    let a = app();
    let had = a.watcher.target.is_some();
    a.watcher.unbind();
    session::write(&[]);
    notify(if had { "已解绑并恢复。" } else { "当前没有绑定任何窗口。" });
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
    let msg = note.filter(|n| !n.is_empty()).unwrap_or_else(|| "配置已重载。".to_string());
    notify(&msg);
    update_tray();
}

fn open_config() {
    let path = Config::config_path();
    let _ = Config::load();
    let p = path.to_string_lossy().to_string();
    std::process::Command::new("cmd")
        .args(["/C", "start", "", &p])
        .spawn()
        .ok();
    notify(&format!("配置文件: {p}"));
}

fn about() {
    let a = app();
    let msg = format!(
        "PeekGuard 0.1.0\n\n\
         热键:\n  Ctrl+Alt+P  绑定/解绑当前窗口\n  Ctrl+Alt+H  立即隐身/恢复\n  Ctrl+Alt+L  切换锁存模式\n\n\
         当前: {}，方式 {}，延迟 {} ms\n\
         配置文件: {}",
        a.watcher.state.label(),
        tray::mode_name(a.watcher.cfg.mode),
        a.watcher.cfg.delay_ms,
        Config::config_path().display()
    );
    message_box(&msg, "关于 PeekGuard", 0x40);
}

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

// ---------------------------------------------------------------- helpers

fn message_box(text: &str, caption: &str, ty: u32) {
    let t = to_wide(text);
    let c = to_wide(caption);
    unsafe {
        MessageBoxW(std::ptr::null_mut(), t.as_ptr(), c.as_ptr(), ty);
    }
}

/// Transient balloon notification via the tray icon.
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

/// All visible top-level windows with a title, used for rule autobind.
fn top_level_windows() -> Vec<HWND> {
    let mut out: Vec<HWND> = Vec::new();
    unsafe extern "system" fn cb(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let v = &mut *(lparam as *mut Vec<HWND>);
        if is_visible(hwnd) && !window_title(hwnd).is_empty() {
            v.push(hwnd);
        }
        1
    }
    unsafe {
        EnumWindows(Some(cb), &mut out as *mut Vec<HWND> as LPARAM);
    }
    out
}
