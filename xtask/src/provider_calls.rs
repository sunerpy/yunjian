//! `xtask provider-calls`：实测「随包命中零次模型调用、冷诗恰好一次」。
//!
//! # 这条断言为什么必须独立跑，而不是读日志
//!
//! todo 75 的验收要求「一个 provider 调用计数器确认随包集的诗零次调用、冷诗恰好一次」。
//! 计数器是唯一能回答这件事的东西：从命令输出上看，随包命中与一次成功的模型调用
//! 长得一模一样，而两者对用户的账单差一次请求。
//!
//! # 为什么要量两路：fixture 种子**与**待发布数据集
//!
//! 「零次模型调用」和「用户看到的是模型写的赏析」是**两件事**，必须分别立。历史上这份
//! 报告只量 fixture 那一路，理由是当时的 `dataset/appreciations.json` 每条正文都是未生成
//! 标记，用它量会得到「零调用成立、用户看到了赏析不成立」的假 PASS。那个前提**已经不成立**
//! 了：数据集现在是开放权重模型的真实输出（清单 `generation_executed=true`）。
//!
//! 前提没了，但两路都得留，各自证明的东西不同：
//!
//! - **fixture 那一路**证明的是**缓存路径本身**。正文是一句带「fixture」字样的固定文本，
//!   与任何产品内容无关，所以它在数据集重新生成、模型换代、覆盖集扩大之后仍然是同一条
//!   确定性实验——它回答「随包层有没有真的被读到」。
//! - **待发布数据集那一路**证明的是**这一份要发出去的工件**在运行期零调用，且随包命中
//!   返回的正文与数据集里那一条**逐字相同**、不含未生成标记。它走的是运行期那条导入路径
//!   （`AppreciationCache::replace_shipped_seed`），不是就地 `INSERT`——手写 SQL 灌进去的行
//!   只能证明「表里有行」，证不了「发布链路会把这些行灌成这样」。
//!
//! 少了后一路，「零调用」就可能是靠一份永不发布的 fixture 撑起来的；少了前一路，数据集
//! 一换内容这条实验就跟着漂。两路的计数都必须是 0，任一不是即中止。
//!
//! # 「零调用」与「正文是模型输出」在报告里仍然是两条断言
//!
//! 本子命令只回答调用次数与正文一致性，**不**替 `generation_executed` 下裁决：那条由
//! `clean-install-report` 的 `shipped_dataset_is_model_output` 独立承担。所以数据集万一
//! 退回未生成状态，本子命令依然会（正确地）报零调用，而那条断言会（正确地）记未执行。
//! 把两者合成一条会让「随包不花钱」与「随包有内容」互相顶替，那正是要避免的事。
//!
//! # 计数器实现在 xtask 里的原因
//!
//! `yunjian-ai` 里那个 `CountingProvider` 在 `#[cfg(test)]` 模块内且非 `pub`，crate 外
//! 看不到。与其为了复用去改动 crate 的可见性，这里按同一手法（`AtomicUsize` + 实现
//! `AppreciationProvider`）在 xtask 内实现一个——判据是「调用次数」，两处实现不会漂移。

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use rusqlite::{Connection, OpenFlags};
use serde::{Deserialize, Serialize};
use yunjian_ai::cache::{
    AppreciationCache, CacheSource, CachedAppreciationProvider,
    DEFAULT_APPRECIATION_CACHE_CAPACITY, ShippedAppreciation,
};
use yunjian_ai::pregenerate::NOT_GENERATED_MARKER;
use yunjian_ai::provider::{
    APPRECIATION_TEMPLATE_VERSION, Appreciation, AppreciationProgress, AppreciationProvider,
    AppreciationRequest, AppreciationStreamItem, ProviderId,
};
use yunjian_core::assets::AppreciationSeedManifest;
use yunjian_core::operation::{OperationHandle, start_operation};
use yunjian_core::{CorpusConfig, CorpusHandle, PoemDetailRequest};

