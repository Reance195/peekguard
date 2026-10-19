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
//!
//! The journal line format itself is platform-specific and lives next to the
//! hide implementation (`plat::hide::journal_line` / `from_journal_line`); this
//! module only decides *when* to write and replay.

use crate::plat::hide::{from_journal_line, Hidden};
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
            out.push_str(&h.journal_line());
        }
        std::fs::write(path, out)
    };
    if result.is_ok() {
        *last_written() = Some(sig);
    }
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
        if let Some(hidden) = from_journal_line(line) {
            hidden.revert();
            fixed += 1;
        }
    }
    let _ = std::fs::remove_file(&path);
    fixed
}
