//! Custom "About" window.
//!
//! Replaces the old `MessageBoxW` with a real, painted window: the generated
//! application icon, the version, the hotkey list, and a clickable GitHub link.
//!
//! The window is a fixed-size popup with a standard caption (so it drags and
//! closes the way windows should) but its client area is drawn by us, which is
//! what makes it look like part of the app rather than a system alert.
//!
//! All sizes below are given at 96 DPI and scaled by the target monitor's DPI
//! before use, so the window and its text stay legible on high-DPI displays.
#![allow(non_snake_case)]

use crate::icon;
use crate::win::*;
use std::ffi::c_void;

const ABOUT_CLASS: &str = "PeekGuard.About.v1";
const VERSION: &str = "0.1.0";
const URL: &str = "https://github.com/Reance195/peekguard";
const URL_LABEL: &str = "github.com/Reance195/peekguard";

// --- base layout, in 96-DPI pixels -----------------------------------------
const BASE_W: i32 = 520;
const BASE_H: i32 = 348;
const PAD: i32 = 28;
const ICON_PX: i32 = 64;

// colours (COLORREF = 0x00BBGGRR)
const BG: COLORREF = 0x00FF_FFFF;
const TITLE: COLORREF = 0x0020_2020;
const MUTED: COLORREF = 0x0080_8080;
const BODY: COLORREF = 0x0030_3030;
const LINK: COLORREF = 0x00CC_6600; // RGB(0,102,204)
const RULE: COLORREF = 0x00E0_E0E0;

const LOGPIXELSY: i32 = 90;

/// The big icon, created once and reused on every repaint.
static mut ABOUT_ICON: HICON = std::ptr::null_mut();

/// Scale factor from 96 DPI, using a device context. 1.0 if it cannot be read.
unsafe fn scale_of(dc: *mut c_void) -> f32 {
    if dc.is_null() {
        return 1.0;
    }
    let dpi = GetDeviceCaps(dc, LOGPIXELSY);
    if dpi <= 0 {
        1.0
    } else {
        dpi as f32 / 96.0
    }
}

/// Scale for the monitor the system would pick for a new window.
fn screen_scale() -> f32 {
    unsafe {
        let dc = GetDC(std::ptr::null_mut());
        let s = scale_of(dc);
        ReleaseDC(std::ptr::null_mut(), dc);
        s
    }
}

/// Scale for the monitor a window currently sits on.
fn window_scale(hwnd: HWND) -> f32 {
    unsafe {
        let dc = GetDC(hwnd);
        let s = scale_of(dc);
        ReleaseDC(hwnd, dc);
        s
    }
}

fn sc(v: i32, scale: f32) -> i32 {
    (v as f32 * scale).round() as i32
}

/// Show the About window, or focus the existing one.
pub fn show(owner: HWND) {
    unsafe {
        let class = to_wide(ABOUT_CLASS);
        if focus_existing(&class) {
            return;
        }
        let scale = screen_scale();
        if ABOUT_ICON.is_null() {
            let px = sc(ICON_PX, scale).clamp(16, 256) as usize;
            ABOUT_ICON = icon::create_icon(px).unwrap_or_else(icon::stock_icon);
        }
        let hinst = GetModuleHandleW(std::ptr::null());
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(about_proc),
            hInstance: hinst,
            hIcon: icon::create_icon(32).unwrap_or_else(icon::stock_icon),
            hIconSm: icon::create_icon(16).unwrap_or_else(icon::stock_icon),
            lpszClassName: class.as_ptr(),
            ..Default::default()
        };
        RegisterClassExW(&wc);

        let style = WS_CAPTION | WS_SYSMENU;
        let mut r = RECT {
            left: 0,
            top: 0,
            right: sc(BASE_W, scale),
            bottom: sc(BASE_H, scale),
        };
        AdjustWindowRect(&mut r, style, 0);
        let w = r.width();
        let h = r.height();
        let screen = virtual_screen();
        let x = screen.left + (screen.width() - w) / 2;
        let y = screen.top + (screen.height() - h) / 2;

        let title = to_wide("关于 PeekGuard");
        let hwnd = CreateWindowExW(
            WS_EX_TOPMOST as u32,
            class.as_ptr(),
            title.as_ptr(),
            style,
            x,
            y,
            w,
            h,
            owner,
            std::ptr::null_mut(),
            hinst,
            std::ptr::null_mut(),
        );
        if hwnd.is_null() {
            return;
        }
        ShowWindow(hwnd, SW_SHOWNORMAL);
        SetForegroundWindow(hwnd);
    }
}

