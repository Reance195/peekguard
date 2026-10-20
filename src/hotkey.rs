//! Global hotkey combos: grammar, canonical form and config plumbing.
//!
//! The config file stores combos as readable text ("Ctrl+Alt+P", "Cmd+Alt+P"),
//! so one file stays understandable on every platform. This module owns the
//! grammar; each platform module maps the parsed struct onto its own key codes
//! (Win32 virtual keys, X11 keysyms, Carbon virtual key codes).

#[cfg(not(windows))]
use std::sync::atomic::{AtomicBool, Ordering};

use crate::config::Config;

/// The non-modifier key of a combo.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    /// Uppercase ASCII letter, `b'A'..=b'Z'`.
    Letter(u8),
    /// ASCII digit, `b'0'..=b'9'`.
    Digit(u8),
    /// Function key, `1..=12`.
    F(u8),
}

/// One parsed hotkey: any number of modifiers plus exactly one key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Combo {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    /// The Windows key / Command key. The OS may reserve some of these.
    pub meta: bool,
    pub key: Key,
}

impl std::fmt::Display for Combo {
    /// Canonical spelling, e.g. `Win+Ctrl+Alt+P` or `Cmd+Alt+P`. Meta comes
    /// first, then the usual Ctrl / Alt / Shift order.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.meta {
            f.write_str(if cfg!(target_os = "macos") {
                "Cmd+"
            } else {
                "Win+"
            })?;
        }
        if self.ctrl {
            f.write_str("Ctrl+")?;
        }
        if self.alt {
            f.write_str("Alt+")?;
        }
        if self.shift {
            f.write_str("Shift+")?;
        }
        match self.key {
            Key::Letter(c) => write!(f, "{}", c as char),
            Key::Digit(c) => write!(f, "{}", c as char),
            Key::F(n) => write!(f, "F{n}"),
        }
    }
}

#[cfg(target_os = "macos")]
pub const DEFAULT_BIND: &str = "Cmd+Alt+P";
#[cfg(target_os = "macos")]
pub const DEFAULT_TOGGLE: &str = "Cmd+Alt+H";
#[cfg(target_os = "macos")]
pub const DEFAULT_LATCH: &str = "Cmd+Alt+L";

#[cfg(not(target_os = "macos"))]
pub const DEFAULT_BIND: &str = "Ctrl+Alt+P";
#[cfg(not(target_os = "macos"))]
pub const DEFAULT_TOGGLE: &str = "Ctrl+Alt+H";
#[cfg(not(target_os = "macos"))]
pub const DEFAULT_LATCH: &str = "Ctrl+Alt+L";

/// Modifier bit for [`modifier`].
const BIT_CTRL: u8 = 1;
const BIT_ALT: u8 = 2;
const BIT_SHIFT: u8 = 4;
const BIT_META: u8 = 8;

fn modifier(word: &str) -> Option<u8> {
    match word.to_ascii_lowercase().as_str() {
        "ctrl" | "control" => Some(BIT_CTRL),
        "alt" | "option" | "opt" => Some(BIT_ALT),
        "shift" => Some(BIT_SHIFT),
        "cmd" | "command" | "super" | "win" | "meta" => Some(BIT_META),
        _ => None,
    }
}

fn parse_key(word: &str) -> Result<Key, String> {
    let up = word.to_ascii_uppercase();
    let bytes = up.as_bytes();
    if bytes.len() == 1 && bytes[0].is_ascii_uppercase() {
        return Ok(Key::Letter(bytes[0]));
    }
    if bytes.len() == 1 && bytes[0].is_ascii_digit() {
        return Ok(Key::Digit(bytes[0]));
    }
    if let Some(rest) = up.strip_prefix('F') {
        if let Ok(n) = rest.parse::<u8>() {
            if (1..=12).contains(&n) {
                return Ok(Key::F(n));
            }
        }
    }
    Err(format!("不支持的按键: {word}（可用 A-Z、0-9、F1-F12）"))
}

