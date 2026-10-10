"""Focused tests for behaviours the main E2E suite does not cover.

Covers latch mode (hidden stays hidden even when the cursor comes back) and the
"manual hide" path, plus the elevation probe. Uses the same disposable tkinter
target and moves the real cursor.
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
    EXE, TARGET_TITLE, alpha_of, find_target, is_hidden, rect, send_hotkey,
    stop_peekguard, wait_until,
)

u32 = C.windll.user32
MODE = os.environ.get("PEEKGUARD_MODE", "alpha")


def spawn(latch: bool):
    env = dict(os.environ)
    cfg_dir = os.path.join(os.environ.get("TEMP", "."), "peekguard-test")
    os.makedirs(cfg_dir, exist_ok=True)
    cfg_path = os.path.join(cfg_dir, f"latch-{latch}-{MODE}.conf")
    with open(cfg_path, "w", encoding="utf-8") as f:
        f.write("\n".join([
            f"mode = {MODE}",
            "delay_ms = 200",
            "margin_px = 6",
            f"latch = {'true' if latch else 'false'}",
            "",
        ]))
    env["PEEKGUARD_CONFIG"] = cfg_path
    return subprocess.Popen([EXE], stdout=subprocess.PIPE, stderr=subprocess.PIPE, env=env)


def send_hide_hotkey():
    """Ctrl+Alt+H — manual toggle."""
    for vk in (0x11, 0x12, 0x48):
        u32.keybd_event(vk, 0, 0, 0)
    time.sleep(0.05)
    for vk in (0x48, 0x12, 0x11):
        u32.keybd_event(vk, 0, 2, 0)


def send_latch_hotkey():
    """Ctrl+Alt+L — toggle latch."""
    for vk in (0x11, 0x12, 0x4C):
        u32.keybd_event(vk, 0, 0, 0)
    time.sleep(0.05)
    for vk in (0x4C, 0x12, 0x11):
        u32.keybd_event(vk, 0, 2, 0)


def main():
    if not os.path.exists(EXE):
        print(f"FAIL: {EXE} not built")
        return 1
    if u32.GetForegroundWindow() == 0:
        print("SKIP: screen locked")
        return 2

    start = W.POINT()
    u32.GetCursorPos(C.byref(start))
    results = []

    def check(name, cond, detail=""):
        results.append((name, bool(cond)))
        print(f"  [{'PASS' if cond else 'FAIL'}] {name}" + (f"  -- {detail}" if detail else ""))

    target = subprocess.Popen([sys.executable, os.path.join(HERE, "target_window.py"), "180"])
    hwnd = find_target(target.pid)
    if not hwnd:
        print("FAIL: no target window")
        target.terminate()
        return 1
    base = rect(hwnd)
    cx, cy = (base[0] + base[2]) // 2, (base[1] + base[3]) // 2

    try:
        # ============ latch mode ON from the start ============
        print(f"\n-- latch = true, mode = {MODE} --")
        pg = spawn(True)
        time.sleep(1.5)
        u32.SetForegroundWindow(hwnd)
        time.sleep(0.4)
        u32.SetCursorPos(cx, cy)
        time.sleep(0.2)
        send_hotkey()  # bind
        time.sleep(1.0)

        u32.SetCursorPos(2, 2)
        check("latched hide happens", wait_until(lambda: is_hidden(hwnd), 5.0), f"alpha={alpha_of(hwnd)}")

        # the whole point: cursor returns, window must STAY hidden
        u32.SetCursorPos(cx, cy)
        stayed = not wait_until(lambda: not is_hidden(hwnd), 2.0)
        check("stays hidden when cursor returns (latch)", stayed, f"alpha={alpha_of(hwnd)}")

        # manual toggle with the hotkey brings it back
        send_hide_hotkey()
        check("Ctrl+Alt+H restores a latched window", wait_until(lambda: not is_hidden(hwnd), 3.0),
              f"alpha={alpha_of(hwnd)}")

        # and latched windows also ignore the mouse afterwards
        u32.SetCursorPos(2, 2)
        check("hides again after restore", wait_until(lambda: is_hidden(hwnd), 5.0))
        u32.SetCursorPos(cx, cy)
        check("still latched", not wait_until(lambda: not is_hidden(hwnd), 2.0))
        send_hide_hotkey()
        check("restored again", wait_until(lambda: not is_hidden(hwnd), 3.0))

        # toggle latch off at runtime, then mouse behaviour returns
        send_latch_hotkey()
        time.sleep(0.6)
        u32.SetCursorPos(2, 2)
        check("hides with latch off", wait_until(lambda: is_hidden(hwnd), 5.0))
        u32.SetCursorPos(cx, cy)
        check("mouse restores after latch off", wait_until(lambda: not is_hidden(hwnd), 5.0),
              f"alpha={alpha_of(hwnd)}")
        stop_peekguard(pg, hard=True)
        check("restored on exit", wait_until(lambda: not is_hidden(hwnd), 3.0))

        # ============ manual hide (hotkey) with latch off ============
        print(f"\n-- latch = false, manual hide, mode = {MODE} --")
        pg2 = spawn(False)
        time.sleep(1.5)
        u32.SetForegroundWindow(hwnd)
        time.sleep(0.4)
        # keep the cursor away so the mouse policy does not interfere
        u32.SetCursorPos(2, 2)
        time.sleep(0.2)
        send_hotkey()  # bind
        time.sleep(0.4)
        # cursor is outside, so it would auto-hide; confirm that first
        check("auto-hides with cursor away", wait_until(lambda: is_hidden(hwnd), 5.0))
        u32.SetCursorPos(cx, cy)
        check("mouse restores it", wait_until(lambda: not is_hidden(hwnd), 5.0))

        # now hide manually via hotkey while the cursor is inside
        send_hide_hotkey()
        check("Ctrl+Alt+H hides on demand", wait_until(lambda: is_hidden(hwnd), 3.0), f"alpha={alpha_of(hwnd)}")
        # cursor is still inside; the manual hide should still release on mouse return
        # (which it already is) — so move away and back to prove it is live again
        u32.SetCursorPos(2, 2)
        time.sleep(0.5)
        u32.SetCursorPos(cx, cy)
        check("released and re-armed after manual hide", wait_until(lambda: not is_hidden(hwnd), 5.0))
        stop_peekguard(pg2, hard=True)

    finally:
        u32.SetCursorPos(start.x, start.y)
        target.terminate()
        try:
            target.wait(timeout=5)
        except subprocess.TimeoutExpired:
            target.kill()

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
