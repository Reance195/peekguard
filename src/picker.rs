//! Crosshair window picker.
//!
//! `Ctrl+Alt+P` (and the tray menu) open this instead of grabbing the window
//! that happens to be in front: a bright frame tracks whatever is under the
//! cursor, the pointer turns into a crosshair, and a left click binds the
//! highlighted top-level window. Right-click or Esc cancels.
//!
//! The frame is a topmost, click-through, colour-keyed layered popup, so it
//! never steals focus and is invisible except for its border. Mouse input is
//! captured by it for the duration, so the click that selects a window is not
//! also delivered to that window.
#![allow(non_snake_case)]

use crate::win::*;

const FRAME_CLASS: &str = "PeekGuard.Picker.v1";

/// Colour key for the frame interior: a colour nothing else on screen uses.
const KEY: COLORREF = 0x00FF_00FF; // magenta
/// Frame border, chosen to read over both light and dark window chrome.
const BORDER: COLORREF = 0x0000_50FF; // orange-red
const BORDER_PX: i32 = 3;

/// Clicks in the first moments after opening are ignored: the same key press
/// that started the picker can still have the button held down.
const GRACE_MS: u64 = 150;

/// What the caller should do after a picker tick.
pub enum Outcome {
    /// Keep picking.
    Continue,
    /// Left click: bind this window.
    Pick(HWND),
    /// Right click or Esc.
    Cancel,
}

pub struct Picker {
    frame: HWND,
    /// The top-level window currently highlighted.
    target: HWND,
    started: u64,
    cursor: HCURSOR,
}

impl Picker {
    /// Create and show the frame. `None` if the window could not be created.
    pub fn open() -> Option<Picker> {
        unsafe {
            let hinst = GetModuleHandleW(std::ptr::null());
            let class = to_wide(FRAME_CLASS);
            let wc = WNDCLASSEXW {
                cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
                lpfnWndProc: Some(frame_proc),
                hInstance: hinst,
                hCursor: LoadCursorW(std::ptr::null_mut(), IDC_CROSS as usize as *const u16),
                lpszClassName: class.as_ptr(),
                ..Default::default()
            };
            RegisterClassExW(&wc);

            let title = to_wide("");
            let frame = CreateWindowExW(
                (WS_EX_TOPMOST
                    | WS_EX_TRANSPARENT
                    | WS_EX_TOOLWINDOW
                    | WS_EX_LAYERED
                    | WS_EX_NOACTIVATE) as u32,
                class.as_ptr(),
                title.as_ptr(),
                WS_POPUP,
                0,
                0,
                0,
                0,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                hinst,
                std::ptr::null_mut(),
            );
            if frame.is_null() {
                return None;
            }
            SetLayeredWindowAttributes(frame, KEY, 0, LWA_COLORKEY);
            ShowWindow(frame, SW_SHOWNA);
            // Swallow mouse input for the duration so the selecting click does
            // not land on the window being selected. Hit-testing is unaffected
            // because the frame is WS_EX_TRANSPARENT.
            SetCapture(frame);
            Some(Picker {
                frame,
                target: std::ptr::null_mut(),
                started: tick_ms(),
                cursor: LoadCursorW(std::ptr::null_mut(), IDC_CROSS as usize as *const u16),
            })
        }
    }

    /// One tick: follow the cursor, then test for a click or a cancel.
    pub fn tick(&mut self, now: u64) -> Outcome {
        if key_down(VK_ESCAPE) || key_down(VK_RBUTTON) {
            return Outcome::Cancel;
        }
        if !self.cursor.is_null() {
            unsafe {
                SetCursor(self.cursor);
            }
        }

        let under = root_at(cursor_pos());
        if is_valid(under) && !is_own_window(under) && under != self.target {
            self.target = under;
            self.fit(under);
        }

        if now >= self.started + GRACE_MS && key_down(VK_LBUTTON) && is_valid(self.target) {
            return Outcome::Pick(self.target);
        }
        Outcome::Continue
    }

    /// Tear the frame down (destroy releases the capture).
    pub fn close(&mut self) {
        if !self.frame.is_null() {
            unsafe {
                ReleaseCapture();
                DestroyWindow(self.frame);
            }
            self.frame = std::ptr::null_mut();
        }
    }

    /// Move the frame to wrap `hwnd` with a small outward margin.
    fn fit(&self, hwnd: HWND) {
        let r = window_rect(hwnd);
        if r.is_empty() {
            return;
        }
        let m = 1;
        unsafe {
            SetWindowPos(
                self.frame,
                HWND_TOPMOST,
                r.left - m,
                r.top - m,
                r.width() + 2 * m,
                r.height() + 2 * m,
                SWP_NOACTIVATE,
            );
            InvalidateRect(self.frame, std::ptr::null(), 0);
        }
    }
}

impl Drop for Picker {
    fn drop(&mut self) {
        self.close();
    }
}

fn key_down(vk: i32) -> bool {
    unsafe { (GetAsyncKeyState(vk) as u16 & 0x8000) != 0 }
}

/// Top-level window under `p`, or null. Our own windows are filtered out.
fn root_at(p: POINT) -> HWND {
    unsafe {
        let h = WindowFromPoint(p);
        if h.is_null() {
            return std::ptr::null_mut();
        }
        let root = GetAncestor(h, GA_ROOT);
        if root.is_null() {
            h
        } else {
            root
        }
    }
}

fn is_own_window(h: HWND) -> bool {
    class_name(h).starts_with("PeekGuard.")
}

unsafe extern "system" fn frame_proc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    match msg {
        WM_ERASEBKGND => 1,
        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            let hdc = BeginPaint(hwnd, &mut ps);
            let mut rc = RECT::default();
            GetClientRect(hwnd, &mut rc);
            let key = CreateSolidBrush(KEY);
            FillRect(hdc, &rc, key);
            DeleteObject(key);

            let pen = CreateSolidBrush(BORDER);
            // Four edges, so the border is `BORDER_PX` thick on every side.
            let edges = [
                RECT {
                    left: 0,
                    top: 0,
                    right: rc.right,
                    bottom: BORDER_PX,
                },
                RECT {
                    left: 0,
                    top: rc.bottom - BORDER_PX,
                    right: rc.right,
                    bottom: rc.bottom,
                },
                RECT {
                    left: 0,
                    top: 0,
                    right: BORDER_PX,
                    bottom: rc.bottom,
                },
                RECT {
                    left: rc.right - BORDER_PX,
                    top: 0,
                    right: rc.right,
                    bottom: rc.bottom,
                },
            ];
            for e in edges {
                FillRect(hdc, &e, pen);
            }
            DeleteObject(pen);

            EndPaint(hwnd, &ps);
            0
        }
        _ => DefWindowProcW(hwnd, msg, w, l),
    }
}
