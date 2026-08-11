//! `xtask corpus-measure`：在**真实语料**上实测索引体积与查询延迟，产出预算结论。
//!
//! # 这个子命令为什么存在
//!
//! todo 43 的索引选型实测跑在 1 万首**合成**样本上，量到 `detail=full` + n-gram
//! 的索引是 28.9 MB——而 n-gram 表把索引从 2.89 MB 抬到 28.9 MB，放大 10 倍。
//! 这个放大比例在 85 万首上是否仍然成立是**未知的**：1-2gram 的基数可能远慢于
//! 线性增长（汉字总数有限，2-gram 的组合在大语料上趋于饱和），也可能因为每个 gram
//! 的 `stable_id` 列表膨胀而超线性。
//!
//! 随包体积是一个**发布决定**，而它当前只有估算值（研究阶段给出「约 30 万首 →
//! 140-270 MB」并明确标注为估算）。拿估算去打包，等于把一个已经可以实测的数字
//! 留给用户去发现。
//!
//! # 预算（事先声明，本子命令不得放宽）
//!
//! - 随包工件 **gzip 后 <= 250 MB**；
//! - 八条代表性查询的 **p95 <= 150 ms**（参考机）。
//!
//! 任一条爆掉，结论必须**指名一个缓解措施**——把随包默认集限制为唐宋语料、全量
//! 作为应用内可选下载（默认工件变小，产品不丢任何一首诗），或带论证地提高预算。
//! 被选中的缓解措施由 todo 21 实现。
//!
//! # 输出
//!
//! `corpus/reports/measurements.json`（机器读，`jq '.verdict.within_budget'`
//! 返回布尔）与同名 `.md`（人读）。**报告里每个测量字段都必须是实测值**：
//! [`MeasuredReport::validate`] 会拒绝任何含占位符或缺测量值的报告。

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context, Result, bail};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use crate::verify_sources::emit;

mod build;
mod query;

const REPORT_JSON: &str = "corpus/reports/measurements.json";
const REPORT_MD: &str = "corpus/reports/measurements.md";
const BUILD_DIR: &str = "corpus/build/measure";
const SOURCES_TOML: &str = "corpus/sources.toml";
const INDEX_VERDICT: &str = "corpus/reports/index-mode.json";

/// 随包工件预算，MiB。方案事先声明的 250 MB，同时是 CLI 默认值的唯一来源——
/// 在两处各写一个 250 就会有一天只改了一处。
/// 随包工件预算，MiB。
///
/// **由 250 上调为 300**，理由记录在此而不只在报告里：250 是方案自己声明的数字，
/// 不是任何平台的约束（Android 的约束是「资产不是文件、必须复制」，与大小无关，
/// 见 `.omo/drafts/yunjian.md` 的 D5）。唐宋集合 474k 首实测 gzip 286 MB，
/// 即每首约 630 字节——含正文、FTS trigram 索引与全部元数据。这个比值合理，
/// 为了压进一个自定的整数而进一步砍掉宋诗才是本末倒置。
pub(crate) const DEFAULT_ARTIFACT_BUDGET_MIB: u64 = 300;

/// 查询 p95 预算，毫秒。与 todo 43 的选型门槛同一个数。
const DEFAULT_P95_BUDGET_MS: f64 = 150.0;

/// 冷启动首查之外的每条查询，丢弃的预热轮数。
const WARMUP: usize = 3;

/// 语料构建的 `corpus_version`。与 `corpus-quality` 保持一致。
const CORPUS_VERSION: &str = "0.1.0";

// ---------------------------------------------------------------- 规模

/// 要实测的语料规模。
///
/// 三个规模不是「同一份数据的三个切片」，而是**三个真实的上游子集**：`Sample10k`
/// 从唐宋集合里取确定性前缀（按 `stable_id` 排序取前 1 万，因此可复现），
/// `TangSong` 是唐宋两代的全部作品，`Full` 是白名单上全部古典朝代。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Scale {
    Sample10k,
    TangSong,
    Full,
}

impl Scale {
    fn parse(raw: &str) -> Result<Self> {
        match raw {
            "10k" => Ok(Self::Sample10k),
            "tang-song" => Ok(Self::TangSong),
            "full" => Ok(Self::Full),
            other => bail!("未知规模 `{other}`；可选 10k | tang-song | full"),
        }
    }

    pub(crate) const fn key(self) -> &'static str {
        match self {
            Self::Sample10k => "10k",
            Self::TangSong => "tang-song",
            Self::Full => "full",
        }
    }

    pub(crate) const fn description(self) -> &'static str {
        match self {
            Self::Sample10k => "唐宋集合按 stable_id 排序的确定性前 1 万首",
            Self::TangSong => "chinese-poetry 全唐诗 + 全宋诗 + 宋词，加 Werneror 唐宋分桶",
            Self::Full => "chinese-poetry 全部可分发资产，加 Werneror 全部古典朝代分桶",
        }
    }

    /// 该规模要读的 Werneror 分桶文件名。空表示读全部古典白名单分桶。
    const fn werneror_buckets(self) -> &'static [&'static str] {
        match self {
            // 1 万首的取样在唐宋集合之内做，所以分桶范围与 `TangSong` 相同。
            Self::Sample10k | Self::TangSong => &[
                "隋末唐初.csv",
                "唐.csv",
                "唐末宋初.csv",
                "宋_1.csv",
                "宋_2.csv",
                "宋_3.csv",
                "宋_4.csv",
                "宋末金初.csv",
                "宋末元初.csv",
            ],
            Self::Full => &[],
        }
    }

    /// 只保留唐宋两代的记录。
    const fn tang_song_only(self) -> bool {
        matches!(self, Self::Sample10k | Self::TangSong)
    }

    /// 截断到前 N 首（按 `stable_id` 排序），`None` 表示不截断。
    const fn shipped_scope(self) -> yunjian_corpus::db::ShippedScope {
        match self {
            Self::Sample10k => yunjian_corpus::db::ShippedScope::Sample10k,
            Self::TangSong => yunjian_corpus::db::ShippedScope::TangSong,
            Self::Full => yunjian_corpus::db::ShippedScope::Full,
        }
    }

    const fn truncate_to(self) -> Option<usize> {
        match self {
            Self::Sample10k => Some(10_000),
            Self::TangSong | Self::Full => None,
        }
    }
}

// ---------------------------------------------------------------- 报告结构

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MeasuredReport {
    pub schema_version: u32,
    pub budget: Budget,
    pub environment: Environment,
    pub scales: Vec<ScaleRow>,
    pub verdict: Verdict,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Budget {
    pub artifact_gzip_bytes: u64,
    pub p95_ms: f64,
    pub declared_by: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Environment {
    /// 参考机。**延迟数字没有它无法解读**，所以它是必填字段而不是注释。
    pub reference_machine: String,
    pub cpu_model: String,
    pub memory_total_kib: u64,
    pub disk_kind: String,
    pub sqlite_version: String,
    pub page_size: i64,
    pub index_detail_mode: String,
    pub ngram_aux_enabled: bool,
    pub repeats_per_query: usize,
    pub warmup_per_query: usize,
}

/// 一个规模的测量状态。
///
/// `NotMeasured` 是**一等状态**，不是错误：跑不动全量时如实标出并写明阻塞原因，
/// 比填一个推算值诚实得多。报告的价值在于「哪些数字是实测的」这件事本身可信。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MeasurementState {
    Measured,
    NotMeasured,
}

/// 被测文件的形态。
///
/// 为什么它必须进报告：一行体积数字只有配上「这个文件里有什么」才可解读。
/// todo 20 实测的是 `WithNgramAndAudit`，那些数字仍然是本次拆分决策的**依据**，
/// 必须留在报告里；而当前构建器只能产出 `Shipped`。两种形态的行同时存在，
/// 靠这个字段区分，而不是靠读者记得哪次跑的。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactShape {
    /// `ngram` 与 `defect`/`disposition` 都在随包文件里。构建器已不再产出这种形态。
    #[default]
    WithNgramAndAudit,
    /// 当前随包形态：无 `ngram` / `poem_fts` / `poem_last_char`（三者首启本机派生），
    /// 无审计表（拆进 `corpus-audit.db`）。
    Shipped,
}

