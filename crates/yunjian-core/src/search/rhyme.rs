//! 韵书维度的押韵检索。
//!
//! 三个入口全部落在 `rhyme` 与 `poem_rhyme_group` 两张普通表上，走 B-tree 等值连接，
//! **一个都不碰 FTS**：押韵是一条查表关系，不是文本相似度。
//!
//! # 韵书是必填维度
//!
//! 每个入口都要求传入 [`RhymeBook`]，且没有默认值。平水韵是诗韵，对词牌格律并不适用，
//! 所以「这两个字押韵吗」在不指定韵书时根本没有答案；给 `book` 加一个隐式默认，等于让
//! 调用方能不知不觉拿一首词去对平水韵。带韵书的调用编译通过：
//!
//! ```
//! # use yunjian_core::rhyme::RhymeBook;
//! # use yunjian_core::{CorpusHandle, search::rhyme::rhyme_groups_of};
//! # fn demo(handle: &CorpusHandle) {
//! let _ = rhyme_groups_of(handle, '东', RhymeBook::Pingshui);
//! # }
//! ```
//!
//! 去掉韵书则编译不过。两条 doctest 成对存在：单看失败那条无法排除「因别的原因失败」，
//! 有了上面这条对照，失败的原因就只能是缺了韵书维度。
//!
//! ```compile_fail
//! # use yunjian_core::{CorpusHandle, search::rhyme::rhyme_groups_of};
//! # fn demo(handle: &CorpusHandle) {
//! let _ = rhyme_groups_of(handle, '东');
//! # }
//! ```
//!
//! # 三种「否」必须分得开
//!
//! 押韵检索有三种截然不同的负面答案，混在一起就是对格律的虚假陈述：
//!
//! | 情形 | 表达 | 为什么不能合并 |
//! |---|---|---|
//! | 韵书未随包（中华新韵） | [`Error::RhymeBookUnavailable`] | 我们没有这本书，不是这些字不押韵 |
//! | 字不在这本韵书里 | [`RhymeVerdict::Indeterminate`] | 书在、字不在，无从肯定也无从否定 |
//! | 字都在、但不同韵部 | [`RhymeVerdict::NoRhyme`] | 这一条才是真正的否定判断 |
//!
//! 同理，作品的韵部归属若 [`RhymeConfidence::Unresolved`]（todo 18 的韵脚投票没能唯一
//! 消歧），它进 [`RhymeGroupMatches::unresolved`] 而不是 [`RhymeGroupMatches::hits`]：
//! 「可能」既不是「是」，也不是「不是」。

use crate::rhyme::{RhymeBook, RhymeConfidence, RhymeTone};
use crate::{CorpusHandle, Error, Result};
use rusqlite::{Connection, OptionalExtension, params};

/// 按韵书与韵部取作品的 SQL。
///
/// `poem_rhyme_group_idx(rhyme_book, rhyme_group, poem_id)` 的前两列正是这里的等值约束，
/// 回表走 `poem` 的主键自动索引。声调写成 `?3 IS NULL OR ...` 而不是拼两条 SQL：它不在
/// 索引里，本就是取到候选之后的过滤，拼字符串只会多出一条要各自验证的执行路径。
pub const RHYME_GROUP_POEMS_SQL: &str = "SELECT g.poem_id, g.tone, g.confidence, p.title, p.author \
FROM poem_rhyme_group AS g \
JOIN poem AS p ON p.stable_id = g.poem_id \
WHERE g.rhyme_book = ?1 AND g.rhyme_group = ?2 AND (?3 IS NULL OR g.tone = ?3) \
ORDER BY g.poem_id, g.tone";

/// 取某个字在某本韵书里的全部（韵部, 声调）的 SQL。命中 `rhyme_character_idx`。
pub const CHARACTER_RHYME_GROUPS_SQL: &str = "SELECT rhyme_group, tone FROM rhyme \
WHERE rhyme_book = ?1 AND character = ?2 \
ORDER BY rhyme_group, tone";

