//! Minimal hand-written Win32 FFI. Zero external crates.
//!
//! Type names keep their Win32 spelling (`HWND`, `NOTIFYICONDATAW`, …) on
//! purpose: a hand-written binding is meant to be checked line-by-line against
//! the SDK headers, and renaming anything only gets in the way of that.
#![allow(non_snake_case, non_camel_case_types, dead_code)]
#![allow(clippy::upper_case_acronyms)]

use std::ffi::c_void;

pub type HWND = *mut c_void;
pub type HINSTANCE = *mut c_void;
pub type HMENU = *mut c_void;
pub type HICON = *mut c_void;
pub type HCURSOR = *mut c_void;
pub type HBRUSH = *mut c_void;
pub type HANDLE = *mut c_void;
pub type HKEY = *mut c_void;
pub type WPARAM = usize;
pub type LPARAM = isize;
pub type LRESULT = isize;
pub type BOOL = i32;
pub type COLORREF = u32;

// ---- window styles
pub const GWL_EXSTYLE: i32 = -20;
pub const WS_EX_TRANSPARENT: isize = 0x0000_0020;
pub const WS_EX_TOOLWINDOW: isize = 0x0000_0080;
pub const WS_EX_LAYERED: isize = 0x0008_0000;
pub const WS_EX_TOPMOST: isize = 0x0000_0008;
pub const WS_EX_NOACTIVATE: isize = 0x0800_0000;

// ---- window classes (styles / hit-testing)
pub const WS_POPUP: u32 = 0x8000_0000;
pub const WS_CAPTION: u32 = 0x00C0_0000;
pub const WS_SYSMENU: u32 = 0x0008_0000;
pub const GA_ROOT: u32 = 2;

// ---- layered window
pub const LWA_COLORKEY: u32 = 0x1;
pub const LWA_ALPHA: u32 = 0x2;

// ---- ShowWindow
pub const SW_HIDE: i32 = 0;
pub const SW_SHOWNORMAL: i32 = 1;
pub const SW_SHOWMINNOACTIVE: i32 = 7;
pub const SW_SHOWNA: i32 = 8;

// ---- stock cursors / GDI text
pub const IDC_ARROW: u16 = 32512;
pub const IDC_CROSS: u16 = 32515;
pub const IDC_HAND: u16 = 32649;
pub const TRANSPARENT: i32 = 1;
pub const FW_NORMAL: i32 = 400;
pub const FW_SEMIBOLD: i32 = 600;
pub const DEFAULT_CHARSET: u32 = 1;
pub const DT_LEFT: u32 = 0x0000;
pub const DT_VCENTER: u32 = 0x0004;
pub const DT_SINGLELINE: u32 = 0x0020;
pub const DT_CALCRECT: u32 = 0x0400;
pub const DT_NOPREFIX: u32 = 0x0800;

// ---- virtual keys we poll
pub const VK_LBUTTON: i32 = 0x01;
pub const VK_RBUTTON: i32 = 0x02;
pub const VK_ESCAPE: i32 = 0x1B;

// ---- SetWindowPos
pub const SWP_NOSIZE: u32 = 0x0001;
pub const SWP_NOMOVE: u32 = 0x0002;
pub const SWP_NOZORDER: u32 = 0x0004;
pub const SWP_NOACTIVATE: u32 = 0x0010;

// ---- GetSystemMetrics
pub const SM_XVIRTUALSCREEN: i32 = 76;
pub const SM_YVIRTUALSCREEN: i32 = 77;
pub const SM_CXVIRTUALSCREEN: i32 = 78;
pub const SM_CYVIRTUALSCREEN: i32 = 79;

// ---- hotkeys
pub const MOD_ALT: u32 = 0x0001;
pub const MOD_CONTROL: u32 = 0x0002;
pub const MOD_SHIFT: u32 = 0x0004;
pub const MOD_NOREPEAT: u32 = 0x4000;

