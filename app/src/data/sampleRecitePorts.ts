/**
 * 非 Tauri 宿主下的背诵端口替身，以及 Tauri 宿主下的 IPC 形状。
 *
 * # 替身**不实现评分内核**，它只搬固定样例
 *
 * 这是本文件最要紧的一条。评分内核在 `crates/yunjian-recite/` 里，替身若为了让
 * `vite dev` 看起来「能用」而在 TypeScript 里算一遍对齐与分数，那么本 todo 想守住的
 * 边界当场就破了——而且是从测试替身这个最不显眼的地方破的。
 *
 * 所以 [`createSampleRecitePorts`] 的 `submitAnswer` **无条件返回同一份写死的样例载荷**，
 * 里面的分数、对齐操作与建议等级都是照内核公式**手算一次后写死**的常量
 * （算式见 [`SAMPLE_ATTEMPT`] 的注释，可以逐项复核）。它**不看用户键入了什么**：
 *
 * - 「按作答挑一份样例」需要先判断作答对不对，那就是第二条评分路径；
 * - 「照作答算一份分数」直接就是把内核抄进前端。
 *
 * 代价是样例模式下无论键入什么，结果视图都显示同一份差异，且显示的「作答」是样例里的
 * 那一串而不是用户键入的那一串。这在 `App.tsx` 的常驻样例横幅之下是可接受的，
 * **而且它恰好是目视 QA 需要的那份数据**：五类差异（漏读/增读/回读/替换/近音替换）
 * 一次全部出现，否则得手打一段刚好触发五类的作答才能看清标记是否可分。
 *
 * # 样例数据自报身份
 *
 * 与 `data/samplePorts.ts`、`data/sampleSettingsPorts.ts` 同一条规矩：正文只用毫无争议的
 * 公有领域名篇（《静夜思》），复习库路径写明是样例路径，日序号标注为样例值。
 * 一张 dev 截图会被当成产品行为，所以样例里不能出现任何看起来像真实统计的数字。
 */

import { invoke } from "@tauri-apps/api/core";
import type {
  ReciteAttempt,
  ReciteCommit,
  ReciteDue,
  ReciteOp,
  ReciteSession,
  ReciteStats,
  ReviewItem,
} from "../contracts/recite";
import { DEFAULT_CLOZE_RATIO } from "../contracts/recite";
import type {
  ReciteAnswerRequest,
  ReciteCommitRequest,
  RecitePorts,
  RecitePracticePort,
  ReciteReviewPort,
  ReciteSessionRequest,
} from "./recitePorts";

/**
 * todo 64 要注册的命令名。
 *
 * 与 `data/tauriPorts.ts` 的 `IPC_COMMANDS`、`data/sampleSettingsPorts.ts` 的
 * `SETTINGS_IPC_COMMANDS` 同一条理由：命令名写错是**静默失败**（`invoke` 的 promise
 * 被拒，界面只看到一条「出题失败」），所以它必须是一个能被 grep 出来核对的清单。
 * 名字取 snake_case，与既有十五个一致。
 */
export const RECITE_IPC_COMMANDS = {
  startSession: "recite_start_session",
  submitAnswer: "recite_submit_answer",
  commitGrade: "recite_commit_grade",
  due: "recite_due",
  stats: "recite_stats",
} as const;

function inTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

/** Tauri 宿主下的背诵端口；不在宿主里时返回 `null`。 */
export function createTauriRecitePorts(): RecitePorts | null {
  if (!inTauri()) {
    return null;
  }

  const practice: RecitePracticePort = {
    startSession: (request: ReciteSessionRequest) =>
      invoke<ReciteSession>(RECITE_IPC_COMMANDS.startSession, { request }),
    submitAnswer: (request: ReciteAnswerRequest) =>
      invoke<ReciteAttempt>(RECITE_IPC_COMMANDS.submitAnswer, { request }),
  };

  const review: ReciteReviewPort = {
    commitGrade: (request: ReciteCommitRequest) =>
      invoke<ReciteCommit>(RECITE_IPC_COMMANDS.commitGrade, { request }),
    due: (includeFuture: boolean) => invoke<ReciteDue>(RECITE_IPC_COMMANDS.due, { includeFuture }),
    stats: () => invoke<ReciteStats>(RECITE_IPC_COMMANDS.stats),
  };

  return { practice, review };
}

/* ────────────────────────── 以下全是写死的样例常量 ────────────────────────── */

/** 样例复习库路径。写成一眼能看出是样例的形状，不冒用真实的 `app.data_dir`。 */
const SAMPLE_DATABASE = "（样例）尚未连接复习库";

