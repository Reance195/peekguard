"""无头 IPC 端到端测试：验证命名管道契约，全程不移动真实光标、不注入键鼠事件。

和 e2e_ipc.py 不同，这套用例不打开取景器、不调用 SetCursorPos/mouse_event。
它把目标窗口直接摆到光标正下方，让规则自动绑定，再用 IPC 命令驱动隐藏/恢复。
这样既能覆盖状态机与配置重载，又不会打断正在使用电脑的人。

覆盖：
  1. 管道启动并返回完整快照
  2. 规则扫描在无任何输入的情况下自动绑定目标
  3. hide/show/toggle 真实作用到窗口（包含幂等与手动隐藏的保持）
  4. latch 与窗口状态对得上（隐藏期间不因外部鼠标移动而自动恢复）
  5. move 模式下几何位置往返不漂移
  6. reload 读入新的 delay_ms、恢复已隐藏窗口并重新开启规则扫描
  7. hotkey 命令的参数校验、占用检测与规范化
  8. 超长无换行请求被截断而不会卡死服务
  9. 多个并发客户端能排队完成
 10. unbind 幂等、抑制规则重绑，解绑后的窗口命令干净报错
 11. 用例自身从不移动光标；外部移动鼠标不会影响隐藏阶段的断言

环境变量与其它套件一致：PEEKGUARD_MUTEX 使用 scratch_mutex()，
PEEKGUARD_CONFIG/PEEKGUARD_PIPE 均指向本次运行私有资源。
"""
import ctypes as C
import json
import os
import subprocess
import sys
import threading
import time
from ctypes import wintypes as W

from e2e_live import (
    EXE,
    TARGET_TITLE,
    is_hidden,
    rect,
    scratch_mutex,
    spawn_target,
    stop_peekguard,
    u32,
    wait_until,
)

k32 = C.windll.kernel32

GENERIC_READ = 0x80000000
GENERIC_WRITE = 0x40000000
OPEN_EXISTING = 3
INVALID_HANDLE_VALUE = 0xFFFFFFFFFFFFFFFF
SWP_NOSIZE = 0x0001
SWP_NOZORDER = 0x0004
SWP_NOACTIVATE = 0x0010

HERE = os.path.dirname(os.path.abspath(__file__))
PIPE = rf"\\.\pipe\PeekGuard-IPC-headless-{os.getpid()}"
AUTOSTART_VALUE = f"PeekGuardIpcHeadless{os.getpid()}"
CFG_DIR = os.path.join(os.environ.get("TEMP", "."), "peekguard-test")
CFG_PATH = os.path.join(CFG_DIR, f"peekguard-ipc-headless-{os.getpid()}.conf")
DEBUG = os.environ.get("PEEKGUARD_DEBUG", "") == "1"

# 三个热键都选冷门组合，避免和用户正在运行的 PeekGuard 的默认热键
# (Ctrl+Alt+P/H/L) 抢占。RegisterHotKey 是系统级资源，抢不到会直接报错。
HOTKEY_BIND = "Ctrl+Alt+Shift+F9"
HOTKEY_TOGGLE = "Ctrl+Alt+Shift+F10"
HOTKEY_LATCH = "Ctrl+Alt+Shift+F11"

STATE_FIELDS = {
    "version", "bound", "bound_desc", "bound_process", "state", "hidden", "mode",
    "latch", "delay_ms", "rules", "scan", "picking", "elevated", "autostart", "config",
    "hotkey_bind", "hotkey_toggle", "hotkey_latch",
}

k32.CreateFileW.argtypes = [C.c_wchar_p, W.DWORD, W.DWORD, C.c_void_p, W.DWORD, W.DWORD, W.HANDLE]
k32.CreateFileW.restype = C.c_void_p
k32.WriteFile.argtypes = [W.HANDLE, C.c_void_p, W.DWORD, C.POINTER(W.DWORD), C.c_void_p]
k32.ReadFile.argtypes = [W.HANDLE, C.c_void_p, W.DWORD, C.POINTER(W.DWORD), C.c_void_p]
k32.CloseHandle.argtypes = [W.HANDLE]
u32.GetCursorPos.argtypes = [C.POINTER(W.POINT)]
u32.GetCursorPos.restype = W.BOOL
u32.SetWindowPos.argtypes = [W.HWND, W.HWND, C.c_int, C.c_int, C.c_int, C.c_int, W.UINT]
u32.SetWindowPos.restype = W.BOOL
u32.GetClassNameW.argtypes = [W.HWND, C.POINTER(C.c_wchar), C.c_int]
u32.GetClassNameW.restype = C.c_int


