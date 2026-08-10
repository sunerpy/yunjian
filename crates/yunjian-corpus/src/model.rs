//! 规范语料记录与 append-only 稳定 ID 注册表。

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::OpenOptions;
use std::io::{BufWriter, Write};
use std::path::Path;
use yunjian_core::{Error, Result};

const CONTENT_HASH_HEX_LEN: usize = 16;
const STABLE_ID_HEX_LEN: usize = 16;
const GROUP_HASH_HEX_LEN: usize = 12;

/// 规范记录的文体。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Genre {
    Shi,
    Ci,
    Qu,
    Fu,
    Wen,
}

/// 正文原始字形所用的书写系统。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Script {
    Simplified,
    Traditional,
    Mixed,
}

/// 逐记录许可分类。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LicenseClass {
    PublicDomain,
    Permissive,
    Restricted,
}

/// 文本的可查询来源种类。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProvenanceKind {
    #[serde(rename = "原文")]
    Original,
    #[serde(rename = "集评-PD")]
    PublicDomainCommentary,
    #[serde(rename = "AI")]
    Ai,
}

/// 每条记录随身携带的来源与许可证明。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Provenance {
    pub source_name: String,
    pub source_rev: String,
    pub license: String,
    pub license_class: LicenseClass,
    pub kind: ProvenanceKind,
}

/// 十五个规范朝代键；无法归一的输入应在入库阶段进入质量报告。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Dynasty {
    #[serde(rename = "先秦")]
    PreQin,
    #[serde(rename = "秦")]
    Qin,
    #[serde(rename = "汉")]
    Han,
    #[serde(rename = "三国")]
    ThreeKingdoms,
    #[serde(rename = "晋")]
    Jin,
    #[serde(rename = "南北朝")]
    NorthernSouthern,
    #[serde(rename = "隋")]
    Sui,
    #[serde(rename = "唐")]
    Tang,
    #[serde(rename = "五代十国")]
    FiveDynasties,
    #[serde(rename = "宋")]
    Song,
    #[serde(rename = "辽")]
    Liao,
    #[serde(rename = "金")]
    JurchenJin,
    #[serde(rename = "元")]
    Yuan,
    #[serde(rename = "明")]
    Ming,
    #[serde(rename = "清")]
    Qing,
}

impl Dynasty {
    pub const ALL: [Self; 15] = [
        Self::PreQin,
        Self::Qin,
        Self::Han,
        Self::ThreeKingdoms,
        Self::Jin,
        Self::NorthernSouthern,
        Self::Sui,
        Self::Tang,
        Self::FiveDynasties,
        Self::Song,
        Self::Liao,
        Self::JurchenJin,
        Self::Yuan,
        Self::Ming,
        Self::Qing,
    ];

    /// 返回写入规范记录的稳定键。
    pub const fn as_key(self) -> &'static str {
        match self {
            Self::PreQin => "先秦",
            Self::Qin => "秦",
            Self::Han => "汉",
            Self::ThreeKingdoms => "三国",
            Self::Jin => "晋",
            Self::NorthernSouthern => "南北朝",
            Self::Sui => "隋",
            Self::Tang => "唐",
            Self::FiveDynasties => "五代十国",
            Self::Song => "宋",
            Self::Liao => "辽",
            Self::JurchenJin => "金",
            Self::Yuan => "元",
            Self::Ming => "明",
            Self::Qing => "清",
        }
    }

    /// 归一朝代但原样返回输入串，供 `dynasty_raw` 永久保存。
    pub fn canonicalize(raw: &str) -> Result<(Self, String)> {
        let canonical = match raw.trim() {
            "先秦" => Self::PreQin,
            "秦" | "秦代" => Self::Qin,
            "汉" | "漢" | "汉代" | "漢代" => Self::Han,
            "三国" | "三國" => Self::ThreeKingdoms,
            "晋" | "晉" | "魏晋" | "魏晉" => Self::Jin,
            "南北朝" => Self::NorthernSouthern,
            "隋" | "隋代" | "隋末唐初" => Self::Sui,
            "唐" | "唐代" | "唐末宋初" => Self::Tang,
            "五代" | "五代十国" | "五代十國" => Self::FiveDynasties,
            "宋" | "宋代" | "宋末金初" | "宋末元初" => Self::Song,
            "辽" | "遼" => Self::Liao,
            "金" | "金末元初" => Self::JurchenJin,
            "元" | "yuan" | "元末明初" => Self::Yuan,
            "明" | "明代" | "明末清初" => Self::Ming,
            "清" | "清代" => Self::Qing,
            other => return Err(corpus_error(format!("无法归一朝代：{other}"))),
        };
        Ok((canonical, raw.to_owned()))
    }
}

/// locator 的来源；位置型 locator 才需要位移检测。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceLocatorKind {
    Native,
    Positional,
}

/// 内容无关的上游身份锚。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceLocator {
    value: String,
    kind: SourceLocatorKind,
}

impl SourceLocator {
    /// 优先使用的原生键形式，例如 `chinese-poetry:<id>`。
    pub fn native(source_name: &str, native_id: &str) -> Self {
        Self {
            value: format!("{source_name}:{native_id}"),
            kind: SourceLocatorKind::Native,
        }
    }

    /// 仅供无原生键来源使用的位置形式。
    pub fn positional(source_name: &str, relative_path: &str, ordinal: usize) -> Self {
        Self {
            value: format!("{source_name}:{relative_path}:{ordinal}"),
            kind: SourceLocatorKind::Positional,
        }
    }

    pub fn as_str(&self) -> &str {
        &self.value
    }

    pub const fn kind(&self) -> SourceLocatorKind {
        self.kind
    }
}

/// 入库器交给身份模型的规范化候选记录。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecordInput {
    pub source_locator: SourceLocator,
    pub genre: Genre,
    pub title: String,
    pub title_raw: String,
    pub author: String,
    pub dynasty: Dynasty,
    pub dynasty_raw: String,
    pub body_lines: Vec<String>,
    pub body_original: String,
    pub script: Script,
    pub provenance: Provenance,
}

