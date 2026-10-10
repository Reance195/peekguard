# PeekGuard — 设计稿

日期：2026-10-10
状态：已确认（用户授权按推荐方案直接实现）

## 目标

把「防窥小窗」的能力从 QQ 内部搬出来，做成对**任意窗口**生效的独立小工具：

> 鼠标移开 → 窗口隐身；鼠标回到原位 → 窗口恢复。

## 已确认的决策

| 议题 | 决策 |
|---|---|
| 适用范围 | 任意顶层窗口（进程名/类名/标题规则 + 临时绑定） |
| 绑定方式 | C：临时绑定随手用，满意可存成规则常驻 |
| 主隐身手段 | Alpha 透明度（主）＋ 可切换的移屏 / ShowWindow 硬核模式 |
| 触发 | 纯几何鼠标进出 + 延迟防抖；锁存模式可开关 |
| 技术栈 | Rust + 零依赖手写 Win32 FFI，单文件 exe |

## 探测结论（2026-10-10，本机 Windows 实测）

| 原语 | 结果 |
|---|---|
| `SetLayeredWindowAttributes(alpha=0)` | ✅ 对 Edge / 微信 / Windows Terminal 全部生效，且目标程序不会自愈 |
| `SetWindowPos` 移出虚拟屏 | ✅ 生效，不自愈，可精确还原 |
| `ShowWindow(SW_HIDE)` / `SW_SHOWNA` | ✅ 生效，非激活式恢复 |
| `DwmSetWindowAttribute(DWMWA_CLOAK)`（跨进程） | ❌ `E_ACCESSDENIED` — 该路线判死，不实现 |

**环境实测**：主屏 1920x1200，副屏 3840x2160 起始于 `(1920, -958)` —— 存在负坐标，
因此一切坐标运算必须在虚拟屏空间进行，且必须启用 per-monitor DPI awareness v2。

**已识别的坑**（会在实现中处理或显式提示）：
- 提权的目标窗口 → `SetWindowLongPtrW` 失败（err=5），需提示"请以管理员身份运行本工具"
- `UpdateLayeredWindow` 型窗口（`GetLayeredWindowAttributes` 失败）→ alpha 模式不适用，自动回退移屏
- 已最小化的窗口 → 跳过隐藏，恢复时用 `SW_SHOWMINNOACTIVE`
- 进程崩溃后窗口会卡在隐身状态 → 需要有会话恢复文件

## 架构

```
main.rs   入口：DPI、单实例、崩溃恢复、消息循环
win.rs    手写 Win32 FFI（user32 / kernel32 / shell32）
hide.rs   隐身原语：Snapshot + hide()/restore()，三模式
watch.rs  状态机：绑定目标、鼠标轮询、延迟防抖、锁存
rules.rs  规则匹配（进程 / 类名 / 标题子串）
config.rs 配置文件读写（自定义行式格式，零依赖）
tray.rs   托盘图标 + 右键菜单
```

### 状态机

```
Unbound ──绑定(热键/规则命中/托盘)──> Bound(Visible)
Bound(Visible) ──鼠标离开矩形且超过 delay──> Hidden
Hidden ──鼠标回到隐藏前矩形──> Bound(Visible)      # latch 关闭时
Hidden ──热键/托盘──> Bound(Visible)               # latch 开启时唯一出口
Bound(*) ──目标窗口销毁/解绑/退出──> Unbound       # 退出时必须恢复
```

### 安全约束

- 不注入、不子类化、不装任何全局钩子（无 `SetWindowsHookEx`）
- 只用 `GetCursorPos` 轮询做几何判定，对目标进程零侵入
- 退出路径（含 `WM_DESTROY`/托盘退出）必须先恢复全部受保护窗口
- 会话文件记录隐藏中的窗口，异常退出后下次启动自动恢复

## 交互

| 热键 | 作用 |
|---|---|
| `Ctrl+Alt+P` | 绑定 / 解绑当前前台窗口 |
| `Ctrl+Alt+H` | 手动隐藏 / 恢复当前绑定窗口 |
| `Ctrl+Alt+L` | 切换锁存模式 |

托盘：双击 = 绑定/解绑；右键 = 完整菜单（模式、延迟、穿透、锁存、存规则、解绑全部、重载配置、退出）。

## 非目标（YAGNI）

- 不做 mask 遮罩、不做毛玻璃糊化、不做录屏专用排除
- 不做 GUI 设置窗（配置文件 + 托盘菜单足够）
- 不做开机自启（系统任务计划或启动文件夹自己放快捷方式）
