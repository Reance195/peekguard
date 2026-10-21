//! Hide primitives: snapshot a window's state, then hide/restore it losslessly.
//!
//! Three strategies, all verified working cross-process on this machine:
//!   Alpha — add WS_EX_LAYERED, set alpha to configured value. Click-through preserved.
//!   Move  — SetWindowPos the window past the virtual screen edge.
//!   Hide  — ShowWindow(SW_HIDE) / SW_SHOWNA.
//!
//! DWM cloak (DWMWA_CLOAK) is deliberately NOT used: it returns E_ACCESSDENIED
//! for windows owned by other processes. Verified on this machine.

use crate::config::HideMode;
use crate::hide_common::{self, Entry};
use crate::win::*;
use std::ffi::c_void;

/// Everything needed to put the window back exactly as we found it.
#[derive(Clone, Debug)]
pub struct Snapshot {
    pub hwnd: HWND,
    pub rect: RECT,
    pub alpha: Option<u8>,
    pub was_layered: bool,
    pub ex_style: isize,
    pub was_minimized: bool,
}

impl Snapshot {
    pub fn capture(hwnd: HWND) -> Option<Snapshot> {
        if !is_valid(hwnd) {
            return None;
        }
        Some(Snapshot {
            hwnd,
            rect: window_rect(hwnd),
            alpha: layered_alpha(hwnd),
            was_layered: ex_style(hwnd) & WS_EX_LAYERED != 0,
            ex_style: ex_style(hwnd),
            was_minimized: is_minimized(hwnd),
        })
    }
}

/// One hidden window plus how we hid it.
#[derive(Clone, Debug)]
pub struct Hidden {
    pub snap: Snapshot,
    pub mode: HideMode,
}

impl Hidden {
    /// Put the window back. Idempotent: safe to call twice.
    pub fn revert(&self) {
        let h = self.snap.hwnd;
        if !is_valid(h) {
            return;
        }
        match self.mode {
            HideMode::Alpha => {
                let restore_alpha = self.snap.alpha.unwrap_or(255);
                set_layered_alpha(h, restore_alpha);
                if !self.snap.was_layered {
                    // remove the bit we added, but only after the alpha is back
                    let cur = ex_style(h);
                    set_ex_style(h, cur & !WS_EX_LAYERED);
                }
            }
            HideMode::Move => {
                let r = self.snap.rect;
                unsafe {
                    SetWindowPos(
                        h,
                        std::ptr::null_mut(),
                        r.left,
                        r.top,
                        r.width(),
                        r.height(),
                        SWP_NOZORDER | SWP_NOACTIVATE,
                    );
                }
            }
            HideMode::Hide => {
                if self.snap.was_minimized {
                    unsafe {
                        ShowWindow(h, SW_SHOWMINNOACTIVE);
                    }
                } else {
                    unsafe {
                        ShowWindow(h, SW_SHOWNA);
                    }
                }
            }
        }
    }

    /// One line of the crash-recovery journal, newline included.
    /// The mirror parser is `from_journal_line`; keep the two in sync.
    pub fn journal_line(&self) -> String {
        let s = &self.snap;
        let pid = process_id(s.hwnd);
        hide_common::format(
            s.hwnd as usize,
            self.mode,
            (s.rect.left, s.rect.top, s.rect.right, s.rect.bottom),
            &[
                ("alpha", hide_common::number_or_none(s.alpha)),
                (
                    "layered",
                    if s.was_layered {
                        "1".to_string()
                    } else {
                        "0".to_string()
                    },
                ),
                ("exstyle", format!("{:#x}", s.ex_style)),
            ],
            pid,
            &process_name(pid),
        )
    }
}

