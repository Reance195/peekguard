//! Pieces shared by the platform hide implementations.
//!
//! The crash-recovery journal has one line format on every platform:
//! whitespace-separated `key=value` fields, with `hwnd`, `mode`, `rect`, the
//! platform-specific snapshot fields, and `pid` / `proc` for the stale-window
//! guard. Only the snapshot fields and the calls that hide/restore a window
//! differ, so parsing and formatting live here instead of being copied into
//! `hide.rs`, `x11/hide.rs` and `macos/hide.rs`.

use crate::config::HideMode;
use std::sync::OnceLock;

/// A parsed journal line. Fields stay borrowed from the input.
pub struct Entry<'a> {
    fields: Vec<(&'a str, &'a str)>,
}

impl<'a> Entry<'a> {
    pub fn parse(line: &'a str) -> Self {
        Self {
            fields: line
                .split_whitespace()
                .filter_map(|kv| kv.split_once('='))
                .collect(),
        }
    }

    pub fn get(&self, key: &str) -> Option<&'a str> {
        self.fields.iter().find(|(k, _)| *k == key).map(|(_, v)| *v)
    }

    pub fn hwnd(&self) -> Option<usize> {
        usize::from_str_radix(self.get("hwnd")?.trim_start_matches("0x"), 16).ok()
    }

    pub fn mode(&self) -> HideMode {
        self.get("mode")
            .and_then(HideMode::parse)
            .unwrap_or(HideMode::Alpha)
    }

    pub fn rect(&self) -> Option<(i32, i32, i32, i32)> {
        let mut it = self.get("rect")?.split(',');
        let left = it.next()?.parse().ok()?;
        let top = it.next()?.parse().ok()?;
        let right = it.next()?.parse().ok()?;
        let bottom = it.next()?.parse().ok()?;
        if it.next().is_some() {
            return None;
        }
        Some((left, top, right, bottom))
    }

    pub fn number<T: std::str::FromStr>(&self, key: &str) -> Option<T> {
        self.get(key)?.parse().ok()
    }

    /// True unless the line carries both `pid` and `proc` and neither still
    /// matches the window. That is the case where the handle was recycled.
    pub fn same_process(&self, actual_pid: u32, actual_name: &str) -> bool {
        let (Some(pid), Some(process)) = (self.get("pid"), self.get("proc")) else {
            return true;
        };
        match pid.parse::<u32>() {
            Ok(pid) => actual_pid == pid || actual_name.eq_ignore_ascii_case(process),
            Err(_) => true,
        }
    }
}

/// Format one journal line. `extra` carries the platform snapshot fields in
/// the order they should appear in the line.
pub fn format(
    hwnd: usize,
    mode: HideMode,
    rect: (i32, i32, i32, i32),
    extra: &[(&str, String)],
    pid: u32,
    process: &str,
) -> String {
    let (left, top, right, bottom) = rect;
    let mut out = format!(
        "hwnd={hwnd:#x} mode={} rect={left},{top},{right},{bottom}",
        mode.as_str()
    );
    for (key, value) in extra {
        out.push(' ');
        out.push_str(key);
        out.push('=');
        out.push_str(value);
    }
    out.push_str(&format!(" pid={pid} proc={process}\n"));
    out
}

/// Journal fields spell an absent snapshot value as `none`, not as an empty
/// field, so the line stays whitespace-separated.
pub fn number_or_none<T: ToString>(value: Option<T>) -> String {
    value
        .map(|v| v.to_string())
        .unwrap_or_else(|| "none".into())
}

/// Test-only: `PEEKGUARD_REFUSE_HIDE=1` makes every hide attempt fail the way
/// an elevated target does. Cached because the environment is fixed for the
/// lifetime of the process.
pub fn refuse_hide_hook() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| {
        std::env::var("PEEKGUARD_REFUSE_HIDE")
            .map(|v| v == "1")
            .unwrap_or(false)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_opacity_line() {
        let e = Entry::parse(
            "hwnd=0x2a mode=move rect=10,20,30,40 opacity=4294967295 pid=7 proc=foo.exe\n",
        );
        assert_eq!(e.hwnd(), Some(42));
        assert_eq!(e.mode(), HideMode::Move);
        assert_eq!(e.rect(), Some((10, 20, 30, 40)));
        assert_eq!(e.number::<u64>("opacity"), Some(4_294_967_295));
        assert_eq!(e.number::<u8>("alpha"), None);
    }

    #[test]
    fn windows_extras_round_trip_in_order() {
        let line = format(
            0x2a,
            HideMode::Alpha,
            (10, 20, 30, 40),
            &[
                ("alpha", number_or_none(Some(128u8))),
                ("layered", "1".to_string()),
                ("exstyle", format!("{:#x}", 0x80000isize)),
            ],
            7,
            "foo.exe",
        );
        assert_eq!(
            line,
            "hwnd=0x2a mode=alpha rect=10,20,30,40 alpha=128 layered=1 exstyle=0x80000 pid=7 proc=foo.exe\n"
        );
        let e = Entry::parse(&line);
        assert_eq!(e.number::<u8>("alpha"), Some(128));
        assert_eq!(e.get("layered"), Some("1"));
        assert_eq!(e.get("exstyle"), Some("0x80000"));
        assert_eq!(e.mode(), HideMode::Alpha);
    }

    #[test]
    fn missing_fields_fall_back_safely() {
        let e = Entry::parse("");
        assert_eq!(e.hwnd(), None);
        assert_eq!(e.mode(), HideMode::Alpha);
        assert_eq!(e.rect(), None);
        assert_eq!(e.number::<u8>("alpha"), None);
        assert_eq!(e.number::<u64>("opacity"), None);
        assert!(e.same_process(8, "bar.exe"));
    }

    #[test]
    fn malformed_rect_and_numbers_are_rejected() {
        assert_eq!(Entry::parse("rect=1,2,3").rect(), None);
        assert_eq!(Entry::parse("rect=a,b,c,d").rect(), None);
        assert_eq!(Entry::parse("rect=1,2,3,4,5").rect(), None);
        assert_eq!(Entry::parse("alpha=300").number::<u8>("alpha"), None);
        assert_eq!(Entry::parse("hwnd=nope").hwnd(), None);
    }

    #[test]
    fn process_guard_accepts_pid_or_name() {
        let e = Entry::parse("hwnd=0x1 pid=7 proc=foo.exe");
        assert!(e.same_process(7, "bar.exe"));
        assert!(e.same_process(8, "FOO.EXE"));
        assert!(!e.same_process(8, "bar.exe"));

        let partial = Entry::parse("hwnd=0x1 pid=7");
        assert!(partial.same_process(8, "bar.exe"));
    }

    #[test]
    fn number_or_none_matches_the_wire_format() {
        assert_eq!(number_or_none(Some(128u8)), "128");
        assert_eq!(number_or_none::<u8>(None), "none");
    }
}
