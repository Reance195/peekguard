"""End-to-end test for PeekGuard against a real, disposable window.

Covers the whole contract:
  1. bind a window via the Ctrl+Alt+P hotkey
  2. visible while the cursor is inside
  3. hidden after the cursor leaves (past the debounce)
  4. restored when the cursor comes back to the same rect, geometry intact
  5. a *graceful* quit restores the window
  6. a *hard kill* while hidden still leaves the window recoverable on next start

The test moves the real cursor (SetCursorPos). It saves and restores the original
position. Set PEEKGUARD_DEBUG=1 for verbose output.
"""
import ctypes as C
import itertools
import os
import subprocess
import sys
import time
from ctypes import wintypes as W

u32 = C.windll.user32
k32 = C.windll.kernel32

GWL_EXSTYLE = -20
WS_EX_LAYERED = 0x00080000
WM_COMMAND = 0x0111
CMD_QUIT = 199
SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN = 76, 77
SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN = 78, 79

u32.GetWindowLongPtrW.argtypes = [W.HWND, C.c_int]
u32.GetWindowLongPtrW.restype = C.c_ssize_t
u32.SetCursorPos.argtypes = [C.c_int, C.c_int]
u32.GetLayeredWindowAttributes.argtypes = [
    W.HWND, C.POINTER(W.DWORD), C.POINTER(C.c_ubyte), C.POINTER(W.DWORD)
]
u32.FindWindowW.argtypes = [C.c_wchar_p, C.c_wchar_p]
u32.FindWindowW.restype = W.HWND
u32.PostMessageW.argtypes = [W.HWND, C.c_uint, C.c_size_t, C.c_ssize_t]
u32.GetWindowThreadProcessId.argtypes = [W.HWND, C.POINTER(W.DWORD)]
u32.GetWindowThreadProcessId.restype = W.DWORD
u32.AttachThreadInput.argtypes = [W.DWORD, W.DWORD, W.BOOL]
u32.AttachThreadInput.restype = W.BOOL
u32.BringWindowToTop.argtypes = [W.HWND]
u32.BringWindowToTop.restype = W.BOOL
k32.GetCurrentThreadId.restype = W.DWORD

# Every suite's PeekGuard spawn must take its own single-instance mutex. Sharing
# the name with the user's real PeekGuard means the spawned copy loses the mutex,
# puts up a *modal* "already running" box, and then sits there alive but hiding
# nothing — which reads as a guard bug and, on a desktop with no one to click OK,
# hangs the suite outright. A counter keeps concurrently live instances distinct
# too, so a just-killed one that has not released yet cannot block the next.
_mutex_seq = itertools.count()


def scratch_mutex():
    """A mutex name private to this test process and this spawn."""
    return f"PeekGuard-Test-{os.getpid()}-{next(_mutex_seq)}"

HERE = os.path.dirname(os.path.abspath(__file__))
EXE = os.path.join(os.path.dirname(HERE), "target", "debug", "peekguard.exe")
TARGET_TITLE = "PeekGuard Test Target"
MESSAGE_CLASS = "PeekGuard.MessageWindow.v1"
DEBUG = os.environ.get("PEEKGUARD_DEBUG", "") == "1"
MODE = os.environ.get("PEEKGUARD_MODE", "alpha")
JOURNAL = os.path.join(os.environ.get("APPDATA", ""), "PeekGuard", "hidden.journal")


# ------------------------------------------------------------------ helpers

def rect(h):
    r = W.RECT()
    u32.GetWindowRect(h, C.byref(r))
    return (r.left, r.top, r.right, r.bottom)


def alpha_of(h):
    key, alpha, flags = W.DWORD(0), C.c_ubyte(0), W.DWORD(0)
    if not u32.GetLayeredWindowAttributes(h, C.byref(key), C.byref(alpha), C.byref(flags)):
        return None
    return alpha.value if flags.value & 2 else None


def exstyle(h):
    return u32.GetWindowLongPtrW(h, GWL_EXSTYLE) & 0xFFFFFFFF


def visible(h):
    return bool(u32.IsWindowVisible(h))


def is_hidden(h):
    """Hidden if invisible, fully transparent, or parked off the virtual screen."""
    if not visible(h):
        return True
    if alpha_of(h) == 0:
        return True
    r = rect(h)
    vx, vy = u32.GetSystemMetrics(SM_XVIRTUALSCREEN), u32.GetSystemMetrics(SM_YVIRTUALSCREEN)
    vw, vh = u32.GetSystemMetrics(SM_CXVIRTUALSCREEN), u32.GetSystemMetrics(SM_CYVIRTUALSCREEN)
    return r[2] <= vx or r[0] >= vx + vw or r[3] <= vy or r[1] >= vy + vh