/// 下游 SQLite schema 与所有用户引用共同使用的规范记录。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CanonicalRecord {
    pub stable_id: String,
    pub content_hash: String,
    pub work_group: String,
    pub edition_group: String,
    pub source_locator: String,
    pub source_locator_kind: SourceLocatorKind,
    pub genre: Genre,
    pub title: String,
    pub title_raw: String,
    pub ci_tune: Option<String>,
    pub author: String,
    pub dynasty: Dynasty,
    pub dynasty_raw: String,
    pub body_lines: Vec<String>,
    pub body_original: String,
    pub script: Script,
    pub provenance: Provenance,
}

/// append-only JSONL 注册表的三种合法事件。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event")]
pub enum RegistryEvent {
    Mint {
        source_locator: String,
        stable_id: String,
        content_hash: String,
        at_corpus_version: String,
    },
    ContentChanged {
        stable_id: String,
        from_content_hash: String,
        to_content_hash: String,
        at_corpus_version: String,
    },
    Alias {
        stable_id: String,
        from_source_locator: String,
        to_source_locator: String,
        reason: String,
        at_corpus_version: String,
    },
}

/// 单个 `stable_id` fold 后的当前状态。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FoldedRecord {
    pub stable_id: String,
    pub content_hash: String,
    pub current_locator: String,
    pub source_locators: BTreeSet<String>,
}

/// 对事件日志 fold 后的完整当前视图。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegistryState {
    pub records: BTreeMap<String, FoldedRecord>,
    pub locator_to_stable_id: BTreeMap<String, String>,
}

/// 人工审核过的移动授权。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IdMigration {
    pub stable_id: String,
    pub from_locator: String,
    pub to_locator: String,
    pub reason: String,
    pub approved_by: String,
}

/// `corpus/id_migrations.toml` 的顶层结构。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MigrationFile {
    pub migrations: Vec<IdMigration>,
}

impl MigrationFile {
    pub fn from_toml(raw: &str) -> Result<Self> {
        toml::from_str(raw).map_err(|error| corpus_error(format!("解析 ID 迁移文件失败：{error}")))
    }

    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let raw = std::fs::read_to_string(path)?;
        Self::from_toml(&raw)
            .map_err(|error| corpus_error(format!("{}（文件：{}）", error, path.display())))
    }
}

/// 一次重建的分发、隔离与注册表增量。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RebuildOutput {
    pub shippable_records: Vec<CanonicalRecord>,
    pub restricted_records: Vec<CanonicalRecord>,
    pub events: Vec<RegistryEvent>,
}

#[derive(Debug, Clone)]
struct PreparedRecord {
    input: RecordInput,
    content_hash: String,
    work_group: String,
    edition_group: String,
    ci_tune: Option<String>,
}

type CanonicalRecordIndex<'a> = BTreeMap<&'a str, &'a CanonicalRecord>;
type PreparedRecordIndex<'a> = BTreeMap<&'a str, &'a PreparedRecord>;
type PreparedHashIndex<'a> = BTreeMap<&'a str, Vec<&'a PreparedRecord>>;

fn corpus_error(message: impl Into<String>) -> Error {
    Error::Corpus(message.into())
}

fn hash_prefix(parts: &[&str], hex_len: usize) -> String {
    let mut hasher = blake3::Hasher::new();
    for part in parts {
        hasher.update(part.as_bytes());
    }
    let hex = hasher.finalize().to_hex().to_string();
    hex[..hex_len].to_owned()
}

fn normalized_body(lines: &[String]) -> String {
    lines.join("\n")
}

fn stripped_body(body: &str) -> String {
    body.chars()
        .filter(|character| {
            !character.is_whitespace()
                && !character.is_ascii_punctuation()
                && !matches!(
                    character,
                    '，' | '。'
                        | '、'
                        | '；'
                        | '：'
                        | '？'
                        | '！'
                        | '“'
                        | '”'
                        | '‘'
                        | '’'
                        | '（'
                        | '）'
                        | '《'
                        | '》'
                        | '〈'
                        | '〉'
                        | '【'
                        | '】'
                        | '〔'
                        | '〕'
                        | '—'
                        | '…'
                        | '·'
                )
        })
        .collect()
}

/// 内容校验键：作者、规范朝代、题目与规范正文任一改变都会改变它。
pub fn compute_content_hash(author: &str, dynasty: Dynasty, title: &str, body: &str) -> String {
    hash_prefix(
        &[author, dynasty.as_key(), title, body],
        CONTENT_HASH_HEX_LEN,
    )
}

/// 不含作者的作品分组键，使冲突归属可被检测。
pub fn compute_work_group(body: &str) -> String {
    let stripped = stripped_body(body);
    hash_prefix(&[&stripped], GROUP_HASH_HEX_LEN)
}

/// 含作者的版本分组键，用于同一作者名下的近同文本。
pub fn compute_edition_group(author: &str, body: &str) -> String {
    let stripped = stripped_body(body);
    hash_prefix(&[author, &stripped], GROUP_HASH_HEX_LEN)
}

/// 从 `词牌·题目` 中提取词牌；普通题目返回 `None`。
pub fn split_ci_tune(title: &str) -> Option<String> {
    let (tune, _) = title.split_once('·')?;
    (!tune.is_empty()).then(|| tune.to_owned())
}

fn mint_stable_id(identity_anchor: &str, first_seen_corpus_version: &str) -> String {
    hash_prefix(
        &[identity_anchor, first_seen_corpus_version],
        STABLE_ID_HEX_LEN,
    )
}

fn identity_anchor(record: &PreparedRecord) -> &str {
    record.input.source_locator.as_str()
}

fn prepare(input: RecordInput) -> Result<PreparedRecord> {
    let (canonical_dynasty, preserved_raw) = Dynasty::canonicalize(&input.dynasty_raw)?;
    if canonical_dynasty != input.dynasty || preserved_raw != input.dynasty_raw {
        return Err(corpus_error(format!(
            "dynasty 与 dynasty_raw 不一致：{} -> {}",
            input.dynasty_raw,
            input.dynasty.as_key()
        )));
    }
    if input.source_locator.as_str().is_empty() {
        return Err(corpus_error("source_locator 不能为空"));
    }
    let body = normalized_body(&input.body_lines);
    Ok(PreparedRecord {
        content_hash: compute_content_hash(&input.author, input.dynasty, &input.title, &body),
        work_group: compute_work_group(&body),
        edition_group: compute_edition_group(&input.author, &body),
        ci_tune: split_ci_tune(&input.title),
        input,
    })
}

