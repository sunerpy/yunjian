//! `Werneror/Poetry` 入库：只取古典朝代分桶，缺字记录一律隔离。
//!
//! 本模块的形状由三条无法回避的上游事实决定：
//!
//! 1. **许可只覆盖得到它覆盖得住的部分。** 上游仓库整体是 MIT，但 MIT 不能转让
//!    上传者本人不拥有的权利。`当代`（7,905,122 字节）与 `近现代`（8,392,502 字节）
//!    等六个分桶里是仍在保护期的作者，因此入库范围是**朝代分桶的显式白名单**
//!    （[`CLASSICAL_BUCKETS`]，28 个 CSV），而不是「排掉这六个」的黑名单——
//!    上游哪天新增一个分桶，白名单机制下它默认进不来，黑名单机制下它默认进来。
//! 2. **上游有一处不可逆的数据损坏。** README 自己写明生僻 utf8mb4 字符被替换成
//!    `?`。这不是可以修的编码问题，原字已经不在文件里了。含缺字的记录**完全不进
//!    主表**：韵脚推导读行末字、破读表读逐字读音，一个 `?` 就能让这两者输出错误
//!    答案而不报错。隔离而不是丢弃——它们进 [`WernerorOutcome::quarantined`]。
//! 3. **CSV 没有任何原生主键。** 因此 `source_locator` 只能是位置型
//!    `werneror:<文件名>:<文件内序号>`，其位移由 [`crate::model`] 的探测器负责。
//!
//! 与 `chinese-poetry` 的分工是**声明出来的**，不是两次导入撞出来的：
//! `chinese-poetry` 全量入库，Werneror 只补它没有的古典诗作（[`CoveredWorks`]）。

use crate::ingest::{Defect, DefectReason, FileTally, ScriptDetector};
use crate::model::{
    Dynasty, Genre, LicenseClass, Provenance, ProvenanceKind, RecordInput, SourceLocator,
    compute_work_group,
};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use yunjian_core::{Error, Result};

/// `source_locator` 前缀与 `provenance.source_name`。
///
/// 与 `sources.toml` 里的 `name = "Werneror/Poetry"` 是同一来源的两种写法：
/// 清单里写仓库全名以便核对上游，locator 里写短名以免斜杠混进定位串。
pub const SOURCE_NAME: &str = "werneror";

/// `corpus/sources.toml` 锁定的 revision。入库只认这一个版本。
pub const SOURCE_REV: &str = "4cfe49c06858e00d15f84d192fe5294295f79689";

const LICENSE: &str = "MIT";

/// 词牌白名单文件。它本身在 `sources.toml` 里是 `permissive`，可随包。
pub const CIPAI_FILE: &str = "cipai_2.txt";

/// 上游固定表头。逐字节比对，一旦上游改列就带文件名失败而不是错位解析。
pub const HEADER: [&str; 4] = ["题目", "朝代", "作者", "内容"];

/// 「词牌 题目」形态里，词牌至少要有这么多字才认。
///
/// `cipai_2.txt` 里有 113 个二字词牌（`九日`、`三台`、`八归`……），它们同时也是
/// 极常见的诗题起首词。若不设下限，`九日 登高` 这类诗会被判成词。三字及以上的
/// 词牌（`人月圆`、`长相思`、`念奴娇`）几乎不与诗题起首撞车，所以这条下限用
/// 一点召回换掉了绝大部分误判。带间隔号 `·` 的形态不受此限——那本身就是强信号。
const MIN_SPACE_SPLIT_TUNE_CHARS: usize = 3;

fn corpus_error(message: impl Into<String>) -> Error {
    Error::Corpus(message.into())
}

/// 一个获准入库的古典朝代分桶。
///
/// 三个字段都是**在锁定 revision 上实测**得来的，不是抄上游 README：
/// `expected_rows` 是 CSV 数据行数，`expected_lossy_rows` 是至少有一列命中
/// [`has_lossy_char`] 的行数。两者一起把「这次入库应该得到多少条」变成一个
/// 可被证伪的声明。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Bucket {
    /// 仓库根下的 CSV 文件名，同时是 `relative_path`。
    pub file: &'static str,
    /// 该桶 `朝代` 列的唯一字面值（实测每桶只有一种）。
    pub dynasty_label: &'static str,
    pub expected_rows: usize,
    pub expected_lossy_rows: usize,
}

