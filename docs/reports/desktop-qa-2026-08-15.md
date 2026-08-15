# 桌面端真机验收 · 2026-08-15

> [!WARNING]
> **`all_pass` = `false`。本次只在 `linux` 上执行。**
> 本文件是**一次运行**的报告，`platform` 字段就是那一次的平台。文件名只带日期，
> 所以同一天换个平台再跑会覆盖它——要保留多平台结果，跑完一个平台先把报告归档。
> `all_pass` 的语义是**零 FAIL 且零 NOT EXECUTED**，因此只要有任何一条未执行
> 它就是 `false`。它**不能**被读成「三平台都过了」。逐平台执行状态见下方
> 「未执行的平台」一节。

## 被测对象

| 项 | 值 |
| --- | --- |
| 平台 | `linux` |
| 断言集 | `desktop` |
| 应用版本 | `0.1.0` |
| 提交 | `eb4d0b1e192bc706f96edee583613318175bb9f3` |
| 操作系统构建 | `Ubuntu 24.04.4 LTS / Linux 6.17.0-1019-aws` |
| 会话 | `virtual`（`DISPLAY=:99`） |
| 窗口管理器 | `Openbox` |
| 音频输入设备 | 0 |

> [!NOTE]
> 会话是 **Xvfb 虚拟显示**加一个真实窗口管理器，不是物理显示器。窗口映射、
> `_NET_WM_STATE` 迁移、合成输入这些**窗口管理器层**的事实在这里成立；
> 而依赖真实显示硬件的事实（GPU 合成路径、HiDPI 缩放、屏幕色彩）不成立，
> 本报告不对它们下结论。

## 汇总

声明 20 条 · PASS 12 · FAIL 0 · NOT EXECUTED 8

## WebDriver 握手探测

尝试：是 · 成功：是

会话建立成功，DOM 层断言由真实 WebDriver 执行。

## 逐条断言

