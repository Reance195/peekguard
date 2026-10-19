"""End-to-end test for PeekGuard's named-pipe IPC — the path the uTools plugin drives.

The IPC is the contract a plugin or script sees, so the suite talks to it the way a
client does: connect to the pipe, send one text line, read one JSON line back. It
also verifies that commands act on a live window instead of merely updating a status
snapshot:

  1. the pipe answers status/ping with a full state snapshot, before anything exists
  2. window commands fail cleanly while nothing is bound
  3. bind opens the crosshair picker; a real click binds the target window
  4. hide / show / toggle genuinely hide and restore that window
  5. latch / mode / scan / autostart / reload round-trip through the pipe
  6. malformed input is rejected without wedging the server
  7. unbind releases the window and the pipe keeps answering

The suite spawns a private instance: its own mutex, pipe name, config file and
autostart registry value, so it never touches a PeekGuard a user is running.
Set PEEKGUARD_DEBUG=1 for verbose app output.
"""
import ctypes as C
import itertools
import json
import os
import subprocess
import sys
import time
import winreg
from ctypes import wintypes as W

u32 = C.windll.user32
k32 = C.windll.kernel32

GENERIC_READ = 0x80000000
GENERIC_WRITE = 0x40000000
OPEN_EXISTING = 3
INVALID_HANDLE_VALUE = 0xFFFFFFFFFFFFFFFF
VK_ESCAPE = 0x1B
MOUSEEVENTF_LEFTDOWN = 0x0002
MOUSEEVENTF_LEFTUP = 0x0004
SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN = 76, 77
SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN = 78, 79
RUN_KEY = r"Software\Microsoft\Windows\CurrentVersion\Run"

k32.CreateFileW.argtypes = [C.c_wchar_p, W.DWORD, W.DWORD, C.c_void_p, W.DWORD, W.DWORD, W.HANDLE]
k32.CreateFileW.restype = C.c_void_p
k32.WriteFile.argtypes = [W.HANDLE, C.c_void_p, W.DWORD, C.POINTER(W.DWORD), C.c_void_p]
k32.ReadFile.argtypes = [W.HANDLE, C.c_void_p, W.DWORD, C.POINTER(W.DWORD), C.c_void_p]
k32.CloseHandle.argtypes = [W.HANDLE]
u32.SetCursorPos.argtypes = [C.c_int, C.c_int]
u32.mouse_event.argtypes = [W.DWORD, W.DWORD, W.DWORD, W.DWORD, C.c_size_t]
u32.GetLayeredWindowAttributes.argtypes = [
    W.HWND, C.POINTER(W.DWORD), C.POINTER(C.c_ubyte), C.POINTER(W.DWORD)
]
u32.GetWindowThreadProcessId.argtypes = [W.HWND, C.POINTER(W.DWORD)]
u32.GetWindowThreadProcessId.restype = W.DWORD
u32.AttachThreadInput.argtypes = [W.DWORD, W.DWORD, W.BOOL]
u32.AttachThreadInput.restype = W.BOOL
u32.BringWindowToTop.argtypes = [W.HWND]
u32.BringWindowToTop.restype = W.BOOL
k32.GetCurrentThreadId.restype = W.DWORD

# Distinct per spawn, like the other suites: a real PeekGuard running alongside must
# neither take our mutex/pipe nor be affected by our config.
_mutex_seq = itertools.count()

HERE = os.path.dirname(os.path.abspath(__file__))
EXE = os.path.join(os.path.dirname(HERE), "target", "debug", "peekguard.exe")
TARGET_TITLE = "PeekGuard Test Target"
DEBUG = os.environ.get("PEEKGUARD_DEBUG", "") == "1"
PIPE = rf"\\.\pipe\PeekGuard-IPC-e2e-{os.getpid()}"
AUTOSTART_VALUE = f"PeekGuardIpcTest{os.getpid()}"

STATE_FIELDS = {
    "version", "bound", "bound_desc", "bound_process", "state", "hidden", "mode",
    "latch", "delay_ms", "rules", "scan", "picking", "elevated", "autostart", "config",
}


# ------------------------------------------------------------------ pipe client

def pipe_connect(timeout=8.0):
    """Dial the pipe until the server listens, mimicking `net.connect` in the plugin."""
    end = time.time() + timeout
    while time.time() < end:
        h = k32.CreateFileW(PIPE, GENERIC_READ | GENERIC_WRITE, 0, None, OPEN_EXISTING, 0, None)
        if h and h != INVALID_HANDLE_VALUE:
            return h
        time.sleep(0.05)
    return None