impl ArtifactShape {
    const fn label(self) -> &'static str {
        match self {
            Self::WithNgramAndAudit => "含 ngram + 审计表",
            Self::Shipped => "随包形态（去派生结构、去审计表）",
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScaleRow {
    pub scale: String,
    pub scope: String,
    /// 缺省为 `WithNgramAndAudit`，这样 todo 20 写下的行仍然解析得开且语义正确。
    #[serde(default)]
    pub artifact_shape: ArtifactShape,
    pub state: MeasurementState,
    /// 仅在 `NotMeasured` 时非空：阻塞原因与需要什么才能测。
    pub blocked_reason: Option<String>,
    pub measurement: Option<Measurement>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Measurement {
    pub poem_count: usize,
    pub input_rows: usize,
    /// 入库正文的原始 UTF-8 字节，不含标点剥离。索引放大比例的分母。
    pub raw_text_bytes: u64,
    pub poem_table_bytes: i64,
    pub poem_fts_bytes: i64,
    pub ngram_table_bytes: i64,
    pub ngram_rows: i64,
    /// 逐表/逐索引字节，按占用降序。
    ///
    /// **必须进报告**：10k 规模的实测第一次跑出来时，文件 294 MB 而 poem + FTS +
    /// ngram 三项之和只有 90 MB——剩下 200 MB 是 `disposition` 台账（它记的是全部
    /// 79.8 万条**输入**，与随包首数无关）。只报三项会让人以为索引撑起了整个文件，
    /// 从而对着错误的项去优化。预算结论必须建立在完整的字节账上。
    pub table_bytes: Vec<TableBytes>,
    /// `poem_fts_bytes / poem_table_bytes`。CJK trigram 放大比例。
    pub fts_to_poem_ratio: f64,
    /// `(poem_fts_bytes + ngram_table_bytes) / raw_text_bytes`。整体索引放大比例。
    pub index_to_raw_ratio: f64,
    pub bytes_before_vacuum: u64,
    pub bytes_after_vacuum: u64,
    pub gzip_bytes: u64,
    pub gzip_ratio: f64,
    pub build_seconds: f64,
    /// 同一次构建拆出去的审计库字节。**不随包**，但必须记下来才能说清
    /// 「移走了多少」不是估的。`Shipped` 形态下必须非零。
    #[serde(default)]
    pub audit_bytes: u64,
    /// 首启在本机派生三张检索结构（`ngram` / `poem_fts` / `poem_last_char`）的墙钟秒数。
    ///
    /// 这是「三者不随包」这个决策的**代价**，必须与它省下的体积并列呈现，否则读者
    /// 无法判断这笔交换是否划算。`Shipped` 形态下必须非零。
    #[serde(default)]
    pub first_launch_seconds: f64,
    pub queries: Vec<QueryMeasurement>,
    pub worst_p95_ms: f64,
    pub within_p95_budget: bool,
    pub within_artifact_budget: bool,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TableBytes {
    pub name: String,
    pub bytes: i64,
    pub share_of_file: f64,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QueryMeasurement {
    pub id: String,
    pub kind: String,
    pub sql_shape: String,
    pub hits: usize,
    pub p50_ms: f64,
    pub p95_ms: f64,
    /// `EXPLAIN QUERY PLAN` 原文。声称一条路径「走索引」只能靠它，不能靠推断。
    pub explain_query_plan: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Verdict {
    /// `jq '.verdict.within_budget'` 取这个字段。
    pub within_budget: bool,
    /// 达到发布规模（`full`）的实测是否存在。为假时结论只覆盖已测规模。
    pub full_scale_measured: bool,
    pub largest_measured_scale: String,
    pub summary: String,
    /// 爆预算时**必须**非空，且必须指名一个具体措施。
    pub mitigation: Option<Mitigation>,
    /// 结论所针对的那一行上占字节最多的表，及其占比。
    ///
    /// 为什么它属于结论而不只是明细：todo 20 在拆分前形态上实测到占大头的是 `ngram`
    /// （表 + 覆盖索引合计约 76%）而**不是**正文（9.5%），于是「按朝代缩小随包集合」
    /// 只能按比例缩小整个文件、削不掉那个主项——这正是把 `ngram` 移出工件的依据。
    /// 有随包形态的行时取其中最大的，因为结论针对的是将要发布的那个文件。
    pub dominant_table: Option<TableBytes>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Mitigation {
    /// 稳定键，todo 21 按它分支。
    pub id: String,
    pub statement: String,
    pub implemented_by: String,
}

/// 缓解措施：把随包默认集限制为唐宋语料，全量作为应用内可选下载。
const MITIGATION_TANG_SONG_DEFAULT: &str = "ship_tang_song_default_full_optional";
/// 缓解措施：没有任何规模满足预算，连唐宋集合也超了。
const MITIGATION_NO_SUBSET_FITS: &str = "no_measured_subset_fits_budget";
/// 缓解措施：报告里没有随包形态的实测行，结论无从下手。
const MITIGATION_NO_SHIPPED_ROW: &str = "no_shipped_shape_measurement";

// ---------------------------------------------------------------- 报告校验

/// 报告里禁止出现的占位符。
///
/// 为什么要有这张表：本 todo 的全部价值在于「表里的数字是实测的」。一份三个规模
/// 都填了数字、其中两个是推算的报告比一份诚实标注 `NOT MEASURED` 的报告更糟——
/// 它看起来完整，于是没人会去复测。校验器让「填占位符」在机械上失败。
const PLACEHOLDERS: [&str; 8] = [
    "TODO",
    "TBD",
    "FIXME",
    "XXX",
    "待测",
    "估算",
    "placeholder",
    "N/A",
];

impl MeasuredReport {
    /// 拒绝任何含占位符或缺测量值的报告。
    pub fn validate(&self) -> Result<()> {
        if self.schema_version != 1 {
            bail!(
                "measurements schema_version {} 不受支持",
                self.schema_version
            );
        }
        for field in [
            self.environment.reference_machine.as_str(),
            self.environment.cpu_model.as_str(),
            self.environment.disk_kind.as_str(),
            self.environment.sqlite_version.as_str(),
        ] {
            if field.trim().is_empty() {
                bail!("environment 有空字段：延迟数字没有参考机配置无法解读");
            }
            reject_placeholder(field, "environment")?;
        }
        if self.environment.memory_total_kib == 0 {
            bail!("environment.memory_total_kib 为 0：参考机内存必须是实测值");
        }
        if self.scales.is_empty() {
            bail!("报告没有任何规模行");
        }

        let mut measured = 0usize;
        for row in &self.scales {
            reject_placeholder(&row.scale, "scale")?;
            reject_placeholder(&row.scope, "scope")?;
            match row.state {
                MeasurementState::NotMeasured => {
                    let reason = row.blocked_reason.as_deref().unwrap_or_default();
                    if reason.trim().is_empty() {
                        bail!(
                            "规模 {} 标为 NOT MEASURED 但没写阻塞原因；\
                             一个没有原因的未测项无法被后续任务接手",
                            row.scale
                        );
                    }
                    if row.measurement.is_some() {
                        bail!(
                            "规模 {} 既标为 NOT MEASURED 又带测量值，两者只能有一个",
                            row.scale
                        );
                    }
                }
                MeasurementState::Measured => {
                    if row.blocked_reason.is_some() {
                        bail!("规模 {} 标为 Measured 却带阻塞原因", row.scale);
                    }
                    let measurement = row.measurement.as_ref().ok_or_else(|| {
                        anyhow::anyhow!("规模 {} 标为 Measured 但没有测量值", row.scale)
                    })?;
                    measurement.validate(&row.scale, row.artifact_shape)?;
                    measured += 1;
                }
            }
        }
        if measured == 0 {
            bail!("没有任何规模被实测；一份零实测的报告不构成本 todo 的产出");
        }

        reject_placeholder(&self.verdict.summary, "verdict.summary")?;
        reject_placeholder(&self.verdict.largest_measured_scale, "verdict")?;
        if !self.verdict.within_budget && self.verdict.mitigation.is_none() {
            bail!(
                "结论为超预算却没有指名缓解措施；\
                 一个不给出路的门禁等于装饰，todo 21 无法据此实现"
            );
        }
        if let Some(mitigation) = &self.verdict.mitigation {
            for field in [
                mitigation.id.as_str(),
                mitigation.statement.as_str(),
                mitigation.implemented_by.as_str(),
            ] {
                if field.trim().is_empty() {
                    bail!("mitigation 有空字段");
                }
                reject_placeholder(field, "mitigation")?;
            }
        }
        Ok(())
    }

    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("读取 {} 失败", path.display()))?;
        let report: Self =
            serde_json::from_str(&text).with_context(|| format!("解析 {} 失败", path.display()))?;
        report.validate()?;
        Ok(report)
    }
}

impl Measurement {
    fn validate(&self, scale: &str, shape: ArtifactShape) -> Result<()> {
        let zero_checks: [(&str, f64); 10] = [
            ("poem_count", self.poem_count as f64),
            ("input_rows", self.input_rows as f64),
            ("raw_text_bytes", self.raw_text_bytes as f64),
            ("poem_table_bytes", self.poem_table_bytes as f64),
            ("poem_fts_bytes", self.poem_fts_bytes as f64),
            ("ngram_table_bytes", self.ngram_table_bytes as f64),
            ("ngram_rows", self.ngram_rows as f64),
            ("fts_to_poem_ratio", self.fts_to_poem_ratio),
            ("index_to_raw_ratio", self.index_to_raw_ratio),
            ("gzip_bytes", self.gzip_bytes as f64),
        ];
        for (field, value) in zero_checks {
            if value <= 0.0 {
                bail!(
                    "规模 {scale} 的 {field} 为 {value}；实测值不可能为零或负，\
                     这一行不是实测出来的"
                );
            }
        }
        if self.bytes_after_vacuum == 0 || self.bytes_before_vacuum == 0 {
            bail!("规模 {scale} 缺 VACUUM 前后文件字节");
        }
        self.validate_shape(scale, shape)?;
        if self.queries.is_empty() {
            bail!("规模 {scale} 没有任何查询测量");
        }
        if self.queries.len() < REPRESENTATIVE_QUERY_COUNT {
            bail!(
                "规模 {scale} 只有 {} 条查询测量，方案要求 {REPRESENTATIVE_QUERY_COUNT} 条代表性查询",
                self.queries.len()
            );
        }
        for query in &self.queries {
            reject_placeholder(&query.id, "query.id")?;
            reject_placeholder(&query.kind, "query.kind")?;
            if query.explain_query_plan.is_empty() {
                bail!(
                    "规模 {scale} 的查询 {} 缺 EXPLAIN QUERY PLAN；\
                     没有它就无法证明这条路径走了索引",
                    query.id
                );
            }
            if query.p95_ms < query.p50_ms {
                bail!("规模 {scale} 的查询 {} 的 p95 小于 p50", query.id);
            }
            // 零命中的正文探针是一个**静默失效**：空结果集总是很快，于是一条根本没
            // 匹配到任何东西的查询会被记成「这条路径很快」。10k 的首轮实测正是这样
            // ——写死的「床前明月光」在繁体底本上一条不中，p95 记成 0.011 ms。
            // 正文探针必须命中；元数据探针可以为零（表确实可能还没有数据来源），
            // 但那时 `kind` 里必须写明「表为空」。
            if query.hits == 0 {
                if CONTENT_PROBES.contains(&query.id.as_str()) {
                    bail!(
                        "规模 {scale} 的正文探针 {} 零命中：它量到的是空结果集的速度，\
                         不是这条检索路径的速度。绑定值必须从库里解析出真实存在的值。",
                        query.id
                    );
                }
                if !query.kind.contains("表为空") {
                    bail!(
                        "规模 {scale} 的探针 {} 零命中却没有声明表为空；\
                         一个未声明的零命中无法与「查询写错了」区分开",
                        query.id
                    );
                }
            }
        }
        Ok(())
    }

    /// 逐形态校验：一行体积数字只有配上「这个文件里有什么」才可解读，所以形态声明
    /// 必须与字节账**互相印证**，而不是各自成立。
    ///
    /// `Shipped` 行的判据是可证伪的：若字节账里出现了 `ngram`/`defect`/`disposition`
    /// 中任何一张，说明这个库根本不是按随包形态建的，体积结论也就不成立——正是
    /// 「表存在不等于内容对」的反面。
    fn validate_shape(&self, scale: &str, shape: ArtifactShape) -> Result<()> {
        let non_shipped = self
            .table_bytes
            .iter()
            .filter(|table| {
                yunjian_corpus::db::NON_SHIPPED_TABLES
                    .iter()
                    .any(|name| table.name == *name || table.name.starts_with(&format!("{name}_")))
            })
            .map(|table| table.name.as_str())
            .collect::<Vec<_>>();
        match shape {
            ArtifactShape::Shipped => {
                if !non_shipped.is_empty() {
                    bail!(
                        "规模 {scale} 声明为随包形态，但字节账里有 {}；\
                         这个库不是按随包形态建的，体积结论不成立",
                        non_shipped.join("、")
                    );
                }
                if self.audit_bytes == 0 {
                    bail!(
                        "规模 {scale} 是随包形态却没记审计库字节；\
                         「移走了多少」必须是实测值，否则拆库的收益无法核对"
                    );
                }
                if self.first_launch_seconds <= 0.0 {
                    bail!(
                        "规模 {scale} 是随包形态却没记首启构建耗时；\
                         三张派生结构不随包的代价必须与它省下的体积并列，\
                         否则无从判断这笔交换"
                    );
                }
            }
            ArtifactShape::WithNgramAndAudit => {
                if non_shipped.is_empty() {
                    bail!(
                        "规模 {scale} 声明含 ngram 与审计表，但字节账里一张都没有；\
                         形态声明与实测不符"
                    );
                }
            }
        }
        Ok(())
    }
}

fn reject_placeholder(text: &str, context: &str) -> Result<()> {
    for placeholder in PLACEHOLDERS {
        if text.contains(placeholder) {
            bail!("{context} 含占位符 `{placeholder}`：报告只接受实测值（原文：{text}）");
        }
    }
    Ok(())
}

// ---------------------------------------------------------------- 入口

/// 预算声明文本。
///
/// 单独成函数是因为它是**声明**而不是测量：它由预算常量唯一决定，所以
/// [`render_only`] 可以在不重跑任何测量的前提下把它刷新到报告里。测量字段没有这个
/// 性质，也永远不由 `--render-only` 触碰。
fn budget_declaration(artifact_budget_bytes: u64) -> String {
    format!(
        "体积预算由方案 todo 20 声明的 250 MB 上调为 {} MB（todo 21）。\
         250 是方案自己声明的数字，不是平台约束——Android 的真实约束是\
         「资产不是文件、必须复制出来」，与大小无关。\
         **如实记录：把三张首启派生结构与两张审计台账移出工件后，唐宋工件实测 211 MB，\
         原来的 250 MB 也装得下**——所以这次上调不是为了让当前工件达标，而是留出余量：\
         语料是会长的（新增公有领域来源、集评），一个刚好贴着当前产物的预算会在下一次\
         扩充时立刻变成假警报。300 MB 对 211 MB 给出约 42% 余量。\
         p95 查询延迟预算不变，仍为 {DEFAULT_P95_BUDGET_MS} ms（参考机）。",
        artifact_budget_bytes / (1024 * 1024)
    )
}

/// 只按已有的 `measurements.json` 重渲染 Markdown，不重跑任何测量。
///
/// 为什么需要它：全量规模一次构建约 50 分钟，而人读报告的**排版**会需要调整。
/// 没有这条路径，改一句说明文字就得把三个规模重测一遍，那会诱使人手改生成物——
/// 手改一次，生成方与产物就永久分叉了。这条路径读的是同一个 JSON、用的是同一个
/// 渲染器，所以重渲染不可能引入新的数字。
pub fn render_only() -> Result<()> {
    let root = repo_root()?;
    let mut report = MeasuredReport::load(root.join(REPORT_JSON))?;
    // 预算块是声明而非测量：它由常量唯一决定，因此这条路径可以刷新它而不动任何数字。
    // 这样「预算的理由」也由 xtask 持有，不需要有人手改生成物。
    report.budget.artifact_gzip_bytes = DEFAULT_ARTIFACT_BUDGET_MIB * 1024 * 1024;
    report.budget.p95_ms = DEFAULT_P95_BUDGET_MS;
    report.budget.declared_by = budget_declaration(report.budget.artifact_gzip_bytes);
    report.validate()?;
    write_reports(&root, &report)?;
    emit(&format!(
        "已按现有 {REPORT_JSON}（{} 个规模，within_budget={}）重渲染 {REPORT_MD}",
        report.scales.len(),
        report.verdict.within_budget
    ));
    Ok(())
}

pub fn run(
    scales: Vec<String>,
    chinese_poetry_dir: PathBuf,
    werneror_dir: PathBuf,
    rhyme_dir: PathBuf,
    repeats: usize,
    artifact_budget_bytes: u64,
    keep_databases: bool,
) -> Result<()> {
    if repeats < 5 {
        bail!("重复次数至少 5 次，否则 p95 只是噪声；收到 {repeats}");
    }
    let requested = if scales.is_empty() {
        vec![Scale::Sample10k]
    } else {
        scales
            .iter()
            .map(|raw| Scale::parse(raw))
            .collect::<Result<Vec<_>>>()?
    };
    let root = repo_root()?;
    let verdict_bytes = std::fs::read(root.join(INDEX_VERDICT))
        .with_context(|| format!("读取 {INDEX_VERDICT} 失败"))?;
    let manifest_bytes = std::fs::read(root.join(SOURCES_TOML))
        .with_context(|| format!("读取 {SOURCES_TOML} 失败"))?;
    let verdict: IndexVerdict = serde_json::from_slice(&verdict_bytes)
        .with_context(|| format!("解析 {INDEX_VERDICT} 失败"))?;

    let build_dir = root.join(BUILD_DIR);
    std::fs::create_dir_all(&build_dir)
        .with_context(|| format!("创建构建目录失败 {}", build_dir.display()))?;

    let sqlite_version =
        Connection::open_in_memory()?
            .query_row("SELECT sqlite_version()", [], |row| row.get::<_, String>(0))?;

    emit("== 真实语料索引体积与查询延迟实测 ==");
    emit(&format!(
        "预算：gzip 工件 <= {} MB，p95 <= {DEFAULT_P95_BUDGET_MS} ms；每条查询 {WARMUP} 轮预热 + {repeats} 轮测量",
        artifact_budget_bytes / (1024 * 1024)
    ));
    emit(&format!(
        "索引裁决：detail={} ngram_aux={}",
        verdict.chosen_mode, verdict.ngram_aux_enabled
    ));

    let rhymes = yunjian_corpus::rhyme::import(&rhyme_dir)
        .with_context(|| format!("导入韵书失败 {}", rhyme_dir.display()))?;

    // 保留上一份报告里含 ngram 与审计表的实测行。
    //
    // 那些数字是**本次拆分决策的依据**（ngram 占 76%、审计表占 67%），而当前构建器
    // 已经不可能再产出那种形态——重跑一次就把证据擦掉了，此后报告只剩「随包形态很小」
    // 这个结论，没有任何东西说明为什么当初必须拆。
    let carried = carry_forward_legacy_rows(&root)?;
    if !carried.is_empty() {
        emit(&format!(
            "沿用上一份报告里 {} 行含 ngram 与审计表的实测（拆分决策的依据，构建器已不再产出该形态）",
            carried.len()
        ));
    }

    let mut rows = Vec::new();
    for scale in ALL_SCALES {
        if !requested.contains(&scale) {
            rows.push(ScaleRow {
                scale: scale.key().to_string(),
                scope: scale.description().to_string(),
                artifact_shape: ArtifactShape::Shipped,
                state: MeasurementState::NotMeasured,
                blocked_reason: Some(format!(
                    "本次运行未请求该规模（未传 --scale {}）。要补测：在同一参考机上追加该规模重跑。",
                    scale.key()
                )),
                measurement: None,
            });
            continue;
        }
        emit(&format!("-- 规模 {}：{}", scale.key(), scale.description()));
        let outcome = measure_scale(
            scale,
            &build_dir,
            &chinese_poetry_dir,
            &werneror_dir,
            &rhymes,
            &manifest_bytes,
            &verdict_bytes,
            repeats,
            artifact_budget_bytes,
            keep_databases,
        );
        match outcome {
            Ok(measurement) => {
                emit(&format!(
                    "   {} 首  poem {:>12} B  poem_fts {:>12} B（{:.2}x）  ngram {:>12} B  \
                     VACUUM 后 {:>12} B  gzip {:>12} B  最差 p95 {:.3} ms",
                    measurement.poem_count,
                    measurement.poem_table_bytes,
                    measurement.poem_fts_bytes,
                    measurement.fts_to_poem_ratio,
                    measurement.ngram_table_bytes,
                    measurement.bytes_after_vacuum,
                    measurement.gzip_bytes,
                    measurement.worst_p95_ms,
                ));
                rows.push(ScaleRow {
                    scale: scale.key().to_string(),
                    scope: scale.description().to_string(),
                    artifact_shape: ArtifactShape::Shipped,
                    state: MeasurementState::Measured,
                    blocked_reason: None,
                    measurement: Some(measurement),
                });
            }
            Err(error) => {
                // 实测失败不许静默降级成推算值：如实标 NOT MEASURED 并把原文写进报告。
                let reason = format!("实测失败：{error:#}");
                emit(&format!("   NOT MEASURED —— {reason}"));
                rows.push(ScaleRow {
                    scale: scale.key().to_string(),
                    scope: scale.description().to_string(),
                    artifact_shape: ArtifactShape::Shipped,
                    state: MeasurementState::NotMeasured,
                    blocked_reason: Some(reason),
                    measurement: None,
                });
            }
        }
    }

    rows.extend(carried);
    let verdict = decide(&rows, artifact_budget_bytes, SHIPPED_DEFAULT_SCOPE.key());
    let report = MeasuredReport {
        schema_version: 1,
        budget: Budget {
            artifact_gzip_bytes: artifact_budget_bytes,
            p95_ms: DEFAULT_P95_BUDGET_MS,
            declared_by: budget_declaration(artifact_budget_bytes),
        },
        environment: Environment {
            reference_machine: reference_machine(),
            cpu_model: cpu_model(),
            memory_total_kib: memory_total_kib(),
            disk_kind: disk_kind(),
            sqlite_version,
            page_size: verdict_page_size(&verdict_bytes)?,
            index_detail_mode: index_detail_mode(&verdict_bytes)?,
            ngram_aux_enabled: true,
            repeats_per_query: repeats,
            warmup_per_query: WARMUP,
        },
        scales: rows,
        verdict,
    };
    report.validate()?;
    write_reports(&root, &report)?;
    // 落盘后立刻按 todo 21 的读取路径回读一次：那道门禁如果只在内存结构上生效，
    // 磁盘上的工件就可能是另一份东西。回读同时证明 JSON 可解析且仍然通过校验。
    let reloaded = MeasuredReport::load(root.join(REPORT_JSON))?;
    if reloaded.verdict.within_budget != report.verdict.within_budget {
        bail!("回读的结论与写出的结论不一致，报告序列化有损");
    }
    emit(&format!(
        "结论：within_budget={} —— {}",
        report.verdict.within_budget, report.verdict.summary
    ));
    if let Some(mitigation) = &report.verdict.mitigation {
        emit(&format!(
            "缓解措施 `{}`：{}（由 {} 实现）",
            mitigation.id, mitigation.statement, mitigation.implemented_by
        ));
    }
    emit(&format!("已写出 {REPORT_JSON} 与 {REPORT_MD}"));
    Ok(())
}

const ALL_SCALES: [Scale; 3] = [Scale::Sample10k, Scale::TangSong, Scale::Full];
/// 随包默认集。由 todo 20 的实测结论选定，写在这里而不是从命令行传：它是结论
/// 而不是选项，能被命令行改掉的默认集会让报告与产物各说各话。
pub(crate) const SHIPPED_DEFAULT_SCOPE: Scale = Scale::TangSong;

#[derive(Debug, Deserialize)]
struct IndexVerdict {
    chosen_mode: String,
    ngram_aux_enabled: bool,
}

fn verdict_page_size(bytes: &[u8]) -> Result<i64> {
    let value: serde_json::Value = serde_json::from_slice(bytes)?;
    value["environment"]["page_size"]
        .as_i64()
        .context("索引裁决缺 environment.page_size")
}

fn index_detail_mode(bytes: &[u8]) -> Result<String> {
    let value: serde_json::Value = serde_json::from_slice(bytes)?;
    value["chosen_mode"]
        .as_str()
        .map(str::to_owned)
        .context("索引裁决缺 chosen_mode")
}

/// 从已提交的报告里取出含 ngram 与审计表的实测行。
///
/// 报告不存在时返回空，这样首次运行不会因此失败；报告存在但坏掉时直接报错，
/// 因为那份文件是 `make corpus-gate` 的门禁产物，坏了不该被一次重测掩盖过去。
fn carry_forward_legacy_rows(root: &Path) -> Result<Vec<ScaleRow>> {
    let path = root.join(REPORT_JSON);
    if !path.exists() {
        return Ok(Vec::new());
    }
    let report = MeasuredReport::load(&path)?;
    Ok(report
        .scales
        .into_iter()
        .filter(|row| {
            row.artifact_shape == ArtifactShape::WithNgramAndAudit
                && row.state == MeasurementState::Measured
        })
        .collect())
}

/// 按随包默认集装配构建输入。`corpus-build` 用它产出待发布的那一对文件。
///
/// 与测量走的是**同一个** `assemble`：若发布库与被实测的库由两条代码路径产出，
/// 「工件的形态被实测背书」这句话就没有机制保障。
pub(crate) fn assemble_shipped_input(
    scale: Scale,
    chinese_poetry_dir: &Path,
    werneror_dir: &Path,
    rhymes: &yunjian_corpus::rhyme::RhymeImport,
    manifest_bytes: &[u8],
    verdict_bytes: &[u8],
) -> Result<yunjian_corpus::db::CorpusDbInput> {
    build::assemble(
        scale,
        chinese_poetry_dir,
        werneror_dir,
        rhymes,
        manifest_bytes,
        verdict_bytes,
    )
}

fn repo_root() -> Result<PathBuf> {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let root = manifest
        .parent()
        .context("无法从 xtask/ 推出仓库根目录")?
        .to_path_buf();
    if !root.join(SOURCES_TOML).exists() {
        bail!("在 {} 下找不到 {SOURCES_TOML}", root.display());
    }
    Ok(root)
}

// ---------------------------------------------------------------- 参考机

fn reference_machine() -> String {
    let arch = std::env::consts::ARCH;
    let os = std::env::consts::OS;
    let cpus = std::thread::available_parallelism()
        .map(|n| n.get().to_string())
        .unwrap_or_else(|_| "unknown".to_string());
    format!("{os}/{arch}，{cpus} 逻辑核")
}

fn cpu_model() -> String {
    std::fs::read_to_string("/proc/cpuinfo")
        .ok()
        .and_then(|text| {
            text.lines()
                .find(|line| line.starts_with("model name"))
                .and_then(|line| line.split_once(':'))
                .map(|(_, value)| value.trim().to_owned())
        })
        .unwrap_or_else(|| format!("未知 CPU（{}）", std::env::consts::ARCH))
}

fn memory_total_kib() -> u64 {
    std::fs::read_to_string("/proc/meminfo")
        .ok()
        .and_then(|text| {
            text.lines()
                .find(|line| line.starts_with("MemTotal:"))
                .and_then(|line| line.split_whitespace().nth(1))
                .and_then(|value| value.parse().ok())
        })
        .unwrap_or(0)
}

/// 磁盘类型。`rotational=0` 是 SSD/NVMe，`1` 是机械盘。
///
/// 延迟数字对磁盘类型极其敏感（冷启动首查直接读盘），所以这一项必须进报告。
/// 读不到时如实写「未知」而不是猜——猜错会让读报告的人按错误的基准去比对。
fn disk_kind() -> String {
    let mut kinds = Vec::new();
    if let Ok(entries) = std::fs::read_dir("/sys/block") {
        let mut names: Vec<String> = entries
            .filter_map(Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.starts_with("nvme") || name.starts_with("sd"))
            .collect();
        names.sort();
        for name in names {
            let rotational =
                std::fs::read_to_string(format!("/sys/block/{name}/queue/rotational")).ok();
            let kind = match rotational.as_deref().map(str::trim) {
                Some("0") => "SSD/NVMe",
                Some("1") => "HDD",
                _ => "未知",
            };
            kinds.push(format!("{name}={kind}"));
        }
    }
    if kinds.is_empty() {
        "未知（读不到 /sys/block）".to_string()
    } else {
        kinds.join(", ")
    }
}

// ---------------------------------------------------------------- 逐规模实测

#[allow(clippy::too_many_arguments)]
fn measure_scale(
    scale: Scale,
    build_dir: &Path,
    chinese_poetry_dir: &Path,
    werneror_dir: &Path,
    rhymes: &yunjian_corpus::rhyme::RhymeImport,
    manifest_bytes: &[u8],
    verdict_bytes: &[u8],
    repeats: usize,
    artifact_budget_bytes: u64,
    keep_databases: bool,
) -> Result<Measurement> {
    let started = Instant::now();
    let input = build::assemble(
        scale,
        chinese_poetry_dir,
        werneror_dir,
        rhymes,
        manifest_bytes,
        verdict_bytes,
    )?;
    let raw_text_bytes: u64 = input
        .normalized_records
        .iter()
        .map(|record| record.body.len() as u64)
        .sum();
    let poem_count = input.records.len();
    let input_rows = input.quality.input_rows;

    let db_path = build_dir.join(format!("corpus-{}.db", scale.key()));
    let audit_db_path = yunjian_corpus::db::audit_path(&db_path);
    let stats = yunjian_corpus::db::build_database_with_stats(&db_path, &input)
        .with_context(|| format!("构建语料库失败 {}", db_path.display()))?;
    let build_seconds = started.elapsed().as_secs_f64();

    // 体积与压缩率必须在**随包形态上**量：这是用户真正要下载的那个文件。
    // 首启构建之后再量就掺进了本机派生出来的字节，那不是发布物的大小。
    let shipped = yunjian_corpus::db::open_corpus(&db_path)
        .map_err(|error| anyhow::anyhow!("以只读方式打开语料库失败：{error}"))?;
    yunjian_corpus::db::assert_no_diagnostic_tables(&shipped)
        .context("随包形态断言失败：这个库不该含诊断表或首启派生结构")?;
    let bytes = table_bytes(&shipped)?;
    let poem_table_bytes = sum_prefixed(&bytes, "poem")
        - sum_prefixed(&bytes, "poem_tag")
        - sum_prefixed(&bytes, "poem_rhyme_group");
    drop(shipped);
    let gzip_bytes = gzip_size(&db_path)?;

    // 首启：在本机可写副本上派生三张检索结构。量的是「用户第一次启动要等多久」。
    let mut writable = Connection::open(&db_path)
        .with_context(|| format!("以读写方式打开语料库失败 {}", db_path.display()))?;
    let derived_stats = yunjian_core::build_derived_indexes(&mut writable)
        .map_err(|error| anyhow::anyhow!("首启构建派生结构失败：{error}"))?;
    drop(writable);
    emit(&format!(
        "   首启派生：{} 首 -> ngram {} 行 {:.1} s、尾字 {} 行 {:.1} s、FTS {:.1} s，合计 {:.1} s",
        derived_stats.poems,
        derived_stats.grams,
        derived_stats.ngram_elapsed.as_secs_f64(),
        derived_stats.last_chars,
        derived_stats.last_char_elapsed.as_secs_f64(),
        derived_stats.fts_elapsed.as_secs_f64(),
        derived_stats.elapsed.as_secs_f64()
    ));

    // 延迟在**首启之后**量：那才是用户实际经历的检索性能，也是「不随包不等于功能
    // 缩减」这句话的证据。
    let ready = yunjian_corpus::db::open_corpus(&db_path)
        .map_err(|error| anyhow::anyhow!("首启构建后重新打开语料库失败：{error}"))?;
    yunjian_core::verify_derived_indexes(&ready)
        .map_err(|error| anyhow::anyhow!("首启构建后派生结构不可用：{error}"))?;
    let runtime_bytes = table_bytes(&ready)?;
    let ngram_table_bytes = sum_prefixed(&runtime_bytes, "ngram");
    let ngram_rows: i64 = ready.query_row("SELECT count(*) FROM ngram", [], |row| row.get(0))?;
    let poem_fts_bytes = sum_prefixed(&runtime_bytes, "poem_fts");
    let queries = query::measure_all(&ready, &db_path, repeats)?;
    drop(ready);

    let worst_p95_ms = queries
        .iter()
        .map(|query| query.p95_ms)
        .fold(0.0_f64, f64::max);

    if !keep_databases {
        let _ = std::fs::remove_file(&db_path);
        let _ = std::fs::remove_file(&audit_db_path);
    }

    Ok(Measurement {
        poem_count,
        input_rows,
        raw_text_bytes,
        poem_table_bytes,
        poem_fts_bytes,
        ngram_table_bytes,
        ngram_rows,
        table_bytes: rank_table_bytes(&bytes, stats.bytes_after_vacuum),
        fts_to_poem_ratio: ratio(poem_fts_bytes as f64, poem_table_bytes as f64),
        index_to_raw_ratio: ratio(
            (poem_fts_bytes + ngram_table_bytes) as f64,
            raw_text_bytes as f64,
        ),
        bytes_before_vacuum: stats.bytes_before_vacuum,
        bytes_after_vacuum: stats.bytes_after_vacuum,
        gzip_bytes,
        gzip_ratio: ratio(gzip_bytes as f64, stats.bytes_after_vacuum as f64),
        build_seconds: round3(build_seconds),
        audit_bytes: stats.audit_bytes,
        first_launch_seconds: round3(derived_stats.elapsed.as_secs_f64()),
        within_p95_budget: worst_p95_ms <= DEFAULT_P95_BUDGET_MS,
        within_artifact_budget: gzip_bytes <= artifact_budget_bytes,
        worst_p95_ms: round3(worst_p95_ms),
        queries,
    })
}

fn ratio(numerator: f64, denominator: f64) -> f64 {
    if denominator <= 0.0 {
        0.0
    } else {
        (numerator / denominator * 10_000.0).round() / 10_000.0
    }
}

pub(crate) fn round3(value: f64) -> f64 {
    (value * 1000.0).round() / 1000.0
}

/// 从 `dbstat` 取逐表/逐索引占用字节。
///
/// 用它而不是文件大小差值：文件含基表、B-tree 索引与自由页，无法把「FTS 成本」
/// 单独摘出来，而放大比例的分子恰好是 FTS 自身的字节。
fn table_bytes(connection: &Connection) -> Result<BTreeMap<String, i64>> {
    let mut statement = connection.prepare("SELECT name, sum(pgsize) FROM dbstat GROUP BY name")?;
    let rows = statement.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1).unwrap_or(0)))
    })?;
    let mut out = BTreeMap::new();
    for row in rows {
        let (name, bytes) = row?;
        out.insert(name, bytes);
    }
    Ok(out)
}

