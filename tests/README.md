# Tests

## Rust unit tests

    cargo test

17 tests, no side effects (plus one `#[ignore]`d diagnostic that asserts nothing). Covers config parsing, rule matching (including the
`process:` / `class:` / `title:` combinators), the scanner's rate limiting and
per-window suppression, autostart's registry-value naming, and the icon renderer.

## Live end-to-end tests

These spawn the real binary, create a disposable tkinter window, and **move the
real cursor** with `SetCursorPos`. They save and restore the original cursor
position, and skip themselves if the screen is locked.

    python tests/e2e_live.py        # bind, hide, restore, clean quit, crash recovery
    python tests/e2e_latch.py       # latch mode and manual (hotkey) hide
    python tests/e2e_rules.py       # rule-driven auto-protection + autostart registry
    python tests/e2e_robustness.py  # minimized / refusing / dying targets
    python tests/e2e_panic.py       # panic hook recovery (release binary)

Environment:

| Variable | Effect |
|---|---|
| `PEEKGUARD_MODE=alpha\|move\|hide` | which hide strategy to certify |
| `PEEKGUARD_DEBUG=1` | verbose log to stderr and `%APPDATA%\PeekGuard\debug.log` |
| `PEEKGUARD_CONFIG` | point at a throwaway config |
| `PEEKGUARD_MUTEX` | **test only** — name the single-instance mutex |
| `PEEKGUARD_AUTOSTART_VALUE` | registry value name to use instead of `PeekGuard` |
| `PEEKGUARD_REFUSE_HIDE=1` | **test only** — make every hide fail, standing in for an elevated target |
| `PEEKGUARD_PANIC_AFTER=<ms>` | **test only** — panic after N ms, to exercise the crash hook |

Each run writes its own config under `%TEMP%\peekguard-test\` so the modes do not
interfere with your real `peekguard.conf`.

Every suite names its own mutex via `PEEKGUARD_MUTEX` (`e2e_live.scratch_mutex()`).
Without that, a test instance collides with the PeekGuard you have running: it
loses the global single-instance mutex and puts up a **modal** "already running"
box, then sits there alive and hides nothing. That looks exactly like a guard bug
and hangs the suite on a desktop where nobody clicks OK. If you write a new suite,
use `scratch_mutex()` for every spawn.

### What the live suite proves

1. binding via the real `Ctrl+Alt+P` hotkey chord
2. the window stays visible while the cursor is inside
3. it really becomes invisible (alpha 0 / off-screen / `SW_HIDE`) once the cursor leaves
4. it comes back when the cursor returns to the *original* rect
5. geometry is unchanged after a round trip (catches drift, important for `move`)
6. a graceful quit restores the window and clears the journal
7. a hard kill while hidden leaves the window recoverable, and the next start restores it.
   The journal is asserted to exist *before* the kill, and the window is proven hidden
   while PeekGuard was alive: the journal is written before the window is touched, so
   "invisible with no record" — the one unrecoverable state — cannot occur
8. latch mode keeps the window hidden even when the cursor returns
9. `Ctrl+Alt+H` works while the cursor is parked on the window

### What the rules suite proves

1. a window matching `rule =` is guarded with no manual bind at all
2. it still hides/restores correctly under rule control
3. an explicit unbind releases *that* window and the scanner does not re-bind it
4. the scanner keeps working — a later matching window is still picked up
   (this is the check that caught the first, wrong design: unbind used to pause
   the whole scanner, which silently killed the rule)
5. a **minimized** match does not capture the guard. `hide()` cannot act on a
   minimized window, so if the scan picks one the single-window guard is parked
   there permanently and every other match goes unprotected. This check fails if
   the `is_minimized` skip is removed; it was written after exactly that bug was
   found with a real minimized application on the desktop.
6. the target dying does not leave a phantom binding
7. autostart writes a real `Run` entry pointing at the current exe, and removes it
   again — using a scratch value name so your own entry is never touched

### What the robustness suite proves

1. minimizing the bound window does not make it look like it is *refusing* to hide
   (both cases return the same "no" from `hide()`, and confusing them means a
   bogus admin-rights warning plus a permanent blacklisting of an innocent window)
2. a window that genuinely refuses is dropped after one report, not retried every
   tick — the count in the debug log is asserted directly
3. a refused window leaves no journal entry, so nothing tries to "restore" it later
   (the refusal hook fires *after* the journal write, like a real elevated target
   does, so this also covers the cleanup path)
4. the target closing while hidden leaves nothing stranded, and a new window
   matching the same rule is still protected

### What the panic suite proves

Runs the **release** binary, because the release profile uses `panic = "abort"` and
the question is precisely whether the hook still fires before the abort (it does).

1. a window is genuinely hidden first — the journal is polled as proof
2. the process then dies from the panic *after* that hide
3. the hook restored the window and cleared the journal on the way out
4. the geometry is intact afterwards
5. a second run panics with nothing hidden, and a visible window is left untouched

Check 1 is a guard, not decoration: an earlier version of this test passed all its
"window was restored" checks while never having hidden anything. Every recovery
claim is now gated on `hidden_at is not None` so the suite cannot pass vacuously.

## Writing new checks

`check()` records and prints; the exit code is non-zero if anything failed. Prefer
`wait_until(pred, timeout)` over fixed sleeps — the suites move the real cursor, so
timing is never perfectly deterministic. Always restore the cursor in a `finally`,
and always `PEEKGUARD_MODE`-parameterise anything that hides a window, since the
three strategies fail in different ways.