# ------------------------------------------------------------------ 管道客户端

def pipe_connect(timeout=8.0):
    """反复拨号，直到服务端的管道实例可用。"""
    end = time.time() + timeout
    while time.time() < end:
        h = k32.CreateFileW(PIPE, GENERIC_READ | GENERIC_WRITE, 0, None, OPEN_EXISTING, 0, None)
        if h and h != INVALID_HANDLE_VALUE:
            return h
        time.sleep(0.05)
    return None


def read_line(h):
    """读一行 UTF-8 JSON；服务端先挂断时返回 None。"""
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
    """一次完整往返：连接、发一行、读一行 JSON。"""
    h = pipe_connect(timeout)
    if not h:
        return None
    try:
        data = line.encode("utf-8") + b"\n"
        buf = C.create_string_buffer(data)
        wrote = W.DWORD(0)
        k32.WriteFile(h, buf, len(data), C.byref(wrote), None)
        text = read_line(h)
        return json.loads(text) if text else None
    finally:
        k32.CloseHandle(h)


def state():
    """当前状态快照，失败时返回空 dict。"""
    return (send("status") or {}).get("state", {})


# ------------------------------------------------------------------ 目标与配置

def class_of(hwnd):
    buf = C.create_unicode_buffer(256)
    u32.GetClassNameW(hwnd, buf, len(buf))
    return buf.value


def place_under_cursor(hwnd, tries=5):
    """把窗口居中摆到光标下面，让光标处在守护区内。

    这里只移动窗口，不移动光标；之后只通过 IPC 显式隐藏/恢复。
    """
    for _ in range(tries):
        pt = W.POINT()
        u32.GetCursorPos(C.byref(pt))
        left, top, right, bottom = rect(hwnd)
        w, h = right - left, bottom - top
        x, y = pt.x - w // 2, pt.y - h // 2
        u32.SetWindowPos(
            hwnd, None, x, y, 0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE
        )
        r = rect(hwnd)
        if r[0] <= pt.x <= r[2] and r[1] <= pt.y <= r[3]:
            return (pt.x, pt.y)
        time.sleep(0.1)
    return None


def write_conf(delay_ms, cls):
    """写一份私有配置；热键保持冷门组合，避免和真实实例冲突。"""
    os.makedirs(CFG_DIR, exist_ok=True)
    lines = [
        "mode = alpha",
        f"delay_ms = {delay_ms}",
        "margin_px = 6",
        "latch = false",
        "check_update = false",
        f"hotkey_bind = {HOTKEY_BIND}",
        f"hotkey_toggle = {HOTKEY_TOGGLE}",
        f"hotkey_latch = {HOTKEY_LATCH}",
        f"rule = class:{cls} | title:{TARGET_TITLE}",
        "",
    ]
    with open(CFG_PATH, "w", encoding="utf-8") as f:
        f.write("\n".join(lines))


def spawn_peekguard():
    env = dict(os.environ)
    if DEBUG:
        env["PEEKGUARD_DEBUG"] = "1"
    env["PEEKGUARD_CONFIG"] = CFG_PATH
    env["PEEKGUARD_MUTEX"] = scratch_mutex()
    env["PEEKGUARD_PIPE"] = PIPE
    env["PEEKGUARD_AUTOSTART_VALUE"] = AUTOSTART_VALUE
    return subprocess.Popen([EXE], stdout=subprocess.PIPE, stderr=subprocess.PIPE, env=env)


# ------------------------------------------------------------------ 特殊请求

def flood_oversized(size=70 * 1024):
    """发送一条超过 64 KiB 且没有换行的请求。

    服务端在超限时不会回复，而是直接断开。写操作可能因管道关闭而阻塞，
    所以放到后台线程里，避免整个套件被卡住。
    """
    out = {}

    def worker():
        h = pipe_connect(8.0)
        out["connect"] = bool(h)
        if not h:
            return
        try:
            payload = b"x" * size
            buf = C.create_string_buffer(payload)
            wrote = W.DWORD(0)
            ok = k32.WriteFile(h, buf, len(payload), C.byref(wrote), None)
            out["write_ok"] = bool(ok)
            out["wrote"] = wrote.value
            out["reply"] = read_line(h)
        except Exception as e:  # noqa: BLE001 - 记录异常即可
            out["error"] = repr(e)
        finally:
            k32.CloseHandle(h)

    t = threading.Thread(target=worker, daemon=True)
    t.start()
    t.join(8.0)
    out["finished"] = not t.is_alive()
    return out


