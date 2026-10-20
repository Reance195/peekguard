//! Tray icon + context menu.

use crate::config::HideMode;
use crate::win::*;

pub const TRAY_UID: u32 = 1;

// menu command ids
pub const CMD_BIND: usize = 100;
pub const CMD_TOGGLE: usize = 101;
pub const CMD_MODE_ALPHA: usize = 110;
pub const CMD_MODE_MOVE: usize = 112;
pub const CMD_MODE_HIDE: usize = 113;
pub const CMD_LATCH: usize = 111;
pub const CMD_DELAY_UP: usize = 120;
pub const CMD_DELAY_DOWN: usize = 121;
pub const CMD_SAVE_RULE: usize = 130;
pub const CMD_UNBIND_ALL: usize = 131;
pub const CMD_SCAN: usize = 132;
pub const CMD_RELOAD: usize = 140;
pub const CMD_OPEN_CONFIG: usize = 141;
pub const CMD_AUTOSTART: usize = 160;
pub const CMD_CHECK_UPDATE: usize = 170;
pub const CMD_UPDATE_TOGGLE: usize = 171;
pub const CMD_OPEN_UPDATE: usize = 172;
pub const CMD_ABOUT: usize = 150;
pub const CMD_QUIT: usize = 199;

pub struct Tray {
    pub data: NOTIFYICONDATAW,
    pub alive: bool,
}

impl Tray {
    pub fn add(hwnd: HWND, icon: HICON, tip: &str) -> Tray {
        let mut data = NOTIFYICONDATAW {
            cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: hwnd,
            uID: TRAY_UID,
            uFlags: NIF_MESSAGE | NIF_ICON | NIF_TIP,
            uCallbackMessage: WM_APP + 1,
            hIcon: icon,
            ..Default::default()
        };
        set_tip(&mut data, tip);
        let alive = unsafe { Shell_NotifyIconW(NIM_ADD, &data) != 0 };
        Tray { data, alive }
    }

    pub fn set_tip(&mut self, tip: &str) {
        set_tip(&mut self.data, tip);
        if self.alive {
            unsafe {
                Shell_NotifyIconW(NIM_MODIFY, &self.data);
            }
        }
    }

    pub fn remove(&mut self) {
        if self.alive {
            unsafe {
                Shell_NotifyIconW(NIM_DELETE, &self.data);
            }
            self.alive = false;
        }
    }
}

impl Drop for Tray {
    fn drop(&mut self) {
        self.remove();
    }
}

fn set_tip(data: &mut NOTIFYICONDATAW, tip: &str) {
    let wide = to_wide(tip);
    let n = wide.len().min(data.szTip.len());
    data.szTip[..n].copy_from_slice(&wide[..n]);
    // ensure termination
    if n == data.szTip.len() {
        data.szTip[n - 1] = 0;
    }
}

/// Configuration snapshot the menu needs for check marks and labels.
pub struct MenuState {
    pub bound: Option<String>,
    pub hidden: bool,
    pub mode: HideMode,
    pub latch: bool,
    pub delay_ms: u64,
    pub is_elevated: bool,
    /// How many `rule =` lines are configured.
    pub rules: usize,
    /// Rule scanning is not paused.
    pub scan_enabled: bool,
    pub autostart: bool,
    /// Check GitHub for a new release on startup.
    pub check_update: bool,
    /// Version of a newer release the last check found, if any.
    pub update_available: Option<String>,
    /// Live hotkey labels ("Ctrl+Alt+P" …), so the menu never goes stale
    /// after the user rebinds a combo.
    pub hotkey_bind: String,
    pub hotkey_toggle: String,
    pub hotkey_latch: String,
}

