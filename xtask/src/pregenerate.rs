//! `xtask pregenerate`：用**开放权重模型**预生成随包赏析数据集。
//!
//! # 为什么这个子命令不能用闭源 API
//!
//! 产物是**由我们分发**的内容。研究阶段逐条读过条款后（`.omo/drafts/yunjian.md` 的
//! C11）：Anthropic 把 Output 的权利让给客户但禁止用其服务训练竞品；OpenAI 的对应条款
//! 因站点 403 **未能核实**；DeepSeek 的条款**完全未核实**。三条里两条是未知，所以随包
//! 数据集改用可下载权重生成——权重不附带限制输出再分发的 API 条款，那两条未知因此与
//! 本产物无关。门禁在 `yunjian_ai::pregenerate` 里，本文件只是它的驱动。
//!
//! # 覆盖范围是显式声明的，不是「尽量多」
//!
//! v1 只覆盖 [`ANTHOLOGY_TAGS`] 四个选本，合计数千首。全语料 47 万首里绝大多数没有人
//! 会去读赏析，逐首生成既让成本失控也让工件体积失控。覆盖范围写在清单的
//! `coverage_tags` 里，任何扩大都是一次显式改动。
//!
//! # 为什么要先抽子集语料，而不是直接打开随包库
//!
//! 事实块（`grounding`）必须由 `yunjian_ai::AppreciationRequest` 渲染，否则算出来的
//! `grounding_digest` 与运行期查随包表时算的那个对不上，随包行会 100% 落空。而它需要
//! 一个 `PoemDetail`，也就需要 `CorpusHandle`——`CorpusHandle::open` 会在库里**就地**
//! 建首启派生结构（唐宋规模约十分钟）。随包工件恰恰不该带那些表（`corpus-package`
//! 有断言守着），所以直接开它会污染待发布的文件。
//!
//! 于是流程是「只读打开源库 -> 解析覆盖集 -> 把这几十首抽成一个临时子集库 -> 在子集上
//! 开 `CorpusHandle`」。派生跑在几十首上是瞬时的，源库一个字节都不动。
//!
//! # 覆盖集有两条筛选途径，都会写进清单
//!
//! 首选读语料的 `poem_tag`。但随包库当前 `poem_tag` 为空（构建管线尚未把
//! `assign_tags` 的产物接进去），此时回退到按 `tags.toml` 的评审名单以 `(作者, 题目)`
//! 解析——那正是词表自己声明的名单键。用了哪一条写进 `coverage_selector`，
//! 不静默降级。

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use rusqlite::{Connection, OpenFlags, params};
use yunjian_ai::pregenerate::{
    ANTHOLOGY_TAGS, CoverageSelector, DATASET_SCHEMA_VERSION, DatasetManifest,
    NOT_GENERATED_MARKER, OpenWeightModel, PregeneratedDataset, PregeneratedRecord,
    ensure_disclosure, sha256_hex,
};
use yunjian_ai::provider::{
    APPRECIATION_TEMPLATE_VERSION, AppreciationProvider, AppreciationRequest,
};
use yunjian_ai::{GenAiProvider, GenAiProviderConfig, ProviderKind};
use yunjian_core::{CorpusConfig, CorpusHandle, PoemDetail};

use crate::verify_sources::emit;

/// 默认的开放权重配置：MIT 许可的 DeepSeek 蒸馏权重，由本地 Ollama 加载。
pub const DEFAULT_MODEL: &str = "deepseek-r1:7b";
/// 默认权重许可（SPDX）。
pub const DEFAULT_MODEL_LICENSE: &str = "MIT";
/// 默认本地运行时。
pub const DEFAULT_PROVIDER: &str = "ollama";

/// 数据集与披露文件所在目录。
const DATASET_DIR: &str = "dataset";
/// 数据集文件名。验收的 `jq` 断言读它。
const DATASET_FILE: &str = "appreciations.json";
/// 清单文件名。元数据全在这里，因此数据集文件的形状永不改变。
const MANIFEST_FILE: &str = "appreciations.manifest.json";
/// 披露文件名。打包前逐条校验。
const DISCLOSURE_FILE: &str = "README.md";

