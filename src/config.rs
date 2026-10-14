//! Config + rules persistence. Hand-rolled line format, zero dependencies.
//!
//! File format (`peekguard.conf`, next to the exe or in %APPDATA%\PeekGuard):
//!
//! ```text
//! # comment
//! delay_ms = 350
//! margin_px = 6
//! mode = alpha | move | hide
//! alpha = 0
//! latch = false
//! restore_on_focus = true
//! autobind = process:QQ.exe
//! rule = process:QQ.exe |  (match any title)
//! rule = process:msedge.exe | 防窥|小说     (title regex-free substring, |-separated)
//! ```
#![allow(dead_code)]

use std::fmt::Write as _;
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HideMode {
    /// Set layered alpha to `alpha` (0 = invisible). Cheapest, most compatible.
    Alpha,
    /// Move the window off the virtual screen. Strongest: gone from alt-tab previews too.
    Move,
    /// ShowWindow(SW_HIDE). Simple but the taskbar button stays.
    Hide,
}

impl HideMode {
    pub fn parse(s: &str) -> Option<HideMode> {
        match s.trim().to_ascii_lowercase().as_str() {
            "alpha" | "transparent" | "fade" => Some(HideMode::Alpha),
            "move" | "offscreen" => Some(HideMode::Move),
            "hide" | "swhide" => Some(HideMode::Hide),
            _ => None,
        }
    }
    pub fn as_str(&self) -> &'static str {
        match self {
            HideMode::Alpha => "alpha",
            HideMode::Move => "move",
            HideMode::Hide => "hide",
        }
    }
    pub fn next(&self) -> HideMode {
        match self {
            HideMode::Alpha => HideMode::Move,
            HideMode::Move => HideMode::Hide,
            HideMode::Hide => HideMode::Alpha,
        }
    }
}

/// One binding rule. All specified fields must match (AND).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Rule {
    /// e.g. "QQ.exe" (case-insensitive, exact file name)
    pub process: Option<String>,
    /// e.g. "Chrome_WidgetWin_1" (case-insensitive exact)
    pub class: Option<String>,
    /// comma-separated substrings; any one matching is enough
    pub title: Option<String>,
}

impl Rule {
    /// True when this rule constrains at least one field. A rule that constrains
    /// nothing would otherwise match every window, so it is treated as a non-rule.
    pub fn is_constrained(&self) -> bool {
        self.process.is_some() || self.class.is_some() || self.title.is_some()
    }

    /// Does `process` (a bare exe name, case-insensitive) satisfy this rule?
    /// A rule with no `process` field accepts any.
    pub fn matches_process(&self, process: &str) -> bool {
        match &self.process {
            Some(p) => process.eq_ignore_ascii_case(p),
            None => true,
        }
    }

    /// Does `class` satisfy this rule? A rule with no `class` field accepts any.
    pub fn matches_class(&self, class: &str) -> bool {
        match &self.class {
            Some(c) => class.eq_ignore_ascii_case(c),
            None => true,
        }
    }

    /// Does `title` satisfy this rule? A rule with no `title` field accepts any;
    /// otherwise the comma-separated substrings are OR'd, case-insensitively.
    pub fn matches_title(&self, title: &str) -> bool {
        match &self.title {
            Some(t) => {
                let hay = title.to_lowercase();
                t.split(',')
                    .map(|s| s.trim().to_lowercase())
                    .any(|needle| !needle.is_empty() && hay.contains(&needle))
            }
            None => true,
        }
    }

    /// All specified fields must match (AND). Kept as the one-shot form for callers
    /// that already hold all three strings; `rules` evaluates the parts lazily so a
    /// window rejected on its process name never pays for its class or title.
    pub fn matches(&self, process: &str, class: &str, title: &str) -> bool {
        self.is_constrained()
            && self.matches_process(process)
            && self.matches_class(class)
            && self.matches_title(title)
    }

    pub fn describe(&self) -> String {
        let mut parts = Vec::new();
        if let Some(p) = &self.process {
            parts.push(format!("process={p}"));
        }
        if let Some(c) = &self.class {
            parts.push(format!("class={c}"));
        }
        if let Some(t) = &self.title {
            parts.push(format!("title~{t}"));
        }
        parts.join(" ")
    }

    pub fn parse(s: &str) -> Option<Rule> {
        let mut r = Rule::default();
        for field in s.split('|') {
            let field = field.trim();
            if field.is_empty() {
                continue;
            }
            let (k, v) = field.split_once(':').or_else(|| field.split_once('='))?;
            let v = v.trim().to_string();
            if v.is_empty() {
                continue;
            }
            match k.trim().to_ascii_lowercase().as_str() {
                "process" | "proc" | "exe" => r.process = Some(v),
                "class" => r.class = Some(v),
                "title" => r.title = Some(v),
                _ => {}
            }
        }
        if r.process.is_none() && r.class.is_none() && r.title.is_none() {
            None
        } else {
            Some(r)
        }
    }
}

