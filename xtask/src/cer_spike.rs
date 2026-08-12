//! `xtask cer-spike`：文言 CER 实测。
//!
//! ## 为什么不用真人朗读
//!
//! 唯一公开的中文朗诵语料 MCGA 是 CC BY-NC-SA-4.0 且只放了 test split，NC 条款直接
//! 排除本项目（记在 `corpus/DENYLIST.md`）。本 todo 因此**刻意不以真人录音为门禁**：
//! 参考音频由 `models.toml` 里两把已核实许可的 TTS 音色合成，再叠加信道与语速增强。
//!
//! 这样得到的 CER 是真人 CER 的**乐观上界**，报告里必须原样写明。乐观上界的用途是
//! 单向的：它足以证伪（上界都超过 10%，真人只会更差，语音评分必须退到
//! `guided_practice`），但不足以证成（上界达标不代表真人达标）。2026-08-11 的裁决进一步
//! 判定：CER 77.01% 下送进对齐的文本本身是噪声，**完整度并不比字准更可信**，所以早先的
//! `completeness_only` 取值也被废掉了。裁决只在 `guided_practice` 与 `coverage_advisory`
//! 之间选，**永远没有 `full`，也不再有 `completeness_only`**。
//!
//! ## fixture 从哪来
//!
//! 不手打。50 首取自已核实为 MIT 的 `chinese-poetry` 在**锁定 revision**
//! 上的 `蒙学/tangshisanbaishou.json` 与 `纳兰性德/纳兰性德诗集.json`，两者在
//! `corpus/sources.toml` 里都是 `license_class = "public_domain"` 且 `shippable = true`。
//! 体裁标签直接用上游自带的 `type` 字段（五言絕句 / 七言律詩 / 樂府 …），因此逐体裁
//! CER 表的分组不是本项目现编的。
//!
//! 选取规则是**每个体裁桶按上游顺序取前 N 首**。不做人工挑选：唐诗三百首本身就是
//! 名篇选集，其任意子集都是名篇，而人工挑选既不可复现又会引入选择偏差。

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::verify_sources::emit;

const FIXTURES: &str = "crates/yunjian-voice/tests/fixtures/cer-poems.toml";
const REPORT_JSON: &str = "docs/reports/asr-cer.json";
const REPORT_MD: &str = "docs/reports/asr-cer.md";
/// `--force-cer` 的产物落这里，不覆盖实测报告，也不入库。
const SELFCHECK_JSON: &str = "docs/reports/asr-cer.selfcheck.json";
const SELFCHECK_MD: &str = "docs/reports/asr-cer.selfcheck.md";

/// 事先声明的分叉阈值。增强集上的总 CER 超过它，语音练习退到 `guided_practice`。
/// 看到数字之后不得重新协商。
pub const CER_THRESHOLD: f64 = 0.10;

/// 上游锁定 revision，与 `corpus/sources.toml` 里 `chinese-poetry/chinese-poetry` 的一致。
const CORPUS_REV: &str = "b8594f81a89752241442f2ce267d6f66f96704ee";
const TANGSHI_PATH: &str = "蒙学/tangshisanbaishou.json";
const NALAN_PATH: &str = "纳兰性德/纳兰性德诗集.json";

/// 每个体裁桶取多少首。合计 50。
const QUOTAS: &[(&str, usize)] = &[
    ("五言絕句", 8),
    ("七言絕句", 10),
    ("五言律詩", 8),
    ("七言律詩", 8),
    ("五言古詩", 5),
    ("七言古詩", 3),
    ("樂府", 4),
    ("词", 4),
];

/// 单条参考文本的字数上限。
///
/// 不是排版偏好，是 Whisper 的硬约束：它的输入是 30 秒窗口，超出部分直接被丢弃。
/// 中文合成约 4-5 字/秒，72 字约 15-18 秒，留足余量。超限的长篇（古诗、乐府）
/// 按整句截到上限内，并在 fixture 里标 `excerpt = true`——截断这件事必须显式，
/// 否则 CER 会因为「后半段根本没被识别」而恶化，看起来却像模型不行。
const MAX_CHARS: usize = 72;

// ---------------------------------------------------------------------------
// fixture
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, Deserialize)]
struct FixtureSet {
    schema_version: u32,
    source_repo: String,
    source_rev: String,
    #[serde(rename = "poem")]
    poems: Vec<Poem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Poem {
    id: String,
    genre: String,
    title: String,
    author: String,
    /// 规范简体正文，含标点。TTS 读的是它。
    body: String,
    /// 只留汉字的参考串。CER 按它计算——标点、空白与英文在语音里没有对应物，
    /// 计进去会把「模型不输出标点」误算成识别错误。
    reference: String,
    source_path: String,
    excerpt: bool,
}

/// 从锁定 revision 重建 fixture。需要网络。
pub fn refresh(root: &Path) -> Result<()> {
    let tangshi = fetch_json(&raw_url(TANGSHI_PATH))?;
    let nalan = fetch_json(&raw_url(NALAN_PATH))?;

    let converter = ferrous_opencc::OpenCC::from_config(ferrous_opencc::config::BuiltinConfig::T2s)
        .map_err(|e| anyhow::anyhow!("OpenCC t2s 配置加载失败：{e}"))?;

    let mut poems: Vec<Poem> = Vec::new();

    let buckets = tangshi
        .get("content")
        .and_then(|c| c.as_array())
        .context("tangshisanbaishou.json 缺少 content 数组")?;

    for (genre, quota) in QUOTAS {
        if *genre == "词" {
            continue;
        }
        let bucket = buckets
            .iter()
            .find(|b| b.get("type").and_then(|t| t.as_str()) == Some(*genre))
            .with_context(|| format!("上游没有体裁桶 `{genre}`"))?;
        let items = bucket
            .get("content")
            .and_then(|c| c.as_array())
            .with_context(|| format!("体裁桶 `{genre}` 缺少 content"))?;
        let mut taken = 0usize;
        for item in items {
            if taken == *quota {
                break;
            }
            let title = converter.convert(item["chapter"].as_str().unwrap_or_default());
            let author = converter.convert(item["author"].as_str().unwrap_or_default());
            let lines: Vec<String> = item["paragraphs"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|l| l.as_str())
                        .map(|l| clean_line(&converter.convert(l)))
                        .filter(|l| !l.is_empty())
                        .collect()
                })
                .unwrap_or_default();
            if let Some(poem) = assemble(&poems, genre, &title, &author, &lines, TANGSHI_PATH) {
                poems.push(poem);
                taken += 1;
            }
        }
        if taken < *quota {
            bail!("体裁 `{genre}` 只取到 {taken} 首，不足 {quota} 首");
        }
    }

    let ci_quota = QUOTAS
        .iter()
        .find(|(g, _)| *g == "词")
        .map(|(_, q)| *q)
        .unwrap_or_default();
    let ci_items = nalan.as_array().context("纳兰性德诗集应是数组")?;
    let mut taken = 0usize;
    for item in ci_items {
        if taken == ci_quota {
            break;
        }
        let title = converter.convert(item["title"].as_str().unwrap_or_default());
        let author = converter.convert(item["author"].as_str().unwrap_or_default());
        let lines: Vec<String> = item["para"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|l| l.as_str())
                    .map(|l| clean_line(&converter.convert(l)))
                    .filter(|l| !l.is_empty())
                    .collect()
            })
            .unwrap_or_default();
        if let Some(poem) = assemble(&poems, "词", &title, &author, &lines, NALAN_PATH) {
            poems.push(poem);
            taken += 1;
        }
    }
    if taken < ci_quota {
        bail!("词只取到 {taken} 首，不足 {ci_quota} 首");
    }

    let set = FixtureSet {
        schema_version: 1,
        source_repo: "chinese-poetry/chinese-poetry".to_owned(),
        source_rev: CORPUS_REV.to_owned(),
        poems,
    };
    let path = root.join(FIXTURES);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut text = String::from(FIXTURE_HEADER);
    text.push_str(&toml::to_string_pretty(&set).context("序列化 fixture 失败")?);
    std::fs::write(&path, text).with_context(|| format!("写出 {} 失败", path.display()))?;
    emit(&format!(
        "已由锁定 revision {} 重建 {}：{} 首",
        &CORPUS_REV[..12],
        FIXTURES,
        set.poems.len()
    ));
    Ok(())
}

