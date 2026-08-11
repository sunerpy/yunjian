//! 构建期主题标签打标。
//!
//! # 这个模块存在的理由
//!
//! 标签决定用户在 `browse_by_tag` 里看到哪些诗，选本标签还进入检索排序的静态显赫度。
//! 因此每一条标签都必须能回答「凭什么」。本模块把这个问题的答案收敛成两种、且只有
//! 两种可复核的来源，都写在签入的 [`VOCABULARY_TOML`] 里：
//!
//! 1. **关键词规则**——正文或题目里出现某个字面串。任何一条这样得来的标签，
//!    都能用一次字符串查找复现。
//! 2. **评审名单**——按 `(author, title)` 显式增删，每条附理由。
//!
//! **没有第三种来源。** [`assign_tags`] 的入参只有词表与记录，不接受模型输出，
//! 也不产出词表未声明的标签名。
//!
//! # 为什么匹配用归一后的文本
//!
//! 词表里的关键词是规范简体（「早发」），而 `chinese-poetry/全唐诗` 的题目与作者是
//! 繁体（「早發白帝城」）。若直接拿原串匹配，规则在语料的一大半上静默失效——不报错，
//! 只是一条标签都不出。所以标题、作者与正文都先过 [`Normalizer::canonicalize`]，
//! 与全文索引所用的是同一个折叠器；两处用不同折叠器会让「规则命中」与「检索命中」
//! 对同一个字给出不同答案。
//!
//! # 无效 `deny` 是硬错误
//!
//! 一条什么都没移除的 `deny` 是死配置，会让人误以为某个误报已被处理。
//! [`assign_tags`] 因此在评审名单里存在无效 `deny` 时失败。`add` 与规则重复**不是**
//! 错误：那是把契约锚定的那几首诗钉住，使它们不随规则措辞的改动而漂移。

#[cfg(test)]
mod tests;

use crate::db::PoemTagRow;
use crate::model::CanonicalRecord;
use crate::normalize::{NormalizedRecord, Normalizer};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use yunjian_core::{Error, Result};

/// 签入的策展词表。构建期打标的唯一事实来源。
pub const VOCABULARY_TOML: &str = include_str!("../tags.toml");

/// 当前词表的 schema 版本。
pub const VOCABULARY_SCHEMA_VERSION: u32 = 1;

/// 评审理由的最短长度（字符）。比这更短的不可能说清「凭什么这样标」。
const MIN_REASON_CHARS: usize = 8;

fn tag_error(message: impl Into<String>) -> Error {
    Error::Corpus(message.into())
}

/// 标签的类别。
///
/// 类别不只是展示分组：[`TagKind::Anthology`] 在结构上不允许有关键词规则，因为
/// 「某选本收了这首诗」是历史事实而不是文本特征，任何字面串都推不出它。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TagKind {
    /// 主题，如 思乡 / 送别 / 边塞。
    Theme,
    /// 物象，如 月 / 雨。
    Imagery,
    /// 季节。
    Season,
    /// 节日。
    Festival,
    /// 地域。
    Place,
    /// 选本收录。
    Anthology,
}

impl TagKind {
    /// 写进数据库与报告的稳定键。
    #[must_use]
    pub const fn as_key(self) -> &'static str {
        match self {
            Self::Theme => "theme",
            Self::Imagery => "imagery",
            Self::Season => "season",
            Self::Festival => "festival",
            Self::Place => "place",
            Self::Anthology => "anthology",
        }
    }
}

/// 一个标签的声明与它的关键词规则。
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TagDeclaration {
    /// 标签名。同时是数据库里的主键，所以它就是用户看到的那个串。
    pub name: String,
    /// 类别。
    pub kind: TagKind,
    /// 一句话释义，供界面与复核使用。
    pub gloss: String,
    /// 正文关键词。
    #[serde(default)]
    pub body_keywords: Vec<String>,
    /// 题目关键词。
    #[serde(default)]
    pub title_keywords: Vec<String>,
    /// 没有关键词规则时，必须写明为什么没有。
    #[serde(default)]
    pub rule_note: String,
}