fn validate_hex(value: &str, expected_len: usize, field: &str) -> Result<()> {
    if value.len() != expected_len
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(corpus_error(format!(
            "{field} 必须是 {expected_len} 位小写十六进制：{value}"
        )));
    }
    Ok(())
}

fn apply_event(state: &mut RegistryState, event: &RegistryEvent) -> Result<()> {
    match event {
        RegistryEvent::Mint {
            source_locator,
            stable_id,
            content_hash,
            at_corpus_version,
        } => {
            validate_hex(stable_id, STABLE_ID_HEX_LEN, "stable_id")?;
            validate_hex(content_hash, CONTENT_HASH_HEX_LEN, "content_hash")?;
            if at_corpus_version.is_empty() {
                return Err(corpus_error("Mint.at_corpus_version 不能为空"));
            }
            if state.records.contains_key(stable_id) {
                return Err(corpus_error(format!(
                    "stable_id 碰撞或重复 Mint：{stable_id}"
                )));
            }
            if let Some(existing) = state.locator_to_stable_id.get(source_locator) {
                return Err(corpus_error(format!(
                    "source_locator {source_locator} 已绑定 stable_id {existing}，不能再绑定 {stable_id}"
                )));
            }
            let mut source_locators = BTreeSet::new();
            source_locators.insert(source_locator.clone());
            state.records.insert(
                stable_id.clone(),
                FoldedRecord {
                    stable_id: stable_id.clone(),
                    content_hash: content_hash.clone(),
                    current_locator: source_locator.clone(),
                    source_locators,
                },
            );
            state
                .locator_to_stable_id
                .insert(source_locator.clone(), stable_id.clone());
        }
        RegistryEvent::ContentChanged {
            stable_id,
            from_content_hash,
            to_content_hash,
            at_corpus_version,
        } => {
            validate_hex(from_content_hash, CONTENT_HASH_HEX_LEN, "from_content_hash")?;
            validate_hex(to_content_hash, CONTENT_HASH_HEX_LEN, "to_content_hash")?;
            if at_corpus_version.is_empty() {
                return Err(corpus_error("ContentChanged.at_corpus_version 不能为空"));
            }
            let current = state.records.get_mut(stable_id).ok_or_else(|| {
                corpus_error(format!("ContentChanged 引用了未知 stable_id：{stable_id}"))
            })?;
            if current.content_hash != *from_content_hash {
                return Err(corpus_error(format!(
                    "ContentChanged.from_content_hash 与当前状态不符：{stable_id} 当前为 {}，事件声称为 {from_content_hash}",
                    current.content_hash
                )));
            }
            current.content_hash.clone_from(to_content_hash);
        }
        RegistryEvent::Alias {
            stable_id,
            from_source_locator,
            to_source_locator,
            reason,
            at_corpus_version,
        } => {
            if reason.trim().is_empty() || at_corpus_version.is_empty() {
                return Err(corpus_error("Alias.reason 与 at_corpus_version 不能为空"));
            }
            let from_id = state
                .locator_to_stable_id
                .get(from_source_locator)
                .ok_or_else(|| {
                    corpus_error(format!(
                        "Alias.from_source_locator 未注册：{from_source_locator}"
                    ))
                })?;
            if from_id != stable_id {
                return Err(corpus_error(format!(
                    "Alias 来源 {from_source_locator} 属于 {from_id}，不属于 {stable_id}"
                )));
            }
            if let Some(existing) = state.locator_to_stable_id.get(to_source_locator)
                && existing != stable_id
            {
                return Err(corpus_error(format!(
                    "source_locator {to_source_locator} 已绑定 stable_id {existing}，不能 alias 到 {stable_id}"
                )));
            }
            let current = state
                .records
                .get_mut(stable_id)
                .ok_or_else(|| corpus_error(format!("Alias 引用了未知 stable_id：{stable_id}")))?;
            current.source_locators.insert(from_source_locator.clone());
            current.source_locators.insert(to_source_locator.clone());
            current.current_locator.clone_from(to_source_locator);
            state
                .locator_to_stable_id
                .insert(to_source_locator.clone(), stable_id.clone());
        }
    }
    Ok(())
}

/// 将完整事件日志 fold 为当前状态；既有事件从不改写。
pub fn fold_registry(events: &[RegistryEvent]) -> Result<RegistryState> {
    let mut state = RegistryState::default();
    for event in events {
        apply_event(&mut state, event)?;
    }
    Ok(state)
}

/// 逐行读取 append-only JSONL 注册表。
pub fn load_registry_events(path: impl AsRef<Path>) -> Result<Vec<RegistryEvent>> {
    let path = path.as_ref();
    if !path.exists() {
        return Ok(Vec::new());
    }
    let raw = std::fs::read_to_string(path)?;
    raw.lines()
        .enumerate()
        .filter(|(_, line)| !line.trim().is_empty())
        .map(|(index, line)| {
            serde_json::from_str(line).map_err(|error| {
                corpus_error(format!(
                    "解析 ID 注册表失败（{}:{}）：{error}",
                    path.display(),
                    index + 1
                ))
            })
        })
        .collect()
}

/// 只以 append 模式增加事件，保证调用前的文件字节前缀保持不变。
pub fn append_registry_events(path: impl AsRef<Path>, events: &[RegistryEvent]) -> Result<()> {
    if events.is_empty() {
        return Ok(());
    }
    let path = path.as_ref();
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)?;
    }
    let file = OpenOptions::new().create(true).append(true).open(path)?;
    let mut writer = BufWriter::new(file);
    for event in events {
        serde_json::to_writer(&mut writer, event)
            .map_err(|error| corpus_error(format!("序列化 ID 注册表事件失败：{error}")))?;
        writer.write_all(b"\n")?;
    }
    writer.flush()?;
    Ok(())
}