const FIXTURE_HEADER: &str = "# 文言 CER 实测的参考文本。**由 `cargo run -p xtask -- cer-spike --refresh-fixtures` 生成，不要手改。**\n\
#\n\
# 全部取自 `chinese-poetry/chinese-poetry` 锁定 revision 上的公有领域文本\n\
# （`corpus/sources.toml` 里 license_class = \"public_domain\"、shippable = true 的资产），\n\
# 繁体经 OpenCC `t2s` 转简体，上游夹注（形如「(明年 一作：年年)」）已剥除。\n\
#\n\
# `reference` 只留汉字：标点与空白在语音里没有对应物，计进 CER 会把「模型不输出标点」\n\
# 误算成识别错误。`excerpt = true` 表示原作超过 Whisper 的 30 秒窗口，已按整句截断。\n\n";

/// 组装一首，去重并按 `MAX_CHARS` 截断。返回 `None` 表示该条不可用（重复或空）。
fn assemble(
    existing: &[Poem],
    genre: &str,
    title: &str,
    author: &str,
    lines: &[String],
    source_path: &str,
) -> Option<Poem> {
    if title.is_empty() || lines.is_empty() {
        return None;
    }
    let id = format!("{genre}:{author}:{title}");
    if existing.iter().any(|p| p.id == id) {
        return None;
    }

    let mut body = String::new();
    let mut excerpt = false;
    for line in lines {
        if body.chars().count() + line.chars().count() > MAX_CHARS {
            excerpt = true;
            break;
        }
        body.push_str(line);
    }
    if body.is_empty() {
        return None;
    }
    let reference: String = body.chars().filter(|c| is_han(*c)).collect();
    if reference.chars().count() < 8 {
        return None;
    }

    Some(Poem {
        id,
        genre: genre.to_owned(),
        title: title.to_owned(),
        author: author.to_owned(),
        body,
        reference,
        source_path: source_path.to_owned(),
        excerpt,
    })
}

/// 剥除上游的编者夹注与空白。
///
/// 夹注形如「(明年 一作：年年)」「(顰 一作：蹙)」，是校勘信息而非正文；读出来会
/// 直接污染参考文本。半角与全角括号都要处理——上游两种都用过。
fn clean_line(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut depth = 0usize;
    for ch in line.chars() {
        match ch {
            '(' | '（' => depth += 1,
            ')' | '）' => depth = depth.saturating_sub(1),
            _ if depth == 0 && !ch.is_whitespace() => out.push(ch),
            _ => {}
        }
    }
    out
}

fn is_han(c: char) -> bool {
    matches!(c, '\u{3400}'..='\u{4dbf}' | '\u{4e00}'..='\u{9fff}' | '\u{f900}'..='\u{fa6f}')
}

fn raw_url(path: &str) -> String {
    let encoded: String = path.split('/').map(urlencode).collect::<Vec<_>>().join("/");
    format!(
        "https://raw.githubusercontent.com/chinese-poetry/chinese-poetry/{CORPUS_REV}/{encoded}"
    )
}

fn urlencode(seg: &str) -> String {
    let mut out = String::new();
    for b in seg.as_bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(char::from(*b));
        } else {
            let _ = write!(out, "%{b:02X}");
        }
    }
    out
}

fn fetch_json(url: &str) -> Result<serde_json::Value> {
    let mut resp = ureq::get(url)
        .call()
        .with_context(|| format!("请求 {url} 失败"))?;
    let bytes = resp
        .body_mut()
        .read_to_vec()
        .with_context(|| format!("读取 {url} 响应体失败"))?;
    serde_json::from_slice(&bytes).with_context(|| format!("解析 {url} 的 JSON 失败"))
}

fn load_fixtures(root: &Path) -> Result<FixtureSet> {
    let path = root.join(FIXTURES);
    let text = std::fs::read_to_string(&path).with_context(|| {
        format!(
            "读取 {} 失败；先跑 `cargo run -p xtask -- cer-spike --refresh-fixtures` 生成",
            path.display()
        )
    })?;
    toml::from_str(&text).with_context(|| format!("解析 {} 失败", path.display()))
}

// ---------------------------------------------------------------------------
// CER
// ---------------------------------------------------------------------------

/// 字符错误率：编辑距离 / 参考长度。
///
/// 不开 `voice` 时只有单元测试调用它——那正是「不开语音就不会有测量结果」的体现，
/// 所以此处按 cfg 精确豁免 dead_code，而不是给整个模块开一张通行证。
///
/// 分母是参考长度而非 max(参考, 假设)，这是 CER 的标准定义，代价是插入过多时可以
/// 超过 1.0——那是应有的语义：胡乱输出比什么都不输出更糟。
#[must_use]
#[cfg_attr(
    all(not(feature = "voice"), not(test)),
    allow(dead_code, reason = "无 voice 特性时不存在测量路径，只有测试调用它")
)]
pub fn cer(reference: &str, hypothesis: &str) -> f64 {
    let r: Vec<char> = reference.chars().filter(|c| is_han(*c)).collect();
    let h: Vec<char> = normalize_hypothesis(hypothesis);
    if r.is_empty() {
        return if h.is_empty() { 0.0 } else { 1.0 };
    }
    let distance = levenshtein(&r, &h);
    #[expect(
        clippy::cast_precision_loss,
        reason = "字数在千以内，远未触及 f64 精度边界"
    )]
    let out = distance as f64 / r.len() as f64;
    out
}

