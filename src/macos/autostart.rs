//! Login autostart through a LaunchAgent plist in `~/Library/LaunchAgents`.
//!
//! Writing the plist is all it takes: launchd scans the directory at login
//! and `RunAtLoad` starts the program. Disabling just removes the file.

/// `PEEKGUARD_AUTOSTART_VALUE` renames the plist (used by tests); the label
/// stays the same either way.
fn value_name() -> String {
    std::env::var("PEEKGUARD_AUTOSTART_VALUE")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| "peekguard".to_string())
}

fn home_dir() -> std::path::PathBuf {
    std::env::var("HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::path::PathBuf::from("."))
}

fn plist_path() -> std::path::PathBuf {
    home_dir()
        .join("Library/LaunchAgents")
        .join(format!("{}.plist", value_name()))
}

const LABEL: &str = "com.peekguard.autostart";

/// The executable path that will be written into the plist.
pub fn command() -> Option<String> {
    let exe = std::env::current_exe().ok()?;
    Some(exe.to_string_lossy().to_string())
}

fn escape_xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn unescape_xml(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

fn plist_body(exe: &str) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \
         \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
         <plist version=\"1.0\">\n\
         <dict>\n\
         \x20   <key>Label</key>\n\
         \x20   <string>{LABEL}</string>\n\
         \x20   <key>ProgramArguments</key>\n\
         \x20   <array>\n\
         \x20       <string>{}</string>\n\
         \x20   </array>\n\
         \x20   <key>RunAtLoad</key>\n\
         \x20   <true/>\n\
         </dict>\n\
         </plist>\n",
        escape_xml(exe)
    )
}

/// The executable currently written into the plist, if any.
pub fn current() -> Option<String> {
    let text = std::fs::read_to_string(plist_path()).ok()?;
    let start = text.find("ProgramArguments")?;
    let rest = &text[start..];
    let open = rest.find("<string>")? + "<string>".len();
    let close = rest[open..].find("</string>")? + open;
    Some(unescape_xml(&rest[open..close]))
}

pub fn enabled() -> bool {
    matches!(current(), Some(c) if !c.trim().is_empty())
}

pub fn set(enable: bool) -> Result<(), String> {
    let path = plist_path();
    if enable {
        let Some(exe) = command() else {
            return Err("无法定位可执行文件".to_string());
        };
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("无法创建 {}：{e}", dir.display()))?;
        }
        std::fs::write(&path, plist_body(&exe))
            .map_err(|e| format!("无法写入 {}：{e}", path.display()))
    } else {
        match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(format!("无法删除 {}：{e}", path.display())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plist_escapes_xml_entities() {
        assert_eq!(escape_xml("a&b<c>d"), "a&amp;b&lt;c&gt;d");
        assert_eq!(unescape_xml("a&amp;b&lt;c&gt;d"), "a&b<c>d");
    }

    #[test]
    fn plist_body_carries_label_and_executable() {
        let body = plist_body("/tmp/peek guard");
        assert!(body.contains("<string>com.peekguard.autostart</string>"));
        assert!(body.contains("<string>/tmp/peek guard</string>"));
        assert!(body.contains("<key>RunAtLoad</key>"));
        assert!(body.starts_with("<?xml"));
    }
}