/// 古典朝代分桶白名单。**这是入库范围的唯一定义。**
///
/// 顺序与 `corpus/sources.toml` 的资产顺序一致，便于逐条核对；
/// [`tests::allow_list_matches_sources_manifest`] 会强制两者相等。
pub const CLASSICAL_BUCKETS: &[Bucket] = &[
    Bucket {
        file: "先秦.csv",
        dynasty_label: "先秦",
        expected_rows: 570,
        expected_lossy_rows: 22,
    },
    Bucket {
        file: "秦.csv",
        dynasty_label: "秦",
        expected_rows: 2,
        expected_lossy_rows: 0,
    },
    Bucket {
        file: "汉.csv",
        dynasty_label: "汉",
        expected_rows: 363,
        expected_lossy_rows: 22,
    },
    Bucket {
        file: "魏晋.csv",
        dynasty_label: "魏晋",
        expected_rows: 3_020,
        expected_lossy_rows: 31,
    },
    Bucket {
        file: "魏晋末南北朝初.csv",
        dynasty_label: "魏晋末南北朝初",
        expected_rows: 1,
        expected_lossy_rows: 0,
    },
    Bucket {
        file: "南北朝.csv",
        dynasty_label: "南北朝",
        expected_rows: 4_586,
        expected_lossy_rows: 60,
    },
    Bucket {
        file: "隋.csv",
        dynasty_label: "隋",
        expected_rows: 1_170,
        expected_lossy_rows: 13,
    },
    Bucket {
        file: "隋末唐初.csv",
        dynasty_label: "隋末唐初",
        expected_rows: 472,
        expected_lossy_rows: 1,
    },
    Bucket {
        file: "唐.csv",
        dynasty_label: "唐",
        expected_rows: 49_195,
        expected_lossy_rows: 355,
    },
    Bucket {
        file: "唐末宋初.csv",
        dynasty_label: "唐末宋初",
        expected_rows: 1_118,
        expected_lossy_rows: 7,
    },
    Bucket {
        file: "宋_1.csv",
        dynasty_label: "宋",
        expected_rows: 78_557,
        expected_lossy_rows: 398,
    },
    Bucket {
        file: "宋_2.csv",
        dynasty_label: "宋",
        expected_rows: 78_557,
        expected_lossy_rows: 559,
    },
    Bucket {
        file: "宋_3.csv",
        dynasty_label: "宋",
        expected_rows: 65_000,
        expected_lossy_rows: 420,
    },
    Bucket {
        file: "宋_4.csv",
        dynasty_label: "宋",
        expected_rows: 65_000,
        expected_lossy_rows: 541,
    },
    Bucket {
        file: "宋末金初.csv",
        dynasty_label: "宋末金初",
        expected_rows: 234,
        expected_lossy_rows: 1,
    },
    Bucket {
        file: "宋末元初.csv",
        dynasty_label: "宋末元初",
        expected_rows: 12_058,
        expected_lossy_rows: 69,
    },
    Bucket {
        file: "辽.csv",
        dynasty_label: "辽",
        expected_rows: 22,
        expected_lossy_rows: 0,
    },
    Bucket {
        file: "金.csv",
        dynasty_label: "金",
        expected_rows: 2_741,
        expected_lossy_rows: 31,
    },
    Bucket {
        file: "金末元初.csv",
        dynasty_label: "金末元初",
        expected_rows: 3_019,
        expected_lossy_rows: 20,
    },
    Bucket {
        file: "元.csv",
        dynasty_label: "元",
        expected_rows: 37_375,
        expected_lossy_rows: 302,
    },
    Bucket {
        file: "元末明初.csv",
        dynasty_label: "元末明初",
        expected_rows: 15_736,
        expected_lossy_rows: 194,
    },
    Bucket {
        file: "明_1.csv",
        dynasty_label: "明",
        expected_rows: 59_478,
        expected_lossy_rows: 606,
    },
    Bucket {
        file: "明_2.csv",
        dynasty_label: "明",
        expected_rows: 59_000,
        expected_lossy_rows: 538,
    },
    Bucket {
        file: "明_3.csv",
        dynasty_label: "明",
        expected_rows: 59_479,
        expected_lossy_rows: 249,
    },
    Bucket {
        file: "明_4.csv",
        dynasty_label: "明",
        expected_rows: 59_000,
        expected_lossy_rows: 692,
    },
    Bucket {
        file: "明末清初.csv",
        dynasty_label: "明末清初",
        expected_rows: 17_700,
        expected_lossy_rows: 144,
    },
    Bucket {
        file: "清_1.csv",
        dynasty_label: "清",
        expected_rows: 45_091,
        expected_lossy_rows: 552,
    },
    Bucket {
        file: "清_2.csv",
        dynasty_label: "清",
        expected_rows: 44_998,
        expected_lossy_rows: 286,
    },
];