/// 识别结果的规范化：只留汉字。
///
/// Whisper 中文常输出繁体，而参考文本是简体。繁简差异是**字形**差异不是识别错误，
/// 把它算进 CER 会得到一个与真实听辨能力无关的数字。因此这里做 t2s 归一。
/// 转换表放在函数外的 `OnceLock` 里：OpenCC 词典加载有成本，逐条转换会让测量时间
/// 被字典 I/O 主导。
#[cfg_attr(
    all(not(feature = "voice"), not(test)),
    allow(dead_code, reason = "只被 `cer` 调用，随它一起在无 voice 构建里不可达")
)]
fn normalize_hypothesis(text: &str) -> Vec<char> {
    static CONVERTER: std::sync::OnceLock<Option<ferrous_opencc::OpenCC>> =
        std::sync::OnceLock::new();
    let converter = CONVERTER.get_or_init(|| {
        ferrous_opencc::OpenCC::from_config(ferrous_opencc::config::BuiltinConfig::T2s).ok()
    });
    let simplified = converter
        .as_ref()
        .map_or_else(|| text.to_owned(), |c| c.convert(text));
    simplified.chars().filter(|c| is_han(*c)).collect()
}

#[cfg_attr(
    all(not(feature = "voice"), not(test)),
    allow(dead_code, reason = "只被 `cer` 调用，随它一起在无 voice 构建里不可达")
)]
fn levenshtein(a: &[char], b: &[char]) -> usize {
    if a.is_empty() {
        return b.len();
    }
    if b.is_empty() {
        return a.len();
    }
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0usize; b.len() + 1];
    for (i, ca) in a.iter().enumerate() {
        cur[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let cost = usize::from(ca != cb);
            cur[j + 1] = (prev[j] + cost).min(prev[j + 1] + 1).min(cur[j] + 1);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

// ---------------------------------------------------------------------------
// 报告
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, Deserialize)]
pub struct Report {
    pub schema_version: u32,
    pub measured: bool,
    pub measured_at: Option<String>,
    /// `guided_practice` | `coverage_advisory`。**永远不会是 `full` 或 `completeness_only`。**
    pub scoring_mode: String,
    pub cer_threshold: f64,
    pub fixture: FixtureSummary,
    pub method: Method,
    /// 未实测时说明阻塞原因与所需条件；实测时为 `null`。
    pub not_measured_reason: Option<String>,
    pub optimistic_bound_statement: String,
    pub overall: Option<Aggregate>,
    #[serde(default)]
    pub per_genre: Vec<GenreRow>,
    #[serde(default)]
    pub per_condition: Vec<ConditionRow>,
    #[serde(default)]
    pub per_model: Vec<ModelRow>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct FixtureSummary {
    pub poems: usize,
    pub genres: usize,
    pub source_repo: String,
    pub source_rev: String,
    pub excerpted: usize,
    pub human_recordings_used: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Method {
    pub tts_models: Vec<String>,
    pub asr_models: Vec<String>,
    pub conditions: Vec<String>,
    pub utterances_planned: usize,
    pub utterances_measured: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Aggregate {
    pub cer: f64,
    pub utterances: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenreRow {
    pub genre: String,
    pub poems: usize,
    pub utterances: usize,
    pub cer: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConditionRow {
    pub condition: String,
    pub utterances: usize,
    pub cer: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelRow {
    pub asr_model: String,
    pub tts_model: String,
    pub utterances: usize,
    pub cer: f64,
}

/// 裁决只有两个取值，`full` 与 `completeness_only` 都不在其中。
///
/// # 为什么 `completeness_only` 也被排除了
///
/// 它是本函数早先的一个取值，2026-08-11 的裁决把它废掉了：它假设开放转写至少还能支撑
/// 「完整度」，但实测 CER 77.01% 意味着送进对齐的文本本身就是噪声，**完整度并不比字准
/// 更可信**。一个读它的下游会去实现一个不可靠的完整度指标，那比不给指标更糟。
///
/// 于是 v1 落在 `guided_practice`：跟读形态，只报「是否开口 / 停顿 / 相对节奏」这类可直接
/// 观测的事实，FSRS 等级由用户自己选。`coverage_advisory` 需要先通过一个独立的 KWS spike
/// （门槛事先冻结、参数禁止逐诗调参、holdout 音色冻结）才允许开放。
#[must_use]
pub fn verdict(overall_cer: Option<f64>) -> &'static str {
    match overall_cer {
        // 即便 CER 低于阈值也只到 `coverage_advisory`：合成上界说明不了真实说话人的情况。
        Some(c) if c <= CER_THRESHOLD => "coverage_advisory",
        // 未实测或超阈值都落最保守的一侧。
        _ => "guided_practice",
    }
}

pub fn render_markdown(report: &Report) -> String {
    let mut md = String::new();
    let _ = writeln!(md, "# 文言语音识别 CER 实测\n");
    let _ = writeln!(
        md,
        "> 由 `cargo run -p xtask -- cer-spike` 生成，机器可读版本见 [`asr-cer.json`](asr-cer.json)。\n"
    );

    let _ = writeln!(md, "## 裁决\n");
    let _ = writeln!(md, "| 项 | 值 |");
    let _ = writeln!(md, "| --- | --- |");
    let _ = writeln!(md, "| `scoring_mode` | **`{}`** |", report.scoring_mode);
    let _ = writeln!(
        md,
        "| 是否实测 | {} |",
        if report.measured {
            "是"
        } else {
            "**否（NOT MEASURED）**"
        }
    );
    let _ = writeln!(
        md,
        "| 总 CER | {} |",
        report.overall.as_ref().map_or_else(
            || "NOT MEASURED".to_owned(),
            |a| format!("{:.2}%", a.cer * 100.0)
        )
    );
    let _ = writeln!(md, "| 阈值 | {:.0}% |", report.cer_threshold * 100.0);
    let _ = writeln!(
        md,
        "| 实测句数 | {} / {} |",
        report.method.utterances_measured, report.method.utterances_planned
    );
    let _ = writeln!(md);

    let _ = writeln!(
        md,
        "`scoring_mode` 的取值域**只有** `guided_practice` 与 `coverage_advisory` 两个，\
         **永远不会是 `full`**。todo 48、51、56、57 读的就是这个字段。\n"
    );
    let _ = writeln!(
        md,
        "- `coverage_advisory`：仅在 KWS spike 通过冻结门槛后开放，报「检测到 N/M 句」这类覆盖度，不报逐字准确率。"
    );
    let _ = writeln!(
        md,
        "- `guided_practice`：v1 契约。跟读形态，只报是否开口／停顿／相对节奏这类可观测事实，FSRS 等级由用户自选。"
    );
    let _ = writeln!(
        md,
        "- 打字路径是确定性比对，字准照常作为分数，不受本裁决影响。\n"
    );

    if let Some(reason) = &report.not_measured_reason {
        let _ = writeln!(md, "## 未实测的原因\n");
        let _ = writeln!(md, "{reason}\n");
    }

    let _ = writeln!(md, "## 这个数字是乐观上界\n");
    let _ = writeln!(md, "{}\n", report.optimistic_bound_statement);

    let _ = writeln!(md, "## 方法\n");
    let _ = writeln!(md, "| 项 | 值 |");
    let _ = writeln!(md, "| --- | --- |");
    let _ = writeln!(
        md,
        "| 参考文本 | {} 首，{} 个体裁 |",
        report.fixture.poems, report.fixture.genres
    );
    let _ = writeln!(
        md,
        "| 文本来源 | `{}` @ `{}` |",
        report.fixture.source_repo,
        &report.fixture.source_rev[..12.min(report.fixture.source_rev.len())]
    );
    let _ = writeln!(
        md,
        "| 因超 30 秒窗口而截断 | {} 首 |",
        report.fixture.excerpted
    );
    let _ = writeln!(
        md,
        "| 是否使用真人录音 | {} |",
        if report.fixture.human_recordings_used {
            "是"
        } else {
            "否（刻意如此，见下）"
        }
    );
    let _ = writeln!(
        md,
        "| TTS 音色 | {} |",
        join_code(&report.method.tts_models)
    );
    let _ = writeln!(
        md,
        "| ASR 模型 | {} |",
        join_code(&report.method.asr_models)
    );
    let _ = writeln!(
        md,
        "| 增强条件 | {} |",
        join_code(&report.method.conditions)
    );
    let _ = writeln!(md);

    if !report.per_genre.is_empty() {
        let _ = writeln!(md, "## 逐体裁 CER\n");
        let _ = writeln!(md, "| 体裁 | 首数 | 句数 | CER |");
        let _ = writeln!(md, "| --- | --: | --: | --: |");
        for row in &report.per_genre {
            let _ = writeln!(
                md,
                "| {} | {} | {} | {:.2}% |",
                row.genre,
                row.poems,
                row.utterances,
                row.cer * 100.0
            );
        }
        let _ = writeln!(md);
    }

    if !report.per_condition.is_empty() {
        let _ = writeln!(md, "## 逐增强条件 CER\n");
        let _ = writeln!(md, "| 条件 | 句数 | CER |");
        let _ = writeln!(md, "| --- | --: | --: |");
        for row in &report.per_condition {
            let _ = writeln!(
                md,
                "| `{}` | {} | {:.2}% |",
                row.condition,
                row.utterances,
                row.cer * 100.0
            );
        }
        let _ = writeln!(md);
    }

    if !report.per_model.is_empty() {
        let _ = writeln!(md, "## 逐模型组合 CER\n");
        let _ = writeln!(md, "| ASR | TTS | 句数 | CER |");
        let _ = writeln!(md, "| --- | --- | --: | --: |");
        for row in &report.per_model {
            let _ = writeln!(
                md,
                "| `{}` | `{}` | {} | {:.2}% |",
                row.asr_model,
                row.tts_model,
                row.utterances,
                row.cer * 100.0
            );
        }
        let _ = writeln!(md);
    }

    if report.measured {
        let _ = writeln!(md, "## 这个数字说明了什么\n");
        let _ = writeln!(
            md,
            "CER 在 {:.0}% 量级，是阈值的七倍以上。这不是「调参能救回来」的差距，\
             而是**任务与模型的不匹配**，有三条独立证据：\n",
            report.overall.as_ref().map_or(0.0, |a| a.cer * 100.0)
        );
        let _ = writeln!(
            md,
            "1. **换更大的模型不管用。** tiny / base / small 三个尺寸的 CER 几乎相同（见逐模型表），"
        );
        let _ = writeln!(
            md,
            "   而正常的「模型太小」应当表现为随尺寸单调下降。它没有下降。"
        );
        let _ = writeln!(
            md,
            "2. **加噪也几乎不管用。** clean 与 10 dB 粉噪的差距只有几个百分点（见逐条件表）。"
        );
        let _ = writeln!(
            md,
            "   一个正常工作的识别器对 10 dB 噪声应当明显更差。它本来就已经在下限附近。"
        );
        let _ = writeln!(
            md,
            "3. **错误形态是「音近字不同」，不是「乱码」。** 逐句转写里典型如"
        );
        let _ = writeln!(
            md,
            "   `寥落古行宫` → `樂國行功`、`床前明月光` → `长前名月光`：声母韵母大致对得上，"
        );
        let _ = writeln!(
            md,
            "   字选错了。也就是说声学前端在工作，**语言模型在把音节往现代汉语常用词上拽**——"
        );
        let _ = writeln!(md, "   而文言恰恰是低频词、单音节词与倒装的密集区。\n");
        let _ = writeln!(
            md,
            "第三条已用**官方 CLI 在同一段音频上交叉验证**过：`sherpa-onnx-offline` 直接跑出的\
             结果同样是 `长前名月光 地上`。因此这不是本项目绑定层的缺陷，而是 Whisper 在文言上的真实表现。\n"
        );
        let _ = writeln!(
            md,
            "**推论**：即便将来换用许可可用的更强模型，也不应假定字准会跨过阈值。语音路径的"
        );
        let _ = writeln!(
            md,
            "设计必须以「字准不可用」为前提，而不是把它当成一个待优化的指标。\n"
        );
    }

    let _ = writeln!(md, "## 复现\n");
    let _ = writeln!(md, "```bash");
    let _ = writeln!(md, "# 1) 核实模型许可并生成 models.lock.json");
    let _ = writeln!(md, "cargo run -p xtask -- verify-models");
    let _ = writeln!(md, "# 2) 按 models.toml 下载并解包权重到 models/cache/");
    let _ = writeln!(
        md,
        "#    （权重不入库，`.gitignore` 已排除 /models/cache/）"
    );
    let _ = writeln!(
        md,
        "# 3) 重建参考文本（需网络，从锁定 revision 取公有领域原文）"
    );
    let _ = writeln!(md, "cargo run -p xtask -- cer-spike --refresh-fixtures");
    let _ = writeln!(
        md,
        "# 4) 实测。需 --features voice，它会拉入 GPL-3.0 的 espeak-ng 原生依赖"
    );
    let _ = writeln!(md, "cargo run -p xtask --features voice -- cer-spike");
    let _ = writeln!(md, "```\n");
    let _ = writeln!(
        md,
        "另有 [`asr-cer-human.md`](asr-cer-human.md) 供将来自愿贡献的真人录音填充。\
         它是**可选**的，不是本结论的门禁。\n"
    );

    md
}

fn join_code(items: &[String]) -> String {
    if items.is_empty() {
        return "（无）".to_owned();
    }
    items
        .iter()
        .map(|s| format!("`{s}`"))
        .collect::<Vec<_>>()
        .join("、")
}

const OPTIMISTIC_STATEMENT: &str = "本表的 CER **由 TTS 合成音加信道增强测得，不是真人朗读**，因此它是真实说话人 CER 的\
**乐观上界**：合成音只有单一说话人、没有口音、没有吞音、没有真实房间的混响与远场衰减，\
韵律也比人朗读规整。增强（8 kHz 窄带往返、20 dB 与 10 dB 粉噪、±10% 变速不变调）\
逼近的是**信道与语速**，逼近不了说话人差异。\n\n\
之所以只能这样测：唯一公开的中文朗诵语料 MCGA 是 CC BY-NC-SA-4.0 且只放了 test split，\
NC 条款排除本项目（见 `corpus/DENYLIST.md`），而本 todo 刻意不以真人录音为门禁。\n\n\
乐观上界的用途是单向的——它足以**证伪**（上界都超过阈值，真人只会更差），\
不足以**证成**（上界达标不代表真人达标）。所以即便通过，语音路径上的逐字准确率也\
永远只是 advisory，不会升为分数。";

// ---------------------------------------------------------------------------
// 入口
// ---------------------------------------------------------------------------

pub fn run(
    root_override: Option<PathBuf>,
    refresh_fixtures: bool,
    force_cer: Option<f64>,
    limit: Option<usize>,
    dump: Option<PathBuf>,
    render_only: bool,
) -> Result<()> {
    let root = root_override.map_or_else(repo_root, Ok)?;

    if refresh_fixtures {
        return refresh(&root);
    }

    if render_only {
        return render_only_from_json(&root);
    }

    let mut fixtures = load_fixtures(&root)?;
    if let Some(n) = limit {
        // 试运行通道：先用两三首确认引擎配置对得上，再跑满 50 首。
        // 报告里 `utterances_measured` 与 `utterances_planned` 的差值会暴露这是子集。
        fixtures.poems.truncate(n);
        emit(&format!(
            "  注意：--limit {n} 生效，只测前 {n} 首，不是完整测量"
        ));
    }
    let full_poem_count = load_fixtures(&root)?.poems.len();
    let genres: BTreeMap<&str, usize> =
        fixtures.poems.iter().fold(BTreeMap::new(), |mut acc, p| {
            *acc.entry(p.genre.as_str()).or_default() += 1;
            acc
        });

    let conditions: Vec<String> = measure::CONDITIONS
        .iter()
        .map(|c| (*c).to_owned())
        .collect();
    // 计入 ASR 模型数：同一条音频过三个识别器就是三条测量。`utterances_measured`
    // 也是这么数的，两个数必须可比，否则「测了 1800 / 计划 600」读起来像是超额完成。
    let planned = full_poem_count
        * measure::TTS_MODELS.len()
        * conditions.len()
        * measure::ASR_MODELS.len().max(1);

    let mut report = Report {
        schema_version: 1,
        measured: false,
        measured_at: None,
        scoring_mode: String::new(),
        cer_threshold: CER_THRESHOLD,
        fixture: FixtureSummary {
            poems: fixtures.poems.len(),
            genres: genres.len(),
            source_repo: fixtures.source_repo.clone(),
            source_rev: fixtures.source_rev.clone(),
            excerpted: fixtures.poems.iter().filter(|p| p.excerpt).count(),
            human_recordings_used: false,
        },
        method: Method {
            tts_models: measure::TTS_MODELS
                .iter()
                .map(|(n, _)| (*n).to_owned())
                .collect(),
            asr_models: measure::ASR_MODELS
                .iter()
                .map(|(n, _)| (*n).to_owned())
                .collect(),
            conditions,
            utterances_planned: planned,
            utterances_measured: 0,
        },
        not_measured_reason: None,
        optimistic_bound_statement: OPTIMISTIC_STATEMENT.to_owned(),
        overall: None,
        per_genre: Vec::new(),
        per_condition: Vec::new(),
        per_model: Vec::new(),
    };

    if let Some(forced) = force_cer {
        // 门禁自检通道：只改「量出来的数」，不改判据。用它验证阈值真的会翻转裁决。
        //
        // **不覆盖已实测的报告**。实测一轮要一小时，自检只要一秒；让一秒的操作毁掉
        // 一小时的产物是不可接受的（实测已经踩过一次）。已实测时自检写到
        // `.selfcheck.json`，该文件不入库，也不在任何下游 todo 的视野里。
        emit(&format!(
            "  注意：--force-cer {forced} 生效，报告为门禁自检产物，不是真实测量"
        ));
        report.measured = false;
        report.overall = Some(Aggregate {
            cer: forced,
            utterances: 0,
        });
        report.not_measured_reason = Some(format!(
            "本报告由 `--force-cer {forced}` 生成，用于验证 {:.0}% 阈值真的会翻转裁决。\
             它**不是**测量结果，不得作为任何结论的依据。",
            CER_THRESHOLD * 100.0
        ));
    } else {
        measure::fill(&fixtures.poems, &mut report, dump.as_deref())?;
    }

    report.scoring_mode = verdict(report.overall.as_ref().map(|a| a.cer)).to_owned();

    let (json_path, md_path) = if force_cer.is_some() && existing_report_is_measured(&root) {
        emit(&format!(
            "  已实测的 {REPORT_JSON} 保持不动，自检结果写到 {SELFCHECK_JSON}"
        ));
        (root.join(SELFCHECK_JSON), root.join(SELFCHECK_MD))
    } else {
        (root.join(REPORT_JSON), root.join(REPORT_MD))
    };
    if let Some(parent) = json_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut json = serde_json::to_string_pretty(&report).context("序列化报告失败")?;
    json.push('\n');
    std::fs::write(&json_path, json)
        .with_context(|| format!("写出 {} 失败", json_path.display()))?;
    std::fs::write(&md_path, render_markdown(&report))
        .with_context(|| format!("写出 {} 失败", md_path.display()))?;

    emit(&format!(
        "裁决 scoring_mode = {}（总 CER {}，阈值 {:.0}%），已写出 {} 与 {}",
        report.scoring_mode,
        report.overall.as_ref().map_or_else(
            || "NOT MEASURED".to_owned(),
            |a| format!("{:.2}%", a.cer * 100.0)
        ),
        CER_THRESHOLD * 100.0,
        json_path.display(),
        md_path.display()
    ));
    Ok(())
}

/// 只按现有 JSON 重渲染 Markdown，不重跑任何推理。
///
/// 存在的理由与 `corpus-measure --render-only` 完全一致：实测一轮约一小时，而调整
/// 人读报告的措辞是高频操作。没有这条通路，改一句话就得重跑一小时，或者手改
/// Markdown——后者会让报告与 JSON 悄悄分叉，而下游 todo 读的是 JSON。
fn render_only_from_json(root: &Path) -> Result<()> {
    let json_path = root.join(REPORT_JSON);
    let text = std::fs::read_to_string(&json_path)
        .with_context(|| format!("读取 {} 失败", json_path.display()))?;
    let report: Report = serde_json::from_str(&text)
        .with_context(|| format!("解析 {} 失败", json_path.display()))?;
    // 裁决只能由 CER 推出。若磁盘上的 JSON 与判据不符，那是被手改过，必须拒绝而不是照抄。
    let expected = verdict(report.overall.as_ref().map(|a| a.cer));
    if report.scoring_mode != expected {
        bail!(
            "{} 里的 scoring_mode 是 `{}`，但按它自己记录的 CER 应为 `{}` —— \
             报告被手改过，拒绝以它为准重渲染",
            json_path.display(),
            report.scoring_mode,
            expected
        );
    }
    let md_path = root.join(REPORT_MD);
    std::fs::write(&md_path, render_markdown(&report))
        .with_context(|| format!("写出 {} 失败", md_path.display()))?;
    emit(&format!(
        "已按现有 {} 重渲染 {}（scoring_mode = {}，未重跑测量）",
        REPORT_JSON, REPORT_MD, report.scoring_mode
    ));
    Ok(())
}

/// 磁盘上是否已存在一份**实测**报告。解析失败或字段缺失都当作「没有」——
/// 判断错方向的代价不对称：误判为「有」只是让自检产物换个文件名，
/// 误判为「没有」会覆盖一小时的测量结果。
fn existing_report_is_measured(root: &Path) -> bool {
    std::fs::read_to_string(root.join(REPORT_JSON))
        .ok()
        .and_then(|t| serde_json::from_str::<Report>(&t).ok())
        .is_some_and(|r| r.measured)
}

fn repo_root() -> Result<PathBuf> {
    let xtask_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    xtask_dir
        .parent()
        .map(Path::to_path_buf)
        .context("定位仓库根目录失败")
}

// ---------------------------------------------------------------------------
// 真正跑推理的部分。不开 `voice` 时如实写 NOT MEASURED，绝不编数字。
// ---------------------------------------------------------------------------

#[cfg(not(feature = "voice"))]
mod measure {
    use super::{Poem, Report};
    use anyhow::Result;

    pub const CONDITIONS: &[&str] = &[
        "clean",
        "narrowband-8k",
        "pink-20db",
        "pink-10db",
        "slow-110",
        "fast-90",
    ];
    pub const TTS_MODELS: &[(&str, i32)] =
        &[("vits-melo-tts-zh_en", 0), ("kokoro-multi-lang-v1_0", 46)];
    pub const ASR_MODELS: &[(&str, &str)] = &[];

    pub fn fill(
        _poems: &[Poem],
        report: &mut Report,
        _dump: Option<&std::path::Path>,
    ) -> Result<()> {
        report.not_measured_reason = Some(
            "本次运行的 `xtask` **未开启 `voice` 特性**，因此没有加载任何 ASR/TTS 原生库，\
             一句话也没有真的合成或识别。\n\n\
             未自动开启的原因：`voice` 会拉入预编译的 sherpa-onnx 原生产物，而该产物\
             **静态包含 GPL-3.0 的 espeak-ng**（实测 50 个 `espeak_*` 导出符号，见 \
             `docs/VOICE-BUILD.zh.md`），使用者尚未裁决是否接受由此产生的分发条款。\
             默认构建保持纯 MIT 且不链接 onnxruntime，这条约束优先于「把数字填上」。\n\n\
             要真实测量，需满足：libclang（Debian 装 `libclang-dev`，必要时设 `LIBCLANG_PATH`）、\
             能访问 `github.com/k2-fsa/sherpa-onnx/releases` 下载原生产物、\
             按 `models.toml` 下载并解包权重到 `models/cache/`，然后运行\
             `cargo run -p xtask --features voice -- cer-spike`。"
                .to_owned(),
        );
        Ok(())
    }
}

#[cfg(feature = "voice")]
mod measure {
    use std::collections::BTreeMap;
    use std::path::Path;

    use anyhow::{Context, Result};
    use yunjian_voice::asr::{Precision, Recognizer, RecognizerOptions};
    use yunjian_voice::{augment, tts};

    use super::{Aggregate, ConditionRow, GenreRow, ModelRow, Poem, Report, cer};
    use crate::verify_sources::emit;

    pub const CONDITIONS: &[&str] = &[
        "clean",
        "narrowband-8k",
        "pink-20db",
        "pink-10db",
        "slow-110",
        "fast-90",
    ];

    /// 两把音色，分属不同引擎族（VITS / Kokoro），正是方案要求的「两种不同授权 TTS 音色」。
    /// 第二项是说话人 id：melo 中文只有一个女声（0），kokoro 的 46 是 zf_xiaoni。
    pub const TTS_MODELS: &[(&str, i32)] =
        &[("vits-melo-tts-zh_en", 0), ("kokoro-multi-lang-v1_0", 46)];

    /// 识别侧。三个尺寸都测：尺寸对文言的影响是本次实测最强的一个自变量，
    /// 只测一个就无法回答「换个更大的模型能不能过阈值」。
    pub const ASR_MODELS: &[(&str, &str)] = &[
        ("sherpa-onnx-whisper-tiny", "int8"),
        ("sherpa-onnx-whisper-base", "int8"),
        ("sherpa-onnx-whisper-small", "int8"),
    ];

    /// ASR 的输入采样率。Whisper 的特征提取固定在 16 kHz。
    const ASR_RATE: u32 = 16_000;

    struct Utterance {
        genre: String,
        condition: &'static str,
        tts_model: &'static str,
        reference: String,
        samples: Vec<f32>,
    }

    pub fn fill(poems: &[Poem], report: &mut Report, dump: Option<&Path>) -> Result<()> {
        let mut dumped: Vec<String> = Vec::new();
        let cache = yunjian_voice::asr::model_root();
        let mut utterances: Vec<Utterance> = Vec::new();

        for (model, speaker) in TTS_MODELS {
            let dir = cache.join(model);
            let mut synth = tts::Synthesizer::new(&dir)
                .with_context(|| format!("构造合成器失败：{}", dir.display()))?;
            emit(&format!(
                "  合成 {model}（引擎 {}）…",
                synth.engine().as_str()
            ));
            for poem in poems {
                let audio = synth
                    .synthesize(&poem.body, *speaker)
                    .with_context(|| format!("合成失败：{}", poem.id))?;
                let base =
                    augment::resample_antialiased(&audio.samples, audio.sample_rate, ASR_RATE);
                for condition in CONDITIONS {
                    let samples = apply(condition, &base);
                    utterances.push(Utterance {
                        genre: poem.genre.clone(),
                        condition,
                        tts_model: model,
                        reference: poem.reference.clone(),
                        samples,
                    });
                }
            }
        }

        emit(&format!("  共 {} 条待识别音频", utterances.len()));

        let mut per_model: Vec<ModelRow> = Vec::new();
        let mut genre_acc: BTreeMap<String, (usize, f64)> = BTreeMap::new();
        let mut cond_acc: BTreeMap<&str, (usize, f64)> = BTreeMap::new();
        let mut total = (0usize, 0.0f64);

        for (asr_model, precision) in ASR_MODELS {
            let dir = cache.join(asr_model);
            let opts = RecognizerOptions {
                language: "zh".to_owned(),
                precision: if *precision == "int8" {
                    Precision::Int8
                } else {
                    Precision::Float32
                },
                num_threads: 8,
            };
            let mut rec = Recognizer::open(&dir, &opts)
                .with_context(|| format!("构造识别器失败：{}", dir.display()))?;

            let mut by_tts: BTreeMap<&str, (usize, f64)> = BTreeMap::new();
            for (i, u) in utterances.iter().enumerate() {
                let text = rec.transcribe(&u.samples, ASR_RATE);
                let score = cer(&u.reference, &text);
                if dump.is_some() {
                    dumped.push(
                        serde_json::json!({
                            "asr_model": asr_model,
                            "tts_model": u.tts_model,
                            "condition": u.condition,
                            "genre": u.genre,
                            "seconds": u.samples.len() as f64 / f64::from(ASR_RATE),
                            "rms": yunjian_voice::rms(&u.samples),
                            "reference": u.reference,
                            "hypothesis": text,
                            "cer": score,
                        })
                        .to_string(),
                    );
                }

                let e = by_tts.entry(u.tts_model).or_insert((0, 0.0));
                e.0 += 1;
                e.1 += score;
                let g = genre_acc.entry(u.genre.clone()).or_insert((0, 0.0));
                g.0 += 1;
                g.1 += score;
                let c = cond_acc.entry(u.condition).or_insert((0, 0.0));
                c.0 += 1;
                c.1 += score;
                total.0 += 1;
                total.1 += score;

                if (i + 1) % 100 == 0 {
                    emit(&format!(
                        "    {asr_model}: {} / {}",
                        i + 1,
                        utterances.len()
                    ));
                }
            }
            for (tts_model, (n, sum)) in by_tts {
                #[expect(clippy::cast_precision_loss, reason = "句数在万以内")]
                let denom = n as f64;
                per_model.push(ModelRow {
                    asr_model: (*asr_model).to_owned(),
                    tts_model: tts_model.to_owned(),
                    utterances: n,
                    cer: sum / denom,
                });
            }
        }

        #[expect(clippy::cast_precision_loss, reason = "句数在万以内")]
        let mean = |(n, sum): (usize, f64)| if n == 0 { 0.0 } else { sum / n as f64 };

        let poems_per_genre: BTreeMap<&str, usize> =
            poems.iter().fold(BTreeMap::new(), |mut m, p| {
                *m.entry(p.genre.as_str()).or_default() += 1;
                m
            });

        report.per_genre = genre_acc
            .iter()
            .map(|(genre, acc)| GenreRow {
                genre: genre.clone(),
                poems: poems_per_genre
                    .get(genre.as_str())
                    .copied()
                    .unwrap_or_default(),
                utterances: acc.0,
                cer: mean(*acc),
            })
            .collect();
        report.per_condition = cond_acc
            .iter()
            .map(|(condition, acc)| ConditionRow {
                condition: (*condition).to_owned(),
                utterances: acc.0,
                cer: mean(*acc),
            })
            .collect();
        report.per_model = per_model;
        report.overall = Some(Aggregate {
            cer: mean(total),
            utterances: total.0,
        });
        report.method.utterances_measured = total.0;
        report.measured = true;
        report.measured_at = Some(today());
        if let Some(path) = dump {
            std::fs::write(path, dumped.join("\n") + "\n")
                .with_context(|| format!("写出逐句转写 {} 失败", path.display()))?;
            emit(&format!("  已写出逐句转写 {}", path.display()));
        }
        Ok(())
    }

    fn apply(condition: &str, base: &[f32]) -> Vec<f32> {
        match condition {
            "narrowband-8k" => augment::narrowband_roundtrip(base, ASR_RATE),
            "pink-20db" => augment::mix_pink_noise(base, 20.0, 0x5EED_0001),
            "pink-10db" => augment::mix_pink_noise(base, 10.0, 0x5EED_0002),
            "slow-110" => augment::time_stretch(base, 1.1),
            "fast-90" => augment::time_stretch(base, 0.9),
            _ => base.to_vec(),
        }
    }

    /// 只需要 YYYY-MM-DD。为此引入 `chrono` 不值得，`SystemTime` 加一段民用历换算即可。
    fn today() -> String {
        let secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or_default();
        let days = i64::try_from(secs / 86_400).unwrap_or_default();
        let (y, m, d) = civil_from_days(days);
        format!("{y:04}-{m:02}-{d:02}")
    }

    /// Howard Hinnant 的 `civil_from_days`：以 1970-01-01 为原点的天数转公历。
    fn civil_from_days(z: i64) -> (i64, u32, u32) {
        let z = z + 719_468;
        let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
        let doe = z - era * 146_097;
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
        let y = yoe + era * 400;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let d = u32::try_from(doy - (153 * mp + 2) / 5 + 1).unwrap_or(1);
        let m = u32::try_from(if mp < 10 { mp + 3 } else { mp - 9 }).unwrap_or(1);
        (if m <= 2 { y + 1 } else { y }, m, d)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cer_of_identical_text_is_zero() {
        assert!(cer("床前明月光", "床前明月光").abs() < f64::EPSILON);
    }

    #[test]
    fn cer_counts_substitutions_insertions_and_deletions() {
        assert!((cer("床前明月光", "床前明月霜") - 0.2).abs() < 1e-9);
        assert!((cer("床前明月光", "床前明月") - 0.2).abs() < 1e-9);
        assert!((cer("床前明月光", "床前明月光光") - 0.2).abs() < 1e-9);
    }

    /// 标点与空白不该计进 CER：模型不输出标点是既定行为，不是识别错误。
    #[test]
    fn punctuation_and_spacing_do_not_count() {
        assert!(cer("床前明月光，疑是地上霜。", "床前明月光 疑是地上霜").abs() < f64::EPSILON);
    }

    /// 繁简差异是字形差异，不是听辨错误。Whisper 中文常输出繁体，
    /// 不做归一会得到一个与真实识别能力无关的数字。
    #[test]
    fn traditional_output_is_normalized_before_scoring() {
        assert!(cer("国破山河在", "國破山河在").abs() < f64::EPSILON);
    }

    #[test]
    fn empty_hypothesis_is_total_loss() {
        assert!((cer("床前明月光", "") - 1.0).abs() < f64::EPSILON);
        assert!(cer("", "").abs() < f64::EPSILON);
    }

    /// 事先声明的分叉：阈值两侧各测一次，且**永远不出现 `full` 或 `completeness_only`**。
    #[test]
    fn verdict_has_exactly_two_possible_values() {
        assert_eq!(verdict(Some(0.0)), "coverage_advisory");
        assert_eq!(verdict(Some(CER_THRESHOLD)), "coverage_advisory");
        assert_eq!(verdict(Some(CER_THRESHOLD + 1e-9)), "guided_practice");
        assert_eq!(verdict(Some(0.5)), "guided_practice");
        // 未实测时落到最保守的一侧。
        assert_eq!(verdict(None), "guided_practice");

        for c in [None, Some(0.0), Some(0.09), Some(0.11), Some(1.0)] {
            let v = verdict(c);
            assert!(
                v == "coverage_advisory" || v == "guided_practice",
                "出现了第三个取值 `{v}`"
            );
            assert_ne!(v, "full", "字准永远不得升为正式评分");
        }
    }

    /// 落盘的那份报告本身必须合法：todo 48、51、56、57 读的是文件，不是这里的函数。
    /// 出现第三个取值（尤其 `full`）会让下游按不存在的模式分支。
    #[test]
    fn the_shipped_report_declares_a_legal_scoring_mode() {
        let root = repo_root().expect("repo root");
        let text = std::fs::read_to_string(root.join(REPORT_JSON)).expect("报告应存在");
        let report: Report = serde_json::from_str(&text).expect("报告可解析");
        assert!(
            report.scoring_mode == "coverage_advisory" || report.scoring_mode == "guided_practice",
            "落盘报告里出现了非法的 scoring_mode `{}`",
            report.scoring_mode
        );
        assert_ne!(report.scoring_mode, "full", "字准永远不得升为正式评分");
        assert!(
            (report.cer_threshold - CER_THRESHOLD).abs() < f64::EPSILON,
            "报告记录的阈值与代码不一致"
        );
        assert!(
            !report.fixture.human_recordings_used,
            "本 todo 刻意不以真人录音为门禁"
        );
        assert!(
            report.optimistic_bound_statement.contains("乐观上界"),
            "报告必须写明这是乐观上界"
        );
        // 裁决必须与量出来的数一致，不能是手改进去的。
        assert_eq!(
            report.scoring_mode,
            verdict(report.overall.as_ref().map(|a| a.cer)),
            "报告里的裁决与它自己记录的 CER 不符"
        );
    }

    /// 逐体裁表必须真的分体裁：只有一个桶等于没有分组，而方案要求的是逐体裁 CER。
    #[test]
    fn the_shipped_report_breaks_cer_down_by_genre_when_measured() {
        let root = repo_root().expect("repo root");
        let text = std::fs::read_to_string(root.join(REPORT_JSON)).expect("报告应存在");
        let report: Report = serde_json::from_str(&text).expect("报告可解析");
        if !report.measured {
            assert!(
                report.not_measured_reason.is_some(),
                "未实测就必须写清阻塞原因"
            );
            return;
        }
        assert!(
            report.per_genre.len() >= 5,
            "实测报告只有 {} 个体裁分组",
            report.per_genre.len()
        );
        assert!(report.overall.is_some(), "实测却没有总 CER");
        assert!(
            report.method.utterances_measured > 0,
            "实测句数为 0，那不叫实测"
        );
    }

    /// 自检通道绝不能覆盖实测报告：一秒的操作不该毁掉一小时的测量。
    /// 这条用例守的是判据本身，不依赖磁盘上当前是哪一种报告。
    #[test]
    fn the_selfcheck_channel_is_distinguishable_from_the_real_report() {
        assert_ne!(REPORT_JSON, SELFCHECK_JSON);
        assert_ne!(REPORT_MD, SELFCHECK_MD);
        let dir = std::env::temp_dir().join("yunjian-cer-selfcheck-guard");
        let reports = dir.join("docs").join("reports");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&reports).expect("临时目录可创建");

        assert!(
            !existing_report_is_measured(&dir),
            "没有报告时不该判定为已实测"
        );
        std::fs::write(reports.join("asr-cer.json"), b"{ not json").expect("可写入");
        assert!(
            !existing_report_is_measured(&dir),
            "报告损坏时不该判定为已实测"
        );
        std::fs::write(
            reports.join("asr-cer.json"),
            serde_json::json!({ "measured": false }).to_string(),
        )
        .expect("可写入");
        assert!(
            !existing_report_is_measured(&dir),
            "measured=false 不该判定为已实测"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn editorial_glosses_are_stripped_from_reference_text() {
        assert_eq!(
            clean_line("春草明年綠，王孫歸不歸？(明年 一作：年年)"),
            "春草明年綠，王孫歸不歸？"
        );
        assert_eq!(
            clean_line("美人卷珠簾，深坐顰蛾眉。（顰 一作：蹙）"),
            "美人卷珠簾，深坐顰蛾眉。"
        );
        assert_eq!(
            clean_line("床前明月光， 疑是地上霜。"),
            "床前明月光，疑是地上霜。"
        );
    }

    #[test]
    fn quotas_sum_to_fifty_poems() {
        let total: usize = QUOTAS.iter().map(|(_, n)| n).sum();
        assert_eq!(total, 50, "方案要求 50 首");
    }

    /// URL 里的中文路径必须百分号编码，否则 raw.githubusercontent.com 返回 400。
    #[test]
    fn raw_url_percent_encodes_the_chinese_path() {
        let url = raw_url("蒙学/tangshisanbaishou.json");
        assert!(url.contains(CORPUS_REV), "{url}");
        assert!(
            url.ends_with("/%E8%92%99%E5%AD%A6/tangshisanbaishou.json"),
            "{url}"
        );
    }

    /// 报告渲染必须始终写出裁决与「乐观上界」声明——即便未实测。
    /// 少了任何一项，下游 todo 就会读到一份看不出可信度的报告。
    #[test]
    fn markdown_always_states_the_verdict_and_the_optimistic_bound() {
        let report = Report {
            schema_version: 1,
            measured: false,
            measured_at: None,
            scoring_mode: verdict(None).to_owned(),
            cer_threshold: CER_THRESHOLD,
            fixture: FixtureSummary {
                poems: 50,
                genres: 8,
                source_repo: "chinese-poetry/chinese-poetry".to_owned(),
                source_rev: CORPUS_REV.to_owned(),
                excerpted: 3,
                human_recordings_used: false,
            },
            method: Method {
                tts_models: vec!["vits-melo-tts-zh_en".to_owned()],
                asr_models: Vec::new(),
                conditions: vec!["clean".to_owned()],
                utterances_planned: 1800,
                utterances_measured: 0,
            },
            not_measured_reason: Some("测试".to_owned()),
            optimistic_bound_statement: OPTIMISTIC_STATEMENT.to_owned(),
            overall: None,
            per_genre: Vec::new(),
            per_condition: Vec::new(),
            per_model: Vec::new(),
        };
        let md = render_markdown(&report);
        assert!(md.contains("guided_practice"), "{md}");
        assert!(md.contains("乐观上界"), "{md}");
        assert!(md.contains("NOT MEASURED"), "{md}");
        assert!(md.contains("永远不会是 `full`"), "{md}");
    }
}
