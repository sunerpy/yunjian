//! 随包赏析数据集的预生成契约：开放权重门禁、逐条溯源与披露校验。
//!
//! # 为什么这一整套是法律约束而不是工程偏好
//!
//! 随包数据集是**由我们分发**的内容，因此它的每一个字都要能回答「你凭什么可以发它」。
//! 研究结论（`.omo/drafts/yunjian.md` 的 C11）逐条读过三家的条款后是这样的：
//!
//! - Anthropic 商用条款把 Output 的权利让给客户，但同时禁止用其服务「构建竞品，
//!   包括训练竞争模型」；
//! - OpenAI 的对应条款**未能核实**（站点返回 403，需要人工阅读）；
//! - DeepSeek 的条款**完全未核实**。
//!
//! 三条里两条是未知。所以随包数据集不走任何闭源 API，而是用**可下载权重**生成——
//! 下载下来的权重不附带「限制再分发模型输出」的 API 条款，那两条未知因此与本产物无关。
//! 这是绕开不确定性，不是在不确定性里赌一把。
//!
//! 于是本模块把三件事做成**可执行的门禁**而不是文档里的一句话：
//!
//! 1. [`OpenWeightModel::new`] 拒绝任何不在 [`OPEN_WEIGHT_LICENSES`] 里的许可，
//!    并单独识别 [`ProviderKind`] 里那些闭源 API 供应商，点名开放权重要求；
//! 2. [`PregeneratedDataset::push`] 拒绝溯源字段不全、`reviewed` 不为 `false`、
//!    或「声明未执行推理却带着像模型输出的正文」的记录；
//! 3. [`ensure_disclosure`] 在打包写盘**之前**校验披露文本，缺任一要点即中止。
//!
//! # 键在 `stable_id`，不是 `content_hash`
//!
//! 与 [`crate::cache`] 的随包表完全一致，理由也一致：上游宋词有约 4,278 处待修讹误
//! （`.omo/drafts/yunjian.md` 的 A6），一次文本修正会换掉 `content_hash`，键在它上面
//! 的整个数据集会当场变成孤儿。`stable_id` 锚在与内容无关的 `source_locator` 上，
//! 修正后仍指向同一首诗；而「这条赏析是不是对着改动前的文本写的」由
//! `grounding_digest` 单独回答——两个字段各管一件事，不可互相替代。
//!
//! # 用户自费生成的内容永远不进这里
//!
//! [`existing_pregenerated_ids`] 只读 [`SHIPPED_TABLE`]，[`ensure_readable_table`] 对
//! [`LOCAL_CACHE_TABLE`] 硬失败。用户拿自己的 key 生成的赏析是**他的** Output，
//! 未经同意收进公开数据集是越界的，与它质量如何无关。
//!
//! # 生成期允许如实降级，发布期不允许
//!
//! 上面那套门禁管的是「这条内容我们凭什么可以发」，它在没有推理条件的机器上也要能跑完
//! ——所以未执行推理时产物如实标 `generation_executed=false`、每条正文写
//! [`NOT_GENERATED_MARKER`]，这是**正确行为**：本地开发与 CI 的管线校验都需要它。
//!
//! 但「允许生成一份占位」与「允许把占位发出去」是两件事，而此前只有前者被表达出来，
//! 于是每一次 Release 都发出了 16 条占位。[`ensure_releasable`] 补的正是后者：它是**发布
//! 侧**的裁决，`generation_executed=false` 在这里是硬失败。两个判据分处两个函数而不是
//! 合成一个开关，因为它们回答的是不同的问题——「这次跑没跑推理」与「这份产物能不能发」。
//!
//! # 为什么发布侧不只看 `generation_executed`
//!
//! `generation_executed` 是产物**自述**的一个布尔值，手写一份 JSON 就能把它写成 `true`。
//! 所以发布侧不采信任何自述字段，而是拿**待发布语料**把能重算的都重算一遍：覆盖集必须与
//! 语料解析出的那一组 `stable_id` 相等，每条的 `grounding_digest` 必须等于用同一个
//! [`crate::provider::AppreciationRequest`] 对同一首诗重新渲染出来的那个值
//! （见 [`ReleaseExpectation`]）。伪造者要过这道门就必须真的跑通整条抽子集 + 渲染事实块的
//! 管线，绕不过去；跑通之后他仅剩 `text` 一个自由度，而那时他做的事已经是「人手写赏析」，
//! 与「倒一份占位」不是同一个代价量级。
//!
//! **不声称密码学上不可伪造。** 没有可信签名方时，「这段文字出自某个 7B 权重」本身无法证明。
//! 能做的是把每一条可重算的性质都重算、把最便宜的伪造形态（占位、把一段话复制 16 份、
//! 长度不足的存根）逐个堵死，并把剩下那部分如实说清楚——见 `dataset/README.md`。

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use rusqlite::{Connection, OpenFlags};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::cache::ShippedAppreciation;
use crate::genai_provider::ProviderKind;
use yunjian_core::{Error, Result};

