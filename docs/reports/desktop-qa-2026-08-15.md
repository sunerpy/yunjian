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
| 提交 | `38b1b4ca84fa08ec66d2a0974d1b6f75d7f3e55e` |
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

声明 20 条 · PASS 3 · FAIL 0 · NOT EXECUTED 17

## WebDriver 握手探测

尝试：是 · 成功：否

`POST /session` 未能返回会话：POST http://127.0.0.1:4444/session 失败。本机实测：应用进程真的起来了、真实窗口映射成功、WebKit 的 inspector server 端口也在监听，但 `WebKitWebDriver` 从不向它建立连接，请求一直挂到超时。

## 逐条断言

| 断言 | 通道 | 裁决 | 依据 | 截图 |
| --- | --- | --- | --- | --- |
| `artifact_present`<br>构建产物存在且可执行，动态库依赖全部可解析 | 进程 | **PASS** | /config/workspace/ProdDir/AI/yunjian/.omo/worktrees/fix-desktop-defects/target/debug/yunjian-desktop 存在，ldd 未报缺失动态库 | — |
| `installer_runs`<br>安装包（.deb / NSIS / .dmg）能安装并从安装后的路径启动 | 进程 | **NOT EXECUTED** | /config/workspace/ProdDir/AI/yunjian/.omo/worktrees/fix-desktop-defects/target/debug/bundle 下没有安装包产物；本次验收跑的是未打包的构建产物，因此「安装后从安装路径启动」这条链没有被执行<br>**可执行条件**：先 `cargo tauri build`（Linux 上还需 `dpkg-deb` 与 `librsvg2-dev`），再由 harness 安装到一个临时 root 并从该路径启动。NSIS 静默安装时**只等安装器进程本身**，不要等整棵进程树——完成页会把应用作为子进程拉起来，等进程树会永远等不完。 | — |
| `app_launches`<br>应用在交互式桌面会话里启动并映射出一个顶层窗口 | OS 输入 | **PASS** | 窗口 0x600003 已映射，`_NET_WM_NAME` = 「云笺」 | [`desktop-qa/app-launched.png`](desktop-qa/app-launched.png) |
| `custom_titlebar_rendered`<br>窗口没有窗口管理器绘制的边框（decorations: false 生效），标题栏由应用自绘 | OS 输入 | **NOT EXECUTED** | 窗口内容不可观测：等了 20 秒，主色 #f8f8f8 占 100.0000%，显著颜色区 1 个。窗口本身是真的（已映射、`_NET_WM_NAME` 正确、`_NET_WM_ICON` 齐备），但 X `GetImage` 读回来的是一整片单色。本机分不清两种原因——前端根本没渲染，还是 WebKit 把内容合成到了一块 X 读不到的 GL 表面（日志里有 `libEGL warning: DRI3 error: Could not get DRI3 device`，指向后者）。两种情况下点击都落在空处，因此「点了按钮之后窗口状态没变」证不了任何关于产品的事——记成 FAIL 会是一个假故障，故记未执行<br>**可执行条件**：一个 WebKitGTK 会把内容真的合成到窗口上的宿主机。本机（无 GPU 的容器 + Xvfb）实测不会：`WEBKIT_DISABLE_COMPOSITING_MODE=1`、`WEBKIT_DISABLE_DMABUF_RENDERER=1`、`LIBGL_ALWAYS_SOFTWARE=1` 三个单独与组合都试过，读回来始终是单色。一台有真实显示与可用 GL 栈的 Linux 桌面登录会话即可 | [`desktop-qa/webview-paint.png`](desktop-qa/webview-paint.png) |
| `control_minimize_works`<br>点自绘标题栏的最小化按钮，窗口真的最小化 | OS 输入 | **NOT EXECUTED** | 窗口内容不可观测：等了 20 秒，主色 #f8f8f8 占 100.0000%，显著颜色区 1 个。窗口本身是真的（已映射、`_NET_WM_NAME` 正确、`_NET_WM_ICON` 齐备），但 X `GetImage` 读回来的是一整片单色。本机分不清两种原因——前端根本没渲染，还是 WebKit 把内容合成到了一块 X 读不到的 GL 表面（日志里有 `libEGL warning: DRI3 error: Could not get DRI3 device`，指向后者）。两种情况下点击都落在空处，因此「点了按钮之后窗口状态没变」证不了任何关于产品的事——记成 FAIL 会是一个假故障，故记未执行<br>**可执行条件**：一个 WebKitGTK 会把内容真的合成到窗口上的宿主机。本机（无 GPU 的容器 + Xvfb）实测不会：`WEBKIT_DISABLE_COMPOSITING_MODE=1`、`WEBKIT_DISABLE_DMABUF_RENDERER=1`、`LIBGL_ALWAYS_SOFTWARE=1` 三个单独与组合都试过，读回来始终是单色。一台有真实显示与可用 GL 栈的 Linux 桌面登录会话即可 | [`desktop-qa/webview-paint.png`](desktop-qa/webview-paint.png) |
| `control_maximize_works`<br>点自绘标题栏的最大化按钮，窗口真的最大化 | OS 输入 | **NOT EXECUTED** | 窗口内容不可观测：等了 20 秒，主色 #f8f8f8 占 100.0000%，显著颜色区 1 个。窗口本身是真的（已映射、`_NET_WM_NAME` 正确、`_NET_WM_ICON` 齐备），但 X `GetImage` 读回来的是一整片单色。本机分不清两种原因——前端根本没渲染，还是 WebKit 把内容合成到了一块 X 读不到的 GL 表面（日志里有 `libEGL warning: DRI3 error: Could not get DRI3 device`，指向后者）。两种情况下点击都落在空处，因此「点了按钮之后窗口状态没变」证不了任何关于产品的事——记成 FAIL 会是一个假故障，故记未执行<br>**可执行条件**：一个 WebKitGTK 会把内容真的合成到窗口上的宿主机。本机（无 GPU 的容器 + Xvfb）实测不会：`WEBKIT_DISABLE_COMPOSITING_MODE=1`、`WEBKIT_DISABLE_DMABUF_RENDERER=1`、`LIBGL_ALWAYS_SOFTWARE=1` 三个单独与组合都试过，读回来始终是单色。一台有真实显示与可用 GL 栈的 Linux 桌面登录会话即可 | [`desktop-qa/webview-paint.png`](desktop-qa/webview-paint.png) |
| `control_restore_works`<br>最大化后再点同一个按钮，窗口真的还原 | OS 输入 | **NOT EXECUTED** | 窗口内容不可观测：等了 20 秒，主色 #f8f8f8 占 100.0000%，显著颜色区 1 个。窗口本身是真的（已映射、`_NET_WM_NAME` 正确、`_NET_WM_ICON` 齐备），但 X `GetImage` 读回来的是一整片单色。本机分不清两种原因——前端根本没渲染，还是 WebKit 把内容合成到了一块 X 读不到的 GL 表面（日志里有 `libEGL warning: DRI3 error: Could not get DRI3 device`，指向后者）。两种情况下点击都落在空处，因此「点了按钮之后窗口状态没变」证不了任何关于产品的事——记成 FAIL 会是一个假故障，故记未执行<br>**可执行条件**：一个 WebKitGTK 会把内容真的合成到窗口上的宿主机。本机（无 GPU 的容器 + Xvfb）实测不会：`WEBKIT_DISABLE_COMPOSITING_MODE=1`、`WEBKIT_DISABLE_DMABUF_RENDERER=1`、`LIBGL_ALWAYS_SOFTWARE=1` 三个单独与组合都试过，读回来始终是单色。一台有真实显示与可用 GL 栈的 Linux 桌面登录会话即可 | [`desktop-qa/webview-paint.png`](desktop-qa/webview-paint.png) |
| `control_close_works`<br>点自绘标题栏的关闭按钮，主窗口隐藏到托盘且进程继续运行 | OS 输入 | **NOT EXECUTED** | 窗口内容不可观测：等了 20 秒，主色 #f8f8f8 占 100.0000%，显著颜色区 1 个。窗口本身是真的（已映射、`_NET_WM_NAME` 正确、`_NET_WM_ICON` 齐备），但 X `GetImage` 读回来的是一整片单色。本机分不清两种原因——前端根本没渲染，还是 WebKit 把内容合成到了一块 X 读不到的 GL 表面（日志里有 `libEGL warning: DRI3 error: Could not get DRI3 device`，指向后者）。两种情况下点击都落在空处，因此「点了按钮之后窗口状态没变」证不了任何关于产品的事——记成 FAIL 会是一个假故障，故记未执行<br>**可执行条件**：一个 WebKitGTK 会把内容真的合成到窗口上的宿主机。本机（无 GPU 的容器 + Xvfb）实测不会：`WEBKIT_DISABLE_COMPOSITING_MODE=1`、`WEBKIT_DISABLE_DMABUF_RENDERER=1`、`LIBGL_ALWAYS_SOFTWARE=1` 三个单独与组合都试过，读回来始终是单色。一台有真实显示与可用 GL 栈的 Linux 桌面登录会话即可 | [`desktop-qa/webview-paint.png`](desktop-qa/webview-paint.png) |
| `double_click_maximizes_exactly_once`<br>双击标题栏恰好最大化一次（自己再挂一个双击处理器会双切换回原样） | OS 输入 | **NOT EXECUTED** | 窗口内容不可观测：等了 20 秒，主色 #f8f8f8 占 100.0000%，显著颜色区 1 个。窗口本身是真的（已映射、`_NET_WM_NAME` 正确、`_NET_WM_ICON` 齐备），但 X `GetImage` 读回来的是一整片单色。本机分不清两种原因——前端根本没渲染，还是 WebKit 把内容合成到了一块 X 读不到的 GL 表面（日志里有 `libEGL warning: DRI3 error: Could not get DRI3 device`，指向后者）。两种情况下点击都落在空处，因此「点了按钮之后窗口状态没变」证不了任何关于产品的事——记成 FAIL 会是一个假故障，故记未执行<br>**可执行条件**：一个 WebKitGTK 会把内容真的合成到窗口上的宿主机。本机（无 GPU 的容器 + Xvfb）实测不会：`WEBKIT_DISABLE_COMPOSITING_MODE=1`、`WEBKIT_DISABLE_DMABUF_RENDERER=1`、`LIBGL_ALWAYS_SOFTWARE=1` 三个单独与组合都试过，读回来始终是单色。一台有真实显示与可用 GL 栈的 Linux 桌面登录会话即可 | [`desktop-qa/webview-paint.png`](desktop-qa/webview-paint.png) |
| `drag_from_title_text`<br>按住标题文字本身拖动，窗口位置真的改变（data-tauri-drag-region="deep"） | OS 输入 | **NOT EXECUTED** | 窗口内容不可观测：等了 20 秒，主色 #f8f8f8 占 100.0000%，显著颜色区 1 个。窗口本身是真的（已映射、`_NET_WM_NAME` 正确、`_NET_WM_ICON` 齐备），但 X `GetImage` 读回来的是一整片单色。本机分不清两种原因——前端根本没渲染，还是 WebKit 把内容合成到了一块 X 读不到的 GL 表面（日志里有 `libEGL warning: DRI3 error: Could not get DRI3 device`，指向后者）。两种情况下点击都落在空处，因此「点了按钮之后窗口状态没变」证不了任何关于产品的事——记成 FAIL 会是一个假故障，故记未执行<br>**可执行条件**：一个 WebKitGTK 会把内容真的合成到窗口上的宿主机。本机（无 GPU 的容器 + Xvfb）实测不会：`WEBKIT_DISABLE_COMPOSITING_MODE=1`、`WEBKIT_DISABLE_DMABUF_RENDERER=1`、`LIBGL_ALWAYS_SOFTWARE=1` 三个单独与组合都试过，读回来始终是单色。一台有真实显示与可用 GL 栈的 Linux 桌面登录会话即可 | [`desktop-qa/webview-paint.png`](desktop-qa/webview-paint.png) |
| `taskbar_icon_correct`<br>窗口带 _NET_WM_ICON，任务栏能取到图标 | OS 输入 | **PASS** | `_NET_WM_ICON` 有 2306 个 32 位字（宽高 2 字 + 逐像素 ARGB），任务栏与 alt-tab 能取到图标 | [`desktop-qa/taskbar-icon.png`](desktop-qa/taskbar-icon.png) |
| `tray_icon_correct`<br>托盘图标存在且背景透明 | OS 输入 | **NOT EXECUTED** | 应用已通过 `TrayIconBuilder` 创建托盘图标，但本次 Xvfb + Openbox 会话没有StatusNotifier/AppIndicator 托盘宿主，harness 因而没有可观测的托盘项；图标资产透明度仍由 `xtask verify-icons` 逐字节守卫<br>**可执行条件**：带 StatusNotifier/AppIndicator 托盘宿主的交互式 Linux 桌面，并由 harness 从托盘协议侧观测图标 | — |
| `ime_prefilled_search_box`<br>中文输入法往一个**已有内容**的检索框里输入：不冻结且字符落入框内（tauri#15436） | WebDriver | **NOT EXECUTED** | WebDriver 会话未能建立，DOM 层事实无法观测；刻意不用 mock 或 stub 顶替。握手探测结果：`POST /session` 未能返回会话：POST http://127.0.0.1:4444/session 失败。本机实测：应用进程真的起来了、真实窗口映射成功、WebKit 的 inspector server 端口也在监听，但 `WebKitWebDriver` 从不向它建立连接，请求一直挂到超时。<br>**可执行条件**：`tauri-driver` 与 `WebKitWebDriver` 能为本次构建建立真实自动化会话；也可在支持 embedded WebDriver 的宿主机上改用 `@wdio/tauri-service` | — |
| `ime_prefilled_search_box_no_freeze`<br>承 tauri#15436：聚焦已有内容的检索框并输入后，界面仍然响应（OS 层可证的那一半） | OS 输入 | **NOT EXECUTED** | 窗口内容不可观测：等了 20 秒，主色 #f8f8f8 占 100.0000%，显著颜色区 1 个。窗口本身是真的（已映射、`_NET_WM_NAME` 正确、`_NET_WM_ICON` 齐备），但 X `GetImage` 读回来的是一整片单色。本机分不清两种原因——前端根本没渲染，还是 WebKit 把内容合成到了一块 X 读不到的 GL 表面（日志里有 `libEGL warning: DRI3 error: Could not get DRI3 device`，指向后者）。两种情况下点击都落在空处，因此「点了按钮之后窗口状态没变」证不了任何关于产品的事——记成 FAIL 会是一个假故障，故记未执行<br>**可执行条件**：一个 WebKitGTK 会把内容真的合成到窗口上的宿主机。本机（无 GPU 的容器 + Xvfb）实测不会：`WEBKIT_DISABLE_COMPOSITING_MODE=1`、`WEBKIT_DISABLE_DMABUF_RENDERER=1`、`LIBGL_ALWAYS_SOFTWARE=1` 三个单独与组合都试过，读回来始终是单色。一台有真实显示与可用 GL 栈的 Linux 桌面登录会话即可 | [`desktop-qa/webview-paint.png`](desktop-qa/webview-paint.png) |
| `two_char_search_returns_results`<br>两字检索（明月）返回结果行 | WebDriver | **NOT EXECUTED** | WebDriver 会话未能建立，DOM 层事实无法观测；刻意不用 mock 或 stub 顶替。握手探测结果：`POST /session` 未能返回会话：POST http://127.0.0.1:4444/session 失败。本机实测：应用进程真的起来了、真实窗口映射成功、WebKit 的 inspector server 端口也在监听，但 `WebKitWebDriver` 从不向它建立连接，请求一直挂到超时。<br>**可执行条件**：`tauri-driver` 与 `WebKitWebDriver` 能为本次构建建立真实自动化会话；也可在支持 embedded WebDriver 的宿主机上改用 `@wdio/tauri-service` | — |
| `corpus_first_run_materialization`<br>首启语料物化完成并显示进度 | WebDriver | **NOT EXECUTED** | WebDriver 会话未能建立，DOM 层事实无法观测；刻意不用 mock 或 stub 顶替。握手探测结果：`POST /session` 未能返回会话：POST http://127.0.0.1:4444/session 失败。本机实测：应用进程真的起来了、真实窗口映射成功、WebKit 的 inspector server 端口也在监听，但 `WebKitWebDriver` 从不向它建立连接，请求一直挂到超时。<br>**可执行条件**：`tauri-driver` 与 `WebKitWebDriver` 能为本次构建建立真实自动化会话；也可在支持 embedded WebDriver 的宿主机上改用 `@wdio/tauri-service` | — |
| `shipped_appreciation_without_key`<br>没有 API key 时随包赏析仍能渲染，且带「AI 赏析」标签与未审校说明 | WebDriver | **NOT EXECUTED** | WebDriver 会话未能建立，DOM 层事实无法观测；刻意不用 mock 或 stub 顶替。握手探测结果：`POST /session` 未能返回会话：POST http://127.0.0.1:4444/session 失败。本机实测：应用进程真的起来了、真实窗口映射成功、WebKit 的 inspector server 端口也在监听，但 `WebKitWebDriver` 从不向它建立连接，请求一直挂到超时。<br>**可执行条件**：`tauri-driver` 与 `WebKitWebDriver` 能为本次构建建立真实自动化会话；也可在支持 embedded WebDriver 的宿主机上改用 `@wdio/tauri-service` | — |
| `voice_round_succeeds_end_to_end`<br>语音背诵一轮端到端成功：采集 -> ASR -> 无偏置评分这条链跑通 | WebDriver | **NOT EXECUTED** | WebDriver 会话未能建立，且本次探测到的音频输入设备数为 0；DOM 与真实采集链均无法执行。握手探测结果：`POST /session` 未能返回会话：POST http://127.0.0.1:4444/session 失败。本机实测：应用进程真的起来了、真实窗口映射成功、WebKit 的 inspector server 端口也在监听，但 `WebKitWebDriver` 从不向它建立连接，请求一直挂到超时。<br>**可执行条件**：`tauri-driver` 与 `WebKitWebDriver` 能为本次构建建立真实自动化会话；也可在支持 embedded WebDriver 的宿主机上改用 `@wdio/tauri-service`；另需至少一个非 monitor 的可采集音频输入设备及可用语音模型 | — |
| `voice_degradation_states_reason`<br>失败路径单独验证：语音不可用时切到打字模式并显示具体原因 | WebDriver | **NOT EXECUTED** | WebDriver 会话未能建立，DOM 层事实无法观测；刻意不用 mock 或 stub 顶替。握手探测结果：`POST /session` 未能返回会话：POST http://127.0.0.1:4444/session 失败。本机实测：应用进程真的起来了、真实窗口映射成功、WebKit 的 inspector server 端口也在监听，但 `WebKitWebDriver` 从不向它建立连接，请求一直挂到超时。<br>**可执行条件**：`tauri-driver` 与 `WebKitWebDriver` 能为本次构建建立真实自动化会话；也可在支持 embedded WebDriver 的宿主机上改用 `@wdio/tauri-service` | — |
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