impl TagDeclaration {
    /// 该标签有没有关键词规则。
    #[must_use]
    pub fn has_rules(&self) -> bool {
        !self.body_keywords.is_empty() || !self.title_keywords.is_empty()
    }

    /// 规则是否命中这首诗。题目与正文任一命中即算。
    fn matches(&self, title: &str, body: &str) -> bool {
        self.body_keywords
            .iter()
            .any(|keyword| body.contains(keyword))
            || self
                .title_keywords
                .iter()
                .any(|keyword| title.contains(keyword))
    }
}

/// 评审名单的一条。
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewedEntry {
    /// 作者，规范简体。
    pub author: String,
    /// 题目，规范简体。
    pub title: String,
    /// 追加的标签。允许与规则重复。
    #[serde(default)]
    pub add: Vec<String>,
    /// 移除的标签。必须真的移除掉某个东西，否则构建失败。
    #[serde(default)]
    pub deny: Vec<String>,
    /// 为什么这样标。空值让构建失败。
    pub reason: String,
}

impl ReviewedEntry {
    fn key(&self) -> (&str, &str) {
        (self.author.as_str(), self.title.as_str())
    }
}

/// 已校验的策展词表。
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TagVocabulary {
    /// 词表自身的版本。
    pub schema_version: u32,
    /// 标签声明。
    #[serde(rename = "tag")]
    pub tags: Vec<TagDeclaration>,
    /// 评审名单。
    #[serde(rename = "reviewed", default)]
    pub reviewed: Vec<ReviewedEntry>,
}

impl TagVocabulary {
    /// 解析并校验签入的词表。
    pub fn shipped() -> Result<Self> {
        Self::parse(VOCABULARY_TOML)
    }

    /// 解析并校验任意一份词表文本。
    pub fn parse(text: &str) -> Result<Self> {
        let vocabulary: Self = toml::from_str(text)
            .map_err(|error| tag_error(format!("解析标签词表失败：{error}")))?;
        vocabulary.validate()?;
        Ok(vocabulary)
    }

    /// 全部标签名，有序去重。
    #[must_use]
    pub fn names(&self) -> BTreeSet<&str> {
        self.tags.iter().map(|tag| tag.name.as_str()).collect()
    }

    /// 按名取声明。
    #[must_use]
    pub fn declaration(&self, name: &str) -> Option<&TagDeclaration> {
        self.tags.iter().find(|tag| tag.name == name)
    }

