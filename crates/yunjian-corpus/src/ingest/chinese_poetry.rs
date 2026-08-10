//! `chinese-poetry/chinese-poetry` 入库。
//!
//! 这个仓库整体是 MIT，但**并非逐文件干净**：todo 9 的逐资产审计在其中查出十个
//! 夹带现代注释、赏析或百科式简介的文件。因此本模块的形状由两条规则决定：
//!
//! 1. **按字段白名单读取，而非读完再删。** 现代字段（`notes` / `prologue` /
//!    `abstract` / `preface` / 现代撰写的 `desc`）在反序列化结构体上**没有对应
//!    字段**，serde 直接跳过，那些字节从不进入任何 Rust 值。「我们不分发第三方
//!    赏析」因此是结构上成立的，不依赖谁记得删。
//! 2. **判据是文本是否前现代，不是字段名看起来危不危险。** 所以《幽梦影》的
//!    `comment`（清人友人评语「曹秋岳曰」）与 `全唐诗/authors.*.json` 的 `desc`
//!    （原书文言小传）照发，而 `蒙学/wenzimengqiu.json` 的 `author` 字段虽然叫
//!    author，装的却是现代传记，必须截断到姓名。

use crate::ingest::{
    Defect, DefectReason, FileTally, IngestOutcome, RecordStrains, ScriptDetector, StrainAlignment,
    StrainLine,
};
use crate::model::{
    Dynasty, Genre, LicenseClass, Provenance, ProvenanceKind, RecordInput, SourceLocator,
};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use yunjian_core::{Error, Result};

/// `sources.toml` 里的来源名，同时是 `source_locator` 的前缀。
pub const SOURCE_NAME: &str = "chinese-poetry";

/// `corpus/sources.toml` 锁定的 revision。入库只认这一个版本。
pub const SOURCE_REV: &str = "b8594f81a89752241442f2ce267d6f66f96704ee";

const LICENSE: &str = "MIT";

/// 上游平仄目录。与 `全唐诗/` 同名文件一一对应（对应关系需逐条复核，见
/// [`StrainAlignment`]）。
const STRAINS_DIR: &str = "strains/json";

fn corpus_error(message: impl Into<String>) -> Error {
    Error::Corpus(message.into())
}

/// 正文粒度。上游四种互不兼容，必须显式分派而不是靠字段探测。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BodyShape {
    /// `paragraphs: [String]`，逐行。全唐诗、宋词、元曲、五代诗词、水墨唐诗。
    ParagraphLines,
    /// `content: [String]`，逐章。诗经、楚辞。
    ChapterLines,
    /// 单个字符串。作者小传的 `desc`、《幽梦影》的 `content`。
    SingleString,
    /// `content: [{…, content | paragraphs}]`，卷—篇—行嵌套。蒙学。
    NestedVolumes,
}

/// 资产的读取方式。粒度之外还要区分标题、作者与评语的来源。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AssetKind {
    Poems,
    ChapterBook,
    AuthorBiographies,
    YouMengYing,
    Primer,
    /// 整文件皆现代编者文字，古典正文无从分离，一条不取。
    ModernOnly,
}

impl AssetKind {
    const fn body_shape(self) -> BodyShape {
        match self {
            Self::Poems => BodyShape::ParagraphLines,
            Self::ChapterBook => BodyShape::ChapterLines,
            Self::AuthorBiographies | Self::YouMengYing => BodyShape::SingleString,
            Self::Primer | Self::ModernOnly => BodyShape::NestedVolumes,
        }
    }
}

