/**
 * 背诵界面的传输形状与固定文案。
 *
 * # 每一个标识符都是从 Rust 源码抄来的，没有一个是发明的
 *
 * 本项目已经**六次**因为凭记忆写标识符而栽（`.omo/notepads/yunjian/issues.md`），
 * 所以下面每个字符串取值、每个字段名都带出处（`文件:行`）。抄的对象有两处：
 *
 * - `crates/yunjian-recite/`：内核类型（`AlignOp` / `TypedScore` / `FsrsGrade` / `ReviewState`）。
 * - `crates/yunjian-cli/src/output.rs`：这些内核类型的**线上镜像**（`OpOut` / `ScoreOut` /
 *   `ReviewItemOut` / `ReciteOut` / `ReciteDueOut` / `ReciteStatsOut`）。内核类型本身
 *   **没有 serde derive**，所以线上形状由镜像层决定，本文件镜像的是镜像层。
 *
 * `yunjian recite` / `recite due` / `recite stats` 三个子命令（todo 58）已经把整套背诵
 * 功能跑通了，桌面端是它之上的**薄壳**，因此字段名逐字一致，见
 * `.omo/notepads/yunjian/learnings.md` 的「todo 58」一节。
 *
 * # 界面不做任何评分，这一条写在类型里
 *
 * [`ReciteScore`] 的五个数值字段与 [`ReciteOp`] 的操作列表**全部由内核给出**，本层只搬运。
 * 没有任何函数从 [`ReciteOp`] 反推 [`ReciteScore`]，也没有把比例换算成百分比的地方——
 * 乘一个 100 也是在分数上做算术，而一旦前端开始算术，「评分只在内核」这条边界
 * 就没有可执行的判据了。这句话不是自律，`recite/__tests__/noScoreArithmetic.test.ts`
 * 把它做成了机制（正反两向：既禁算术，也确认这些字段真的被读了）。
 *
 * # 一处必须显式声明的措辞边界
 *
 * `fluency` 这个字段名在内核里的注释是「打字路径没有时序信号，使用中性满值且
 * **不表示发音质量**」（`score.rs:85-86`）。界面标签取 [`SCORE_LABEL`] 里的
 * `节奏连贯度`，并且**永远与 [`NO_PRONUNCIATION_NOTE`] 同屏出现**。
 * 把它写成「发音」「读音标准」「口音」中的任何一个都是一句谎话——
 * 打字路径根本没有音频，语音路径按 2026-08-11 的裁决也只报「是否开口/停顿/相对节奏」。
 */

/* ────────────────────────── 练习形态 ────────────────────────── */

/**
 * 用户可以请求的四种形态。
 *
 * 逐字取自 `crates/yunjian-cli/src/cli.rs:196-203` 的 `Mode::as_key()`，
 * 那里的注释写明这些串「写进载荷的稳定标识，与命令行取值逐字一致」。
 *
 * **注意 `first-char` 是连字符而不是下划线**，与其余 snake_case 的词汇表不同步。
 * 猜成 `first_char` 会让形态选择静默落到默认分支。
 */
export const RECITE_MODE_IDS = ["cloze", "first-char", "masked", "voice"] as const;

export type ReciteModeId = (typeof RECITE_MODE_IDS)[number];

/**
 * 实际能被内核执行的三种形态。
 *
 * 逐字取自 `crates/yunjian-cli/src/command.rs:474-480` 的 `mode_key()`。
 * `voice` **不在其中**：`Mode::practice()`（`cli.rs:207-215`）对它返回 `None`，
 * 由调用方决定退化，见 [`ReciteSession.fallback_reason`]。
 */
export type ExecutedModeId = Exclude<ReciteModeId, "voice">;

/** 四种形态的中文名。取自 `output.rs:1242-1249` 的 `render_mode()`，不另起译名。 */
export const RECITE_MODE_LABEL: Record<ReciteModeId, string> = {
  cloze: "挖空",
  "first-char": "首字提示",
  masked: "遮挡",
  voice: "语音",
};

/** 每种形态一句说明，取自 `crates/yunjian-recite/src/modes.rs:122-129` 的变体文档注释。 */
export const RECITE_MODE_HINT: Record<ReciteModeId, string> = {
  cloze: "按比例挖掉若干字，优先挖韵脚，其次实词。",
  "first-char": "每行只留第一个字。",
  masked: "从全文可见逐行遮到全遮。",
  voice: "逐句示范朗读、用户复诵；只反馈是否开口、停顿与相对节奏，不做机器评分。",
};