    fn validate(&self) -> Result<()> {
        if self.schema_version != VOCABULARY_SCHEMA_VERSION {
            return Err(tag_error(format!(
                "标签词表 schema_version 为 {}，本构建只认 {VOCABULARY_SCHEMA_VERSION}",
                self.schema_version
            )));
        }
        if self.tags.is_empty() {
            return Err(tag_error("标签词表里没有任何标签声明"));
        }
        let mut seen = BTreeSet::new();
        for tag in &self.tags {
            if tag.name.trim().is_empty() {
                return Err(tag_error("标签名不得为空"));
            }
            if !seen.insert(tag.name.as_str()) {
                return Err(tag_error(format!("标签 `{}` 重复声明", tag.name)));
            }
            if tag.gloss.trim().is_empty() {
                return Err(tag_error(format!("标签 `{}` 缺少 gloss 释义", tag.name)));
            }
            if !tag.has_rules() && tag.rule_note.trim().is_empty() {
                return Err(tag_error(format!(
                    "标签 `{}` 没有任何关键词规则，必须在 rule_note 里写明为什么没有\
                     ——「没有规则」是一条设计判断，不是省略",
                    tag.name
                )));
            }
            if tag.kind == TagKind::Anthology && tag.has_rules() {
                return Err(tag_error(format!(
                    "选本标签 `{}` 不得有关键词规则：某选本收了哪首诗是历史事实而非\
                     文本特征，任何字面串都推不出它",
                    tag.name
                )));
            }
            // 逐列查重而不是合起来查：同一个串出现在正文与题目两列是正常的
            // （「相思」既是正文语也是题名），它们匹配的是不同字段。
            for (column, keywords) in [
                ("body_keywords", &tag.body_keywords),
                ("title_keywords", &tag.title_keywords),
            ] {
                let mut seen_keywords = BTreeSet::new();
                for keyword in keywords {
                    if keyword.trim().is_empty() {
                        return Err(tag_error(format!(
                            "标签 `{}` 的 {column} 里有空关键词",
                            tag.name
                        )));
                    }
                    if !seen_keywords.insert(keyword.as_str()) {
                        return Err(tag_error(format!(
                            "标签 `{}` 的 {column} 里 `{keyword}` 重复",
                            tag.name
                        )));
                    }
                }
            }
        }

        let mut reviewed_keys = BTreeSet::new();
        for entry in &self.reviewed {
            if entry.author.trim().is_empty() || entry.title.trim().is_empty() {
                return Err(tag_error("评审名单的 author 与 title 都不得为空"));
            }
            if !reviewed_keys.insert(entry.key()) {
                return Err(tag_error(format!(
                    "评审名单里 {}《{}》出现两条；同一首诗的增删必须写在一条里，\
                     否则读者无法判断哪一条生效",
                    entry.author, entry.title
                )));
            }
            if entry.reason.chars().count() < MIN_REASON_CHARS {
                return Err(tag_error(format!(
                    "{}《{}》的 reason 太短（{} 字）：「为什么这首诗是这个标签」是标签\
                     体系全部可审计性的落点",
                    entry.author,
                    entry.title,
                    entry.reason.chars().count()
                )));
            }
            if entry.add.is_empty() && entry.deny.is_empty() {
                return Err(tag_error(format!(
                    "{}《{}》的评审条目既不增也不删，是一条死配置",
                    entry.author, entry.title
                )));
            }
            for name in entry.add.iter().chain(&entry.deny) {
                if !seen.contains(name.as_str()) {
                    return Err(tag_error(format!(
                        "{}《{}》引用了未声明的标签 `{name}`",
                        entry.author, entry.title
                    )));
                }
            }
            let added: BTreeSet<&str> = entry.add.iter().map(String::as_str).collect();
            if let Some(conflict) = entry.deny.iter().find(|name| added.contains(name.as_str())) {
                return Err(tag_error(format!(
                    "{}《{}》同时 add 与 deny 了 `{conflict}`",
                    entry.author, entry.title
                )));
            }
        }
        Ok(())
    }
}

/// 一次打标的账目。
///
/// 逐标签与逐评审条目的命中数都要报出来，因为评审名单按 `(author, title)` 匹配，
/// 同作者的同名作品会被一并打上——那不一定是错的，但必须看得见。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct TagReport {
    /// 至少带一个标签的作品数。
    pub tagged_poems: usize,
    /// 产出的 `poem_tag` 行数。
    pub rows: usize,
    /// 逐标签命中的作品数。
    pub per_tag: BTreeMap<String, usize>,
    /// 逐评审条目命中的作品数，键为 `作者《题目》`。
    pub reviewed_hits: BTreeMap<String, usize>,
}

/// 打标产出。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TagAssignment {
    /// 可直接写库的 `poem_tag` 行，按 `(poem_id, tag)` 有序。
    pub rows: Vec<PoemTagRow>,
    /// 账目。
    pub report: TagReport,
}

/// 打标只需要的四个字段。
///
/// 单独立出来而不是直接吃 [`CanonicalRecord`]，是为了让规则与评审名单能在不构造整条
/// 语料记录（十七个字段）的前提下被测——判据只用到这四个，多要一个字段就多一处
/// 「测试里填了什么」与「生产里是什么」可能不一致的地方。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PoemFacts<'a> {
    /// 作品的稳定标识。
    pub stable_id: &'a str,
    /// 作者原串（本模块内部归一，调用方不必先折叠）。
    pub author: &'a str,
    /// 题目原串。
    pub title: &'a str,
    /// 正文；若已有规范简体正文应传它，否则传原串。
    pub body: &'a str,
}

