# 桌面端真机验收 · 2026-08-17

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
| 提交 | `a9ca77e44161da3c0506d1befef7850e1c72da57` |
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

声明 20 条 · PASS 4 · FAIL 2 · NOT EXECUTED 14

## WebDriver 握手探测

尝试：是 · 成功：是

会话建立成功，DOM 层断言由真实 WebDriver 执行。

## 逐条断言

| 断言 | 通道 | 裁决 | 依据 | 截图 |
| --- | --- | --- | --- | --- |
| `artifact_present`<br>构建产物存在且可执行，动态库依赖全部可解析 | 进程 | **PASS** | /config/work/yunjian-pregen/target/debug/yunjian-desktop 存在，ldd 未报缺失动态库 | — |
| `installer_runs`<br>安装包（.deb / NSIS / .dmg）能安装并从安装后的路径启动 | 进程 | **NOT EXECUTED** | /config/work/yunjian-pregen/target/debug/bundle 下没有安装包产物；本次验收跑的是未打包的构建产物，因此「安装后从安装路径启动」这条链没有被执行<br>**可执行条件**：先 `cargo tauri build`（Linux 上还需 `dpkg-deb` 与 `librsvg2-dev`），再由 harness 安装到一个临时 root 并从该路径启动。NSIS 静默安装时**只等安装器进程本身**，不要等整棵进程树——完成页会把应用作为子进程拉起来，等进程树会永远等不完。 | — |
| `app_launches`<br>应用在交互式桌面会话里启动并映射出一个顶层窗口 | OS 输入 | **FAIL** | 启动后 30 秒内没有映射出顶层窗口 | — |
| `custom_titlebar_rendered`<br>窗口没有窗口管理器绘制的边框（decorations: false 生效），标题栏由应用自绘 | OS 输入 | **NOT EXECUTED** | harness 在到达这条之前结束<br>**可执行条件**：重跑本子命令并检查上面失败的那一步 | — |
| `control_minimize_works`<br>点自绘标题栏的最小化按钮，窗口真的最小化 | OS 输入 | **NOT EXECUTED** | harness 在到达这条之前结束<br>**可执行条件**：重跑本子命令并检查上面失败的那一步 | — |
| `control_maximize_works`<br>点自绘标题栏的最大化按钮，窗口真的最大化 | OS 输入 | **NOT EXECUTED** | harness 在到达这条之前结束<br>**可执行条件**：重跑本子命令并检查上面失败的那一步 | — |
| `control_restore_works`<br>最大化后再点同一个按钮，窗口真的还原 | OS 输入 | **NOT EXECUTED** | harness 在到达这条之前结束<br>**可执行条件**：重跑本子命令并检查上面失败的那一步 | — |
| `control_close_works`<br>点自绘标题栏的关闭按钮，主窗口隐藏到托盘且进程继续运行 | OS 输入 | **NOT EXECUTED** | harness 在到达这条之前结束<br>**可执行条件**：重跑本子命令并检查上面失败的那一步 | — |
| `double_click_maximizes_exactly_once`<br>双击标题栏恰好最大化一次（自己再挂一个双击处理器会双切换回原样） | OS 输入 | **NOT EXECUTED** | harness 在到达这条之前结束<br>**可执行条件**：重跑本子命令并检查上面失败的那一步 | — |
| `drag_from_title_text`<br>按住标题文字本身拖动，窗口位置真的改变（data-tauri-drag-region="deep"） | OS 输入 | **NOT EXECUTED** | harness 在到达这条之前结束<br>**可执行条件**：重跑本子命令并检查上面失败的那一步 | — |
| `taskbar_icon_correct`<br>窗口带 _NET_WM_ICON，任务栏能取到图标 | OS 输入 | **NOT EXECUTED** | harness 在到达这条之前结束<br>**可执行条件**：重跑本子命令并检查上面失败的那一步 | — |
| `tray_icon_correct`<br>托盘图标存在且背景透明 | OS 输入 | **NOT EXECUTED** | harness 在到达这条之前结束<br>**可执行条件**：重跑本子命令并检查上面失败的那一步 | — |
| `ime_prefilled_search_box`<br>中文输入法往一个**已有内容**的检索框里输入：不冻结且字符落入框内（tauri#15436） | WebDriver | **PASS** | 检索框预填「明月」后再次聚焦并输入「千里」，`input.value` 为「明月千里」——字符全部落入框内，无吞字 | [`desktop-qa/ime-prefilled-dom.png`](desktop-qa/ime-prefilled-dom.png) |
| `ime_prefilled_search_box_no_freeze`<br>承 tauri#15436：聚焦已有内容的检索框并输入后，界面仍然响应（OS 层可证的那一半） | OS 输入 | **NOT EXECUTED** | harness 在到达这条之前结束<br>**可执行条件**：重跑本子命令并检查上面失败的那一步 | — |
| `two_char_search_returns_results`<br>两字检索（明月）返回结果行 | WebDriver | **PASS** | 检索「明月」后摘要为「估计命中 7291 条，本页显示 20 条」 | [`desktop-qa/two-char-search.png`](desktop-qa/two-char-search.png) |
| `corpus_first_run_materialization`<br>首启语料物化完成并显示进度 | WebDriver | **FAIL** | 起始状态 absent=true；点「下载语料库」之后等了 900 秒，既没有出现 `corpus-facts` 也没有出现 `corpus-error`，界面停在原地 | — |
| `shipped_appreciation_without_key`<br>没有 API key 时随包赏析仍能渲染，且带「AI 赏析」标签与未审校说明 | WebDriver | **PASS** | 未配置任何 API key 下打开「出塞」，`ai-source` 为「随包预生成」（命中随包表，非现场生成），标签为「AI 赏析」，并带未审校说明「AI 生成，未经人工审校」；正文首段为「<<未生成：本条不是模型输出，需开放权重模型推理>>」。**正文当前是随包数据集里的未生成标记**，即本条证明的是渲染与标注链路成立，不是生成能力成立 | [`desktop-qa/shipped-appreciation.png`](desktop-qa/shipped-appreciation.png) |
| `voice_round_succeeds_end_to_end`<br>语音背诵一轮端到端成功：采集 -> ASR -> 无偏置评分这条链跑通 | WebDriver | **NOT EXECUTED** | 被测产物未编译 `voice` 特性，语音在可用性探测这一步就降级为「本版本未编译语音」，采集与 ASR 都不存在，这一轮无从判定成功；降级本身由 `voice_degradation_states_reason` 单独判定。判词：「本版本未编译语音 —— 本版本未编译语音能力，已切换到打字练习。打字练习的评分与语音练习共用同一个内核，功能完整。」<br>**可执行条件**：用 `--features custom-protocol,voice` 构建被测产物（该特性同时是许可边界，见 `docs/VOICE-BUILD.zh.md`），并提供一个非 monitor 的可采集输入设备 | [`desktop-qa/voice-round.png`](desktop-qa/voice-round.png) |
| `voice_degradation_states_reason`<br>失败路径单独验证：语音不可用时切到打字模式并显示具体原因 | WebDriver | **NOT EXECUTED** | WebDriver 会话未能建立，DOM 层事实无法观测；刻意不用 mock 或 stub 顶替。握手探测结果：`POST /session` 未能返回会话：POST http://127.0.0.1:37401/session 失败；driver 仍在运行；它的 stderr 末尾是「2026-08-17T20:14:43.485265184+08:00  INFO yunjian_core::logger: 日志已初始化 level="info" json=false timezone="local" to_file=true dir=/config/.local/share/yunjian/logs / 2026-08-17T20:14:43.485436368+08:00  INFO yunjian_app: 云笺桌面端启动 app="yunjian" version="0.1.0" / 2026-08-17T20:14:43.502063124+08:00  INFO yunjian_ai::keystore: 凭据 store 不可用，继续走降级链 backend="secret_service" reason=Platform failure: zbus error: org.freedesktop.DBus.Error.ServiceUnknown: The name org.freedesktop.secrets was not provided by any .service files /  / thread 'main' (4176478) panicked at /config/.cargo/registry/src/mirrors.aliyun.com-0671735e7cc7f5e7/tao-0.35.3/src/platform_impl/linux/event_loop.rs:217:53: / Failed to initialize gtk backend!: BoolError { message: "Failed to initialize GTK", filename: "/config/.cargo/registry/src/mirrors.aliyun.com-0671735e7cc7f5e7/gtk-0.18.2/src/rt.rs", function: "gtk::rt::init", line: 141 } / note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace」<br>**可执行条件**：`tauri-driver` 与 `WebKitWebDriver` 能为本次构建建立真实自动化会话 | — |
| `app_exits_cleanly`<br>从托盘菜单选择「退出」后，应用以退出码 0 正常结束且不留孤儿进程 | 进程 | **NOT EXECUTED** | harness 在到达这条之前结束<br>**可执行条件**：重跑本子命令并检查上面失败的那一步 | — |

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
