# 语音模型权重的许可署名

本目录是**分发用**的署名副本：每个在 [`../models.toml`](../models.toml) 里登记的模型，
这里都有一份它的许可原文。云笺按需下载权重（安装包里不含任何权重），下载下来的每一份
都受这里对应的那份许可约束。

## 与 `models/licenses/` 的关系

`models/licenses/` 是**证据**：由 `cargo run -p xtask -- verify-models` 按 `license_sha256`
逐字节校验，用来证明「清单里写的 SPDX 与上游锁定 revision 的实际内容一致」。

本目录是**署名**：给分发件用。两者内容逐字节相同，由
`cargo test -p yunjian-voice models` 的 `licenses_directory_holds_a_file_for_every_manifest_entry`
断言钉住——不一致就意味着「分发时给出的许可原文」与「经过校验的那一份」是两份东西。

两处都不许手工编辑，也不许被格式化工具改写（见 `.gitattributes` 与 `.oxfmtignore`）。

## 文件名与扩展名

文件名是发布包名。扩展名跟着 `license_evidence` 走：

| 扩展名     | 证据形态                               | 含义                                                     |
| ---------- | -------------------------------------- | -------------------------------------------------------- |
| `.LICENSE` | `package_license` / `upstream_license` | 许可全文                                                 |
| `.CARD.md` | `model_card`                           | HuggingFace 模型卡，其 front-matter 的 `license:` 即声明 |

## 当前清单

| 文件                                                                 | 许可       | 原始权重                                    |
| -------------------------------------------------------------------- | ---------- | ------------------------------------------- |
| `sherpa-onnx-whisper-tiny.LICENSE`                                   | MIT        | OpenAI Whisper tiny                         |
| `sherpa-onnx-whisper-base.LICENSE`                                   | MIT        | OpenAI Whisper base                         |
| `sherpa-onnx-whisper-small.LICENSE`                                  | MIT        | OpenAI Whisper small                        |
| `sherpa-onnx-streaming-zipformer-bilingual-zh-en-2023-02-20.CARD.md` | Apache-2.0 | pfluo/k2fsa-zipformer-chinese-english-mixed |
| `vits-melo-tts-zh_en.LICENSE`                                        | MIT        | MyShell.ai MeloTTS                          |
| `kokoro-multi-lang-v1_0.LICENSE`                                     | Apache-2.0 | hexgrad/Kokoro-82M                          |
| `kitten-nano-en-v0_2-fp16.LICENSE`                                   | Apache-2.0 | KittenML/kitten-tts-nano-0.2                |

只收 MIT 与 Apache-2.0，没有例外。被拒的模型连同理由在
[`../models/DENYLIST.md`](../models/DENYLIST.md)，运行时门禁（`yunjian-voice` 的
`models` 模块）会读同一份名单，命中即拒绝加载。

## 夹带产物另需注意

`kokoro-multi-lang-v1_0` 与 `kitten-nano-en-v0_2-fp16` 的发布包内**夹带 GPL-3.0 的
espeak-ng 发音词典数据**。那不在本目录覆盖范围内，它的分发影响记在 `models.toml` 的
`[[model.bundled]]` 与 [`../docs/VOICE-BUILD.zh.md`](../docs/VOICE-BUILD.zh.md)：
**开启语音的分发件整体须按 GPL-3.0 条款提供。**