/// fixture 种子的正文。刻意带「fixture」字样：万一它出现在任何用户可见的地方，
/// 一眼就能看出这不是产品内容。
const FIXTURE_TEXT: &str = "随包赏析 fixture 正文（验证缓存路径用，非模型输出，永不发布）";
/// fixture 种子声明的权重与运行时。必须过 `OpenWeightModel` 门禁。
const FIXTURE_MODEL: &str = "deepseek-r1:7b";
const FIXTURE_MODEL_LICENSE: &str = "MIT";
/// 用户侧 provider 标识。与 fixture 种子的生成方**故意不同**：随包命中必须与
/// 用户用哪个 provider 无关。
const USER_PROVIDER: &str = "counting-user-provider";

/// 待发布数据集与它的清单，相对仓库根。刻意不做成命令行参数：本子命令量的就是
/// **要发出去的那一份**，允许指向别处等于允许把这条断言指到一份好看的工件上。
const RELEASED_SEED: &str = "dataset/appreciations.json";
const RELEASED_MANIFEST: &str = "dataset/appreciations.manifest.json";

/// 报告里回显的正文长度上限。全文进 JSON 会让报告长到没人读，而没人读的判词与没有等价。
const TEXT_HEAD_CHARS: usize = 60;

/// 待发布数据集清单里本子命令要用的字段。
#[derive(Debug, Deserialize)]
struct ReleasedManifest {
    template_version: String,
    corpus_version: String,
    record_count: usize,
    generation_executed: bool,
    appreciations_sha256: String,
    model: String,
    model_license: String,
}

/// 待发布数据集里本子命令要用的字段。
#[derive(Debug, Deserialize)]
struct ReleasedRecord {
    stable_id: String,
    text: String,
}

/// 实测结果。写成 JSON 供 `clean-install-report` 读取，不靠人眼转抄数字。
#[derive(Debug, Serialize)]
struct ProviderCallReport {
    /// fixture 种子里那首诗，解析时发生的模型调用次数。期望 0。
    shipped_calls: usize,
    /// 不在 fixture 种子里那首诗，解析时发生的模型调用次数。期望 1。
    cold_calls: usize,
    /// 同一首冷诗再解析一次的累计调用次数。期望仍是 1（第二次走用户缓存）。
    cold_calls_after_repeat: usize,
    /// 随包命中的来源标记。期望 `shipped`。
    shipped_source: String,
    /// 冷诗首次解析的来源标记。期望 `generated`。
    cold_source: String,
    /// 随包命中返回的正文。
    shipped_text: String,
    /// 用来做实验的两个 `stable_id`。
    shipped_poem: String,
    cold_poem: String,
    /// 上面那组数字用的是 fixture 种子而不是待发布数据集。恒为 `true`，写进报告备查。
    fixture_seed: bool,
    /// fixture 种子的正文，供报告如实说明这一路验的是缓存路径而非产品内容。
    fixture_text: String,
    /// 待发布数据集经运行期导入路径落库后，随包命中发生的模型调用次数。期望 0。
    released_seed_calls: usize,
    /// 待发布数据集那一路的来源标记。期望 `shipped`。
    released_seed_source: String,
    /// 用来做这一路实验的作品，取自数据集的第一条能在语料里查到的记录。
    released_seed_poem: String,
    /// 随包命中返回的正文是否与数据集里那一条逐字相同。
    released_seed_text_matches_dataset: bool,
    /// 返回正文是否仍含未生成标记。必须为 `false`——**行数不是内容**，报告里要能看见这一点。
    released_seed_text_has_marker: bool,
    /// 返回正文的字符数与首段，供人眼复核「这是一段赏析」而不只是「查到了一行」。
    released_seed_text_chars: usize,
    released_seed_text_head: String,
    /// 数据集清单如实回显。裁决由 `shipped_dataset_is_model_output` 承担，这里只记事实。
    released_seed_record_count: usize,
    released_seed_generation_executed: bool,
    released_seed_model: String,
    released_seed_model_license: String,
    /// 导入走的是运行期那条路径的名字，写进报告以区别于就地 `INSERT`。
    released_seed_import_path: String,
}

struct CountingProvider {
    id: ProviderId,
    calls: Arc<AtomicUsize>,
}