/// 数据集文件的 schema 版本。
pub const DATASET_SCHEMA_VERSION: u32 = 1;

/// 允许用于生成随包数据集的**权重**许可（SPDX）。
///
/// 只有两项，与 `models.toml` 的语音模型允许列表刻意保持同一口径：这两个许可对
/// 「拿权重跑出来的东西怎么用」不设条件，因此产物可以被我们再分发。放宽这个列表
/// 等于放宽随包内容的授权基础，必须是一次显式的、有人看过条款的改动。
pub const OPEN_WEIGHT_LICENSES: [&str; 2] = ["MIT", "Apache-2.0"];

/// 允许用于生成随包数据集的**运行时**标识。
///
/// 判据是「这个运行时加载的是本地权重文件，而不是替某家托管服务转发请求」。
/// 名字之外没有别的判据可用，所以这是一个白名单而不是一条规则——白名单会在
/// 有人想加新运行时的时候逼出一次显式改动。
pub const OPEN_WEIGHT_PROVIDERS: [&str; 4] = ["ollama", "vllm", "llama-cpp", "local-weights"];

/// 随包层表名。预生成唯一允许读取的赏析表。
pub const SHIPPED_TABLE: &str = "appreciation_shipped";

/// 用户自费层表名。预生成**禁止**读取。
pub const LOCAL_CACHE_TABLE: &str = "appreciation_cache";

/// 未真正执行推理时写入正文的显式标记。
///
/// 它存在的理由是：管道、门禁与溯源字段都可以在没有推理条件的机器上验证，
/// 但那种情况下产物**不能长得像**真跑过。这个标记让「本条不是模型输出」写在
/// 数据里而不是只写在报告里，[`PregeneratedDataset::push`] 双向校验它。
pub const NOT_GENERATED_MARKER: &str = "<<未生成：本条不是模型输出，需开放权重模型推理>>";

/// 发布侧接受的赏析正文字符数下界。
///
/// [`PregeneratedDataset::push`] 只拒绝**空**正文与恰好等于 [`NOT_GENERATED_MARKER`] 的正文，
/// 于是一条 `"略"` 能过它。实测 16 条真输出是 187–506 字，这个下界离最短的那条还有一倍余量，
/// 因此它拒绝的只会是存根而不会是短赏析。
pub const MIN_APPRECIATION_CHARS: usize = 80;

/// 权重摘要的十六进制长度（SHA-256）。
pub const MODEL_DIGEST_HEX_LEN: usize = 64;

/// 覆盖集使用的选本标签。**显式声明覆盖目标**，不尝试全语料。
///
/// 全语料 47 万首里绝大多数没人会去读赏析，逐首生成既让成本失控也让产物体积失控。
/// 这四个选本是「读者真的会去看赏析」的那个范围，合计数千首。
pub const ANTHOLOGY_TAGS: [&str; 4] = ["唐诗三百首", "宋词三百首", "千家诗", "古诗文名篇"];

/// `dataset/README.md` 必须逐条覆盖的披露要点。
///
/// 每一条都对应一项实际风险，缺任一条这份数据集就不该被分发：
/// 前三条是 C11 记下的准确性披露义务（模型会编造典故、错置作者），
/// 第四条把上游「不得用于训练竞品模型」的条件向下游传递。
pub const DISCLOSURE_MARKERS: [&str; 5] = [
    "AI-generated",
    "未经领域专家审校",
    "编造",
    "独立核实",
    "competing",
];