// ---- process / token
pub const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
pub const TOKEN_QUERY: u32 = 0x0008;
pub const TOKEN_ELEVATION: u32 = 20;
pub const ERROR_ALREADY_EXISTS: u32 = 183;
pub const ERROR_SUCCESS: u32 = 0;
pub const ERROR_FILE_NOT_FOUND: u32 = 2;

// ---- registry
pub const HKEY_CURRENT_USER: HKEY = 0x8000_0001u32 as usize as HKEY;
pub const KEY_QUERY_VALUE: u32 = 0x0001;
pub const KEY_SET_VALUE: u32 = 0x0002;
pub const REG_SZ: u32 = 1;

// ---- messages
pub const WM_DESTROY: u32 = 0x0002;
pub const WM_PAINT: u32 = 0x000F;
pub const WM_CLOSE: u32 = 0x0010;
pub const WM_ERASEBKGND: u32 = 0x0014;
pub const WM_SETCURSOR: u32 = 0x0020;
pub const WM_KEYDOWN: u32 = 0x0100;
pub const WM_COMMAND: u32 = 0x0111;
pub const WM_TIMER: u32 = 0x0113;
pub const WM_HOTKEY: u32 = 0x0312;
pub const WM_APP: u32 = 0x8000;
pub const WM_RBUTTONUP: u32 = 0x0205;
pub const WM_LBUTTONUP: u32 = 0x0201;
pub const WM_LBUTTONDBLCLK: u32 = 0x0203;
pub const WM_CONTEXTMENU: u32 = 0x007B;
pub const WM_NULL: u32 = 0x0000;

// ---- menus
pub const MF_STRING: u32 = 0x0000;
pub const MF_CHECKED: u32 = 0x0008;
pub const MF_SEPARATOR: u32 = 0x0800;
pub const MF_GRAYED: u32 = 0x0001;
pub const MF_POPUP: u32 = 0x0010;
pub const MF_RADIOCHECK: u32 = 0x0200;
pub const TPM_RIGHTBUTTON: u32 = 0x0002;
pub const TPM_RETURNCMD: u32 = 0x0100;

// ---- tray
pub const NIM_ADD: u32 = 0;
pub const NIM_MODIFY: u32 = 1;
pub const NIM_DELETE: u32 = 2;
pub const NIF_MESSAGE: u32 = 0x01;
pub const NIF_ICON: u32 = 0x02;
pub const NIF_TIP: u32 = 0x04;
pub const NIF_INFO: u32 = 0x10;
pub const IDI_APPLICATION: u16 = 32512;

pub const HWND_MESSAGE: HWND = -3isize as HWND;
pub const HWND_TOPMOST: HWND = -1isize as HWND;

// ================================================================ structs

#[repr(C)]
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct POINT {
    pub x: i32,
    pub y: i32,
}

#[repr(C)]
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct RECT {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl RECT {
    pub fn width(&self) -> i32 {
        self.right - self.left
    }
    pub fn height(&self) -> i32 {
        self.bottom - self.top
    }
    pub fn is_empty(&self) -> bool {
        self.width() <= 0 || self.height() <= 0
    }
    /// Inside test with `margin` px of slack on every side.
    pub fn contains_with_margin(&self, x: i32, y: i32, margin: i32) -> bool {
        x >= self.left - margin
            && x < self.right + margin
            && y >= self.top - margin
            && y < self.bottom + margin
    }
    pub fn intersects(&self, o: &RECT) -> bool {
        self.left < o.right && o.left < self.right && self.top < o.bottom && o.top < self.bottom
    }
}

#[repr(C)]
pub struct MSG {
    pub hwnd: HWND,
    pub message: u32,
    pub wParam: WPARAM,
    pub lParam: LPARAM,
    pub time: u32,
    pub pt: POINT,
}

impl Default for MSG {
    fn default() -> Self {
        MSG {
            hwnd: std::ptr::null_mut(),
            message: 0,
            wParam: 0,
            lParam: 0,
            time: 0,
            pt: POINT::default(),
        }
    }
}

pub type WNDPROC = unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM) -> LRESULT;

