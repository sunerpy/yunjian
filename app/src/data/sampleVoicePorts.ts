/**
 * Tauri 宿主下的语音端口，以及非 Tauri 宿主下的样例替身。
 *
 * # 替身**不做任何合成、采集或识别**
 *
 * 与 `data/sampleRecitePorts.ts` 同一条规矩，而在语音这里更要紧：替身若为了让 `vite dev`
 * 看起来「能用」而在 TypeScript 里造一份节奏连贯度，那么本 todo 想守住的边界当场就破了，
 * 而且是从测试替身这个最不显眼的地方破的。所以 [`createSampleVoicePorts`] 的产出是
 * **照公式手算一次后写死**的常量（算式见 [`SAMPLE_REPORT`] 的注释，可逐项复核），
 * 运行时一个乘号都没有。
 *
 * 替身**确实**做一件事：按真实的事件顺序把事件吐出来（示范 → 部分假设 → 提示 → 观察 →
 * 产出），因为界面的状态机要靠那个顺序驱动，而顺序是接线的一部分而不是评分的一部分。
 *
 * # 样例数据自报身份
 *
 * 正文只用毫无争议的公有领域名篇（《静夜思》）。音频地址写成一眼能看出取不到东西的形状：
 * 一张 dev 截图会被当成产品行为，而样例里的每个数字都是我写死的。
 */

import { invoke } from "@tauri-apps/api/core";
import type {
  FootMark,
  ModelFetchEvent,
  VoiceAvailability,
  VoiceDemonstration,
  VoiceModelOutcome,
  VoiceOutcome,
  VoiceReport,
  VoiceSessionEvent,
} from "../contracts/voice";
import { VOICE_NO_MACHINE_SCORE_NOTE } from "../contracts/voice";
import { progressChannel } from "./progressChannel";
import type { VoiceFetchModelRequest, VoicePort } from "./voicePorts";

/**
 * Rust 侧注册的命令名。
 *
 * 与 `data/tauriPorts.ts` 的 `IPC_COMMANDS` 同一条理由：命令名写错是**静默失败**，
 * 所以它必须是一个能被 grep 出来核对的清单。四条逐字取自
 * `crates/yunjian-app/src/ipc.rs` 的 `generate_handler!` 列表。
 */
export const VOICE_IPC_COMMANDS = {
  availability: "voice_availability",
  demonstrate: "voice_demonstrate",
  startSession: "voice_start_session",
  fetchModel: "voice_fetch_model",
  cancel: "cancel_operation",
} as const;

function inTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

/** Tauri 宿主下的语音端口；不在宿主里时返回 `null`。 */
export function createTauriVoicePort(): VoicePort | null {
  if (!inTauri()) {
    return null;
  }
  return {
    availability: () => invoke<VoiceAvailability>(VOICE_IPC_COMMANDS.availability),
    demonstrate: (poemId) =>
      invoke<VoiceDemonstration>(VOICE_IPC_COMMANDS.demonstrate, {
        request: { poem_id: poemId },
      }),
    // Channel 必须作为一个**命令参数**传过去，参数名要与 Rust 侧的形参名逐字一致
    // （`on_event`）。漏传它不是「没有进度」而是整条命令失败：Tauri 从请求里反序列化
    // `Channel`，缺参数就报 invalid args，而那条错误看起来像「会话开不起来」。
    // 建通道与订阅它由 `progressChannel` 一起做完，见那个模块。
    startSession: (request, onEvent) =>
      invoke<VoiceOutcome>(VOICE_IPC_COMMANDS.startSession, {
        request,
        onEvent: progressChannel<VoiceSessionEvent>(onEvent),
      }),
    fetchModel: (request, onEvent) =>
      invoke<VoiceModelOutcome>(VOICE_IPC_COMMANDS.fetchModel, {
        request,
        onEvent: progressChannel<ModelFetchEvent>(onEvent),
      }),
    cancel: (operationId) => invoke<boolean>(VOICE_IPC_COMMANDS.cancel, { operationId }),
  };
}

/* ────────────────────────── 以下全是写死的样例常量 ────────────────────────── */

/** 样例作品的四句。与 `data/samplePorts.ts` 说的是同一首诗。 */
const SAMPLE_LINES = ["床前明月光", "疑是地上霜", "举头望明月", "低头思故乡"] as const;