pub fn hide(hwnd: HWND, mode: HideMode, alpha: u8) -> Option<Hidden> {
    let snap = Snapshot::capture(hwnd)?;
    if snap.was_minimized {
        return None;
    }
    // Journal the intent *before* the window is touched, not after.
    //
    // The whole point of the journal is that a crash can never leave an invisible
    // window with no record of it. Writing it afterwards leaves exactly that gap: the
    // window is already invisible while the record is still only in memory, so dying
    // in between strands it — the one outcome this file exists to prevent. This was
    // not theoretical; a hard kill landing in that window is how it was caught.
    //
    // Writing first makes the two failure modes asymmetric, which is the point: a
    // crash in between leaves a journal entry for a window that is still visible, and
    // "restoring" that is a harmless no-op, whereas the reverse is unrecoverable.
    // A hide that fails outright is also covered — the caller rewrites the
    // authoritative set afterwards, so the entry is cleaned up again.
    let hidden = Hidden { snap, mode };
    crate::session::write(std::slice::from_ref(&hidden));
    // Deliberately after the journal write, so the hook behaves like the real failure
    // it stands in for. A genuinely elevated target fails *inside* the hide call
    // (SetWindowLongPtrW refuses), i.e. after the entry has been written; short-
    // circuiting earlier would leave the cleanup path untested.
    if hide_common::refuse_hide_hook() {
        crate::session::write(&[]);
        return None;
    }
    let ok = match mode {
        HideMode::Alpha => hide_alpha(hwnd, &hidden.snap, alpha),
        HideMode::Move => hide_move(hwnd, &hidden.snap),
        HideMode::Hide => hide_showwindow(hwnd),
    };
    if ok {
        Some(hidden)
    } else {
        crate::session::write(&[]);
        None
    }
}

fn hide_alpha(hwnd: HWND, snap: &Snapshot, alpha: u8) -> bool {
    let _ = snap;
    let cur = ex_style(hwnd);
    if cur & WS_EX_LAYERED == 0 {
        // SetWindowLongPtrW returns 0 both on failure and when the old value was 0,
        // so trust the read-back rather than the return value.
        set_ex_style(hwnd, cur | WS_EX_LAYERED);
        if ex_style(hwnd) & WS_EX_LAYERED == 0 {
            // failed — most commonly because the target runs elevated
            return false;
        }
    }
    set_layered_alpha(hwnd, alpha)
}

fn hide_move(hwnd: HWND, snap: &Snapshot) -> bool {
    let vs = virtual_screen();
    // park it just to the right of everything, keeping its size and vertical position
    let park_x = vs.right + 64;
    unsafe {
        SetWindowPos(
            hwnd,
            std::ptr::null_mut(),
            park_x,
            snap.rect.top,
            snap.rect.width(),
            snap.rect.height(),
            SWP_NOZORDER | SWP_NOACTIVATE,
        ) != 0
    }
}

fn hide_showwindow(hwnd: HWND) -> bool {
    unsafe { ShowWindow(hwnd, SW_HIDE) != 0 }
}

/// Whether the process owning a window runs elevated.
///
/// Read-only: opens the target's token and asks. If we cannot even query it
/// (access denied), we assume it is elevated, because that is precisely the
/// situation where our style writes fail.
pub fn owner_elevated(hwnd: HWND) -> bool {
    if !is_valid(hwnd) {
        return false;
    }
    let pid = process_id(hwnd);
    if pid == 0 || pid == std::process::id() {
        return false;
    }
    unsafe {
        let proc = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if proc.is_null() {
            return true; // cannot inspect it: treat as hostile
        }
        let mut token: HANDLE = std::ptr::null_mut();
        if OpenProcessToken(proc, TOKEN_QUERY, &mut token) == 0 {
            CloseHandle(proc);
            return true;
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
        CloseHandle(proc);
        ok == 0 || elevated != 0
    }
}

// ----------------------------------------------------------- journal parsing
// The wire format is shared through `hide_common`; this module only rebuilds
// the Windows snapshot, whose extra fields are alpha/layered/exstyle.

/// Parse one journal line back into a `Hidden`. `None` means the line is
/// malformed, its window is gone, or the HWND was recycled by another process.
pub fn from_journal_line(line: &str) -> Option<Hidden> {
    let entry = Entry::parse(line);
    let hwnd = entry.hwnd()? as HWND;
    if !is_valid(hwnd) {
        return None; // window is gone; nothing to do
    }
    // If the process restarted, the HWND may have been recycled: check the pid.
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
    let alpha = entry.number("alpha");
    let was_layered = entry.get("layered").map(|v| v == "1").unwrap_or(false);
    let ex_style = entry
        .get("exstyle")
        .and_then(|v| isize::from_str_radix(v.trim_start_matches("0x"), 16).ok())
        .unwrap_or_else(|| ex_style(hwnd));
    Some(Hidden {
        mode,
        snap: Snapshot {
            hwnd,
            rect,
            alpha,
            was_layered,
            ex_style,
            was_minimized: false,
        },
    })
}
