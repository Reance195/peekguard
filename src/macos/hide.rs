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
use crate::hide_common::{self, Entry};

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
        let pid = process_id(s.hwnd);
        hide_common::format(
            s.hwnd as usize,
            self.mode,
            (s.rect.left, s.rect.top, s.rect.right, s.rect.bottom),
            &[("opacity", hide_common::number_or_none(s.opacity))],
            pid,
            &process_name(pid),
        )
    }
}

/// Hide a window with the given strategy.  On success the window is
/// guaranteed to be recorded in the crash-recovery journal.
pub fn hide(hwnd: HWND, mode: HideMode, _alpha: u8) -> Option<Hidden> {
    let snap = Snapshot::capture(hwnd)?;
    let hidden = Hidden { snap, mode };
    crate::session::write(std::slice::from_ref(&hidden));

    if hide_common::refuse_hide_hook() {
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

/// Parse one journal line produced by `journal_line`.  Returns `None` for
/// stale entries (window gone, or the pid/process pair no longer matches,
/// guarding against id reuse) so `session::replay` just skips them.
pub fn from_journal_line(line: &str) -> Option<Hidden> {
    let entry = Entry::parse(line);
    let hwnd = entry.hwnd()? as HWND;
    if hwnd == 0 || !is_valid(hwnd) {
        return None;
    }

    // Guard against window id reuse: the process must still match.
    let actual_pid = process_id(hwnd);
    if !entry.same_process(actual_pid, &process_name(actual_pid)) {
        return None;
    }

    let mode = entry.mode();
    let rect = entry
        .rect()
        .map(|(left, top, right, bottom)| RECT {
            left,
            top,
            right,
            bottom,
        })
        .unwrap_or_else(|| window_rect(hwnd));
    let opacity = entry.number("opacity");

    Some(Hidden {
        snap: Snapshot {
            hwnd,
            rect,
            opacity,
        },
        mode,
    })
}