impl CountingProvider {
    fn new(id: &str) -> Result<Self> {
        Ok(Self {
            id: ProviderId::new(id).map_err(|error| anyhow::anyhow!("{error}"))?,
            calls: Arc::new(AtomicUsize::new(0)),
        })
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }

    fn handle(&self) -> Self {
        Self {
            id: self.id.clone(),
            calls: Arc::clone(&self.calls),
        }
    }
}

#[async_trait]
impl AppreciationProvider for CountingProvider {
    async fn appreciate(&self, request: AppreciationRequest) -> yunjian_core::Result<Appreciation> {
        let sequence = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
        Ok(Appreciation {
            text: format!("第 {sequence} 次模型生成（计数器）"),
            model: request.model().to_owned(),
            provider: self.id(),
            generated_at: sequence as u64,
            template_version: request.template_version().to_owned(),
            grounding_digest: request.grounding_digest().to_owned(),
            usage: None,
        })
    }

    async fn appreciate_stream(
        &self,
        request: AppreciationRequest,
    ) -> yunjian_core::Result<OperationHandle<AppreciationProgress, AppreciationStreamItem>> {
        let result = self.appreciate(request).await?;
        Ok(start_operation(move |reporter| {
            reporter.item(AppreciationStreamItem::Complete(result));
            Ok(())
        }))
    }

    fn id(&self) -> ProviderId {
        self.id.clone()
    }
}