/**
 * 提示文本里代表被遮住一字的字符。
 *
 * 逐字取自 `crates/yunjian-recite/src/modes.rs:34` 的 `MASK_CHARACTER`：
 * **全角下划线 `＿`（U+FF3F），不是 ASCII `_`**。内核选它的理由是 ASCII 下划线属标点，
 * 会被 `content_chars` 剥掉，于是「提示里有几个空」在任何按正文字符计数的地方都看不见。
 */
export const MASK_CHARACTER = "＿";

/**
 * 挖空比例的默认值。
 *
 * 取自 `crates/yunjian-recite/src/modes.rs:55` 的 `ClozeOptions::DEFAULT_RATIO`。
 */
export const DEFAULT_CLOZE_RATIO = 0.3;

/* ────────────────────────── 对齐操作 ────────────────────────── */

/**
 * 一次对齐里的一项朗读行为。
 *
 * 内部 tag 是 `kind`，取值 snake_case——`output.rs:904-906` 的
 * `#[serde(tag = "kind", rename_all = "snake_case")]`。
 * **回读那一项的 tag 是 `re_recitation`**（变体名 `ReRecitation` 的 snake_case 是
 * 两个词而不是三个），猜成 `rerecitation` 会让回读落到默认分支。
 * 与之相对，[`ReciteOpsSummary`] 里的计数字段却叫 `rerecitation_count`——
 * **这两处刻意不同名，因为 Rust 侧就不同名**（`score.rs:71-72` 的字段名 vs
 * `align.rs:36` 的变体名），不要「顺手统一」。
 *
 * 下标都是**归一化文本**的字符位置（去标点、去空白、过 `variant_map`），
 * 与提示文本的下标不通用——见 `align.rs:71-74` 与 `modes.rs:201-205`。
 *
 * `near_homophone` 只在 `substitution` 上出现，由内核的 `classify_substitution`
 * 判定（`output.rs:1006-1013`），**前端不自己判近音**：那需要读音表，而读音表在内核里。
 */
export type ReciteOp =
  | { kind: "normal"; reference_index: number; attempt_index: number; character: string }
  | { kind: "deletion"; reference_index: number; reference: string }
  | { kind: "insertion"; reference_index: number; attempt_index: number; attempt: string }
  | {
      kind: "re_recitation";
      reference_start: number;
      reference_end: number;
      attempt_start: number;
      attempt_end: number;
      text: string;
    }
  | {
      kind: "substitution";
      reference_index: number;
      attempt_index: number;
      reference: string;
      attempt: string;
      near_homophone: boolean;
    };

/** [`ReciteOp`] 的 tag 取值，供穷举与测试使用。 */
export const RECITE_OP_KINDS = [
  "normal",
  "deletion",
  "insertion",
  "re_recitation",
  "substitution",
] as const;

/**
 * 各类对齐操作的计数。
 *
 * 字段名逐一对应 `crates/yunjian-recite/src/score.rs:63-73` 的 `OpsSummary`
 * （线上镜像见 `output.rs:875-890` 的 `OpsSummaryOut`）。
 */
export interface ReciteOpsSummary {
  normal_count: number;
  deletion_count: number;
  insertion_count: number;
  rerecitation_count: number;
  substitution_count: number;
}

/* ────────────────────────── 分数 ────────────────────────── */

/**
 * 一次打字作答的分数。**五个字段全部由内核给出，界面一个也不算。**
 *
 * 字段名逐一对应 `crates/yunjian-recite/src/score.rs:78-92` 的 `TypedScore`
 * （线上镜像见 `output.rs:846-859` 的 `ScoreOut`）。
 *
 * 三处内核语义，界面必须照着呈现而不是重新解释：
 *
 * - `completeness` 是「未漏读字符所占比例」，**不含替换与增读**。
 * - `accuracy_lenient` 在近音宽容层接入前**等于** `accuracy_strict`
 *   （`score.rs:158`）。两者相等不是 bug，所以两个数都要显示：只显示一个的界面
 *   在宽容层生效那天会静默改变含义。
 * - `is_rejected` 由「匹配过低」或「字符错误率过高」任一条触发（`score.rs:160`），
 *   它是**内核的判定**，不是从分数推出来的阈值比较。
 *
 * 比例一律以 `[0, 1]` 的小数呈现，**不换算百分比**——与 `output.rs:842-844`
 * 对 `ScoreOut` 的同一条约定一致。
 */
