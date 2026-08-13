/**
 * 语音跟读的传输形状与固定文案。
 *
 * # 每一个标识符都是从 Rust 源码抄来的
 *
 * 本项目已经因为凭记忆写标识符栽了**四次**（`.omo/notepads/yunjian/issues.md`），所以下面
 * 每个字符串取值、每个字段名都带出处（`文件:行`）。抄的对象只有一处：
 * `crates/yunjian-app/src/voice_ipc.rs`。那个模块是 `pub mod`，它的 `Serialize` 实现就是
 * 线上形状本身，没有第二层镜像可以偏移。
 *
 * # 界面不做任何评分，这一条写在类型里
 *
 * [`VoiceReport`] 的九个字段**没有一个是自由文本**——`relative_rhythm` 与 `coherence_label`
 * 分别是枚举串与固定标签。这不是巧合：`voice_ipc.rs` 里有一条
 * `report_payload_keys_are_frozen` 逐键钉住这个键集，任何人加一个能承载转写的字段都要先
 * 推翻那条断言。所以本文件把同一份键集再声明一遍是**安全的**：两边一起改才编译得过。
 *
 * # 偏置假设只能进诊断区
 *
 * [`AsrPartial.biased`] 是**唯一**一处会出现识别转写的地方，而它自带
 * `diagnostics_only: true` 与 [`AsrPartial.note`]。文言识别的字错率实测 77.01%
 * （`docs/reports/asr-cer.md`，且那是 TTS 合成音的乐观上界），因此按 2026-08-11 的裁决它
 * 既不代表用户说了什么，也不参与任何评分——界面拿它做实时前缀高亮，仅此而已。
 */

import type { Event } from "./operation";

/* ────────────────────────── 降级原因 ────────────────────────── */

/**
 * 十条降级原因的线上串。
 *
 * 逐字取自 `voice_ipc.rs` 的 `degrade_reason_key()`；顺序取自同文件的
 * `WIRE_DEGRADE_REASONS`，那个常量另有一条断言保证十条互不相同。
 *
 * **十条各有独立文案是产品要求，不是冗余。** 「麦克风不可用」这一句话在这十种情形下的
 * 下一步动作完全不同——去系统设置放开授权、插一个麦克风、关掉占用它的程序、升级系统、
 * 下模型、重新构建。合并任意两条就等于在一半情形下误导用户。
 */
export const DEGRADE_REASON_IDS = [
  "feature_disabled",
  "system_too_old",
  "permission_denied",
  "permission_restricted",
  "permission_undetermined",
  "no_input_device",
  "model_unavailable",
  "device_busy",
  "recognition_rejected",
  "capture_failed",
] as const;

export type DegradeReasonId = (typeof DEGRADE_REASON_IDS)[number];

/**
 * 每条原因的界面短标签。
 *
 * **只是标签，不是解释。** 完整的中文解释由 Rust 侧 `permission::explain()` 给出并随
 * [`TypedFallback.message`] 送来，界面**不自己编一句**：那个函数按平台给出「去哪个设置页」
 * （macOS 与 Windows 的路径不同），前端重写一遍必然漂移。
 */
export const DEGRADE_REASON_LABEL: Record<DegradeReasonId, string> = {
  feature_disabled: "本版本未编译语音",
  system_too_old: "系统版本过低",
  permission_denied: "麦克风授权被拒绝",
  permission_restricted: "麦克风被策略禁用",
  permission_undetermined: "尚未获得麦克风授权",
  no_input_device: "没有可用的麦克风",
  model_unavailable: "语音模型未就绪",
  device_busy: "麦克风被占用",
  recognition_rejected: "这一次录音未被识别接受",
  capture_failed: "麦克风打开失败",
};

/**
 * 降级到打字练习的落点。
 *
 * 字段名逐一对应 `voice_ipc.rs` 的 `TypedFallbackOut`。
 *
 * `completed_lines` **不清零**：一次设备故障不该让用户重头再来，所以界面要把它显示出来，
 * 让用户知道自己已经复诵到第几句。
 */
