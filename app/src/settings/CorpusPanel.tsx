/**
 * 语料状态面板：版本、记录数、取用/更新动作。
 *
 * # 「没有语料库」不是错误
 *
 * README 说明随包语料要下载 211 MiB，所以首次启动必然是 `absent`。
 * 把它渲染成错误横幅会让每个新用户第一眼看到一条红字。这里是一句陈述加一个按钮。
 *
 * # 字段名从 `CorpusMeta` 抄来
 *
 * 版本是 `corpus_version`，记录数是 **`poem_count`**（不是 `record_count`）。
 * 另外三个字段（`index_detail_mode` / `derived_indexes` / `shipped_scope`）也一起显示：
 * 「检索结构首启本机派生」是一个用户能感知到的行为（约 10 分钟），
 * 只显示版本与首数会让那件事变成一个没有解释的卡顿。
 */

import { type CSSProperties, useCallback, useEffect, useState } from "react";
import type { CorpusProgress, CorpusStatus } from "../contracts/settings";
import { CORPUS_PROGRESS_LABEL } from "../contracts/settings";
import type { CorpusPort } from "../data/settingsPorts";

export interface CorpusPanelProps {
  port: CorpusPort;
}

/**
 * 比例的百分数写法。
 *
 * 用 `Intl` 而不是自己把比例换算成整数百分数：一来省掉一次算术
 * （`__tests__/noScoreArithmetic.test.ts` 拦的正是那类写法），
 * 二来百分号的位置与数字形状交给 locale，不写死。
 */
const PERCENT = new Intl.NumberFormat("zh-CN", { style: "percent", maximumFractionDigits: 0 });

/**
 * 这一段进度的已完成比例，`null` 表示这一段没有可用的分母。
 *
 * 只有两段带分母（解压的字节、派生的作品数），其余五段是里程碑。分母为零时也返回
 * `null` 而不是 0——内核明写「`total == 0` 表示该步的总量未知，UI 应当显示不确定进度
 * 而不是 0%」（`crates/yunjian-core/src/derive.rs` 的 `DeriveProgress`），
 * 而派生第一步 `Scan` 发出来的正是 `total: 0`。显示成 0% 会让它看起来卡住了。
 */
function fraction(progress: CorpusProgress): number | null {
  if (progress.stage === "decompressing") {
    return progress.bytes_total > 0 ? progress.bytes_done / progress.bytes_total : null;
  }
  if (progress.stage === "deriving") {
    return progress.total > 0 ? progress.done / progress.total : null;
  }
  return null;
}

/** 这一段除标题之外还能说的一句话；`null` 表示无可补充。 */
function detail(progress: CorpusProgress): string | null {
  switch (progress.stage) {
    case "decompressing":
      return progress.bytes_total > 0
        ? `${mib(progress.bytes_done)} / ${mib(progress.bytes_total)} MiB`
        : `${mib(progress.bytes_done)} MiB`;
    case "deriving":
      return progress.total > 0
        ? `${progress.step}（${progress.done.toLocaleString("zh-CN")} / ${progress.total.toLocaleString("zh-CN")} 首）`
        : progress.step;
    case "verifying_archive":
      return `${mib(progress.bytes)} MiB`;
    case "derive_failed":
      // 派生失败之后内核仍会走到 `ready`（`corpus.rs` 的 `open_with_progress`），
      // 所以这是一句告知：语料能用，只是两字查询会退化。
      return `${progress.reason}；语料库仍可用，下次启动会重试`;
    default:
      return null;
  }
}

/** 字节数换成 MiB，保留一位小数。211 MiB 这个量级用字节读不出来。 */
function mib(bytes: number): string {
  return (bytes / 1_048_576).toFixed(1);
}