/// 文件选择规则。不引入 glob 依赖，分片家族用前缀加后缀表达。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FileSelect {
    Exact(&'static str),
    Prefixed(&'static str),
}

/// 朝代来源。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DynastySource {
    Fixed(&'static str),
    /// 元曲把朝代写成字面串 `"yuan"`，由 `Dynasty::canonicalize` 归一。
    RecordField(&'static str),
    /// 古文观止逐篇的 `author` 形如 `"先秦：左丘明 "`，朝代在冒号之前。
    LeafAuthorPrefix(&'static str),
}

/// 作者来源。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AuthorSource {
    RecordField(&'static str),
    /// 文件级 `author` 字段，截断到首个全角左括号之前。
    ///
    /// `蒙学/wenzimengqiu.json` 的 `author` 是现代传记
    /// （「王筠（1784-1854），字貫山……他的著作有《說文釋例》」），只有括号前的
    /// 姓名是可分发的；其余是现代撰写，必须在读取时就丢掉。
    FileFieldNameOnly,
    Fixed(&'static str),
}

#[derive(Debug, Clone, Copy)]
struct Asset {
    dir: &'static str,
    select: FileSelect,
    kind: AssetKind,
    genre: Genre,
    dynasty: DynastySource,
    author: AuthorSource,
    /// 本资产显式丢弃的现代字段名。仅作声明与自检用；这些名字在解析结构体上
    /// 没有对应字段，因此声明与实现是分开的两条证据。
    dropped_modern_fields: &'static [&'static str],
    /// 锁定 revision 上实测的期望条数，用于 1% 容差断言。
    expected_records: usize,
    attach_strains: bool,
}

/// 入库清单。每个条目都是一次显式决定，新增上游文件默认不入库。
///
/// `shippable = false` 的十个文件在此按**字段级抽取**处理：取古典正文，现代
/// 字段根本不读。`五代诗词/nantang/{intro,authors}.json` 是例外——它们通篇是
/// 现代编者题解与传记，没有古典正文可取，故声明为 [`AssetKind::ModernOnly`]。
const ASSETS: &[Asset] = &[
    Asset {
        dir: "全唐诗",
        select: FileSelect::Prefixed("poet.tang."),
        kind: AssetKind::Poems,
        genre: Genre::Shi,
        dynasty: DynastySource::Fixed("唐"),
        author: AuthorSource::RecordField("author"),
        dropped_modern_fields: &[],
        expected_records: 57_603,
        attach_strains: true,
    },
    Asset {
        dir: "全唐诗",
        select: FileSelect::Prefixed("poet.song."),
        kind: AssetKind::Poems,
        genre: Genre::Shi,
        dynasty: DynastySource::Fixed("宋"),
        author: AuthorSource::RecordField("author"),
        dropped_modern_fields: &[],
        expected_records: 254_225,
        attach_strains: true,
    },
    Asset {
        dir: "全唐诗",
        select: FileSelect::Exact("authors.tang.json"),
        kind: AssetKind::AuthorBiographies,
        genre: Genre::Wen,
        dynasty: DynastySource::Fixed("唐"),
        author: AuthorSource::RecordField("name"),
        dropped_modern_fields: &[],
        expected_records: 2_573,
        attach_strains: false,
    },
    Asset {
        dir: "全唐诗",
        select: FileSelect::Exact("authors.song.json"),
        kind: AssetKind::AuthorBiographies,
        genre: Genre::Wen,
        dynasty: DynastySource::Fixed("宋"),
        author: AuthorSource::RecordField("name"),
        dropped_modern_fields: &[],
        expected_records: 8_928,
        attach_strains: false,
    },
    Asset {
        dir: "宋词",
        select: FileSelect::Prefixed("ci.song."),
        kind: AssetKind::Poems,
        genre: Genre::Ci,
        dynasty: DynastySource::Fixed("宋"),
        author: AuthorSource::RecordField("author"),
        dropped_modern_fields: &[],
        expected_records: 21_053,
        attach_strains: false,
    },
    Asset {
        dir: "诗经",
        select: FileSelect::Exact("shijing.json"),
        kind: AssetKind::ChapterBook,
        genre: Genre::Shi,
        dynasty: DynastySource::Fixed("先秦"),
        author: AuthorSource::Fixed("佚名"),
        dropped_modern_fields: &[],
        expected_records: 305,
        attach_strains: false,
    },
    Asset {
        dir: "楚辞",
        select: FileSelect::Exact("chuci.json"),
        kind: AssetKind::ChapterBook,
        genre: Genre::Fu,
        dynasty: DynastySource::Fixed("先秦"),
        author: AuthorSource::RecordField("author"),
        dropped_modern_fields: &[],
        expected_records: 65,
        attach_strains: false,
    },
    Asset {
        dir: "元曲",
        select: FileSelect::Exact("yuanqu.json"),
        kind: AssetKind::Poems,
        genre: Genre::Qu,
        dynasty: DynastySource::RecordField("dynasty"),
        author: AuthorSource::RecordField("author"),
        dropped_modern_fields: &[],
        expected_records: 10_914,
        attach_strains: false,
    },
    Asset {
        dir: "幽梦影",
        select: FileSelect::Exact("youmengying.json"),
        kind: AssetKind::YouMengYing,
        genre: Genre::Wen,
        dynasty: DynastySource::Fixed("清"),
        author: AuthorSource::Fixed("张潮"),
        dropped_modern_fields: &[],
        expected_records: 772,
        attach_strains: false,
    },
    Asset {
        dir: "五代诗词/huajianji",
        select: FileSelect::Prefixed("huajianji-"),
        kind: AssetKind::Poems,
        genre: Genre::Ci,
        dynasty: DynastySource::Fixed("五代十国"),
        author: AuthorSource::RecordField("author"),
        dropped_modern_fields: &["notes"],
        expected_records: 498,
        attach_strains: false,
    },
    Asset {
        dir: "五代诗词/nantang",
        select: FileSelect::Exact("poetrys.json"),
        kind: AssetKind::Poems,
        genre: Genre::Ci,
        dynasty: DynastySource::Fixed("五代十国"),
        author: AuthorSource::RecordField("author"),
        dropped_modern_fields: &["notes"],
        expected_records: 45,
        attach_strains: false,
    },
    Asset {
        dir: "五代诗词/nantang",
        select: FileSelect::Exact("intro.json"),
        kind: AssetKind::ModernOnly,
        genre: Genre::Wen,
        dynasty: DynastySource::Fixed("五代十国"),
        author: AuthorSource::Fixed("佚名"),
        dropped_modern_fields: &["desc"],
        expected_records: 0,
        attach_strains: false,
    },
    Asset {
        dir: "五代诗词/nantang",
        select: FileSelect::Exact("authors.json"),
        kind: AssetKind::ModernOnly,
        genre: Genre::Wen,
        dynasty: DynastySource::Fixed("五代十国"),
        author: AuthorSource::Fixed("佚名"),
        dropped_modern_fields: &["desc"],
        expected_records: 0,
        attach_strains: false,
    },
    Asset {
        dir: "水墨唐诗",
        select: FileSelect::Exact("shuimotangshi.json"),
        kind: AssetKind::Poems,
        genre: Genre::Shi,
        dynasty: DynastySource::Fixed("唐"),
        author: AuthorSource::RecordField("author"),
        dropped_modern_fields: &["prologue"],
        expected_records: 176,
        attach_strains: false,
    },
    Asset {
        dir: "蒙学",
        select: FileSelect::Exact("guwenguanzhi.json"),
        kind: AssetKind::Primer,
        genre: Genre::Wen,
        dynasty: DynastySource::LeafAuthorPrefix("清"),
        author: AuthorSource::RecordField("author"),
        dropped_modern_fields: &["abstract"],
        expected_records: 222,
        attach_strains: false,
    },
    Asset {
        dir: "蒙学",
        select: FileSelect::Exact("shenglvqimeng.json"),
        kind: AssetKind::Primer,
        genre: Genre::Wen,
        dynasty: DynastySource::Fixed("清"),
        author: AuthorSource::RecordField("author"),
        dropped_modern_fields: &["abstract"],
        expected_records: 30,
        attach_strains: false,
    },
    Asset {
        dir: "蒙学",
        select: FileSelect::Exact("wenzimengqiu.json"),
        kind: AssetKind::Primer,
        genre: Genre::Wen,
        dynasty: DynastySource::Fixed("清"),
        author: AuthorSource::FileFieldNameOnly,
        dropped_modern_fields: &["abstract", "preface"],
        expected_records: 4,
        attach_strains: false,
    },
    Asset {
        dir: "蒙学",
        select: FileSelect::Exact("youxueqionglin.json"),
        kind: AssetKind::Primer,
        genre: Genre::Wen,
        dynasty: DynastySource::Fixed("明"),
        author: AuthorSource::RecordField("author"),
        dropped_modern_fields: &["abstract"],
        expected_records: 33,
        attach_strains: false,
    },
    Asset {
        dir: "蒙学",
        select: FileSelect::Exact("zengguangxianwen.json"),
        kind: AssetKind::Primer,
        genre: Genre::Wen,
        dynasty: DynastySource::Fixed("明"),
        author: AuthorSource::RecordField("author"),
        dropped_modern_fields: &["abstract"],
        expected_records: 2,
        attach_strains: false,
    },
];

/// 全部资产在锁定 revision 上的期望**入库**总数。
///
/// 与上游读入条数不同：`expected_records` 是实测入库数，正文为空的条目已按
/// [`DefectReason::MissingBody`] 排除（元曲 11057 读入 10914 入库、全唐诗
/// 57607 读入 57603 入库、全宋诗 254248 读入 254225 入库、作者小传只有带
/// `desc` 的才有正文）。声明入库数而非读入数，是为了让 1% 容差断言在上游
/// 补上缺失正文时**发现变化**，而不是永远为真。
pub fn expected_total_records() -> usize {
    ASSETS.iter().map(|asset| asset.expected_records).sum()
}

/// 现代评注字段名总表。任何一个都不得出现在解析结构体上。
pub fn dropped_modern_fields() -> Vec<&'static str> {
    let mut fields = ASSETS
        .iter()
        .flat_map(|asset| asset.dropped_modern_fields.iter().copied())
        .collect::<Vec<_>>();
    fields.sort_unstable();
    fields.dedup();
    fields
}

/// 全唐诗与宋词等诗词条目。
///
/// 现代字段刻意缺席：`notes`（花间集、南唐二主词）与 `prologue`（水墨唐诗）在此
/// 没有对应字段，serde 直接跳过，其内容从不进入进程内存中的任何记录。
#[derive(Debug, Deserialize)]
struct PoemRecord {
    #[serde(default)]
    author: Option<String>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    rhythmic: Option<String>,
    #[serde(default)]
    dynasty: Option<String>,
    #[serde(default)]
    paragraphs: Vec<String>,
    #[serde(default)]
    id: Option<String>,
}

/// 诗经、楚辞：`content[]` 逐章。
#[derive(Debug, Deserialize)]
struct ChapterRecord {
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    chapter: Option<String>,
    #[serde(default)]
    section: Option<String>,
    #[serde(default)]
    author: Option<String>,
    #[serde(default)]
    content: Vec<String>,
}

/// 作者小传。`desc` 是《全唐詩》原书文言小传，公有领域，照发。
#[derive(Debug, Deserialize)]
struct AuthorRecord {
    name: String,
    #[serde(default)]
    desc: String,
    #[serde(default)]
    id: Option<String>,
}

/// 幽梦影。`comment` 是清人友人评语（「曹秋岳曰」），前现代评点，公有领域。
#[derive(Debug, Deserialize)]
struct YouMengYingRecord {
    content: String,
    /// 上游在 219 条里有 10 条把无评语写成空串 `""` 而不是空数组，见
    /// [`StringOrLines`]：类型不一致是上游事实，必须容纳而不是让整文件解析失败。
    #[serde(default)]
    comment: StringOrLines,
}

/// 容纳上游同一字段既可能是字符串又可能是字符串数组的形状。
#[derive(Debug, Default, Deserialize)]
#[serde(untagged)]
enum StringOrLines {
    One(String),
    Many(Vec<String>),
    #[default]
    Absent,
}

impl StringOrLines {
    fn lines(&self) -> Vec<&str> {
        match self {
            Self::One(value) if !value.trim().is_empty() => vec![value.as_str()],
            Self::One(_) | Self::Absent => Vec::new(),
            Self::Many(values) => values.iter().map(String::as_str).collect(),
        }
    }
}

/// 蒙学读物。`abstract` 与 `preface` 是现代撰写，此处没有对应字段。
#[derive(Debug, Deserialize)]
struct PrimerFile {
    title: String,
    #[serde(default)]
    author: Option<String>,
    #[serde(default)]
    content: Vec<PrimerNode>,
}

/// 蒙学的卷—篇—行嵌套节点。带 `paragraphs` 者为叶。
#[derive(Debug, Deserialize)]
struct PrimerNode {
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    chapter: Option<String>,
    #[serde(default)]
    author: Option<String>,
    #[serde(default)]
    paragraphs: Vec<String>,
    #[serde(default)]
    content: Vec<PrimerNode>,
}

/// 上游平仄条目。
#[derive(Debug, Deserialize)]
struct StrainsRecord {
    #[serde(default)]
    strains: Vec<String>,
    #[serde(default)]
    id: Option<String>,
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T> {
    let raw = std::fs::read_to_string(path)
        .map_err(|error| corpus_error(format!("读取 {} 失败：{error}", path.display())))?;
    serde_json::from_str(&raw)
        .map_err(|error| corpus_error(format!("解析 {} 失败：{error}", path.display())))
}

fn select_files(root: &Path, asset: &Asset) -> Result<Vec<PathBuf>> {
    let dir = root.join(asset.dir);
    match asset.select {
        FileSelect::Exact(name) => {
            let path = dir.join(name);
            if !path.is_file() {
                return Err(corpus_error(format!("缺少上游文件：{}", path.display())));
            }
            Ok(vec![path])
        }
        FileSelect::Prefixed(prefix) => {
            let mut files = std::fs::read_dir(&dir)
                .map_err(|error| corpus_error(format!("读取目录 {} 失败：{error}", dir.display())))?
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(|error| corpus_error(format!("枚举 {} 失败：{error}", dir.display())))?
                .into_iter()
                .map(|entry| entry.path())
                .filter(|path| {
                    path.is_file()
                        && path
                            .file_name()
                            .and_then(|name| name.to_str())
                            .is_some_and(|name| name.starts_with(prefix) && name.ends_with(".json"))
                })
                .collect::<Vec<_>>();
            files.sort();
            if files.is_empty() {
                return Err(corpus_error(format!(
                    "资产 {}/{prefix}*.json 在 {} 下没有匹配任何文件",
                    asset.dir,
                    dir.display()
                )));
            }
            Ok(files)
        }
    }
}

fn relative_path(asset: &Asset, path: &Path) -> String {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    format!("{}/{name}", asset.dir)
}

fn provenance(kind: ProvenanceKind) -> Provenance {
    Provenance {
        source_name: SOURCE_NAME.to_owned(),
        source_rev: SOURCE_REV.to_owned(),
        license: LICENSE.to_owned(),
        license_class: LicenseClass::PublicDomain,
        kind,
    }
}

/// 取括号前的姓名。见 [`AuthorSource::FileFieldNameOnly`]。
fn name_before_paren(raw: &str) -> String {
    raw.split(['（', '('])
        .next()
        .unwrap_or(raw)
        .trim()
        .to_owned()
}

/// 从「曹秋岳曰：可想见其南面百城时。」中拆出评者与评语。
fn split_commentator(raw: &str) -> (String, String) {
    match raw.split_once('曰') {
        Some((speaker, rest)) if !speaker.trim().is_empty() && speaker.chars().count() <= 8 => (
            speaker.trim().to_owned(),
            rest.trim_start_matches(['：', ':']).trim().to_owned(),
        ),
        _ => ("佚名".to_owned(), raw.trim().to_owned()),
    }
}

struct FileContext<'a> {
    asset: &'a Asset,
    relative_path: String,
    detector: &'a ScriptDetector,
}

impl FileContext<'_> {
    fn locator(&self, native_id: Option<&str>, ordinal: usize) -> SourceLocator {
        match native_id {
            Some(id) if !id.is_empty() => SourceLocator::native(SOURCE_NAME, id),
            _ => SourceLocator::positional(SOURCE_NAME, &self.relative_path, ordinal),
        }
    }

    fn defect(&self, ordinal: usize, reason: DefectReason, detail: impl Into<String>) -> Defect {
        Defect {
            relative_path: self.relative_path.clone(),
            ordinal,
            reason,
            detail: detail.into(),
        }
    }

    fn build(
        &self,
        locator: SourceLocator,
        title: String,
        author: String,
        dynasty_raw: &str,
        body_lines: Vec<String>,
        kind: ProvenanceKind,
    ) -> Result<RecordInput> {
        let (dynasty, dynasty_raw) = Dynasty::canonicalize(dynasty_raw)?;
        let body_original = body_lines.join("\n");
        Ok(RecordInput {
            source_locator: locator,
            genre: self.asset.genre,
            title: title.clone(),
            title_raw: title,
            author,
            dynasty,
            dynasty_raw,
            script: self.detector.detect(&body_original),
            body_lines,
            body_original,
            provenance: provenance(kind),
        })
    }
}

fn dynasty_and_author(
    context: &FileContext<'_>,
    record_author: Option<&str>,
    record_dynasty: Option<&str>,
    file_author: Option<&str>,
) -> (String, String) {
    let asset = context.asset;
    let dynasty = match asset.dynasty {
        DynastySource::Fixed(value) => value.to_owned(),
        DynastySource::RecordField(_) => record_dynasty
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("元")
            .to_owned(),
        DynastySource::LeafAuthorPrefix(fallback) => record_author
            .and_then(|value| value.split_once('：'))
            .map(|(prefix, _)| prefix.trim().to_owned())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| fallback.to_owned()),
    };
    let author = match asset.author {
        AuthorSource::Fixed(value) => value.to_owned(),
        AuthorSource::FileFieldNameOnly => file_author
            .map(name_before_paren)
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "佚名".to_owned()),
        AuthorSource::RecordField(_) => {
            let raw = record_author.unwrap_or_default().trim();
            let trimmed = match asset.dynasty {
                DynastySource::LeafAuthorPrefix(_) => {
                    raw.split_once('：').map_or(raw, |(_, name)| name).trim()
                }
                _ => raw,
            };
            if trimmed.is_empty() {
                file_author
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .unwrap_or("佚名")
                    .to_owned()
            } else {
                trimmed.to_owned()
            }
        }
    };
    (dynasty, author)
}

