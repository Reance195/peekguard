//! Crash recovery.
//!
//! If PeekGuard dies while a window is invisible, the user is left with an
//! invisible window and no obvious way to get it back. So every hide is
//! journalled to disk and replayed (reverted) on next start.
//!
//! The ordering matters and is the reason `write` is called *before* the window is
//! touched rather than after: see the comment in `hide::hide`. In short, the only
//! unrecoverable state is "invisible with no journal entry", so the entry is always
//! written first. Extra entries are harmless — replaying a visible window is a no-op.

use crate::config::HideMode;
use crate::hide::{Hidden, Snapshot};
use crate::win::*;
use std::sync::{Mutex, MutexGuard};

/// What we believe is currently on disk, as (hwnd, mode) pairs.
///
/// A single hide rewrites the journal twice — once inside `hide` (which is
/// load-bearing for crash safety and must stay) and once more from the tick loop
/// with the authoritative set. Both are synchronous disk writes on the critical
/// path, right between the user moving the mouse and the window disappearing.
/// Remembering the last signature turns every redundant call into a no-op.
///
/// Only recorded after a write actually succeeded, so a transient I/O failure is
/// retried rather than silently believed.
static LAST_WRITTEN: Mutex<Option<Vec<(usize, &'static str)>>> = Mutex::new(None);

fn signature(hidden: &[Hidden]) -> Vec<(usize, &'static str)> {
    hidden
        .iter()
        .map(|h| (h.snap.hwnd as usize, h.mode.as_str()))
        .collect()
}

fn last_written() -> MutexGuard<'static, Option<Vec<(usize, &'static str)>>> {
    LAST_WRITTEN.lock().unwrap_or_else(|e| e.into_inner())
}

/// Forget what we think is on disk. Called by `replay`, which deletes the file
/// behind our back, and by tests that swap the journal out from under us.
pub fn invalidate_cache() {
    *last_written() = None;
}

fn journal_path() -> std::path::PathBuf {
    let mut p = crate::config::Config::config_dir();
    let _ = std::fs::create_dir_all(&p);
    p.push("hidden.journal");
    p
}

fn fmt_rect(r: &RECT) -> String {
    format!("{},{},{},{}", r.left, r.top, r.right, r.bottom)
}

fn parse_rect(s: &str) -> Option<RECT> {
    let v: Vec<i32> = s.split(',').filter_map(|x| x.trim().parse().ok()).collect();
    if v.len() != 4 {
        return None;
    }
    Some(RECT {
        left: v[0],
        top: v[1],
        right: v[2],
        bottom: v[3],
    })
}

/// Rewrite the journal with exactly the given set of hidden windows.
pub fn write(hidden: &[Hidden]) {
    let sig = signature(hidden);
    if last_written().as_deref() == Some(sig.as_slice()) {
        return;
    }
    let path = journal_path();
    let result: std::io::Result<()> = if hidden.is_empty() {
        match std::fs::remove_file(&path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            other => other.map(|_| ()),
        }
    } else {
        let mut out = String::new();
        for h in hidden {
            let s = &h.snap;
            out.push_str(&format!(
                "hwnd={:#x} mode={} rect={} alpha={} layered={} exstyle={:#x} pid={} proc={}\n",
                s.hwnd as usize,
                h.mode.as_str(),
                fmt_rect(&s.rect),
                s.alpha
                    .map(|a| a.to_string())
                    .unwrap_or_else(|| "none".into()),
                if s.was_layered { 1 } else { 0 },
                s.ex_style,
                process_id(s.hwnd),
                process_name(process_id(s.hwnd)),
            ));
        }
        std::fs::write(path, out)
    };
    if result.is_ok() {
        *last_written() = Some(sig);
    }
}

fn get<'a>(fields: &'a [(&str, String)], key: &str) -> Option<&'a str> {
    fields
        .iter()
        .find(|(k, _)| *k == key)
        .map(|(_, v)| v.as_str())
}

/// Restore anything left over from a previous run. Returns how many were fixed.
pub fn replay() -> usize {
    // We are about to delete the file, so whatever we thought was on disk is gone.
    invalidate_cache();
    let path = journal_path();
    let Ok(text) = std::fs::read_to_string(&path) else {
        return 0;
    };
    let mut fixed = 0usize;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let mut fields: Vec<(&str, String)> = Vec::new();
        for part in line.split_whitespace() {
            if let Some((k, v)) = part.split_once('=') {
                fields.push((k, v.to_string()));
            }
        }
        let Some(hwnd_str) = get(&fields, "hwnd") else {
            continue;
        };
        let Ok(raw) = usize::from_str_radix(hwnd_str.trim_start_matches("0x"), 16) else {
            continue;
        };
        let hwnd = raw as HWND;
        if !is_valid(hwnd) {
            continue; // window is gone; nothing to do
        }
        // If the process restarted, the HWND may have been recycled: check the pid.
        if let (Some(pid_s), Some(proc_s)) = (get(&fields, "pid"), get(&fields, "proc")) {
            if let Ok(pid) = pid_s.parse::<u32>() {
                let actual = process_id(hwnd);
                let actual_name = process_name(actual);
                if actual != pid && !actual_name.eq_ignore_ascii_case(proc_s) {
                    continue;
                }
            }
        }
        let mode = get(&fields, "mode")
            .and_then(HideMode::parse)
            .unwrap_or(HideMode::Alpha);
        let rect = get(&fields, "rect")
            .and_then(parse_rect)
            .unwrap_or_else(|| window_rect(hwnd));
        let alpha = get(&fields, "alpha").and_then(|v| v.parse::<u8>().ok());
        let was_layered = get(&fields, "layered").map(|v| v == "1").unwrap_or(false);
        let ex_style = get(&fields, "exstyle")
            .and_then(|v| isize::from_str_radix(v.trim_start_matches("0x"), 16).ok())
            .unwrap_or_else(|| ex_style(hwnd));

        let hidden = Hidden {
            mode,
            snap: Snapshot {
                hwnd,
                rect,
                alpha,
                was_layered,
                ex_style,
                was_minimized: false,
            },
        };
        hidden.revert();
        fixed += 1;
    }
    let _ = std::fs::remove_file(&path);
    fixed
}
