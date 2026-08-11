use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use yunjian_core::{Error, Result, content_chars, split_metrical_lines};

use crate::model::{CanonicalRecord, Genre};

const YUEFU_TITLES_TOML: &str = include_str!("../../../corpus/yuefu_titles.toml");

/// 作品的结构化体裁。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Form {
    /// 五言绝句。
    Wujue,
    /// 七言绝句。
    Qijue,
    /// 五言律诗。
    Wulv,
    /// 七言律诗。
    Qilv,
    /// 来源直接标明的乐府作品。
    Yuefu,
    /// 词。
    Ci,
    /// 结构存在但不符合四种近体诗定长模式。
    Irregular,
    /// 缺少足够信息，无法判定。
    Unknown,
}

impl Form {
    /// 返回 SQLite 中使用的稳定键。
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Wujue => "wujue",
            Self::Qijue => "qijue",
            Self::Wulv => "wulv",
            Self::Qilv => "qilv",
            Self::Yuefu => "yuefu",
            Self::Ci => "ci",
            Self::Irregular => "irregular",
            Self::Unknown => "unknown",
        }
    }
}

/// 一首作品的结构化体裁与独立乐府标记。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FormClassification {
    /// 主体裁；乐府旧题不会覆盖可由结构确认的近体诗体裁。
    pub form: Form,
    /// 标题是否为签入清单中的乐府旧题或以旧题开头。
    pub is_yuefu: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct YuefuTitleList {
    title: Vec<YuefuTitle>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct YuefuTitle {
    value: String,
    source: String,
    note: String,
}

fn corpus_error(message: impl Into<String>) -> Error {
    Error::Corpus(message.into())
}

fn yuefu_titles() -> Result<BTreeSet<String>> {
    let list: YuefuTitleList = toml::from_str(YUEFU_TITLES_TOML)
        .map_err(|error| corpus_error(format!("解析 corpus/yuefu_titles.toml 失败：{error}")))?;
    let mut titles = BTreeSet::new();
    for entry in list.title {
        if entry.value.trim().is_empty()
            || entry.source.trim().is_empty()
            || entry.note.trim().is_empty()
        {
            return Err(corpus_error(
                "乐府旧题条目必须同时包含非空 value、source 与 note",
            ));
        }
        if !titles.insert(entry.value) {
            return Err(corpus_error("corpus/yuefu_titles.toml 含重复旧题"));
        }
    }
    Ok(titles)
}

fn source_form(record: &CanonicalRecord) -> Option<Form> {
    if record.provenance.source_name == "chinese-poetry/chinese-poetry" {
        if record.source_locator.contains(":诗经/")
            || record.source_locator.contains(":楚辞/")
            || record.source_locator.contains(":元曲/")
        {
            return Some(Form::Irregular);
        }
        if record.source_locator.contains(":五代诗词/") {
            return Some(Form::Ci);
        }
    }
    match record.genre {
        Genre::Ci => Some(Form::Ci),
        Genre::Qu | Genre::Fu | Genre::Wen => Some(Form::Irregular),
        Genre::Shi => None,
    }
}

fn structural_form(body: &str) -> Form {
    let lengths = split_metrical_lines(body)
        .map(|line| content_chars(line).count())
        .filter(|length| *length > 0)
        .collect::<Vec<_>>();
    match lengths.as_slice() {
        [5, 5, 5, 5] => Form::Wujue,
        [7, 7, 7, 7] => Form::Qijue,
        [5, 5, 5, 5, 5, 5, 5, 5] => Form::Wulv,
        [7, 7, 7, 7, 7, 7, 7, 7] => Form::Qilv,
        [] => Form::Unknown,
        _ => Form::Irregular,
    }
}

/// 按来源、词牌、乐府旧题与格律结构的固定优先级判定体裁。
pub fn classify(record: &CanonicalRecord) -> Result<FormClassification> {
    let titles = yuefu_titles()?;
    let is_yuefu = titles
        .iter()
        .any(|title| record.title == *title || record.title.starts_with(title));
    let form = source_form(record)
        .or_else(|| record.ci_tune.as_ref().map(|_| Form::Ci))
        .unwrap_or_else(|| structural_form(&record.body_original));
    let form = if form == Form::Unknown && is_yuefu {
        Form::Yuefu
    } else {
        form
    };
    Ok(FormClassification { form, is_yuefu })
}

#[cfg(test)]
mod tests;