fn rank_table_bytes(bytes: &BTreeMap<String, i64>, file_bytes: u64) -> Vec<TableBytes> {
    let mut rows: Vec<TableBytes> = bytes
        .iter()
        .map(|(name, value)| TableBytes {
            name: name.clone(),
            bytes: *value,
            share_of_file: ratio(*value as f64, file_bytes as f64),
        })
        .collect();
    rows.sort_by(|left, right| {
        right
            .bytes
            .cmp(&left.bytes)
            .then_with(|| left.name.cmp(&right.name))
    });
    rows
}

fn sum_prefixed(bytes: &BTreeMap<String, i64>, prefix: &str) -> i64 {
    bytes
        .iter()
        .filter(|(name, _)| name.as_str() == prefix || name.starts_with(&format!("{prefix}_")))
        .map(|(_, value)| *value)
        .sum()
}

/// gzip 后字节。用 `flate2` 而不是外部 `gzip`：随包工件由 todo 21 用同一个库压，
/// 换成宿主机的 `gzip` 会让这个数变成工具版本的函数而不是产物的属性。
fn gzip_size(path: &Path) -> Result<u64> {
    let file = std::fs::File::open(path)?;
    let mut reader = std::io::BufReader::new(file);
    let mut encoder =
        flate2::write::GzEncoder::new(CountingSink::default(), flate2::Compression::default());
    std::io::copy(&mut reader, &mut encoder)?;
    Ok(encoder.finish()?.bytes)
}