fn ingest_poems(
    context: &FileContext<'_>,
    path: &Path,
    outcome: &mut IngestOutcome,
) -> Result<usize> {
    let records: Vec<PoemRecord> = read_json(path)?;
    let strains = if context.asset.attach_strains {
        Some(load_strains(path)?)
    } else {
        None
    };
    let mut emitted = 0;
    for (ordinal, record) in records.iter().enumerate() {
        let title = record
            .title
            .as_deref()
            .or(record.rhythmic.as_deref())
            .unwrap_or_default()
            .trim()
            .to_owned();
        if record.paragraphs.iter().all(|line| line.trim().is_empty()) {
            outcome.defects.push(context.defect(
                ordinal,
                DefectReason::MissingBody,
                format!("《{title}》没有可取的古典正文"),
            ));
            continue;
        }
        let (dynasty_raw, author) = dynasty_and_author(
            context,
            record.author.as_deref(),
            record.dynasty.as_deref(),
            None,
        );
        let locator = context.locator(record.id.as_deref(), ordinal);
        let input = context.build(
            locator,
            title,
            author,
            &dynasty_raw,
            record.paragraphs.clone(),
            ProvenanceKind::Original,
        )?;
        if let Some(strains) = strains.as_ref() {
            attach_strains(context, strains, record, ordinal, &input, outcome);
        }
        outcome.records.push(input);
        emitted += 1;
    }
    Ok(emitted)
}

