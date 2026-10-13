"""Crash-safety test: a panic while a window is hidden must not strand it.

The other suites verify recovery after `kill -9`. This one covers the gentler and
more likely failure: the program itself panics (a bug, an unexpected API result).
That path runs through the panic hook, which is separate code from the normal
shutdown path, so it needs its own test.

Runs against the **release** binary on purpose: the release profile sets
`panic = "abort"`, and the whole point is to prove the hook still fires before the
abort. (Verified: panic hooks run during unwinding *and* before an abort.)

Two things this test refuses to do:

* assume the hide happened — the journal is polled as proof, and every
  "restored" claim is guarded by that proof, so it cannot pass vacuously;
* depend on window-focus races — binding goes through the rule scanner, which is
  deterministic, rather than through a hotkey that needs the target to be
  foreground at the right instant.

`PEEKGUARD_PANIC_AFTER` is a test-only hook compiled into the binary for exactly
this purpose.
"""
import ctypes as C
import os
import subprocess
import sys
import time
from ctypes import wintypes as W

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
from e2e_live import (  # noqa: E402
    MODE, TARGET_TITLE, find_target, is_hidden, rect, scratch_mutex, spawn_target,
    u32, wait_until,
)

ROOT = os.path.dirname(HERE)
RELEASE_EXE = os.path.join(ROOT, "target", "release", "peekguard.exe")
JOURNAL = os.path.join(os.environ.get("APPDATA", ""), "PeekGuard", "hidden.journal")

# Long enough that a healthy build hides the window well before this fires, short
# enough to keep the suite quick.
PANIC_AFTER_MS = 6000


def spawn(extra_env=None, rules=None):
    env = dict(os.environ)
    cfg = os.path.join(os.environ.get("TEMP", "."), "peekguard-test", "peekguard-panic.conf")
    os.makedirs(os.path.dirname(cfg), exist_ok=True)
    lines = [f"mode = {MODE}", "delay_ms = 150", "margin_px = 6"]
    lines += list(rules or [])
    with open(cfg, "w", encoding="utf-8") as f:
        f.write("\n".join(lines) + "\n")
    env["PEEKGUARD_CONFIG"] = cfg
    env["PEEKGUARD_MUTEX"] = scratch_mutex()
    env.update(extra_env or {})
    return subprocess.Popen([RELEASE_EXE], env=env)


def journal_text():
    try:
        with open(JOURNAL, encoding="utf-8", errors="replace") as f:
            return f.read().strip()
    except OSError:
        return ""


def journal_names(hwnd):
    return f"hwnd=0x{hwnd:x}" in journal_text()


def main():
    if not os.path.exists(RELEASE_EXE):
        print(f"SKIP: release binary not built at {RELEASE_EXE}")
        return 2
    if u32.GetForegroundWindow() == 0:
        print("SKIP: no foreground window (screen locked?)")
        return 2

    results = []

    def check(name, cond, detail=""):
        results.append((name, bool(cond)))
        print(f"  [{'PASS' if cond else 'FAIL'}] {name}" + (f"  -- {detail}" if detail else ""))

    start = W.POINT()
    u32.GetCursorPos(C.byref(start))
    print(f"release binary, mode under test: {MODE}")

    target = None
    pg = None
    try:
        target, hwnd = spawn_target("150")
        if not hwnd:
            print("FAIL: target window never appeared")
            return 1
        base = rect(hwnd)
        cx, cy = (base[0] + base[2]) // 2, (base[1] + base[3]) // 2
        cls = C.create_unicode_buffer(256)
        u32.GetClassNameW(hwnd, cls, 256)
        use_rule = [f"rule = class:{cls.value} | title:{TARGET_TITLE}"]
        print(f"   target hwnd={hwnd:#x} class={cls.value} rect={base}")

        # ------------------------------------------------ panic with a hidden window
        print("\n-- panic while hidden --")
        # Park the cursor outside so the rule-bound window is immediately hidden.
        u32.SetCursorPos(2, 2)
        # Clear any stale journal from earlier runs so "the journal names our
        # window" is real evidence and not leftovers.
        try:
            os.remove(JOURNAL)
        except OSError:
            pass

        t0 = time.monotonic()
        pg = spawn({"PEEKGUARD_PANIC_AFTER": str(PANIC_AFTER_MS)}, rules=use_rule)
        hidden_at = None
        while time.monotonic() - t0 < (PANIC_AFTER_MS / 1000.0) and pg.poll() is None:
            if journal_names(hwnd) and is_hidden(hwnd):
                hidden_at = time.monotonic() - t0
                break
            time.sleep(0.1)

        check("rule bound and hid the window", hidden_at is not None,
              f"t={hidden_at:.2f}s" if hidden_at else "never hidden")
        check("journal recorded the hidden window", journal_names(hwnd),
              journal_text()[:80] or "(empty)")

        rc = pg.wait(timeout=20)
        died_at = time.monotonic() - t0
        check("process died from the panic", rc != 0, f"rc={rc:#x}")
        check("panic fired after the hide, not before",
              hidden_at is not None and died_at > hidden_at,
              f"hidden@{hidden_at:.2f}s died@{died_at:.2f}s")

        # The whole point: the hook must have given the window back on the way out.
        # Guarded by `hidden_at` so this cannot pass without a real hide.
        restored = wait_until(lambda: not is_hidden(hwnd), 6.0)
        check("panic hook restored the window",
              hidden_at is not None and restored,
              "" if hidden_at else "no hide happened, so nothing was proven")
        check("panic hook cleared the journal", journal_text() == "",
              journal_text()[:80])
        check("window geometry intact after recovery", rect(hwnd) == base,
              f"{rect(hwnd)} vs {base}")

        pg = None
        time.sleep(0.5)

        # ------------------------------------------------ panic with nothing hidden
        print("\n-- panic with nothing hidden --")
        pg = spawn({"PEEKGUARD_PANIC_AFTER": "1500"})   # no rules: never binds
        rc = pg.wait(timeout=15)
        check("process died again", rc != 0, f"rc={rc:#x}")
        check("a visible window is left visible", not is_hidden(hwnd))
        check("geometry still untouched", rect(hwnd) == base, f"{rect(hwnd)} vs {base}")
    finally:
        u32.SetCursorPos(start.x, start.y)
        for proc in (pg, target):
            if proc is not None and proc.poll() is None:
                try:
                    proc.kill(); proc.wait(timeout=3)
                except Exception:
                    pass

    failed = [n for n, ok in results if not ok]
    print()
    if failed:
        print(f"RESULT: {len(results) - len(failed)}/{len(results)} passed")
        for n in failed:
            print(f"  FAILED: {n}")
        return 1
    print(f"RESULT: all {len(results)} checks passed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
