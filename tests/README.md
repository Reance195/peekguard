# Tests

## Rust unit tests

    cargo test

Covers config parsing, rule matching and the icon renderer. No side effects.

## Live end-to-end tests

These spawn the real binary, create a disposable tkinter window, and **move the
real cursor** with `SetCursorPos`. They save and restore the original cursor
position, and skip themselves if the screen is locked.

    python tests/e2e_live.py     # bind, hide, restore, clean quit, crash recovery
    python tests/e2e_latch.py    # latch mode and manual (hotkey) hide

Environment:

| Variable | Effect |
|---|---|
| `PEEKGUARD_MODE=alpha\|move\|hide` | which hide strategy to certify |
| `PEEKGUARD_DEBUG=1` | verbose log to stderr and `%APPDATA%\PeekGuard\debug.log` |

Each run writes its own config under `%TEMP%\peekguard-test\` so the modes do not
interfere with your real `peekguard.conf`.

### What the live suite proves

1. binding via the real `Ctrl+Alt+P` hotkey chord
2. the window stays visible while the cursor is inside
3. it really becomes invisible (alpha 0 / off-screen / `SW_HIDE`) once the cursor leaves
4. it comes back when the cursor returns to the *original* rect
5. geometry is unchanged after a round trip (catches drift, important for `move`)
6. a graceful quit restores the window and clears the journal
7. a hard kill while hidden leaves the window recoverable, and the next start restores it
8. latch mode keeps the window hidden even when the cursor returns
9. `Ctrl+Alt+H` works while the cursor is parked on the window