struct StrainsFile {
    by_index: Vec<StrainsRecord>,
    by_id: BTreeMap<String, usize>,
}

fn load_strains(poem_path: &Path) -> Result<StrainsFile> {
    let file_name = poem_path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| corpus_error(format!("无法取得文件名：{}", poem_path.display())))?;
    let root = poem_path
        .parent()
        .and_then(Path::parent)
        .ok_or_else(|| corpus_error(format!("无法定位检出根：{}", poem_path.display())))?;
    let path = root.join(STRAINS_DIR).join(file_name);
    let by_index: Vec<StrainsRecord> = read_json(&path)?;
    let by_id = by_index
        .iter()
        .enumerate()
        .filter_map(|(index, record)| record.id.clone().map(|id| (id, index)))
        .collect();
    Ok(StrainsFile { by_index, by_id })
}

fn attach_strains(
    context: &FileContext<'_>,
    strains: &StrainsFile,
    record: &PoemRecord,
    ordinal: usize,
    input: &RecordInput,
    outcome: &mut IngestOutcome,
) {
    let native_id = record.id.as_deref().unwrap_or_default();
    let positional = strains.by_index.get(ordinal);
    let (entry, alignment) = match positional {
        Some(entry) if entry.id.as_deref() == Some(native_id) => {
            (Some(entry), StrainAlignment::Positional)
        }
        _ => {
            let recovered = strains
                .by_id
                .get(native_id)
                .and_then(|index| strains.by_index.get(*index));
            outcome.defects.push(context.defect(
                ordinal,
                DefectReason::StrainsMisaligned,
                format!(
                    "平仄文件同下标 id 为 {}，与诗 id {native_id} 不符；{}",
                    positional
                        .and_then(|entry| entry.id.as_deref())
                        .unwrap_or("<缺失>"),
                    if recovered.is_some() {
                        "已按原生 id 在同文件内改挂"
                    } else {
                        "同文件内也找不到该 id，未挂平仄"
                    }
                ),
            ));
            (recovered, StrainAlignment::RecoveredByNativeId)
        }
    };
    let Some(entry) = entry else { return };
    if entry.strains.is_empty() {
        outcome.defects.push(context.defect(
            ordinal,
            DefectReason::StrainsUnavailable,
            format!("上游未为 id {native_id} 算出平仄"),
        ));
        return;
    }
    if entry.strains.len() != input.body_lines.len() {
        outcome.defects.push(context.defect(
            ordinal,
            DefectReason::StrainsLineMismatch,
            format!(
                "平仄 {} 行与正文 {} 行不符，不挂以免声调错配到字上",
                entry.strains.len(),
                input.body_lines.len()
            ),
        ));
        return;
    }
    outcome.strains.push(RecordStrains {
        source_locator: input.source_locator.as_str().to_owned(),
        lines: entry
            .strains
            .iter()
            .map(|raw| StrainLine::parse(raw))
            .collect(),
        alignment,
    });
}