/// 一次通过开放权重门禁的生成配置。
///
/// 构造它是唯一进入数据集的入口，因此「许可与运行时是否合规」在有任何记录产生
/// **之前**就已判定完毕。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenWeightModel {
    /// 权重标识，例如 `deepseek-r1:7b`。
    pub model: String,
    /// 权重许可（SPDX），必须在 [`OPEN_WEIGHT_LICENSES`] 内。
    pub model_license: String,
    /// 本地运行时标识，必须在 [`OPEN_WEIGHT_PROVIDERS`] 内。
    pub provider: String,
}

impl OpenWeightModel {
    /// 校验并构造。
    ///
    /// # Errors
    ///
    /// 运行时是 [`ProviderKind`] 里的闭源 API 供应商时返回
    /// [`Error::PregenerationClosedProvider`]；运行时不在白名单内、或许可不在
    /// [`OPEN_WEIGHT_LICENSES`] 内时返回 [`Error::PregenerationRejected`]。
    pub fn new(
        model: impl Into<String>,
        model_license: impl Into<String>,
        provider: impl Into<String>,
    ) -> Result<Self> {
        let model = model.into();
        let model_license = model_license.into();
        let provider = provider.into();

        if model.trim().is_empty() {
            return Err(Error::PregenerationRejected(
                "权重标识不能为空；数据集的每条记录都要能指回具体权重".to_owned(),
            ));
        }

        if let Some(kind) = closed_api_provider(&provider) {
            return Err(Error::PregenerationClosedProvider {
                provider: kind.as_str().to_owned(),
            });
        }
        if !OPEN_WEIGHT_PROVIDERS.contains(&provider.as_str()) {
            return Err(Error::PregenerationRejected(format!(
                "运行时 `{provider}` 不在开放权重运行时白名单 {OPEN_WEIGHT_PROVIDERS:?} 内；\
                 随包数据集只能由加载本地权重的运行时生成"
            )));
        }
        if !OPEN_WEIGHT_LICENSES.contains(&model_license.as_str()) {
            return Err(Error::PregenerationRejected(format!(
                "权重许可 `{model_license}` 不在开放权重许可白名单 {OPEN_WEIGHT_LICENSES:?} 内；\
                 只有这些许可对「权重跑出来的产物如何再分发」不设条件"
            )));
        }
        Ok(Self {
            model,
            model_license,
            provider,
        })
    }
}

/// 若 `provider` 命中某个闭源 API 供应商则返回它。
///
/// 名单**由 [`ProviderKind::ALL`] 派生**而不是另抄一份：BYOK 侧新增一个供应商时，
/// 它自动被本门禁认作闭源，不需要有人记得同步第二份名单。唯一的例外是
/// [`ProviderKind::Ollama`]——它加载的是本地权重，不是托管服务。
#[must_use]
pub fn closed_api_provider(provider: &str) -> Option<ProviderKind> {
    ProviderKind::ALL
        .iter()
        .copied()
        .filter(|kind| !matches!(kind, ProviderKind::Ollama))
        .find(|kind| kind.as_str() == provider)
}

/// 数据集里的一条预生成赏析。
///
/// 键是 `stable_id`。`reviewed` 恒为 `false` 且由 [`PregeneratedDataset::push`] 强制——
/// 一条声称已审校的 AI 赏析在本项目里没有产生途径，出现即是数据被篡改。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PregeneratedRecord {
    /// 作品的稳定标识。**键**。
    pub stable_id: String,
    /// 题目。人工复核时用，不参与任何查找。
    pub title: String,
    /// 作者。同上。
    pub author: String,
    /// 命中的选本标签，有序。
    pub anthology_tags: Vec<String>,
    /// 生成模型。
    pub model: String,
    /// 权重许可（SPDX）。
    pub model_license: String,
    /// 本地运行时标识。
    pub provider: String,
    /// 生成完成时的 Unix 秒数。
    pub generated_at: u64,
    /// 提示词模板版本。
    pub template_version: String,
    /// 生成时事实块的摘要。
    pub grounding_digest: String,
    /// 是否经过人工审校。恒为 `false`。
    pub reviewed: bool,
    /// 赏析正文；未执行推理时为 [`NOT_GENERATED_MARKER`]。
    pub text: String,
}

