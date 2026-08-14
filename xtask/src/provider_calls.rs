//! `xtask provider-calls`：实测「随包命中零次模型调用、冷诗恰好一次」。
//!
//! # 这条断言为什么必须独立跑，而不是读日志
//!
//! todo 75 的验收要求「一个 provider 调用计数器确认随包集的诗零次调用、冷诗恰好一次」。
//! 计数器是唯一能回答这件事的东西：从命令输出上看，随包命中与一次成功的模型调用
//! 长得一模一样，而两者对用户的账单差一次请求。
//!
//! # 为什么用 fixture 种子，而不是待发布的数据集
//!
//! 待发布的 `dataset/appreciations.json` 当前 `generation_executed=false`，每条正文是
//! 未生成标记（本机没有开放权重推理条件）。用它验这条断言，「零调用」会成立而
//! 「用户看到了赏析」不成立——那样的 PASS 是假的。因此这里显式用一份 **fixture 种子**：
//! 正文是 fixture 文本，永不发布，验的是**缓存路径**而不是产品内容。报告里如实这么写。
//!
//! # 计数器实现在 xtask 里的原因
//!
//! `yunjian-ai` 里那个 `CountingProvider` 在 `#[cfg(test)]` 模块内且非 `pub`，crate 外
//! 看不到。与其为了复用去改动 crate 的可见性，这里按同一手法（`AtomicUsize` + 实现
//! `AppreciationProvider`）在 xtask 内实现一个——判据是「调用次数」，两处实现不会漂移。

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use rusqlite::{Connection, OpenFlags};
use serde::Serialize;
use yunjian_ai::cache::{
    AppreciationCache, CacheSource, CachedAppreciationProvider,
    DEFAULT_APPRECIATION_CACHE_CAPACITY, ShippedAppreciation,
};
use yunjian_ai::provider::{
    Appreciation, AppreciationProgress, AppreciationProvider, AppreciationRequest,
    AppreciationStreamItem, ProviderId,
};
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
    /// 本次用的是 fixture 种子而不是待发布数据集。恒为 `true`，写进报告备查。
    fixture_seed: bool,
    /// fixture 种子的正文，供报告如实说明「验的是缓存路径而非产品内容」。
    fixture_text: String,
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

/// 抽两首诗建子集语料，只把其中一首写进随包表，然后数调用次数。
pub fn run(corpus_db: PathBuf, out: PathBuf) -> Result<()> {
    use crate::verify_sources::emit;

    emit("== provider 调用计数实测 ==");

    crate::prerequisite::require_corpus_db(&corpus_db)?;

    // 只读打开源库取两个 stable_id。**绝不 `CorpusHandle::open` 源库**：那会在库里
    // 就地建首启派生结构（唐宋规模约十分钟），而随包工件恰恰不该带那些表。
    let picked = pick_two(&corpus_db)?;
    let [shipped_id, cold_id] = picked;
    emit(&format!("随包首：{shipped_id}；冷诗：{cold_id}"));

    let workspace =
        std::env::temp_dir().join(format!("yunjian-provider-calls-{}", std::process::id()));
    crate::pregenerate::prepare_workspace(&workspace)?;
    let subset = workspace.join("subset.db");
    crate::pregenerate::extract_subset(&corpus_db, &subset, &[shipped_id.clone(), cold_id.clone()])
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

    let app_data_dir = workspace.join("profile");
    let cache = Arc::new(AppreciationCache::open(
        &app_data_dir,
        corpus_version,
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
    emit(&format!("已写出 {}", out.display()));

    let _ = std::fs::remove_dir_all(&workspace);

    // 计数不符即中止：这个子命令的产物是一条断言，不是一份观测记录。
    if report.shipped_calls != 0 {
        bail!("随包命中发生了 {} 次模型调用，期望 0", report.shipped_calls);
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
        bail!("随包命中返回的正文不是 fixture 文本，随包层没有真的被读到");
    }
    Ok(())
}

const fn source_name(source: CacheSource) -> &'static str {
    match source {
        CacheSource::Shipped => "shipped",
        CacheSource::Local => "local",
        CacheSource::Generated => "generated",
    }
}

/// 从只读源库里取两首**有正文**的诗。取两首而不是一首：冷诗必须真的存在于语料里，
/// 否则「冷诗恰好一次调用」会退化成「读不到作品就报错」。
fn pick_two(corpus_db: &std::path::Path) -> Result<[String; 2]> {
    let connection = Connection::open_with_flags(
        corpus_db,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI,
    )
    .with_context(|| format!("只读打开 {} 失败", corpus_db.display()))?;
    let mut statement = connection.prepare(
        "SELECT stable_id FROM poem WHERE body IS NOT NULL AND body <> '' \
         ORDER BY stable_id LIMIT 2",
    )?;
    let ids = statement
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    match ids.as_slice() {
        [first, second] => Ok([first.clone(), second.clone()]),
        other => bail!("语料里只取到 {} 首诗，实验需要两首", other.len()),
    }
}