fn ingest_chapter_book(
    context: &FileContext<'_>,
    path: &Path,
    outcome: &mut IngestOutcome,
) -> Result<usize> {
    let records: Vec<ChapterRecord> = read_json(path)?;
    let mut emitted = 0;
    for (ordinal, record) in records.iter().enumerate() {
        let mut title = record.title.clone().unwrap_or_default().trim().to_owned();
        if let Some(section) = record
            .section
            .as_deref()
            .map(str::trim)
            .filter(|section| !section.is_empty() && *section != title)
        {
            let chapter = record.chapter.as_deref().map(str::trim).unwrap_or_default();
            title = if chapter.is_empty() {
                format!("{section}/{title}")
            } else {
                format!("{chapter}/{section}/{title}")
            };
        }
        if record.content.iter().all(|line| line.trim().is_empty()) {
            outcome.defects.push(context.defect(
                ordinal,
                DefectReason::MissingBody,
                format!("《{title}》的 content 为空"),
            ));
            continue;
        }
        let (dynasty_raw, author) =
            dynasty_and_author(context, record.author.as_deref(), None, None);
        outcome.records.push(context.build(
            context.locator(None, ordinal),
            title,
            author,
            &dynasty_raw,
            record.content.clone(),
            ProvenanceKind::Original,
        )?);
        emitted += 1;
    }
    Ok(emitted)
}