/// 抽三首诗建子集语料，fixture 与待发布数据集各量一路，然后数调用次数。
pub fn run(corpus_db: PathBuf, out: PathBuf) -> Result<()> {
    use crate::verify_sources::emit;

    emit("== provider 调用计数实测 ==");

    crate::prerequisite::require_corpus_db(&corpus_db)?;

    let root = crate::index_spike::repo_root()?;
    let seed_path = root.join(RELEASED_SEED);
    let released_manifest: ReleasedManifest = read_json(&root.join(RELEASED_MANIFEST))?;
    let released_records: Vec<ReleasedRecord> = read_json(&seed_path)?;
    emit(&format!(
        "待发布数据集：{} 条，模板 {}，语料 {}，generation_executed={}",
        released_records.len(),
        released_manifest.template_version,
        released_manifest.corpus_version,
        released_manifest.generation_executed
    ));

    // 只读打开源库取 stable_id。**绝不 `CorpusHandle::open` 源库**：那会在库里
    // 就地建首启派生结构（唐宋规模约十分钟），而随包工件恰恰不该带那些表。
    let released_ids = released_records
        .iter()
        .map(|record| record.stable_id.clone())
        .collect::<BTreeSet<_>>();
    let [shipped_id, cold_id] = pick_two(&corpus_db, &released_ids)?;
    let released_id = pick_released(&corpus_db, &released_records)?;
    let released_record = released_records
        .iter()
        .find(|record| record.stable_id == released_id)
        .context("刚挑出的数据集记录又找不回来了")?;
    emit(&format!(
        "fixture 首：{shipped_id}；冷诗：{cold_id}；数据集首：{released_id}"
    ));

    let workspace =
        std::env::temp_dir().join(format!("yunjian-provider-calls-{}", std::process::id()));
    crate::pregenerate::prepare_workspace(&workspace)?;
    let subset = workspace.join("subset.db");
    crate::pregenerate::extract_subset(
        &corpus_db,
        &subset,
        &[shipped_id.clone(), cold_id.clone(), released_id.clone()],
    )
    .context("抽取子集语料失败（源库不会被改动）")?;

    let handle = CorpusHandle::open(&CorpusConfig {
        path: Some(subset.clone()),
        data_dir: workspace.join("corpus-data"),
        archive: None,
    })
    .context("打开子集语料失败")?;
    let corpus_version = handle.meta().corpus_version.clone();
    let client = yunjian_core::Yunjian::new(handle);

    let shipped_detail = client
        .poem_detail(PoemDetailRequest {
            poem_id: shipped_id.clone(),
        })
        .with_context(|| format!("读取 {shipped_id} 详情失败"))?;
    let cold_detail = client
        .poem_detail(PoemDetailRequest {
            poem_id: cold_id.clone(),
        })
        .with_context(|| format!("读取 {cold_id} 详情失败"))?;

    let released_detail = client
        .poem_detail(PoemDetailRequest {
            poem_id: released_id.clone(),
        })
        .with_context(|| format!("读取 {released_id} 详情失败"))?;

    // 两路各用一个全新 profile。`replace_shipped_seed` 以单一事务**清空**随包表，
    // 共用一个 profile 会让后跑的那一路把前一路的随包行删掉，于是前一路的数字失去依据。
    let app_data_dir = workspace.join("profile-fixture");
    let cache = Arc::new(AppreciationCache::open(
        &app_data_dir,
        corpus_version.clone(),
        DEFAULT_APPRECIATION_CACHE_CAPACITY,
    )?);

    // fixture 种子只写一首。`grounding_digest` 必须由 `AppreciationRequest` 渲染得出，
    // 否则运行期查随包表时算出的摘要对不上，那一行会被判 stale 而 100% 落空。
    let shipped_request = AppreciationRequest::new(shipped_detail.clone(), FIXTURE_MODEL);
    cache
        .insert_shipped(&ShippedAppreciation {
            stable_id: shipped_id.clone(),
            template_version: shipped_request.template_version().to_owned(),
            model: FIXTURE_MODEL.to_owned(),
            model_license: FIXTURE_MODEL_LICENSE.to_owned(),
            grounding_digest: shipped_request.grounding_digest().to_owned(),
            text: FIXTURE_TEXT.to_owned(),
            generated_at: 1,
        })
        .context("写 fixture 随包行失败")?;

    let counter = CountingProvider::new(USER_PROVIDER)?;
    let probe = counter.handle();
    let cached = CachedAppreciationProvider::new(counter, Arc::clone(&cache));

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;

    // 用户请求用的是**另一个模型标识**：随包命中按 `stable_id` + 模板版本查，
    // 与用户选了哪个模型无关。若这里因为模型不同而落空，随包集对用户就是不存在的。
    let shipped_hit = runtime
        .block_on(cached.resolve(AppreciationRequest::new(shipped_detail, "user-model")))
        .context("解析随包首失败")?;
    let shipped_calls = probe.calls();

    let cold_hit = runtime
        .block_on(cached.resolve(AppreciationRequest::new(cold_detail.clone(), "user-model")))
        .context("解析冷诗失败")?;
    let cold_calls = probe.calls() - shipped_calls;

    // 再来一次同一首冷诗：第二次必须走用户缓存，累计仍是一次。少了这一步，
    // 「恰好一次」就只证明了「至少一次」。
    runtime
        .block_on(cached.resolve(AppreciationRequest::new(cold_detail, "user-model")))
        .context("重复解析冷诗失败")?;
    let cold_calls_after_repeat = probe.calls() - shipped_calls;

    let released = measure_released_seed(
        &workspace.join("profile-released"),
        &corpus_version,
        &seed_path,
        &released_manifest,
        released_detail,
    )?;

    let report = ProviderCallReport {
        shipped_calls,
        cold_calls,
        cold_calls_after_repeat,
        shipped_source: source_name(shipped_hit.source).to_owned(),
        cold_source: source_name(cold_hit.source).to_owned(),
        shipped_text: shipped_hit.appreciation.text.clone(),
        shipped_poem: shipped_id,
        cold_poem: cold_id,
        fixture_seed: true,
        fixture_text: FIXTURE_TEXT.to_owned(),
        released_seed_calls: released.calls,
        released_seed_source: released.source,
        released_seed_poem: released_id.clone(),
        released_seed_text_matches_dataset: released.text == released_record.text,
        released_seed_text_has_marker: released.text.contains(NOT_GENERATED_MARKER),
        released_seed_text_chars: released.text.chars().count(),
        released_seed_text_head: head(&released.text),
        released_seed_record_count: released_manifest.record_count,
        released_seed_generation_executed: released_manifest.generation_executed,
        released_seed_model: released_manifest.model.clone(),
        released_seed_model_license: released_manifest.model_license.clone(),
        released_seed_import_path: "AppreciationCache::replace_shipped_seed".to_owned(),
    };

    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut json = serde_json::to_string_pretty(&report)?;
    json.push('\n');
    std::fs::write(&out, &json).with_context(|| format!("写 {} 失败", out.display()))?;

    emit(&format!(
        "随包命中：{} 次调用（来源 {}）；冷诗：{} 次（来源 {}）；重复后累计 {} 次",
        report.shipped_calls,
        report.shipped_source,
        report.cold_calls,
        report.cold_source,
        report.cold_calls_after_repeat
    ));
    emit(&format!(
        "待发布数据集：{} 次调用（来源 {}），正文 {} 字、与数据集逐字一致={}、含未生成标记={}；首段「{}」",
        report.released_seed_calls,
        report.released_seed_source,
        report.released_seed_text_chars,
        report.released_seed_text_matches_dataset,
        report.released_seed_text_has_marker,
        report.released_seed_text_head
    ));
    emit(&format!("已写出 {}", out.display()));

    let _ = std::fs::remove_dir_all(&workspace);

    // 计数不符即中止：这个子命令的产物是一条断言，不是一份观测记录。
    if report.shipped_calls != 0 {
        bail!(
            "fixture 随包命中发生了 {} 次模型调用，期望 0",
            report.shipped_calls
        );
    }
    if report.cold_calls != 1 {
        bail!("冷诗发生了 {} 次模型调用，期望恰好 1", report.cold_calls);
    }
    if report.cold_calls_after_repeat != 1 {
        bail!(
            "重复请求同一首冷诗后累计 {} 次调用，期望仍是 1",
            report.cold_calls_after_repeat
        );
    }
    if report.shipped_text != FIXTURE_TEXT {
        bail!("fixture 随包命中返回的正文不是 fixture 文本，随包层没有真的被读到");
    }
    if report.released_seed_calls != 0 {
        bail!(
            "待发布数据集的随包命中发生了 {} 次模型调用，期望 0",
            report.released_seed_calls
        );
    }
    if report.released_seed_source != "shipped" {
        bail!(
            "待发布数据集那一首的来源是 {}，期望 shipped；零调用若不是来自随包命中就没有意义",
            report.released_seed_source
        );
    }
    // 「表里有行」不等于「行里有赏析」，这正是本轮要修的那件事：逐字比对数据集正文，
    // 并单独拦未生成标记——只比字符串相等时，两边都是占位标记也会一致。
    if !report.released_seed_text_matches_dataset {
        bail!(
            "随包命中返回的正文与 {} 里那一条不同；发布出去的内容与用户看到的内容不是一份",
            seed_path.display()
        );
    }
    if report.released_seed_text_has_marker {
        bail!(
            "随包命中返回的正文仍含未生成标记 `{NOT_GENERATED_MARKER}`；\
             这份数据集不是模型输出，不得当成赏析发布"
        );
    }
    Ok(())
}