/// Show the context menu and return the chosen command id (0 = dismissed).
pub fn show_menu(menu_owner: HWND, st: &MenuState) -> usize {
    unsafe {
        let menu = CreatePopupMenu();
        if menu.is_null() {
            return 0;
        }

        let bind_label = match &st.bound {
            Some(desc) => format!("解绑  ({})", desc),
            None => format!("选择窗口并绑定…  ({})", st.hotkey_bind),
        };
        append(menu, MF_STRING, CMD_BIND, &bind_label);

        if st.bound.is_some() {
            let toggle_label = if st.hidden {
                format!("恢复显示  ({})", st.hotkey_toggle)
            } else {
                format!("立即隐身  ({})", st.hotkey_toggle)
            };
            append(menu, MF_STRING, CMD_TOGGLE, &toggle_label);
        }

        append(menu, MF_SEPARATOR, 0, "");

        let mode_menu = CreatePopupMenu();
        if mode_menu.is_null() {
            DestroyMenu(menu);
            return 0;
        }
        for (id, m, desc) in [
            (
                CMD_MODE_ALPHA,
                HideMode::Alpha,
                "透明  — 最兼容，鼠标穿透保留",
            ),
            (
                CMD_MODE_MOVE,
                HideMode::Move,
                "移出屏幕  — 连 Alt+Tab 预览都藏掉",
            ),
            (CMD_MODE_HIDE, HideMode::Hide, "隐藏窗口  — 任务栏按钮仍在"),
        ] {
            let f = MF_STRING | MF_RADIOCHECK | if m == st.mode { MF_CHECKED } else { 0 };
            append(mode_menu, f, id, &format!("{}   {}", mode_name(m), desc));
        }
        append(menu, MF_POPUP, mode_menu as usize, "隐身方式");

        let scan_flags = if st.scan_enabled {
            MF_STRING | MF_CHECKED
        } else {
            MF_STRING
        };
        append(
            menu,
            scan_flags,
            CMD_SCAN,
            &format!("规则自动保护（{} 条规则）", st.rules),
        );

        let latch_flags = if st.latch {
            MF_STRING | MF_CHECKED
        } else {
            MF_STRING
        };
        append(
            menu,
            latch_flags,
            CMD_LATCH,
            &format!("锁存模式（只能手动恢复）  ({})", st.hotkey_latch),
        );

        append(
            menu,
            MF_STRING,
            CMD_DELAY_DOWN,
            &format!("缩短防抖延迟  ({} ms)", st.delay_ms),
        );
        append(
            menu,
            MF_STRING,
            CMD_DELAY_UP,
            &format!("延长防抖延迟  ({} ms)", st.delay_ms),
        );

        append(menu, MF_SEPARATOR, 0, "");

        let save_flags = if st.bound.is_some() {
            MF_STRING
        } else {
            MF_STRING | MF_GRAYED
        };
        append(menu, save_flags, CMD_SAVE_RULE, "把当前绑定存为常驻规则");

        append(menu, save_flags, CMD_UNBIND_ALL, "解绑当前窗口（保留规则）");
        append(menu, MF_STRING, CMD_RELOAD, "重载配置");
        append(menu, MF_STRING, CMD_OPEN_CONFIG, "打开配置文件");

        let startup_flags = if st.autostart {
            MF_STRING | MF_CHECKED
        } else {
            MF_STRING
        };
        append(menu, startup_flags, CMD_AUTOSTART, "开机自动启动");

        let update_flags = if st.check_update {
            MF_STRING | MF_CHECKED
        } else {
            MF_STRING
        };
        append(menu, update_flags, CMD_UPDATE_TOGGLE, "启动时自动检查更新");

        append(menu, MF_STRING, CMD_CHECK_UPDATE, "检查更新…");
        if let Some(v) = &st.update_available {
            append(menu, MF_STRING, CMD_OPEN_UPDATE, &format!("下载新版本 {v}"));
        }

        append(menu, MF_SEPARATOR, 0, "");

        if st.is_elevated {
            append(menu, MF_STRING | MF_GRAYED, CMD_ABOUT, "已以管理员身份运行");
        }
        append(menu, MF_STRING, CMD_ABOUT, "关于 PeekGuard");
        append(menu, MF_SEPARATOR, 0, "");
        append(menu, MF_STRING, CMD_QUIT, "退出");

        let pt = cursor_pos();
        // Required so the menu closes when clicking elsewhere.
        SetForegroundWindow(menu_owner);
        let cmd = TrackPopupMenu(
            menu,
            TPM_RIGHTBUTTON | TPM_RETURNCMD,
            pt.x,
            pt.y,
            0,
            menu_owner,
            std::ptr::null(),
        );
        PostMessageW(menu_owner, WM_NULL, 0, 0);
        DestroyMenu(menu); // destroys `mode_menu` with it
        cmd as usize
    }
}

fn append(menu: HMENU, flags: u32, id: usize, text: &str) {
    let wide = to_wide(text);
    unsafe {
        if flags & MF_SEPARATOR != 0 {
            AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null());
        } else {
            AppendMenuW(menu, flags, id, wide.as_ptr());
        }
    }
}

pub fn mode_name(m: HideMode) -> &'static str {
    match m {
        HideMode::Alpha => "透明",
        HideMode::Move => "移出屏幕",
        HideMode::Hide => "隐藏窗口",
    }
}