fn ingest_author_biographies(
    context: &FileContext<'_>,
    path: &Path,
    outcome: &mut IngestOutcome,
) -> Result<usize> {
    let records: Vec<AuthorRecord> = read_json(path)?;
    let mut emitted = 0;
    for (ordinal, record) in records.iter().enumerate() {
        let name = record.name.trim();
        if record.desc.trim().is_empty() {
            outcome.defects.push(context.defect(
                ordinal,
                DefectReason::MissingBody,
                format!("{name} 没有小传正文"),
            ));
            continue;
        }
        let (dynasty_raw, author) = dynasty_and_author(context, Some(name), None, None);
        outcome.records.push(context.build(
            context.locator(record.id.as_deref(), ordinal),
            format!("{name} 小传"),
            author,
            &dynasty_raw,
            vec![record.desc.trim().to_owned()],
            ProvenanceKind::Original,
        )?);
        emitted += 1;
    }
    Ok(emitted)
}

fn ingest_youmengying(
    context: &FileContext<'_>,
    path: &Path,
    outcome: &mut IngestOutcome,
) -> Result<usize> {
    let records: Vec<YouMengYingRecord> = read_json(path)?;
    let mut emitted = 0;
    let mut ordinal = 0usize;
    for (index, record) in records.iter().enumerate() {
        let title = format!("幽梦影 其{}", index + 1);
        if record.content.trim().is_empty() {
            outcome.defects.push(context.defect(
                ordinal,
                DefectReason::MissingBody,
                format!("{title} 正文为空"),
            ));
        } else {
            let (dynasty_raw, author) = dynasty_and_author(context, None, None, None);
            outcome.records.push(context.build(
                context.locator(None, ordinal),
                title.clone(),
                author,
                &dynasty_raw,
                vec![record.content.trim().to_owned()],
                ProvenanceKind::Original,
            )?);
            emitted += 1;
        }
        ordinal += 1;
        for (comment_index, comment) in record.comment.lines().into_iter().enumerate() {
            let (speaker, body) = split_commentator(comment);
            if body.is_empty() {
                outcome.defects.push(context.defect(
                    ordinal,
                    DefectReason::MissingBody,
                    format!("{title} 第 {} 条评语为空", comment_index + 1),
                ));
            } else {
                outcome.records.push(context.build(
                    context.locator(None, ordinal),
                    format!("{title} 评{}", comment_index + 1),
                    speaker,
                    "清",
                    vec![body],
                    ProvenanceKind::PublicDomainCommentary,
                )?);
                emitted += 1;
            }
            ordinal += 1;
        }
    }
    Ok(emitted)
}