/// 已知的近现代/当代分桶。**仅用于把排除理由写得准确，不承担排除职责。**
///
/// 排除由 [`CLASSICAL_BUCKETS`] 的白名单完成：不在白名单里就进不来。这份名单
/// 只回答「为什么进不来」——是已知的保护期问题，还是一个我们没见过的新桶。
/// 因此往这里漏加一个文件名不会造成许可事故，只会让理由变成
/// [`ExclusionReason::NotOnClassicalAllowList`]。
pub const KNOWN_MODERN_BUCKETS: &[&str] = &[
    "清末民国初.csv",
    "清末近现代初.csv",
    "近现代.csv",
    "近现代末当代初.csv",
    "民国末当代初.csv",
    "当代.csv",
];

/// 随仓 fixture 目录里**实际提供**的古典分桶。
///
/// 它不是第二份白名单，而是「本仓能离线跑到的那一小块」：白名单 28 个分桶在锁定
/// revision 上动辄十几 MB，全部签进 fixture 会让仓库体积失控，所以 fixture 只覆盖
/// 这 7 个小桶。`当代.csv` 与 `未来.csv` 虽然也在 fixture 目录里，但刻意不列——
/// 前者是已知近现代桶，后者根本不在白名单上，两者都必须由策略排除而不是由这份
/// 名单排除。
///
/// **凡是接受「fixture 目录」作为输入的命令都必须按它裁剪分桶**，否则会去要一个
/// 从来没打算签进来的文件，然后把这件事报成「数据缺失」。
/// [`tests::fixture_bucket_list_matches_the_fixture_directory`] 扫真实目录守住它。
pub const FIXTURE_BUCKETS: &[&str] = &[
    "先秦.csv",
    "秦.csv",
    "魏晋末南北朝初.csv",
    "隋末唐初.csv",
    "唐.csv",
    "宋末金初.csv",
    "辽.csv",
];

/// 按文件名从古典白名单取出分桶，顺序与传入的名单一致。
///
/// 刻意不让调用方自己拼 [`Bucket`]：`dynasty_label` 与期望行数只能有一份声明，
/// 各处重新写一遍就等于让白名单有第二个事实来源，两份漂移后调用方仍然通过而
/// 产品行为已经变了。
pub fn buckets_by_file(files: &[&str]) -> Result<Vec<Bucket>> {
    files
        .iter()
        .map(|file| {
            CLASSICAL_BUCKETS
                .iter()
                .find(|bucket| bucket.file == *file)
                .copied()
                .ok_or_else(|| corpus_error(format!("古典白名单里没有分桶 {file}")))
        })
        .collect()
}

/// 全部白名单分桶在锁定 revision 上的实测数据行数之和。
pub fn expected_total_rows() -> usize {
    CLASSICAL_BUCKETS
        .iter()
        .map(|bucket| bucket.expected_rows)
        .sum()
}

/// 全部白名单分桶在锁定 revision 上的实测缺字行数之和。
pub fn expected_total_lossy_rows() -> usize {
    CLASSICAL_BUCKETS
        .iter()
        .map(|bucket| bucket.expected_lossy_rows)
        .sum()
}

/// 一个分桶未被入库的原因。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExclusionReason {
    /// 已知的近现代/当代分桶：作者极可能仍在保护期，上游 MIT 覆盖不到。
    ModernAuthorsLikelyInCopyright,
    /// 不在古典白名单上（含上游新增的分桶）。默认排除。
    NotOnClassicalAllowList,
}

impl ExclusionReason {
    const fn as_str(self) -> &'static str {
        match self {
            Self::ModernAuthorsLikelyInCopyright => "已知近现代/当代分桶，保护期未过",
            Self::NotOnClassicalAllowList => "不在古典朝代白名单上",
        }
    }
}

/// 一个被排除的分桶及其行数。行数照数不照收：声明一个数字，才能发现上游变化。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BucketExclusion {
    pub file: String,
    pub reason: ExclusionReason,
    /// 该文件的数据行数。解析失败时为 0，原因写进 `detail`。
    pub rows: usize,
    pub detail: String,
}