impl PregeneratedRecord {
    /// 转换成可直接喂进 [`crate::cache::AppreciationCache::insert_shipped`] 的种子行。
    ///
    /// 字段与 `appreciation_shipped` 的列一一对应；缺任何一项随包命中都会退化成
    /// 「有行但用不上」。
    #[must_use]
    pub fn to_shipped(&self) -> ShippedAppreciation {
        ShippedAppreciation {
            stable_id: self.stable_id.clone(),
            template_version: self.template_version.clone(),
            model: self.model.clone(),
            model_license: self.model_license.clone(),
            grounding_digest: self.grounding_digest.clone(),
            text: self.text.clone(),
            generated_at: self.generated_at,
        }
    }
}

/// 一份待写盘的数据集。
///
/// 顶层刻意是**记录数组**而不是带元数据的对象：验收断言是
/// `jq -e 'all(.[]; …)' dataset/appreciations.json`，而 `all(.[]; …)` 遍历的是数组元素。
/// 元数据另存 [`DatasetManifest`]，因此加元数据永远不会改变数据集文件的形状。
#[derive(Debug, Clone, Default)]
pub struct PregeneratedDataset {
    records: Vec<PregeneratedRecord>,
    generation_executed: bool,
}

impl PregeneratedDataset {
    /// 新建一份数据集。
    ///
    /// `generation_executed` 声明本次是否真的跑了推理。它进入 [`Self::push`] 的判据：
    /// 声明未执行时每条正文必须是 [`NOT_GENERATED_MARKER`]，声明已执行时反之。
    #[must_use]
    pub fn new(generation_executed: bool) -> Self {
        Self {
            records: Vec::new(),
            generation_executed,
        }
    }

    /// 本次是否真的执行了推理。
    #[must_use]
    pub const fn generation_executed(&self) -> bool {
        self.generation_executed
    }

    /// 已收录的记录。
    #[must_use]
    pub fn records(&self) -> &[PregeneratedRecord] {
        &self.records
    }

    /// 校验并收录一条记录。
    ///
    /// # Errors
    ///
    /// 任一溯源字段为空、`reviewed` 不为 `false`、正文与
    /// [`Self::generation_executed`] 的声明不一致、许可不在
    /// [`OPEN_WEIGHT_LICENSES`] 内、运行时不合规、或 `stable_id` 重复时返回
    /// [`Error::PregenerationRejected`]。
    pub fn push(&mut self, record: PregeneratedRecord) -> Result<()> {
        // 许可与运行时走与首次配置完全相同的那一个判据，不在这里另写一遍：
        // 两处判据一旦分叉，就会出现「配置时被拒、逐条时放行」的缝。
        OpenWeightModel::new(&record.model, &record.model_license, &record.provider)?;

        for (field, value) in [
            ("stable_id", record.stable_id.as_str()),
            ("template_version", record.template_version.as_str()),
            ("grounding_digest", record.grounding_digest.as_str()),
            ("text", record.text.as_str()),
        ] {
            if value.trim().is_empty() {
                return Err(Error::PregenerationRejected(format!(
                    "记录 `{}` 的 {field} 为空；缺溯源字段的记录不得进入随包数据集",
                    record.stable_id
                )));
            }
        }

        if record.reviewed {
            return Err(Error::PregenerationRejected(format!(
                "记录 `{}` 的 reviewed 为 true；本项目没有产生已审校 AI 赏析的途径，\
                 该字段恒为 false",
                record.stable_id
            )));
        }

        let marker = record.text == NOT_GENERATED_MARKER;
        if self.generation_executed && marker {
            return Err(Error::PregenerationRejected(format!(
                "记录 `{}` 声明已执行推理却带着未生成标记；两者只能有一个成立",
                record.stable_id
            )));
        }
        if !self.generation_executed && !marker {
            return Err(Error::PregenerationRejected(format!(
                "记录 `{}` 声明未执行推理，正文必须恰好是 `{NOT_GENERATED_MARKER}`；\
                 未跑推理的产物不得长得像模型输出",
                record.stable_id
            )));
        }

        if self
            .records
            .iter()
            .any(|existing| existing.stable_id == record.stable_id)
        {
            return Err(Error::PregenerationRejected(format!(
                "`stable_id` `{}` 重复；随包表以 (stable_id, template_version) 为主键，\
                 重复行会互相覆盖",
                record.stable_id
            )));
        }

        self.records.push(record);
        Ok(())
    }