| 断言 | 通道 | 裁决 | 依据 | 截图 |
| --- | --- | --- | --- | --- |
| `artifact_present`<br>构建产物存在且可执行，动态库依赖全部可解析 | 进程 | **PASS** | /config/workspace/ProdDir/AI/yj-fix-input/target/debug/yunjian-desktop 存在，ldd 未报缺失动态库 | — |
| `installer_runs`<br>安装包（.deb / NSIS / .dmg）能安装并从安装后的路径启动 | 进程 | **NOT EXECUTED** | /config/workspace/ProdDir/AI/yj-fix-input/target/debug/bundle 下没有安装包产物；本次验收跑的是未打包的构建产物，因此「安装后从安装路径启动」这条链没有被执行<br>**可执行条件**：先 `cargo tauri build`（Linux 上还需 `dpkg-deb` 与 `librsvg2-dev`），再由 harness 安装到一个临时 root 并从该路径启动。NSIS 静默安装时**只等安装器进程本身**，不要等整棵进程树——完成页会把应用作为子进程拉起来，等进程树会永远等不完。 | — |
| `app_launches`<br>应用在交互式桌面会话里启动并映射出一个顶层窗口 | OS 输入 | **PASS** | 窗口 0x600003 已映射，`_NET_WM_NAME` = 「云笺」 | [`desktop-qa/app-launched.png`](desktop-qa/app-launched.png) |
| `custom_titlebar_rendered`<br>窗口没有窗口管理器绘制的边框（decorations: false 生效），标题栏由应用自绘 | OS 输入 | **PASS** | `_NET_FRAME_EXTENTS` = [0, 0, 0, 0]，窗口管理器没有绘制边框；标题栏由应用自绘 | [`desktop-qa/custom-titlebar.png`](desktop-qa/custom-titlebar.png) |
| `control_minimize_works`<br>点自绘标题栏的最小化按钮，窗口真的最小化 | OS 输入 | **PASS** | 点最小化按钮后 `_NET_WM_STATE` 带上了 `_NET_WM_STATE_HIDDEN` | [`desktop-qa/control-minimize.png`](desktop-qa/control-minimize.png) |
| `control_maximize_works`<br>点自绘标题栏的最大化按钮，窗口真的最大化 | OS 输入 | **PASS** | 起始态已归一为非最大化，点最大化按钮后 `_NET_WM_STATE` 带上了 `_NET_WM_STATE_MAXIMIZED_VERT/HORZ` | [`desktop-qa/control-maximize.png`](desktop-qa/control-maximize.png) |
| `control_restore_works`<br>最大化后再点同一个按钮，窗口真的还原 | OS 输入 | **PASS** | 再点同一个按钮后最大化状态被清除，窗口还原 | [`desktop-qa/control-restore.png`](desktop-qa/control-restore.png) |
| `control_close_works`<br>点自绘标题栏的关闭按钮，主窗口隐藏到托盘且进程继续运行 | OS 输入 | **PASS** | 点关闭按钮后主窗口从 `_NET_CLIENT_LIST` 消失，应用进程仍在运行，符合驻留托盘契约 | [`desktop-qa/control-close.png`](desktop-qa/control-close.png) |
| `double_click_maximizes_exactly_once`<br>双击标题栏恰好最大化一次（自己再挂一个双击处理器会双切换回原样） | OS 输入 | **PASS** | 双击前 maximized=false，一次双击后 maximized=true（恰好切换一次） | [`desktop-qa/double-click-maximize.png`](desktop-qa/double-click-maximize.png) |
| `drag_from_title_text`<br>按住标题文字本身拖动，窗口位置真的改变（data-tauri-drag-region="deep"） | OS 输入 | **PASS** | 按住标题文字（窗口内 18,20）拖动后，窗口从 (200, 150) 移到 (300, 225) | [`desktop-qa/drag-from-title-text.png`](desktop-qa/drag-from-title-text.png) |
| `taskbar_icon_correct`<br>窗口带 _NET_WM_ICON，任务栏能取到图标 | OS 输入 | **PASS** | `_NET_WM_ICON` 有 2306 个 32 位字（宽高 2 字 + 逐像素 ARGB），任务栏与 alt-tab 能取到图标 | [`desktop-qa/taskbar-icon.png`](desktop-qa/taskbar-icon.png) |
| `tray_icon_correct`<br>托盘图标存在且背景透明 | OS 输入 | **NOT EXECUTED** | 应用已通过 `TrayIconBuilder` 创建托盘图标，但本次 Xvfb + Openbox 会话没有StatusNotifier/AppIndicator 托盘宿主，harness 因而没有可观测的托盘项；图标资产透明度仍由 `xtask verify-icons` 逐字节守卫<br>**可执行条件**：带 StatusNotifier/AppIndicator 托盘宿主的交互式 Linux 桌面，并由 harness 从托盘协议侧观测图标 | — |
| `ime_prefilled_search_box`<br>中文输入法往一个**已有内容**的检索框里输入：不冻结且字符落入框内（tauri#15436） | WebDriver | **NOT EXECUTED** | 会话已建立，但本条的驱动步骤尚未实现<br>**可执行条件**：实现该条的 WebDriver 驱动步骤 | — |
| `ime_prefilled_search_box_no_freeze`<br>承 tauri#15436：聚焦已有内容的检索框并输入后，界面仍然响应（OS 层可证的那一半） | OS 输入 | **PASS** | 往检索框输入中文后再次聚焦并继续输入，窗口仍响应双击最大化，未冻结 | [`desktop-qa/ime-prefilled-no-freeze.png`](desktop-qa/ime-prefilled-no-freeze.png) |
| `two_char_search_returns_results`<br>两字检索（明月）返回结果行 | WebDriver | **PASS** | 检索「明月」后摘要为「估计命中 7291 条，本页显示 20 条」 | [`desktop-qa/two-char-search.png`](desktop-qa/two-char-search.png) |
| `corpus_first_run_materialization`<br>首启语料物化完成并显示进度 | WebDriver | **NOT EXECUTED** | 会话已建立，但本条的驱动步骤尚未实现<br>**可执行条件**：实现该条的 WebDriver 驱动步骤 | — |
| `shipped_appreciation_without_key`<br>没有 API key 时随包赏析仍能渲染，且带「AI 赏析」标签与未审校说明 | WebDriver | **NOT EXECUTED** | 会话已建立，但本条的驱动步骤尚未实现<br>**可执行条件**：实现该条的 WebDriver 驱动步骤 | — |
| `voice_round_succeeds_end_to_end`<br>语音背诵一轮端到端成功：采集 -> ASR -> 无偏置评分这条链跑通 | WebDriver | **NOT EXECUTED** | 会话已建立，但本条的驱动步骤尚未实现<br>**可执行条件**：实现该条的 WebDriver 驱动步骤 | — |
| `voice_degradation_states_reason`<br>失败路径单独验证：语音不可用时切到打字模式并显示具体原因 | WebDriver | **NOT EXECUTED** | 会话已建立，但本条的驱动步骤尚未实现<br>**可执行条件**：实现该条的 WebDriver 驱动步骤 | — |
| `app_exits_cleanly`<br>从托盘菜单选择「退出」后，应用以退出码 0 正常结束且不留孤儿进程 | 进程 | **NOT EXECUTED** | 应用的正常退出入口是托盘菜单「退出」；本次 Xvfb + Openbox 会话没有托盘宿主，harness 无法显示并点击该菜单，未用 kill 信号冒充正常退出<br>**可执行条件**：带 StatusNotifier/AppIndicator 托盘宿主的交互式 Linux 桌面，并由 harness 观测托盘项后点击「退出」 | — |

## 未执行的平台

| 平台 | 原因 | 可执行条件 |
| --- | --- | --- |
| `windows` | 本机是 Linux 容器，没有 Windows 宿主机，也没有 msedgedriver；session 0 的服务账户跑不出交互式桌面会话 | 一台装有 WebView2 运行时的 Windows 11 交互式登录会话，加上与 Edge 主版本号匹配的 msedgedriver |
| `macos` | 本机没有 macOS 宿主机；且 WKWebView 没有官方 WebDriver 工具，公证构建还需要签名身份 | 一台交互式登录的 macOS 机器、一个 Developer ID 签名身份，以及 @wdio/tauri-service 的 embedded WebDriver（macOS 唯一可行的驱动方式） |

## `all_pass` 的语义，以及退出码为什么与它不同义

`all_pass` 取**最严格**的定义：零 FAIL 且零 NOT EXECUTED。终验会消费这个字段，
而它最容易造成的误读是「三平台都过了」——只要 Windows 或 macOS 未执行，那句话
就是假的。因此宁可让它在任何缺口下都为 `false`，另用 `executed_pass` / `failed` /
`not_executed` 三个计数表达细节。

退出码只看 `FAIL`。`NOT EXECUTED` 是 harness 尽了责的结果——它如实说明了自己
做不到什么，那不是缺陷。把未执行也判成非零会奖励一个很坏的动作：**删掉**跑不了
的断言好让命令变绿。