/// 缺字命中的列。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LossyField {
    Title,
    Author,
    Body,
}

impl LossyField {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Title => "题目",
            Self::Author => "作者",
            Self::Body => "内容",
        }
    }
}

/// 一条被缺字隔离的记录。它**永不进入主表**，但完整留档以便复核与上游报错。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuarantinedRecord {
    pub source_locator: String,
    pub relative_path: String,
    pub ordinal: usize,
    pub title_raw: String,
    pub author: String,
    pub dynasty_raw: String,
    /// 恒为 `true`：这个结构只在缺字命中时构造，字段存在是为了让下游 schema
    /// 与缺陷报告里的标记同名可查。
    pub has_lossy_char: bool,
    pub lossy_fields: Vec<LossyField>,
    pub body_original: String,
}

/// 一条因 `chinese-poetry` 已收录而不重复入库的记录。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DuplicateRecord {
    pub source_locator: String,
    pub relative_path: String,
    pub ordinal: usize,
    pub title_raw: String,
    /// 判重键：简体化后的 `work_group`。
    pub work_group: String,
}

/// 一次 Werneror 入库的全部产出。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WernerorOutcome {
    /// 可进主表的记录。不含任何缺字记录，也不含 `chinese-poetry` 已有的作品。
    pub records: Vec<RecordInput>,
    pub defects: Vec<Defect>,
    pub tallies: Vec<FileTally>,
    pub quarantined: Vec<QuarantinedRecord>,
    pub exclusions: Vec<BucketExclusion>,
    pub duplicates: Vec<DuplicateRecord>,
}

impl WernerorOutcome {
    pub fn emitted(&self) -> usize {
        self.records.len()
    }

    pub fn tally(&self, relative_path: &str) -> Option<&FileTally> {
        self.tallies
            .iter()
            .find(|tally| tally.relative_path == relative_path)
    }

    pub fn exclusion(&self, file: &str) -> Option<&BucketExclusion> {
        self.exclusions
            .iter()
            .find(|exclusion| exclusion.file == file)
    }

    /// 被策略排除的分桶合计行数。
    pub fn excluded_rows(&self) -> usize {
        self.exclusions.iter().map(|exclusion| exclusion.rows).sum()
    }

    /// 某文件下被隔离的记录数。
    pub fn quarantined_in(&self, relative_path: &str) -> usize {
        self.quarantined
            .iter()
            .filter(|record| record.relative_path == relative_path)
            .count()
    }

    /// 某文件下被判重掉的记录数。
    pub fn duplicates_in(&self, relative_path: &str) -> usize {
        self.duplicates
            .iter()
            .filter(|record| record.relative_path == relative_path)
            .count()
    }
}

/// `chinese-poetry` 已覆盖的作品集合，按判重键索引。
///
/// 这是「逐来源取舍」的落地形态：`chinese-poetry` 全量入库在前，Werneror 拿着
/// 它的作品键集合进来，只补集合外的诗。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CoveredWorks {
    keys: BTreeSet<String>,
}

impl CoveredWorks {
    /// 没有任何前置来源时使用（例如单测只关心 Werneror 自身的行为）。
    pub fn empty() -> Self {
        Self::default()
    }

    /// 由 `chinese-poetry` 的入库产物构建。
    pub fn from_records(detector: &ScriptDetector, records: &[RecordInput]) -> Self {
        Self {
            keys: records
                .iter()
                .map(|record| dedup_key(detector, &record.body_lines.join("\n")))
                .collect(),
        }
    }

    pub fn contains(&self, key: &str) -> bool {
        self.keys.contains(key)
    }

    pub fn len(&self) -> usize {
        self.keys.len()
    }

    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }
}

/// 跨来源判重键：**先简体化再取 `work_group`**。
///
/// 不能直接用原字形的 `work_group`。`chinese-poetry/全唐诗` 是繁体、Werneror 是
/// 简体，同一首诗两边字形不同，按原字形判重一条都判不出来。繁简归一本身属于
/// 构建期的另一道工序，这里只借它算键，**不改写任何 `body`**：
/// [`RecordInput::body_original`] 与 [`RecordInput::body_lines`] 始终是上游字形。
fn dedup_key(detector: &ScriptDetector, body: &str) -> String {
    compute_work_group(&detector.simplify(body))
}