export interface ReciteScore {
  completeness: number;
  accuracy_strict: number;
  accuracy_lenient: number;
  /** 打字路径无时序信号，内核给中性满值；**不表示发音质量**，见本文件头部。 */
  fluency: number;
  is_rejected: boolean;
  ops_summary: ReciteOpsSummary;
}

/** 四个数值字段的界面标签。措辞取自 `output.rs:1215` 的人类输出，不另起译名。 */
export const SCORE_LABEL = {
  completeness: "完整度",
  accuracy_strict: "严格字准",
  accuracy_lenient: "宽容字准",
  fluency: "节奏连贯度",
} as const;

/**
 * 结果视图必须出现的那一句。
 *
 * 逐字取自 `output.rs:1221` 的人类输出行。任务书要求界面含「不评估发音标准度」，
 * 这一句既包含那六个字，又把「节奏连贯度为什么是满值」一并说清楚——
 * 只写前半句会让用户以为满值代表读得好。
 */
export const NO_PRONUNCIATION_NOTE = "不评估发音标准度；打字路径的节奏连贯度为中性值。";

/**
 * `is_rejected` 为真时那一句。
 *
 * 前半截逐字取自 `output.rs:1219`。**后半截刻意与命令行不同**：命令行写
 * 「本次记为最低档」，因为 CLI 一次调用里评级已经落账；桌面端把评级拆成了
 * 「先看分数、再确认等级」两步（见 [`ReciteAttempt`]），此时说「已记为」是假话，
 * 所以改成「内核建议记为最低档」。这处偏离是刻意的，记在这里而不是留给读者猜。
 */
export const REJECTED_NOTE =
  "内核判为拒绝识别：作答与本篇相去过远，内核建议记为最低档；等级仍可在下方调整。";

/* ────────────────────────── FSRS 等级 ────────────────────────── */

/**
 * 四档等级的线上串。
 *
 * 逐字取自 `output.rs:820-827` 的 `grade_key()`；顺序取自
 * `crates/yunjian-recite/src/schedule.rs:36` 的 `FsrsGrade::ALL`
 * （`Again` → `Hard` → `Good` → `Easy`，即由差到好），照抄顺序不要重排。
 */
export const FSRS_GRADE_IDS = ["again", "hard", "good", "easy"] as const;

export type FsrsGradeId = (typeof FSRS_GRADE_IDS)[number];

/** 四档等级的中文名。逐字取自 `output.rs:831-838` 的 `grade_label()`。 */
export const FSRS_GRADE_LABEL: Record<FsrsGradeId, string> = {
  again: "重来",
  hard: "困难",
  good: "良好",
  easy: "轻松",
};

/** 四档等级各自一句说明。取自 `schedule.rs:24-31` 的变体文档注释。 */
export const FSRS_GRADE_HINT: Record<FsrsGradeId, string> = {
  again: "未能回忆，需要尽快再次复习。",
  hard: "回忆困难。",
  good: "正常回忆。",
  easy: "轻松且准确地回忆。",
};

/**
 * 这一次的等级是怎么来的。
 *
 * 逐字取自 `crates/yunjian-cli/src/command.rs:426-432`：`--grade` 给出时是
 * `user_chosen`，否则走 `grade_typed` 是 `typed_mapping`。
 */
export type GradeSource = "typed_mapping" | "user_chosen";

/** 两种来源的界面说法。 */
export const GRADE_SOURCE_LABEL: Record<GradeSource, string> = {
  typed_mapping: "由内核按打字分数映射",
  user_chosen: "由你手动指定",
};

/**
 * 语音路径的等级只能手选，这一句说明为什么。
 *
 * 依据是 2026-08-11 的裁决（`.omo/notepads/yunjian/problems.md`）：文言 ASR 实测
 * CER 77.01%，那是 TTS 合成音的乐观上界，因此 v1 语音契约是「示范 + 复诵 + 节奏反馈
 * + 用户自评」，**不做机器自动评分**，也不做完整度推断。
 */
export const VOICE_GRADE_IS_MANUAL_NOTE =
  "语音路径不做机器评分：文言识别的字错率实测过高，任何自动完整度都是噪声。" +
  "语音练习的等级一律由你自己选定。";

/**
 * 四个评级阈值，原样取自 `[recite.grading]`。
 *
 * 字段名逐一对应 `crates/yunjian-core/src/config.rs:274-283` 的 `GradingConfig`。
 * **界面只显示它们，不用它们重算等级**：等级由 `grade_typed`
 * （`schedule.rs:60-70`）按严格优先级得出，在前端再算一遍就会多出第二套规则。
 * 这与 `output.rs:1316-1319` 对 `ReciteStatsOut.grading` 的同一条约定一致。
 */