export default function CorpusPanel({ port }: CorpusPanelProps) {
  const [status, setStatus] = useState<CorpusStatus | null>(null);
  const [busy, setBusy] = useState(false);
  const [progress, setProgress] = useState<CorpusProgress | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let disposed = false;
    void port
      .corpusStatus()
      .then((next) => {
        if (!disposed) {
          setStatus(next);
        }
      })
      .catch((cause: unknown) => {
        if (!disposed) {
          setError(cause instanceof Error ? cause.message : String(cause));
        }
      });
    return () => {
      disposed = true;
    };
  }, [port]);

  const onFetch = useCallback(() => {
    setBusy(true);
    setError(null);
    // 清成 `null`，而进度块的出现只看 `busy`：第一条事件到达之前也得有话说，
    // 否则「已禁用的按钮 + 一言不发的面板」与「点了没反应」不可区分。那一段渲染成
    // 「正在准备……」，见下面 `busy` 那个分支。
    setProgress(null);
    void port
      .fetchCorpus((event) => {
        // `progress` 可合并（`contracts/operation.ts`）：只留最新一条，不累积列表。
        // 终止事件不写进进度——`failed` 的原因走 `error`，`done` 之后由事实表接管。
        if (event.type === "progress") {
          setProgress(event.payload);
        }
      })
      .then(setStatus)
      .catch((cause: unknown) => {
        setError(cause instanceof Error ? cause.message : String(cause));
      })
      .finally(() => {
        setBusy(false);
        // 收工后清掉：留着「正在解压」挨着已经渲染出来的事实表，读起来像还没跑完。
        setProgress(null);
      });
  }, [port]);

  return (
    <section className="settings-section" aria-label="语料库">
      <h2 className="settings-section__title">语料库</h2>

      {status === null && error === null && (
        <p className="settings-section__note" data-testid="corpus-loading">
          正在读取语料库状态……
        </p>
      )}

      {/* `!busy`：物化跑起来之后「尚未下载语料库」就成了一句陈旧的话——它挨着
          「正在解压语料库」一起显示，读起来自相矛盾。取用期间由进度块独占发言权。 */}
      {status !== null && status.kind === "absent" && !busy && (
        <>
          <p className="settings-section__note" data-testid="corpus-absent">
            尚未下载语料库。检索与阅读需要它；下载后检索结构会在首次启动时在本机派生。
          </p>
          {/* 时长与体积放在**按下之前**：这是一个十分钟的动作，看到代价再决定要不要开始
              比开始之后才被告知有用。它也因此不属于进度块——那一块只讲此刻在做什么。 */}
          <p className="settings-section__note" data-testid="corpus-cost">
            约 211 MiB，唐宋规模实测约十分钟，其中大部分时间花在建候选索引上。
          </p>
        </>
      )}

      {status !== null && status.kind === "ready" && (
        <dl className="settings-facts" data-testid="corpus-facts">
          <dt>语料版本</dt>
          <dd data-testid="corpus-version">{status.meta.corpus_version}</dd>
          <dt>收录作品</dt>
          {/* `toLocaleString` 给千分位：47 万这个量级不分组就读不出来。 */}
          <dd className="settings-facts__number" data-testid="corpus-poem-count">
            {status.meta.poem_count.toLocaleString("zh-CN")} 首
          </dd>
          <dt>schema 版本</dt>
          <dd className="settings-facts__number" data-testid="corpus-schema-version">
            {status.meta.schema_version}
          </dd>
          <dt>构建时间</dt>
          <dd data-testid="corpus-built-at">{status.meta.built_at}</dd>
          <dt>索引模式</dt>
          <dd data-testid="corpus-index-mode">{status.meta.index_detail_mode}</dd>
          <dt>派生索引</dt>
          <dd data-testid="corpus-derived-indexes">{status.meta.derived_indexes}</dd>
          <dt>随包范围</dt>
          <dd data-testid="corpus-shipped-scope">{status.meta.shipped_scope}</dd>
        </dl>
      )}

      <div className="settings-actions">
        <button
          type="button"
          className="settings-button"
          data-testid="fetch-corpus"
          disabled={busy}
          onClick={onFetch}>
          {status !== null && status.kind === "ready" ? "检查更新" : "下载语料库"}
        </button>
      </div>

      {busy && <CorpusProgressView progress={progress} />}

      {error !== null && (
        <p className="settings-list__refused" role="alert" data-testid="corpus-error">
          {error}
        </p>
      )}
    </section>
  );
}

/**
 * 物化过程中的进度块。
 *
 * # 为什么条与文字都要，而不是只留一个
 *
 * 有分母的两段（解压、派生）画条，其余五段没有分母只能报里程碑——如果只画条，那五段就是
 * 一根不动的空槽；如果只写字，唯一漫长的派生（实测 487.5 s）就看不出还要多久。所以两者
 * 都在，且**文字是那句权威**：条只是它的图形化。
 *
 * # 无障碍
 *
 * `role="progressbar"` + `aria-valuenow/min/max` 让读屏器报出比例；没有分母时**刻意不给**
 * `aria-valuenow`，那正是 ARIA 表达「不确定进度」的方式，填一个 0 会被读成「0%，卡住了」。
 * 整块带 `aria-live="polite"`，于是阶段推进会被播报而不打断用户。
 */
function CorpusProgressView({ progress }: { progress: CorpusProgress | null }) {
  const ratio = progress === null ? null : fraction(progress);
  const line = progress === null ? null : detail(progress);
  return (
    <div className="corpus-progress" aria-live="polite" data-testid="corpus-progress">
      <p className="corpus-progress__stage">
        <span data-testid="corpus-progress-stage">
          {progress === null ? "正在准备……" : CORPUS_PROGRESS_LABEL[progress.stage]}
        </span>
        {ratio !== null && (
          <span className="corpus-progress__percent" data-testid="corpus-progress-percent">
            {PERCENT.format(ratio)}
          </span>
        )}
      </p>
      <div
        className="corpus-progress__track"
        role="progressbar"
        aria-label="语料库物化进度"
        aria-valuemin={0}
        aria-valuemax={1}
        {...(ratio === null
          ? {}
          : { "aria-valuenow": ratio, "aria-valuetext": PERCENT.format(ratio) })}>
        <div
          className={
            ratio === null
              ? "corpus-progress__bar corpus-progress__bar--indeterminate"
              : "corpus-progress__bar"
          }
          // 比例以自定义属性交给 CSS，由那边 `calc()` 换成轨道宽度。
          // 前端不在 TypeScript 里做比例换算：`__tests__/noScoreArithmetic.test.ts`
          // 拦的就是这类写法，而那条门禁的边界（分数权威只在内核）值得连带守住。
          style={
            ratio === null ? undefined : ({ "--corpus-progress-ratio": ratio } as CSSProperties)
          }
        />
      </div>
      {line !== null && (
        <p className="corpus-progress__detail" data-testid="corpus-progress-detail">
          {line}
        </p>
      )}
    </div>
  );
}