/**
 * 样例作品：《静夜思》。公有领域，正文毫无争议。
 *
 * `stable_id` 与 `data/samplePorts.ts` 里的那一条一致，于是两个界面说的是同一首诗。
 */
const SAMPLE_POEM = {
  poem_id: "sample-jingyesi",
  title: "静夜思",
  author: "李白",
  dynasty: "唐",
} as const;

/** 归一化后的参考诗文：去标点、去空白后的 20 个字。 */
const SAMPLE_REFERENCE = "床前明月光疑是地上霜举头望明月低头思故乡";

/**
 * 样例作答：刻意让五类差异各出现至少一次。
 *
 * 逐字对照 [`SAMPLE_OPS`]：回读「明月光」、望→看（异音替换）、多读一个「啊」、
 * 思→丝（近音替换）、漏掉「故」。
 */
const SAMPLE_ANSWER = "床前明月光明月光疑是地上霜举头看明月啊低头丝乡";

/**
 * 五类差异齐备的对齐操作列表，按作答顺序排列。
 *
 * **这是写死的样例，不是算出来的。** 下标口径与内核一致：`reference_index` 是
 * 归一化参考文本的字符位置，`attempt_index` 是归一化作答文本的字符位置。
 */
const SAMPLE_OPS: ReciteOp[] = [
  { kind: "normal", reference_index: 0, attempt_index: 0, character: "床" },
  { kind: "normal", reference_index: 1, attempt_index: 1, character: "前" },
  { kind: "normal", reference_index: 2, attempt_index: 2, character: "明" },
  { kind: "normal", reference_index: 3, attempt_index: 3, character: "月" },
  { kind: "normal", reference_index: 4, attempt_index: 4, character: "光" },
  {
    kind: "re_recitation",
    reference_start: 2,
    reference_end: 5,
    attempt_start: 5,
    attempt_end: 8,
    text: "明月光",
  },
  { kind: "normal", reference_index: 5, attempt_index: 8, character: "疑" },
  { kind: "normal", reference_index: 6, attempt_index: 9, character: "是" },
  { kind: "normal", reference_index: 7, attempt_index: 10, character: "地" },
  { kind: "normal", reference_index: 8, attempt_index: 11, character: "上" },
  { kind: "normal", reference_index: 9, attempt_index: 12, character: "霜" },
  { kind: "normal", reference_index: 10, attempt_index: 13, character: "举" },
  { kind: "normal", reference_index: 11, attempt_index: 14, character: "头" },
  {
    kind: "substitution",
    reference_index: 12,
    attempt_index: 15,
    reference: "望",
    attempt: "看",
    near_homophone: false,
  },
  { kind: "normal", reference_index: 13, attempt_index: 16, character: "明" },
  { kind: "normal", reference_index: 14, attempt_index: 17, character: "月" },
  { kind: "insertion", reference_index: 15, attempt_index: 18, attempt: "啊" },
  { kind: "normal", reference_index: 15, attempt_index: 19, character: "低" },
  { kind: "normal", reference_index: 16, attempt_index: 20, character: "头" },
  {
    kind: "substitution",
    reference_index: 17,
    attempt_index: 21,
    reference: "思",
    attempt: "丝",
    near_homophone: true,
  },
  { kind: "deletion", reference_index: 18, reference: "故" },
  { kind: "normal", reference_index: 19, attempt_index: 22, character: "乡" },
];

/**
 * 样例作答的分数。
 *
 * **照内核公式手算一次后写死**，算式如下（`score.rs:133-166` 的 `score_alignment`），
 * 参考长度 20、漏读 1、增读 1、替换 2、回读 1、相符 17：
 *
 * - `completeness`：一减漏读占比，即 `1 - 1/20` = `0.95`
 * - `accuracy_strict`：一减字符错误率，错误数 `2 + 1 + 1 = 4`，即 `1 - 4/20` = `0.8`
 * - `accuracy_lenient`：宽容层接入前等于严格值，即 `0.8`
 * - `fluency`：打字路径的中性满值 `1`
 * - `is_rejected`：匹配率 `17/20 = 0.85` 高于 `0.35`、字符错误率 `0.2` 低于 `0.6`，故 `false`
 *
 * 写下算式是为了让这份常量可复核；**运行时一个乘号都没有**。
 */
const SAMPLE_SCORE = {
  completeness: 0.95,
  accuracy_strict: 0.8,
  accuracy_lenient: 0.8,
  fluency: 1,
  is_rejected: false,
  ops_summary: {
    normal_count: 17,
    deletion_count: 1,
    insertion_count: 1,
    rerecitation_count: 1,
    substitution_count: 2,
  },
} as const;