fn previous_indexes<'a>(
    state: &RegistryState,
    previous: &'a [CanonicalRecord],
) -> Result<(CanonicalRecordIndex<'a>, CanonicalRecordIndex<'a>)> {
    let mut by_locator = BTreeMap::new();
    let mut by_stable_id = BTreeMap::new();
    for record in previous {
        if by_locator
            .insert(record.source_locator.as_str(), record)
            .is_some()
        {
            return Err(corpus_error(format!(
                "上一版快照含重复 locator：{}",
                record.source_locator
            )));
        }
        if by_stable_id
            .insert(record.stable_id.as_str(), record)
            .is_some()
        {
            return Err(corpus_error(format!(
                "上一版快照含重复 stable_id：{}",
                record.stable_id
            )));
        }
        let registered = state
            .locator_to_stable_id
            .get(&record.source_locator)
            .ok_or_else(|| {
                corpus_error(format!(
                    "上一版记录的 locator 未出现在注册表：{}",
                    record.source_locator
                ))
            })?;
        if registered != &record.stable_id {
            return Err(corpus_error(format!(
                "上一版记录与注册表身份不一致：{}",
                record.source_locator
            )));
        }
        let folded = state
            .records
            .get(&record.stable_id)
            .ok_or_else(|| corpus_error(format!("注册表缺少 stable_id：{}", record.stable_id)))?;
        if folded.content_hash != record.content_hash {
            return Err(corpus_error(format!(
                "上一版记录与注册表 content_hash 不一致：{}",
                record.stable_id
            )));
        }
    }
    Ok((by_locator, by_stable_id))
}

fn prepared_indexes(
    prepared: &[PreparedRecord],
) -> Result<(PreparedRecordIndex<'_>, PreparedHashIndex<'_>)> {
    let mut by_anchor = BTreeMap::new();
    let mut by_hash = BTreeMap::<&str, Vec<&PreparedRecord>>::new();
    for record in prepared {
        let anchor = identity_anchor(record);
        if by_anchor.insert(anchor, record).is_some() {
            return Err(corpus_error(format!(
                "本次输入含重复 source_locator：{anchor}"
            )));
        }
        by_hash
            .entry(record.content_hash.as_str())
            .or_default()
            .push(record);
    }
    Ok((by_anchor, by_hash))
}

fn detect_positional_shifts(
    prepared: &[PreparedRecord],
    previous_by_locator: &BTreeMap<&str, &CanonicalRecord>,
    previous: &[CanonicalRecord],
    current_by_hash: &BTreeMap<&str, Vec<&PreparedRecord>>,
) -> Result<()> {
    for current in prepared {
        if current.input.source_locator.kind() != SourceLocatorKind::Positional {
            continue;
        }
        let anchor = identity_anchor(current);
        let Some(old) = previous_by_locator.get(anchor) else {
            continue;
        };
        if old.content_hash == current.content_hash {
            continue;
        }

        let old_content_moved =
            current_by_hash
                .get(old.content_hash.as_str())
                .is_some_and(|records| {
                    records
                        .iter()
                        .any(|record| identity_anchor(record) != anchor)
                });
        let new_content_moved = previous.iter().any(|record| {
            record.content_hash == current.content_hash && record.source_locator != anchor
        });
        let identity_shape_changed =
            old.author != current.input.author || old.title != current.input.title;

        if old_content_moved || new_content_moved || identity_shape_changed {
            return Err(corpus_error(format!(
                "疑似位置位移：locator {anchor} 从《{}》/{} 变为《{}》/{}；禁止把它记录为 ContentChanged，需人工检查上游序号变化",
                old.title, old.author, current.input.title, current.input.author
            )));
        }
    }
    Ok(())
}

fn migration_for<'a>(
    migrations: &'a [IdMigration],
    candidates: &[&CanonicalRecord],
    to_locator: &str,
) -> Result<Option<&'a IdMigration>> {
    let matches = migrations
        .iter()
        .filter(|migration| {
            migration.to_locator == to_locator
                && candidates.iter().any(|candidate| {
                    candidate.stable_id == migration.stable_id
                        && candidate.source_locator == migration.from_locator
                })
        })
        .collect::<Vec<_>>();
    if matches.len() > 1 {
        return Err(corpus_error(format!(
            "同一目标 locator 存在多条迁移授权：{to_locator}"
        )));
    }
    let Some(migration) = matches.first().copied() else {
        return Ok(None);
    };
    if migration.reason.trim().is_empty() || migration.approved_by.trim().is_empty() {
        return Err(corpus_error(format!(
            "迁移授权缺少 reason 或 approved_by：{to_locator}"
        )));
    }
    Ok(Some(migration))
}

fn candidate_message(candidates: &[&CanonicalRecord]) -> String {
    candidates
        .iter()
        .map(|record| {
            format!(
                "{} 《{}》 {} [{}]",
                record.stable_id, record.title, record.author, record.source_locator
            )
        })
        .collect::<Vec<_>>()
        .join("；")
}

fn canonical_record(prepared: PreparedRecord, stable_id: String) -> CanonicalRecord {
    CanonicalRecord {
        stable_id,
        content_hash: prepared.content_hash,
        work_group: prepared.work_group,
        edition_group: prepared.edition_group,
        source_locator: prepared.input.source_locator.as_str().to_owned(),
        source_locator_kind: prepared.input.source_locator.kind(),
        genre: prepared.input.genre,
        title: prepared.input.title,
        title_raw: prepared.input.title_raw,
        ci_tune: prepared.ci_tune,
        author: prepared.input.author,
        dynasty: prepared.input.dynasty,
        dynasty_raw: prepared.input.dynasty_raw,
        body_lines: prepared.input.body_lines,
        body_original: prepared.input.body_original,
        script: prepared.input.script,
        provenance: prepared.input.provenance,
    }
}

/// 依据上一版快照与 fold 后注册表重建规范记录，并返回仅需追加的事件。
pub fn rebuild_corpus(
    state: &RegistryState,
    previous: &[CanonicalRecord],
    inputs: Vec<RecordInput>,
    corpus_version: &str,
    migrations: &[IdMigration],
) -> Result<RebuildOutput> {
    rebuild_corpus_internal(state, previous, inputs, corpus_version, migrations, true)
}

