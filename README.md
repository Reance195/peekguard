# PeekGuard

English | [简体中文](README.zh-CN.md)

[![CI](https://github.com/Reance195/peekguard/actions/workflows/ci.yml/badge.svg)](https://github.com/Reance195/peekguard/actions/workflows/ci.yml)
[![license](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

Bind any window. The mouse leaves — it disappears. The mouse comes back — it returns.

This is the behaviour of QQ's "privacy peek window" in China, applied to *any* window
instead of just QQ.

```
cursor inside the window      350 ms after it left         cursor returns
┌──────────────┐           ┌──────────────┐           ┌──────────────┐
│  chat window  │   ────>   │  fully clear  │   ────>   │  chat window  │
│  as normal    │           │  nothing on   │           │  back to      │
│               │           │  screen       │           │  normal       │
└──────────────┘           └──────────────┘           └──────────────┘
```

- **Zero dependencies** — standard library and hand-written Win32 FFI only. Nothing
  from crates.io.
- **One file** — a ~275 KB exe. No installer. Lives in the tray.
- **Zero intrusion** — no injection, no subclassing, no global hooks. The target
  process is never touched.
- **Never leaves a mess** — a crash, a task-manager kill, or an abnormal exit all get
  your windows restored on the next launch.
- **Standing rules** — write one `rule =` line and PeekGuard figures out which windows
  to protect on its own.

> Windows only (x86_64).

## Download

Grab `peekguard.exe` from the [Releases](../../releases). Run it — there is nothing to
install.

## Build from source

Requires Rust 1.87 or newer (the code uses `is_multiple_of`):

```sh
git clone https://github.com/Reance195/peekguard.git
cd peekguard
cargo build --release      # -> target/release/peekguard.exe
```

On first run a default config is written to `%APPDATA%\PeekGuard\peekguard.conf`.
PeekGuard then sits in the tray; it has no main window.

## Usage

| Action | Hotkey |
|---|---|
| Bind / unbind the foreground window | `Ctrl+Alt+P` |
| Hide / restore right now | `Ctrl+Alt+H` |
| Toggle latch mode | `Ctrl+Alt+L` |

Tray icon:

- **Double-click** — bind / unbind the foreground window
- **Right-click** — full menu (hide strategy, rule auto-protection, delay, latch,
  save rule, run at login, reload config, quit)

### The usual flow

1. Switch to the window you want guarded (QQ, a browser, your stock app…)
2. Press `Ctrl+Alt+P` — the tray reports `bound: QQ.exe — my chat window`
3. Move the mouse away and the window hides; move back to the same spot and it returns
4. If you like it, tray → "save the current binding as a standing rule" and every
   launch from now on is guarded automatically

## Three hide strategies

Cycle them from tray → "hide strategy".

| Strategy | How | Good for |
|---|---|---|
| **alpha** (default) | set the window to zero transparency | smoothest; the geometry is untouched on restore; enough to block the view next to you |
| **move** | shove the window off the virtual screen | the most thorough — even the Alt-Tab thumbnail shows nothing |
| **hide** | `ShowWindow(SW_HIDE)` | widest compatibility, at the cost of leaving a taskbar button |

> In alpha mode the window still accepts clicks. That is deliberate: you can keep
> using the window while it is invisible.

## Latch mode

Toggle with `Ctrl+Alt+L`, or set `latch = true` in the config.

Once latched, a hidden window stays hidden even when the mouse comes back. Only
`Ctrl+Alt+H` or the tray menu will bring it back.

This is for the case where someone is already standing next to you. Otherwise you
move the mouse back and the window pops up in front of them.

## Standing rules (automatic protection)

Once you write a `rule =` line, PeekGuard rescans the window list once a second and
takes over any window that matches — **no manual binding, ever again**. Handy for
"QQ should always be guarded when it opens".

The scan picks one target per pass, in this order:

1. A match that is the foreground window wins — whatever you are looking at gets
   guarded first
2. Otherwise the topmost match in z-order
3. **Minimized windows are skipped.** They cannot be hidden at all, so parking the
   guard on one would leave every other match unprotected. A minimized window is
   reselected the moment it is restored, so skipping loses nothing.

A matched window gets a one-off "can you actually hide?" check up front. If it truly
refuses (typically an elevated target), PeekGuard says so once and remembers that
window — **no per-second retry spam**, and no hide/restore flapping.

> Two different things, on purpose: a window that gets *minimized after* being guarded
> keeps its binding (PeekGuard does not hop to another target); point 3 above is only
> about the scanner **choosing** a new target.

### Turning it off

Tray menu → "rule auto-protection", with a checkmark. Turning it off affects only the
rules — manual binding keeps working.

### What unbind actually means

This trips people up, so it is spelled out. Unbinding (`Ctrl+Alt+P`, or "unbind the
current window" from the menu) applies to **that one window** — it does not switch off
the rule:

| Action | The window you unbound | Later windows of the same kind |
|---|---|---|
| Unbind the current window | stays released; the rule does not take it back | guarded as usual |
| Turn "rule auto-protection" off in the menu | released immediately | unguarded until you turn it back on |
| Reload config | everything is recomputed | guarded as usual |

The reasoning: what you usually want is not "disable this rule" but "leave this one
alone". If unbinding also disabled the rule, unbinding QQ once would mean QQ is never
guarded again for the rest of the session — which is clearly not what anyone means.

## Run at login

Tray menu → "start with Windows", with a checkmark (read live from the registry, not
cached).

It writes a single `PeekGuard` value under
`HKCU\Software\Microsoft\Windows\CurrentVersion\Run`, pointing at the full path of the
current exe (quoted, so spaces are fine). The registry rather than the Startup folder or
Task Scheduler: no elevation, no console window flash, no watchdog process.

If you move the exe the checkmark stays on (it matches by name) — untick and re-tick it
once to repoint it at the new location.

## Configuration

`%APPDATA%\PeekGuard\peekguard.conf`, or point `PEEKGUARD_CONFIG` somewhere else.
Reload it from the tray menu after editing.

```ini
# ms to wait after the cursor leaves before hiding (debounce, so a shaky
# mouse does not make the window flicker)
delay_ms = 350

# px of slack around the window rect when testing whether the cursor is inside
margin_px = 6

# hide strategy: alpha | move | hide
mode = alpha

# opacity used in alpha mode (0 = invisible)
alpha = 0

# true = stay hidden after hiding; restore manually
latch = false

# true = only react to the cursor while the target is the foreground window
hide_only_when_focused = false

# already-minimized windows are skipped
# deprecated: the behaviour is now unconditional (a minimized window cannot be
# hidden anyway). The key is kept so old configs do not warn; the value has no
# effect.
skip_minimized = true

# windows bound automatically at startup (may be empty)
autobind =

# standing rules: automatically guard matching windows when they appear
# fields are separated by | ; inside title:, commas mean "or"
rule = process:QQ.exe
rule = process:msedge.exe | title:novel,stocks
```

Rule fields: `process:` (process name, case-insensitive), `class:` (window class name),
`title:` (title substring; comma-separated values mean "or"). Multiple fields on one line
are combined with **AND**.

## Known limitations

| Situation | Behaviour | Notes |
|---|---|---|
| Target runs elevated | refused, with a message | Windows UIPI: an elevated process's windows cannot be modified by a normal one. **Run PeekGuard elevated and this goes away** |
| UWP apps (Settings, Photos, Calculator) | may not work | these are hosted by `ApplicationFrameHost`; the real window handle is not the one you see |
| Programs using `UpdateLayeredWindow` | alpha mode is inert | detected automatically; switch to "move" instead |
| The target restyles its own window | PeekGuard "heals" it | hide once more |
| Multiple monitors / negative coordinates | handled | geometry is in virtual-screen space, and per-monitor DPI awareness is on |

### Safety boundaries

- **No injection, no subclassing, no global hooks.** The whole thing is
  `GetCursorPos` polled every 40 ms plus a geometry test. The target process is never
  modified.
- No DWM cloak: cross-process calls to it return `E_ACCESSDENIED` in practice
  (see [`docs/superpowers/specs`](docs/superpowers/specs/2026-10-10-peekguard-design.md)).
- **Always restored before exit.** Both a normal quit and the tray's quit path restore
  every window first.
- **A crash does not strand your windows.** Every hide is journalled to
  `hidden.journal`; the next launch restores from it.
- **Never "hidden with no record".** The journal is written *before* the window is
  touched. The one unrecoverable state would be "invisible with nothing written down",
  so the record always hits the disk first. The reverse — one redundant entry — is
  harmless, because restoring an already-visible window is a no-op. (That invariant is
  pinned down by 720 high-frequency samples; see
  [`docs/superpowers/specs`](docs/superpowers/specs/2026-10-10-peekguard-design.md).)
- **Even a panic restores your windows.** A panic hook runs even under `panic = "abort"`,
  so the windows go back before the process dies — verified end-to-end by
  `tests/e2e_panic.py`.

## Development

```sh
cargo build --release      # -> target/release/peekguard.exe (~275 KB, zero deps)
cargo test                 # 17 unit tests (no side effects, plus 1 #[ignore] diagnostic)

# end-to-end (moves the real cursor, needs an interactive desktop)
for m in alpha move hide; do PEEKGUARD_MODE=$m python tests/e2e_live.py; done         # 23 checks
for m in alpha move hide; do PEEKGUARD_MODE=$m python tests/e2e_latch.py; done        # 15 checks
for m in alpha move hide; do PEEKGUARD_MODE=$m python tests/e2e_rules.py; done        # 16 checks
for m in alpha move hide; do PEEKGUARD_MODE=$m python tests/e2e_robustness.py; done   # 16 checks
for m in alpha move hide; do PEEKGUARD_MODE=$m python tests/e2e_panic.py; done        # 10 checks
```

240 end-to-end checks in total, all green (each strategy is certified separately). Every
suite names its own `PEEKGUARD_MUTEX` single-instance name, so it never fights the
PeekGuard you already have running; the rules under test pin both `class:` and `title:`
so they only ever match the suite's own window.

`PEEKGUARD_MODE=alpha|move|hide` selects which hide strategy a suite certifies.
`PEEKGUARD_DEBUG=1` turns on verbose logging to stderr and
`%APPDATA%\PeekGuard\debug.log`. The remaining knobs are listed in
[`tests/README.md`](tests/README.md).

CI runs `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`,
and `cargo build --release`. The end-to-end suites are **not** in CI: they move the real
mouse and need an interactive desktop.

The build is warning-free. The FFI modules keep their Win32 type names (`HWND`,
`NOTIFYICONDATAW`, …) on purpose — those `upper_case_acronyms` warnings are allowed
in place, with a note, because the whole point of a hand-written binding is that it
can be checked line-by-line against the SDK headers.

### Layout

| File | Role |
|---|---|
| `src/win.rs` | hand-written Win32 FFI (zero deps, x86_64 only), including the registry |
| `src/hide.rs` | hide primitives + state snapshot / lossless restore |
| `src/watch.rs` | state machine: cursor polling, debounce, latch |
| `src/rules.rs` | standing-rule scanner: rate limiting, target picking, per-window suppression |
| `src/config.rs` | config parsing + rule matching |
| `src/autostart.rs` | run at login (HKCU Run directly, no subprocess) |
| `src/session.rs` | crash-recovery journal |
| `src/tray.rs` | tray icon + menu |
| `src/icon.rs` | icon generated at runtime (no resource compiler needed) |

## Code signing policy

**The released `peekguard.exe` is not code-signed.** This project applied to the
SignPath Foundation free signing program and was not accepted, and a commercial
certificate is out of reach for a hobby project. Rather than leave that unstated, here
is what it means on your machine and what you can check instead.

What is verifiable without a signature:

- every release is built by GitHub Actions from this repository; the workflow is public
  at `.github/workflows/ci.yml`
- each release publishes a SHA-256 checksum of the exe (`SHA256SUMS.txt`)
- with zero third-party dependencies, building it yourself is a few hundred lines of Rust
  and nothing from crates.io, so `diff` is a complete audit

| Symptom | Why |
|---|---|
| SmartScreen shows "Windows protected your PC" | reputation is built from install counts, and since March 2024 a signature alone does not grant it either |
| UAC names an "unknown publisher" | the exe carries no signature |
| Smart App Control refuses to launch it | SAC blocks unsigned executables unless its policy is changed |

### Team roles

| Role | Members | Scope |
|---|---|---|
| Authors | [`Owners`](https://github.com/Reance195/peekguard/people?query=role%3Aowner) | may change source and CI without further review |
| Reviewers | [`Owners`](https://github.com/Reance195/peekguard/people?query=role%3Aowner) | review every pull request |

This is a single-maintainer project, so both roles currently resolve to the same
person. The first additional contributor to be merged is expected to take over Reviewer.

### Privacy

**PeekGuard transfers no information to other networked systems.** There is no network
code in it at all — no telemetry, no update check, no crash reporting. The complete set
of Win32 APIs it imports is visible in `src/win.rs`; none of them open a socket or an
internet connection. It ships with zero third-party dependencies
(`Cargo.toml` has no `[dependencies]` section).

Everything it writes is local, and the full list is here:

| Path | What | When |
|---|---|---|
| `%APPDATA%\PeekGuard\peekguard.conf` | configuration | written on first run; overridable via `PEEKGUARD_CONFIG`, or a `peekguard.conf` placed next to the exe |
| `%APPDATA%\PeekGuard\hidden.journal` | crash-recovery journal | only while windows are hidden; removed once they are restored |
| `%APPDATA%\PeekGuard\debug.log` | diagnostic log | only when `PEEKGUARD_DEBUG=1` |

The tray icon is generated at runtime and is never written to disk.

The one system change PeekGuard can make is a `PeekGuard` value under
`HKCU\Software\Microsoft\Windows\CurrentVersion\Run`, and only after you opt in through
the "start with Windows" tray item. Unticking that item deletes the value.

It never injects code, subclasses a window, or installs a global hook. The only
cross-process calls it makes are `PROCESS_QUERY_LIMITED_INFORMATION`, used to read a
window's process name for `rule = process:…` matching and to detect an elevated target.

### Uninstallation

1. Tray → quit
2. If "start with Windows" is ticked, untick it — this removes the registry value
3. Delete the exe
4. Delete `%APPDATA%\PeekGuard\`

## License

[MIT](LICENSE) © 2026 PeekGuard contributors