# PeekGuard

[English](README.md) | 简体中文

[![CI](https://github.com/Reance195/peekguard/actions/workflows/ci.yml/badge.svg)](https://github.com/Reance195/peekguard/actions/workflows/ci.yml)
[![license](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

绑定任意窗口，鼠标移开自动隐身，鼠标移回自动恢复。

这就是 QQ「防窥小窗」的那个行为，但作用于**任何**窗口，不只是 QQ。

```
鼠标在窗口里                鼠标移开 350ms 后           鼠标移回原位置
┌──────────────┐           ┌──────────────┐           ┌──────────────┐
│  聊天窗口     │   ────>   │  （完全透明）  │   ────>   │  聊天窗口     │
│  正常显示     │           │  屏幕上看不见  │           │  恢复显示     │
└──────────────┘           └──────────────┘           └──────────────┘
```

- **零依赖**：只用标准库 + 手写 Win32 FFI，没有 crates.io 依赖
- **单文件 exe**：约 275 KB，免安装，运行在托盘
- **零侵入**：不注入、不子类化、不装全局钩子，对目标进程完全不碰
- **绝不留残局**：崩溃、强杀、异常退出都会在下次启动时还原窗口
- **常驻规则**：写一行 `rule =`，程序自己认出该保护哪些窗口

> 仅支持 Windows（x86_64）。

## 下载

从 [Releases](../../releases) 下载 `peekguard.exe`，直接双击运行即可，无需安装。

## 从源码构建

需要 Rust 1.87 或更新版本（用到 `is_multiple_of`）：

```sh
git clone https://github.com/Reance195/peekguard.git
cd peekguard
cargo build --release      # -> target/release/peekguard.exe
```

首次运行会在 `%APPDATA%\PeekGuard\peekguard.conf` 生成默认配置。
启动后进入系统托盘，没有主窗口。

## 用法

| 操作 | 快捷键 |
|---|---|
| 绑定 / 解绑当前前台窗口 | `Ctrl+Alt+P` |
| 立即隐身 / 恢复 | `Ctrl+Alt+H` |
| 切换锁存模式 | `Ctrl+Alt+L` |

托盘图标：

- **双击** = 绑定 / 解绑当前前台窗口
- **右键** = 完整菜单（隐身方式、规则自动保护、延迟、锁存、存规则、开机自启、重载配置、退出）

### 典型流程

1. 切到你想保护的窗口（QQ、浏览器、股票软件……）
2. 按 `Ctrl+Alt+P` 绑定 —— 托盘提示「已绑定: QQ.exe — 我的聊天窗口」
3. 鼠标移开，窗口自动隐身；鼠标移回原位置，窗口自动恢复
4. 觉得好用，托盘右键 →「把当前绑定存为常驻规则」，以后这个程序一打开就自动受保护

## 三种隐身方式

托盘右键 →「隐身方式」循环切换。

| 方式 | 做法 | 适用 |
|---|---|---|
| **透明** (默认) | 把窗口设为 0 透明度 | 最平滑，恢复位置完全不变，能挡住旁边人的视线 |
| **移出屏幕** | 把窗口挪到虚拟屏外 | 最彻底：连 Alt-Tab 缩略图里都看不到内容 |
| **隐藏窗口** | `ShowWindow(SW_HIDE)` | 兼容性最好，但任务栏按钮会留着 |

> 透明模式下窗口依然能被点击。这是刻意的——你可以在"隐形"状态下继续用它。

## 锁存模式

`Ctrl+Alt+L` 开关，或配置文件里 `latch = true`。

开启后：窗口一旦隐身，**鼠标移回来也不会自动显示**，只能用 `Ctrl+Alt+H` 或托盘手动恢复。

这是给"领导已经站在你旁边"的场景准备的。否则你鼠标一挪回去，窗口当着人家的面又弹出来了。

## 常驻规则（自动保护）

写了 `rule =` 之后，PeekGuard 每秒扫一次窗口列表，发现有窗口命中规则就自动接管，
**不需要每次手动绑定**。适合「QQ 一打开就该受保护」这种固定习惯。

扫描的挑窗口顺序，决定了多开时的行为：

1. 前台窗口里的命中者优先——你正在看的那个窗口总是优先被保护
2. 否则选 z 序最靠前的那个（即最上层）
3. **最小化的窗口直接跳过**：它根本没法隐身，选中它会让守卫卡在一个永远隐不掉的
   窗口上，结果反倒是你真正在用的那个窗口一直得不到保护。窗口还原后下一秒就会
   被重新选中，所以跳过不会漏保护。

命中的窗口会先做一次"能否隐身"的自检：如果真的隐不了（典型是被提权程序挡住），
会明确提示一次并记住这个窗口，**不会每秒重试刷屏**，更不会陷入隐身/恢复的来回抖动。

> 注意区分：**被守卫之后才最小化**的窗口会保留原有绑定（不主动换人），这是有意的；
> 上面第 3 条说的是扫描器**挑选新目标**时不选最小化的窗口。

### 开关

托盘菜单 →「规则自动保护」，带勾选状态。关掉只影响规则，手动绑定照常工作。

### 解绑的语义

这条规则很容易踩坑，所以明确写下来。解绑（`Ctrl+Alt+P` 或菜单「解绑当前窗口」）
**只针对当前这个窗口**，不是关掉规则：

| 操作 | 被解绑的那个窗口 | 之后新出现的同类窗口 |
|---|---|---|
| 解绑当前窗口 | 保持释放，规则不再抢回 | 照常自动保护 |
| 菜单关掉「规则自动保护」 | 立即释放 | 不保护，直到重新打开 |
| 重载配置 | 全部重新计算 | 照常自动保护 |

之所以这样设计：你要的通常不是"关掉整条规则"，而是"这次别管它"。
如果解绑就等于关规则，那你解绑一次 QQ，之后重新打开 QQ 就再也不受保护了——
这显然不是本意。

## 开机自启

托盘菜单 →「开机自动启动」，带勾选状态（实时读注册表，不缓存）。

实现在 `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` 下写一条 `PeekGuard`，
指向当前 exe 的完整路径（带引号，能处理路径里的空格）。
用注册表而不是启动文件夹/计划任务：不需要管理员权限、不弹控制台窗口、没有守护进程。

移动了 exe 位置之后，勾选状态仍显示为已开启（按名字识别），
取消再重新勾选一次即可指向新路径。

## 配置文件

`%APPDATA%\PeekGuard\peekguard.conf`，也可以用环境变量 `PEEKGUARD_CONFIG` 指向别处。
改完在托盘菜单点「重载配置」生效。

```ini
# 鼠标离开窗口后等待多少毫秒才隐身（防抖，避免鼠标一抖就闪）
delay_ms = 350

# 判定"鼠标在窗口内"时四周额外给的像素余量
margin_px = 6

# 隐身方式: alpha(透明) | move(移出屏幕) | hide(隐藏窗口)
mode = alpha

# alpha 模式下的透明度，0 = 完全不可见
alpha = 0

# true = 隐身之后必须手动恢复
latch = false

# true = 只在目标窗口是前台窗口时才响应鼠标
hide_only_when_focused = false

# 已经最小化的窗口不处理
# 已废弃：行为已无条件生效（最小化的窗口本来就没法隐身），保留键名只是
# 为了让旧配置不报警，值不再有任何效果。
skip_minimized = true

# 启动时自动绑定的窗口（可留空）
autobind =

# 常驻规则：窗口出现时自动纳入保护
# 字段用 | 分隔；title 内用 , 表示"或"
rule = process:QQ.exe
rule = process:msedge.exe | title:小说,股票
```

规则字段：`process:`（进程名，忽略大小写）、`class:`（窗口类名）、`title:`（标题子串，逗号分隔表示"或"）。
同一行内的多个字段是**与**关系。

## 已知限制

| 情况 | 表现 | 说明 |
|---|---|---|
| 目标以管理员身份运行 | 拒绝操作并提示 | Windows 的 UIPI 机制，提权进程的窗口不接受普通进程修改。**用管理员身份启动 PeekGuard 即可解决** |
| UWP 应用（设置、照片、计算器） | 可能无效 | 这类窗口由 `ApplicationFrameHost` 托管，实际窗口句柄不是你所看到的那个 |
| 使用 `UpdateLayeredWindow` 的程序 | 透明模式无效 | 自动检测；换「移出屏幕」方式即可 |
| 目标程序自己重设窗口样式 | 会被"治好" | 需要重新隐身 |
| 多显示器 / 负坐标 | 已处理 | 坐标基于虚拟屏空间，并启用 per-monitor DPI awareness |

### 安全边界

- **不注入、不子类化、不装全局钩子**——只用 `GetCursorPos` 每 40ms 轮询做几何判断，对目标进程零侵入
- 不用 DWM cloak：实测跨进程调用返回 `E_ACCESSDENIED`（见 [`docs/superpowers/specs`](docs/superpowers/specs/2026-10-10-peekguard-design.md)）
- **退出前一定恢复**：正常退出、托盘退出都会先还原所有窗口
- **崩溃也不留残局**：每次隐身都写日志到 `hidden.journal`，异常退出后下次启动自动恢复
- **不会出现"藏了但没记录"**：journal 是**先写、再动窗口**的。唯一不可恢复的状态是"窗口不可见而没有记录"，所以记录总是先落盘；反过来多写一条是无害的——
  "还原"一个本来就可见的窗口等于空操作。（这条不变式用 720 次高频采样验证过，
  见 [`docs/superpowers/specs`](docs/superpowers/specs/2026-10-10-peekguard-design.md)）
- **程序自己崩了也会还原**：装了 panic hook，即使 `panic = "abort"` 也保证在进程死前
  把窗口放回去（这一条有专门的端到端测试 `tests/e2e_panic.py` 验证）

## 开发

```sh
cargo build --release      # -> target/release/peekguard.exe (~275 KB，零依赖)
cargo test                 # 17 个单元测试（零副作用，另有 1 个 #[ignore] 诊断用例）

# 端到端（会移动真实鼠标，需要交互式桌面）
for m in alpha move hide; do PEEKGUARD_MODE=$m python tests/e2e_live.py; done         # 23 项
for m in alpha move hide; do PEEKGUARD_MODE=$m python tests/e2e_latch.py; done        # 15 项
for m in alpha move hide; do PEEKGUARD_MODE=$m python tests/e2e_rules.py; done        # 16 项
for m in alpha move hide; do PEEKGUARD_MODE=$m python tests/e2e_robustness.py; done   # 16 项
for m in alpha move hide; do PEEKGUARD_MODE=$m python tests/e2e_panic.py; done        # 10 项
```

合计 240 项端到端检查全绿（三个隐身模式各跑一遍）。每份测试都自己取一个独立的 `PEEKGUARD_MUTEX`
单实例名，所以不会和你正在跑的 PeekGuard 抢互斥；测试用的规则同时限定
`class:` 和 `title:`，只命中测试自己的窗口。

`PEEKGUARD_MODE=alpha|move|hide` 切换端到端测试的隐身方式，
`PEEKGUARD_DEBUG=1` 输出详细日志到 stderr 和 `%APPDATA%\PeekGuard\debug.log`。
其余环境变量见 [`tests/README.md`](tests/README.md)。

CI 跑 `cargo fmt --check`、`cargo clippy --all-targets -- -D warnings`、`cargo test`
和 `cargo build --release`。端到端测试需要交互式桌面（它们要移动真实鼠标），
所以不在 CI 里跑。

构建**零警告**。FFI 模块里刻意保留了 Win32 的类型名（`HWND`、`NOTIFYICONDATAW` 等），
这些 `upper_case_acronyms` 告警已就地 `#![allow]` 并注明原因——手写绑定就是要和
SDK 头文件逐行对得上，改名反而更难核对。

### 结构

| 文件 | 职责 |
|---|---|
| `src/win.rs` | 手写 Win32 FFI（零依赖，x86_64 only），含注册表 |
| `src/hide.rs` | 隐身原语 + 状态快照 / 无损还原 |
| `src/watch.rs` | 状态机：鼠标轮询、延迟防抖、锁存 |
| `src/rules.rs` | 常驻规则扫描：限速轮询、挑选目标、逐窗口抑制 |
| `src/config.rs` | 配置解析 + 规则匹配 |
| `src/autostart.rs` | 开机自启（直接读写 HKCU Run，无子进程） |
| `src/session.rs` | 崩溃恢复日志 |
| `src/tray.rs` | 托盘图标 + 菜单 |
| `src/icon.rs` | 运行时生成图标（无需资源编译器） |

## 代码签名策略（Code signing policy）

[SignPath.io](https://about.signpath.io) 提供免费签名服务，证书由
[SignPath Foundation](https://signpath.org) 签发。

> Free code signing provided by SignPath.io, certificate by SignPath Foundation.

每个发布的 `peekguard.exe` 都由本仓库的 GitHub Actions 构建，每次发布需人工审批后才签名。
因此签名所代表的含义是「该二进制文件是本仓库源码的自动化构建产物」，而不是「有人人工审计过这份代码」。

### 团队角色

| 角色 | 成员 | 职责 |
|---|---|---|
| Authors | [`Owners`](https://github.com/Reance195/peekguard/people?query=role%3Aowner) | 可直接修改源码与 CI，无需额外评审 |
| Reviewers | [`Owners`](https://github.com/Reance195/peekguard/people?query=role%3Aowner) | 评审每一个 Pull Request |
| Approvers | [`Owners`](https://github.com/Reance195/peekguard/people?query=role%3Aowner) | 审批每一次签名请求 |

这是单人维护项目，三种角色目前都指向同一个人。第一个被合并进来的外部贡献者将接管 Reviewer。

### 隐私

**PeekGuard 不会向任何网络系统传输信息。** 它完全没有网络代码 —— 无遥测、无更新检查、无崩溃上报。
`src/win.rs` 里列出了它导入的全部 Win32 API，其中没有任何一个用于打开套接字或网络连接。
项目零第三方依赖（`Cargo.toml` 没有 `[dependencies]` 段）。

它写入的文件全部在本地，完整清单如下：

| 路径 | 内容 | 写入时机 |
|---|---|---|
| `%APPDATA%\PeekGuard\peekguard.conf` | 配置文件 | 首次运行时写入；可用 `PEEKGUARD_CONFIG`，或放在 exe 同目录的 `peekguard.conf` 覆盖 |
| `%APPDATA%\PeekGuard\hidden.journal` | 崩溃恢复日志 | 仅在窗口处于隐身状态时存在；窗口恢复后删除 |
| `%APPDATA%\PeekGuard\debug.log` | 诊断日志 | 仅当 `PEEKGUARD_DEBUG=1` 时 |

托盘图标是运行时生成的，不会落盘。

PeekGuard 唯一可能对系统的修改，是在
`HKCU\Software\Microsoft\Windows\CurrentVersion\Run` 下写一个 `PeekGuard` 值，
且仅在你通过托盘菜单的 "start with Windows" 主动勾选后才会发生。取消勾选即删除该值。

它不会注入代码、不对窗口做子类化、不安装全局钩子。唯一的跨进程调用是
`PROCESS_QUERY_LIMITED_INFORMATION`，用于读取窗口的进程名以匹配 `rule = process:…`，
以及判断目标进程是否以管理员权限运行。

### 卸载

1. 托盘菜单 → 退出
2. 若勾选了 "start with Windows"，取消勾选（这一步会删除注册表值）
3. 删除 exe
4. 删除 `%APPDATA%\PeekGuard\`

## License

[MIT](LICENSE) © 2026 PeekGuard contributors