/// 待发布数据集那一路的实测结果。
struct ReleasedMeasurement {
    calls: usize,
    source: String,
    text: String,
}

/// 用运行期那条导入路径把待发布数据集灌进一个全新 profile，再数一次调用。
///
/// 刻意**不**就地 `INSERT`：手写 SQL 造出来的随包行只能证明「表里有行」，证不了
/// 「发布链路会把这些行灌成这样」。走 `replace_shipped_seed` 顺带把兼容矩阵
/// （语料版本、模板版本、记录数、逐条开放权重门禁）一起过一遍。
fn measure_released_seed(
    app_data_dir: &Path,
    corpus_version: &str,
    seed_path: &Path,
    manifest: &ReleasedManifest,
    detail: yunjian_core::PoemDetail,
) -> Result<ReleasedMeasurement> {
    let cache = Arc::new(AppreciationCache::open(
        app_data_dir,
        corpus_version.to_owned(),
        DEFAULT_APPRECIATION_CACHE_CAPACITY,
    )?);
    let seed_manifest = AppreciationSeedManifest {
        url: seed_path.to_string_lossy().into_owned(),
        sha256: manifest.appreciations_sha256.clone(),
        template_version: manifest.template_version.clone(),
        corpus_version: manifest.corpus_version.clone(),
        record_count: manifest.record_count,
    };
    let imported = cache
        .replace_shipped_seed(seed_path, &seed_manifest, APPRECIATION_TEMPLATE_VERSION)
        .with_context(|| format!("经运行期导入路径灌入 {} 失败", seed_path.display()))?;
    crate::verify_sources::emit(&format!(
        "已按 replace_shipped_seed 导入 {imported} 条随包行（全新 profile）"
    ));

    let counter = CountingProvider::new(USER_PROVIDER)?;
    let probe = counter.handle();
    let cached = CachedAppreciationProvider::new(counter, Arc::clone(&cache));
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let hit = runtime
        .block_on(cached.resolve(AppreciationRequest::new(detail, "user-model")))
        .context("解析待发布数据集里那一首失败")?;
    Ok(ReleasedMeasurement {
        calls: probe.calls(),
        source: source_name(hit.source).to_owned(),
        text: hit.appreciation.text,
    })
}