export interface GradingThresholds {
  again_completeness_below: number;
  hard_accuracy_lenient_below: number;
  hard_rerecitation_above: number;
  easy_accuracy_strict_at_least: number;
}

/* ────────────────────────── 排程 ────────────────────────── */

/**
 * 一条排程项。
 *
 * 字段名逐一对应 `output.rs:1084-1099` 的 `ReviewItemOut`。
 * **注意它把内核的 `stable_id` 改名成了 `poem_id`**（`output.rs:1106`），
 * 而内核 `ReviewState` 里那个字段叫 `stable_id`（`schedule.rs:81`）。
 * 两个名字都是真的，取决于看哪一层；本前端走线上镜像那一层，所以用 `poem_id`。
 *
 * `due_day` 与 `last_review_day` 是 **Unix 日序号**（不是时间戳、不是 ISO 串），
 * 见 `schedule.rs:86-91`。界面显示它时**不做日期换算**：把日序号换成「还有几天」
 * 需要知道「今天是第几天」，而那个基准在内核里（`unix_day_now()`）；
 * 「还有几天」这件事内核已经用 `scheduled_days` 给出来了。
 */
export interface ReviewItem {
  poem_id: string;
  due_day: number;
  last_review_day: number;
  scheduled_days: number;
  stability: number;
  difficulty: number;
  last_grade: FsrsGradeId;
}

/**
 * 复习队列的载荷。
 *
 * 字段名逐一对应 `output.rs:1265-1272` 的 `ReciteDueOut`；`scope` 的两个取值
 * 逐字取自 `command.rs:271`（`"all"` / `"due_today"`）。
 *
 * `items` 已按到期日升序（`schedule.rs:138` 的 `ORDER BY due_day, stable_id`），
 * **前端不重排**：重排一遍就等于在界面里放了第二套排序规则。
 */
export interface ReciteDue {
  database: string;
  scope: "due_today" | "all";
  items: ReviewItem[];
}

/**
 * 复习队列里两条说明的固定文案。
 *
 * # 为什么是常量而不是 JSX 里的文字
 *
 * 两个都是**亲手 QA 抓出来的缺陷**，单元测试全绿时它们都在页面上：
 *
 * 1. 我在 JSX 文字里写了 Markdown 的 `**粗体**`。JSX 不解析 Markdown，
 *    浏览器上原样显示成两个星号。
 * 2. 一段跨行的 JSX 文字在渲染时会把换行折叠成一个空格，于是句号后多出
 *    一个空格（「随之改变。 内核」）。中文排版里那是一个可见的洞。
 *
 * 两条都只在真的看一眼页面时才现形。搬成常量之后，字符串里有什么就渲染什么，
 * 而且 `__tests__` 能直接对着常量断言。要强调就用 `<strong>`，不用星号。
 */
export const QUEUE_DISTRIBUTION_NOTE = "分布按每首作品最近一次的等级计，不是历次复习的直方图。";

export const QUEUE_REGRADE_NOTE =
  "点某一档等级会作为一次新的复习提交给排程器，间隔与稳定度随之改变。" +
  "内核没有「修正上一次评级」的入口，所以这不是改标签。";

/** 空队列时那一句。取自 `output.rs:1286-1292`，把命令行写法换成界面动作。 */
export const EMPTY_QUEUE_NOTE =
  "没有到期项。练一轮即可建立排程；勾选「含未到期」可看尚未到期的部分。";

/**
 * 排程统计的载荷。
 *
 * 字段名逐一对应 `output.rs:1306-1319` 的 `ReciteStatsOut` 与
 * `output.rs:1323-1332` 的 `GradeCountsOut`。
 *
 * `by_last_grade` 是**每首作品最近一次的等级**分布，不是历次复习的直方图——
 * 内核的 `review_log` 是私有的（只喂 `optimize_parameters`），没有公开 history
 * accessor，见 `learnings.md` 的 todo 58 一节。界面文案必须照这个口径写。
 */
export interface ReciteStats {
  database: string;
  scheduled_total: number;
  due_today: number;
  by_last_grade: Record<FsrsGradeId, number>;
  grading: GradingThresholds;
}

/* ────────────────────────── 一局练习的两个阶段 ────────────────────────── */