/// 某本韵书里是否存在某个韵部。命中 `rhyme` 的主键前缀 `(rhyme_book, rhyme_group)`。
const RHYME_GROUP_EXISTS_SQL: &str =
    "SELECT 1 FROM rhyme WHERE rhyme_book = ?1 AND rhyme_group = ?2 LIMIT 1";

/// 平水韵的五个声部前缀。
///
/// 用户会把「下平七阳」整串输进来，而韵书自身的分组键是「七阳」、声调另存一列。前缀因此
/// 要剥掉——这是解析用户输入，不是推断声调：声调维度始终由 `tone` 参数承载。
/// 五个前缀直接取自韵书的嵌套键，所以剥离是有据的，不是猜的。
const TONE_SECTION_PREFIXES: [&str; 5] = ["上平", "下平", "上声", "去声", "入声"];

/// 声调筛选。
///
/// 写成枚举而不是 `Option<RhymeTone>`：`None` 在筛选语境里既能读成「不限」也能读成
/// 「未知声调」，而两者在格律上不是一回事。[`Self::Any`] 只表达前者。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToneFilter {
    /// 不按声调筛选。
    Any,
    /// 只取该声。
    Only(RhymeTone),
}

impl ToneFilter {
    fn bind_key(self) -> Option<&'static str> {
        match self {
            Self::Any => None,
            Self::Only(tone) => Some(tone.as_key()),
        }
    }
}

/// 一个（韵部, 声调）二元组。
///
/// 声调是键的一部分而不是附注：词林正韵的「第一部」同时有平声与仄声，平仄不同则不相押，
/// 所以只比韵部会把不押韵的字对报成押韵。
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct RhymeGroupRef {
    pub rhyme_group: String,
    pub tone: RhymeTone,
}

/// 韵部检索命中的一首作品。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RhymeGroupHit {
    pub poem_id: String,
    pub title: String,
    pub author: String,
    pub rhyme_group: String,
    pub tone: RhymeTone,
    /// 该归属的可信度，透出给 UI 区分「投票解出的」与「本就无歧义的」。
    pub confidence: RhymeConfidence,
}

/// 韵部检索的结果。
///
/// 两个列表刻意分开：[`Self::hits`] 是肯定判断，[`Self::unresolved`] 是保留下来的候选。
/// 把后者丢掉会让「不确定」变成「不存在」，把它并入前者会让猜测变成判断。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RhymeGroupMatches {
    pub book: RhymeBook,
    /// 实际用于匹配的韵部名（已剥掉用户输入里的声部前缀）。
    pub rhyme_group: String,
    /// [`RhymeConfidence::is_positive_claim`] 为真的命中。
    pub hits: Vec<RhymeGroupHit>,
    /// 韵脚未能唯一消歧、因此不计入命中的候选作品。
    pub unresolved: Vec<RhymeGroupHit>,
}

/// 一个字在某本韵书里的全部归属。`groups` 为空表示该书未收此字。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CharacterRhymes {
    pub character: char,
    pub groups: Vec<RhymeGroupRef>,
}

/// 押韵判断的三种结局。
///
/// 三个变体而不是一个 `bool`：`false` 会把「书里没这个字」和「同书不同韵部」压成同一个
/// 答案，而前者根本不构成判断。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RhymeVerdict {
    /// 全部字共享至少一个（韵部, 声调）。
    Rhyme,
    /// 全部字都在这本韵书里，但没有共享的（韵部, 声调）。这是真正的否定判断。
    NoRhyme,
    /// 有字不在这本韵书里，无从肯定也无从否定。
    Indeterminate,
}

/// [`do_these_rhyme`] 的完整回答，附带得出结论的依据。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RhymeAnswer {
    pub book: RhymeBook,
    /// 逐字归属，保留调用方传入的顺序。
    pub characters: Vec<CharacterRhymes>,
    /// 全部字共享的（韵部, 声调）。非空即押韵。
    pub shared: Vec<RhymeGroupRef>,
    /// 同部但异声的韵部。**不构成押韵**，列出来是为了让否定答案能解释自己。
    pub same_group_other_tone: Vec<String>,
    /// 这本韵书未收的字。非空时结论只能是 [`RhymeVerdict::Indeterminate`]。
    pub not_in_book: Vec<char>,
}