pub fn run(
    corpus_db: PathBuf,
    limit: Option<usize>,
    out_dir: Option<PathBuf>,
    model: String,
    model_license: String,
    provider: String,
    endpoint: Option<String>,
) -> Result<()> {
    let root = crate::index_spike::repo_root()?;
    let out_dir = out_dir.unwrap_or_else(|| root.join(DATASET_DIR));

    emit("== 随包赏析数据集预生成 ==");

    // 开放权重门禁先跑：配置不合规时一条记录都不该被生成出来。
    let weights = OpenWeightModel::new(&model, &model_license, &provider)
        .context("开放权重门禁拒绝了本次生成配置")?;
    emit(&format!(
        "开放权重配置：model={} license={} runtime={}",
        weights.model, weights.model_license, weights.provider
    ));

    // 披露也先校验：缺披露的数据集不得被发布，那个判定必须在写盘之前。
    let disclosure_path = out_dir.join(DISCLOSURE_FILE);
    let disclosure = std::fs::read_to_string(&disclosure_path)
        .with_context(|| format!("读取披露文件 {} 失败", disclosure_path.display()))?;
    ensure_disclosure(&disclosure).with_context(|| {
        format!(
            "{} 的披露不完整；随包 AI 赏析未经领域专家审校，缺披露不得打包",
            disclosure_path.display()
        )
    })?;
    emit(&format!("披露校验通过：{}", disclosure_path.display()));

    crate::prerequisite::require_corpus_db(&corpus_db)?;
    let source = open_read_only(&corpus_db)
        .with_context(|| format!("只读打开语料库 {} 失败", corpus_db.display()))?;
    let corpus_version = source
        .query_row(
            "SELECT corpus_version FROM corpus_meta WHERE singleton = 1",
            [],
            |row| row.get::<_, String>(0),
        )
        .context("读取语料版本失败")?;

    let (selector, mut picked) = resolve_coverage(&source, &corpus_db)?;
    emit(&format!(
        "覆盖集：选本 {ANTHOLOGY_TAGS:?}，筛选途径 {}，命中 {} 首",
        selector.as_str(),
        picked.len()
    ));
    if picked.is_empty() {
        bail!(
            "覆盖集为空。语料 {} 的 `poem_tag` 里没有选本标签行，评审名单也解析不出任何作品；\
             预生成拒绝写出一份空数据集",
            corpus_db.display()
        );
    }
    if let Some(limit) = limit
        && picked.len() > limit
    {
        picked.truncate(limit);
        emit(&format!("按 --limit 截到 {} 首", picked.len()));
    }
    drop(source);

    let workspace = out_dir.join(".work");
    let subset = workspace.join("subset.db");
    prepare_workspace(&workspace)?;
    extract_subset(&corpus_db, &subset, &picked).context("抽取子集语料失败（源库不会被改动）")?;
    emit(&format!(
        "子集语料：{}（{} 首）",
        subset.display(),
        picked.len()
    ));

    let handle = CorpusHandle::open(&CorpusConfig {
        path: Some(subset.clone()),
        data_dir: workspace.join("data"),
        archive: None,
    })
    .context("打开子集语料失败")?;
    let client = yunjian_core::Yunjian::new(handle);

    let generator = match endpoint.as_deref() {
        Some(endpoint) => Some(Generator::connect(&weights, endpoint)?),
        None => None,
    };
    let generation_executed = generator.is_some();
    if generation_executed {
        emit("生成模式：调用本地开放权重运行时");
    } else {
        emit(
            "生成模式：未执行推理（未给 --endpoint）。管道、门禁与溯源字段照常校验，\
             每条正文写入未生成标记，清单标 generation_executed=false",
        );
    }

    let mut dataset = PregeneratedDataset::new(generation_executed);
    let now = unix_seconds();
    for stable_id in &picked {
        let detail = client
            .poem_detail(yunjian_core::PoemDetailRequest {
                poem_id: stable_id.clone(),
            })
            .with_context(|| format!("读取作品详情 {stable_id} 失败"))?;
        let request = AppreciationRequest::new(detail.clone(), &weights.model);
        let text = match generator.as_ref() {
            Some(generator) => generator.appreciate(request.clone())?,
            None => NOT_GENERATED_MARKER.to_owned(),
        };
        dataset
            .push(build_record(&detail, &request, &weights, text, now))
            .with_context(|| format!("记录 {stable_id} 未通过预生成门禁"))?;
    }

    let json = dataset.to_json().context("序列化数据集失败")?;
    let digest = sha256_hex(json.as_bytes());
    let manifest = DatasetManifest {
        schema_version: DATASET_SCHEMA_VERSION,
        template_version: APPRECIATION_TEMPLATE_VERSION.to_owned(),
        coverage_tags: ANTHOLOGY_TAGS.iter().map(|tag| (*tag).to_owned()).collect(),
        coverage_selector: selector.as_str().to_owned(),
        record_count: dataset.records().len(),
        model: weights.model.clone(),
        model_license: weights.model_license.clone(),
        provider: weights.provider.clone(),
        generation_executed,
        not_executed_reason: (!generation_executed).then(|| {
            "未提供 --endpoint：本机没有可达的开放权重推理运行时，故只跑管道与门禁".to_owned()
        }),
        appreciations_sha256: digest.clone(),
        corpus_version,
        built_at: now,
    };

    std::fs::create_dir_all(&out_dir)?;
    let dataset_path = out_dir.join(DATASET_FILE);
    std::fs::write(&dataset_path, &json)
        .with_context(|| format!("写出 {} 失败", dataset_path.display()))?;
    let manifest_path = out_dir.join(MANIFEST_FILE);
    let mut manifest_json = serde_json::to_string_pretty(&manifest)?;
    manifest_json.push('\n');
    std::fs::write(&manifest_path, &manifest_json)
        .with_context(|| format!("写出 {} 失败", manifest_path.display()))?;
    std::fs::write(
        out_dir.join(format!("{DATASET_FILE}.sha256")),
        format!("{digest}  {DATASET_FILE}\n"),
    )?;
    let _ = std::fs::remove_dir_all(&workspace);

    emit(&format!(
        "已写出 {}（{} 条，sha256 {digest}）",
        dataset_path.display(),
        manifest.record_count
    ));
    emit(&format!("已写出 {}", manifest_path.display()));
    if !generation_executed {
        emit("NOT EXECUTED：真实推理未执行，本产物不是模型输出，不得当成赏析发布");
    }
    Ok(())
}