#[derive(Clone, Debug)]
pub struct Config {
    /// ms to wait after the cursor leaves before hiding
    pub delay_ms: u64,
    /// px of slack around the window rect when testing cursor insideness
    pub margin_px: i32,
    pub mode: HideMode,
    /// alpha used in Alpha mode (0..=255)
    pub alpha: u8,
    /// when true, once hidden the window stays hidden until hotkey/tray
    pub latch: bool,
    /// auto-hide windows on start that match `autobind`
    pub autobind: Option<Rule>,
    pub rules: Vec<Rule>,
    /// hide the window only while it is the foreground window? (default: no)
    pub hide_only_when_focused: bool,
    /// Accepted for backwards compatibility, but no longer has any effect: a
    /// minimized window is always skipped, because `hide()` cannot act on one and
    /// its `None` return must not be read as the window refusing.
    ///
    /// Kept (rather than deleted) so that existing configs do not suddenly report
    /// `未知配置项` after an upgrade. `to_text` only writes it back when it is set
    /// to a non-default value.
    pub skip_minimized: bool,
    /// Check GitHub for a newer release on startup. This is the only thing that
    /// ever touches the network, so it can be turned off entirely.
    pub check_update: bool,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            delay_ms: 60,
            margin_px: 10,
            mode: HideMode::Alpha,
            alpha: 0,
            latch: false,
            autobind: None,
            rules: Vec::new(),
            hide_only_when_focused: false,
            skip_minimized: true,
            check_update: true,
        }
    }
}

impl Config {
    pub fn config_dir() -> PathBuf {
        if let Ok(appdata) = std::env::var("APPDATA") {
            PathBuf::from(appdata).join("PeekGuard")
        } else {
            PathBuf::from(".")
        }
    }

    pub fn config_path() -> PathBuf {
        if let Ok(p) = std::env::var("PEEKGUARD_CONFIG") {
            if !p.trim().is_empty() {
                return PathBuf::from(p);
            }
        }
        let dir = Self::config_dir();
        let portable = dir.join("peekguard.conf");
        if portable.exists() {
            return portable;
        }
        // fall back to a file next to the executable
        if let Ok(exe) = std::env::current_exe() {
            if let Some(parent) = exe.parent() {
                let beside = parent.join("peekguard.conf");
                if beside.exists() {
                    return beside;
                }
            }
        }
        portable
    }

    pub fn load() -> (Config, Option<String>) {
        let path = Self::config_path();
        match std::fs::read_to_string(&path) {
            Ok(text) => {
                let (cfg, warnings) = Self::parse(&text);
                let note = if warnings.is_empty() {
                    None
                } else {
                    Some(format!("{} ({} 个问题)", path.display(), warnings.len()))
                };
                for w in &warnings {
                    eprintln!("[config] {w}");
                }
                (cfg, note)
            }
            Err(_) => {
                let cfg = Config::default();
                let _ = cfg.save();
                (cfg, Some(format!("已生成默认配置: {}", path.display())))
            }
        }
    }

