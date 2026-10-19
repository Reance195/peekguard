//! Local IPC over a named pipe, so scripts and the uTools plugin can drive
//! PeekGuard while it sits in the tray.
//!
//! Wire protocol — one request per connection, text, newline-framed:
//!
//! ```text
//! client -> server:  "status\n"          (or "hide", "latch on", …)
//! server -> client:  {"ok":true,…}\n     (always a single JSON line)
//! ```
//!
//! The server keeps application state single-threaded: the pipe thread never
//! touches `App`, it only reads a command and posts `WM_IPC_REQUEST` to the
//! message window. The UI thread runs the command, fills the response into the
//! request box and signals the worker's event.
//!
//! Nothing here listens on a network socket. The pipe name is local to this
//! machine and the default security descriptor limits access to the user who
//! started PeekGuard (and administrators). `PEEKGUARD_PIPE` overrides the name
//! so tests can run an isolated instance next to a real one.

use crate::win::*;
use std::ffi::c_void;
use std::ptr;

/// Posted to the message window when a client command is ready. `lParam` is a
/// `*mut Request` allocated by the pipe thread; it stays owned by that thread
/// and is reclaimed only after `event` fires.
pub const WM_IPC_REQUEST: u32 = WM_APP + 3;

/// How long a request may wait for the UI thread. Only ever hit when the
/// message loop itself is wedged (a modal dialog, a hung shell call).
const RESPONSE_TIMEOUT_MS: u32 = 30_000;

/// Reader cap, so a client cannot make the server grow without bound by
/// streaming bytes with no newline.
const MAX_REQUEST_BYTES: usize = 64 * 1024;

/// A request in flight. Allocated on the pipe thread, completed on the UI
/// thread, read back and freed on the pipe thread.
pub struct Request {
    pub cmd: String,
    pub resp: String,
    pub event: HANDLE,
}

/// The pipe the server binds and the plugin dials. `PEEKGUARD_PIPE` makes the
/// name configurable for tests (mirroring `PEEKGUARD_MUTEX`); a bare name gets
/// the `\\.\pipe\` prefix added.
pub fn pipe_name() -> String {
    normalize_pipe_name(std::env::var("PEEKGUARD_PIPE").ok().as_deref())
}

fn normalize_pipe_name(raw: Option<&str>) -> String {
    match raw {
        Some(n) if !n.trim().is_empty() => {
            let n = n.trim();
            if n.starts_with(r"\\.\pipe\") {
                n.to_string()
            } else {
                format!(r"\\.\pipe\{n}")
            }
        }
        _ => r"\\.\pipe\PeekGuard-IPC-v1".to_string(),
    }
}

/// Start the pipe server thread. `hwnd` is the message window that receives
/// `WM_IPC_REQUEST`.
pub fn spawn(hwnd: HWND) {
    let raw = hwnd as usize;
    std::thread::spawn(move || {
        let hwnd = raw as HWND;
        let name = to_wide(&pipe_name());
        loop {
            unsafe {
                let pipe = CreateNamedPipeW(
                    name.as_ptr(),
                    PIPE_ACCESS_DUPLEX,
                    PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT,
                    1, // one client at a time; a second connect waits its turn
                    64 * 1024,
                    64 * 1024,
                    0,
                    ptr::null(),
                );
                if pipe == INVALID_HANDLE_VALUE {
                    // Another process claimed the name; retry rather than spin.
                    Sleep(1000);
                    continue;
                }
                serve(pipe, hwnd);
                let _ = CloseHandle(pipe);
            }
        }
    });
}

/// Accept and answer connections on one pipe instance until the handle breaks,
/// then let the caller rebuild it.
unsafe fn serve(pipe: HANDLE, hwnd: HWND) {
    loop {
        if ConnectNamedPipe(pipe, ptr::null_mut()) == 0 {
            let err = GetLastError();
            if err != ERROR_PIPE_CONNECTED {
                return; // handle is unusable now; rebuild in spawn()
            }
        }
        if let Some(cmd) = read_line(pipe) {
            let resp = dispatch(hwnd, &cmd);
            let mut line = resp;
            line.push('\n');
            write_all(pipe, line.as_bytes());
        }
        let _ = FlushFileBuffers(pipe);
        let _ = DisconnectNamedPipe(pipe);
    }
}

