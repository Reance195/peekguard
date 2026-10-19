//! Autostart through the XDG autostart directory.
//!
//! The equivalent of the Windows `Run` registry value: a `.desktop` file in
//! `~/.config/autostart` that the desktop session launches at login.
//! Same public surface as `crate::autostart`: `command()`, `current()`,
//! `enabled()` and `set(bool)`.

use std::path::PathBuf;

const DEFAULT_NAME: &str = "peekguard";

/// Test hook: `PEEKGUARD_AUTOSTART_VALUE` renames the desktop file so tests
/// can run isolated from a real installation.
fn value_name() -> String {
    std::env::var("PEEKGUARD_AUTOSTART_VALUE")
        .ok()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| DEFAULT_NAME.to_string())
}

fn config_home() -> PathBuf {
    if let Some(dir) = std::env::var_os("XDG_CONFIG_HOME") {
        let p = PathBuf::from(dir);
        if !p.as_os_str().is_empty() {
            return p;
        }
    }
    match std::env::var_os("HOME") {
        Some(h) if !h.is_empty() => PathBuf::from(h).join(".config"),
        _ => PathBuf::from(".config"),
    }
}

fn desktop_path() -> PathBuf {
    config_home()
        .join("autostart")
        .join(format!("{}.desktop", value_name()))
}

/// Quote a path for a desktop-entry `Exec=` value: the whole argument is
/// double-quoted, backslashes and quotes inside are escaped.
fn quote_exec(path: &str) -> String {
    format!("\"{}\"", path.replace('\\', "\\\\").replace('"', "\\\""))
}

/// The `Exec=` value that launches us at login, if the exe path is known.
pub fn command() -> Option<String> {
    let exe = std::env::current_exe().ok()?;
    Some(quote_exec(&exe.to_string_lossy()))
}

/// Read the current `Exec=` value from the autostart entry, if any.
pub fn current() -> Option<String> {
    let text = std::fs::read_to_string(desktop_path()).ok()?;
    for line in text.lines() {
        if let Some(v) = line.strip_prefix("Exec=") {
            return Some(v.trim().to_string());
        }
    }
    None
}

/// Whether an autostart entry for us currently exists.
pub fn enabled() -> bool {
    std::fs::read_to_string(desktop_path())
        .map(|t| t.to_ascii_lowercase().contains("peekguard"))
        .unwrap_or(false)
}

/// Create or remove the autostart entry.
pub fn set(enable: bool) -> Result<(), String> {
    let path = desktop_path();
    if enable {
        let exe = command().ok_or("无法确定程序路径")?;
        let parent = path.parent().ok_or("无法确定 autostart 目录")?;
        std::fs::create_dir_all(parent).map_err(|e| format!("创建目录失败: {e}"))?;
        let body = format!(
            "[Desktop Entry]\n\
             Type=Application\n\
             Name=PeekGuard\n\
             Comment=隐身被保护的窗口：鼠标移开自动隐身，鼠标移回自动恢复\n\
             Exec={exe}\n\
             Terminal=false\n\
             X-GNOME-Autostart-enabled=true\n"
        );
        std::fs::write(&path, body).map_err(|e| format!("写入失败: {e}"))
    } else {
        match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(format!("删除失败: {e}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exec_quoting_follows_desktop_spec() {
        assert_eq!(quote_exec("/usr/bin/peekguard"), "\"/usr/bin/peekguard\"");
        assert_eq!(quote_exec("/opt/a b/peekguard"), "\"/opt/a b/peekguard\"");
        assert_eq!(quote_exec("we\\ird"), "\"we\\\\ird\"");
        assert_eq!(quote_exec("quo\"te"), "\"quo\\\"te\"");
    }

    #[test]
    fn desktop_path_uses_value_name() {
        // No env manipulation here: just prove the path shape is stable.
        let p = desktop_path();
        assert_eq!(p.extension().and_then(|e| e.to_str()), Some("desktop"));
        assert!(p
            .components()
            .any(|c| c.as_os_str().to_string_lossy() == "autostart"));
    }
}
