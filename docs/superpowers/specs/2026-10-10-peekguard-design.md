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
main.rs       入口：DPI、单实例、崩溃恢复、消息循环、扫描调度
win.rs        手写 Win32 FFI（user32 / kernel32 / shell32 / advapi32）
hide.rs       隐身原语：Snapshot + hide()/restore()，三模式
watch.rs      状态机：绑定目标、鼠标轮询、延迟防抖、锁存
rules.rs      规则扫描：限速枚举、目标挑选、逐窗口抑制
config.rs     配置文件读写（自定义行式格式，零依赖）
autostart.rs  HKCU Run 开自启（直接注册表 FFI）
session.rs    journal 崩溃恢复日志
tray.rs       托盘图标 + 右键菜单
icon.rs       运行时生成图标
```

### 状态机

```
Unbound ──绑定(热键/规则命中/托盘)──> Bound(Visible)
Bound(Visible) ──鼠标离开矩形且超过 delay──> Hidden
Hidden ──鼠标回到隐藏前矩形──> Bound(Visible)      # latch 关闭时
Hidden ──热键/托盘──> Bound(Visible)               # latch 开启时唯一出口
Bound(*) ──隐身失败──> Unbound + Refused            # 提权窗口等，不去无限重试
Bound(*) ──目标窗口销毁/解绑/退出──> Unbound       # 退出时必须恢复
```

隐身失败不是"下一次再试"而是终态：`Action::Refused` 会清掉绑定、让 `main`
把该 HWND 交给 `Scanner::suppress`，并向用户提示一次。

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

托盘：双击 = 绑定/解绑；右键 = 完整菜单（模式、规则自动保护、延迟、锁存、存规则、解绑当前窗口、开机自启、重载配置、退出）。

## 非目标（YAGNI）

- 不做 mask 遮罩、不做毛玻璃糊化、不做录屏专用排除
- 不做 GUI 设置窗（配置文件 + 托盘菜单足够）

## 追加特性（v1，2026-10-11）

初版交付后追加的三个能力。共同约束仍是：零依赖、单文件、不动目标进程。

### 1. 常驻规则（`src/rules.rs`）

初版的问题：`cfg.rules` 只被解析、被规则匹配函数使用，但从没有任何地方
真正去扫窗口——README 和托盘菜单却已经在承诺「窗口出现时自动保护」。属于
文档跑在实现前面。补上 `Scanner` 之后该承诺才成立。

要点：

- **限速扫描**（1 Hz）：`due(now_ms)` 是唯一节流点。绑定状态下 `scan_rules()` 直接
  返回，不白耗 CPU。
- **挑选优先级**：前台命中者 > z 序最前。前者保证"你看的那个"优先，
  后者在多开时代价稳定、不会每秒换目标。
- **自检后再绑定**：命中窗口先 `hide_now()` 再 `restore()` 试一次。能隐才留下，
  不能隐就 `suppress` 并提示一次。不做这一步的话，被提权窗口会每 350ms 静默重试到天荒地老。
- **逐窗口抑制，不是全局暂停**。解绑只释放当前窗口，规则对后续新窗口继续有效。

最后一点是实测改出来的：最初实现成了解绑即 `set_muted(true)` 全局暂停，
`tests/e2e_rules.py` 的「重新出现同类窗口仍受保护」直接失败。功能上等于是
「解绑一次 QQ，以后开 QQ 再也不管」——写测试时才暴露出来。

`suppressed` 存的是 HWND 列表，因此每次扫描前 `purge()` 掉失效句柄：
Windows 会回收句柄值，不清掉就可能误抑制一个无关窗口。

### 2. 开机自启（`src/autostart.rs`）

写 `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`，值名 `PeekGuard`。

方案取舍：

| 方案 | 否决理由 |
|---|---|
| 启动文件夹快捷方式 | 需要 COM（`IShellLink`）或 `powershell.exe`，都会引入额外依赖/子进程 |
| 任务计划 `schtasks` | 弹控制台窗，且需要提权 |
| `reg.exe add` | 弹控制台窗，且有引号转义坑 |
| **直接 advapi32 FFI** | 无窗口、无子进程、HKCU 免提权 —— 采用 |

细节：

- 值内容是带引号的 exe 完整路径，能处理含空格的路径
- `enabled()` 按名字匹配（大小写不敏感），所以 exe 被移动后仍显示为已开启，
  重新勾选一次即可修正路径
- 删除不存在的值时 `ERROR_FILE_NOT_FOUND` 视为成功（幂等）
- `PEEKGUARD_AUTOSTART_VALUE` 可覆盖值名，让 e2e 测试不去碰开发者自己的条目

### 3. 崩溃安全网（`install_panic_guard`）

`session.rs` 的 journal 只解决"进程消失后下次启动补救"，中间那段时间窗口是
真的不见了。panic hook 补上这一段：进程死之前先把窗口还回去。

关于 `panic = "abort"`：先验证再下结论——写了个最小用例分别用 unwind 和
abort 编译运行，确认 **hook 在 abort 之前依然会执行**（hook 是在
`begin_panic` 阶段触发的，早于 abort 分支）。所以 release 下的安全网是真实有效的，
不需要为此改掉 `panic = "abort"`。

另外注意 `catch_unwind` 在这里没用：unwinding 穿过 `extern "system"` 的
`wnd_proc` 会直接 abort，所以恢复逻辑放在 hook 里才是对的。

`PEEKGUARD_PANIC_AFTER=<ms>` 是为此编译进二进制、专供测试的触发开关。

## 实测记录：窗口标题读取

有两个结论一开始都猜错了，记在这里免得以后又改回去。

### 猜想一：`GetWindowTextW` 会阻塞

理由看起来很强：MSDN 说对其它进程的窗口，`GetWindowText` 等同于发送
`SendMessage(WM_GETTEXT)`，而 `SendMessage` 是同步的。于是"某个程序卡死会冻结
PeekGuard 的 UI 线程"听起来完全成立。

改成了 `SendMessageTimeoutW` （`SMTO_ABORTIFHUNG | SMTO_BLOCK`），加了超时。

**实测推翻**。造一个故意不再泵消息的窗口作为靶子：

| 调用 | 结果 | 耗时 |
|---|---|---|
| `SendMessageTimeout(WM_NULL, 300ms)` | 失败（确认窗口已挂起） | 306 ms |
| `GetWindowTextW` | **成功，拿到 20 字标题** | **0.09 ms** |
| `SendMessageTimeout(WM_GETTEXTLENGTH, 200ms)` | 失败，`n=0` | 214 ms |

`GetWindowText` 对跨进程窗口读的是**缓存标题**，不进去等对方泵消息（MSDN 的
"This behavior is by design" 就是在说这个）。所以：超时版本不但慢了两千倍，
还会在窗口挂起时把标题丢掉——刚好把它想要保护的场景搞反了。

结论：**用 `GetWindowTextW`**。已在 `win.rs` 的 `window_title` 上写明原因。

### 猜想二：一次扫描只要几十微秒

顺手写了个性能守护测试，结果第一次跑就红了：实测 2-4 ms。

拆开量才发现问题不在代码而在量法：进程刚起来时第一次 `EnumWindows` 要付出
API-set 惰性解析的代价，比稳态慢好几倍。加预热后：

| 步骤 | 耗时（10 窗口） |
|---|---|
| `EnumWindows` | 740 us |
| `process_name`（`OpenProcess` + 查询） | 570 us |
| `class_name` | 37 us |
| `window_title` | 4 us |
| **合计** | **~1.3 ms** |

所以真实数字是 ~1.3 ms/次，不是几十微秒，也不是之前那个被污染的 2-4 ms。
1 Hz 下占一个 tick 的 0.13%。守护测试的阈值定在 25 ms——它防的是算法级退化
（比如把 `window_title` 提到便宜的进程名过滤之前），不是机器噪声。

拆分表本身保留为 `#[ignore]` 的诊断用例，需要时手动跑。