impl RhymeAnswer {
    /// 结论。
    #[must_use]
    pub fn verdict(&self) -> RhymeVerdict {
        if !self.not_in_book.is_empty() {
            RhymeVerdict::Indeterminate
        } else if self.shared.is_empty() {
            RhymeVerdict::NoRhyme
        } else {
            RhymeVerdict::Rhyme
        }
    }
}

/// 取某本韵书某个韵部下的作品。
///
/// `rhyme_group` 接受「七阳」与「下平七阳」两种写法。韵部在该书中不存在时报错而不是回空：
/// 空结果集会被读成「这个韵部没有作品」，而实情是韵部名压根不属于这本书——那正是拿词的
/// 韵部去查平水韵时会发生的事。
pub fn find_by_rhyme_group(
    handle: &CorpusHandle,
    book: RhymeBook,
    rhyme_group: &str,
    tone: ToneFilter,
) -> Result<RhymeGroupMatches> {
    book.ensure_available()?;
    let group = canonical_group_name(rhyme_group);
    if group.is_empty() {
        return Err(search_error("韵部名不能为空"));
    }
    let connection = handle.connect()?;
    if !group_exists(&connection, book, &group)? {
        return Err(unknown_group_error(&connection, book, &group)?);
    }

    let mut statement = connection.prepare_cached(RHYME_GROUP_POEMS_SQL)?;
    let rows = statement
        .query_map(params![book.as_key(), &group, tone.bind_key()], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    let mut matches = RhymeGroupMatches {
        book,
        rhyme_group: group.clone(),
        hits: Vec::new(),
        unresolved: Vec::new(),
    };
    for (poem_id, tone_key, confidence_key, title, author) in rows {
        let hit = RhymeGroupHit {
            poem_id,
            title,
            author,
            rhyme_group: group.clone(),
            tone: parse_tone(&tone_key)?,
            confidence: parse_confidence(&confidence_key)?,
        };
        if hit.confidence.is_positive_claim() {
            matches.hits.push(hit);
        } else {
            matches.unresolved.push(hit);
        }
    }
    Ok(matches)
}

/// 判断若干字在某本韵书里是否相押。
///
/// 判据是韵书归属，**不是**读音相似度：决定古典诗押韵的是这个字被哪本韵书编进哪个韵部，
/// 现代普通话读音在这里没有发言权。
pub fn do_these_rhyme(
    handle: &CorpusHandle,
    characters: &[char],
    book: RhymeBook,
) -> Result<RhymeAnswer> {
    book.ensure_available()?;
    if characters.len() < 2 {
        return Err(search_error(format!(
            "押韵判断至少需要两个字，实际收到 {} 个",
            characters.len()
        )));
    }
    let connection = handle.connect()?;
    let mut per_character = Vec::with_capacity(characters.len());
    for character in characters {
        per_character.push(CharacterRhymes {
            character: *character,
            groups: character_groups(&connection, book, *character)?,
        });
    }

    let not_in_book = per_character
        .iter()
        .filter(|entry| entry.groups.is_empty())
        .map(|entry| entry.character)
        .collect::<Vec<_>>();
    let (shared, same_group_other_tone) = if not_in_book.is_empty() {
        let shared: Vec<RhymeGroupRef> = intersect(&per_character, Clone::clone);
        // 忽略声调再求一次交集，减去已经押上的韵部，剩下的就是「同部异声」。
        let tone_blind: Vec<String> = intersect(&per_character, |group| group.rhyme_group.clone());
        let same_group_other_tone = tone_blind
            .into_iter()
            .filter(|group| shared.iter().all(|hit| &hit.rhyme_group != group))
            .collect();
        (shared, same_group_other_tone)
    } else {
        (Vec::new(), Vec::new())
    };

    Ok(RhymeAnswer {
        book,
        characters: per_character,
        shared,
        same_group_other_tone,
        not_in_book,
    })
}

/// 取某个字在某本韵书里的全部（韵部, 声调）。
///
/// 返回空表示这本书未收此字——与「书本身没随包」是两件事，后者在
/// [`RhymeBook::ensure_available`] 处就已变成错误。
pub fn rhyme_groups_of(
    handle: &CorpusHandle,
    character: char,
    book: RhymeBook,
) -> Result<Vec<RhymeGroupRef>> {
    book.ensure_available()?;
    let connection = handle.connect()?;
    character_groups(&connection, book, character)
}

fn character_groups(
    connection: &Connection,
    book: RhymeBook,
    character: char,
) -> Result<Vec<RhymeGroupRef>> {
    let mut statement = connection.prepare_cached(CHARACTER_RHYME_GROUPS_SQL)?;
    let rows = statement
        .query_map(params![book.as_key(), character.to_string()], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    rows.into_iter()
        .map(|(rhyme_group, tone_key)| {
            Ok(RhymeGroupRef {
                rhyme_group,
                tone: parse_tone(&tone_key)?,
            })
        })
        .collect()
}

/// 取全部字共享的键。`key` 决定共享的粒度：完整二元组，或忽略声调。
fn intersect<K, F>(per_character: &[CharacterRhymes], key: F) -> Vec<K>
where
    K: Ord + Clone,
    F: Fn(&RhymeGroupRef) -> K,
{
    let mut shared: Option<std::collections::BTreeSet<K>> = None;
    for entry in per_character {
        let current = entry.groups.iter().map(&key).collect();
        shared = Some(match shared {
            None => current,
            Some(previous) => previous.intersection(&current).cloned().collect(),
        });
    }
    shared
        .map(|set| set.into_iter().collect())
        .unwrap_or_default()
}

fn group_exists(connection: &Connection, book: RhymeBook, group: &str) -> Result<bool> {
    let found = connection
        .prepare_cached(RHYME_GROUP_EXISTS_SQL)?
        .query_row(params![book.as_key(), group], |_| Ok(()))
        .optional()?;
    Ok(found.is_some())
}

/// 韵部不属于这本书时的错误。若它属于另一本随包韵书，就把这件事说出来。
///
/// 这条信息是「书选错了」而不是「查无结果」：拿词林正韵的「第一部」去问平水韵，得到空集
/// 会被当成「平水韵里这个韵部没有词」，而正确的回答是平水韵为诗韵、根本没有这个韵部。
fn unknown_group_error(connection: &Connection, book: RhymeBook, group: &str) -> Result<Error> {
    let mut elsewhere = Vec::new();
    for other in RhymeBook::SHIPPED {
        if other != book && group_exists(connection, other, group)? {
            elsewhere.push(other.display_name());
        }
    }
    let hint = if elsewhere.is_empty() {
        String::new()
    } else {
        format!(
            "；该韵部属于{}，两本韵书的韵部命名不通用",
            elsewhere.join("、")
        )
    };
    Ok(search_error(format!(
        "{}没有韵部「{group}」{hint}",
        book.display_name()
    )))
}

/// 剥掉用户输入里的声部前缀，得到韵书自身的分组键。
fn canonical_group_name(rhyme_group: &str) -> String {
    let trimmed = rhyme_group.trim();
    for prefix in TONE_SECTION_PREFIXES {
        if let Some(rest) = trimmed.strip_prefix(prefix)
            && !rest.is_empty()
        {
            return rest.to_owned();
        }
    }
    trimmed.to_owned()
}

fn parse_tone(key: &str) -> Result<RhymeTone> {
    RhymeTone::from_key(key).ok_or_else(|| {
        search_error(format!(
            "韵书数据里出现未登记的声调键 `{key}`；声调不可推测"
        ))
    })
}

fn parse_confidence(key: &str) -> Result<RhymeConfidence> {
    RhymeConfidence::from_key(key).ok_or_else(|| {
        search_error(format!(
            "poem_rhyme_group.confidence 出现未登记取值 `{key}`；可信度不可推测"
        ))
    })
}

fn search_error(message: impl Into<String>) -> Error {
    Error::Search(message.into())
}

#[cfg(test)]
mod tests;
