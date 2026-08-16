# 云笺 · 许可与署名（随分发件）

本文件与同目录的两份许可原文一起随**每一个云笺命令行发布归档**分发。
它回答一个问题：**你手上这份产物按什么条款提供，源码去哪里拿。**

- 项目主页与完整源码：<https://github.com/sunerpy/yunjian>
- 逐条第三方清单（模型权重、语料、Rust 与前端依赖）：仓库内 `LICENSES.md`

## 你手上这份归档：GPL-3.0

命令行发布归档由 `--features voice,mcp` 构建，**带离线语音**。

| 文件                  | 内容                                                                    |
| --------------------- | ----------------------------------------------------------------------- |
| `LICENSE-MIT.txt`     | 云笺自身代码的许可（MIT，`Copyright (c) 2026 sunerpy`）                 |
| `LICENSE-GPL-3.0.txt` | GNU 通用公共许可证第 3 版，`espeak-ng` 的许可，因而也是本归档整体的许可 |

**这份归档整体须按 GPL-3.0 条款提供。** 云笺自身的代码仍然是 MIT
（MIT 单向兼容 GPL-3.0，**这不是许可冲突**），但产物里静态包含了一份 GPL-3.0 的
`espeak-ng`，于是**结合作品整体**落在 GPL-3.0 上。

### espeak-ng 署名与依据

| 项目     | 值                                                                                                                                     |
| -------- | -------------------------------------------------------------------------------------------------------------------------------------- |
| 组件     | `espeak-ng`（fork：`csukuangfj/espeak-ng`）                                                                                            |
| 许可     | GNU GPL Version 3                                                                                                                      |
| 上游     | <https://github.com/csukuangfj/espeak-ng>（fork 自 <https://github.com/espeak-ng/espeak-ng>）                                          |
| 进入路径 | k2-fsa/sherpa-onnx 的预编译产物在 `SHERPA_ONNX_ENABLE_TTS=ON` 下经 `cmake/espeak-ng-for-piper.cmake` 把它编入                          |
| 载体     | 本归档内的 `libsherpa-onnx-c-api.{so,dylib,dll}`（Linux musl 归档为静态链入可执行文件）                                                |
| 许可原文 | 本目录 `LICENSE-GPL-3.0.txt`，SHA-256 `8ceb4b9ee5adedde47b31e975c1d90c73ad27b6b165a1dcd80c7c545eb65b903`（取自上游 fork 的 `COPYING`） |

**这不是推测，是可复核的事实。** 在共享库上数导出符号即可：

```
nm -D --defined-only libsherpa-onnx-c-api.so | grep -c 'espeak_'   # → 50
```

**大小写敏感**——加 `-i` 会因 `OfflineSpeakerDiarization` 里的 `neSpeak` 多算 14 个。

### 取得对应源码

- 云笺自身：<https://github.com/sunerpy/yunjian>（完整开源，含构建脚本）
- `espeak-ng`：<https://github.com/csukuangfj/espeak-ng>
- `sherpa-onnx`（Apache-2.0，内含上一行）：<https://github.com/k2-fsa/sherpa-onnx>
- `onnxruntime`（MIT）：<https://github.com/microsoft/onnxruntime>

## 桌面安装包不在此列：纯 MIT

桌面安装包（`.deb` / `.AppImage` / `.dmg` / `.msi` / `.exe`）**不编译 `voice` 特性**，
不链接任何 onnxruntime，因此整体按 **MIT** 提供。判据是可实测的：

```
ldd <桌面可执行文件> | grep -i onnx    # → 空
```

桌面安装包内随带的许可是仓库根 `LICENSE`（MIT）。

## 语音模型权重不在归档内

安装包与归档里**不含任何模型权重**。权重按需下载，只接受 MIT 与 Apache-2.0，
每一份的许可原文在仓库 `licenses/` 目录。其中 `kokoro-multi-lang-v1_0` 与
`kitten-nano-en-v0_2-fp16` 的发布包**夹带 GPL-3.0 的 espeak-ng 发音词典数据**，
详见仓库 `LICENSES.md` 与 `docs/VOICE-BUILD.zh.md`。