/**
 * 样例音频地址。
 *
 * 写成一个**取不到东西**的形状而不是伪造一段可播放音频：样例模式下没有合成器，装一个
 * 假音频只会让「示范音能不能放」这件事在 dev 里恒真，而那正是最需要在真宿主上验的一条。
 * `<audio>` 会安静地报错，界面据此显示「样例模式没有音频」。
 */
const SAMPLE_AUDIO_URL = "yunjian-audio://localhost/sample-not-available";

/** 样例合成的采样率。与 Rust 侧目标采样率同值，于是时刻换算口径一致。 */
const SAMPLE_SAMPLE_RATE = 16_000;

/**
 * 一行五言的两个音步及其时刻。
 *
 * 五言切二三（`prosody::foot_widths`，只看字数、不需要任何外部数据），音步之间的静音是
 * `Prosody::CLASSICAL` 的 `foot_pause_ms = 120`。所以一行的时刻是：
 * `[0, 500)` 二字、静音 120、`[620, 1370)` 三字——两字段按每字 250 ms 写死。
 * 这些数是**手算的常量**，不是算出来的：样例没有合成器，任何「算」都是在前端造一个
 * 第二份拼接实现。
 */
function sampleMarks(line: number, text: string): FootMark[] {
  const head = text.slice(0, 2);
  const tail = text.slice(2);
  const base = line * 1_770;
  return [
    {
      line,
      index_in_line: 0,
      text: head,
      start_ms: base,
      end_ms: base + 500,
    },
    {
      line,
      index_in_line: 1,
      text: tail,
      start_ms: base + 620,
      end_ms: base + 1_370,
    },
  ];
}

const SAMPLE_MARKS: FootMark[] = SAMPLE_LINES.flatMap((text, line) => sampleMarks(line, text));

/** 整段示范音的时长：四行 × 1770 ms 减去末尾那一段行间静音（行末不补静音）。 */
const SAMPLE_DURATION_MS = 4 * 1_770 - 400;

/**
 * 样例产出。**照公式手算一次后写死**，算式如下（`session.rs` 的 `coherence`
 * 与 `VoiceSessionConfig::default`）：
 *
 * 时序取每行三段语音活动、起始时刻 `[0, 400, 800]`、每行 1200 ms、长停顿 1 次。
 * 四行拼接（`SpeechTimeline::concat` 按各行开始时刻平移）后：
 *
 * - 起始间隔序列是 `400, 400, 400`（行内）与 `400`（跨行，1200→1600），全为 400，
 *   所以**方差为 0**；
 * - 长停顿 `1 × 4 = 4` 次；
 * - 总时长 `4800 ms`，示范音期望时长 `SAMPLE_DURATION_MS = 6680 ms`，时长比
 *   `4800 / 6680 ≈ 0.7186`。
 *
 * 三项按 `scale / (scale + x)` 归一后相乘：
 *
 * - 匀速度 `250000 / (250000 + 0) = 1`
 * - 连贯度 `2 / (2 + 4) = 0.333333…`
 * - 速度 `0.5 / (0.5 + |0.7186 - 1|) = 0.5 / 0.78144 ≈ 0.63983`
 *
 * 乘积 `≈ 0.21328`。相对节奏：时长比 `0.7186` 低于 `1 - 0.25 = 0.75`，判 `faster`。
 *
 * 写下算式是为了让这份常量可复核；**运行时一个乘号都没有。**
 */
const SAMPLE_REPORT: VoiceReport = {
  spoke: true,
  long_pause_count: 4,
  relative_rhythm: "faster",
  coherence: 0.21328,
  coherence_label: "节奏连贯度",
  gap_variance_ms2: 0,
  duration_ratio: 0.7186,
  lines_attempted: 4,
  prompt_count: 4,
};

/** 样例的部分假设。偏置那一路刻意写成一段**明显不对**的转写。 */
const SAMPLE_PARTIAL_UNBIASED = "床钱名月光";

/**
 * 偏置一路的样例转写。
 *
 * 刻意与原文一致：偏置解码把诗文本当 hotwords，因此它**倾向于吐出原文，哪怕用户跳过了
 * 整句**——这正是它不能进入评分的原因（`recognize.rs` 的模块文档）。样例照实呈现这一点，
 * 免得读者以为「偏置那一路更准所以更该用」。
 */
