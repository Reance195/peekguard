//! Run-at-login, via `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`.
//!
//! HKCU is per-user and needs no elevation, so this works from a normal launch
//! and never triggers UAC.
//!
//! The registry is read/written directly through `advapi32` rather than shelling
//! out to `reg.exe`: one less process spawned, and no console window flash.
//!
//! `PEEKGUARD_AUTOSTART_VALUE` overrides the value name, which lets the e2e
//! tests exercise the real registry round trip without touching the entry the
//! user actually relies on.

use crate::win::*;

const RUN_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
const DEFAULT_VALUE: &str = "PeekGuard";

fn value_name() -> String {
    std::env::var("PEEKGUARD_AUTOSTART_VALUE")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_VALUE.to_string())
}

/// The command line we want Windows to run at login: our exe, quoted, in case the
/// path contains spaces.
pub fn command() -> Option<String> {
    let exe = std::env::current_exe().ok()?;
    Some(format!("\"{}\"", exe.display()))
}

fn open_run_key(access: u32) -> Option<HKEY> {
    unsafe {
        let sub = to_wide(RUN_KEY);
        let mut key: HKEY = std::ptr::null_mut();
        if RegOpenKeyExW(HKEY_CURRENT_USER, sub.as_ptr(), 0, access, &mut key) == 0 {
            Some(key)
        } else {
            None
        }
    }
}

/// The raw command currently registered, if any.
pub fn current() -> Option<String> {
    let key = open_run_key(KEY_QUERY_VALUE)?;
    let name = to_wide(&value_name());
    unsafe {
        let mut ty: u32 = 0;
        let mut len: u32 = 0;
        if RegQueryValueExW(
            key,
            name.as_ptr(),
            std::ptr::null_mut(),
            &mut ty,
            std::ptr::null_mut(),
            &mut len,
        ) != 0
            || ty != REG_SZ
            || len < 2
        {
            RegCloseKey(key);
            return None;
        }
        let mut buf = vec![0u8; len as usize];
        let rc = RegQueryValueExW(
            key,
            name.as_ptr(),
            std::ptr::null_mut(),
            &mut ty,
            buf.as_mut_ptr(),
            &mut len,
        );
        RegCloseKey(key);
        if rc != 0 {
            return None;
        }
        // `chunks_exact(2)` rather than `as_chunks::<2>()`: decoding a UTF-16 byte
        // stream two bytes at a time is exactly what chunks_exact says, and the
        // tuple-and-index shape of `as_chunks` only gets in the way of it.
        #[allow(clippy::chunks_exact_to_as_chunks)]
        let words: Vec<u16> = buf
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        Some(
            String::from_utf16_lossy(&words)
                .trim_end_matches('\0')
                .to_string(),
        )
    }
}

/// Is PeekGuard registered to start at login?
///
/// Deliberately "is there an entry that looks like us" rather than "does it match
/// the current exe exactly": after the user moves the exe the stored path is
/// stale, and reporting that as "off" would be confusing. Re-enabling rewrites the
/// path, which doubles as the fix for a stale entry.
pub fn enabled() -> bool {
    current()
        .map(|v| v.to_ascii_lowercase().contains("peekguard"))
        .unwrap_or(false)
}

/// Turn run-at-login on or off. Returns a human-readable error on failure.
pub fn set(enable: bool) -> Result<(), String> {
    let key = open_run_key(KEY_QUERY_VALUE | KEY_SET_VALUE)
        .ok_or_else(|| "无法打开注册表 Run 项".to_string())?;
    let name = to_wide(&value_name());
    let rc = unsafe {
        if enable {
            let cmd = command().ok_or_else(|| "无法确定程序路径".to_string())?;
            let data = to_wide(&cmd);
            RegSetValueExW(
                key,
                name.as_ptr(),
                0,
                REG_SZ,
                data.as_ptr() as *const u8,
                (data.len() * 2) as u32,
            )
        } else {
            let rc = RegDeleteValueW(key, name.as_ptr());
            // Removing something that was never there is a success, not an error.
            if rc == ERROR_FILE_NOT_FOUND {
                ERROR_SUCCESS
            } else {
                rc
            }
        }
    };
    unsafe {
        RegCloseKey(key);
    }
    if rc == ERROR_SUCCESS {
        Ok(())
    } else {
        Err(format!("注册表操作失败 (code {rc})"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_is_quoted() {
        let c = command().expect("test binary has a path");
        assert!(
            c.starts_with('"') && c.ends_with('"'),
            "must be quoted: {c}"
        );
    }

    #[test]
    fn read_does_not_panic_on_missing_entry() {
        // Never asserts a value: the developer's machine may or may not have it.
        let _ = current();
        let _ = enabled();
    }
}