/// 按词表给每条记录打标。
///
/// `normalized` 提供规范简体正文；缺某条记录的归一结果时回退到 `body_lines` 的拼接，
/// 因为归一是另一道工序，本模块不该因为它缺一条就整体失败。
pub fn assign_tags(
    vocabulary: &TagVocabulary,
    records: &[CanonicalRecord],
    normalized: &[NormalizedRecord],
    normalizer: &Normalizer,
) -> Result<TagAssignment> {
    let bodies: BTreeMap<&str, &str> = normalized
        .iter()
        .map(|record| (record.stable_id.as_str(), record.body.as_str()))
        .collect();
    let fallbacks: Vec<String> = records
        .iter()
        .map(|record| record.body_lines.join(""))
        .collect();
    let facts: Vec<PoemFacts<'_>> = records
        .iter()
        .zip(&fallbacks)
        .map(|(record, fallback)| PoemFacts {
            stable_id: record.stable_id.as_str(),
            author: record.author.as_str(),
            title: record.title.as_str(),
            body: bodies
                .get(record.stable_id.as_str())
                .copied()
                .unwrap_or(fallback.as_str()),
        })
        .collect();
    assign_tags_to_facts(vocabulary, &facts, normalizer)
}

/// [`assign_tags`] 的实现体，也是测试的入口。生产与测试共用这一份逻辑。
pub fn assign_tags_to_facts(
    vocabulary: &TagVocabulary,
    facts: &[PoemFacts<'_>],
    normalizer: &Normalizer,
) -> Result<TagAssignment> {
    let reviewed: BTreeMap<(&str, &str), &ReviewedEntry> = vocabulary
        .reviewed
        .iter()
        .map(|entry| (entry.key(), entry))
        .collect();

    let mut rows = Vec::new();
    let mut report = TagReport::default();
    let mut deny_used: BTreeSet<(&str, &str, &str)> = BTreeSet::new();
    let mut entry_matched: BTreeSet<(&str, &str)> = BTreeSet::new();

    for poem in facts {
        let title = normalizer.canonicalize(poem.title);
        let author = normalizer.canonicalize(poem.author);
        let body = normalizer.canonicalize(poem.body);

        let mut assigned: BTreeSet<&str> = vocabulary
            .tags
            .iter()
            .filter(|tag| tag.matches(&title, &body))
            .map(|tag| tag.name.as_str())
            .collect();

        if let Some(entry) = reviewed.get(&(author.as_str(), title.as_str())) {
            entry_matched.insert(entry.key());
            for name in &entry.add {
                assigned.insert(name.as_str());
            }
            for name in &entry.deny {
                if assigned.remove(name.as_str()) {
                    deny_used.insert((entry.author.as_str(), entry.title.as_str(), name.as_str()));
                }
            }
            *report
                .reviewed_hits
                .entry(format!("{}《{}》", entry.author, entry.title))
                .or_default() += 1;
        }

        if assigned.is_empty() {
            continue;
        }
        report.tagged_poems += 1;
        for name in assigned {
            *report.per_tag.entry(name.to_owned()).or_default() += 1;
            rows.push(PoemTagRow {
                poem_id: poem.stable_id.to_owned(),
                tag: name.to_owned(),
            });
        }
    }

    // 无效 `deny` 是硬错误：它什么都没移除，却让读者以为某个误报已被处理。
    //
    // **只对命中过的评审条目判定。** 一条评审条目所指的诗不在本次构建范围内时
    // （`corpus-measure` 会在 10k 抽样上跑），它的 deny 根本没有机会生效，那不是
    // 死配置而是范围之外——把两者混为一谈会让抽样规模的构建全部失败。
    let mut dead = Vec::new();
    for entry in &vocabulary.reviewed {
        if !entry_matched.contains(&entry.key()) {
            continue;
        }
        for name in &entry.deny {
            if !deny_used.contains(&(entry.author.as_str(), entry.title.as_str(), name.as_str())) {
                dead.push(format!("{}《{}》→ {name}", entry.author, entry.title));
            }
        }
    }
    if !dead.is_empty() {
        return Err(tag_error(format!(
            "评审名单里有 {} 条 deny 一个标签都没移除，是死配置：{}。\
             规则若已不会误报，就该把这条 deny 删掉，而不是留着让人以为它在起作用",
            dead.len(),
            dead.join("；")
        )));
    }

    rows.sort_by(|left, right| (&left.poem_id, &left.tag).cmp(&(&right.poem_id, &right.tag)));
    report.rows = rows.len();
    Ok(TagAssignment { rows, report })
}