fn rebuild_corpus_internal(
    state: &RegistryState,
    previous: &[CanonicalRecord],
    inputs: Vec<RecordInput>,
    corpus_version: &str,
    migrations: &[IdMigration],
    detect_shifts: bool,
) -> Result<RebuildOutput> {
    if corpus_version.trim().is_empty() {
        return Err(corpus_error("corpus_version 不能为空"));
    }
    if !state.records.is_empty() && previous.is_empty() && !inputs.is_empty() {
        return Err(corpus_error(
            "非空注册表重建时必须提供上一版规范记录快照，避免位置位移漏检",
        ));
    }

    let prepared = inputs
        .into_iter()
        .map(prepare)
        .collect::<Result<Vec<_>>>()?;
    let (previous_by_locator, previous_by_stable_id) = previous_indexes(state, previous)?;
    let (current_by_anchor, current_by_hash) = prepared_indexes(&prepared)?;

    if detect_shifts {
        detect_positional_shifts(&prepared, &previous_by_locator, previous, &current_by_hash)?;
    }

    let disappeared = previous
        .iter()
        .filter(|record| !current_by_anchor.contains_key(record.source_locator.as_str()))
        .collect::<Vec<_>>();

    let mut exact_moves = BTreeMap::<String, &CanonicalRecord>::new();
    for current in &prepared {
        let anchor = identity_anchor(current);
        if state.locator_to_stable_id.contains_key(anchor) {
            continue;
        }
        let candidates = disappeared
            .iter()
            .copied()
            .filter(|old| old.content_hash == current.content_hash)
            .collect::<Vec<_>>();
        let current_hash_count = current_by_hash
            .get(current.content_hash.as_str())
            .map_or(0, Vec::len);
        if candidates.len() == 1 && current_hash_count == 1 {
            exact_moves.insert(anchor.to_owned(), candidates[0]);
        }
    }
    if exact_moves.len() > 1 {
        return Err(corpus_error(format!(
            "疑似位置位移：同一批次检测到 {} 条内容搬移；只允许自动处理干净的一对一移动",
            exact_moves.len()
        )));
    }

    let mut events = Vec::new();
    let mut assignments = BTreeMap::<String, String>::new();
    let mut assigned_ids = BTreeMap::<String, String>::new();

    for current in &prepared {
        let anchor = identity_anchor(current);
        let stable_id = if let Some(existing_id) = state.locator_to_stable_id.get(anchor) {
            let old = previous_by_stable_id
                .get(existing_id.as_str())
                .ok_or_else(|| {
                    corpus_error(format!(
                        "注册表已有 locator {anchor}，但上一版快照缺少 stable_id {existing_id}"
                    ))
                })?;
            let folded = state
                .records
                .get(existing_id)
                .ok_or_else(|| corpus_error(format!("注册表缺少 stable_id：{existing_id}")))?;
            if old.source_locator != anchor && folded.current_locator != anchor {
                events.push(RegistryEvent::Alias {
                    stable_id: existing_id.clone(),
                    from_source_locator: folded.current_locator.clone(),
                    to_source_locator: anchor.to_owned(),
                    reason: "历史 locator 再次成为当前位置".to_owned(),
                    at_corpus_version: corpus_version.to_owned(),
                });
            }
            if folded.content_hash != current.content_hash {
                events.push(RegistryEvent::ContentChanged {
                    stable_id: existing_id.clone(),
                    from_content_hash: folded.content_hash.clone(),
                    to_content_hash: current.content_hash.clone(),
                    at_corpus_version: corpus_version.to_owned(),
                });
            }
            existing_id.clone()
        } else if let Some(old) = exact_moves.get(anchor) {
            events.push(RegistryEvent::Alias {
                stable_id: old.stable_id.clone(),
                from_source_locator: old.source_locator.clone(),
                to_source_locator: anchor.to_owned(),
                reason: "内容未变的干净一对一移动".to_owned(),
                at_corpus_version: corpus_version.to_owned(),
            });
            old.stable_id.clone()
        } else {
            let candidates = disappeared
                .iter()
                .copied()
                .filter(|old| {
                    old.author == current.input.author && old.title == current.input.title
                })
                .collect::<Vec<_>>();
            if let Some(migration) = migration_for(migrations, &candidates, anchor)? {
                let old = previous_by_stable_id
                    .get(migration.stable_id.as_str())
                    .ok_or_else(|| {
                        corpus_error(format!(
                            "迁移授权引用了上一版不存在的 stable_id：{}",
                            migration.stable_id
                        ))
                    })?;
                events.push(RegistryEvent::Alias {
                    stable_id: migration.stable_id.clone(),
                    from_source_locator: migration.from_locator.clone(),
                    to_source_locator: migration.to_locator.clone(),
                    reason: format!(
                        "{}（approved_by: {}）",
                        migration.reason, migration.approved_by
                    ),
                    at_corpus_version: corpus_version.to_owned(),
                });
                if old.content_hash != current.content_hash {
                    events.push(RegistryEvent::ContentChanged {
                        stable_id: migration.stable_id.clone(),
                        from_content_hash: old.content_hash.clone(),
                        to_content_hash: current.content_hash.clone(),
                        at_corpus_version: corpus_version.to_owned(),
                    });
                }
                migration.stable_id.clone()
            } else if !candidates.is_empty() {
                return Err(corpus_error(format!(
                    "记录移动且改字，自动重匹配不可能；请在 id_migrations.toml 增加人工迁移。候选：{}；目标：{anchor}",
                    candidate_message(&candidates)
                )));
            } else {
                let new_id = mint_stable_id(anchor, corpus_version);
                if state.records.contains_key(&new_id) || assigned_ids.contains_key(&new_id) {
                    return Err(corpus_error(format!(
                        "stable_id 碰撞：{new_id}（source_locator: {anchor}）"
                    )));
                }
                events.push(RegistryEvent::Mint {
                    source_locator: anchor.to_owned(),
                    stable_id: new_id.clone(),
                    content_hash: current.content_hash.clone(),
                    at_corpus_version: corpus_version.to_owned(),
                });
                new_id
            }
        };

        if let Some(other_anchor) = assigned_ids.insert(stable_id.clone(), anchor.to_owned()) {
            return Err(corpus_error(format!(
                "同一 stable_id {stable_id} 在本次输入绑定了两个 locator：{other_anchor} 与 {anchor}"
            )));
        }
        assignments.insert(anchor.to_owned(), stable_id);
    }

    let mut next_state = state.clone();
    for event in &events {
        apply_event(&mut next_state, event)?;
    }

    let mut output = RebuildOutput {
        events,
        ..RebuildOutput::default()
    };
    for current in prepared {
        let anchor = identity_anchor(&current).to_owned();
        let stable_id = assignments
            .remove(&anchor)
            .ok_or_else(|| corpus_error(format!("缺少身份分配：{anchor}")))?;
        let record = canonical_record(current, stable_id);
        match record.provenance.license_class {
            LicenseClass::Restricted => output.restricted_records.push(record),
            LicenseClass::PublicDomain | LicenseClass::Permissive => {
                output.shippable_records.push(record);
            }
        }
    }
    Ok(output)
}

