/**
 * 背诵界面的数据访问端口。
 *
 * # 与 `data/ports.ts`、`data/settingsPorts.ts` 同一条理由
 *
 * 命令本体属 todo 64，而背诵界面（本 todo）排在它之前落地。所以界面先把它需要的形状
 * 说清楚，todo 64 去实现，测试用替身实现，两侧对着同一份签名。
 *
 * 每个方法都对应**已经存在**的 Rust API，没有一个是新发明的能力：
 *
 * | 端口方法 | Rust 侧的实现路径 | 出处 |
 * | --- | --- | --- |
 * | `startSession` | `PracticeSession::start(handle, body, mode)` | `modes.rs:173` |
 * | `submitAnswer` | `TypedAttempt::new` → `review_typed` → `align` → `grade_typed` | `command.rs:421-431` |
 * | `commitGrade` | `Scheduler::review(stable_id, grade)` | `schedule.rs:159` |
 * | `due` | `Scheduler::due_today()` / `due_on(i64::MAX)` | `schedule.rs:129`、`schedule.rs:134` |
 * | `stats` | 上面两个的计数 + `config.recite.grading` | `command.rs:277-289` |
 *
 * # 命令行的一次调用在这里是三个方法，这是刻意的
 *
 * `yunjian recite <id>` 一次调用里顺序做完四件事：出题、读 stdin、评分、写排程。
 * 界面必须让用户在**看到分数之后**再决定等级——语音路径按 2026-08-11 的裁决
 * **只能**手选等级，打字路径也允许覆盖内核的映射值。所以评分与落账必须可分。
 *
 * 内核本来就是两个独立入口（`score_typed` / `review_typed` 与 `Scheduler::review`），
 * 拆开不引入任何新逻辑，也不改变字段名。**这处形状差异是本 todo 向 todo 64 提出的契约**，
 * 与 todo 62 对 `PurgeScope` 的处理同一路：界面把它要的形状说清楚，IPC 层去实现它。
 *
 * # 「最多提交一次」的落点
 *
 * `commitGrade` 每调用一次就是 FSRS 里的一次复习。因此调用它的组件
 * （`recite/ReciteScreen.tsx`）必须保证一局练习最多调一次，对应 todo 56 被裁决改写后的
 * 验收「用户确认后最多提交一次」。这一条**不能**靠端口层拦住：端口是无状态的，
 * 幂等性只有会话状态知道。所以它有一条组件级断言盯着。
 *
 * # 复习队列刻意不依赖语料库
 *
 * `recite due` 在命令行里**不打开语料库**（`learnings.md` 的 todo 58 一节，已实测缺语料时
 * 仍退出 0）：复习状态按 `stable_id` 存在 `app.data_dir/recite.db`，与语料无关。
 * 所以 [`ReciteReviewPort.due`] 返回的 [`ReviewItem`] 里**只有 `poem_id` 没有题目与作者**，
 * 界面也不为了补标题去调 `poem_detail`。想显示标题就得开语料库，
 * 那会让「排程能不能看」取决于「语料下没下」，把一个本来无关的依赖装回去。
 */

import type {
  FsrsGradeId,
  ReciteAttempt,
  ReciteCommit,
  ReciteDue,
  ReciteModeId,
  ReciteSession,
  ReciteStats,
} from "../contracts/recite";

/**
 * 出题请求。
 *
 * 三个可选参数与命令行的 `--ratio` / `--seed` / `--masked-lines` 一一对应
 * （`cli.rs:207` 的 `Mode::practice(ratio, seed, masked_lines)`）。
 *
 * **省略 `seed` 时由 Rust 侧按时间取一个并在载荷里回显**，与
 * `command.rs:466-473` 的 `effective_seed` 一致。前端不自己生成种子：那会让
 * 「同一种子在任何机器上给出同一组空位」这条性质多出一个不受内核控制的来源。
 */
export interface ReciteSessionRequest {
  poem_id: string;
  mode: ReciteModeId;
  ratio?: number;
  seed?: number;
  masked_lines?: number;
}

/**
 * 作答请求。
 *
 * 出题参数要原样带回来：内核的 `PracticeSession` 是**无状态重建**的
 * （`PracticeSession::start` 每次现算），载荷里回显的 `seed` 就是为了让第二次调用
 * 重建出同一局。少带一个参数就会重建成另一局，于是 `hidden_indices` 与用户看到的
 * 提示不一致。
 *
 * `answer` 是用户原样键入的文本，**不做任何前端预处理**：去标点、去空白与
 * `variant_map` 改写都在内核的 `normalize_text` 里（`align.rs:82-84`），
 * 在前端先做一遍等于把归一化规则分叉成两份。
 */
export interface ReciteAnswerRequest extends ReciteSessionRequest {
  answer: string;
}

/** 落账请求。`grade` 由用户确认或采纳内核建议值。 */
export interface ReciteCommitRequest {
  poem_id: string;
  grade: FsrsGradeId;
  /**
   * 这个等级是用户改过的，还是直接采纳了内核的映射值。
   *
   * 取值域是 `GradeSource`，语义与 `command.rs:426-432` 一致：改过是 `user_chosen`，
   * 原样采纳是 `typed_mapping`。**它只影响载荷里的自述，不影响排程计算**——
   * `Scheduler::review` 只收等级本身。
   */
  chosen_by_user: boolean;
}

/** 一局练习：出题与作答。 */
export interface RecitePracticePort {
  /** 出题。返回的载荷**不含参考诗文**，见 `contracts/recite.ts` 的 `ReciteSession`。 */
  startSession(request: ReciteSessionRequest): Promise<ReciteSession>;
  /**
   * 提交作答，取回内核的分数、对齐操作与建议等级。**本调用不写排程。**
   *
   * 空作答**不得**调到这里：按零分记账会往复习历史写一条用户没做过的记录，
   * 事后无法撤回（`learnings.md` 的 todo 58 第 3 条）。拦截点在提交按钮。
   */
  submitAnswer(request: ReciteAnswerRequest): Promise<ReciteAttempt>;
}

/** 排程：落账、队列与统计。 */
export interface ReciteReviewPort {
  /** 提交一次复习。**每次调用都是 FSRS 里的一次真实复习。** */
  commitGrade(request: ReciteCommitRequest): Promise<ReciteCommit>;
  /** `includeFuture` 为真时等价于 `recite due --all`，即整份排程。 */
  due(includeFuture: boolean): Promise<ReciteDue>;
  stats(): Promise<ReciteStats>;
}

/** 背诵界面需要的全部端口。 */
export interface RecitePorts {
  practice: RecitePracticePort;
  review: ReciteReviewPort;
}
