# 文言语音识别 CER 实测

> 由 `cargo run -p xtask -- cer-spike` 生成，机器可读版本见 [`asr-cer.json`](asr-cer.json)。

## 裁决

| 项 | 值 |
| --- | --- |
| `scoring_mode` | **`completeness_only`** |
| 是否实测 | 是 |
| 总 CER | 77.01% |
| 阈值 | 10% |
| 实测句数 | 1800 / 1800 |

`scoring_mode` 的取值域**只有** `advisory_accuracy` 与 `completeness_only` 两个，**永远不会是 `full`**。todo 48、51、56、57 读的就是这个字段。

- `advisory_accuracy`：完整度与流畅度作为分数呈现；逐字准确率标注为「ASR 估计值 / 仅供参考」，不计入分数。
- `completeness_only`：连 advisory 的字准都不展示，只给完整度与流畅度。
- 打字路径是确定性比对，字准照常作为分数，不受本裁决影响。

## 这个数字是乐观上界

本表的 CER **由 TTS 合成音加信道增强测得，不是真人朗读**，因此它是真实说话人 CER 的**乐观上界**：合成音只有单一说话人、没有口音、没有吞音、没有真实房间的混响与远场衰减，韵律也比人朗读规整。增强（8 kHz 窄带往返、20 dB 与 10 dB 粉噪、±10% 变速不变调）逼近的是**信道与语速**，逼近不了说话人差异。

之所以只能这样测：唯一公开的中文朗诵语料 MCGA 是 CC BY-NC-SA-4.0 且只放了 test split，NC 条款排除本项目（见 `corpus/DENYLIST.md`），而本 todo 刻意不以真人录音为门禁。

乐观上界的用途是单向的——它足以**证伪**（上界都超过阈值，真人只会更差），不足以**证成**（上界达标不代表真人达标）。所以即便通过，语音路径上的逐字准确率也永远只是 advisory，不会升为分数。

## 方法

| 项 | 值 |
| --- | --- |
| 参考文本 | 50 首，8 个体裁 |
| 文本来源 | `chinese-poetry/chinese-poetry` @ `b8594f81a897` |
| 因超 30 秒窗口而截断 | 4 首 |
| 是否使用真人录音 | 否（刻意如此，见下） |
| TTS 音色 | `vits-melo-tts-zh_en`、`kokoro-multi-lang-v1_0` |
| ASR 模型 | `sherpa-onnx-whisper-tiny`、`sherpa-onnx-whisper-base`、`sherpa-onnx-whisper-small` |
| 增强条件 | `clean`、`narrowband-8k`、`pink-20db`、`pink-10db`、`slow-110`、`fast-90` |

## 逐体裁 CER

| 体裁 | 首数 | 句数 | CER |
| --- | --: | --: | --: |
| 七言古詩 | 3 | 108 | 75.07% |
| 七言律詩 | 8 | 288 | 78.66% |
| 七言絕句 | 10 | 360 | 77.97% |
| 五言古詩 | 5 | 180 | 80.14% |
| 五言律詩 | 8 | 288 | 74.98% |
| 五言絕句 | 8 | 288 | 74.58% |
| 樂府 | 4 | 144 | 77.29% |
| 词 | 4 | 144 | 77.55% |

## 逐增强条件 CER

| 条件 | 句数 | CER |
| --- | --: | --: |
| `clean` | 300 | 75.98% |
| `fast-90` | 300 | 79.06% |
| `narrowband-8k` | 300 | 77.01% |
| `pink-10db` | 300 | 78.99% |
| `pink-20db` | 300 | 76.63% |
| `slow-110` | 300 | 74.42% |

## 逐模型组合 CER

| ASR | TTS | 句数 | CER |
| --- | --- | --: | --: |
| `sherpa-onnx-whisper-tiny` | `kokoro-multi-lang-v1_0` | 300 | 78.83% |
| `sherpa-onnx-whisper-tiny` | `vits-melo-tts-zh_en` | 300 | 77.29% |
| `sherpa-onnx-whisper-base` | `kokoro-multi-lang-v1_0` | 300 | 79.45% |
| `sherpa-onnx-whisper-base` | `vits-melo-tts-zh_en` | 300 | 75.40% |
| `sherpa-onnx-whisper-small` | `kokoro-multi-lang-v1_0` | 300 | 79.88% |
| `sherpa-onnx-whisper-small` | `vits-melo-tts-zh_en` | 300 | 71.25% |

## 这个数字说明了什么

CER 在 77% 量级，是阈值的七倍以上。这不是「调参能救回来」的差距，而是**任务与模型的不匹配**，有三条独立证据：

1. **换更大的模型不管用。** tiny / base / small 三个尺寸的 CER 几乎相同（见逐模型表），
   而正常的「模型太小」应当表现为随尺寸单调下降。它没有下降。
2. **加噪也几乎不管用。** clean 与 10 dB 粉噪的差距只有几个百分点（见逐条件表）。
   一个正常工作的识别器对 10 dB 噪声应当明显更差。它本来就已经在下限附近。
3. **错误形态是「音近字不同」，不是「乱码」。** 逐句转写里典型如
   `寥落古行宫` → `樂國行功`、`床前明月光` → `长前名月光`：声母韵母大致对得上，
   字选错了。也就是说声学前端在工作，**语言模型在把音节往现代汉语常用词上拽**——
   而文言恰恰是低频词、单音节词与倒装的密集区。

第三条已用**官方 CLI 在同一段音频上交叉验证**过：`sherpa-onnx-offline` 直接跑出的结果同样是 `长前名月光 地上`。因此这不是本项目绑定层的缺陷，而是 Whisper 在文言上的真实表现。

**推论**：即便将来换用许可可用的更强模型，也不应假定字准会跨过阈值。语音路径的
设计必须以「字准不可用」为前提，而不是把它当成一个待优化的指标。

## 复现

```bash
# 1) 核实模型许可并生成 models.lock.json
cargo run -p xtask -- verify-models
# 2) 按 models.toml 下载并解包权重到 models/cache/
#    （权重不入库，`.gitignore` 已排除 /models/cache/）
# 3) 重建参考文本（需网络，从锁定 revision 取公有领域原文）
cargo run -p xtask -- cer-spike --refresh-fixtures
# 4) 实测。需 --features voice，它会拉入 GPL-3.0 的 espeak-ng 原生依赖
cargo run -p xtask --features voice -- cer-spike
```

另有 [`asr-cer-human.md`](asr-cer-human.md) 供将来自愿贡献的真人录音填充。它是**可选**的，不是本结论的门禁。