fn build_record(
    detail: &PoemDetail,
    request: &AppreciationRequest,
    weights: &OpenWeightModel,
    text: String,
    now: u64,
) -> PregeneratedRecord {
    PregeneratedRecord {
        stable_id: detail.poem.stable_id.clone(),
        title: detail.poem.title.clone(),
        author: detail.poem.author.clone(),
        anthology_tags: detail
            .tags
            .iter()
            .filter(|tag| ANTHOLOGY_TAGS.contains(&tag.as_str()))
            .cloned()
            .collect(),
        model: weights.model.clone(),
        model_license: weights.model_license.clone(),
        provider: weights.provider.clone(),
        generated_at: now,
        template_version: request.template_version().to_owned(),
        grounding_digest: request.grounding_digest().to_owned(),
        reviewed: false,
        text,
    }
}

/// 本地开放权重运行时的驱动。
///
/// 只在给了 `--endpoint` 时构造。运行时不可达时**中止**而不是回落到未生成标记：
/// 「我要求真跑」与「跑不起来就算了」是两个不同的意图，后者会让人以为数据集是真的。
struct Generator {
    provider: GenAiProvider,
    runtime: tokio::runtime::Runtime,
}

impl Generator {
    fn connect(weights: &OpenWeightModel, endpoint: &str) -> Result<Self> {
        if weights.provider != ProviderKind::Ollama.as_str() {
            bail!(
                "当前只实现了经 Ollama 加载本地权重的生成路径，收到运行时 `{}`",
                weights.provider
            );
        }
        let provider = GenAiProvider::with_secret(
            GenAiProviderConfig::new(ProviderKind::Ollama)
                .with_base_url(endpoint)
                .with_model_override(&weights.model),
            None,
        )
        .context("构造本地开放权重供应商失败")?;
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .context("创建异步运行时失败")?;
        Ok(Self { provider, runtime })
    }

    fn appreciate(&self, request: AppreciationRequest) -> Result<String> {
        let appreciation = self
            .runtime
            .block_on(self.provider.appreciate(request))
            .context("开放权重运行时生成赏析失败")?;
        let text = appreciation.text.trim();
        if text.is_empty() {
            bail!("运行时返回了空赏析；空正文不得进入随包数据集");
        }
        // 剥掉首尾空白而不是原样收下：推理型权重（deepseek-r1 一类）的思维块被运行时摘掉后
        // 会留下前导空行，实测 16/16 条都以 `\n\n` 开头。那不是内容，是剥离留下的痕迹，
        // 带进随包表就成了详情页赏析面板顶上一段空白。判据与本函数自己的空正文检查一致：
        // 既然「trim 后为空」算没有内容，trim 掉的那部分就不是内容。
        Ok(text.to_owned())
    }
}

fn resolve_coverage(
    source: &Connection,
    corpus_db: &Path,
) -> Result<(CoverageSelector, Vec<String>)> {
    let by_tag = select_by_poem_tag(source)?;
    if !by_tag.is_empty() {
        return Ok((CoverageSelector::PoemTag, by_tag));
    }
    emit(&format!(
        "语料 {} 的 `poem_tag` 里没有选本标签行，回退到 tags.toml 的评审名单按 (作者, 题目) 解析",
        corpus_db.display()
    ));
    Ok((CoverageSelector::ReviewedRoster, select_by_roster(source)?))
}