/// 判断一个字符是否处于 CJK 上下文（汉字、扩展区、兼容区、全角标点）。
fn is_cjk_context(character: char) -> bool {
    matches!(character,
        '\u{3000}'..='\u{303F}'      // CJK 标点：。，、；：《》【】
        | '\u{3400}'..='\u{4DBF}'    // 扩展 A
        | '\u{4E00}'..='\u{9FFF}'    // 基本区
        | '\u{F900}'..='\u{FAFF}'    // 兼容表意
        | '\u{FF00}'..='\u{FFEF}'    // 全角形式
        | '\u{20000}'..='\u{3FFFF}'  // 扩展 B 及以后
    )
}

/// 文本里是否存在 CJK 上下文中的 ASCII `?`，即上游不可逆丢失的痕迹。
///
/// 判据是**紧邻**：一段连续 `?` 的左右任一侧是 CJK 字符或全角标点，就是缺字。
/// 之所以不简单地「含 `?` 即算」，是因为半角问号在纯 ASCII 语境里可能是正当
/// 标点；之所以不看全角 `？`，是因为古典诗文的疑问句用的正是全角 `？`，那是
/// 内容而不是损坏。整个字段全是 `?` 的极端情况也算缺字。
pub fn has_lossy_char(text: &str) -> bool {
    if !text.contains('?') {
        return false;
    }
    let characters: Vec<char> = text.chars().collect();
    let mut index = 0;
    while index < characters.len() {
        if characters[index] != '?' {
            index += 1;
            continue;
        }
        let start = index;
        while index < characters.len() && characters[index] == '?' {
            index += 1;
        }
        let left = start.checked_sub(1).map(|position| characters[position]);
        let right = characters.get(index).copied();
        if left.is_some_and(is_cjk_context) || right.is_some_and(is_cjk_context) {
            return true;
        }
    }
    characters.iter().all(|character| *character == '?')
}

/// 词牌白名单，来自上游 `cipai_2.txt`。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CipaiList {
    tunes: BTreeSet<String>,
}

impl CipaiList {
    /// 逐行读取；空行忽略，首尾空白剪掉。空文件视为上游异常，带路径失败。
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let raw = std::fs::read_to_string(path)
            .map_err(|error| corpus_error(format!("读取 {} 失败：{error}", path.display())))?;
        let tunes: BTreeSet<String> = raw
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(str::to_owned)
            .collect();
        if tunes.is_empty() {
            return Err(corpus_error(format!(
                "词牌白名单 {} 一行都没有；没有白名单就无法安全切分词牌",
                path.display()
            )));
        }
        Ok(Self { tunes })
    }

    pub fn contains(&self, tune: &str) -> bool {
        self.tunes.contains(tune)
    }

    pub fn len(&self) -> usize {
        self.tunes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tunes.is_empty()
    }
}

/// 归一题目并判定文体。
///
/// 返回 `(title, genre)`：`title` 是写进记录的规范题目，`genre` 由词牌是否命中
/// 白名单决定。命中「词牌 题目」形态时把分隔符换成间隔号 `·`，因为
/// [`crate::model`] 的 `ci_tune` 就是按 `·` 切的；上游原串保留在
/// [`RecordInput::title_raw`] 里，一个字都不丢。
fn resolve_title(raw: &str, cipai: &CipaiList) -> (String, Genre) {
    let raw = raw.trim();
    if let Some((head, rest)) = raw.split_once('·') {
        let (head, rest) = (head.trim(), rest.trim());
        if cipai.contains(head) {
            return (format!("{head}·{rest}"), Genre::Ci);
        }
    }
    if let Some((head, rest)) = raw.split_once(' ') {
        let (head, rest) = (head.trim(), rest.trim());
        if head.chars().count() >= MIN_SPACE_SPLIT_TUNE_CHARS
            && !rest.is_empty()
            && cipai.contains(head)
        {
            return (format!("{head}·{rest}"), Genre::Ci);
        }
    }
    if raw.chars().count() >= MIN_SPACE_SPLIT_TUNE_CHARS && cipai.contains(raw) {
        return (raw.to_owned(), Genre::Ci);
    }
    (raw.to_owned(), Genre::Shi)
}

/// 把上游的单串正文切成行。
///
/// 上游把整首诗压成一个字符串，句读只有标点。切在句末标点之后（保留标点），
/// 得到的粒度与 `chinese-poetry` 的 `paragraphs` 一致——后续韵脚推导读的是
/// 行末字，粒度不一致就会读到句中字。
fn split_body_lines(body: &str) -> Vec<String> {
    let mut lines = Vec::new();
    for segment in body.split('\n') {
        let mut current = String::new();
        for character in segment.chars() {
            current.push(character);
            if matches!(character, '。' | '！' | '？' | '；') {
                let line = current.trim().to_owned();
                if !line.is_empty() {
                    lines.push(line);
                }
                current.clear();
            }
        }
        let tail = current.trim();
        if !tail.is_empty() {
            lines.push(tail.to_owned());
        }
    }
    lines
}