export interface TypedFallback {
  reason: DegradeReasonId;
  message: string;
  completed_lines: number;
}

/* ────────────────────────── 可用性 ────────────────────────── */

/**
 * 语音在本机可用不可用。
 *
 * 内部 tag 是 `kind`，取值 `voice` / `typed`——`voice_ipc.rs` 的 `VoiceAvailabilityOut`
 * 带 `#[serde(tag = "kind", rename_all = "snake_case")]`。
 *
 * **界面在渲染任何语音控件之前先问这一条。** 先画一个录音按钮再在点下去的时候报错，
 * 与「这条路本来就走不通，原因如下」是两种完全不同的体验。
 */
export type VoiceAvailability =
  | { kind: "voice"; coherence_label: string; note: string }
  | { kind: "typed"; reason: DegradeReasonId; message: string };

/* ────────────────────────── 示范与高亮 ────────────────────────── */

/**
 * 一个音步在示范音里的位置，毫秒。**karaoke 高亮的唯一驱动源。**
 *
 * 字段名逐一对应 `voice_ipc.rs` 的 `FootMarkOut`。
 *
 * # 这些时刻为什么可信
 *
 * 它们是**拼接的算术结果**，不是对齐出来的：朗读由逐音步分别合成、在 Rust 侧插静音拼成
 * （`prosody::splice`），所以每一段起止于第几个样本是个确定值。强制对齐上游明确不做
 * （sherpa-onnx #3536），且识别器只暴露 token 的 **start** 时间、没有 stop（#985）——
 * 换句话说，拼接不是「实现节奏的一种办法」，它同时是我们能拿到时间戳的**唯一**办法。
 */
export interface FootMark {
  line: number;
  index_in_line: number;
  text: string;
  start_ms: number;
  end_ms: number;
}

/** 示范命令的载荷。字段名对应 `voice_ipc.rs` 的 `VoiceDemonstrationOut`。 */
export interface VoiceDemonstration {
  /**
   * 自定义 URI 协议下的音频地址。
   *
   * **音频本体不经命令返回值。** 实测把 6.3 MB 的 PCM 当返回值序列化会在线上膨胀到
   * 22.5 MB（`learnings.md` 的 todo 64 一节），而一段二十秒的朗读正是这个量级。
   * 界面把它交给 `<audio src>`。
   */
  audio_url: string;
  sample_rate: number;
  duration_ms: number;
  /** 逐音步时间戳。**长度恒等于音步数。** */
  marks: FootMark[];
}

/**
 * 按播放进度挑出当前应高亮的那个音步下标。
 *
 * 返回 `-1` 表示此刻不在任何音步内（落在音步之间的静音里）。**用半开区间**
 * `[start_ms, end_ms)`：相邻音步之间有静音间隔，所以不存在边界重叠，但用闭区间会让
 * 恰好落在 `end_ms` 的那一帧同时命中两个音步。
 *
 * 这是本文件唯一一处算术，而它算的是**下标**不是分数：从时刻找区间是查找，不是评分。
 */
export function markAt(marks: readonly FootMark[], atMs: number): number {
  return marks.findIndex((mark) => atMs >= mark.start_ms && atMs < mark.end_ms);
}

/* ────────────────────────── 会话事件 ────────────────────────── */

/** 会话阶段。字段名与取值对应 `voice_ipc.rs` 的 `VoiceStageOut`。 */
export type VoiceStage =
  | { stage: "idle" }
  | { stage: "demonstrating"; line: number }
  | { stage: "listening"; line: number }
  | { stage: "awaiting_grade" }
  | { stage: "degraded"; fallback: TypedFallback };

/**
 * 五个阶段的界面说法。
 *
 * 「示范」与「复诵」是**两个互斥阶段**而不是两个开关，因为播放与录音绝不重叠——重叠会让
 * 识别器完美听见扬声器里自己的示范音，从而得到一个虚假的满覆盖
 * （`session.rs` 的模块文档第 2 条）。文案照这个事实写。
 */
