"""End-to-end tests for the features added after the first release.

1. Rule-driven auto-protection: a window matching `rule = process:...` gets
   guarded with no manual bind, is restored when it goes away, and the guard is
   re-acquired when a matching window appears again.
2. Explicit unbind pauses rule protection instead of letting the scanner
   immediately re-bind the same window.
3. Autostart writes and removes a real HKCU Run entry (under a scratch value
   name, so the developer's own entry is never touched).

Uses the same disposable tkinter target as the other suites.
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
    EXE, MESSAGE_CLASS, MODE, TARGET_TITLE, find_target, find_message_window,
    focus_window, is_hidden, rect, spawn_target,
    scratch_mutex, spawn_peekguard as _spawn, stop_peekguard, u32,
    wait_until,
)

WM_COMMAND = 0x0111
CMD_UNBIND_ALL = 131
CMD_QUIT = 199
CMD_BIND = 100

TEST_VALUE = "PeekGuardTestEntry"
RUN_KEY = r"Software\Microsoft\Windows\CurrentVersion\Run"

adv = C.windll.advapi32
adv.RegOpenKeyExW.argtypes = [W.HKEY, C.c_wchar_p, W.DWORD, W.DWORD, C.POINTER(W.HKEY)]
adv.RegQueryValueExW.argtypes = [W.HKEY, C.c_wchar_p, C.c_void_p, C.POINTER(W.DWORD),
                                 C.c_void_p, C.POINTER(W.DWORD)]
adv.RegSetValueExW.argtypes = [W.HKEY, C.c_wchar_p, W.DWORD, W.DWORD, C.c_void_p, W.DWORD]
adv.RegDeleteValueW.argtypes = [W.HKEY, C.c_wchar_p]
adv.RegCloseKey.argtypes = [W.HKEY]
HKCU = W.HKEY(0x80000001)
KEY_ALL_ACCESS = 0xF003F


def read_run_value(name):
    key = W.HKEY()
    if adv.RegOpenKeyExW(HKCU, RUN_KEY, 0, KEY_ALL_ACCESS, C.byref(key)) != 0:
        return None
    try:
        buf = C.create_unicode_buffer(1024)
        n = W.DWORD(C.sizeof(buf))
        ty = W.DWORD()
        rc = adv.RegQueryValueExW(key, name, None, C.byref(ty), buf, C.byref(n))
        return buf.value if rc == 0 else None
    finally:
        adv.RegCloseKey(key)


def spawn_with_rules(cfg_lines):
    """Spawn peekguard with a config written exactly as given."""
    env = dict(os.environ)
    env["PEEKGUARD_CONFIG"] = cfg_path = os.path.join(
        os.environ.get("TEMP", "."), "peekguard-test", "peekguard-rules.conf")
    os.makedirs(os.path.dirname(cfg_path), exist_ok=True)
    with open(cfg_path, "w", encoding="utf-8") as f:
        f.write("\n".join(cfg_lines + [""]))
    env["PEEKGUARD_MUTEX"] = scratch_mutex()
    return subprocess.Popen([EXE], stdout=subprocess.PIPE, stderr=subprocess.PIPE, env=env)


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

    target_proc = None
    pg = None
    try:
        # ---------------------------------------------------------- rule scanning
        print("\n-- rule auto-protection --")
        target_proc, hwnd = spawn_target("180")
        if not hwnd:
            print("FAIL: target window never appeared")
            return 1
        base = rect(hwnd)
        cx, cy = (base[0] + base[2]) // 2, (base[1] + base[3]) // 2

        # The rule keys off the test process name, which is `python.exe`; that is
        # ambiguous on a dev box, so use the window class instead.
        cls = C.create_unicode_buffer(256)
        u32.GetClassNameW(hwnd, cls, 256)
        # Scoped to this target by title as well as class: `TkTopLevel` alone
        # also matches other tkinter apps that may be open on the desktop.
        rule = f"rule = class:{cls.value} | title:{TARGET_TITLE}"
        print(f"   rule: {rule}")

        # No hotkey, no bind: the scanner must find the window on its own.
        #
        # The target starts *minimized* and PeekGuard is launched before any
        # hideable match exists, because that is the arrangement that used to starve
        # the rule: the scanner bought the minimized handle — which `hide()` cannot
        # act on — and parked the single-window guard there for good, leaving the
        # window the user was actually working in unprotected. The scan must skip
        # the minimized candidate and take the hideable one when it shows up.
        u32.ShowWindow(hwnd, 0x6)  # SW_MINIMIZE
        time.sleep(0.5)

        pg = spawn_with_rules(["mode = alpha", "delay_ms = 200", "margin_px = 6", rule])
        time.sleep(2.5)
        check("peekguard started with rules", pg.poll() is None)

        other, hwnd_other = spawn_target("120")
        check("a visible window matching the same rule exists", hwnd_other != 0,
              f"hwnd={hwnd_other:#x}")
        u32.SetCursorPos(2, 2)
        check(
            "a minimized match does not starve a visible one",
            wait_until(lambda: is_hidden(hwnd_other), 10.0),
            f"minimized={hwnd:#x} visible={hwnd_other:#x}",
        )
        # Back to a single visible target for the rest of the flow.
        if other.poll() is None:
            other.kill()
            other.wait(timeout=5)
        u32.ShowWindow(hwnd, 0x9)  # SW_RESTORE
        focus_window(hwnd)
        time.sleep(2.5)

        u32.SetCursorPos(cx, cy)
        time.sleep(1.5)
        u32.SetCursorPos(2, 2)
        check(
            "rule-matched window hides with no manual bind",
            wait_until(lambda: is_hidden(hwnd), 8.0),
            f"alpha={hwnd}",
        )

        u32.SetCursorPos(cx, cy)
        check(
            "and restores when the cursor returns",
            wait_until(lambda: not is_hidden(hwnd), 5.0),
        )

        # An explicit unbind must win: the scanner should not re-bind it.
        u32.SetCursorPos(2, 2)
        wait_until(lambda: is_hidden(hwnd), 5.0)
        msg = find_message_window()
        check("found peekguard message window", msg != 0)
        if msg:
            u32.PostMessageW(msg, WM_COMMAND, CMD_UNBIND_ALL, 0)
            time.sleep(0.5)
            check(
                "explicit unbind restores the window",
                wait_until(lambda: not is_hidden(hwnd), 4.0),
            )
            # Sit still well past several scan intervals: a re-bind would hide it.
            u32.SetCursorPos(2, 2)
            time.sleep(3.0)
            check("released window is not re-bound by the rule", not is_hidden(hwnd),
                  f"alpha={hwnd}")

        # Closing the target must not leave a phantom binding behind.
        target_proc.terminate()
        target_proc.wait(timeout=5)
        time.sleep(2.0)
        check("survives its target disappearing", pg.poll() is None)

        # A fresh matching window should be picked up again.
        target_proc, hwnd2 = spawn_target("120")
        check("second target appeared", hwnd2 != 0)
        if hwnd2:
            u32.SetCursorPos(2, 2)
            check(
                "re-acquires protection for a new matching window",
                wait_until(lambda: is_hidden(hwnd2), 8.0),
            )

        u32.SetCursorPos(start.x, start.y)
        if pg and pg.poll() is None:
            stop_peekguard(pg, hard=True)
        pg = None
        time.sleep(0.5)

        # -------------------------------------------------------------- autostart
        print("\n-- autostart registry entry --")
        adv.RegDeleteValueW(W.HKEY(0x80000001), TEST_VALUE)  # clean slate
        env = dict(os.environ)
        env["PEEKGUARD_AUTOSTART_VALUE"] = TEST_VALUE
        # No CLI for this; drive it through the tray command by posting WM_COMMAND.
        env["PEEKGUARD_CONFIG"] = os.path.join(
            os.environ.get("TEMP", "."), "peekguard-test", "peekguard-startup.conf")
        # Kept out of the user's single-instance mutex; see e2e_live.scratch_mutex.
        env["PEEKGUARD_MUTEX"] = scratch_mutex()
        p = subprocess.Popen([EXE], stdout=subprocess.PIPE, stderr=subprocess.PIPE, env=env)
        time.sleep(2.0)
        m = find_message_window()
        check("peekguard up for autostart test", p.poll() is None and m != 0)
        if m:
            check("entry absent before toggling", read_run_value(TEST_VALUE) is None)
            u32.PostMessageW(m, WM_COMMAND, 160, 0)  # CMD_AUTOSTART
            time.sleep(1.0)
            val = read_run_value(TEST_VALUE)
            check("autostart writes a Run entry", val is not None, repr(val))
            check("entry points at this exe",
                  bool(val) and "peekguard" in val.lower(), repr(val))
            u32.PostMessageW(m, WM_COMMAND, 160, 0)
            time.sleep(1.0)
            check("autostart removes the entry again",
                  read_run_value(TEST_VALUE) is None)
        stop_peekguard(p, hard=True)
        adv.RegDeleteValueW(W.HKEY(0x80000001), TEST_VALUE)
    finally:
        u32.SetCursorPos(start.x, start.y)
        for proc in (locals().get("other"), pg, target_proc):
            if proc is not None and proc.poll() is None:
                try:
                    proc.kill()
                    proc.wait(timeout=3)
                except Exception:
                    pass
        try:
            adv.RegDeleteValueW(W.HKEY(0x80000001), TEST_VALUE)
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