fn select_by_poem_tag(source: &Connection) -> Result<Vec<String>> {
    let placeholders = ANTHOLOGY_TAGS
        .iter()
        .enumerate()
        .map(|(index, _)| format!("?{}", index + 1))
        .collect::<Vec<_>>()
        .join(", ");
    let mut statement = source.prepare(&format!(
        "SELECT DISTINCT poem_id FROM poem_tag WHERE tag IN ({placeholders}) ORDER BY poem_id"
    ))?;
    let rows = statement.query_map(rusqlite::params_from_iter(ANTHOLOGY_TAGS), |row| {
        row.get::<_, String>(0)
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

fn select_by_roster(source: &Connection) -> Result<Vec<String>> {
    let vocabulary = yunjian_corpus::tag::TagVocabulary::shipped()
        .map_err(|error| anyhow::anyhow!("解析策展词表失败：{error}"))?;
    let mut statement =
        source.prepare("SELECT stable_id FROM poem WHERE author = ?1 AND title = ?2")?;
    let mut picked = BTreeSet::new();
    for entry in &vocabulary.reviewed {
        if !entry
            .add
            .iter()
            .any(|tag| ANTHOLOGY_TAGS.contains(&tag.as_str()))
        {
            continue;
        }
        let rows = statement.query_map(params![entry.author, entry.title], |row| {
            row.get::<_, String>(0)
        })?;
        let matched = rows.collect::<rusqlite::Result<Vec<_>>>()?;
        emit(&format!(
            "  评审名单 {} 《{}》 命中 {} 首",
            entry.author,
            entry.title,
            matched.len()
        ));
        picked.extend(matched);
    }
    Ok(picked.into_iter().collect())
}

pub(crate) fn prepare_workspace(workspace: &Path) -> Result<()> {
    if workspace.exists() {
        std::fs::remove_dir_all(workspace)?;
    }
    std::fs::create_dir_all(workspace)?;
    Ok(())
}

/// 把选中的作品抽成一个独立的子集语料库。
///
/// schema 从源库的 `sqlite_master` 原样复制，而不是在这里重抄一份 DDL——重抄的那份
/// 必然会与语料 schema 漂移，而漂移出来的差异只会在生成期才暴露。
pub(crate) fn extract_subset(
    source_path: &Path,
    dest_path: &Path,
    picked: &[String],
) -> Result<()> {
    let dest = Connection::open_with_flags(
        dest_path,
        OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_CREATE
            | OpenFlags::SQLITE_OPEN_URI,
    )?;
    let uri = format!(
        "file:{}?mode=ro",
        source_path.to_str().context("语料库路径含非 UTF-8 字节")?
    );
    dest.execute("ATTACH DATABASE ?1 AS src", params![uri])?;

    let mut statement = dest.prepare(
        "SELECT sql FROM src.sqlite_master \
         WHERE sql IS NOT NULL AND name NOT LIKE 'sqlite_%' \
         ORDER BY CASE type WHEN 'table' THEN 0 ELSE 1 END, name",
    )?;
    let ddl = statement
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    drop(statement);
    for sql in &ddl {
        dest.execute_batch(sql)?;
    }

    dest.execute_batch("CREATE TEMP TABLE picked(id TEXT PRIMARY KEY) WITHOUT ROWID")?;
    {
        let mut insert = dest.prepare("INSERT OR IGNORE INTO picked(id) VALUES (?1)")?;
        for id in picked {
            insert.execute(params![id])?;
        }
    }

    // 顺序受外键约束：tag 与 author 必须先于引用它们的行。
    for sql in [
        "INSERT INTO tag SELECT * FROM src.tag",
        "INSERT INTO author SELECT * FROM src.author \
         WHERE name IN (SELECT author FROM src.poem WHERE stable_id IN (SELECT id FROM picked))",
        "INSERT INTO poem SELECT * FROM src.poem WHERE stable_id IN (SELECT id FROM picked)",
        "INSERT INTO poem_tag SELECT * FROM src.poem_tag WHERE poem_id IN (SELECT id FROM picked)",
        "INSERT INTO poem_rhyme_group SELECT * FROM src.poem_rhyme_group \
         WHERE poem_id IN (SELECT id FROM picked)",
        "INSERT INTO commentary SELECT * FROM src.commentary WHERE poem_id IN (SELECT id FROM picked)",
        "INSERT INTO rhyme SELECT * FROM src.rhyme",
        "INSERT INTO variant_map SELECT * FROM src.variant_map",
        "INSERT INTO corpus_meta SELECT * FROM src.corpus_meta",
    ] {
        dest.execute(sql, [])
            .with_context(|| format!("抽取子集失败：{sql}"))?;
    }
    dest.execute(
        "UPDATE corpus_meta SET poem_count = (SELECT COUNT(*) FROM poem) WHERE singleton = 1",
        [],
    )?;
    dest.execute_batch("DETACH DATABASE src")?;
    Ok(())
}

fn open_read_only(path: &Path) -> Result<Connection> {
    Ok(Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI,
    )?)
}

fn unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or_default()
}