/**
 * 样例的建议等级：`hard`。
 *
 * 同样是照内核规则（`schedule.rs:60-70` 的 `grade_typed`）与默认阈值
 * （`config.rs:285-294` 的 `GradingConfig::default`）手算后写死：未被拒绝且完整度
 * `0.95` 不低于 `0.6`，所以不是 `again`；宽容字准 `0.8` 低于 `0.85`，落 `hard`。
 * 回读 `1` 次高于阈值 `0`，同样落 `hard`——两条各自成立，正好演示
 * 「任何回读封顶 Hard」这条规则。
 */
const SAMPLE_SUGGESTED_GRADE = "hard" as const;

/**
 * 样例的排程项。
 *
 * 日序号是**样例值**，不是从当前时间算出来的：算一次「今天是第几天」就得在前端引入
 * 一个与内核 `unix_day_now()` 并行的时间基准，而两个基准迟早会在跨时区时对不上。
 */
const SAMPLE_QUEUE: ReviewItem[] = [
  {
    poem_id: "sample-jingyesi",
    due_day: 20670,
    last_review_day: 20668,
    scheduled_days: 2,
    stability: 3.41,
    difficulty: 5.62,
    last_grade: "hard",
  },
  {
    poem_id: "sample-dengguanquelou-a",
    due_day: 20674,
    last_review_day: 20667,
    scheduled_days: 7,
    stability: 9.08,
    difficulty: 4.15,
    last_grade: "good",
  },
  {
    poem_id: "sample-chunxiao",
    due_day: 20675,
    last_review_day: 20675,
    scheduled_days: 1,
    stability: 1.02,
    difficulty: 7.31,
    last_grade: "again",
  },
];

/**
 * 队列里没有这一首时用的样例排程项。
 *
 * 单独写一条而不是取 `SAMPLE_QUEUE[0]`：数组下标在开了
 * `noUncheckedIndexedAccess` 的项目里是可选类型，而这里需要一个确定存在的值。
 */
const SAMPLE_FALLBACK_ITEM: ReviewItem = {
  poem_id: "sample-unknown",
  due_day: 20675,
  last_review_day: 20675,
  scheduled_days: 1,
  stability: 1,
  difficulty: 5,
  last_grade: "again",
};

/** 只在 `--all` 等价物下出现的那一条：尚未到期。 */
const SAMPLE_FUTURE_ITEM: ReviewItem = {
  poem_id: "sample-dengguanquelou-b",
  due_day: 20702,
  last_review_day: 20672,
  scheduled_days: 30,
  stability: 31.5,
  difficulty: 3.04,
  last_grade: "easy",
};

/**
 * 各形态的样例提示文本。
 *
 * 三份都是**写死的**：挖空要按种子挑位置（`modes.rs:314-320` 的 `hidden_indices` 优先级规则）、
 * 首字提示与遮挡要按呈现行切句（`split_metrical_lines`），三件事都在内核里。
 * 被遮处用 `MASK_CHARACTER` 那个全角下划线，与内核 `render_prompt` 的输出形状一致。
 */
const SAMPLE_PROMPTS: Record<"cloze" | "first-char" | "masked", string> = {
  cloze: "床前明＿光，疑是地上＿。举头望明月，低＿思故乡。",
  "first-char": "床＿＿＿＿，疑＿＿＿＿。举＿＿＿＿，低＿＿＿＿。",
  masked: "＿＿＿＿＿，＿＿＿＿＿。举头望明月，低头思故乡。",
};

/** 各形态样例提示里被遮位置的下标（原文正文字符序列）。同样写死。 */
const SAMPLE_HIDDEN: Record<"cloze" | "first-char" | "masked", number[]> = {
  cloze: [3, 9, 16],
  "first-char": [1, 2, 3, 4, 6, 7, 8, 9, 11, 12, 13, 14, 16, 17, 18, 19],
  masked: [0, 1, 2, 3, 4, 5, 6, 7, 8, 9],
};

/**
 * 样例里语音形态的退化原因。
 *
 * 逐字取自 `command.rs:307-320` 的 `VoiceFallback::message`，取
 * `SessionUnavailable` 那一支——因为它就是此刻的真实原因：语音会话（todo 66）
 * 还没接进来。**不自己编一句**，也不报成「理论上可用」。
 */
const SAMPLE_VOICE_FALLBACK =
  "语音会话尚未接入本版本；已退化为挖空打字练习并照常计入排程；评分内核与语音路径完全相同";