fn primer_leaves<'a>(
    node: &'a PrimerNode,
    ancestry: &[&'a str],
    leaves: &mut Vec<(String, &'a PrimerNode)>,
) {
    let own = node
        .title
        .as_deref()
        .or(node.chapter.as_deref())
        .map(str::trim)
        .filter(|label| !label.is_empty());
    let mut path = ancestry.to_vec();
    if let Some(label) = own {
        path.push(label);
    }
    if node.paragraphs.iter().any(|line| !line.trim().is_empty()) {
        leaves.push((path.join("/"), node));
        return;
    }
    for child in &node.content {
        primer_leaves(child, &path, leaves);
    }
}

fn ingest_primer(
    context: &FileContext<'_>,
    path: &Path,
    outcome: &mut IngestOutcome,
) -> Result<usize> {
    let file: PrimerFile = read_json(path)?;
    let book = file.title.trim();
    let mut leaves = Vec::new();
    for node in &file.content {
        primer_leaves(node, &[book], &mut leaves);
    }
    let mut emitted = 0;
    for (ordinal, (title, node)) in leaves.iter().enumerate() {
        let (dynasty_raw, author) = dynasty_and_author(
            context,
            node.author.as_deref(),
            None,
            file.author.as_deref(),
        );
        let dynasty_raw = match Dynasty::canonicalize(&dynasty_raw) {
            Ok(_) => dynasty_raw,
            Err(_) => {
                outcome.defects.push(context.defect(
                    ordinal,
                    DefectReason::UnknownDynasty,
                    format!("《{title}》的朝代串「{dynasty_raw}」无法归一"),
                ));
                continue;
            }
        };
        outcome.records.push(context.build(
            context.locator(None, ordinal),
            title.clone(),
            author,
            &dynasty_raw,
            node.paragraphs.clone(),
            ProvenanceKind::Original,
        )?);
        emitted += 1;
    }
    Ok(emitted)
}

/// 声明整文件皆现代编者文字，一条不取，并留下可查的处置记录。
fn record_modern_only(context: &FileContext<'_>, outcome: &mut IngestOutcome) {
    outcome.defects.push(context.defect(
        0,
        DefectReason::ModernCommentaryInseparable,
        format!(
            "整文件为现代编者撰写（字段 {}），无古典正文可分离，按 sources.toml 的 shippable=false 排除",
            context.asset.dropped_modern_fields.join("、")
        ),
    ));
}

fn count_input_records(kind: AssetKind, path: &Path) -> Result<usize> {
    let value: serde_json::Value = read_json(path)?;
    let count = match (kind.body_shape(), &value) {
        (BodyShape::NestedVolumes, serde_json::Value::Object(map)) => map
            .get("content")
            .and_then(serde_json::Value::as_array)
            .map_or(0, Vec::len),
        (_, serde_json::Value::Array(items)) => items.len(),
        _ => 0,
    };
    Ok(count)
}

/// 按锁定 revision 的检出目录入库全部声明资产。
///
/// 任何声明会产出记录的文件若产出零条，立刻带文件名失败：静默空吞比报错危险，
/// 上游一次分片重命名就能让整个资产悄悄消失。
pub fn ingest(root: impl AsRef<Path>) -> Result<IngestOutcome> {
    let root = root.as_ref();
    let detector = ScriptDetector::new()?;
    let mut outcome = IngestOutcome::default();
    for asset in ASSETS {
        for path in select_files(root, asset)? {
            let context = FileContext {
                asset,
                relative_path: relative_path(asset, &path),
                detector: &detector,
            };
            let input_records = count_input_records(asset.kind, &path)?;
            let emitted = match asset.kind {
                AssetKind::Poems => ingest_poems(&context, &path, &mut outcome)?,
                AssetKind::ChapterBook => ingest_chapter_book(&context, &path, &mut outcome)?,
                AssetKind::AuthorBiographies => {
                    ingest_author_biographies(&context, &path, &mut outcome)?
                }
                AssetKind::YouMengYing => ingest_youmengying(&context, &path, &mut outcome)?,
                AssetKind::Primer => ingest_primer(&context, &path, &mut outcome)?,
                AssetKind::ModernOnly => {
                    record_modern_only(&context, &mut outcome);
                    0
                }
            };
            if emitted == 0 && asset.kind != AssetKind::ModernOnly {
                return Err(corpus_error(format!(
                    "上游文件 {} 产出 0 条记录（读入 {input_records} 条）；声明为可入库资产的文件不得空吞",
                    context.relative_path
                )));
            }
            outcome.tallies.push(FileTally {
                relative_path: context.relative_path,
                input_records,
                emitted_records: emitted,
            });
        }
    }
    Ok(outcome)
}

#[cfg(test)]
mod tests;