    /// 渲染成待写盘的 JSON 数组文本（末尾带换行）。
    ///
    /// # Errors
    ///
    /// 序列化失败时返回 [`Error::Corpus`]。
    pub fn to_json(&self) -> Result<String> {
        let mut json = serde_json::to_string_pretty(&self.records)
            .map_err(|error| Error::Corpus(format!("序列化赏析数据集失败：{error}")))?;
        json.push('\n');
        Ok(json)
    }

    /// 转成可直接导入随包层的种子行，按 `stable_id` 有序。
    #[must_use]
    pub fn to_shipped(&self) -> Vec<ShippedAppreciation> {
        let mut rows = self
            .records
            .iter()
            .map(PregeneratedRecord::to_shipped)
            .collect::<Vec<_>>();
        rows.sort_by(|left, right| left.stable_id.cmp(&right.stable_id));
        rows
    }
}

/// 与数据集一同发布的清单。
///
/// `sha256` 用 SHA-256 而不是 BLAKE3：它要能被 `sha256sum` 与 GitHub 侧的摘要互相
/// 核对，这与语料工件、模型权重的旁文件是同一个口径。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DatasetManifest {
    /// 数据集 schema 版本。
    pub schema_version: u32,
    /// 生成用的提示词模板版本。
    pub template_version: String,
    /// 覆盖集使用的选本标签。
    pub coverage_tags: Vec<String>,
    /// 覆盖集是怎么筛出来的，见 [`CoverageSelector`]。
    pub coverage_selector: String,
    /// 记录条数。
    pub record_count: usize,
    /// 生成模型。
    pub model: String,
    /// 权重许可（SPDX）。
    pub model_license: String,
    /// 本地运行时标识。
    pub provider: String,
    /// 运行时自报的权重摘要（十六进制，无 `sha256:` 前缀）；未执行推理时为 `None`。
    ///
    /// 它回答的是「哪个权重摘要产生了这份产物」——`model` 只是一个标签（`deepseek-r1:7b`
    /// 可以指向任何字节），而摘要能与公开的权重仓库互相核对。未执行推理时**没有**这个值可写，
    /// 所以它同时是一个结构性证据：[`ensure_releasable`] 要求发布物必须带它。
    #[serde(default)]
    pub model_digest: Option<String>,
    /// 本次是否真的执行了推理。
    pub generation_executed: bool,
    /// 未执行推理时的原因；执行了则为 `None`。
    pub not_executed_reason: Option<String>,
    /// `appreciations.json` 的 SHA-256。
    pub appreciations_sha256: String,
    /// 语料库版本。
    pub corpus_version: String,
    /// 生成时的 Unix 秒数。
    pub built_at: u64,
}

/// 覆盖集的筛选途径。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoverageSelector {
    /// 语料的 `poem_tag` 表里有选本标签行，直接按标签筛。
    PoemTag,
    /// `poem_tag` 里没有选本行，按 `tags.toml` 的评审名单以 `(作者, 题目)` 解析。
    ReviewedRoster,
}

impl CoverageSelector {
    /// 写进清单的稳定标识。
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::PoemTag => "poem_tag",
            Self::ReviewedRoster => "reviewed_roster",
        }
    }
}

/// 计算内容的 SHA-256 十六进制摘要。
#[must_use]
pub fn sha256_hex(content: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content);
    hasher
        .finalize()
        .iter()
        .fold(String::with_capacity(64), |mut out, byte| {
            use std::fmt::Write as _;
            let _ = write!(out, "{byte:02x}");
            out
        })
}

/// 判定某张赏析表是否允许被预生成路径读取。
///
/// # Errors
///
/// 表是 [`LOCAL_CACHE_TABLE`] 或任何非 [`SHIPPED_TABLE`] 的名字时返回
/// [`Error::PregenerationRejected`]。
pub fn ensure_readable_table(table: &str) -> Result<()> {
    if table == SHIPPED_TABLE {
        return Ok(());
    }
    if table == LOCAL_CACHE_TABLE {
        return Err(Error::PregenerationRejected(format!(
            "预生成拒绝读取 `{LOCAL_CACHE_TABLE}`：那是用户拿自己的 key 生成的内容，\
             属于用户的 Output，未经同意不得收进公开数据集"
        )));
    }
    Err(Error::PregenerationRejected(format!(
        "预生成只允许读取 `{SHIPPED_TABLE}`，收到 `{table}`"
    )))
}