fn head(text: &str) -> String {
    let trimmed = text.trim().replace(['\n', '\r'], " ");
    if trimmed.chars().count() <= TEXT_HEAD_CHARS {
        return trimmed;
    }
    let clipped = trimmed.chars().take(TEXT_HEAD_CHARS).collect::<String>();
    format!("{clipped}…")
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
    let bytes = std::fs::read(path).with_context(|| format!("读 {} 失败", path.display()))?;
    serde_json::from_slice(&bytes).with_context(|| format!("解析 {} 失败", path.display()))
}

const fn source_name(source: CacheSource) -> &'static str {
    match source {
        CacheSource::Shipped => "shipped",
        CacheSource::Local => "local",
        CacheSource::Generated => "generated",
    }
}

/// 从只读源库里取两首**有正文**的诗，两首都必须**不在**待发布数据集里。
///
/// 取两首而不是一首：冷诗必须真的存在于语料里，否则「冷诗恰好一次调用」会退化成
/// 「读不到作品就报错」。排除数据集覆盖集是后来补的：冷诗若恰好被数据集收录，
/// 导入待发布种子的那一路会让它变成随包命中，「恰好一次调用」就会莫名变成零次。
fn pick_two(corpus_db: &Path, excluded: &BTreeSet<String>) -> Result<[String; 2]> {
    let connection = open_read_only(corpus_db)?;
    let mut statement = connection.prepare(
        "SELECT stable_id FROM poem WHERE body IS NOT NULL AND body <> '' ORDER BY stable_id",
    )?;
    let mut picked = Vec::with_capacity(2);
    for id in statement.query_map([], |row| row.get::<_, String>(0))? {
        let id = id?;
        if excluded.contains(&id) {
            continue;
        }
        picked.push(id);
        if picked.len() == 2 {
            break;
        }
    }
    match picked.as_slice() {
        [first, second] => Ok([first.clone(), second.clone()]),
        other => bail!(
            "语料里只取到 {} 首数据集之外的诗，实验需要两首",
            other.len()
        ),
    }
}

/// 取数据集里第一条**能在语料里查到**的记录。
///
/// 不直接用第一条：数据集键在 `stable_id` 上，语料一旦做过文本修正就可能有记录成为孤儿，
/// 那时候「随包命中」会退化成「读不到作品就报错」，而报错读起来像缓存路径坏了。
fn pick_released(corpus_db: &Path, records: &[ReleasedRecord]) -> Result<String> {
    let connection = open_read_only(corpus_db)?;
    let mut statement = connection
        .prepare("SELECT 1 FROM poem WHERE stable_id = ?1 AND body IS NOT NULL AND body <> ''")?;
    for record in records {
        if statement.exists(rusqlite::params![record.stable_id])? {
            return Ok(record.stable_id.clone());
        }
    }
    bail!(
        "待发布数据集里的 {} 条记录没有一条能在 {} 里查到；随包命中无从发生",
        records.len(),
        corpus_db.display()
    )
}

fn open_read_only(corpus_db: &Path) -> Result<Connection> {
    Connection::open_with_flags(
        corpus_db,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI,
    )
    .with_context(|| format!("只读打开 {} 失败", corpus_db.display()))
}
