# 桌面端真机验收 · 2026-08-18

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
| 提交 | `cf274e869872dca20bc324c96b4944397a4ed581` |
| 操作系统构建 | `Ubuntu 24.04.4 LTS / Linux 6.17.0-1019-aws` |
| 会话 | `virtual`（`DISPLAY=:99`） |
| 窗口管理器 | `Openbox` |
| 非 monitor 音频输入设备 | 0 |
| 真实采集探测 | 本次 xtask 未开 `capture` 特性，没有做真实采集探测；用 `cargo run -p xtask --features capture -- acceptance` 才会做 |

> [!NOTE]
> 会话是 **Xvfb 虚拟显示**加一个真实窗口管理器，不是物理显示器。窗口映射、
> `_NET_WM_STATE` 迁移、合成输入这些**窗口管理器层**的事实在这里成立；
> 而依赖真实显示硬件的事实（GPU 合成路径、HiDPI 缩放、屏幕色彩）不成立，
> 本报告不对它们下结论。

## 汇总

声明 20 条 · PASS 18 · FAIL 1 · NOT EXECUTED 1

## WebDriver 握手探测

尝试：是 · 成功：是

会话建立成功，DOM 层断言由真实 WebDriver 执行。

## 逐条断言

| 断言 | 通道 | 裁决 | 依据 | 截图 |
| --- | --- | --- | --- | --- |
| `artifact_present`<br>构建产物存在且可执行，动态库依赖全部可解析 | 进程 | **PASS** | /config/workspace/ProdDir/AI/yunjian/target/debug/yunjian-desktop 存在，ldd 未报缺失动态库 | — |
| `installer_runs`<br>安装包（.deb / NSIS / .dmg）能安装并从安装后的路径启动 | 进程 | **PASS** | 把 云笺_0.1.0_amd64.deb 解到 /config/workspace/ProdDir/AI/yunjian/target/acceptance/install-root 后，从安装路径 /config/workspace/ProdDir/AI/yunjian/target/acceptance/install-root/usr/bin/yunjian-desktop 启动，映射出顶层窗口且 `_NET_WM_NAME` = 「云笺」 | — |
| `app_launches`<br>应用在交互式桌面会话里启动并映射出一个顶层窗口 | OS 输入 | **PASS** | 窗口 0x600003 已映射，`_NET_WM_NAME` = 「云笺」 | [`desktop-qa/app-launched.png`](desktop-qa/app-launched.png) |
| `custom_titlebar_rendered`<br>窗口没有窗口管理器绘制的边框（decorations: false 生效），标题栏由应用自绘 | OS 输入 | **PASS** | `_NET_FRAME_EXTENTS` = [0, 0, 0, 0]，窗口管理器没有绘制边框；标题栏由应用自绘 | [`desktop-qa/custom-titlebar.png`](desktop-qa/custom-titlebar.png) |
| `control_minimize_works`<br>点自绘标题栏的最小化按钮，窗口真的最小化 | OS 输入 | **PASS** | 点最小化按钮后 `_NET_WM_STATE` 带上了 `_NET_WM_STATE_HIDDEN` | [`desktop-qa/control-minimize.png`](desktop-qa/control-minimize.png) |
| `control_maximize_works`<br>点自绘标题栏的最大化按钮，窗口真的最大化 | OS 输入 | **PASS** | 起始态已归一为非最大化，点最大化按钮后 `_NET_WM_STATE` 带上了 `_NET_WM_STATE_MAXIMIZED_VERT/HORZ` | [`desktop-qa/control-maximize.png`](desktop-qa/control-maximize.png) |
| `control_restore_works`<br>最大化后再点同一个按钮，窗口真的还原 | OS 输入 | **PASS** | 再点同一个按钮后最大化状态被清除，窗口还原 | [`desktop-qa/control-restore.png`](desktop-qa/control-restore.png) |
| `control_close_works`<br>点自绘标题栏的关闭按钮，主窗口隐藏到托盘且进程继续运行 | OS 输入 | **PASS** | 点关闭按钮后主窗口从 `_NET_CLIENT_LIST` 消失，应用进程仍在运行，符合驻留托盘契约 | [`desktop-qa/control-close.png`](desktop-qa/control-close.png) |
| `double_click_maximizes_exactly_once`<br>双击标题栏恰好最大化一次（自己再挂一个双击处理器会双切换回原样） | OS 输入 | **PASS** | 双击前 maximized=false，一次双击后 maximized=true（恰好切换一次） | [`desktop-qa/double-click-maximize.png`](desktop-qa/double-click-maximize.png) |
| `drag_from_title_text`<br>按住标题文字本身拖动，窗口位置真的改变（data-tauri-drag-region="deep"） | OS 输入 | **PASS** | 按住标题文字（窗口内 18,20）拖动后，窗口从 (200, 150) 移到 (300, 225) | [`desktop-qa/drag-from-title-text.png`](desktop-qa/drag-from-title-text.png) |
| `taskbar_icon_correct`<br>窗口带 _NET_WM_ICON，任务栏能取到图标 | OS 输入 | **PASS** | `_NET_WM_ICON` 有 2306 个 32 位字（宽高 2 字 + 逐像素 ARGB），任务栏与 alt-tab 能取到图标 | [`desktop-qa/taskbar-icon.png`](desktop-qa/taskbar-icon.png) |
| `tray_icon_correct`<br>托盘图标存在且背景透明 | OS 输入 | **PASS** | 托盘项已在会话总线上注册（`Status`=「Active」），运行期图标为 /run/user/1000/tray-icon/tray-icon-3824140-1-0.png（32×32），其中 46.9% 的像素 alpha 为 0，背景确为透明 | [`desktop-qa/tray-icon.png`](desktop-qa/tray-icon.png) |
| `ime_prefilled_search_box`<br>中文输入法往一个**已有内容**的检索框里输入：不冻结且字符落入框内（tauri#15436） | WebDriver | **PASS** | 检索框预填「明月」后再次聚焦并输入「千里」，`input.value` 为「明月千里」——字符全部落入框内，无吞字 | [`desktop-qa/ime-prefilled-dom.png`](desktop-qa/ime-prefilled-dom.png) |
| `ime_prefilled_search_box_no_freeze`<br>承 tauri#15436：聚焦已有内容的检索框并输入后，界面仍然响应（OS 层可证的那一半） | OS 输入 | **PASS** | 往检索框输入中文后再次聚焦并继续输入，窗口仍响应双击最大化，未冻结 | [`desktop-qa/ime-prefilled-no-freeze.png`](desktop-qa/ime-prefilled-no-freeze.png) |
| `two_char_search_returns_results`<br>两字检索（明月）返回结果行 | WebDriver | **PASS** | 检索「明月」后摘要为「估计命中 7291 条，本页显示 20 条」 | [`desktop-qa/two-char-search.png`](desktop-qa/two-char-search.png) |
| `corpus_first_run_materialization`<br>首启语料物化完成并显示进度 | WebDriver | **PASS** | 起始状态是 `corpus-absent`（空数据目录，确为首启），物化过程显示了 20 条不同的进度文本（「正在核对归档摘要212.8 MiB」、「归档摘要已核对」、「正在解压语料库48.1 MiB」 …… 中间省略 14 条 …… 「正在解压语料库602.1 MiB」、「正在本机派生检索结构读取诗词正文」、「正在本机派生检索结构100%读取诗词正文（474,043 / 474,043 首）」），完成后渲染出 `corpus-facts`（语料版本0.1.0收录作品474,043 首schema 版本2构建时间2026-08-10T00:00:00Z索引模式full派生索引first_launch随包范围tang-song）。进度块按设计在收工后让位给事实表，所以那张整屏截图拍不到它；物化中途另截了一张 `corpus-progress.png` | [`desktop-qa/corpus-first-run.png`](desktop-qa/corpus-first-run.png) |
| `shipped_appreciation_without_key`<br>没有 API key 时随包赏析仍能渲染，且带「AI 赏析」标签与未审校说明 | WebDriver | **FAIL** | 未配置 API key 下打开「出塞」，赏析面板缺少：赏析正文仍是未生成标记（随包数据集不是模型输出）；实测 `ai-source`=「随包预生成」、标签=「AI 赏析」、说明=「AI 生成，未经人工审校」、正文=「<<未生成：本条不是模型输出，需开放权重模型推理>>」 | [`desktop-qa/shipped-appreciation.png`](desktop-qa/shipped-appreciation.png) |
| `voice_round_succeeds_end_to_end`<br>语音背诵一轮端到端成功：采集 -> ASR -> 无偏置评分这条链跑通 | WebDriver | **NOT EXECUTED** | 被测产物未编译 `voice` 特性，语音在可用性探测这一步就降级为「本版本未编译语音」，采集与 ASR 都不存在，这一轮无从判定成功；降级本身由 `voice_degradation_states_reason` 单独判定。判词：「本版本未编译语音 —— 本版本未编译语音能力，已切换到打字练习。打字练习的评分与语音练习共用同一个内核，功能完整。」<br>**可执行条件**：用 `--features custom-protocol,voice` 构建被测产物（该特性同时是许可边界，见 `docs/VOICE-BUILD.zh.md`），并提供一个非 monitor 的可采集输入设备 | [`desktop-qa/voice-round.png`](desktop-qa/voice-round.png) |
| `voice_degradation_states_reason`<br>失败路径单独验证：语音不可用时切到打字模式并显示具体原因 | WebDriver | **PASS** | 把模型目录指空之后语音降级，界面报出的具体原因是「本版本未编译语音」（在契约定义的十个原因码内），完整判词「本版本未编译语音 —— 本版本未编译语音能力，已切换到打字练习。打字练习的评分与语音练习共用同一个内核，功能完整。」，并已切到打字练习（`voice-typed-handoff` 与 `recite-answer` 都在） | [`desktop-qa/voice-degradation.png`](desktop-qa/voice-degradation.png) |
| `app_exits_cleanly`<br>从托盘菜单选择「退出」后，应用以退出码 0 正常结束且不留孤儿进程 | 进程 | **PASS** | 从托盘菜单点「退出」（菜单项 id 5，实际菜单为 显示/隐藏主窗、今日复习、设置、退出）后，应用以退出码 0 结束，且没有留下同名孤儿进程 | — |

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