/// 极小 RFC 4180 读取器。
///
/// 为什么不引入 `csv` crate：工作区的依赖版本集中锁定在根 `Cargo.toml`，而这里
/// 需要的只是最规整的一种 CSV——四列全带引号、`""` 转义、CRLF 行尾（实测 28 个
/// 文件全部如此，其中 4 个文件确实出现 `""` 转义，共 130 处）。为一个状态机新增
/// 一条工作区依赖不划算。
///
/// 为什么不用 `split(',')`：那样会在带引号的字段里切错。本函数认引号，因此含
/// 逗号、含引号、含换行的字段都能原样取出。
fn parse_csv(text: &str) -> Result<Vec<Vec<String>>> {
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut row: Vec<String> = Vec::new();
    let mut field = String::new();
    let mut in_quotes = false;
    let mut row_started = false;
    let mut characters = text.chars().peekable();
    while let Some(character) = characters.next() {
        if in_quotes {
            if character == '"' {
                if characters.peek() == Some(&'"') {
                    characters.next();
                    field.push('"');
                } else {
                    in_quotes = false;
                }
            } else {
                field.push(character);
            }
            continue;
        }
        match character {
            '"' if field.is_empty() => {
                in_quotes = true;
                row_started = true;
            }
            ',' => {
                row.push(std::mem::take(&mut field));
                row_started = true;
            }
            // 只吞 CRLF 里的 CR；孤立 CR 当内容，避免把行尾判定弄成两套。
            '\r' if characters.peek() == Some(&'\n') => {}
            '\n' => {
                if row_started {
                    row.push(std::mem::take(&mut field));
                    rows.push(std::mem::take(&mut row));
                }
                row_started = false;
            }
            other => {
                field.push(other);
                row_started = true;
            }
        }
    }
    if in_quotes {
        return Err(corpus_error("CSV 在引号未闭合处结束"));
    }
    if row_started {
        row.push(field);
        rows.push(row);
    }
    Ok(rows)
}

/// 读一个分桶 CSV 并校验形状：表头逐字节相符、每行四列、至少一条数据行。
fn read_bucket(path: &Path, file: &str) -> Result<Vec<Vec<String>>> {
    let raw = std::fs::read_to_string(path)
        .map_err(|error| corpus_error(format!("读取 {} 失败：{error}", path.display())))?;
    let rows = parse_csv(&raw)
        .map_err(|error| corpus_error(format!("解析 {} 失败：{error}", path.display())))?;
    let Some(header) = rows.first() else {
        return Err(corpus_error(format!(
            "上游文件 {file} 是空文件，连表头都没有"
        )));
    };
    if header.len() != HEADER.len()
        || header
            .iter()
            .zip(HEADER)
            .any(|(actual, expected)| actual != expected)
    {
        return Err(corpus_error(format!(
            "上游文件 {file} 表头为 {header:?}，与锁定 revision 的 {HEADER:?} 不符；\
             列序变了就会整表错位，故不猜测",
        )));
    }
    let data = rows[1..].to_vec();
    if data.is_empty() {
        return Err(corpus_error(format!(
            "上游文件 {file} 只有表头，零条数据行；声明入库的分桶不得空吞"
        )));
    }
    for (ordinal, row) in data.iter().enumerate() {
        if row.len() != HEADER.len() {
            return Err(corpus_error(format!(
                "上游文件 {file} 第 {ordinal} 条有 {} 列，应为 {} 列",
                row.len(),
                HEADER.len()
            )));
        }
    }
    Ok(data)
}

fn provenance() -> Provenance {
    Provenance {
        source_name: SOURCE_NAME.to_owned(),
        source_rev: SOURCE_REV.to_owned(),
        license: LICENSE.to_owned(),
        license_class: LicenseClass::PublicDomain,
        kind: ProvenanceKind::Original,
    }
}

