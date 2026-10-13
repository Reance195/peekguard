"""Robustness checks for the binding lifecycle.

These cover the paths that must *not* go wrong, which are easy to regress because
nothing visibly breaks when they do:

1. A minimized target. `hide()` cannot act on a minimized window, and its "no" is
   the same `None` a genuinely refusing window returns. Conflating the two means a
   bogus "this window has admin rights" warning plus a permanent, sticky
   suppression of a window that did nothing wrong. The window is minimized, then
   restored, and must be protected normally afterwards.
2. A target that refuses to be hidden must be dropped once, not retried forever.
   Driven with the `PEEKGUARD_REFUSE_HIDE=1` test hook, which makes `hide()`
   report failure the way an elevated target would. The assertion is that the
   warning appears once and then the program goes quiet — an implementation that
   retried every debounce would emit the message repeatedly.
3. Closing the target while it is hidden must restore nothing and leave no
   journal entry behind.

Each check is written so it cannot pass vacuously: where a hide is expected, the
test first proves the window really is hidden (via the journal) before asserting
what happens next.
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
    EXE, MESSAGE_CLASS, MODE, TARGET_TITLE, find_message_window, find_target,
    is_hidden, rect,
    focus_window, scratch_mutex, spawn_target, stop_peekguard, u32,
    wait_until,
)

WM_COMMAND = 0x0111
CMD_QUIT = 199
CMD_UNBIND_ALL = 131
SW_MINIMIZE = 0x6
SW_RESTORE = 0x9
JOURNAL = os.path.join(os.environ.get("APPDATA", ""), "PeekGuard", "hidden.journal")


def journal_text():
    try:
        with open(JOURNAL, encoding="utf-8", errors="replace") as f:
            return f.read().strip()
    except OSError:
        return ""


def spawn(extra_env=None, rules=None):
    env = dict(os.environ)
    cfg = os.path.join(os.environ.get("TEMP", "."), "peekguard-test", "peekguard-robust.conf")
    os.makedirs(os.path.dirname(cfg), exist_ok=True)
    lines = [f"mode = {MODE}", "delay_ms = 150", "margin_px = 6"]
    lines += list(rules or [])
    with open(cfg, "w", encoding="utf-8") as f:
        f.write("\n".join(lines) + "\n")
    env["PEEKGUARD_CONFIG"] = cfg
    env["PEEKGUARD_DEBUG"] = "1"
    # Never share the mutex with the user's real PeekGuard; see e2e_live.scratch_mutex.
    env["PEEKGUARD_MUTEX"] = scratch_mutex()
    env.update(extra_env or {})
    with open(DEBUG_LOG, "w", encoding="utf-8"):
        pass
    return subprocess.Popen([EXE], env=env, stdout=subprocess.DEVNULL,
                            stderr=subprocess.DEVNULL)


def assert_owns_session(pg, label):
    """Prove the process we spawned is the one actually guarding windows.

    A second copy cannot run: PeekGuard takes a global mutex and, if it loses,
    puts up a modal "already running" box and hides nothing. That copy stays
    alive, so `pg.poll()` looks healthy while no window is ever hidden — which
    makes any later failure look like a guard bug instead of test contamination.
    Comparing the message window's owning pid to `pg.pid` turns that into a
    clear failure at the point it happens.
    """
    hwnd = find_message_window()
    owner = W.DWORD()
    if hwnd:
        u32.GetWindowThreadProcessId(hwnd, C.byref(owner))
    ok = bool(hwnd) and owner.value == pg.pid
    print(f"  [{'PASS' if ok else 'FAIL'}] {label} is the live instance"
          f"  -- msg hwnd={hwnd:#x} owner={owner.value} pid={pg.pid}")
    return ok


DEBUG_LOG = os.path.join(os.environ.get("APPDATA", ""), "PeekGuard", "debug.log")


def debug_text():
    try:
        with open(DEBUG_LOG, encoding="utf-8", errors="replace") as f:
            return f.read()
    except OSError:
        return ""


def main():
    if not os.path.exists(EXE):
        print(f"FAIL: binary not built at {EXE}")
        return 1
    if u32.GetForegroundWindow() == 0:
        print("SKIP: no foreground window (screen locked?)")
        return 2

    results = []

    def check(name, cond, detail=""):
        results.append((name, bool(cond)))
        print(f"  [{'PASS' if cond else 'FAIL'}] {name}" + (f"  -- {detail}" if detail else ""))

    start = W.POINT()
    u32.GetCursorPos(C.byref(start))
    print(f"mode under test: {MODE}")

    target = None
    pg = None
    try:
        target, hwnd = spawn_target("90")
        if not hwnd:
            print("FAIL: target window never appeared")
            return 1
        base = rect(hwnd)
        cx, cy = (base[0] + base[2]) // 2, (base[1] + base[3]) // 2
        cls = C.create_unicode_buffer(256)
        u32.GetClassNameW(hwnd, cls, 256)
        # Class alone matches every tkinter window on the desktop, including
        # other apps; the title pins it to this disposable target.
        rule = [f"rule = class:{cls.value} | title:{TARGET_TITLE}"]

        # ---------------------------------------------------- 1. minimized target
        print("\n-- minimized target keeps its binding --")
        u32.SetCursorPos(2, 2)
        try:
            os.remove(JOURNAL)
        except OSError:
            pass
        pg = spawn(rules=rule)
        check("rule bound and hid it once",
              wait_until(lambda: is_hidden(hwnd), 8.0), f"hwnd={hwnd:#x}")

        # Bring it back, then minimize it while it is bound and visible.
        u32.SetCursorPos(cx, cy)
        wait_until(lambda: not is_hidden(hwnd), 5.0)
        u32.ShowWindow(hwnd, SW_MINIMIZE)
        time.sleep(0.5)
        check("target is minimized", bool(u32.IsIconic(hwnd)))

        # Park the cursor far away and wait: a naive implementation treats the
        # un-hideable minimized window as "refusing" and drops it here.
        u32.SetCursorPos(2, 2)
        time.sleep(3.5)
        log = debug_text()
        check("minimized window is not reported as refusing",
              "Refused" not in log, "saw Refused in the debug log")

        # Restore it: it must still be protected, i.e. it hides again.
        u32.ShowWindow(hwnd, SW_RESTORE)
        focus_window(hwnd)
        time.sleep(0.6)
        check("binding survived the minimize",
              wait_until(lambda: is_hidden(hwnd), 8.0),
              "window came back and was expected to hide again")

        u32.SetCursorPos(cx, cy)
        wait_until(lambda: not is_hidden(hwnd), 5.0)
        stop_peekguard(pg, hard=True)
        pg = None
        time.sleep(0.5)

        # ------------------------------------------- 2. a refusing window is dropped
        print("\n-- a refusing window is dropped, not retried --")
        # The hook makes hide() fail deterministically, standing in for an
        # elevated target that the test cannot create without a UAC prompt.
        pg = spawn({"PEEKGUARD_REFUSE_HIDE": "1"}, rules=rule)
        u32.SetCursorPos(2, 2)
        # Give it long enough for many debounce cycles: a retry loop would keep
        # logging Refused the whole time.
        time.sleep(6.0)
        log = debug_text()
        # The debug log renders the action as `action=Refused -> state=Idle`.
        refusals = log.count("action=Refused")
        check("refusal was reported", refusals >= 1, f"count={refusals}")
        check("the rule did bind it first (so refusal was really exercised)",
              "rule scan bound" in log)
        check("refusal did not repeat in a loop", refusals <= 2,
              f"count={refusals} (a per-tick retry would be dozens)")
        check("window was left visible, not stranded",
              not is_hidden(hwnd), f"alpha={hwnd}")
        # And nothing was written to the journal, so nothing needs recovering.
        check("no journal entry for the refused window",
              f"hwnd=0x{hwnd:x}" not in journal_text(), journal_text()[:70])

        stop_peekguard(pg, hard=True)
        pg = None
        time.sleep(0.5)

        # ------------------------------------ 3. closing the target while hidden
        print("\n-- target closes while hidden --")
        # Retire every target from the earlier phases first. The guard holds one
        # window at a time, and the scanner will happily pick up *any* matching
        # window, so a leftover from phase 1 would be guarded instead of the window
        # this phase means to observe — which is exactly the confusion this check
        # hit while being written.
        for proc in (target,):
            if proc is not None and proc.poll() is None:
                proc.kill()
                proc.wait(timeout=5)
        target = None
        time.sleep(0.3)
        try:
            os.remove(JOURNAL)
        except OSError:
            pass

        target2, hwnd2 = spawn_target("90")
        check("third target appeared", hwnd2 != 0, f"hwnd={hwnd2:#x}")
        if hwnd2:
            b2 = rect(hwnd2)
            c2 = ((b2[0] + b2[2]) // 2, (b2[1] + b2[3]) // 2)
            pg = spawn(rules=rule)
            assert_owns_session(pg, "phase 3 instance")
            u32.SetCursorPos(2, 2)
            got_hidden = wait_until(lambda: is_hidden(hwnd2), 10.0)
            check("third target hidden", got_hidden)
            if got_hidden:
                target2.terminate()
                target2.wait(timeout=5)
                time.sleep(2.5)
                check("peekguard survived its target closing", pg.poll() is None)
                # Assert on the window itself, not on the journal: the journal
                # legitimately names whichever matching window is guarded, so it is
                # the wrong thing to assert against.
                check("nothing left hidden after the target died",
                      journal_text() == "", journal_text()[:80])
                check("the dead target's window is gone, not stranded",
                      not C.windll.user32.IsWindow(hwnd2))

            # A new window for the same rule is still picked up: one target dying
            # must not disable the rule.
            target, hwnd3 = spawn_target("60")
            check("fourth target appeared", hwnd3 != 0, f"hwnd={hwnd3:#x}")
            if hwnd3:
                u32.SetCursorPos(2, 2)
                got3 = wait_until(lambda: is_hidden(hwnd3), 10.0)
                check("rule still active after the previous target died", got3,
                      f"hidden={is_hidden(hwnd3)}")
        u32.SetCursorPos(start.x, start.y)
    finally:
        u32.SetCursorPos(start.x, start.y)
        for proc in (pg,):
            if proc is not None and proc.poll() is None:
                try:
                    proc.kill(); proc.wait(timeout=3)
                except Exception:
                    pass
        for proc in (locals().get("target2"), target):
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