    pub fn parse(text: &str) -> (Config, Vec<String>) {
        let mut cfg = Config::default();
        let mut warnings = Vec::new();
        for (lineno, raw) in text.lines().enumerate() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') || line.starts_with("//") {
                continue;
            }
            let (key, value) = match line.split_once('=') {
                Some(kv) => kv,
                None => {
                    warnings.push(format!("第 {} 行缺少 '=': {}", lineno + 1, line));
                    continue;
                }
            };
            let key = key.trim().to_ascii_lowercase();
            let value = value.trim();
            match key.as_str() {
                "delay_ms" | "delay" => match value.parse::<u64>() {
                    Ok(v) => cfg.delay_ms = v.min(60_000),
                    Err(_) => warnings.push(format!("delay_ms 不是数字: {value}")),
                },
                "margin_px" | "margin" => match value.parse::<i32>() {
                    Ok(v) => cfg.margin_px = v.clamp(0, 200),
                    Err(_) => warnings.push(format!("margin_px 不是数字: {value}")),
                },
                "mode" => match HideMode::parse(value) {
                    Some(m) => cfg.mode = m,
                    None => warnings.push(format!("未知 mode: {value}")),
                },
                "alpha" => match value.parse::<i32>() {
                    Ok(v) => cfg.alpha = v.clamp(0, 255) as u8,
                    Err(_) => warnings.push(format!("alpha 不是数字: {value}")),
                },
                "latch" => cfg.latch = parse_bool(value),
                "hide_only_when_focused" | "only_focused" => {
                    cfg.hide_only_when_focused = parse_bool(value)
                }
                "check_update" | "update_check" | "check_updates" => {
                    cfg.check_update = parse_bool(value)
                }
                "skip_minimized" => {
                    // Inert: minimized windows are always skipped now. Parsed so old
                    // config files keep loading, and surfaced once as a note.
                    cfg.skip_minimized = parse_bool(value);
                    if !cfg.skip_minimized {
                        warnings.push(
                            "skip_minimized 已废弃：最小化窗口一律跳过，此项不再生效".to_string(),
                        );
                    }
                }
                "autobind" => {
                    if value.is_empty() {
                        cfg.autobind = None;
                    } else {
                        match Rule::parse(value) {
                            Some(r) => cfg.autobind = Some(r),
                            None => warnings.push(format!("autobind 规则无法解析: {value}")),
                        }
                    }
                }
                "rule" | "rules" => {
                    // allow several rules on one line separated by ';'
                    for chunk in value.split(';') {
                        if chunk.trim().is_empty() {
                            continue;
                        }
                        match Rule::parse(chunk) {
                            Some(r) => cfg.rules.push(r),
                            None => warnings.push(format!("rule 无法解析: {chunk}")),
                        }
                    }
                }
                _ => warnings.push(format!("未知配置项: {key}")),
            }
        }
        (cfg, warnings)
    }

    pub fn to_text(&self) -> String {
        let mut s = String::new();
        let _ = writeln!(s, "# PeekGuard 配置文件");
        let _ = writeln!(s, "# 修改后按托盘菜单“重载配置”生效，或重启程序。");
        let _ = writeln!(s);
        let _ = writeln!(s, "# 鼠标离开窗口后等待多少毫秒才隐身");
        let _ = writeln!(s, "delay_ms = {}", self.delay_ms);
        let _ = writeln!(s, "# 判定“鼠标在窗口内”时四周额外给的像素余量");
        let _ = writeln!(s, "margin_px = {}", self.margin_px);
        let _ = writeln!(
            s,
            "# 隐身方式: alpha(透明) | move(移出屏幕) | hide(隐藏窗口)"
        );
        let _ = writeln!(s, "mode = {}", self.mode.as_str());
        let _ = writeln!(s, "# alpha 模式下的透明度，0 为完全不可见");
        let _ = writeln!(s, "alpha = {}", self.alpha);
        let _ = writeln!(
            s,
            "# true = 隐身之后必须用热键/托盘才能恢复（防领导突然回头）"
        );
        let _ = writeln!(s, "latch = {}", self.latch);
        let _ = writeln!(s, "# true = 只在目标窗口是前台窗口时才响应鼠标");
        let _ = writeln!(
            s,
            "hide_only_when_focused = {}",
            self.hide_only_when_focused
        );
        let _ = writeln!(
            s,
            "# true = 启动时向 GitHub 查询新版本（唯一的联网行为；false 完全不联网）"
        );
        let _ = writeln!(s, "check_update = {}", self.check_update);
        if self.skip_minimized != Config::default().skip_minimized {
            let _ = writeln!(
                s,
                "# 已废弃：最小化窗口一律跳过（此项仅保留兼容，无实际作用）"
            );
            let _ = writeln!(s, "skip_minimized = {}", self.skip_minimized);
        }
        let _ = writeln!(s);
        let _ = writeln!(s, "# 启动时自动绑定的窗口（可留空）");
        let _ = writeln!(
            s,
            "autobind = {}",
            self.autobind
                .as_ref()
                .map(|r| r.describe_rule())
                .unwrap_or_default()
        );
        let _ = writeln!(s);
        let _ = writeln!(s, "# 常驻规则：窗口出现时自动纳入保护");
        let _ = writeln!(
            s,
            "# 格式: rule = process:QQ.exe | title:聊天     字段用 | 分隔；title 内用 , 表示“或”"
        );
        for r in &self.rules {
            let _ = writeln!(s, "rule = {}", r.describe_rule());
        }
        s
    }

    pub fn save(&self) -> std::io::Result<()> {
        let path = Self::config_path();
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        std::fs::write(path, self.to_text())
    }

    pub fn add_rule(&mut self, rule: Rule) {
        if !self.rules.contains(&rule) {
            self.rules.push(rule);
        }
    }
}