const SAMPLE_PARTIAL_BIASED = "床前明月光";

/**
 * 非 Tauri 宿主下的语音端口替身。
 *
 * 默认报**可用**：样例模式要让人能看见语音界面的完整形态。想看降级形态时传
 * `unavailable`，那也是 `__tests__` 用来验「五条失败各显示什么」的入口。
 */
export function createSampleVoicePort(
  unavailable?: Extract<VoiceAvailability, { kind: "typed" }>,
): VoicePort {
  const availability: VoiceAvailability =
    unavailable ??
    ({
      kind: "voice",
      coherence_label: "节奏连贯度",
      note: VOICE_NO_MACHINE_SCORE_NOTE,
    } as const);

  return {
    availability: () => Promise.resolve(availability),

    demonstrate: (_poemId) =>
      Promise.resolve<VoiceDemonstration>({
        audio_url: SAMPLE_AUDIO_URL,
        sample_rate: SAMPLE_SAMPLE_RATE,
        duration_ms: SAMPLE_DURATION_MS,
        marks: SAMPLE_MARKS,
      }),

    startSession: (request, onEvent) => {
      if (availability.kind === "typed") {
        const fallback = {
          reason: availability.reason,
          message: availability.message,
          completed_lines: 0,
        };
        onEvent({
          type: "progress",
          payload: {
            stage: { stage: "degraded", fallback },
            completed_lines: 0,
            total_lines: 0,
          },
        });
        onEvent({ type: "item", payload: { item: "fallback", ...fallback } });
        onEvent({ type: "done" });
        return Promise.resolve<VoiceOutcome>({
          kind: "degraded",
          operation_id: request.operation_id ?? "sample-op",
          fallback,
        });
      }

      SAMPLE_LINES.forEach((text, line) => {
        if (request.demonstrate) {
          onEvent({
            type: "progress",
            payload: {
              stage: { stage: "demonstrating", line },
              completed_lines: line,
              total_lines: SAMPLE_LINES.length,
            },
          });
          onEvent({
            type: "item",
            payload: {
              item: "demonstrated",
              line,
              duration_ms: 1_370,
              marks: sampleMarks(line, text),
            },
          });
        }
        onEvent({
          type: "progress",
          payload: {
            stage: { stage: "listening", line },
            completed_lines: line,
            total_lines: SAMPLE_LINES.length,
          },
        });
        onEvent({
          type: "item",
          payload: {
            item: "asr_partial",
            at_ms: 320,
            unbiased: SAMPLE_PARTIAL_UNBIASED,
            biased: SAMPLE_PARTIAL_BIASED,
            diagnostics_only: true,
            note: "以下转写仅供诊断：文言识别的字错率实测过高，它既不代表你说了什么，也不参与任何评分。",
          },
        });
        onEvent({
          type: "item",
          payload: {
            item: "prompt",
            next_chars: text.slice(0, 2),
            from_index: 0,
            at_ms: 900,
            reason: "trailing_silence",
          },
        });
        onEvent({
          type: "item",
          payload: {
            item: "line_observed",
            line,
            spoke: true,
            long_pause_count: 1,
            total_ms: 1_200,
            onsets_ms: [0, 400, 800],
          },
        });
      });

      onEvent({
        type: "progress",
        payload: {
          stage: { stage: "awaiting_grade" },
          completed_lines: SAMPLE_LINES.length,
          total_lines: SAMPLE_LINES.length,
        },
      });
      onEvent({ type: "item", payload: { item: "report", ...SAMPLE_REPORT } });
      onEvent({ type: "done" });

      return Promise.resolve<VoiceOutcome>({
        kind: "reported",
        operation_id: request.operation_id ?? "sample-op",
        report: SAMPLE_REPORT,
      });
    },

    fetchModel: (request: VoiceFetchModelRequest, onEvent) => {
      onEvent({
        type: "progress",
        payload: { stage: "downloading", bytes_done: 0, bytes_total: 1_024 },
      });
      onEvent({ type: "progress", payload: { stage: "unpacking" } });
      onEvent({ type: "done" });
      return Promise.resolve<VoiceModelOutcome>({
        kind: "ready",
        operation_id: request.operation_id ?? "sample-op",
        name: request.name,
        directory: `（样例）未真的下载 ${request.name}`,
      });
    },

    cancel: () => Promise.resolve(false),
  };
}