/// 读出随包层已有的 `stable_id`，用于续跑时跳过已完成的作品。
///
/// **只读 [`SHIPPED_TABLE`]。** 用户自费层里有同一首诗也照样重新生成——那条记录不属于
/// 这个数据集，把它当成「已完成」等于悄悄把用户的 Output 收编进来。
///
/// # Errors
///
/// 打开数据库或查询失败时返回 [`Error::Db`]。
pub fn existing_pregenerated_ids(
    appreciation_db: impl AsRef<Path>,
    template_version: &str,
) -> Result<BTreeSet<String>> {
    ensure_readable_table(SHIPPED_TABLE)?;
    let connection = Connection::open_with_flags(
        appreciation_db.as_ref(),
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI,
    )?;
    let mut statement = connection.prepare(&format!(
        "SELECT stable_id FROM {SHIPPED_TABLE} WHERE template_version = ?1 AND stale = 0"
    ))?;
    let rows = statement.query_map([template_version], |row| row.get::<_, String>(0))?;
    Ok(rows.collect::<rusqlite::Result<BTreeSet<_>>>()?)
}

/// 校验披露文本逐条覆盖 [`DISCLOSURE_MARKERS`]。
///
/// 打包步骤在写出任何文件**之前**调用它。删掉披露段的产物不得被发布出去，
/// 所以判定必须在落盘之前——落盘之后再检查就已经晚了。
///
/// # Errors
///
/// 缺任一要点时返回 [`Error::PregenerationRejected`] 并点名缺的是哪一条。
pub fn ensure_disclosure(readme: &str) -> Result<()> {
    let missing = DISCLOSURE_MARKERS
        .iter()
        .filter(|marker| !readme.contains(**marker))
        .copied()
        .collect::<Vec<_>>();
    if missing.is_empty() {
        return Ok(());
    }
    Err(Error::PregenerationRejected(format!(
        "数据集披露缺少要点 {missing:?}；随包 AI 赏析未经领域专家审校，\
         缺披露的数据集不得打包发布"
    )))
}

/// 若正文恰好是某个单元重复 k(>=2) 次，返回那个 k。
///
/// # 为什么判据是「精确整除的重复」而不是相似度
///
/// 它拦的是一种实测存在的伪造形态：一句敷衍话重复六遍，正好越过字数下界。
/// 而**相似度阈值在这里量出来是反的**：真种子 16 条两两最大共同前缀占比 0.191，
/// 那份模板伪造只有 0.160——按前缀相似度设阈会先红掉真产物。所以这里只用一条精确的结构
/// 性质：真种子 16 条的重复倍数全是 1，模板伪造是 6，两者不重叠且不需要挑阈值。
fn self_repetition(text: &str) -> Option<usize> {
    let chars = text.chars().collect::<Vec<_>>();
    let len = chars.len();
    (1..=len / 2)
        .filter(|unit| len % unit == 0)
        .find(|unit| chars.chunks(*unit).all(|chunk| chunk == &chars[..*unit]))
        .map(|unit| len / unit)
}

/// 发布侧从**待发布语料**重算出来的、种子必须与之逐项吻合的事实。
///
/// 每一项都刻意是「由别的代码路径独立算出来的值」而不是种子自述的字段：只比自述字段等于
/// 什么都不比。[`ReleaseExpectation::grounding`] 尤其如此——它既是覆盖集（键集），
/// 也是每首诗的事实块摘要（值），而两者都只能由打开语料、抽子集、渲染
/// [`crate::provider::AppreciationRequest`] 得到。
#[derive(Debug, Clone, Copy)]
pub struct ReleaseExpectation<'a> {
    /// 本次待发布语料的 `corpus_version`。
    pub corpus_version: &'a str,
    /// 当前代码里的提示词模板版本（[`crate::provider::APPRECIATION_TEMPLATE_VERSION`]）。
    pub template_version: &'a str,
    /// 由待发布语料重算出的 `stable_id -> grounding_digest`。键集即覆盖集。
    pub grounding: &'a BTreeMap<String, String>,
    /// 种子文件字节的实测 SHA-256。
    pub seed_sha256: &'a str,
}

