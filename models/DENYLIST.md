# 拒绝的语音模型权重

`cargo run -p xtask -- verify-models` 会解析本文件。**只有** `## 拒绝清单` 一节里
形如 ``- `标识符` —— 理由`` 的列表项会被当作条目；标识符对 `models.toml` 里每个
`[[model]]` 的 `name` 与 `url` 做大小写无关的子串匹配，命中即失败。

`verify-models` 同时断言本文件包含一组**必须存在**的标识符（见
`xtask/src/verify_models.rs` 的 `REQUIRED_DENYLIST`）。删掉其中任何一条以便偷偷
放行某个模型，命令会直接失败并指名那条被删的标识符。

判定粒度是**单个发布包**，不是模型族。一个上游 repo 的许可声明只覆盖它自己的产物，
覆盖不了它转换或再分发的东西——这与 `corpus/DENYLIST.md` 用在语料上的是同一条规则。

## 拒绝清单

- `matcha-icefall-zh-baker` —— 上游明写训练数据集「is for `non-commercial` use only」。非商用条款与本项目的分发形态不兼容，与 `MCGA` 被拒的理由同类。
- `vits-zh-hf-` —— 整个 `vits-zh-hf-*` 系列（bronya / echo / keqing / eula / theresa / zenyatta / doom / abyssinvoker / fanchen-* 等 11 个包）在上游既无 LICENSE 文件、模型卡也无 `license:` 声明。声音本身多取自游戏与动画角色，来源授权更不可能干净。许可未核实即不可用。
- `aishell3` —— `vits-zh-aishell3` 与 `vits-icefall-zh-aishell3` 均无许可声明。AISHELL-3 数据集自身的授权条款也未随发布包传递，链条断在第一环。
- `edge-tts` —— 不是可离线分发的权重，而是对未公开的 Bing 端点的调用。其维护者原话「I'd still never risk it for commercial use」，微软方面表示商用「could be a violation of our terms of service」。既无权重许可可核，也不满足离线可用。
- `MCGA` —— 朗诵音频语料，CC BY-NC-SA-4.0（NC 条款排除本项目），且只发布了 test split。CER 的参考音频一律走自有 TTS 合成，不用它。
- `SenseVoice` —— 权重按 _FunASR Model Open Source License Agreement v1.1_ 发布（`FunAudioLLM/SenseVoiceSmall` 模型卡：`license: other`，`license_link` 指向 `modelscope/FunASR/blob/main/MODEL_LICENSE`）。该协议允许商用且只要求署名，但它**既不是 MIT 也不是 Apache-2.0**，不在本项目的允许列表内。研究阶段「ASR 许可整体健康」的结论在这里被推翻。
- `sense-voice` —— 同上，匹配 sherpa-onnx 侧的包名形态（`sherpa-onnx-sense-voice-*`，含全部 rk35xx 变体）。
- `paraformer` —— 同属 FunASR 系（`sherpa-onnx-paraformer-zh-*`），许可与 SenseVoice 相同，故同样拒绝。
- `streaming-zipformer-small-ctc-zh` —— 研究阶段原定的流式模型（`sherpa-onnx-streaming-zipformer-small-ctc-zh-int8-2025-04-01`，20.3 MB）。上游 `csukuangfj/…` 无 LICENSE 文件、模型卡无 `license:` 声明；其 PyTorch 来源 `csukuangfj/icefall-streaming-zipformer-small-ctc-zh-2025-04-01` 同样两者皆无。研究记录里的 _Apache-2.0 (UNVERIFIED)_ 经核实为**无声明**，不是 Apache-2.0。
- `streaming-zipformer-zh-2025-06-30` —— 同批 2025-06-30 的四个流式中文包（fp16 / int8 / xlarge-*）均无许可声明。
- `zipformer-zh-en-2023-11-22` —— 离线中英 zipformer。转换包所在的 HuggingFace repo 已不可匿名访问（API 返回 "Invalid username or password"），其 icefall 来源 `zrjin/icefall-asr-zipformer-multi-zh-en-2023-11-22` 无 LICENSE 文件也无模型卡声明。无法核实。
- `vosk-model` —— Vosk 中文模型整体 Apache-2.0，许可没问题，但实测 CER 23.54 太差，且其 Rust 绑定自 2024-10 起未更新。按能力而非许可排除，列此以防再被当作「许可干净就能用」而重新引入。

## 被拒绝后的影响

**离线识别**：可用的只剩 Whisper 系。这不是次优选择，而是唯一选择——`models.toml`
里三个尺寸的 CER 实测结论见 `docs/reports/asr-cer.md`。

**流式识别**：只有 `sherpa-onnx-streaming-zipformer-bilingual-zh-en-2023-02-20`
的许可链完整（上下游模型卡均声明 apache-2.0），已收进 `models.toml`。但
`sherpa-rs` 0.6.8 没有流式识别器封装，接通它要经 `sherpa_rs_sys` 直调 online API，
属 todo 54。因此实时逐字反馈这条路许可上是通的，实现上还没通。

**中文合成**：`vits-melo-tts-zh_en`（MIT）与 `kokoro-multi-lang-v1_0`（Apache-2.0）
两把音色都干净，正好满足 CER 实测「两种不同授权音色」的要求。其中只有 melo
不夹带 GPL-3.0 的 espeak-ng 数据。
