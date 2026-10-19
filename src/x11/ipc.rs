//! Unix domain socket IPC server for Linux — the counterpart of the Windows
//! named pipe in `src/ipc.rs`.
//!
//! Protocol is identical to Windows: one line of text goes in, one line of
//! JSON comes out. The listener runs on a background thread; requests are
//! handed to the main tick loop through an mpsc channel (adding at most one
//! tick of latency) instead of the Windows PostMessage + event pair.

use std::io::{BufRead, Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Mutex;
use std::time::Duration;

/// Matches the Windows pipe server: give up after half a minute.
const RESPONSE_TIMEOUT_MS: u64 = 30_000;
/// Matches the Windows pipe server: a request line may not exceed 64 KiB.
const MAX_REQUEST_BYTES: u64 = 64 * 1024;

/// Escape a string for embedding in a hand-built JSON document.
pub fn escape_json(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                use std::fmt::Write as _;
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out
}

pub fn err_json(msg: &str) -> String {
    format!(r#"{{"ok":false,"err":"{}"}}"#, escape_json(msg))
}

/// Resolve the socket path. `PEEKGUARD_PIPE` may name an absolute path (used
/// as-is) or a bare name (placed in the runtime dir); the default is a
/// per-user socket in `XDG_RUNTIME_DIR` (or `/tmp` as a fallback).
fn socket_path() -> PathBuf {
    match std::env::var("PEEKGUARD_PIPE") {
        Ok(v) if !v.trim().is_empty() => {
            let v = v.trim();
            if v.starts_with('/') {
                PathBuf::from(v)
            } else {
                super::runtime_dir().join(format!("{v}.sock"))
            }
        }
        _ => super::runtime_dir().join("peekguard-ipc.sock"),
    }
}

/// Queue of pending requests, consumed by [`drain`] from the main loop.
type PendingRequest = (String, Sender<String>);

static REQUEST_RX: Mutex<Option<Receiver<PendingRequest>>> = Mutex::new(None);

/// Start the background listener. Best effort: if the socket cannot be
/// created the hotkeys and the tick loop still work, only remote control is
/// unavailable.
pub fn start() {
    let path = socket_path();
    let (tx, rx) = mpsc::channel::<PendingRequest>();
    *REQUEST_RX.lock().unwrap_or_else(|e| e.into_inner()) = Some(rx);

    std::thread::spawn(move || {
        // A socket file left over from a crash would make bind() fail.
        let _ = std::fs::remove_file(&path);
        let listener = match UnixListener::bind(&path) {
            Ok(l) => l,
            Err(e) => {
                eprintln!("[peekguard] IPC: 无法监听 {}：{e}", path.display());
                return;
            }
        };
        // The runtime dir is per-user, but the /tmp fallback is shared;
        // restrict the socket to its owner either way.
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
        for conn in listener.incoming() {
            let Ok(stream) = conn else { continue };
            let tx = tx.clone();
            std::thread::spawn(move || handle_conn(stream, tx));
        }
    });
}

fn handle_conn(stream: UnixStream, tx: Sender<(String, Sender<String>)>) {
    let Ok(read_half) = stream.try_clone() else {
        return;
    };
    // take() guards against an unterminated flood; a truncated line simply
    // resolves to an unknown command instead of eating memory.
    let mut reader = std::io::BufReader::new(read_half).take(MAX_REQUEST_BYTES);
    let mut line = String::new();
    if reader.read_line(&mut line).is_err() || line.is_empty() {
        return;
    }
    let cmd = line.trim_end_matches(['\r', '\n']).to_string();
    let (resp_tx, resp_rx) = mpsc::channel::<String>();
    if tx.send((cmd, resp_tx)).is_err() {
        return;
    }
    let resp = resp_rx
        .recv_timeout(Duration::from_millis(RESPONSE_TIMEOUT_MS))
        .unwrap_or_else(|_| err_json("IPC 请求处理超时"));
    let mut writer = stream;
    let _ = writer.write_all(resp.as_bytes());
    let _ = writer.write_all(b"\n");
    let _ = writer.flush();
}

/// Called from the main loop once per tick: executes queued IPC commands and
/// ships the responses back to the waiting connections.
pub fn drain<F: FnMut(&str) -> String>(mut execute: F) {
    let guard = REQUEST_RX.lock().unwrap_or_else(|e| e.into_inner());
    let Some(rx) = guard.as_ref() else {
        return;
    };
    while let Ok((cmd, resp_tx)) = rx.try_recv() {
        let resp = execute(&cmd);
        let _ = resp_tx.send(resp);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_quotes_and_controls() {
        assert_eq!(escape_json(r#"a"b\c"#), r#"a\"b\\c"#);
        assert_eq!(escape_json("line\nbreak\t."), "line\\nbreak\\t.");
        assert_eq!(escape_json("\u{1}"), "\\u0001");
    }

    #[test]
    fn keeps_unicode_verbatim() {
        assert_eq!(escape_json("已隐身"), "已隐身");
    }

    #[test]
    fn err_json_wraps_escaped_message() {
        assert_eq!(
            err_json(r#"say "hi""#),
            r#"{"ok":false,"err":"say \"hi\""}"#
        );
    }

    #[test]
    fn default_socket_name_ends_with_sock() {
        if std::env::var("PEEKGUARD_PIPE").is_ok() {
            return;
        }
        assert!(socket_path().to_string_lossy().ends_with(".sock"));
    }
}