/**
 * 出题结果：**刻意不携带参考诗文**。
 *
 * 这是本文件最重要的一处形状决定。命令行的 `ReciteOut`（`output.rs:1134-1180`）里
 * 有 `reference` 字段，因为它是**作答之后**才产生的载荷。桌面端把一局拆成两步，
 * 于是「出题」这一步的载荷有条件携带答案——而一旦它携带了，任何一次手滑
 * （渲染整个对象、写进 `title`、塞进 dev 面板）都会把答案摊在用户眼前。
 *
 * 所以这里照 todo 61 处理缺出处集评、todo 62 处理已存密钥的同一条办法：
 * **让错误在类型上无从表达**。出题阶段拿不到参考诗文，于是「把答案显示出来」
 * 不是被规范禁止，而是没有数据来源可用。
 *
 * `prompt` 保留原标点，被遮处是 [`MASK_CHARACTER`]（`modes.rs:194-197`）。
 * `hidden_indices` 是**原文正文字符**下标，只用于出题与展示，与归一化下标不通用
 * （`modes.rs:200-207`）。
 */
export interface ReciteSession {
  poem_id: string;
  title: string;
  author: string;
  dynasty: string;
  /** 实际执行的形态。请求 `voice` 时这里是退化后的形态。 */
  mode: ExecutedModeId;
  /** 用户请求的形态；仅在退化时出现，值为 `"voice"`（`output.rs:1147`）。 */
  requested_mode?: ReciteModeId;
  /** 退化原因；仅在退化时出现。文案由内核给，界面不自己编（`command.rs:307-320`）。 */
  fallback_reason?: string;
  /** 生效的挖空比例；非挖空形态不出现。 */
  ratio?: number;
  /** 生效的随机种子；非挖空形态不出现。**给出它就能复现同一份挖空。** */
  seed?: number;
  /** 实际遮住的句数；非遮挡形态不出现。 */
  masked_lines?: number;
  prompt: string;
  hidden_indices: number[];
  /** 按呈现行切分得到的行数（`modes.rs:216-220`），供遮挡档位滑块定上界。 */
  line_count: number;
}

/**
 * 作答结果：分数、对齐操作与**内核建议的**等级，但**尚未落账**。
 *
 * 字段名与 `ReciteOut`（`output.rs:1134-1180`）逐一对应，只有一处不同：
 * 那里叫 `grade`，这里叫 [`suggested_grade`]。改名是因为语义真的不同——
 * `ReciteOut.grade` 是**已经提交给排程器**的等级（`command.rs:434` 之后），
 * 而这里是「内核按打字分数映射出来的建议值，等你确认」。
 * 沿用同一个名字会让「已落账」与「待确认」两种状态在类型上不可区分。
 *
 * # 为什么把一次 `yunjian recite` 拆成两个命令
 *
 * 命令行一次调用里顺序做完四件事：出题、读 stdin、评分、`Scheduler::review`。
 * 界面必须让用户在看到分数**之后**再决定等级（语音路径按裁决**只能**手选），
 * 于是评分与落账必须可分。内核本来就是分开的两个入口
 * （`score_typed` / `review_typed` 与 `Scheduler::review`），拆开不需要新逻辑。
 *
 * **代价是必须保证「最多提交一次」**：这一条对应 todo 56 被改写后的验收
 * 「用户确认后最多提交一次」（见 `problems.md` 的 2026-08-11 裁决表）。
 * 落点在 `recite/ReciteScreen.tsx`：提交后按钮不可再按，改为显示排程结果。
 */
export interface ReciteAttempt {
  poem_id: string;
  title: string;
  author: string;
  dynasty: string;
  mode: ExecutedModeId;
  requested_mode?: ReciteModeId;
  fallback_reason?: string;
  ratio?: number;
  seed?: number;
  masked_lines?: number;
  prompt: string;
  hidden_indices: number[];
  /** 归一化后的参考诗文。作答之后才出现，见 [`ReciteSession`] 的说明。 */
  reference: string;
  /** 归一化后的作答。 */
  answer: string;
  score: ReciteScore;
  ops: ReciteOp[];
  /** 内核 `grade_typed` 的映射结果，**待用户确认**。 */
  suggested_grade: FsrsGradeId;
  /** 本次是否为该作品的首次作答。`grade_typed` 只在首次作答时允许评 `easy`。 */
  first_attempt: boolean;
  /** 复习库路径，与 `ReciteOut.database` 同义。 */
  database: string;
}

/**
 * 落账结果：这一次真的写进排程器之后的状态。
 *
 * 四个字段名与 `ReciteOut` 的同名字段一致（`output.rs:1173-1181`）。
 */
export interface ReciteCommit {
  grade: FsrsGradeId;
  grade_source: GradeSource;
  database: string;
  review: ReviewItem;
}