def read_line(h):
    """Read one newline-terminated UTF-8 line; None if the server hung up first."""
    out = b""
    chunk = C.create_string_buffer(4096)
    got = W.DWORD(0)
    while b"\n" not in out:
        if not k32.ReadFile(h, chunk, 4096, C.byref(got), None) or got.value == 0:
            return None
        out += chunk.raw[: got.value]
        if len(out) > 1 << 20:
            return None
    return out.split(b"\n", 1)[0].decode("utf-8")


def send(line, timeout=8.0):
    """One IPC round-trip. Returns the parsed JSON response, or None if offline."""
    h = pipe_connect(timeout)
    if not h:
        return None
    try:
        data = line.encode("utf-8") + b"\n"
        wrote = W.DWORD(0)
        k32.WriteFile(h, C.create_string_buffer(data), len(data), C.byref(wrote), None)
        text = read_line(h)
        return json.loads(text) if text else None
    finally:
        k32.CloseHandle(h)


# ------------------------------------------------------------------ test target

def rect(h):
    r = W.RECT()
    u32.GetWindowRect(h, C.byref(r))
    return (r.left, r.top, r.right, r.bottom)


def alpha_of(h):
    key, alpha, flags = W.DWORD(0), C.c_ubyte(0), W.DWORD(0)
    if not u32.GetLayeredWindowAttributes(h, C.byref(key), C.byref(alpha), C.byref(flags)):
        return None
    return alpha.value if flags.value & 2 else None


def is_hidden(h):
    """Hidden if invisible, fully transparent, or parked off the virtual screen."""
    if not u32.IsWindowVisible(h):
        return True
    if alpha_of(h) == 0:
        return True
    r = rect(h)
    vx, vy = u32.GetSystemMetrics(SM_XVIRTUALSCREEN), u32.GetSystemMetrics(SM_YVIRTUALSCREEN)
    vw, vh = u32.GetSystemMetrics(SM_CXVIRTUALSCREEN), u32.GetSystemMetrics(SM_CYVIRTUALSCREEN)
    return r[2] <= vx or r[0] >= vx + vw or r[3] <= vy or r[1] >= vy + vh


def _enum(pid):
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
        if TARGET_TITLE in b.value:
            out.append(h)
        return True

    u32.EnumWindows(EP(cb), 0)
    return out


def find_target(pid, timeout=8.0):
    end = time.time() + timeout
    while time.time() < end:
        w = _enum(pid)
        if w:
            return w[0]
        time.sleep(0.2)
    return 0


def spawn_target(lifetime="180"):
    proc = subprocess.Popen([sys.executable, os.path.join(HERE, "target_window.py"), lifetime])
    return proc, find_target(proc.pid)


def click_at(x, y):
    u32.SetCursorPos(x, y)
    time.sleep(0.25)  # let the picker tick highlight what is under the cursor
    u32.mouse_event(MOUSEEVENTF_LEFTDOWN, 0, 0, 0, 0)
    time.sleep(0.05)
    u32.mouse_event(MOUSEEVENTF_LEFTUP, 0, 0, 0, 0)


def focus_window(hwnd, tries=6):
    """Give `hwnd` the foreground so the picker really picks it.

    Same dance as e2e_live: a background process may not steal focus, so attach to
    the foreground thread's input queue (which makes the change legal), set, detach.
    """
    for _ in range(tries):
        if u32.GetForegroundWindow() == hwnd:
            return True
        u32.SetForegroundWindow(hwnd)
        if u32.GetForegroundWindow() == hwnd:
            return True
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


def cancel_picker():
    if u32.FindWindowW("PeekGuard.Picker.v1", None):
        u32.keybd_event(VK_ESCAPE, 0, 0, 0)
        time.sleep(0.05)
        u32.keybd_event(VK_ESCAPE, 0, 2, 0)
        time.sleep(0.2)


def wait_until(pred, timeout=5.0, step=0.1):
    end = time.time() + timeout
    while time.time() < end:
        if pred():
            return True
        time.sleep(step)
    return False


# ------------------------------------------------------------------ app control