export const VOICE_STAGE_LABEL: Record<VoiceStage["stage"], string> = {
  idle: "尚未开始",
  demonstrating: "正在示范朗读（此时不录音）",
  listening: "正在录你的复诵（此时不播放）",
  awaiting_grade: "已复诵完，等你自己选一档等级",
  degraded: "已切换到打字练习",
};

/** 会话进度快照。对应 `voice_ipc.rs` 的 `VoiceProgressOut`。 */
export interface VoiceProgress {
  stage: VoiceStage;
  completed_lines: number;
  total_lines: number;
}

/** 卡顿提示的两种触发原因。取自 `voice_ipc.rs` 的 `prompt_reason_key()`。 */
export const PROMPT_REASON_IDS = ["no_speech_yet", "trailing_silence"] as const;

export type PromptReasonId = (typeof PROMPT_REASON_IDS)[number];

/**
 * 两种原因各一句说法。
 *
 * 触发条件**只看能量门控测到的静音时长**，不看识别假设（`recognize.rs` 的
 * `StuckDetector` 刻意不接受任何假设文本）：77% 字错率下 partial 的匹配前缀是噪声，
 * 用它推进位置会把提示指到错误的字上。
 */
export const PROMPT_REASON_LABEL: Record<PromptReasonId, string> = {
  no_speech_yet: "还没听到你开口",
  trailing_silence: "这里停了一会儿",
};

/** 相对示范音的快慢。取自 `voice_ipc.rs` 的 `relative_rhythm_key()`。 */
export const RELATIVE_RHYTHM_IDS = ["slower", "similar", "faster"] as const;

export type RelativeRhythmId = (typeof RELATIVE_RHYTHM_IDS)[number];

/** 三档节奏的中文说法。 */
export const RELATIVE_RHYTHM_LABEL: Record<RelativeRhythmId, string> = {
  slower: "比示范慢",
  similar: "与示范相近",
  faster: "比示范快",
};

/**
 * 一次流式部分假设。**仅供诊断。**
 *
 * 字段名逐一对应 `voice_ipc.rs` 的 `AsrPartialOut`。`diagnostics_only` 恒为 `true`——
 * 它是字段而不是约定，于是「把它当用户反馈渲染」这件事在载荷上就看得见。
 */
export interface AsrPartial {
  at_ms: number;
  unbiased: string | null;
  biased: string | null;
  diagnostics_only: true;
  note: string;
}

/**
 * 会话流上不可丢弃的增量结果。
 *
 * 内部 tag 是 `item`，取值 snake_case——`voice_ipc.rs` 的 `VoiceItemOut` 带
 * `#[serde(tag = "item", rename_all = "snake_case")]`。**注意 newtype 变体是扁平的**：
 * `report` 那一项的字段直接和 `item` 平级，不在一个嵌套对象里。
 */
export type VoiceItem =
  | { item: "demonstrated"; line: number; duration_ms: number; marks: FootMark[] }
  | ({ item: "asr_partial" } & AsrPartial)
  | {
      item: "prompt";
      next_chars: string;
      from_index: number;
      at_ms: number;
      reason: PromptReasonId;
    }
  | {
      item: "line_observed";
      line: number;
      spoke: boolean;
      long_pause_count: number;
      total_ms: number;
      onsets_ms: number[];
    }
  | ({ item: "report" } & VoiceReport)
  | ({ item: "fallback" } & TypedFallback);

/**
 * 一次跟读会话的产出。
 *
 * 字段名逐一对应 `voice_ipc.rs` 的 `VoiceReportOut`，而那个结构体的**键集本身是一道门禁**
 * （`report_payload_keys_are_frozen`）：九个键全部是数与布尔，一个自由文本字段都没有，
 * 于是「把转写当分数显示」在传输层就无从表达。
 *
 * `coherence` **不是读音评分**。它由三项信号相乘得出——起始间隔的方差、长停顿计数、
 * 总时长与示范音期望时长之比——全部来自能量门控，与识别转写无关（`session.rs` 的
 * `RhythmInputs` 只有这三个私有字段，第四种信号无处可进）。
 */