def _enum(pid, css=0):
    """Find top-level windows of `pid`; css=1 matches class, else title substring."""
    out = []
    EP = C.WINFUNCTYPE(C.c_bool, W.HWND, W.LPARAM)

    def cb(h, lp):
        p = W.DWORD(0)
        u32.GetWindowThreadProcessId(h, C.byref(p))
        if p.value != pid:
            return True
        n = u32.GetWindowTextLengthW(h)
        b = C.create_unicode_buffer(n + 1)
        u32.GetWindowTextW(h, b, n + 1)
        cls = C.create_unicode_buffer(256)
        u32.GetClassNameW(h, cls, 256)
        if css:
            if cls.value == TARGET_TITLE or (css == 1 and cls.value.startswith("PeekGuard.")):
                out.append(h)
        elif TARGET_TITLE in b.value:
            out.append(h)
        return True

    u32.EnumWindows(EP(cb), 0)
    return out


def find_target(pid, timeout=8.0):
    end = time.time() + timeout
    while time.time() < end:
        w = _enum(pid, css=0)
        if w:
            return w[0]
        time.sleep(0.2)
    return 0


def spawn_target(lifetime="120", tries=3):
    """Spawn a disposable target window and wait for it to really exist.

    Returns `(proc, hwnd)` with a non-zero hwnd, or `(None, 0)` if the window never
    appeared after `tries` attempts. Drivers should treat a zero hwnd as a hard
    failure of the phase rather than continuing without a window: every check below
    would otherwise pass vacuously against a handle that is not there.
    """
    for _ in range(tries):
        proc = subprocess.Popen(
            [sys.executable, os.path.join(HERE, "target_window.py"), str(lifetime)])
        hwnd = find_target(proc.pid)
        if hwnd:
            return proc, hwnd
        stop_peekguard(proc, hard=True)
    return None, 0


def find_message_window(timeout=5.0):
    """The app window is message-only, so EnumWindows cannot see it: use FindWindowW."""
    cls = C.create_unicode_buffer(MESSAGE_CLASS)
    end = time.time() + timeout
    while time.time() < end:
        h = u32.FindWindowW(cls, None)
        if h:
            return h
        time.sleep(0.1)
    return 0


def wait_until(pred, timeout=5.0, step=0.1):
    end = time.time() + timeout
    while time.time() < end:
        if pred():
            return True
        time.sleep(step)
    return False


def send_hotkey():
    """Ctrl+Alt+P is read by PeekGuard's RegisterHotKey; synthesise the chord."""
    for vk in (0x11, 0x12, 0x50):
        u32.keybd_event(vk, 0, 0, 0)
    time.sleep(0.05)
    for vk in (0x50, 0x12, 0x11):
        u32.keybd_event(vk, 0, 2, 0)


def focus_window(hwnd, tries=6):
    """Give `hwnd` the foreground, retrying until Windows actually agrees.

    `SetForegroundWindow` is allowed to fail: a background process is not normally
    permitted to steal focus, and the caller is a background process. The standard
    way around it is to attach to the current foreground thread's input queue, which
    makes the focus change legal, then detach. Retrying covers the rest (the lock can
    persist for a moment after a process exits).

    Without this the suites are intermittently wrong for a reason that has nothing to
    do with PeekGuard: a missed focus change breaks the binding hotkey, which then
    surfaces as a bogus "not hidden" failure further down. Returns whether the window
    really did become foreground, so a caller can report it honestly.
    """
    for _ in range(tries):
        if u32.GetForegroundWindow() == hwnd:
            return True
        u32.SetForegroundWindow(hwnd)
        if u32.GetForegroundWindow() == hwnd:
            return True
        # Attach to the foreground thread's input queue so the change is permitted.
        fg = u32.GetForegroundWindow()
        target_thread = u32.GetWindowThreadProcessId(fg, None)
        this_thread = k32.GetCurrentThreadId()
        if target_thread and target_thread != this_thread:
            if u32.AttachThreadInput(this_thread, target_thread, True):
                try:
                    u32.SetForegroundWindow(hwnd)
                    u32.BringWindowToTop(hwnd)
                finally:
                    u32.AttachThreadInput(this_thread, target_thread, False)
        if u32.GetForegroundWindow() == hwnd:
            return True
        time.sleep(0.25)
    return u32.GetForegroundWindow() == hwnd