/** 样例的评级阈值，逐字取自 `GradingConfig::default()`（`config.rs:285-294`）。 */
const SAMPLE_GRADING = {
  again_completeness_below: 0.6,
  hard_accuracy_lenient_below: 0.85,
  hard_rerecitation_above: 0,
  easy_accuracy_strict_at_least: 0.97,
} as const;

/** 出题阶段的样例载荷。**不含参考诗文**，与类型一致。 */
function sampleSession(request: ReciteSessionRequest): ReciteSession {
  // 三元式而不是 `degraded ? … : …` 里再取 `request.mode`：后者的类型仍含 `"voice"`，
  // 而 `ReciteSession.mode` 是 `ExecutedModeId`。让类型检查器看见这次收窄，
  // 而不是靠一个布尔在旁边保证。
  const executed: "cloze" | "first-char" | "masked" =
    request.mode === "voice" ? "cloze" : request.mode;
  const degraded = request.mode === "voice";
  return {
    ...SAMPLE_POEM,
    mode: executed,
    ...(degraded ? { requested_mode: "voice" as const } : {}),
    ...(degraded ? { fallback_reason: SAMPLE_VOICE_FALLBACK } : {}),
    ...(executed === "cloze" ? { ratio: request.ratio ?? DEFAULT_CLOZE_RATIO } : {}),
    // 种子由内核在真实路径上按时间取并回显；样例给一个固定值，正好体现「回显即可复现」。
    ...(executed === "cloze" ? { seed: request.seed ?? 20260813 } : {}),
    ...(executed === "masked" ? { masked_lines: request.masked_lines ?? 2 } : {}),
    prompt: SAMPLE_PROMPTS[executed],
    hidden_indices: SAMPLE_HIDDEN[executed],
    line_count: 4,
  };
}

/** 作答阶段的样例载荷。见文件头：**不看 `request.answer`**。 */
const SAMPLE_ATTEMPT = (request: ReciteAnswerRequest): ReciteAttempt => {
  const session = sampleSession(request);
  return {
    ...session,
    reference: SAMPLE_REFERENCE,
    answer: SAMPLE_ANSWER,
    score: SAMPLE_SCORE,
    ops: SAMPLE_OPS,
    suggested_grade: SAMPLE_SUGGESTED_GRADE,
    first_attempt: false,
    database: SAMPLE_DATABASE,
  };
};

/**
 * 非 Tauri 宿主下的背诵端口替身。
 *
 * `commitGrade` 会把提交过的等级记在一个只在内存里的 Map 上，于是复习队列在
 * 一次会话内能反映刚提交的那一次——这不是评分，是**替身在模拟排程库的写入**，
 * 排程算出来的间隔与稳定度仍然是写死的样例值（真实值只有 FSRS 算得出来）。
 */
export function createSampleRecitePorts(): RecitePorts {
  const committed = new Map<string, ReviewItem["last_grade"]>();

  const practice: RecitePracticePort = {
    startSession: (request) => Promise.resolve(sampleSession(request)),
    submitAnswer: (request) => Promise.resolve(SAMPLE_ATTEMPT(request)),
  };

  const review: ReciteReviewPort = {
    commitGrade: (request) => {
      committed.set(request.poem_id, request.grade);
      const existing = SAMPLE_QUEUE.find((item) => item.poem_id === request.poem_id);
      const base = existing ?? SAMPLE_FALLBACK_ITEM;
      return Promise.resolve({
        grade: request.grade,
        grade_source: request.chosen_by_user ? "user_chosen" : "typed_mapping",
        database: SAMPLE_DATABASE,
        review: { ...base, poem_id: request.poem_id, last_grade: request.grade },
      });
    },
    due: (includeFuture) => {
      const items = (includeFuture ? [...SAMPLE_QUEUE, SAMPLE_FUTURE_ITEM] : SAMPLE_QUEUE).map(
        (item) => {
          const grade = committed.get(item.poem_id);
          return grade === undefined ? item : { ...item, last_grade: grade };
        },
      );
      return Promise.resolve({
        database: SAMPLE_DATABASE,
        scope: includeFuture ? "all" : "due_today",
        items,
      });
    },
    stats: () =>
      Promise.resolve({
        database: SAMPLE_DATABASE,
        scheduled_total: 4,
        due_today: 3,
        by_last_grade: { again: 1, hard: 1, good: 1, easy: 1 },
        grading: SAMPLE_GRADING,
      }),
  };

  return { practice, review };
}