#[repr(C)]
pub struct WNDCLASSEXW {
    pub cbSize: u32,
    pub style: u32,
    pub lpfnWndProc: Option<WNDPROC>,
    pub cbClsExtra: i32,
    pub cbWndExtra: i32,
    pub hInstance: HINSTANCE,
    pub hIcon: HICON,
    pub hCursor: HCURSOR,
    pub hbrBackground: HBRUSH,
    pub lpszMenuName: *const u16,
    pub lpszClassName: *const u16,
    pub hIconSm: HICON,
}

impl Default for WNDCLASSEXW {
    fn default() -> Self {
        unsafe { std::mem::zeroed() }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct GUID {
    pub data1: u32,
    pub data2: u16,
    pub data3: u16,
    pub data4: [u8; 8],
}

#[repr(C)]
pub struct NOTIFYICONDATAW {
    pub cbSize: u32,
    pub hWnd: HWND,
    pub uID: u32,
    pub uFlags: u32,
    pub uCallbackMessage: u32,
    pub hIcon: HICON,
    pub szTip: [u16; 128],
    pub dwState: u32,
    pub dwStateMask: u32,
    pub szInfo: [u16; 256],
    pub uVersionOrTimeout: u32,
    pub szInfoTitle: [u16; 64],
    pub dwInfoFlags: u32,
    pub guidItem: GUID,
    pub hBalloonIcon: HICON,
}

impl Default for NOTIFYICONDATAW {
    fn default() -> Self {
        unsafe { std::mem::zeroed() }
    }
}

/// Paint state, mirroring `PAINTSTRUCT`. `BeginPaint` fills it; `EndPaint` reads it.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct PAINTSTRUCT {
    pub hdc: *mut c_void,
    pub fErase: BOOL,
    pub rcPaint: RECT,
    pub fRestore: BOOL,
    pub fIncUpdate: BOOL,
    pub rgbReserved: [u8; 32],
}

impl Default for PAINTSTRUCT {
    fn default() -> Self {
        unsafe { std::mem::zeroed() }
    }
}

// ================================================================ imports

#[link(name = "user32")]
unsafe extern "system" {
    pub fn GetCursorPos(p: *mut POINT) -> BOOL;
    pub fn GetSystemMetrics(i: i32) -> i32;
    pub fn GetForegroundWindow() -> HWND;
    pub fn GetWindowRect(h: HWND, r: *mut RECT) -> BOOL;
    pub fn GetWindowTextW(h: HWND, buf: *mut u16, n: i32) -> i32;
    pub fn GetWindowTextLengthW(h: HWND) -> i32;
    pub fn GetClassNameW(h: HWND, buf: *mut u16, n: i32) -> i32;
    pub fn GetWindowThreadProcessId(h: HWND, pid: *mut u32) -> u32;
    pub fn IsWindow(h: HWND) -> BOOL;
    pub fn IsWindowVisible(h: HWND) -> BOOL;
    pub fn IsIconic(h: HWND) -> BOOL;
    pub fn GetWindowLongPtrW(h: HWND, idx: i32) -> isize;
    pub fn SetWindowLongPtrW(h: HWND, idx: i32, v: isize) -> isize;
    pub fn SetLayeredWindowAttributes(h: HWND, key: COLORREF, alpha: u8, flags: u32) -> BOOL;
    pub fn GetLayeredWindowAttributes(
        h: HWND,
        key: *mut COLORREF,
        alpha: *mut u8,
        flags: *mut u32,
    ) -> BOOL;
    pub fn SetWindowPos(h: HWND, after: HWND, x: i32, y: i32, cx: i32, cy: i32, flags: u32)
        -> BOOL;
    pub fn ShowWindow(h: HWND, cmd: i32) -> BOOL;
    pub fn RegisterHotKey(h: HWND, id: i32, mods: u32, vk: u32) -> BOOL;
    pub fn UnregisterHotKey(h: HWND, id: i32) -> BOOL;
    pub fn GetMessageW(m: *mut MSG, h: HWND, min: u32, max: u32) -> BOOL;
    pub fn TranslateMessage(m: *const MSG) -> BOOL;
    pub fn DispatchMessageW(m: *const MSG) -> LRESULT;
    pub fn PostQuitMessage(code: i32);
    pub fn PostMessageW(h: HWND, msg: u32, w: WPARAM, l: LPARAM) -> BOOL;
    pub fn SetTimer(h: HWND, id: usize, ms: u32, cb: *const c_void) -> usize;
    pub fn KillTimer(h: HWND, id: usize) -> BOOL;
    pub fn DefWindowProcW(h: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT;
    pub fn RegisterClassExW(c: *const WNDCLASSEXW) -> u16;
    pub fn CreateWindowExW(
        ex: u32,
        class: *const u16,
        name: *const u16,
        style: u32,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        parent: HWND,
        menu: HMENU,
        inst: HINSTANCE,
        param: *mut c_void,
    ) -> HWND;
    pub fn DestroyWindow(h: HWND) -> BOOL;
    pub fn CreatePopupMenu() -> HMENU;
    pub fn AppendMenuW(m: HMENU, flags: u32, id: usize, text: *const u16) -> BOOL;
    pub fn DestroyMenu(m: HMENU) -> BOOL;
    pub fn TrackPopupMenu(
        m: HMENU,
        flags: u32,
        x: i32,
        y: i32,
        reserved: i32,
        h: HWND,
        r: *const RECT,
    ) -> BOOL;
    pub fn SetForegroundWindow(h: HWND) -> BOOL;
    pub fn LoadIconW(inst: HINSTANCE, name: *const u16) -> HICON;
    pub fn SetProcessDPIAware() -> BOOL;
    pub fn GetDC(h: HWND) -> *mut c_void;
    pub fn ReleaseDC(h: HWND, dc: *mut c_void) -> i32;
    pub fn MessageBoxW(h: HWND, text: *const u16, caption: *const u16, ty: u32) -> i32;
    pub fn EnumWindows(
        cb: Option<unsafe extern "system" fn(HWND, LPARAM) -> BOOL>,
        lparam: LPARAM,
    ) -> BOOL;
    pub fn WindowFromPoint(p: POINT) -> HWND;
    pub fn GetAncestor(h: HWND, flags: u32) -> HWND;
    pub fn GetAsyncKeyState(vk: i32) -> i16;
    pub fn LoadCursorW(inst: HINSTANCE, name: *const u16) -> HCURSOR;
    pub fn SetCursor(c: HCURSOR) -> HCURSOR;
    pub fn SetCapture(h: HWND) -> HWND;
    pub fn ReleaseCapture() -> BOOL;
    pub fn GetClientRect(h: HWND, r: *mut RECT) -> BOOL;
    pub fn BeginPaint(h: HWND, ps: *mut PAINTSTRUCT) -> *mut c_void;
    pub fn EndPaint(h: HWND, ps: *const PAINTSTRUCT) -> BOOL;
    pub fn InvalidateRect(h: HWND, r: *const RECT, erase: BOOL) -> BOOL;
    pub fn FillRect(dc: *mut c_void, r: *const RECT, brush: HBRUSH) -> i32;
    pub fn FrameRect(dc: *mut c_void, r: *const RECT, brush: HBRUSH) -> i32;
    pub fn DrawTextW(dc: *mut c_void, text: *const u16, n: i32, r: *mut RECT, fmt: u32) -> i32;
    pub fn DrawIcon(dc: *mut c_void, x: i32, y: i32, icon: HICON) -> BOOL;
    pub fn AdjustWindowRect(r: *mut RECT, style: u32, menu: BOOL) -> BOOL;
    pub fn ClientToScreen(h: HWND, p: *mut POINT) -> BOOL;
    pub fn FindWindowW(class: *const u16, name: *const u16) -> HWND;
}

#[link(name = "kernel32")]
unsafe extern "system" {
    pub fn GetModuleHandleW(name: *const u16) -> HINSTANCE;
    pub fn GetLastError() -> u32;
    pub fn OpenProcess(access: u32, inherit: BOOL, pid: u32) -> HANDLE;
    pub fn CloseHandle(h: HANDLE) -> BOOL;
    pub fn QueryFullProcessImageNameW(h: HANDLE, flags: u32, buf: *mut u16, size: *mut u32)
        -> BOOL;
    pub fn GetCurrentProcess() -> HANDLE;
    pub fn CreateMutexW(attr: *const c_void, owner: BOOL, name: *const u16) -> HANDLE;
    pub fn GetTickCount64() -> u64;
    pub fn Sleep(ms: u32);
}

#[link(name = "gdi32")]
unsafe extern "system" {
    pub fn CreateSolidBrush(color: COLORREF) -> HBRUSH;
    pub fn DeleteObject(o: *mut c_void) -> BOOL;
    pub fn SelectObject(dc: *mut c_void, o: *mut c_void) -> *mut c_void;
    pub fn SetTextColor(dc: *mut c_void, color: COLORREF) -> COLORREF;
    pub fn SetBkMode(dc: *mut c_void, mode: i32) -> i32;
    pub fn GetDeviceCaps(dc: *mut c_void, index: i32) -> i32;
    pub fn CreateFontW(
        height: i32,
        width: i32,
        escapement: i32,
        orientation: i32,
        weight: i32,
        italic: u32,
        underline: u32,
        strike: u32,
        charset: u32,
        out_precision: u32,
        clip_precision: u32,
        quality: u32,
        pitch_family: u32,
        face: *const u16,
    ) -> *mut c_void;
}

#[link(name = "shell32")]
unsafe extern "system" {
    pub fn Shell_NotifyIconW(msg: u32, data: *const NOTIFYICONDATAW) -> BOOL;
    pub fn ShellExecuteW(
        hwnd: HWND,
        op: *const u16,
        file: *const u16,
        params: *const u16,
        dir: *const u16,
        show: i32,
    ) -> *mut c_void;
}

#[link(name = "advapi32")]
unsafe extern "system" {
    pub fn OpenProcessToken(proc: HANDLE, access: u32, token: *mut HANDLE) -> BOOL;
    pub fn GetTokenInformation(
        token: HANDLE,
        class: u32,
        info: *mut c_void,
        len: u32,
        ret: *mut u32,
    ) -> BOOL;
    pub fn RegOpenKeyExW(key: HKEY, sub: *const u16, opts: u32, access: u32, out: *mut HKEY)
        -> u32;
    pub fn RegQueryValueExW(
        key: HKEY,
        name: *const u16,
        reserved: *mut u32,
        ty: *mut u32,
        data: *mut u8,
        len: *mut u32,
    ) -> u32;
    pub fn RegSetValueExW(
        key: HKEY,
        name: *const u16,
        reserved: u32,
        ty: u32,
        data: *const u8,
        len: u32,
    ) -> u32;
    pub fn RegDeleteValueW(key: HKEY, name: *const u16) -> u32;
    pub fn RegCloseKey(key: HKEY) -> u32;
}

// ================================================================ safe helpers

pub fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Read a window's caption.
///
/// Deliberately `GetWindowTextW`, **not** `SendMessageTimeoutW(WM_GETTEXT)`.
/// `GetWindowText` is documented to return the cached caption instead of blocking
/// on a cross-process `SendMessage`, and measurement backs that up: against a
/// deliberately hung window it returned in 0.09 ms, while a 200 ms-timeout
/// `WM_GETTEXTLENGTH` both burned the full timeout *and* reported failure — i.e.
/// the "safer" API was ~2000x slower here and lost the title of exactly the
/// windows it was supposed to protect against. See
/// `docs/superpowers/specs/2026-10-10-peekguard-design.md` ("窗口标题读取").
pub fn window_title(h: HWND) -> String {
    unsafe {
        let n = GetWindowTextLengthW(h);
        if n <= 0 {
            return String::new();
        }
        let mut buf = vec![0u16; (n + 1) as usize];
        let got = GetWindowTextW(h, buf.as_mut_ptr(), n + 1);
        String::from_utf16_lossy(&buf[..got.max(0) as usize])
    }
}

pub fn class_name(h: HWND) -> String {
    unsafe {
        let mut buf = vec![0u16; 256];
        let got = GetClassNameW(h, buf.as_mut_ptr(), 256);
        String::from_utf16_lossy(&buf[..got.max(0) as usize])
    }
}

pub fn process_id(h: HWND) -> u32 {
    unsafe {
        let mut pid: u32 = 0;
        GetWindowThreadProcessId(h, &mut pid);
        pid
    }
}

pub fn process_name(pid: u32) -> String {
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if h.is_null() {
            return String::new();
        }
        let mut buf = vec![0u16; 1024];
        let mut n: u32 = 1024;
        let ok = QueryFullProcessImageNameW(h, 0, buf.as_mut_ptr(), &mut n);
        CloseHandle(h);
        if ok == 0 {
            return String::new();
        }
        let path = String::from_utf16_lossy(&buf[..n as usize]);
        path.rsplit(['\\', '/']).next().unwrap_or("").to_string()
    }
}

pub fn window_rect(h: HWND) -> RECT {
    unsafe {
        let mut r = RECT::default();
        GetWindowRect(h, &mut r);
        r
    }
}

pub fn cursor_pos() -> POINT {
    unsafe {
        let mut p = POINT::default();
        GetCursorPos(&mut p);
        p
    }
}

pub fn virtual_screen() -> RECT {
    unsafe {
        let x = GetSystemMetrics(SM_XVIRTUALSCREEN);
        let y = GetSystemMetrics(SM_YVIRTUALSCREEN);
        let w = GetSystemMetrics(SM_CXVIRTUALSCREEN);
        let h = GetSystemMetrics(SM_CYVIRTUALSCREEN);
        RECT {
            left: x,
            top: y,
            right: x + w,
            bottom: y + h,
        }
    }
}

pub fn ex_style(h: HWND) -> isize {
    unsafe { GetWindowLongPtrW(h, GWL_EXSTYLE) }
}

pub fn set_ex_style(h: HWND, v: isize) -> isize {
    unsafe { SetWindowLongPtrW(h, GWL_EXSTYLE, v) }
}

/// Read the current layered alpha. None when the window is not layered with alpha.
pub fn layered_alpha(h: HWND) -> Option<u8> {
    unsafe {
        let mut key: COLORREF = 0;
        let mut alpha: u8 = 0;
        let mut flags: u32 = 0;
        if GetLayeredWindowAttributes(h, &mut key, &mut alpha, &mut flags) == 0 {
            return None;
        }
        if flags & LWA_ALPHA != 0 {
            Some(alpha)
        } else {
            None
        }
    }
}

pub fn set_layered_alpha(h: HWND, alpha: u8) -> bool {
    unsafe { SetLayeredWindowAttributes(h, 0, alpha, LWA_ALPHA) != 0 }
}

pub fn is_valid(h: HWND) -> bool {
    !h.is_null() && unsafe { IsWindow(h) != 0 }
}

pub fn is_visible(h: HWND) -> bool {
    unsafe { IsWindowVisible(h) != 0 }
}

pub fn is_minimized(h: HWND) -> bool {
    unsafe { IsIconic(h) != 0 }
}

pub fn is_elevated() -> bool {
    unsafe {
        let mut token: HANDLE = std::ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return false;
        }
        let mut elevated: i32 = 0;
        let mut ret: u32 = 0;
        let ok = GetTokenInformation(
            token,
            TOKEN_ELEVATION,
            &mut elevated as *mut i32 as *mut c_void,
            std::mem::size_of::<i32>() as u32,
            &mut ret,
        );
        CloseHandle(token);
        ok != 0 && elevated != 0
    }
}

pub fn error_already_exists() -> bool {
    unsafe { GetLastError() == ERROR_ALREADY_EXISTS }
}

pub fn tick_ms() -> u64 {
    unsafe { GetTickCount64() }
}

/// All visible top-level windows with a title, in z-order (front first).
/// Used by rule matching; the guard itself only ever touches one window.
pub fn top_level_windows() -> Vec<HWND> {
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