def bind_target(hwnd, tries=5):
    """Bind `hwnd` with the real hotkey chord and confirm the bind actually took.

    A synthesised chord can be dropped (another process has the combination, the
    input queue coalesces it, the window is not foreground yet). Left unchecked that
    turns into a *later* failure that looks like something else entirely: a test that
    never bound cannot hide anything, so the crash-recovery checks then fail on a
    missing journal entry even though recovery is fine. Confirming the bind keeps the
    hotkey's flakiness out of the checks that are not about the hotkey.

    Verified through `is_hidden` rather than the log, because "it hid" is the thing
    the rest of the suite depends on.
    """
    for _ in range(tries):
        focus_window(hwnd)
        time.sleep(0.3)
        u32.SetCursorPos(2, 2)  # park away, so a successful bind hides promptly
        send_hotkey()
        if wait_until(lambda: is_hidden(hwnd), 2.5):
            return True
    return False


def spawn_peekguard():
    env = dict(os.environ)
    if DEBUG:
        env["PEEKGUARD_DEBUG"] = "1"
    # isolate each run's config so modes can be driven from the environment
    cfg_dir = os.path.join(os.environ.get("TEMP", "."), "peekguard-test")
    os.makedirs(cfg_dir, exist_ok=True)
    cfg_path = os.path.join(cfg_dir, f"peekguard-{MODE}.conf")
    with open(cfg_path, "w", encoding="utf-8") as f:
        f.write("\n".join([f"mode = {MODE}", "delay_ms = 200", "margin_px = 6", "latch = false", ""]))
    env["PEEKGUARD_CONFIG"] = cfg_path
    env["PEEKGUARD_MUTEX"] = scratch_mutex()
    return subprocess.Popen([EXE], stdout=subprocess.PIPE, stderr=subprocess.PIPE, env=env)


def stop_peekguard(p, hard=False, timeout=6):
    try:
        if hard:
            p.kill()
        else:
            p.terminate()
        p.wait(timeout=timeout)
    except subprocess.TimeoutExpired:
        p.kill()
        try:
            p.wait(timeout=3)
        except subprocess.TimeoutExpired:
            pass


# ------------------------------------------------------------------ main