export interface VoiceReport {
  spoke: boolean;
  long_pause_count: number;
  relative_rhythm: RelativeRhythmId;
  coherence: number;
  /** 这个指标唯一允许的名字，由 Rust 侧的 `COHERENCE_LABEL` 给出，界面不另起译名。 */
  coherence_label: string;
  gap_variance_ms2: number;
  duration_ratio: number;
  lines_attempted: number;
  prompt_count: number;
}

/** 会话事件流的元素类型。 */
export type VoiceSessionEvent = Event<VoiceProgress, VoiceItem>;

/** 一次会话跑完之后的落点。对应 `voice_ipc.rs` 的 `VoiceOutcomeOut`。 */
export type VoiceOutcome =
  | { kind: "reported"; operation_id: string; report: VoiceReport }
  | { kind: "degraded"; operation_id: string; fallback: TypedFallback };

/* ────────────────────────── 模型下载 ────────────────────────── */

/** 模型下载的四段进度。对应 `voice_ipc.rs` 的 `ModelFetchOut`。 */
export type ModelFetch =
  | { stage: "downloading"; bytes_done: number; bytes_total: number }
  | { stage: "verifying"; bytes: number }
  | { stage: "verified" }
  | { stage: "unpacking" };

/** 四段进度的中文说法。 */
export const MODEL_FETCH_LABEL: Record<ModelFetch["stage"], string> = {
  downloading: "正在下载",
  verifying: "正在核对摘要",
  verified: "摘要已核对",
  unpacking: "正在解包",
};

/** 模型下载的事件流元素类型。 */
export type ModelFetchEvent = Event<ModelFetch, string>;

/** 取模型完成后的落点。对应 `voice_ipc.rs` 的 `VoiceModelOutcomeOut`。 */
export type VoiceModelOutcome =
  | { kind: "ready"; operation_id: string; name: string; directory: string }
  | { kind: "unavailable"; operation_id: string; fallback: TypedFallback };

/* ────────────────────────── 固定文案 ────────────────────────── */

/**
 * 诊断转写那一段必须同屏出现的说明。
 *
 * **不在这里写死字面量**：它由 Rust 侧的 `ASR_PARTIAL_NOTE` 随每一条 `asr_partial`
 * 送来（[`AsrPartial.note`]），界面原样显示。前端另存一份副本就会在改文案时漂移，
 * 而漂移的那一份恰好是用户看到的那一份。这个常量只是一句说明该去哪里找它。
 */
export const ASR_PARTIAL_NOTE_SOURCE = "由 asr_partial 事件的 note 字段携带";

/**
 * 会话产出那一段必须同屏出现的说明。同样由 Rust 侧送来，见
 * [`VoiceAvailability`] 的 `note`。
 */
export const REPORT_NOTE_SOURCE = "由 voice_availability 的 note 字段携带";

/**
 * 语音路径不做机器评分，这一句写在结果区。
 *
 * 依据是 2026-08-11 的裁决：文言 ASR 实测 CER 77.01%（TTS 合成音的乐观上界），因此 v1
 * 语音契约是「逐句示范 + 复诵 + 只报是否开口/停顿/相对节奏 + 用户自评」。这一句与
 * `contracts/recite.ts` 的 `VOICE_GRADE_IS_MANUAL_NOTE` 说的是同一件事，措辞刻意保持一致。
 */
export const VOICE_NO_MACHINE_SCORE_NOTE =
  "语音路径不做机器评分：文言识别的字错率实测过高，任何自动完整度都是噪声。" +
  "下面各项全部来自能量门控测到的时序，与识别转写无关。";

/** 「示范与录音绝不重叠」这件事要说给用户听。 */
export const NO_OVERLAP_NOTE =
  "示范与录音不会同时进行：边播边录会让识别器听见扬声器里的示范音，得到一个虚假的满覆盖。";