/// Hand the command to the UI thread and wait for its answer.
unsafe fn dispatch(hwnd: HWND, cmd: &str) -> String {
    let event = CreateEventW(ptr::null(), 0, 0, ptr::null());
    if event.is_null() {
        return err_json("无法创建等待事件").to_string();
    }
    let req = Box::into_raw(Box::new(Request {
        cmd: cmd.to_string(),
        resp: String::new(),
        event,
    }));
    if PostMessageW(hwnd, WM_IPC_REQUEST, 0, req as isize) == 0 {
        // No window to hand it to; take the box back and close the event.
        drop(Box::from_raw(req));
        let _ = CloseHandle(event);
        return err_json("PeekGuard 消息窗口不可用").to_string();
    }
    let w = WaitForSingleObject(event, RESPONSE_TIMEOUT_MS);
    if w == WAIT_OBJECT_0 {
        let req = Box::from_raw(req);
        let resp = req.resp.clone();
        drop(req);
        let _ = CloseHandle(event);
        resp
    } else {
        // The UI thread never picked the request up, so it may still write to
        // `*req` and signal the event later. Reclaiming the box — or closing
        // the event handle, whose value could then be recycled — would be a
        // use-after-free. Leak both instead: this path only fires when the
        // message loop is already wedged, and it costs a few dozen bytes.
        crate::dbgmsg!("ipc: UI thread did not answer within {RESPONSE_TIMEOUT_MS}ms");
        err_json("PeekGuard 未响应").to_string()
    }
}

/// Runs on the UI thread (window proc). Fills the response and wakes the
/// worker; the worker reclaims the box afterwards.
pub unsafe fn on_request(l: LPARAM) {
    let req = &mut *(l as *mut Request);
    req.resp = crate::ipc_command(&req.cmd);
    let _ = SetEvent(req.event);
}

/// Read one newline-terminated line from the pipe. `None` when the client
/// disconnects first or the line never ends.
unsafe fn read_line(pipe: HANDLE) -> Option<String> {
    let mut buf: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 512];
    loop {
        let mut got: u32 = 0;
        if ReadFile(
            pipe,
            chunk.as_mut_ptr() as *mut c_void,
            chunk.len() as u32,
            &mut got,
            ptr::null_mut(),
        ) == 0
            || got == 0
        {
            return None;
        }
        buf.extend_from_slice(&chunk[..got as usize]);
        if buf.contains(&b'\n') {
            break;
        }
        if buf.len() > MAX_REQUEST_BYTES {
            return None;
        }
    }
    let nl = buf.iter().position(|b| *b == b'\n').unwrap();
    Some(String::from_utf8_lossy(&buf[..nl]).trim().to_string())
}

unsafe fn write_all(pipe: HANDLE, mut data: &[u8]) {
    while !data.is_empty() {
        let mut wrote: u32 = 0;
        if WriteFile(
            pipe,
            data.as_ptr() as *const c_void,
            data.len() as u32,
            &mut wrote,
            ptr::null_mut(),
        ) == 0
            || wrote == 0
        {
            return;
        }
        data = &data[wrote as usize..];
    }
}

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
    fn pipe_name_defaults_and_overrides() {
        assert!(normalize_pipe_name(None).starts_with(r"\\.\pipe\"));
        assert!(normalize_pipe_name(Some("  ")).starts_with(r"\\.\pipe\"));
        assert_eq!(
            normalize_pipe_name(Some("PeekGuard-Test")),
            r"\\.\pipe\PeekGuard-Test"
        );
        assert_eq!(
            normalize_pipe_name(Some(r"\\.\pipe\Full")),
            r"\\.\pipe\Full"
        );
    }
}