/// Bring an already-open About window to the front instead of stacking another.
unsafe fn focus_existing(class: &[u16]) -> bool {
    let h = FindWindowW(class.as_ptr(), std::ptr::null());
    if h.is_null() {
        false
    } else {
        SetForegroundWindow(h);
        true
    }
}

fn make_font(height: i32, weight: i32, underline: u32, scale: f32) -> *mut c_void {
    let face = to_wide("Segoe UI");
    unsafe {
        CreateFontW(
            sc(height, scale),
            0,
            0,
            0,
            weight,
            0,
            underline,
            0,
            DEFAULT_CHARSET,
            0,
            0,
            0,
            0,
            face.as_ptr(),
        )
    }
}

/// Rect of the link text inside the client area, measured for `hdc`.
unsafe fn link_rect(hdc: *mut c_void, scale: f32) -> RECT {
    let font = make_font(20, FW_NORMAL, 1, scale);
    let old = SelectObject(hdc, font);
    let mut rc = RECT {
        left: sc(PAD, scale),
        top: sc(296, scale),
        right: sc(PAD + 620, scale),
        bottom: sc(328, scale),
    };
    let text = to_wide(URL_LABEL);
    DrawTextW(
        hdc,
        text.as_ptr(),
        -1,
        &mut rc,
        DT_LEFT | DT_SINGLELINE | DT_CALCRECT | DT_NOPREFIX,
    );
    SelectObject(hdc, old);
    DeleteObject(font);
    rc
}

/// Draw one line of text with `font`/`color`, restoring the DC afterwards.
unsafe fn draw_line(hdc: *mut c_void, font: *mut c_void, color: COLORREF, rc: RECT, text: &str) {
    let old = SelectObject(hdc, font);
    SetTextColor(hdc, color);
    SetBkMode(hdc, TRANSPARENT);
    let t = to_wide(text);
    let mut r = rc;
    DrawTextW(
        hdc,
        t.as_ptr(),
        -1,
        &mut r,
        DT_LEFT | DT_SINGLELINE | DT_NOPREFIX,
    );
    SelectObject(hdc, old);
}

fn cursor_over_link(hwnd: HWND, pt: POINT) -> bool {
    unsafe {
        let mut origin = POINT { x: 0, y: 0 };
        ClientToScreen(hwnd, &mut origin);
        point_in(link_rect_client(hwnd), pt.x - origin.x, pt.y - origin.y)
    }
}

/// The link rect in client coordinates, for `hwnd`.
fn link_rect_client(hwnd: HWND) -> RECT {
    unsafe {
        let hdc = GetDC(hwnd);
        let lr = link_rect(hdc, window_scale(hwnd));
        ReleaseDC(hwnd, hdc);
        lr
    }
}

fn point_in(r: RECT, x: i32, y: i32) -> bool {
    x >= r.left && x < r.right && y >= r.top && y < r.bottom
}

