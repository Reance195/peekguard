//! Hiding a window with the Accessibility API, plus the crash-recovery
//! journal helpers `journal_line()` / `from_journal_line()`.
//!
//! Two strategies exist on macOS:
//!
//! * `Move` — park the window just off the right edge of the virtual screen
//!   by setting its AX position.
//! * `Hide` — minimize it (`kAXMinimized`), which lands it in the Dock.
//!
//! `Alpha` has no public cross-application equivalent on macOS, so a hide in
//! that mode fails cleanly and the caller reports it.

use super::*;
use crate::config::HideMode;

/// Everything worth remembering before a window is hidden.
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
            opacity: None,
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
        let h = self.snap.hwnd;
        if !is_valid(h) {
            return;
        }
        match self.mode {
            // Alpha hides are refused before this point, so no snapshot can
            // be in that mode; nothing to undo.
            HideMode::Alpha => {}
            HideMode::Move => {
                ax_set_position(h, self.snap.rect.left as f64, self.snap.rect.top as f64);
            }
            HideMode::Hide => {
                ax_set_minimized(h, false);
            }
        }
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

/// Hide a window with the given strategy.  On success the window is
/// guaranteed to be recorded in the crash-recovery journal.
pub fn hide(hwnd: HWND, mode: HideMode, _alpha: u8) -> Option<Hidden> {
    let snap = Snapshot::capture(hwnd)?;
    let hidden = Hidden { snap, mode };
    crate::session::write(std::slice::from_ref(&hidden));

    if refuse_hide_hook() {
        crate::session::write(&[]);
        return None;
    }

    let ok = match mode {
        // No public transparency for foreign windows: fail so the caller
        // reports it instead of claiming a hide that never happened.
        HideMode::Alpha => false,
        HideMode::Move => {
            let park_x = virtual_screen().right as f64 + 64.0;
            ax_set_position(hwnd, park_x, hidden.snap.rect.top as f64)
        }
        HideMode::Hide => ax_set_minimized(hwnd, true),
    };

    if ok {
        Some(hidden)
    } else {
        crate::session::write(&[]);
        None
    }
}

/// macOS has no per-window elevation flag, so nothing can get in the way.
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