/// 裁决一份种子能否随这次发布发出去。
///
/// 这是**发布门禁**，与 [`PregeneratedDataset::push`] 那道生成期门禁互补而不重叠：
/// 生成期允许如实降级出一份占位（本地开发与 CI 校验管线都需要），发布期一律拒绝。
///
/// # Errors
///
/// 以下任一不成立即返回 [`Error::PregenerationRejected`]，且理由点名真因：
///
/// - `schema_version` 与本代码的 [`DATASET_SCHEMA_VERSION`] 不同；
/// - `generation_executed` 不为 `true`，或它为 `true` 却带着 `not_executed_reason`；
/// - `model_digest` 缺失或不是 [`MODEL_DIGEST_HEX_LEN`] 位小写十六进制；
/// - 模型、许可或运行时不过 [`OpenWeightModel::new`]；
/// - `template_version`、`corpus_version`、`appreciations_sha256`、`record_count`
///   与 [`ReleaseExpectation`] 或记录实况对不上；
/// - 任一记录不过逐条生成期门禁（空字段、`reviewed=true`、重复 `stable_id`、许可越界）；
/// - 任一正文**含有** [`NOT_GENERATED_MARKER`]、短于 [`MIN_APPRECIATION_CHARS`]、
///   与另一条逐字相同、或是同一段文字重复多次凑出的长度；
/// - 任一记录的溯源字段与清单不一致；
/// - 记录的 `stable_id` 集合不等于 [`ReleaseExpectation::grounding`] 的键集；
/// - 任一记录的 `grounding_digest` 不等于重算值。
pub fn ensure_releasable(
    manifest: &DatasetManifest,
    records: &[PregeneratedRecord],
    expected: &ReleaseExpectation<'_>,
) -> Result<()> {
    let reject = |message: String| Err(Error::PregenerationRejected(message));

    if manifest.schema_version != DATASET_SCHEMA_VERSION {
        return reject(format!(
            "种子 schema 版本 {} 与本代码的 {DATASET_SCHEMA_VERSION} 不同；\
             跨 schema 的种子不得发布",
            manifest.schema_version
        ));
    }

    if !manifest.generation_executed {
        return reject(format!(
            "种子清单 `generation_executed=false`：{}。这份产物的每条正文都是未生成标记，\
             不是模型输出——生成期允许如实降级出它，发布期不允许把它发给用户",
            manifest
                .not_executed_reason
                .as_deref()
                .unwrap_or("清单未给出原因")
        ));
    }
    if let Some(reason) = manifest.not_executed_reason.as_deref() {
        return reject(format!(
            "种子清单同时声明 `generation_executed=true` 与未执行原因 `{reason}`；\
             两者只能有一个成立，这份清单是被改过的"
        ));
    }

    let Some(digest) = manifest.model_digest.as_deref() else {
        return reject(
            "种子清单没有 `model_digest`：真跑过推理的运行时报得出权重摘要，没跑过的报不出。\
             缺它的产物无法回答「由哪个权重产生」，不得发布"
                .to_owned(),
        );
    };
    if digest.len() != MODEL_DIGEST_HEX_LEN
        || !digest
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return reject(format!(
            "种子清单的 `model_digest` `{digest}` 不是 {MODEL_DIGEST_HEX_LEN} 位小写十六进制；\
             它必须能与公开权重仓库的摘要逐字互校"
        ));
    }

    OpenWeightModel::new(&manifest.model, &manifest.model_license, &manifest.provider)?;

    if manifest.template_version != expected.template_version {
        return reject(format!(
            "种子按模板 {} 生成，本次发布的代码用模板 {}；提示词换了而种子没重生成，\
             发出去的正文与应用的缓存键对不上",
            manifest.template_version, expected.template_version
        ));
    }
    if manifest.corpus_version != expected.corpus_version {
        return reject(format!(
            "种子声明 corpus_version {}，本次待发布语料是 {}；\
             版本不一致的种子会被导入侧的兼容矩阵拒绝",
            manifest.corpus_version, expected.corpus_version
        ));
    }
    if manifest.appreciations_sha256 != expected.seed_sha256 {
        return reject(format!(
            "种子文件实测摘要 {} 与清单声明 {} 不同；清单描述的不是这个文件",
            expected.seed_sha256, manifest.appreciations_sha256
        ));
    }
    if manifest.record_count != records.len() {
        return reject(format!(
            "清单声明 {} 条，种子文件里实有 {} 条",
            manifest.record_count,
            records.len()
        ));
    }
    if records.is_empty() {
        return reject("种子里没有任何记录；空种子发出去等于没有随包赏析".to_owned());
    }

    // 逐条先过生成期那道门禁，不在这里另抄一遍空字段、`reviewed`、重复键与许可的判据：
    // 两处判据一旦分叉就会出现「生成时被拒、发布时放行」的缝，而那条缝的方向恰好最危险。
    let mut replay = PregeneratedDataset::new(true);
    for record in records {
        replay.push(record.clone())?;
    }

    let mut seen_texts = BTreeSet::new();
    for record in records {
        let text = record.text.trim();
        if text.contains(NOT_GENERATED_MARKER) {
            return reject(format!(
                "记录 `{}` 的正文含未生成标记；`push` 只拦逐字相等，\
                 发布侧要拦的是「标记被塞在一段话里」这种形态",
                record.stable_id
            ));
        }
        let chars = text.chars().count();
        if chars < MIN_APPRECIATION_CHARS {
            return reject(format!(
                "记录 `{}` 的正文只有 {chars} 字，短于发布下界 {MIN_APPRECIATION_CHARS} 字；\
                 那不是一段赏析",
                record.stable_id
            ));
        }
        if !seen_texts.insert(text.to_owned()) {
            return reject(format!(
                "记录 `{}` 的正文与另一条逐字相同；把一段话复制成整份数据集是最省事的伪造形态，\
                 而真实推理不会对不同的诗给出同一段文字",
                record.stable_id
            ));
        }
        if let Some(times) = self_repetition(text) {
            return reject(format!(
                "记录 `{}` 的正文是同一段文字重复 {times} 次凑出来的长度；\
                 那是为了越过字数下界而拼的模板，不是赏析",
                record.stable_id
            ));
        }

        for (field, actual, declared) in [
            ("model", record.model.as_str(), manifest.model.as_str()),
            (
                "model_license",
                record.model_license.as_str(),
                manifest.model_license.as_str(),
            ),
            (
                "provider",
                record.provider.as_str(),
                manifest.provider.as_str(),
            ),
            (
                "template_version",
                record.template_version.as_str(),
                expected.template_version,
            ),
        ] {
            if actual != declared {
                return reject(format!(
                    "记录 `{}` 的 {field} 是 `{actual}`，与清单/本代码声明的 `{declared}` 不同",
                    record.stable_id
                ));
            }
        }

        let Some(recomputed) = expected.grounding.get(&record.stable_id) else {
            return reject(format!(
                "记录 `{}` 不在本次待发布语料解析出的覆盖集里；\
                 种子只能覆盖这次真的要发的那些作品",
                record.stable_id
            ));
        };
        if &record.grounding_digest != recomputed {
            return reject(format!(
                "记录 `{}` 的 grounding_digest 是 {}，而用待发布语料重算得到 {recomputed}；\
                 这条赏析不是对着这次要发的文本写的",
                record.stable_id, record.grounding_digest
            ));
        }
    }

    let present = records
        .iter()
        .map(|record| record.stable_id.clone())
        .collect::<BTreeSet<_>>();
    let missing = expected
        .grounding
        .keys()
        .filter(|id| !present.contains(*id))
        .cloned()
        .collect::<Vec<_>>();
    if !missing.is_empty() {
        return reject(format!(
            "待发布语料解析出 {} 首，种子只覆盖 {} 首，缺 {missing:?}；\
             部分覆盖的种子会让用户在剩下那些作品上看不到随包赏析",
            expected.grounding.len(),
            present.len()
        ));
    }

    Ok(())
}

#[cfg(test)]
mod tests;