unsafe extern "system" fn about_proc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    match msg {
        WM_ERASEBKGND => 1,
        WM_PAINT => {
            let scale = window_scale(hwnd);
            let mut ps = PAINTSTRUCT::default();
            let hdc = BeginPaint(hwnd, &mut ps);
            let mut rc = RECT::default();
            GetClientRect(hwnd, &mut rc);

            let bg = CreateSolidBrush(BG);
            FillRect(hdc, &rc, bg);
            DeleteObject(bg);

            if !ABOUT_ICON.is_null() {
                DrawIcon(hdc, sc(PAD, scale), sc(PAD, scale), ABOUT_ICON);
            }

            let title_f = make_font(30, FW_SEMIBOLD, 0, scale);
            let ver_f = make_font(18, FW_NORMAL, 0, scale);
            let body_f = make_font(18, FW_NORMAL, 0, scale);
            let hint_f = make_font(15, FW_NORMAL, 0, scale);
            let link_f = make_font(20, FW_NORMAL, 1, scale);

            let pad = sc(PAD, scale);
            let ix = sc(PAD + ICON_PX + 16, scale);

            draw_line(
                hdc,
                title_f,
                TITLE,
                RECT {
                    left: ix,
                    top: sc(30, scale),
                    right: rc.right - pad,
                    bottom: sc(76, scale),
                },
                "PeekGuard",
            );
            draw_line(
                hdc,
                ver_f,
                MUTED,
                RECT {
                    left: ix,
                    top: sc(78, scale),
                    right: rc.right - pad,
                    bottom: sc(106, scale),
                },
                &format!("版本 {VERSION}"),
            );

            let rule = CreateSolidBrush(RULE);
            FillRect(
                hdc,
                &RECT {
                    left: pad,
                    top: sc(122, scale),
                    right: rc.right - pad,
                    bottom: sc(124, scale),
                },
                rule,
            );
            DeleteObject(rule);

            draw_line(
                hdc,
                body_f,
                BODY,
                RECT {
                    left: pad,
                    top: sc(134, scale),
                    right: rc.right - pad,
                    bottom: sc(164, scale),
                },
                "绑定任意窗口，鼠标移开自动隐身，移回自动恢复。",
            );

            let rows = [
                ("Ctrl+Alt+P", "选择并绑定窗口"),
                ("Ctrl+Alt+H", "立即隐身 / 恢复"),
                ("Ctrl+Alt+L", "锁存模式"),
            ];
            let mut y = 178;
            for (key, desc) in rows {
                draw_line(
                    hdc,
                    body_f,
                    BODY,
                    RECT {
                        left: pad,
                        top: sc(y, scale),
                        right: sc(PAD + 150, scale),
                        bottom: sc(y + 30, scale),
                    },
                    key,
                );
                draw_line(
                    hdc,
                    body_f,
                    BODY,
                    RECT {
                        left: sc(PAD + 160, scale),
                        top: sc(y, scale),
                        right: rc.right - pad,
                        bottom: sc(y + 30, scale),
                    },
                    desc,
                );
                y += 36;
            }

            // link, underlined (the font carries the underline)
            draw_line(hdc, link_f, LINK, link_rect(hdc, scale), URL_LABEL);
            draw_line(
                hdc,
                hint_f,
                MUTED,
                RECT {
                    left: rc.right - pad - sc(190, scale),
                    top: sc(300, scale),
                    right: rc.right - pad,
                    bottom: sc(328, scale),
                },
                "Esc 或点关闭按钮退出",
            );

            DeleteObject(title_f);
            DeleteObject(ver_f);
            DeleteObject(body_f);
            DeleteObject(hint_f);
            DeleteObject(link_f);

            EndPaint(hwnd, &ps);
            0
        }
        WM_SETCURSOR => {
            // LOWORD(lParam) == HTCLIENT
            if (l & 0xFFFF) == 1 {
                let hit = cursor_over_link(hwnd, cursor_pos());
                let cur = LoadCursorW(
                    std::ptr::null_mut(),
                    (if hit { IDC_HAND } else { IDC_ARROW }) as usize as *const u16,
                );
                SetCursor(cur);
                return 1;
            }
            DefWindowProcW(hwnd, msg, w, l)
        }
        WM_LBUTTONUP => {
            // lParam carries client-area coordinates for mouse messages.
            let x = (l & 0xFFFF) as i16 as i32;
            let y = ((l >> 16) & 0xFFFF) as i16 as i32;
            if point_in(link_rect_client(hwnd), x, y) {
                let op = to_wide("open");
                let file = to_wide(URL);
                ShellExecuteW(
                    hwnd,
                    op.as_ptr(),
                    file.as_ptr(),
                    std::ptr::null(),
                    std::ptr::null(),
                    SW_SHOWNORMAL,
                );
            }
            0
        }
        WM_KEYDOWN => {
            if w as i32 == VK_ESCAPE {
                DestroyWindow(hwnd);
            }
            0
        }
        _ => DefWindowProcW(hwnd, msg, w, l),
    }
}