/// Parse `"Ctrl+Alt+P"` (case-insensitive, spaces tolerated). At least one
/// modifier is required so a bare letter never gets stolen from the desktop.
pub fn parse(text: &str) -> Result<Combo, String> {
    let mut combo = Combo {
        ctrl: false,
        alt: false,
        shift: false,
        meta: false,
        key: Key::Letter(b'P'),
    };
    let mut key: Option<Key> = None;
    for raw in text.split('+') {
        let word = raw.trim();
        if word.is_empty() {
            continue;
        }
        if let Some(bit) = modifier(word) {
            combo.ctrl |= bit & BIT_CTRL != 0;
            combo.alt |= bit & BIT_ALT != 0;
            combo.shift |= bit & BIT_SHIFT != 0;
            combo.meta |= bit & BIT_META != 0;
            continue;
        }
        if key.is_some() {
            return Err(format!("只能有一个主键: {text}"));
        }
        key = Some(parse_key(word)?);
    }
    let Some(key) = key else {
        return Err(format!("缺少主键: {text}（例如 Ctrl+Alt+P）"));
    };
    if !(combo.ctrl || combo.alt || combo.shift || combo.meta) {
        return Err(format!("至少需要一个修饰键: {text}"));
    }
    combo.key = key;
    Ok(combo)
}

/// The three combos the app watches, in bind / toggle / latch order. Invalid
/// config values fall back to the defaults, so a typo can never leave the user
/// without hotkeys.
pub fn from_config(cfg: &Config) -> [Combo; 3] {
    let parse_or = |text: &str, fallback: &str| {
        parse(text).unwrap_or_else(|_| parse(fallback).expect("default combo parses"))
    };
    [
        parse_or(&cfg.hotkey_bind, DEFAULT_BIND),
        parse_or(&cfg.hotkey_toggle, DEFAULT_TOGGLE),
        parse_or(&cfg.hotkey_latch, DEFAULT_LATCH),
    ]
}

#[cfg(not(windows))]
static RELOAD_PENDING: AtomicBool = AtomicBool::new(false);

/// Ask the polling loop (X11) to rebuild its hotkey table.
#[cfg(target_os = "linux")]
pub fn request_reload() {
    RELOAD_PENDING.store(true, Ordering::SeqCst);
}

/// True exactly once after [`request_reload`].
#[cfg(not(windows))]
pub fn take_reload() -> bool {
    RELOAD_PENDING.swap(false, Ordering::SeqCst)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn letter(c: u8) -> Key {
        Key::Letter(c)
    }

    #[test]
    fn parses_a_basic_combo() {
        let c = parse("Ctrl+Alt+P").unwrap();
        assert!(c.ctrl && c.alt && !c.shift && !c.meta);
        assert_eq!(c.key, letter(b'P'));
        assert_eq!(c.to_string(), "Ctrl+Alt+P");
    }

    #[test]
    fn tolerates_case_and_spaces() {
        let c = parse("  ctrl + ALT + p  ").unwrap();
        assert_eq!(c.to_string(), "Ctrl+Alt+P");
    }

    #[test]
    fn modifier_aliases() {
        assert!(parse("control+option+k").unwrap().ctrl);
        assert!(parse("control+option+k").unwrap().alt);
        assert!(parse("command+shift+1").unwrap().meta);
        assert!(parse("super+shift+1").unwrap().meta);
        assert!(parse("win+shift+1").unwrap().meta);
    }

    #[test]
    fn canonical_order_puts_meta_first() {
        let c = parse("Shift+Ctrl+P").unwrap();
        assert_eq!(c.to_string(), "Ctrl+Shift+P");
        let c = parse("Shift+Win+P").unwrap();
        assert_eq!(
            c.to_string(),
            if cfg!(target_os = "macos") {
                "Cmd+Shift+P"
            } else {
                "Win+Shift+P"
            }
        );
    }

    #[test]
    fn function_and_digit_keys() {
        assert_eq!(parse("Ctrl+F12").unwrap().key, Key::F(12));
        assert_eq!(parse("Ctrl+0").unwrap().key, Key::Digit(b'0'));
        assert_eq!(parse("Ctrl+F1").unwrap().to_string(), "Ctrl+F1");
    }

    #[test]
    fn rejects_a_bare_key() {
        assert!(parse("P").is_err());
        assert!(parse("F5").is_err());
    }

    #[test]
    fn rejects_a_missing_key() {
        assert!(parse("Ctrl+Alt").is_err());
    }

    #[test]
    fn rejects_unknown_keys() {
        assert!(parse("Ctrl+Alt+F13").is_err());
        assert!(parse("Ctrl+Alt+Space").is_err());
    }

    #[test]
    fn rejects_two_keys() {
        assert!(parse("Ctrl+P+Q").is_err());
    }

    #[test]
    fn config_falls_back_to_defaults_on_garbage() {
        let cfg = Config {
            hotkey_bind: "not a combo".to_string(),
            ..Config::default()
        };
        let combos = from_config(&cfg);
        assert_eq!(
            combos[0].to_string(),
            parse(DEFAULT_BIND).unwrap().to_string()
        );
    }
}
