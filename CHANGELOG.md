# Changelog

All notable changes to this project are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- **macOS support:** PeekGuard now runs on macOS (Apple silicon). Window control is a
  hand-written CoreGraphics + Accessibility binding in `src/macos/`: reads (geometry,
  owners, process names) come from `CGWindowListCopyWindowInfo` and work without any
  grant, while moves and minimizes go through the Accessibility API (System Settings →
  Privacy & Security → Accessibility — requested on first run). Two hide strategies
  carry over — `move` (AX position, parked off the virtual screen) and `hide` (AX
  minimize); there is no public cross-app transparency API, so alpha mode is not
  offered on this platform. Global hotkeys are `Cmd+Option+P/H/L` (Carbon
  `RegisterEventHotKey`, no extra grant needed). There is no tray here either: the
  Unix-socket IPC applies — the transport moved to `src/uds_ipc.rs`, shared with the
  Linux port (`$XDG_RUNTIME_DIR` → `TMPDIR` → `/tmp`) — notifications go through
  `osascript`, autostart is a `~/Library/LaunchAgents` plist, the config lives in
  `~/Library/Application Support/PeekGuard/`, and the update check uses the system
  `curl`. Syntax-checked with `cargo check --target aarch64-apple-darwin` and
  clippy-clean; not yet exercised on real hardware.

- **Linux (X11) support:** PeekGuard now runs on Linux desktops. The Win32 layer is
  mirrored by a hand-written Xlib binding in `src/x11/` — loaded at runtime with
  `dlopen("libX11.so.6")`, so the binary links nothing and needs no `-dev` packages —
  while `src/plat.rs` forwards the shared vocabulary (window handles, rects, cursor,
  window operations) to whichever platform is active. All three hide strategies carry
  over (`_NET_WM_WINDOW_OPACITY`, `XMoveWindow`, `XUnmapWindow`; alpha mode detects a
  missing compositor and is refused with a hint to pick another mode). There is no
  tray on Linux: the same hotkeys apply, and a Unix domain socket
  (`$XDG_RUNTIME_DIR/peekguard-ipc.sock`, mode 0600) serves the exact IPC protocol of
  the Windows named pipe — `utools-plugin/test-cli.js` doubles as a cross-platform
  client. Notifications go through `notify-send`, autostart is an XDG `.desktop` file,
  the config lives in `~/.config/peekguard/`, and the update check shells out to the
  system `curl`. `PEEKGUARD_PIPE` accepts a Unix socket path on Linux. Exercised
  end-to-end under WSLg.

- **uTools plugin:** [`utools-plugin/`](utools-plugin/) is a companion plugin that drives
  PeekGuard from uTools' search bar — a control panel (`防窥`), one-shot hide/restore
  (`隐身`) and a window-bind shortcut (`绑定窗口`). It remembers a manually chosen
  `peekguard.exe` path and can launch PeekGuard when it is not running. Plain
  HTML/CSS/JS + preload; no build step.
- **Named-pipe IPC:** PeekGuard now serves a local named pipe
  (`\\.\pipe\PeekGuard-IPC-v1`) so scripts (and the plugin) can read a state snapshot
  and drive the usual actions — hide/show/toggle, bind/unbind, latch, mode, scan,
  reload, autostart — with one text line in and one JSON line out. No network port is
  opened, and the default pipe ACL admits only the user who started PeekGuard. Hand
  written in `src/ipc.rs`; the zero-dependency promise holds. Covered by
  `tests/e2e_ipc.py` (40 checks) and `utools-plugin/test-cli.js` (18 checks).
- **Startup notification:** PeekGuard now shows a tray balloon when it starts, so a
  windowless run is no longer silent. Any crash-recovery count and config note are
  folded into the same balloon.
- **Embedded application icon:** the exe now carries a multi-size icon (16–128 px), generated at build time by `build.rs` from the same pure-Rust rasteriser the tray uses — no `.ico` file and no resource compiler. Explorer, the desktop shortcut and the taskbar now show the PeekGuard eye instead of the generic exe icon. (Exe grows to ~380 KB.)
- **Update check:** PeekGuard can now tell you when a newer release exists. On startup, and on demand from the tray menu, it makes a single anonymous `GET` to the public GitHub Releases API and compares its version with the latest `tag_name`; if a newer one exists it shows a tray balloon that opens the release page in your browser. This is the program's only network code — a hand-written WinHTTP call on Windows and the system `curl` on Linux, so the zero-dependency promise holds on both — and it is opt-out via `check_update = false`, which leaves the app fully offline. No machine or user information is sent.

### Changed

- **Restore guard:** restoring a hidden window (hotkey, IPC, or tray menu) while the cursor is away from it no longer lets the watcher re-hide it a tick later — auto-hide now stays suspended until the cursor has been back inside once, so a manual restore sticks.
- **Window picker internals:** the picker's decision logic is now a pure, unit-tested function (first click after the grace period binds; right-click or Esc cancels).
- **About dialog:** Replaced the plain system message box with a custom-painted About window that displays the app icon, version, hotkeys, and a clickable GitHub link. The window now scales with the monitor DPI.
- **Window picker:** `Ctrl+Alt+P` and the tray "选择窗口并绑定…" item now open a crosshair picker. Move the cursor to highlight the target window's border, left-click to bind it, or right-click/Esc to cancel (no background dimming).
- **Single version source:** the About window's version now comes from `Cargo.toml` via `env!("CARGO_PKG_VERSION")` instead of a hard-coded string, so bumping the manifest updates every surface at once.
- **Privacy wording:** the README and SECURITY pages now describe the optional update check accurately instead of claiming the app has no network code at all.

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