def main():
    if not os.path.exists(EXE):
        print(f"FAIL: binary not built at {EXE}")
        return 1
    if u32.GetForegroundWindow() == 0:
        print("SKIP: no foreground window (screen locked?)")
        return 2

    start = W.POINT()
    u32.GetCursorPos(C.byref(start))
    print(f"saved cursor ({start.x},{start.y})")
    print(f"mode under test: {MODE}")

    results = []

    def check(name, cond, detail=""):
        results.append((name, bool(cond)))
        print(f"  [{'PASS' if cond else 'FAIL'}] {name}" + (f"  -- {detail}" if detail else ""))

    target_proc, hwnd = spawn_target("180")
    if not hwnd:
        print("FAIL: target window never appeared")
        target_proc.terminate()
        return 1

    pg = spawn_peekguard()
    time.sleep(1.5)
    if pg.poll() is not None:
        out, err = pg.communicate()
        print("FAIL: peekguard exited immediately")
        print(err.decode(errors="replace"))
        target_proc.terminate()
        return 1

    try:
        base = rect(hwnd)
        cx, cy = (base[0] + base[2]) // 2, (base[1] + base[3]) // 2
        print(f"target hwnd={hwnd:#x} rect={base} alpha={alpha_of(hwnd)} exstyle={exstyle(hwnd):#x}")

        # --- 1. bind via hotkey
        focus_window(hwnd)
        time.sleep(0.5)
        check("target is foreground", u32.GetForegroundWindow() == hwnd)
        u32.SetCursorPos(cx, cy)
        time.sleep(0.3)
        check("bind hotkey took effect", bind_target(hwnd), "retries the chord until the window hides once")
        u32.SetCursorPos(cx, cy)
        check(
            "restored when cursor returns after binding",
            wait_until(lambda: not is_hidden(hwnd), 5.0),
            f"alpha={alpha_of(hwnd)}",
        )
        check("peekguard still alive after bind hotkey", pg.poll() is None)

        # --- 2. visible while cursor inside
        u32.SetCursorPos(cx, cy)
        time.sleep(0.8)
        check("visible while cursor inside", not is_hidden(hwnd), f"alpha={alpha_of(hwnd)}")

        # --- 3. hidden after the cursor leaves
        u32.SetCursorPos(2, 2)
        check(
            "hidden after cursor leaves",
            wait_until(lambda: is_hidden(hwnd), 5.0),
            f"alpha={alpha_of(hwnd)}",
        )

        # --- 4. restored when the cursor comes back
        u32.SetCursorPos(cx, cy)
        check(
            "restored when cursor returns",
            wait_until(lambda: not is_hidden(hwnd), 5.0),
            f"alpha={alpha_of(hwnd)}",
        )
        check("rect unchanged after round trip", rect(hwnd) == base, f"{rect(hwnd)} vs {base}")

        # --- 5. graceful quit while hidden must restore the window
        u32.SetCursorPos(2, 2)
        hide_a = wait_until(lambda: is_hidden(hwnd), 5.0)
        check("hidden again before graceful-quit test", hide_a, f"alpha={alpha_of(hwnd)}")
        if hide_a:
            msg = find_message_window()
            check("found peekguard message window", msg != 0, f"{msg:#x}" if msg else "")
            if msg:
                u32.PostMessageW(msg, WM_COMMAND, CMD_QUIT, 0)
                stopped = wait_until(lambda: pg.poll() is not None, 6.0)
                check("peekguard exited on quit command", stopped)
                if not stopped:
                    stop_peekguard(pg)
                check(
                    "graceful quit restored the window",
                    wait_until(lambda: not is_hidden(hwnd), 3.0),
                    f"alpha={alpha_of(hwnd)}",
                )
                check("journal cleared after clean exit", not os.path.exists(JOURNAL))

        # --- 6. hard kill while hidden must be recoverable on next start
        pg2 = spawn_peekguard()
        time.sleep(1.5)
        check("peekguard restarted", pg2.poll() is None)
        # Rebind: a restart does not remember bindings. Confirmed rather than assumed,
        # otherwise a dropped hotkey shows up later as a bogus recovery failure.
        check("rebound for the crash test", bind_target(hwnd), f"alpha={alpha_of(hwnd)}")
        check("hidden again before crash test", is_hidden(hwnd), f"alpha={alpha_of(hwnd)}")
        # Recovery is only meaningful if there is something to recover, and the
        # journal is that something: it is the sole record of "this window is hidden
        # and must be put back". Establish it explicitly while PeekGuard is still
        # alive rather than checking for it after the kill, so this is a real
        # precondition and not a race against the tick that writes it. A journal that
        # never appears here is a genuine defect (the window would be stranded), and
        # this check names it plainly instead of letting it surface as a confusing
        # "recovery failed" further down.
        check("the hide was journalled before the crash",
              wait_until(lambda: os.path.exists(JOURNAL), 4.0),
              f"journal={os.path.exists(JOURNAL)}")

        stop_peekguard(pg2, hard=True)
        time.sleep(0.5)
        check("window still hidden after hard kill", is_hidden(hwnd), f"alpha={alpha_of(hwnd)}")
        check("journal still present after the kill", os.path.exists(JOURNAL))

        pg3 = spawn_peekguard()
        recovered = wait_until(lambda: not is_hidden(hwnd), 8.0)
        check("crash recovery restored the window", recovered, f"alpha={alpha_of(hwnd)}")
        check("journal cleared after recovery", not os.path.exists(JOURNAL))
        stop_peekguard(pg3, hard=True)

        # final sanity while the target is still alive
        check("target fully restored at end", not is_hidden(hwnd), f"alpha={alpha_of(hwnd)}")
        if MODE == "move":
            check("move mode left no geometry drift", rect(hwnd) == base, f"{rect(hwnd)} vs {base}")
    finally:
        u32.SetCursorPos(start.x, start.y)
        for p in (pg,):
            if p.poll() is None:
                stop_peekguard(p, hard=True)
        target_proc.terminate()
        try:
            target_proc.wait(timeout=5)
        except subprocess.TimeoutExpired:
            target_proc.kill()

    if os.path.exists(JOURNAL):
        print(f"\nNOTE: journal left behind at {JOURNAL}:")
        print(open(JOURNAL, encoding="utf-8", errors="replace").read())

    failed = [n for n, ok in results if not ok]
    print()
    if failed:
        print(f"RESULT: {len(results) - len(failed)}/{len(results)} passed")
        for n in failed:
            print(f"  FAILED: {n}")
        if DEBUG:
            err = pg.stderr.read().decode(errors="replace") if pg.stderr else ""
            if err:
                print("--- peekguard stderr (tail) ---")
                print(err[-4000:])
        return 1
    print(f"RESULT: all {len(results)} checks passed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