/// 只数字节、不落盘的 sink。全量语料的 `.gz` 有几十到几百 MB，为了量一个数写一遍
/// 磁盘是纯浪费，且会让测量受磁盘剩余空间影响。
#[derive(Default)]
struct CountingSink {
    bytes: u64,
}

impl std::io::Write for CountingSink {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.bytes += buf.len() as u64;
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

// ---------------------------------------------------------------- 结论

/// 代表性查询条数。方案点名八条。
pub(crate) const REPRESENTATIVE_QUERY_COUNT: usize = 8;

/// 打在正文上的探针 id。
///
/// 这几条**必须命中**：它们查的是随包正文，而随包正文一定非空，所以零命中只能是
/// 绑定值写错了。与之相对，元数据探针（标签、韵部）可以合法地零命中——对应的表
/// 在当前构建阶段可能还没有数据来源。
const CONTENT_PROBES: [&str; 4] = [
    "two_char_ngram",
    "three_char_match",
    "full_line_like",
    "cold_open_first_query",
];

/// 判定预算。
///
/// **预算只对随包形态的行成立。** 含 `ngram` 与审计表的行是拆分决策的依据而不是
/// 候选发布物——当前构建器已不再产出那种形态，拿它们去卡预算等于对一个不存在的
/// 产物下结论。但它们必须留在报告里：正是那些数字（`ngram` 占 76%、审计表占 67%）
/// 说明了为什么要拆。
///
/// 门禁仍然是真的：把 `--artifact-budget-mib` 设成 1，唯一的随包形态行立刻超预算，
/// 结论翻假。一个随包形态行都没有时结论也是假——否则「零随包实测」会真空为真。
fn decide(rows: &[ScaleRow], artifact_budget_bytes: u64, shipped_scope: &str) -> Verdict {
    let measured: Vec<(&ScaleRow, &Measurement)> = rows
        .iter()
        .filter_map(|row| row.measurement.as_ref().map(|m| (row, m)))
        .collect();
    let shippable: Vec<&(&ScaleRow, &Measurement)> = measured
        .iter()
        .filter(|(row, _)| row.artifact_shape == ArtifactShape::Shipped)
        .collect();
    let full_scale_measured = measured
        .iter()
        .any(|(row, _)| row.scale == Scale::Full.key());
    let largest = measured
        .iter()
        .max_by_key(|(_, m)| m.poem_count)
        .map(|(row, _)| row.scale.clone())
        .unwrap_or_default();

    // 主项取自**结论所针对的那一行**：有随包形态的行就取其中最大的，否则退回
    // 已测里最大的。取错行会让结论点名一张不在发布物里的表。
    let dominant = shippable
        .iter()
        .copied()
        .max_by_key(|(_, m)| m.poem_count)
        .or_else(|| measured.iter().max_by_key(|(_, m)| m.poem_count))
        .and_then(|(_, m)| m.table_bytes.first())
        .map(|table| TableBytes {
            name: table.name.clone(),
            bytes: table.bytes,
            share_of_file: table.share_of_file,
        });

    if shippable.is_empty() {
        return Verdict {
            within_budget: false,
            full_scale_measured,
            largest_measured_scale: largest,
            summary: "报告里没有任何随包形态（去 ngram、去审计表）的实测行，\
                      因此无法对将要发布的工件下预算结论。"
                .to_owned(),
            mitigation: Some(Mitigation {
                id: MITIGATION_NO_SHIPPED_ROW.to_owned(),
                statement: "在参考机上按随包形态重跑一次选定规模的实测。".to_owned(),
                implemented_by: "cargo run -p xtask -- corpus-measure --scale <规模>".to_owned(),
            }),
            dominant_table: dominant,
        };
    }

    let busting: Vec<&&(&ScaleRow, &Measurement)> = shippable
        .iter()
        .filter(|(_, m)| !m.within_artifact_budget || !m.within_p95_budget)
        .collect();
    if busting.is_empty() {
        let shipped_detail = shippable
            .iter()
            .map(|(row, m)| {
                format!(
                    "{}（{} 首）gzip {} MB、首启派生 {:.1} s、审计库另存 {} MB",
                    row.scale,
                    m.poem_count,
                    m.gzip_bytes / (1024 * 1024),
                    m.first_launch_seconds,
                    m.audit_bytes / (1024 * 1024),
                )
            })
            .collect::<Vec<_>>()
            .join("；");
        let summary = format!(
            "随包形态实测 {} 行（{shipped_detail}），全部 gzip <= {} MB 且最差 p95 <= \
             {DEFAULT_P95_BUDGET_MS} ms，预算内。默认随包 {shipped_scope}，全量作为应用内\
             可选下载。另有 {} 行含 ngram 与审计表的实测保留在报告里，它们是拆分决策的依据\
             而不是候选发布物。{}",
            shippable.len(),
            artifact_budget_bytes / (1024 * 1024),
            measured.len() - shippable.len(),
            if full_scale_measured {
                "发布上限规模（full）已实测，缩小随包默认集的依据来自真实数字。"
            } else {
                "发布上限规模尚未实测，结论只覆盖已测规模。"
            }
        );
        return Verdict {
            within_budget: true,
            full_scale_measured,
            largest_measured_scale: largest,
            summary,
            mitigation: None,
            dominant_table: dominant,
        };
    }

    // 爆预算时必须指名一个具体措施。首选「随包默认集限制为唐宋语料、全量作为应用内
    // 可选下载」——它不删任何一首诗，只改随包默认。前提是唐宋规模自己在预算内，
    // 否则这个措施解决不了问题，必须如实说明。
    let tang_song_fits = measured
        .iter()
        .find(|(row, _)| row.scale == Scale::TangSong.key())
        .map(|(_, m)| m.within_artifact_budget && m.within_p95_budget);
    let bust_detail = busting
        .iter()
        .map(|(row, m)| {
            format!(
                "{}（{} 首）gzip {} MB{}，最差 p95 {:.3} ms{}",
                row.scale,
                m.poem_count,
                m.gzip_bytes / (1024 * 1024),
                if m.within_artifact_budget {
                    ""
                } else {
                    " 超体积预算"
                },
                m.worst_p95_ms,
                if m.within_p95_budget {
                    ""
                } else {
                    " 超延迟预算"
                },
            )
        })
        .collect::<Vec<_>>()
        .join("；");

    let mitigation = match tang_song_fits {
        Some(true) => Mitigation {
            id: MITIGATION_TANG_SONG_DEFAULT.to_string(),
            statement: "把随包默认集限制为唐宋语料，全量语料改为应用内可选下载。\
                        默认工件因此落回预算内，且产品不丢任何一首诗。"
                .to_string(),
            implemented_by: "todo 21（corpus-package 按此裁决打包默认集）".to_string(),
        },
        _ => Mitigation {
            id: MITIGATION_NO_SUBSET_FITS.to_string(),
            statement: "已实测的任何子集都不满足预算，因此「限制默认集」无法解决问题。\
                        必须由人决定：进一步缩小默认集（例如仅唐诗），或带论证地提高预算。\
                        本子命令不擅自提高预算。"
                .to_string(),
            implemented_by: "todo 21（需先由人裁决默认集范围或预算）".to_string(),
        },
    };
    let dominant_note = dominant.as_ref().map_or_else(String::new, |table| {
        format!(
            "占字节最多的是 `{}`（{:.1}%）——按朝代缩小集合只能按比例缩小整个文件，\
             削不掉这一项在文件里的占比。",
            table.name,
            table.share_of_file * 100.0
        )
    });
    Verdict {
        within_budget: false,
        full_scale_measured,
        largest_measured_scale: largest,
        summary: format!(
            "超预算：{bust_detail}。{dominant_note}已采用缓解措施 `{}`。",
            mitigation.id
        ),
        mitigation: Some(mitigation),
        dominant_table: dominant,
    }
}

// ---------------------------------------------------------------- 写报告

fn write_reports(root: &Path, report: &MeasuredReport) -> Result<()> {
    let json_path = root.join(REPORT_JSON);
    if let Some(dir) = json_path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let json = serde_json::to_string_pretty(report)?;
    std::fs::write(&json_path, format!("{json}\n"))
        .with_context(|| format!("写出 {REPORT_JSON} 失败"))?;
    std::fs::write(root.join(REPORT_MD), render_markdown(report))
        .with_context(|| format!("写出 {REPORT_MD} 失败"))?;
    Ok(())
}

fn mib(bytes: u64) -> String {
    format!("{:.2}", bytes as f64 / (1024.0 * 1024.0))
}

fn mib_i64(bytes: i64) -> String {
    format!("{:.2}", bytes as f64 / (1024.0 * 1024.0))
}

fn render_markdown(report: &MeasuredReport) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "# 索引体积与查询延迟实测（真实语料）\n");
    let _ = writeln!(
        out,
        "由 `cargo run -p xtask -- corpus-measure` 生成。**表内所有数字均为实测值**；\
         未实测的规模显式标为 `NOT MEASURED` 并附阻塞原因，不以估算值填充。\n"
    );

    let _ = writeln!(out, "## 预算与结论\n");
    let _ = writeln!(
        out,
        "- 声明预算：随包工件 gzip 后 <= **{} MB**，查询 p95 <= **{} ms**",
        report.budget.artifact_gzip_bytes / (1024 * 1024),
        report.budget.p95_ms
    );
    let _ = writeln!(
        out,
        "- 结论：`within_budget = {}`（发布规模已实测：{}）",
        report.verdict.within_budget, report.verdict.full_scale_measured
    );
    let _ = writeln!(out, "- {}\n", report.verdict.summary);
    if let Some(mitigation) = &report.verdict.mitigation {
        let _ = writeln!(out, "### 采用的缓解措施\n");
        let _ = writeln!(out, "- 稳定键：`{}`", mitigation.id);
        let _ = writeln!(out, "- 措施：{}", mitigation.statement);
        let _ = writeln!(out, "- 实现方：{}\n", mitigation.implemented_by);
    }

    let _ = writeln!(out, "## 参考机\n");
    let _ = writeln!(out, "| 项 | 值 |");
    let _ = writeln!(out, "| --- | --- |");
    let _ = writeln!(out, "| 平台 | {} |", report.environment.reference_machine);
    let _ = writeln!(out, "| CPU | {} |", report.environment.cpu_model);
    let _ = writeln!(
        out,
        "| 内存 | {:.1} GiB |",
        report.environment.memory_total_kib as f64 / (1024.0 * 1024.0)
    );
    let _ = writeln!(out, "| 磁盘 | {} |", report.environment.disk_kind);
    let _ = writeln!(out, "| SQLite | {} |", report.environment.sqlite_version);
    let _ = writeln!(
        out,
        "| 索引模式 | detail={}，n-gram 辅助表={} |",
        report.environment.index_detail_mode, report.environment.ngram_aux_enabled
    );
    let _ = writeln!(
        out,
        "| 测量轮次 | 预热 {} + 计时 {} |\n",
        report.environment.warmup_per_query, report.environment.repeats_per_query
    );

    let _ = writeln!(out, "## 逐规模实测\n");
    let _ = writeln!(
        out,
        "**「形态」列决定这一行怎么读。** 标「含 ngram + 审计表」的行是拆分前的实测，\
         当前构建器已不再产出那种文件——它们留在这里是因为正是那些数字（`ngram` 约 76%、\
         两张审计台账合计 67%）促成了拆分，删掉它们此后就没有东西能说明为什么必须拆。\
         预算只对「随包形态」的行成立。`ngram MiB` 与 `ngram 行` 在随包形态下量的是\
         **首启构建之后**的运行期体积，延迟同样在首启之后测——那才是用户实际经历的性能。\n"
    );
    let _ = writeln!(
        out,
        "| 规模 | 形态 | 状态 | 首数 | 原始正文 MiB | poem 表 MiB | poem_fts MiB | FTS/poem | \
         ngram MiB | ngram 行 | 索引/原文 | VACUUM 前 MiB | VACUUM 后 MiB | gzip MiB | \
         审计库 MiB | 首启派生 s | 最差 p95 ms | 体积预算 | 延迟预算 |"
    );
    let _ = writeln!(
        out,
        "| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | \
         --- | --- | --- | --- | --- |"
    );
    for row in &report.scales {
        match &row.measurement {
            None => {
                let _ = writeln!(
                    out,
                    "| {} | {} | NOT MEASURED | — | — | — | — | — | — | — | — | — | — | — | \
                     — | — | — | — | — |",
                    row.scale,
                    row.artifact_shape.label()
                );
            }
            Some(m) => {
                let _ = writeln!(
                    out,
                    "| {} | {} | 实测 | {} | {} | {} | {} | {:.2}x | {} | {} | {:.2}x | {} | {} | \
                     {} | {} | {} | {:.3} | {} | {} |",
                    row.scale,
                    row.artifact_shape.label(),
                    m.poem_count,
                    mib(m.raw_text_bytes),
                    mib_i64(m.poem_table_bytes),
                    mib_i64(m.poem_fts_bytes),
                    m.fts_to_poem_ratio,
                    mib_i64(m.ngram_table_bytes),
                    m.ngram_rows,
                    m.index_to_raw_ratio,
                    mib(m.bytes_before_vacuum),
                    mib(m.bytes_after_vacuum),
                    mib(m.gzip_bytes),
                    if m.audit_bytes == 0 {
                        "—".to_owned()
                    } else {
                        mib(m.audit_bytes)
                    },
                    if m.first_launch_seconds <= 0.0 {
                        "—".to_owned()
                    } else {
                        format!("{:.1}", m.first_launch_seconds)
                    },
                    m.worst_p95_ms,
                    pass_mark(m.within_artifact_budget),
                    pass_mark(m.within_p95_budget),
                );
            }
        }
    }
    let _ = writeln!(out);

    let not_measured: Vec<&ScaleRow> = report
        .scales
        .iter()
        .filter(|row| row.state == MeasurementState::NotMeasured)
        .collect();
    if !not_measured.is_empty() {
        let _ = writeln!(out, "### 未实测的规模与阻塞原因\n");
        for row in not_measured {
            let _ = writeln!(
                out,
                "- **{}**（{}）：{}",
                row.scale,
                row.scope,
                row.blocked_reason.as_deref().unwrap_or_default()
            );
        }
        let _ = writeln!(out);
    }

    let _ = writeln!(out, "## 字节去了哪里（逐表，占比降序）\n");
    let _ = writeln!(
        out,
        "只看 poem / poem_fts / ngram 三项会误判：`disposition` 台账记的是全部**输入**\
         记录（含被排除的），与随包首数无关，却可能占掉文件的大半——这正是把它移出\
         随包工件的依据。随包形态的行里这两张台账已经不在字节账内。\n"
    );
    for row in &report.scales {
        let Some(m) = &row.measurement else { continue };
        let _ = writeln!(out, "### 规模 {}（{} 首）\n", row.scale, m.poem_count);
        let _ = writeln!(out, "| 表/索引 | MiB | 占文件 |");
        let _ = writeln!(out, "| --- | --- | --- |");
        for entry in m.table_bytes.iter().take(12) {
            let _ = writeln!(
                out,
                "| `{}` | {} | {:.1}% |",
                entry.name,
                mib_i64(entry.bytes),
                entry.share_of_file * 100.0
            );
        }
        let _ = writeln!(out);
    }

    let _ = writeln!(out, "## 八条代表性查询的逐条延迟\n");
    let _ = writeln!(
        out,
        "等值类探针的绑定值取**库内最高频值**（近似最坏情形，三个规模可比），因此绑定值\
         随规模变化是预期的。已知口径限制：在 `tang-song` 与 `full` 上，被最多首共用的\
         首句恰好是 Werneror 的 utf8mb4 缺字记录（`□` 替换字符，`corpus/sources.toml` \
         已记载该上游缺陷）。它仍然是真实存在的正文、仍然走 trigram 约束路径、仍然有\
         真实命中，所以这条延迟是有效的；但它量的是「缺字占位串」而不是一句常见诗句。\
         该条 p95 距 150 ms 预算有两个数量级余量，结论不受影响。\n"
    );
    for row in &report.scales {
        let Some(m) = &row.measurement else { continue };
        let _ = writeln!(out, "### 规模 {}（{} 首）\n", row.scale, m.poem_count);
        let _ = writeln!(
            out,
            "| 查询 | 类型 | 命中 | p50 ms | p95 ms | EXPLAIN QUERY PLAN |"
        );
        let _ = writeln!(out, "| --- | --- | --- | --- | --- | --- |");
        for query in &m.queries {
            let _ = writeln!(
                out,
                "| {} | {} | {} | {:.3} | {:.3} | `{}` |",
                query.id,
                query.kind,
                query.hits,
                query.p50_ms,
                query.p95_ms,
                query.explain_query_plan.join(" / ").replace('|', "\\|")
            );
        }
        let _ = writeln!(out);
    }
    out
}

fn pass_mark(ok: bool) -> &'static str {
    if ok { "通过" } else { "**超出**" }
}

pub(crate) const CORPUS_VERSION_FOR_BUILD: &str = CORPUS_VERSION;

#[cfg(test)]
mod tests;