impl Rule {
    /// Serialise back into the config file grammar.
    pub fn describe_rule(&self) -> String {
        let mut parts = Vec::new();
        if let Some(p) = &self.process {
            parts.push(format!("process:{p}"));
        }
        if let Some(c) = &self.class {
            parts.push(format!("class:{c}"));
        }
        if let Some(t) = &self.title {
            parts.push(format!("title:{t}"));
        }
        parts.join(" | ")
    }
}

fn parse_bool(v: &str) -> bool {
    matches!(
        v.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on" | "y"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rule_matches_process() {
        let r = Rule::parse("process:QQ.exe").unwrap();
        assert!(r.matches("qq.exe", "", "anything"));
        assert!(!r.matches("msedge.exe", "", "anything"));
    }

    #[test]
    fn rule_title_or_semantics() {
        let r = Rule::parse("process:msedge.exe | title:小说,股票").unwrap();
        assert!(r.matches("msedge.exe", "", "起点 - 小说阅读"));
        assert!(r.matches("msedge.exe", "", "行情 - 股票"));
        assert!(!r.matches("msedge.exe", "", "github"));
        assert!(!r.matches("chrome.exe", "", "小说"));
    }

    #[test]
    fn title_or_is_case_insensitive() {
        let r = Rule::parse("title:GitHub,gitlab").unwrap();
        assert!(r.matches("any.exe", "", "github · where software is built"));
    }

    #[test]
    fn field_predicates_agree_with_matches() {
        // The scanner evaluates process/class/title one at a time so a window
        // rejected on a cheap field never pays for a dearer one. That is only
        // sound if the per-field predicates reproduce `matches` exactly, so pin
        // them together on every combination of absent/present/match/miss.
        let cases = [
            ("process:QQ.exe", "qq.exe", "", "chat", true),
            ("process:QQ.exe", "msedge.exe", "", "chat", false),
            (
                "class:Chrome_WidgetWin_1",
                "x.exe",
                "chrome_widgetwin_1",
                "",
                true,
            ),
            ("class:Chrome_WidgetWin_1", "x.exe", "other", "", false),
            ("title:小说,股票", "x.exe", "", "行情-股票", true),
            ("title:小说,股票", "x.exe", "", "github", false),
            (
                "process:msedge.exe | class:abc | title:news",
                "msedge.exe",
                "abc",
                "daily news",
                true,
            ),
            (
                "process:msedge.exe | class:abc | title:news",
                "msedge.exe",
                "xyz",
                "daily news",
                false,
            ),
            (
                "process:msedge.exe | class:abc | title:news",
                "edge.exe",
                "abc",
                "daily news",
                false,
            ),
        ];
        for (spec, proc, class, title, want) in cases {
            let r = Rule::parse(spec).unwrap();
            let all = r.matches(proc, class, title);
            let parts = r.matches_process(proc) && r.matches_class(class) && r.matches_title(title);
            assert_eq!(all, want, "{spec}: matches");
            assert_eq!(parts, want, "{spec}: per-field predicates");
            assert_eq!(all, parts, "{spec}: the two forms must agree");
        }
    }

    #[test]
    fn unconstrained_rule_matches_nothing() {
        let r = Rule::default();
        assert!(!r.is_constrained());
        assert!(!r.matches("any.exe", "cls", "title"));
        // And the per-field predicates accept everything in isolation, which is
        // why `is_constrained` — not the predicates — is what keeps a blank rule
        // from swallowing every window.
        assert!(r.matches_process("any.exe"));
        assert!(r.matches_class("cls"));
        assert!(r.matches_title("title"));
    }

    #[test]
    fn empty_autobind_is_not_a_warning() {
        let (cfg, warns) = Config::parse("autobind = \nrule = \n");
        assert!(cfg.autobind.is_none());
        assert!(cfg.rules.is_empty());
        assert!(warns.is_empty(), "{warns:?}");
    }

    #[test]
    fn config_roundtrip() {
        let mut cfg = Config {
            delay_ms: 500,
            mode: HideMode::Move,
            ..Default::default()
        };
        cfg.add_rule(Rule::parse("process:QQ.exe | title:聊天").unwrap());
        let text = cfg.to_text();
        let (back, warns) = Config::parse(&text);
        assert!(warns.is_empty(), "{warns:?}");
        assert_eq!(back.delay_ms, 500);
        assert_eq!(back.mode, HideMode::Move);
        assert_eq!(back.rules.len(), 1);
        assert!(back.rules[0].matches("QQ.exe", "", "我的聊天窗口"));
    }

    #[test]
    fn bad_lines_warn_but_do_not_fail() {
        let (cfg, warns) = Config::parse("delay_ms = abc\nmode = nope\nnothing here\n");
        assert_eq!(cfg.delay_ms, Config::default().delay_ms);
        assert_eq!(warns.len(), 3);
    }
}