#[cfg(test)]
fn rebuild_corpus_for_test(
    state: &RegistryState,
    previous: &[CanonicalRecord],
    inputs: Vec<RecordInput>,
    corpus_version: &str,
    migrations: &[IdMigration],
    detect_shifts: bool,
) -> Result<RebuildOutput> {
    rebuild_corpus_internal(
        state,
        previous,
        inputs,
        corpus_version,
        migrations,
        detect_shifts,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};

    const V1: &str = "corpus-v1";
    const V2: &str = "corpus-v2";

    fn provenance(license_class: LicenseClass) -> Provenance {
        Provenance {
            source_name: "fixture".to_owned(),
            source_rev: "0123456789abcdef0123456789abcdef01234567".to_owned(),
            license: "MIT".to_owned(),
            license_class,
            kind: ProvenanceKind::Original,
        }
    }

    fn input(locator: SourceLocator, title: &str, author: &str, body: &str) -> RecordInput {
        RecordInput {
            source_locator: locator,
            genre: Genre::Shi,
            title: title.to_owned(),
            title_raw: title.to_owned(),
            author: author.to_owned(),
            dynasty: Dynasty::Tang,
            dynasty_raw: "唐".to_owned(),
            body_lines: vec![body.to_owned()],
            body_original: body.to_owned(),
            script: Script::Simplified,
            provenance: provenance(LicenseClass::PublicDomain),
        }
    }

    fn positional(ordinal: usize, title: &str, author: &str, body: &str) -> RecordInput {
        input(
            SourceLocator::positional("fixture", "poems.csv", ordinal),
            title,
            author,
            body,
        )
    }

    fn native(id: &str, title: &str, author: &str, body: &str) -> RecordInput {
        input(
            SourceLocator::native("chinese-poetry", id),
            title,
            author,
            body,
        )
    }

    fn all_records(output: &RebuildOutput) -> Vec<CanonicalRecord> {
        output
            .shippable_records
            .iter()
            .chain(&output.restricted_records)
            .cloned()
            .collect()
    }

    fn first_build(inputs: Vec<RecordInput>) -> (RegistryState, Vec<CanonicalRecord>) {
        let output = rebuild_corpus(&RegistryState::default(), &[], inputs, V1, &[])
            .expect("首次构建应成功");
        let state = fold_registry(&output.events).expect("Mint 日志应可 fold");
        (state, all_records(&output))
    }

    fn assert_suspected_shift(previous: &[CanonicalRecord], inputs: Vec<RecordInput>) {
        let events = previous
            .iter()
            .map(|record| RegistryEvent::Mint {
                source_locator: record.source_locator.clone(),
                stable_id: record.stable_id.clone(),
                content_hash: record.content_hash.clone(),
                at_corpus_version: V1.to_owned(),
            })
            .collect::<Vec<_>>();
        let state = fold_registry(&events).expect("fixture registry should fold");
        let error =
            rebuild_corpus(&state, previous, inputs, V2, &[]).expect_err("位置位移必须阻止构建");
        assert!(
            error.to_string().contains("疑似位置位移"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn stable_id_survives_content_correction() {
        let original = positional(0, "静夜思", "李白", "床前明月光");
        let (state, previous) = first_build(vec![original]);
        let before = &previous[0];

        let corrected = positional(0, "静夜思", "李白", "床前明月辉");
        let output = rebuild_corpus(&state, &previous, vec![corrected], V2, &[])
            .expect("同 locator 的正文修订应成功");
        let after = &output.shippable_records[0];

        assert_eq!(before.stable_id, after.stable_id);
        assert_ne!(before.content_hash, after.content_hash);
        assert_ne!(before.work_group, after.work_group);
        assert!(matches!(
            output.events.as_slice(),
            [RegistryEvent::ContentChanged { stable_id, from_content_hash, to_content_hash, .. }]
                if stable_id == &before.stable_id
                    && from_content_hash == &before.content_hash
                    && to_content_hash == &after.content_hash
        ));
    }

    #[test]
    fn fold_six_events_reconstructs_current_state() {
        let events = vec![
            RegistryEvent::Mint {
                source_locator: "source:a".to_owned(),
                stable_id: "1111111111111111".to_owned(),
                content_hash: "aaaaaaaaaaaaaaaa".to_owned(),
                at_corpus_version: "v1".to_owned(),
            },
            RegistryEvent::Mint {
                source_locator: "source:b".to_owned(),
                stable_id: "2222222222222222".to_owned(),
                content_hash: "bbbbbbbbbbbbbbbb".to_owned(),
                at_corpus_version: "v1".to_owned(),
            },
            RegistryEvent::ContentChanged {
                stable_id: "1111111111111111".to_owned(),
                from_content_hash: "aaaaaaaaaaaaaaaa".to_owned(),
                to_content_hash: "cccccccccccccccc".to_owned(),
                at_corpus_version: "v2".to_owned(),
            },
            RegistryEvent::Alias {
                stable_id: "1111111111111111".to_owned(),
                from_source_locator: "source:a".to_owned(),
                to_source_locator: "source:a-moved".to_owned(),
                reason: "upstream rename".to_owned(),
                at_corpus_version: "v2".to_owned(),
            },
            RegistryEvent::ContentChanged {
                stable_id: "2222222222222222".to_owned(),
                from_content_hash: "bbbbbbbbbbbbbbbb".to_owned(),
                to_content_hash: "dddddddddddddddd".to_owned(),
                at_corpus_version: "v3".to_owned(),
            },
            RegistryEvent::Alias {
                stable_id: "2222222222222222".to_owned(),
                from_source_locator: "source:b".to_owned(),
                to_source_locator: "source:b-moved".to_owned(),
                reason: "upstream rename".to_owned(),
                at_corpus_version: "v3".to_owned(),
            },
        ];

        let state = fold_registry(&events).expect("valid event log should fold");
        let first = state
            .records
            .get("1111111111111111")
            .expect("first id should exist");
        assert_eq!(first.content_hash, "cccccccccccccccc");
        assert_eq!(first.current_locator, "source:a-moved");
        assert_eq!(first.source_locators.len(), 2);
        assert_eq!(
            state.locator_to_stable_id.get("source:b"),
            Some(&"2222222222222222".to_owned())
        );
        assert_eq!(
            state.locator_to_stable_id.get("source:b-moved"),
            Some(&"2222222222222222".to_owned())
        );
    }

    #[test]
    fn new_locator_appends_mint() {
        let output = rebuild_corpus(
            &RegistryState::default(),
            &[],
            vec![positional(0, "春晓", "孟浩然", "春眠不觉晓")],
            V1,
            &[],
        )
        .expect("new locator should mint");
        assert!(matches!(
            output.events.as_slice(),
            [RegistryEvent::Mint { source_locator, .. }]
                if source_locator == "fixture:poems.csv:0"
        ));
    }

    #[test]
    fn moved_and_edited_record_requires_reviewed_migration() {
        let original = positional(0, "静夜思", "李白", "床前明月光");
        let (state, previous) = first_build(vec![original]);
        let moved = input(
            SourceLocator::positional("fixture", "renamed.csv", 7),
            "静夜思",
            "李白",
            "床前明月辉",
        );

        let error = rebuild_corpus(&state, &previous, vec![moved.clone()], V2, &[])
            .expect_err("moved-and-edited record cannot be guessed");
        let message = error.to_string();
        assert!(message.contains("人工迁移"));
        assert!(message.contains("候选"));
        assert!(message.contains(&previous[0].stable_id));

        let migration = IdMigration {
            stable_id: previous[0].stable_id.clone(),
            from_locator: previous[0].source_locator.clone(),
            to_locator: moved.source_locator.as_str().to_owned(),
            reason: "upstream moved and corrected the record".to_owned(),
            approved_by: "corpus-maintainer".to_owned(),
        };
        let output = rebuild_corpus(&state, &previous, vec![moved], V2, &[migration])
            .expect("reviewed migration should authorize the move");
        assert_eq!(output.shippable_records[0].stable_id, previous[0].stable_id);
        assert!(output.events.iter().any(|event| matches!(
            event,
            RegistryEvent::Alias { stable_id, .. } if stable_id == &previous[0].stable_id
        )));
        assert!(
            output
                .events
                .iter()
                .any(|event| matches!(event, RegistryEvent::ContentChanged { .. }))
        );
    }

    #[test]
    fn head_insertion_is_rejected_as_suspected_shift() {
        let (_, previous) = first_build(vec![
            positional(0, "甲", "作者甲", "甲正文"),
            positional(1, "乙", "作者乙", "乙正文"),
            positional(2, "丙", "作者丙", "丙正文"),
        ]);
        assert_suspected_shift(
            &previous,
            vec![
                positional(0, "新", "新作者", "新正文"),
                positional(1, "甲", "作者甲", "甲正文"),
                positional(2, "乙", "作者乙", "乙正文"),
                positional(3, "丙", "作者丙", "丙正文"),
            ],
        );
    }

    #[test]
    fn middle_deletion_is_rejected_as_suspected_shift() {
        let (_, previous) = first_build(vec![
            positional(0, "甲", "作者甲", "甲正文"),
            positional(1, "乙", "作者乙", "乙正文"),
            positional(2, "丙", "作者丙", "丙正文"),
        ]);
        assert_suspected_shift(
            &previous,
            vec![
                positional(0, "甲", "作者甲", "甲正文"),
                positional(1, "丙", "作者丙", "丙正文"),
            ],
        );
    }

    #[test]
    fn two_record_reorder_is_rejected_as_suspected_shift() {
        let (_, previous) = first_build(vec![
            positional(0, "甲", "作者甲", "甲正文"),
            positional(1, "乙", "作者乙", "乙正文"),
        ]);
        assert_suspected_shift(
            &previous,
            vec![
                positional(0, "乙", "作者乙", "乙正文"),
                positional(1, "甲", "作者甲", "甲正文"),
            ],
        );
    }

    #[test]
    fn positional_locator_reuse_is_rejected_as_suspected_shift() {
        let (_, previous) = first_build(vec![
            positional(0, "甲", "作者甲", "甲正文"),
            positional(1, "乙", "作者乙", "乙正文"),
            positional(2, "丙", "作者丙", "丙正文"),
        ]);
        assert_suspected_shift(
            &previous,
            vec![
                positional(0, "甲", "作者甲", "甲正文"),
                positional(1, "新", "新作者", "完全无关的新正文"),
                positional(2, "丙", "作者丙", "丙正文"),
            ],
        );
    }

    #[test]
    fn disabling_shift_detection_demonstrates_silent_id_reassignment() {
        let inputs = vec![
            positional(0, "甲", "作者甲", "甲正文"),
            positional(1, "乙", "作者乙", "乙正文"),
            positional(2, "丙", "作者丙", "丙正文"),
        ];
        let (state, previous) = first_build(inputs);
        let shifted = vec![
            positional(0, "新", "新作者", "新正文"),
            positional(1, "甲", "作者甲", "甲正文"),
            positional(2, "乙", "作者乙", "乙正文"),
            positional(3, "丙", "作者丙", "丙正文"),
        ];

        let output = rebuild_corpus_for_test(&state, &previous, shifted, V2, &[], false)
            .expect("without the detector the corrupt rebuild appears successful");
        let rebuilt = all_records(&output);
        for old in &previous {
            let now = rebuilt
                .iter()
                .find(|record| record.stable_id == old.stable_id)
                .expect("every old id is still present but attached to another work");
            assert_ne!(old.title, now.title, "stable id was silently reassigned");
        }
    }

    #[test]
    fn locator_cannot_be_bound_to_a_second_stable_id() {
        let events = vec![
            RegistryEvent::Mint {
                source_locator: "source:one".to_owned(),
                stable_id: "1111111111111111".to_owned(),
                content_hash: "aaaaaaaaaaaaaaaa".to_owned(),
                at_corpus_version: "v1".to_owned(),
            },
            RegistryEvent::Mint {
                source_locator: "source:one".to_owned(),
                stable_id: "2222222222222222".to_owned(),
                content_hash: "bbbbbbbbbbbbbbbb".to_owned(),
                at_corpus_version: "v2".to_owned(),
            },
        ];
        let error = fold_registry(&events).expect_err("locator reuse must fail");
        assert!(error.to_string().contains("已绑定"));
    }

    #[test]
    fn native_keys_survive_wholesale_reordering() {
        let inputs = vec![
            native("id-a", "甲", "作者甲", "甲正文"),
            native("id-b", "乙", "作者乙", "乙正文"),
            native("id-c", "丙", "作者丙", "丙正文"),
        ];
        let (state, previous) = first_build(inputs);
        let before = previous
            .iter()
            .map(|record| (record.source_locator.clone(), record.stable_id.clone()))
            .collect::<BTreeMap<_, _>>();
        let reordered = vec![
            native("id-c", "丙", "作者丙", "丙正文"),
            native("id-a", "甲", "作者甲", "甲正文"),
            native("id-b", "乙", "作者乙", "乙正文"),
        ];
        let output = rebuild_corpus(&state, &previous, reordered, V2, &[])
            .expect("native keys are order-independent");
        let after = output
            .shippable_records
            .iter()
            .map(|record| (record.source_locator.clone(), record.stable_id.clone()))
            .collect::<BTreeMap<_, _>>();
        assert_eq!(before, after);
        assert!(output.events.is_empty());
    }

    #[test]
    fn appending_events_never_rewrites_existing_log_bytes() {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "yunjian-id-registry-{}-{}.jsonl",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let mint = RegistryEvent::Mint {
            source_locator: "source:one".to_owned(),
            stable_id: "1111111111111111".to_owned(),
            content_hash: "aaaaaaaaaaaaaaaa".to_owned(),
            at_corpus_version: "v1".to_owned(),
        };
        append_registry_events(&path, std::slice::from_ref(&mint)).expect("first append");
        let prefix = fs::read(&path).expect("read prefix");
        let changed = RegistryEvent::ContentChanged {
            stable_id: "1111111111111111".to_owned(),
            from_content_hash: "aaaaaaaaaaaaaaaa".to_owned(),
            to_content_hash: "bbbbbbbbbbbbbbbb".to_owned(),
            at_corpus_version: "v2".to_owned(),
        };
        append_registry_events(&path, &[changed]).expect("second append");
        let after = fs::read(&path).expect("read full log");
        assert!(after.starts_with(&prefix));
        assert!(after.len() > prefix.len());
        let loaded = load_registry_events(&path).expect("log should parse");
        assert_eq!(loaded.len(), 2);
        fs::remove_file(path).expect("remove temp registry");
    }

    #[test]
    fn work_group_ignores_author_but_edition_group_does_not() {
        let body = "大江东去，浪淘尽。";
        assert_eq!(compute_work_group(body), compute_work_group(body));
        assert_eq!(
            compute_work_group(body),
            compute_work_group("大江东去 浪淘尽")
        );
        assert_ne!(
            compute_edition_group("苏轼", body),
            compute_edition_group("辛弃疾", body)
        );
        let first = input(SourceLocator::native("fixture", "a"), "赤壁", "苏轼", body);
        let second = input(
            SourceLocator::native("fixture", "b"),
            "赤壁",
            "辛弃疾",
            body,
        );
        let output = rebuild_corpus(&RegistryState::default(), &[], vec![first, second], V1, &[])
            .expect("same body under two authors is retained");
        assert_eq!(
            output.shippable_records[0].work_group,
            output.shippable_records[1].work_group
        );
        assert_ne!(
            output.shippable_records[0].edition_group,
            output.shippable_records[1].edition_group
        );
    }

    #[test]
    fn ci_tune_is_split_from_combined_title() {
        assert_eq!(split_ci_tune("念奴娇·赤壁怀古"), Some("念奴娇".to_owned()));
        assert_eq!(split_ci_tune("静夜思"), None);
    }

    #[test]
    fn dynasty_raw_is_reversible_for_all_keys_and_cross_dynasty_label() {
        assert_eq!(Dynasty::ALL.len(), 15);
        for dynasty in Dynasty::ALL {
            let raw = dynasty.as_key();
            let (canonical, preserved) = Dynasty::canonicalize(raw).expect("canonical key");
            assert_eq!(canonical, dynasty);
            assert_eq!(preserved, raw);
        }
        let (canonical, preserved) = Dynasty::canonicalize("唐末宋初").expect("cross dynasty");
        assert_eq!(canonical, Dynasty::Tang);
        assert_eq!(preserved, "唐末宋初");
    }

    #[test]
    fn restricted_records_never_enter_shippable_collection() {
        let mut restricted = positional(0, "受限", "作者", "受限正文");
        restricted.provenance = provenance(LicenseClass::Restricted);
        let output = rebuild_corpus(&RegistryState::default(), &[], vec![restricted], V1, &[])
            .expect("restricted input is retained separately");
        assert!(output.shippable_records.is_empty());
        assert_eq!(output.restricted_records.len(), 1);
    }

    #[test]
    fn every_record_has_a_non_empty_provenance_kind() {
        let output = rebuild_corpus(
            &RegistryState::default(),
            &[],
            vec![positional(0, "春晓", "孟浩然", "春眠不觉晓")],
            V1,
            &[],
        )
        .expect("record should build");
        let value = serde_json::to_value(&output.shippable_records[0]).expect("serialize record");
        assert_eq!(value["provenance"]["kind"], "原文");
        assert!(
            !value["provenance"]["kind"]
                .as_str()
                .expect("kind is a string")
                .is_empty()
        );
    }

    #[test]
    fn clean_one_to_one_move_is_auto_aliased() {
        let original = positional(0, "静夜思", "李白", "床前明月光");
        let (state, previous) = first_build(vec![original]);
        let moved = input(
            SourceLocator::positional("fixture", "renamed.csv", 0),
            "静夜思",
            "李白",
            "床前明月光",
        );
        let output = rebuild_corpus(&state, &previous, vec![moved], V2, &[])
            .expect("one unchanged move should alias automatically");
        assert_eq!(output.shippable_records[0].stable_id, previous[0].stable_id);
        assert!(matches!(
            output.events.as_slice(),
            [RegistryEvent::Alias { stable_id, reason, .. }]
                if stable_id == &previous[0].stable_id && reason.contains("一对一")
        ));
    }

    #[test]
    fn migration_file_parses_one_line_authorization() {
        let raw = r#"migrations = [{ stable_id = "1111111111111111", from_locator = "a:0", to_locator = "b:0", reason = "reviewed move", approved_by = "maintainer" }]"#;
        let file = MigrationFile::from_toml(raw).expect("migration TOML should parse");
        assert_eq!(file.migrations.len(), 1);
        assert_eq!(file.migrations[0].approved_by, "maintainer");
    }
}