def parallel_pings(n=4):
    results = [None] * n

    def worker(i):
        results[i] = send("ping", timeout=10.0)

    threads = [threading.Thread(target=worker, args=(i,)) for i in range(n)]
    for t in threads:
        t.start()
    for t in threads:
        t.join(12.0)
    return results


# ------------------------------------------------------------------ 主流程

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

    results = []

    def check(name, cond, detail=""):
        results.append((name, bool(cond)))
        print(f"  [{'PASS' if cond else 'FAIL'}] {name}" + (f"  -- {detail}" if detail else ""))

    target_proc, hwnd = spawn_target("180")
    if not hwnd:
        print("FAIL: target window never appeared")
        return 1

    cursor_at = place_under_cursor(hwnd)
    check("target placed under the cursor", cursor_at is not None)
    if cursor_at is None:
        target_proc.terminate()
        return 1

    cls = class_of(hwnd)
    # 初始防抖拉到上限：外部鼠标可能在用例运行期间移动，
    # 长延迟能避免把“用户碰了鼠标”误判成自动隐藏。
    write_conf(60000, cls)

    pg = spawn_peekguard()
    time.sleep(1.0)
    if pg.poll() is not None:
        out, err = pg.communicate()
        print("FAIL: peekguard exited immediately")
        print(err.decode(errors="replace"))
        target_proc.terminate()
        return 1

    try:
        ready = None
        deadline = time.time() + 10
        while time.time() < deadline and ready is None:
            ready = send("status", timeout=1.0)
        check("pipe is listening", ready is not None)
        if ready is None:
            return 1

        st = ready.get("state", {})
        check(
            "status is ok with a complete state",
            ready.get("ok") and STATE_FIELDS <= set(st),
            f"missing={STATE_FIELDS - set(st)}" if STATE_FIELDS - set(st) else "",
        )
        check("reports a version", bool(st.get("version")), str(st.get("version")))
        check("ping answers", (send("ping") or {}).get("ok") is True)

        # --- 1. 规则自动绑定，不产生任何输入事件
        bound = wait_until(lambda: state().get("bound") is True, 6.0)
        check("rule binds the target without input", bound)
        if not bound:
            return 1
        st = state()
        check(
            "snapshot describes the rule binding",
            TARGET_TITLE in st.get("bound_desc", "")
            and "python" in st.get("bound_process", "").lower(),
            f"{st.get('bound_desc')!r} / {st.get('bound_process')!r}",
        )
        check(
            "config is in effect",
            st.get("rules") == 1
            and st.get("mode") == "alpha"
            and st.get("delay_ms") == 60000
            and st.get("scan") is True
            and st.get("picking") is False,
            f"rules={st.get('rules')} mode={st.get('mode')} delay={st.get('delay_ms')} "
            f"scan={st.get('scan')} picking={st.get('picking')}",
        )
        check(
            "initial hotkeys are the isolated combos",
            (st.get("hotkey_bind"), st.get("hotkey_toggle"), st.get("hotkey_latch"))
            == (HOTKEY_BIND, HOTKEY_TOGGLE, HOTKEY_LATCH),
            f"{st.get('hotkey_bind')} / {st.get('hotkey_toggle')} / {st.get('hotkey_latch')}",
        )
        check("config path is the private one", st.get("config") == CFG_PATH, str(st.get("config")))

        # 光标在窗口内；60 秒防抖保证外部鼠标随便移动也不会自动隐藏
        time.sleep(0.9)
        check("stays visible while the cursor is inside", not is_hidden(hwnd))

        # 隐藏阶段全部开着 latch：外部鼠标移动不会触发自动恢复，
        # 这样用例只验证 IPC 驱动的显式 hide/show，不受桌面活动干扰。
        r = send("latch on") or {}
        check("latch on before the manual-hide checks",
              r.get("ok") is True and r.get("state", {}).get("latch") is True)

        # --- 2. hide / show / toggle
        r = send("hide") or {}
        check("IPC hide succeeds", r.get("ok") is True, repr(r.get("msg")))
        check("snapshot agrees it is hidden", r.get("state", {}).get("hidden") is True)
        check("window really hides", wait_until(lambda: is_hidden(hwnd), 4.0))
        time.sleep(0.6)
        check("manual hide survives while the cursor stays inside", is_hidden(hwnd))
        r = send("hide") or {}
        check("hide is idempotent", r.get("ok") is True and "已经" in r.get("msg", ""))

        r = send("show") or {}
        check("IPC show succeeds", r.get("ok") is True, repr(r.get("msg")))
        check("window really returns", wait_until(lambda: not is_hidden(hwnd), 4.0))
        time.sleep(0.5)
        check("stays visible after an explicit show", not is_hidden(hwnd))
        r = send("show") or {}
        check("show on a visible window is a no-op", r.get("ok") is True and "当前可见" in r.get("msg", ""))

        r = send("toggle") or {}
        check("toggle hides a visible window", r.get("ok") is True and r.get("state", {}).get("hidden") is True)
        check("window hidden after toggle", wait_until(lambda: is_hidden(hwnd), 4.0))
        r = send("toggle") or {}
        check("toggle restores a hidden window", r.get("ok") is True and r.get("state", {}).get("hidden") is False)
        check("window visible after toggle", wait_until(lambda: not is_hidden(hwnd), 4.0))

        # --- 3. latch 与状态机联动
        r = send("latch on") or {}
        check("latch on again is a no-op", r.get("ok") is True and "已经" in r.get("msg", ""))
        r = send("hide") or {}
        check(
            "hide under latch reports the latched state",
            r.get("ok") is True and r.get("state", {}).get("state") == "latched",
            repr(r.get("state", {}).get("state")),
        )
        check("window hidden under latch", wait_until(lambda: is_hidden(hwnd), 4.0))
        r = send("show") or {}
        check("show releases a latched hide", r.get("ok") is True and r.get("state", {}).get("hidden") is False)
        check("window visible after releasing latch hide", wait_until(lambda: not is_hidden(hwnd), 4.0))
        r = send("latch off") or {}
        check("latch off", r.get("ok") is True and r.get("state", {}).get("latch") is False)
        r = send("latch on") or {}
        check("latch back on for the remaining checks",
              r.get("ok") is True and r.get("state", {}).get("latch") is True)

        # --- 4. move 模式几何往返
        base = rect(hwnd)
        r = send("mode move") or {}
        check("mode move applied", r.get("ok") is True and r.get("state", {}).get("mode") == "move")
        r = send("hide") or {}
        check("hide in move mode", r.get("ok") is True and r.get("state", {}).get("hidden") is True)
        check("window parked off-screen", wait_until(lambda: is_hidden(hwnd), 4.0))
        moved = rect(hwnd)
        check("move mode changed the geometry", moved != base, f"{moved} vs {base}")
        r = send("show") or {}
        check("show in move mode", r.get("ok") is True)
        check("window visible again", wait_until(lambda: not is_hidden(hwnd), 4.0))
        check("geometry round-trips exactly", rect(hwnd) == base, f"{rect(hwnd)} vs {base}")
        r = send("mode alpha") or {}
        check("mode alpha restored", r.get("ok") is True and r.get("state", {}).get("mode") == "alpha")

        # --- 5. scan 暂停 / reload 生效 / scan 恢复
        r = send("scan off") or {}
        check("scan off", r.get("ok") is True and r.get("state", {}).get("scan") is False)
        r = send("hide") or {}
        check("hide before reload", r.get("ok") is True and r.get("state", {}).get("hidden") is True)
        check("window hidden before reload", wait_until(lambda: is_hidden(hwnd), 4.0))

        write_conf(5000, cls)
        r = send("reload") or {}
        st = r.get("state", {})
        check("reload succeeds", r.get("ok") is True)
        check("reload applies delay_ms", st.get("delay_ms") == 5000, str(st.get("delay_ms")))
        check("reload restores a hidden window", st.get("hidden") is False)
        check("window visible after reload", wait_until(lambda: not is_hidden(hwnd), 4.0))
        check("reload resumes rule scanning", st.get("scan") is True)
        r = send("scan off") or {}
        check("scan off round-trip", r.get("ok") is True and r.get("state", {}).get("scan") is False)
        r = send("scan on") or {}
        check("scan on round-trip", r.get("ok") is True and r.get("state", {}).get("scan") is True)

        # --- 6. hotkey 命令
        r = send("hotkey") or {}
        check("hotkey without args explains the usage", r.get("ok") is False and "用法" in r.get("err", ""))
        r = send("hotkey camera Ctrl+Alt+Shift+F7") or {}
        check("unknown hotkey action is rejected", r.get("ok") is False and "用法" in r.get("err", ""))
        r = send("hotkey bind Ctrl+Alt+Shift+NotAKey") or {}
        check("invalid combo is rejected", r.get("ok") is False and "不支持" in r.get("err", ""))

        r = send("hotkey bind Ctrl+Alt+Shift+F12") or {}
        st = r.get("state", {})
        check(
            "hotkey bind applies and canonicalises",
            r.get("ok") is True and st.get("hotkey_bind") == "Ctrl+Alt+Shift+F12",
            f"{r.get('msg')!r} / {st.get('hotkey_bind')!r}",
        )
        r = send("hotkey bind ctrl+alt+shift+f12") or {}
        check(
            "lowercase spelling is canonicalised",
            r.get("ok") is True and r.get("state", {}).get("hotkey_bind") == "Ctrl+Alt+Shift+F12",
            repr(r.get("state", {}).get("hotkey_bind")),
        )
        r = send("hotkey latch Ctrl+Alt+Shift+F12") or {}
        check("a combo already used by another slot is refused", r.get("ok") is False and "占用" in r.get("err", ""))
        r = send("hotkey bind Ctrl+Alt+Shift+F9") or {}
        st = r.get("state", {})
        check(
            "hotkey bind restores the isolated combo",
            r.get("ok") is True and st.get("hotkey_bind") == HOTKEY_BIND,
            f"{r.get('msg')!r} / {st.get('hotkey_bind')!r}",
        )

        # --- 7. 超长请求与并发客户端
        flood = flood_oversized()
        check(
            "oversized request is cut off without a reply",
            flood.get("connect") is True and flood.get("finished") is True and flood.get("reply") is None,
            f"{flood}",
        )
        check(
            "server survives the oversized request",
            (send("status", timeout=4.0) or {}).get("ok") is True,
        )
        replies = parallel_pings(4)
        check(
            "concurrent clients all get answers",
            all(r is not None and r.get("ok") is True for r in replies),
            f"{[bool(r and r.get('ok')) for r in replies]}",
        )

        # --- 8. unbind 与解绑后的命令
        r = send("unbind") or {}
        check("unbind releases the binding", r.get("ok") is True and r.get("state", {}).get("bound") is False)
        check("window is visible after unbind", wait_until(lambda: not is_hidden(hwnd), 4.0))
        time.sleep(1.6)
        check("rules do not immediately re-bind the released window", state().get("bound") is False)
        r = send("unbind") or {}
        check("unbind without a binding is a no-op", r.get("ok") is True and "没有绑定" in r.get("msg", ""))
        r = send("hide") or {}
        check("hide without a binding fails cleanly", r.get("ok") is False and "绑定" in r.get("err", ""))
        r = send("show") or {}
        check("show without a binding fails cleanly", r.get("ok") is False and "绑定" in r.get("err", ""))
        r = send("toggle") or {}
        check("toggle without a binding fails cleanly", r.get("ok") is False and "绑定" in r.get("err", ""))

        # 只做记录，不做断言：用例自身从不调用 SetCursorPos，
        # 但林先生可能刚好在动鼠标，坐标变化不能算失败。
        end = W.POINT()
        u32.GetCursorPos(C.byref(end))
        print(
            f"  note: cursor at ({end.x},{end.y}) when leaving; "
            f"this suite never calls SetCursorPos (started at ({start.x},{start.y}))"
        )
    finally:
        if pg.poll() is None:
            stop_peekguard(pg, hard=True)
        target_proc.terminate()
        try:
            target_proc.wait(timeout=5)
        except subprocess.TimeoutExpired:
            target_proc.kill()
        try:
            os.remove(CFG_PATH)
        except OSError:
            pass

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
