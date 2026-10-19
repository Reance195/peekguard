//! Hiding strategies for X11 windows.
//!
//! This mirrors the public surface of the Windows implementation in
//! `crate::hide`: `hide()`, `owner_elevated()`, `Hidden`/`Snapshot` and the
//! crash-recovery journal helpers `journal_line()` / `from_journal_line()`.
//!
//! Three strategies map onto EWMH conventions:
//!
//! * `Alpha` – set `_NET_WM_WINDOW_OPACITY` to the configured alpha.  Only
//!   meaningful while a compositing manager runs (detected through the
//!   `_NET_WM_CM_S0` selection); without one the attempt is refused so the
//!   watcher stops instead of pretending the window is hidden.
//! * `Move`  – park the window just beyond the right edge of the screen.
//! * `Hide`  – `XUnmapWindow`; the window manager also drops it from the
//!   taskbar, which roughly matches the Windows `SW_HIDE` semantics.
//!
//! Invariant (same as the Windows side): the journal line is written
//! *before* the window is touched, so a crash in between can never leave an
//! invisible window unrecorded.

use std::os::raw::{c_long, c_uchar};

use super::*;
use crate::config::HideMode;

/// Everything needed to bring one window back from a hidden state.
#[derive(Clone, Debug)]
pub struct Snapshot {
    pub hwnd: HWND,
    pub rect: RECT,
    pub opacity: Option<u64>,
}

impl Snapshot {
    /// Capture the state of `hwnd` before it is hidden.  Returns `None` for
    /// windows that are gone or currently minimized (nothing to hide).
    pub fn capture(hwnd: HWND) -> Option<Snapshot> {
        if !is_valid(hwnd) {
            return None;
        }
        if is_minimized(hwnd) {
            return None;
        }
        Some(Snapshot {
            hwnd,
            rect: window_rect(hwnd),
            opacity: prop_u64(hwnd, atoms().NET_WM_WINDOW_OPACITY),
        })
    }
}

/// A hidden window plus everything needed to restore it.
#[derive(Clone, Debug)]
pub struct Hidden {
    pub snap: Snapshot,
    pub mode: HideMode,
}

impl Hidden {
    /// Undo the hide.  Idempotent, and a no-op once the window is gone.
    pub fn revert(&self) {
        let Ok(x) = lib() else { return };
        let h = self.snap.hwnd;
        if !is_valid(h) {
            return;
        }
        match self.mode {
            HideMode::Alpha => unsafe {
                match self.snap.opacity {
                    Some(v) => {
                        let value = v as c_long;
                        (x.XChangeProperty)(
                            dpy(),
                            h,
                            atoms().NET_WM_WINDOW_OPACITY,
                            atoms().CARDINAL,
                            32,
                            0,
                            &value as *const c_long as *const c_uchar,
                            1,
                        );
                    }
                    None => {
                        (x.XDeleteProperty)(dpy(), h, atoms().NET_WM_WINDOW_OPACITY);
                    }
                }
            },
            HideMode::Move => unsafe {
                (x.XMoveWindow)(dpy(), h, self.snap.rect.left, self.snap.rect.top);
            },
            HideMode::Hide => unsafe {
                (x.XMapWindow)(dpy(), h);
            },
        }
        flush();
    }

    /// One line for the crash-recovery journal, consumed by
    /// `from_journal_line` after a restart.
    pub fn journal_line(&self) -> String {
        let s = &self.snap;
        format!(
            "hwnd={:#x} mode={} rect={},{},{},{} opacity={} pid={} proc={}\n",
            s.hwnd as usize,
            self.mode.as_str(),
            s.rect.left,
            s.rect.top,
            s.rect.right,
            s.rect.bottom,
            s.opacity
                .map(|v| v.to_string())
                .unwrap_or_else(|| "none".into()),
            process_id(s.hwnd),
            process_name(process_id(s.hwnd)),
        )
    }
}

/// Test hook: `PEEKGUARD_REFUSE_HIDE=1` makes `hide()` fail like a window
/// that refuses to be hidden.
fn refuse_hide_hook() -> bool {
    std::env::var("PEEKGUARD_REFUSE_HIDE")
        .map(|v| v == "1")
        .unwrap_or(false)
}

/// A compositing manager must own the `_NET_WM_CM_S0` selection for the
/// opacity property to have any visible effect.
fn compositor_running() -> bool {
    let Ok(x) = lib() else { return false };
    unsafe { (x.XGetSelectionOwner)(dpy(), atoms().NET_WM_CM_S0) != 0 }
}

/// Map a 0-255 alpha onto the 0..=0xffffffff range used by
/// `_NET_WM_WINDOW_OPACITY`.
fn opacity_value(alpha: u8) -> u64 {
    alpha as u64 * 0xffff_ffffu64 / 255
}