## 测试策略补充

初版的 e2e 全部依赖"先按热键绑定"，这要求目标窗口在恰好那一刻是前台窗口，
存在竞态。规则测试改为让 `Scanner` 自己绑定（确定性），热键路径仍由
`e2e_live.py` 覆盖，两边都测到。

`tests/e2e_panic.py` 有一条**非空洞性保证**：早期版本在"窗口其实没被隐"的情况下
把所有恢复类断言都跑绿了。现在每条恢复断言前面都要求 `hidden_at is not None`，
即先证明确实藏过，再说"还原成功"。

当前矩阵（每个隐身模式各跑一遍）：

| 套件 | 检查数 |
|---|---|
| `cargo test` | 17 (+1 ignored diagnostic) |
| `e2e_live.py` | 22（`move` 模式多 1 项几何检查） |
| `e2e_latch.py` | 15 |
| `e2e_rules.py` | 16 |
| `e2e_robustness.py` | 16 |
| `e2e_panic.py` | 10 |

合计：`live` 22+23+22 = 67，`latch` 15×3 = 45，`rules` 16×3 = 48，
`robustness` 16×3 = 48，`panic` 10×3 = 30，即 **238 项端到端检查**，
加 17 项单元测试。每个隐身模式各跑一遍，全绿。

## 测试过程中抓到的两个真问题