/// 列出仓库根下全部 CSV 文件名，用于发现白名单之外的分桶。
fn list_csv_files(root: &Path) -> Result<Vec<String>> {
    let mut files = std::fs::read_dir(root)
        .map_err(|error| corpus_error(format!("读取目录 {} 失败：{error}", root.display())))?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|error| corpus_error(format!("枚举 {} 失败：{error}", root.display())))?
        .into_iter()
        .filter(|entry| entry.path().is_file())
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| name.ends_with(".csv"))
        .collect::<Vec<_>>();
    files.sort();
    Ok(files)
}

/// 数一个被排除分桶的行数：只数，不构造任何记录。
///
/// 数出来是为了让「排除了多少」是个声明过的数字。解析失败不致命——被排除的
/// 文件本来就不进产物，把原因记在 `detail` 里即可。
fn count_excluded_rows(path: &Path) -> (usize, Option<String>) {
    match std::fs::read_to_string(path)
        .map_err(|error| error.to_string())
        .and_then(|raw| {
            parse_csv(&raw)
                .map_err(|error| error.to_string())
                .map(|rows| rows.len().saturating_sub(1))
        }) {
        Ok(rows) => (rows, None),
        Err(error) => (0, Some(error)),
    }
}

fn exclude(root: &Path, file: &str) -> BucketExclusion {
    let reason = if KNOWN_MODERN_BUCKETS.contains(&file) {
        ExclusionReason::ModernAuthorsLikelyInCopyright
    } else {
        ExclusionReason::NotOnClassicalAllowList
    };
    let (rows, error) = count_excluded_rows(&root.join(file));
    let detail = match error {
        Some(error) => format!("{}（行数未能统计：{error}）", reason.as_str()),
        None => format!("{}；已数出 {rows} 行，全部不入库", reason.as_str()),
    };
    BucketExclusion {
        file: file.to_owned(),
        reason,
        rows,
        detail,
    }
}

struct BucketContext<'a> {
    bucket: &'a Bucket,
    detector: &'a ScriptDetector,
    cipai: &'a CipaiList,
    covered: &'a CoveredWorks,
}

impl BucketContext<'_> {
    fn defect(&self, ordinal: usize, reason: DefectReason, detail: impl Into<String>) -> Defect {
        Defect {
            relative_path: self.bucket.file.to_owned(),
            ordinal,
            reason,
            detail: detail.into(),
        }
    }
}

/// 处理一条数据行。返回 `Some(record)` 才进主表。
fn ingest_row(
    context: &BucketContext<'_>,
    ordinal: usize,
    row: &[String],
    outcome: &mut WernerorOutcome,
) -> Result<Option<RecordInput>> {
    let title_raw = row[0].trim();
    let dynasty_raw = row[1].trim();
    let author = {
        let value = row[2].trim();
        if value.is_empty() { "佚名" } else { value }
    };
    let body_raw = row[3].trim();
    let locator = SourceLocator::positional(SOURCE_NAME, context.bucket.file, ordinal);

    if dynasty_raw != context.bucket.dynasty_label {
        outcome.defects.push(context.defect(
            ordinal,
            DefectReason::BucketLabelMismatch,
            format!(
                "朝代列为「{dynasty_raw}」，与分桶声明的「{}」不符；\
                 分桶里混进别的朝代可能意味着上游重排，故不入库",
                context.bucket.dynasty_label
            ),
        ));
        return Ok(None);
    }

    // 缺字优先于一切：一条被污染的行连繁简探测都不该做，更不该进主表。
    let lossy_fields: Vec<LossyField> = [
        (LossyField::Title, title_raw),
        (LossyField::Author, author),
        (LossyField::Body, body_raw),
    ]
    .into_iter()
    .filter(|(_, text)| has_lossy_char(text))
    .map(|(field, _)| field)
    .collect();
    if !lossy_fields.is_empty() {
        let names = lossy_fields
            .iter()
            .map(|field| field.as_str())
            .collect::<Vec<_>>()
            .join("、");
        outcome.defects.push(context.defect(
            ordinal,
            DefectReason::LossyCharacter,
            format!(
                "《{title_raw}》的 {names} 含 CJK 上下文中的半角 `?`：\
                 上游把生僻 utf8mb4 字符替换成了 `?`，原字不可恢复。\
                 整条隔离，不进主表——行末字被污染会让韵脚推导与破读表给出错误答案。"
            ),
        ));
        outcome.quarantined.push(QuarantinedRecord {
            source_locator: locator.as_str().to_owned(),
            relative_path: context.bucket.file.to_owned(),
            ordinal,
            title_raw: title_raw.to_owned(),
            author: author.to_owned(),
            dynasty_raw: dynasty_raw.to_owned(),
            has_lossy_char: true,
            lossy_fields,
            body_original: body_raw.to_owned(),
        });
        return Ok(None);
    }

    let body_lines = split_body_lines(body_raw);
    if body_lines.is_empty() {
        outcome.defects.push(context.defect(
            ordinal,
            DefectReason::MissingBody,
            format!("《{title_raw}》正文为空"),
        ));
        return Ok(None);
    }

    let body_original = body_lines.join("\n");
    let key = dedup_key(context.detector, &body_original);
    if context.covered.contains(&key) {
        outcome.duplicates.push(DuplicateRecord {
            source_locator: locator.as_str().to_owned(),
            relative_path: context.bucket.file.to_owned(),
            ordinal,
            title_raw: title_raw.to_owned(),
            work_group: key,
        });
        return Ok(None);
    }

    let (dynasty, dynasty_raw) = match Dynasty::canonicalize(dynasty_raw) {
        Ok(canonical) => canonical,
        Err(error) => {
            outcome.defects.push(context.defect(
                ordinal,
                DefectReason::UnknownDynasty,
                format!("《{title_raw}》的朝代串无法归一：{error}"),
            ));
            return Ok(None);
        }
    };
    let (title, genre) = resolve_title(title_raw, context.cipai);
    Ok(Some(RecordInput {
        source_locator: locator,
        genre,
        title,
        title_raw: title_raw.to_owned(),
        author: author.to_owned(),
        dynasty,
        dynasty_raw,
        script: context.detector.detect(&body_original),
        body_lines,
        body_original,
        provenance: provenance(),
    }))
}