/// Hide a window with the given strategy.  On success the window is
/// guaranteed to be recorded in the crash-recovery journal.
pub fn hide(hwnd: HWND, mode: HideMode, alpha: u8) -> Option<Hidden> {
    let snap = Snapshot::capture(hwnd)?;
    let park_y = snap.rect.top;
    let hidden = Hidden { snap, mode };
    crate::session::write(std::slice::from_ref(&hidden));

    if refuse_hide_hook() {
        crate::session::write(&[]);
        return None;
    }

    let Ok(x) = lib() else {
        crate::session::write(&[]);
        return None;
    };

    let ok = match mode {
        HideMode::Alpha => {
            if !compositor_running() {
                false
            } else {
                let value = opacity_value(alpha) as c_long;
                unsafe {
                    (x.XChangeProperty)(
                        dpy(),
                        hwnd,
                        atoms().NET_WM_WINDOW_OPACITY,
                        atoms().CARDINAL,
                        32,
                        0,
                        &value as *const c_long as *const c_uchar,
                        1,
                    );
                }
                // Read back to prove the property actually stuck.
                prop_u64(hwnd, atoms().NET_WM_WINDOW_OPACITY)
                    .map(|v| v == opacity_value(alpha))
                    .unwrap_or(false)
            }
        }
        HideMode::Move => {
            let park_x = virtual_screen().right + 64;
            unsafe { (x.XMoveWindow)(dpy(), hwnd, park_x, park_y) };
            true
        }
        HideMode::Hide => {
            unsafe { (x.XUnmapWindow)(dpy(), hwnd) };
            true
        }
    };

    flush();

    if ok {
        Some(hidden)
    } else {
        crate::session::write(&[]);
        None
    }
}

/// X11 has no per-window elevation flag, so nothing can get in the way.
pub fn owner_elevated(_hwnd: HWND) -> bool {
    false
}

fn parse_rect(s: &str) -> Option<RECT> {
    let mut it = s.split(',');
    let left = it.next()?.parse().ok()?;
    let top = it.next()?.parse().ok()?;
    let right = it.next()?.parse().ok()?;
    let bottom = it.next()?.parse().ok()?;
    Some(RECT {
        left,
        top,
        right,
        bottom,
    })
}

fn field<'a>(fields: &'a [(&str, &str)], key: &str) -> Option<&'a str> {
    fields.iter().find(|(k, _)| *k == key).map(|(_, v)| *v)
}

/// Parse one journal line produced by `journal_line`.  Returns `None` for
/// stale entries (window gone, or the pid/process pair no longer matches,
/// guarding against id reuse) so `session::replay` just skips them.
pub fn from_journal_line(line: &str) -> Option<Hidden> {
    let fields: Vec<(&str, &str)> = line
        .split_whitespace()
        .filter_map(|kv| kv.split_once('='))
        .collect();

    let hwnd_hex = field(&fields, "hwnd")?;
    let hwnd = u64::from_str_radix(hwnd_hex.trim_start_matches("0x"), 16).ok()?;
    if hwnd == 0 || !is_valid(hwnd) {
        return None;
    }

    // Guard against window id reuse: the process must still match.
    if let Some(pid_raw) = field(&fields, "pid") {
        if let Some(proc_raw) = field(&fields, "proc") {
            if let Ok(pid) = pid_raw.parse::<u32>() {
                let actual_pid = process_id(hwnd);
                let actual_name = process_name(actual_pid);
                if actual_pid != pid && !actual_name.eq_ignore_ascii_case(proc_raw) {
                    return None;
                }
            }
        }
    }

    let mode = field(&fields, "mode")
        .and_then(HideMode::parse)
        .unwrap_or(HideMode::Alpha);
    let rect = field(&fields, "rect")
        .and_then(parse_rect)
        .unwrap_or_else(|| window_rect(hwnd));
    let opacity = field(&fields, "opacity").and_then(|v| v.parse::<u64>().ok());

    Some(Hidden {
        snap: Snapshot {
            hwnd,
            rect,
            opacity,
        },
        mode,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opacity_maps_full_range() {
        assert_eq!(opacity_value(0), 0);
        assert_eq!(opacity_value(255), 0xffff_ffff);
        assert_eq!(opacity_value(128), 128 * 0xffff_ffff / 255);
    }

    #[test]
    fn parse_rect_reads_four_numbers() {
        let r = parse_rect("10,20,30,40").unwrap();
        assert_eq!((r.left, r.top, r.right, r.bottom), (10, 20, 30, 40));
        assert!(parse_rect("1,2,3").is_none());
        assert!(parse_rect("a,b,c,d").is_none());
    }

    #[test]
    fn field_lookup_works() {
        let fields = [("hwnd", "0x2a"), ("mode", "alpha")];
        assert_eq!(field(&fields, "hwnd"), Some("0x2a"));
        assert_eq!(field(&fields, "missing"), None);
    }
}