这两个都不是"测试写错了"，而是测出来的产品缺陷。记下来是因为它们都属于
**不报错、但行为错**的那类，只读代码很难发现。

### 1. 最小化窗口被误判为"拒绝隐身"

`hide()` 对最小化窗口返回 `None`，而"隐不了"（典型是被提权程序挡住）也返回
`None`。两个含义完全不同的失败共用一个返回值，于是最小化的窗口被当成拒绝处理：

- 弹出一条（错误的）"该窗口可能以管理员权限运行"提示
- 并且 `scanner.suppress(hwnd)` 把它**永久拉黑**，一个什么都没做错的窗口被记仇

修法是把最小化判断改成无条件：最小化时保持 `State::Armed`、返回 `Action::None`，
绑定不变。`skip_minimized` 配置项因此失去意义，标记为废弃但保留键名，
免得旧配置被"未知配置项"警告刷屏。

### 2. 扫描器会选中最小化窗口，把守卫卡死

这个比上一个严重。守卫同一时刻只守一个窗口，而扫描器原先不区分窗口可见性：
选中一个最小化窗口后，它既隐不掉（`hide()` 对它无效），又占着唯一的守卫名额，
**结果规则里真正该保护的可见窗口永远轮不到**。

现场是一个无关的、自己最小化了的 tkinter 应用刚好命中同一条 `class:` 规则：
它的窗口 `IsWindowVisible` 为真、但处于最小化状态，守卫就卡在它身上了。

修法：扫描挑选新目标时跳过 `!is_visible(hwnd) || is_minimized(hwnd)`。
要区分**已经被守卫之后才最小化**的窗口——那种保留绑定、不主动换人，行为不变；
改的只是扫描器的挑选逻辑。

两者都有非空洞的回归测试：把 `is_minimized` 跳过删掉，
`e2e_rules.py` 的「a minimized match does not starve a visible one」会真的失败
（已在禁用修复的构建上实测确认过）。

### 3. journal 写入顺序：先隐身、后写日志

前两个是「行为错」，这个是**时序错**，危害最大。

原先是 `hide()` 先把窗口隐掉，回到 `main` 的 `on_tick()` 再写 journal。
也就是说存在一个几毫秒的窗口期：**窗口已经不可见，而"它被藏起来了"这件事还只存在
内存里**。在这中间被硬杀（崩溃、任务管理器结束进程、断电），窗口就被永久留在隐身
状态——journal 机制存在的全部意义就是杜绝这个结果，而它恰好漏在这里。

抓到的过程：`e2e_live.py` 的崩溃恢复一节偶发失败，而且症状是矛盾的——
"窗口确实隐身了"通过，但"崩溃前写了 journal"失败。也就是窗口真的隐形了却没记录，
正是上面那个缝。

修法是把顺序倒过来：**先写 journal，再动窗口**。这样两个失败方向就不对称了，
而不对称正是关键：

| 崩在哪 | 结果 |
|---|---|
| 写下 journal 之后、窗口还没隐 | journal 里有一条"可见窗口"的记录 → 下次启动"还原"它是**无害的空操作** |
| 先隐窗口、journal 还没写（旧行为） | 窗口不可见且无记录 → **永久失踪，不可恢复** |

隐不掉的窗口（提权目标）也不会留下垃圾记录：调用方在动作之后会重写权威集合，
失败路径会把自己的条目清掉（`hide()` 内部也直接清一次）。

用 720 次高频采样验证了这条不变式：**在三个隐身模式下，任何时刻都不会出现
"窗口不可见但没有 journal 记录"**，violations = 0。

顺带修正了测试桩的保真度：`PEEKGUARD_REFUSE_HIDE` 原先在写 journal **之前**就返回，
而真实的提权窗口是在 hide 调用**内部**失败的（写在 journal 之后）。挪到写之后，
清理路径才真正被测到。

### 附：测试隔离的两个坑

都不是产品问题，但都会让测试结果变得不可信：

- **单实例互斥名**。测试实例会和用户真实运行着的 PeekGuard 抢同一个全局互斥，
  输了的那份会弹**模态框**、进程活着却什么都不做——看起来像守卫有 bug，
  在没人点确定的环境里还会一直挂住。现在测试通过 `PEEKGUARD_MUTEX` 各取独立
  名字（`e2e_live.scratch_mutex()`）。
- **规则写得太宽**。`rule = class:TkTopLevel` 会命中桌面上任何其他 tkinter 应用，
  测试于是可能观察到一个"不是自己"的窗口。测试规则现在同时限定 `class:` 和
  `title:`，只命中测试自己的窗口。