/// 按锁定 revision 的完整检出入库全部 28 个白名单分桶。
///
/// `covered` 是 `chinese-poetry` 已收录作品的判重键集合；传
/// [`CoveredWorks::empty`] 表示不做跨来源判重。
pub fn ingest(root: impl AsRef<Path>, covered: &CoveredWorks) -> Result<WernerorOutcome> {
    ingest_buckets(root, CLASSICAL_BUCKETS, covered)
}

/// 只入库 `buckets` 指定的分桶，供 fixture 与部分检出使用。
///
/// **策略仍由 [`CLASSICAL_BUCKETS`] 全量决定**：目录里凡不在那份白名单上的 CSV
/// 一律进 [`WernerorOutcome::exclusions`]，无论本次要不要读它。换句话说这个参数
/// 只能缩小「读哪些」，不能扩大「允许哪些」。
///
/// 三种硬失败，都带文件名：声明的文件不存在、表头与锁定 revision 不符、文件只有
/// 表头。静默空吞比报错危险得多——上游一次分片重命名就能让整个分桶悄悄消失，
/// 而记录数下降在一份 76 万行的语料里没人看得出来。
pub fn ingest_buckets(
    root: impl AsRef<Path>,
    buckets: &[Bucket],
    covered: &CoveredWorks,
) -> Result<WernerorOutcome> {
    let root = root.as_ref();
    let detector = ScriptDetector::new()?;
    let cipai = CipaiList::load(root.join(CIPAI_FILE))?;
    let mut outcome = WernerorOutcome::default();

    let allowed: BTreeMap<&str, &Bucket> = CLASSICAL_BUCKETS
        .iter()
        .map(|bucket| (bucket.file, bucket))
        .collect();
    for file in list_csv_files(root)? {
        if !allowed.contains_key(file.as_str()) {
            outcome.exclusions.push(exclude(root, &file));
        }
    }

    for bucket in buckets {
        let path: PathBuf = root.join(bucket.file);
        if !path.is_file() {
            return Err(corpus_error(format!(
                "缺少上游文件：{}；白名单声明了它就必须存在",
                path.display()
            )));
        }
        let rows = read_bucket(&path, bucket.file)?;
        let context = BucketContext {
            bucket,
            detector: &detector,
            cipai: &cipai,
            covered,
        };
        let mut emitted = 0;
        for (ordinal, row) in rows.iter().enumerate() {
            if let Some(record) = ingest_row(&context, ordinal, row, &mut outcome)? {
                outcome.records.push(record);
                emitted += 1;
            }
        }
        outcome.tallies.push(FileTally {
            relative_path: bucket.file.to_owned(),
            input_records: rows.len(),
            emitted_records: emitted,
        });
    }
    Ok(outcome)
}

#[cfg(test)]
mod tests;
