# PeekGuard uTools 插件

把 [PeekGuard](../)（防窥守卫）装进 uTools：一条指令看状态、隐身、绑定窗口，
不用离开键盘去点托盘图标。

插件本身没有任何构建步骤，纯原生 HTML/CSS/JS + Node.js preload，
通过命名管道与 PeekGuard 主程序通信。插件目录已附带 `peekguard.exe`，安装后即可使用。

## 安装

- **应用市场**：在 uTools 插件市场搜索「PeekGuard」或「防窥守卫」，安装即可。
- **本地调试**：uTools → 开发者工具 → 项目 → 新建 → 选择本目录（`utools-plugin/`）。

主程序（`peekguard.exe`）已随插件附带；如内置版本被清理或需要更新，可在面板点
「下载 PeekGuard」从 [GitHub Releases](https://github.com/Reance195/peekguard/releases/latest)
下载最新版，放到“下载”目录后点「重新检测」。也可以手动指定程序路径，或从 Release 页面
自行下载安装。

面板在未检测到运行时，会按下面的顺序自动探测，找不到可在面板里手动选择并记住路径。

1. 面板里手动指定的路径（记住后优先生效）
2. `PEEKGUARD_EXE` 环境变量
3. `%LOCALAPPDATA%\Programs\peekguard\peekguard.exe`
4. `%USERPROFILE%\Downloads\peekguard.exe`
5. `%USERPROFILE%\Desktop\peekguard.exe`
6. 插件目录下的 `peekguard.exe`
7. `PATH` 里的 `peekguard.exe`

## 指令

| 指令 | 行为 |
| --- | --- |
| `防窥` / `peekguard` / `防窥守卫` | 打开控制面板（状态、按钮、开关，随主程序实时刷新） |
| `隐身` / `隐身切换` / `hide` | 不开窗口，立即隐身/恢复被绑定的窗口 |
| `绑定窗口` / `保护窗口` / `bind` | 不开窗口，直接打开十字光标选择器 |

面板里可以：隐身/恢复、绑定/解绑、重载配置、开关键盘锁存与规则保护、
切换隐身方式（透明 / 移出屏幕 / 隐藏窗口）、开关开机自启、指定主程序路径。

## 与主程序的协议

插件把一行文本命令写进命名管道 `\\.\pipe\PeekGuard-IPC-v1`
（可用 `PEEKGUARD_PIPE` 改），读回一行 JSON。

| 命令 | 说明 |
| --- | --- |
| `status` / `ping` | 查询状态，无副作用 |
| `hide` / `show` / `toggle` | 隐身 / 恢复 / 切换 |
| `bind` / `unbind` | 打开十字光标选择器 / 解绑并恢复 |
| `latch on\|off\|toggle` | 锁存模式 |
| `mode alpha\|move\|hide` | 隐身方式 |
| `scan on\|off\|toggle` | 规则自动保护 |
| `reload` | 重载配置文件 |
| `autostart on\|off\|toggle` | 开机自启 |

响应始终是一行 JSON，`state` 是执行后的完整状态快照：

```json
{"ok":true,"msg":"已隐身","state":{"version":"0.2.0","bound":true,"bound_desc":"QQ.exe — 消息","bound_process":"QQ.exe","state":"hidden","hidden":true,"mode":"alpha","latch":false,"delay_ms":60,"rules":0,"scan":true,"picking":false,"elevated":false,"autostart":false,"config":"C:\\Users\\…\\peekguard.conf"}}
```

失败时为 `{"ok":false,"err":"…","state":{…}}`。
`state.state` 取值：`idle`（未绑定）/ `armed`（守护中）/ `pending`（即将隐身）/
`hidden`（已隐身）/ `latched`（已隐身·锁定）。
出于安全考虑，协议里没有“退出”“下载”“执行任意程序”一类的命令。

## 命令行测试工具

不依赖 uTools 也能验证主程序与协议（连的管道名可用 `PEEKGUARD_PIPE` 指定）：

```powershell
node test-cli.js            # 跑全套用例（只做可复原的操作）
node test-cli.js status     # 发单条命令并打印 JSON 响应
node test-cli.js latch on   # 带参数的命令
```

退出码：`0` = 成功；`1` = 命令被拒绝（`ok:false`）；`2` = 连不上主程序。

## 环境变量

| 变量 | 用途 |
| --- | --- |
| `PEEKGUARD_PIPE` | 自定义管道名（主程序与客户端需一致，测试隔离用） |
| `PEEKGUARD_EXE` | 指定主程序路径（探测顺序第 2 位） |

## 目录结构

```
utools-plugin/
├── plugin.json        # uTools 插件清单（入口/图标/preload/指令）
├── preload.js         # 桥接层：管道通信、路径探测与启动（仅 Node 原生模块）
├── index.html         # 控制面板结构
├── app.js             # 面板逻辑与快捷指令处理
├── style.css          # 样式（跟随系统深浅色）
├── logo.png           # 256×256 图标（与主程序图标同源）
├── peekguard.exe      # 随插件附带的主程序（Windows）
├── test-cli.js        # 不依赖 uTools 的命令行测试工具
├── tools/
│   └── make-logo.ps1  # 重新生成 logo.png（pwsh + System.Drawing）
└── README.md
```
