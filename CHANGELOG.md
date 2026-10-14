# Changelog

All notable changes to this project are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed

- **Window picker:** `Ctrl+Alt+P` and the tray "选择窗口并绑定…" item now open a crosshair picker. Move the cursor to highlight the target window's border, left-click to bind it, or right-click/Esc to cancel (no background dimming).
- **About dialog:** Replaced the plain system message box with a custom-painted About window that displays the app icon, version, hotkeys, and a clickable GitHub link.

## [0.1.0] — 2026-10-13

First public release.

### Added

- Bind any top-level window with `Ctrl+Alt+P`, unbind with the same chord; double-click
  the tray icon for the same thing.
- Hide the bound window when the cursor leaves it, restore it when the cursor comes
  back to the original rect. Cursor position is polled every 40 ms with a configurable
  debounce (`delay_ms`, default 350) and edge slack (`margin_px`, default 6).
- Three hide strategies, cycled from the tray: `alpha` (zero transparency, default),
  `move` (off the virtual screen), `hide` (`ShowWindow(SW_HIDE)`). Alpha mode is
  detected as inapplicable for `UpdateLayeredWindow` windows and skipped.
- Latch mode (`Ctrl+Alt+L`): once hidden, stay hidden until `Ctrl+Alt+H` or the tray
  menu restores the window.
- Manual hide / restore on demand with `Ctrl+Alt+H`.
- Standing rules: `rule = process:… | class:… | title:…` lines are rescanned once a
  second, foreground matches win, minimized windows are skipped, and a window that
  genuinely refuses to hide is reported once and suppressed rather than retried.
- Per-window unbind that does not disable the rule that re-picks it.
- Run at login via a single `HKCU\…\Run` value, read live from the registry for the
  menu's checkmark.
- Crash-safe restore: every hide is journalled to `hidden.journal` before the window is
  touched, and a panic hook restores everything even under `panic = "abort"`.
- Generated-at-runtime tray icon (no `.ico`, no resource compiler) with a fallback to
  the stock application icon.
- `debug.log` when `PEEKGUARD_DEBUG=1` is set.

### Notes

- Windows x86_64 only.
- Zero dependencies — standard library and hand-written Win32 FFI.
- 17 unit tests plus 240 end-to-end checks across the three hide strategies.

[0.1.0]: https://github.com/Reance195/peekguard/releases/tag/v0.1.0