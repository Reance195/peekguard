## What this changes

<!-- One or two sentences. If the change is large enough that this is hard to write, it probably wants to be split. -->

## Why

<!-- The problem being solved, or the bug being fixed. Link the issue this closes, if any. -->

## Type

- [ ] Bug fix (no behaviour change beyond the fix)
- [ ] Feature (hide, restore, rules, autostart, or crash recovery)
- [ ] Refactor (no behaviour change)
- [ ] Documentation or packaging only

## Checklist

Ground rules from CONTRIBUTING.md, restated because CI will not catch them:

- [ ] **No new dependency.** No `[dependencies]` entry, no new crates.io import. If one is
      unavoidable, the reason is spelled out below and it is a good one.
- [ ] **No intrusion into the target process.** No `WriteProcessMemory`, no
      `CreateRemoteThread`, no `SetWindowsHookEx`, no subclassing. The only cross-process
      handle opened is `OpenProcess` with `PROCESS_QUERY_LIMITED_INFORMATION`.
- [ ] **The restore path is touched alongside the hide path.** Nothing can reach the state
      "hidden, no journal entry" — the journal is written before the window is moved.
- [ ] **Win32 names stay Win32** in `src/win.rs` and `src/icon.rs`.
- [ ] No new files written outside `%APPDATA%\PeekGuard`, and no registry writes beyond
      the autostart value.

Verification, locally:

- [ ] `cargo fmt --all -- --check`
- [ ] `cargo clippy --all-targets -- -D warnings`
- [ ] `cargo test`
- [ ] `cargo build --release`

End-to-end behaviour, if this touches hiding, restoring, rules, or autostart:

- [ ] Ran the relevant suite in `tests/` against a release build
      (`PEEKGUARD_MUTEX` must point at a scratch mutex, and `PEEKGUARD_MODE` must be
      set, or the checks will hit the wrong instance — see CONTRIBUTING.md)
- [ ] Manually confirmed the window comes back: cursor out, cursor in, `Ctrl+Alt+H`,
      and after killing the process with Task Manager