def spawn_peekguard():
    env = dict(os.environ)
    if DEBUG:
        env["PEEKGUARD_DEBUG"] = "1"
    cfg_dir = os.path.join(os.environ.get("TEMP", "."), "peekguard-test")
    os.makedirs(cfg_dir, exist_ok=True)
    cfg_path = os.path.join(cfg_dir, "peekguard-ipc.conf")
    with open(cfg_path, "w", encoding="utf-8") as f:
        f.write("\n".join(["mode = alpha", "delay_ms = 200", "margin_px = 6", "latch = false", ""]))
    env["PEEKGUARD_CONFIG"] = cfg_path
    env["PEEKGUARD_MUTEX"] = f"PeekGuard-IpcTest-{os.getpid()}-{next(_mutex_seq)}"
    env["PEEKGUARD_PIPE"] = PIPE
    env["PEEKGUARD_AUTOSTART_VALUE"] = AUTOSTART_VALUE
    return subprocess.Popen([EXE], stdout=subprocess.PIPE, stderr=subprocess.PIPE, env=env)


def stop_peekguard(p, hard=True):
    try:
        p.kill() if hard else p.terminate()
        p.wait(timeout=6)
    except subprocess.TimeoutExpired:
        p.kill()


def autostart_value_present():
    try:
        with winreg.OpenKey(winreg.HKEY_CURRENT_USER, RUN_KEY) as k:
            winreg.QueryValueEx(k, AUTOSTART_VALUE)
        return True
    except OSError:
        return False


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
    results = []

    def check(name, cond, detail=""):
        results.append((name, bool(cond)))
        print(f"  [{'PASS' if cond else 'FAIL'}] {name}" + (f"  -- {detail}" if detail else ""))

    target_proc, hwnd = spawn_target()
    if not hwnd:
        print("FAIL: target window never appeared")
        target_proc.kill()
        return 1

    pg = spawn_peekguard()
    resp = None
    try:
        # --- 1. the pipe comes up and answers a full snapshot
        ready = None
        deadline = time.time() + 10
        while time.time() < deadline and ready is None:
            ready = send("status", timeout=1.0)
        check("pipe is listening", ready is not None)
        if ready is None:
            return 1
        st = ready.get("state", {})
        check("status is ok with a complete state", ready.get("ok") and STATE_FIELDS <= set(st),
              f"missing={STATE_FIELDS - set(st)}" if STATE_FIELDS - set(st) else "")
        check("fresh instance is idle and unbound",
              st.get("state") == "idle" and st.get("bound") is False and st.get("hidden") is False)
        check("reports a version", bool(st.get("version")), str(st.get("version")))
        check("ping answers", (send("ping") or {}).get("ok") is True)

        # --- 2. window commands fail cleanly before anything is bound
        r = send("hide") or {}
        check("hide without a binding fails", r.get("ok") is False and "绑定" in r.get("err", ""),
              repr(r.get("err")))
        r = send("show") or {}
        check("show without a binding fails", r.get("ok") is False)
        r = send("toggle") or {}
        check("toggle without a binding fails", r.get("ok") is False)

        # --- 3. bind through the picker the IPC opens, then click the target
        r = send("bind") or {}
        check("bind opens the picker", r.get("ok") is True and r.get("state", {}).get("picking") is True)
        base = rect(hwnd)
        cx, cy = (base[0] + base[2]) // 2, (base[1] + base[3]) // 2
        focus_window(hwnd)
        time.sleep(0.3)
        click_at(cx, cy)
        bound = wait_until(lambda: (send("status") or {}).get("state", {}).get("bound") is True, 4.0)
        check("click binds the target", bound)
        st = (send("status") or {}).get("state", {})
        check("snapshot describes the binding",
              st.get("bound_desc", "") != "" and "python" in st.get("bound_process", "").lower(),
              f"{st.get('bound_desc')!r} / {st.get('bound_process')!r}")

        # --- 4. hide / show / toggle act on the real window
        u32.SetCursorPos(2, 2)  # park away so hiding can actually happen
        r = send("hide") or {}
        check("hide succeeds", r.get("ok") is True, repr(r.get("msg")))
        check("window is really hidden", wait_until(lambda: is_hidden(hwnd), 4.0),
              f"alpha={alpha_of(hwnd)}")
        check("snapshot agrees it is hidden", r.get("state", {}).get("hidden") is True)
        r = send("hide") or {}
        check("hide is idempotent", r.get("ok") is True and "已经" in r.get("msg", ""))

        r = send("show") or {}
        check("show succeeds", r.get("ok") is True)
        check("window is really visible", wait_until(lambda: not is_hidden(hwnd), 4.0),
              f"alpha={alpha_of(hwnd)}")

        u32.SetCursorPos(2, 2)
        check("hide again for toggle", (send("hide") or {}).get("ok") is True)
        r = send("toggle") or {}
        check("toggle restores a hidden window", r.get("ok") is True and r.get("state", {}).get("hidden") is False)
        check("window visible after toggle", wait_until(lambda: not is_hidden(hwnd), 4.0))
        u32.SetCursorPos(2, 2)
        r = send("toggle") or {}
        check("toggle hides a visible window", r.get("ok") is True and r.get("state", {}).get("hidden") is True)
        check("window hidden after toggle", wait_until(lambda: is_hidden(hwnd), 4.0))

        # --- 5. settings round-trip through the pipe
        r = send("latch on") or {}
        check("latch on", r.get("ok") is True and r.get("state", {}).get("latch") is True)
        r = send("latch on") or {}
        check("latch on again is a no-op", r.get("ok") is True and "已经" in r.get("msg", ""))
        r = send("latch off") or {}
        check("latch off restores", r.get("ok") is True and r.get("state", {}).get("latch") is False)

        r = send("mode move") or {}
        check("mode switch", r.get("ok") is True and r.get("state", {}).get("mode") == "move")
        r = send("mode bogus") or {}
        check("bad mode is rejected", r.get("ok") is False and "用法" in r.get("err", ""))
        r = send("mode alpha") or {}
        check("mode restores", r.get("ok") is True and r.get("state", {}).get("mode") == "alpha")

        r = send("scan off") or {}
        check("scan off", r.get("ok") is True and r.get("state", {}).get("scan") is False)
        r = send("scan on") or {}
        check("scan on restores", r.get("ok") is True and r.get("state", {}).get("scan") is True)

        r = send("reload") or {}
        check("reload", r.get("ok") is True)

        r = send("autostart on") or {}
        check("autostart on", r.get("ok") is True and r.get("state", {}).get("autostart") is True)
        check("autostart wrote the isolated registry value", autostart_value_present())
        r = send("autostart off") or {}
        check("autostart off", r.get("ok") is True and r.get("state", {}).get("autostart") is False)

        # --- 6. malformed input cannot wedge the server
        r = send("bogus-command") or {}
        check("unknown command is rejected", r.get("ok") is False and "未知命令" in r.get("err", ""))
        r = send("") or {}
        check("empty command is rejected", r.get("ok") is False)
        h = pipe_connect()
        if h:
            k32.WriteFile(h, C.create_string_buffer(b"no newline ever"), 15, C.byref(W.DWORD(0)), None)
            k32.CloseHandle(h)  # client gives up mid-line; the server must recover
        check("server survives a truncated request",
              (send("status", timeout=4.0) or {}).get("ok") is True)

        # --- 7. unbind releases the window and the pipe keeps answering
        r = send("unbind") or {}
        check("unbind succeeds", r.get("ok") is True and r.get("state", {}).get("bound") is False)
        check("window is restored after unbind", wait_until(lambda: not is_hidden(hwnd), 4.0),
              f"alpha={alpha_of(hwnd)}")
    finally:
        cancel_picker()
        u32.SetCursorPos(start.x, start.y)
        if pg.poll() is None:
            stop_peekguard(pg)
        target_proc.terminate()
        try:
            target_proc.wait(timeout=5)
        except subprocess.TimeoutExpired:
            target_proc.kill()
        try:
            with winreg.OpenKey(winreg.HKEY_CURRENT_USER, RUN_KEY, 0, winreg.KEY_SET_VALUE) as k:
                winreg.DeleteValue(k, AUTOSTART_VALUE)
        except OSError:
            pass

    # the pipe must disappear with the process, so the plugin's "not running" probe works
    check("pipe closes when the app exits", pipe_connect(timeout=1.5) is None)

    failed = [n for n, ok in results if not ok]
    print()
    if failed:
        print(f"RESULT: {len(results) - len(failed)}/{len(results)} passed")
        for n in failed:
            print(f"  FAILED: {n}")
        if DEBUG and pg.stderr:
            err = pg.stderr.read().decode(errors="replace")
            if err:
                print("--- peekguard stderr (tail) ---")
                print(err[-4000:])
        return 1
    print(f"RESULT: all {len(results)} checks passed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
