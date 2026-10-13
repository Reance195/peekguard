# Contributing

Thanks for looking at PeekGuard. It is a small, self-contained Rust program with no
dependencies, and the bar for a change is simply: **does it still build clean, still
pass the tests, and still restore the user's windows when something goes wrong.**

## Ground rules

1. **Keep it dependency-free.** The whole appeal of the project is that it is a single
   ~260 KB exe with nothing from crates.io. A new dependency needs a very good reason.
2. **Stay out of the target process.** No injection, no `SetWindowsHookEx`, no
   subclassing. Polling `GetCursorPos` and testing geometry is the entire mechanism.
3. **Never strand a window.** If you touch the hide path, you touch the restore path.
   "Invisible with no journal entry" is the one unrecoverable state, and the journal is
   written before the window is touched to keep it that way.
4. **Win32 names stay Win32.** `src/win.rs` and `src/icon.rs` deliberately keep names
   like `NOTIFYICONDATAW` so the binding can be diffed against the SDK headers. Their
   `upper_case_acronyms` and `non_snake_case` lints are allowed in place — leave them
   alone.

## Before you open a pull request

```sh
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo build --release
```

CI runs exactly those, on Windows, plus a build on the MSRV (1.87.0).

## End-to-end tests

The `tests/*.py` suites spawn the real binary, create a disposable tkinter window, and
**move the real cursor**. They need an interactive desktop, so they do not run in CI —
please run them locally if you touch `hide.rs`, `watch.rs`, `rules.rs`, or `session.rs`:

```sh
for m in alpha move hide; do PEEKGUARD_MODE=$m python tests/e2e_live.py; done
for m in alpha move hide; do PEEKGUARD_MODE=$m python tests/e2e_latch.py; done
for m in alpha move hide; do PEEKGUARD_MODE=$m python tests/e2e_rules.py; done
for m in alpha move hide; do PEEKGUARD_MODE=$m python tests/e2e_robustness.py; done
for m in alpha move hide; do PEEKGUARD_MODE=$m python tests/e2e_panic.py; done
```

Two things will bite you if you write a new suite, both documented in
[`tests/README.md`](tests/README.md):

- Name your single-instance mutex with `PEEKGUARD_MUTEX` (`scratch_mutex()`). Without it
  the spawned instance collides with a running PeekGuard, puts up a **modal** "already
  running" box, and sits there hiding nothing — which looks exactly like a guard bug.
- Parameterise anything that hides a window with `PEEKGUARD_MODE`. The three strategies
  fail in different ways, so a check that only ever runs `alpha` proves very little.

## Reporting a bug

Include the contents of `%APPDATA%\PeekGuard\peekguard.conf` and, if you can, a run
with `PEEKGUARD_DEBUG=1` — the log names the target's process, class, and title, which
is usually enough to see what the rule matched or missed.