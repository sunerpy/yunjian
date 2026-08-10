//! 韵书维度的共享词汇。
//!
//! 这个模块放在 `yunjian-core` 而不是 `yunjian-corpus`，理由是依赖方向：构建期的
//! 韵书导入在 `yunjian-corpus`，而运行期的押韵检索在 `yunjian-core`，后者不能依赖
//! 前者。两侧必须对「有哪几本韵书」「哪本没随包」达成同一套判断，所以词汇只能落在
//! 被双方共享的这一层。
//!
//! # 为什么 [`RhymeBook::Xinyun`] 从第一天就存在
//!
//! 中华新韵是 2005 年的现代出版物，上游仓库的 MIT 许可覆盖不了它转录的内容，因此
//! 它的数据**不随包分发**。但枚举槽位现在就留着：将来授权链核实清楚，加它只是往
//! 数据库里多灌一批行，而不是一次涉及类型、schema 与查询签名的迁移。
//!
//! # 为什么缺书必须是错误而不是空集
//!
//! 「查不到」和「不押韵」在格律上是两回事。若对 [`RhymeBook::Xinyun`] 的查询返回空
//! 结果集，调用方无从区分「这两个字在中华新韵里不同韵部」与「我们根本没有中华新韵」，
//! 于是缺数据会被当成否定判断呈现给用户——那是对格律的虚假陈述。所以查询一本未随包
//! 的韵书返回类型化的 [`Error::RhymeBookUnavailable`](crate::Error::RhymeBookUnavailable)，
//! 调用方必须显式处理，无法误当成「不押韵」。

use serde::{Deserialize, Serialize};

/// 云笺认识的三本韵书。
///
/// 韵书是**必填维度**而不是可选参数：平水韵是诗韵，对词牌格律并不适用，所以
/// 「这两个字押韵吗」在不指定韵书时没有答案。检索接口一律要求传入本枚举，
/// 于是「不小心混用两本书」在类型层面就不可表达。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RhymeBook {
    /// 平水韵。前现代韵书，公有领域。用于诗。
    Pingshui,
    /// 词林正韵（清 戈载，1821）。前现代韵书，公有领域。用于词。
    Cilin,
    /// 中华新韵（2005）。现代出版物，授权链未核实，**不随包分发**。
    Xinyun,
}

impl RhymeBook {
    /// 三本韵书的全集，含未随包的那本。
    ///
    /// 遍历时包含 [`Self::Xinyun`] 是刻意的：任何「对每本韵书都要做点什么」的代码
    /// 都会被迫面对它不可用这件事，而不是因为它不在列表里就悄悄漏掉。
    pub const ALL: [Self; 3] = [Self::Pingshui, Self::Cilin, Self::Xinyun];

    /// 随包分发的韵书。产品实际发两本：平水韵用于诗，词林正韵用于词。
    pub const SHIPPED: [Self; 2] = [Self::Pingshui, Self::Cilin];

    /// 写入数据库与配置的稳定键。
    pub const fn as_key(self) -> &'static str {
        match self {
            Self::Pingshui => "pingshui",
            Self::Cilin => "cilin",
            Self::Xinyun => "xinyun",
        }
    }

    /// 中文书名，用于面向用户的文案。
    pub const fn display_name(self) -> &'static str {
        match self {
            Self::Pingshui => "平水韵",
            Self::Cilin => "词林正韵",
            Self::Xinyun => "中华新韵",
        }
    }

    /// 该韵书是否随包分发。
    pub const fn is_shipped(self) -> bool {
        self.unavailable_reason().is_none()
    }

    /// 未随包的原因；随包的返回 `None`。
    ///
    /// 原因是常量而不是运行期查表：它是一条许可判定，不该随部署环境变化。
    pub const fn unavailable_reason(self) -> Option<&'static str> {
        match self {
            Self::Pingshui | Self::Cilin => None,
            Self::Xinyun => Some(
                "中华新韵是 2005 年的现代出版物，上游仓库的 MIT 许可无法为其内容授权；\
                 授权链核实前不随包分发",
            ),
        }
    }

    /// 解析稳定键。
    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|book| book.as_key() == key)
    }

    /// 该韵书可用则返回 `Ok(())`，否则返回类型化的不可用错误。
    ///
    /// 每个查询入口都应先过这道闸，让「缺书」在产生任何结果集之前就变成错误。
    pub fn ensure_available(self) -> crate::Result<()> {
        match self.unavailable_reason() {
            None => Ok(()),
            Some(reason) => Err(crate::Error::RhymeBookUnavailable { book: self, reason }),
        }
    }
}

/// 韵书里的声调维度。
///
/// 与 [`yunjian_corpus::ingest::Tone`] 的平仄二分**不是**一回事：那个描述某个字在
/// 某句诗里该用平还是仄，这个描述某个字在韵书里归属哪个声。两本书的声调粒度本身就
/// 不同——平水韵分到四声（且平声再分上下平两卷），词林正韵把上声去声并成「仄声」。
/// 这里不把词林正韵的仄声硬拆成上去：上游没有那个信息，拆就是编造。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RhymeTone {
    /// 平声。平水韵的上平声部与下平声部、词林正韵的平声。
    Level,
    /// 上声。仅平水韵。
    Rising,
    /// 去声。仅平水韵。
    Departing,
    /// 入声。两本书都有。
    Entering,
    /// 仄声。词林正韵把上去两声并列于此，上游未再细分，故不拆。
    Oblique,
}

impl RhymeTone {
    /// 写入数据库的稳定键。
    pub const fn as_key(self) -> &'static str {
        match self {
            Self::Level => "level",
            Self::Rising => "rising",
            Self::Departing => "departing",
            Self::Entering => "entering",
            Self::Oblique => "oblique",
        }
    }

    /// 中文声名。
    pub const fn display_name(self) -> &'static str {
        match self {
            Self::Level => "平",
            Self::Rising => "上",
            Self::Departing => "去",
            Self::Entering => "入",
            Self::Oblique => "仄",
        }
    }

    /// 该声在格律上算平还是仄。
    ///
    /// 平水韵四声可判；词林正韵的 [`Self::Oblique`] 本身就是仄。入声归仄。
    pub const fn is_level(self) -> bool {
        matches!(self, Self::Level)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Error;

    #[test]
    fn xinyun_slot_exists_but_is_not_shipped() {
        // 槽位在，所以将来放行它是数据变更而不是迁移。
        assert!(RhymeBook::ALL.contains(&RhymeBook::Xinyun));
        assert!(!RhymeBook::Xinyun.is_shipped());
        assert!(!RhymeBook::SHIPPED.contains(&RhymeBook::Xinyun));
    }

    #[test]
    fn shipped_set_is_exactly_the_two_public_domain_books() {
        assert_eq!(RhymeBook::SHIPPED, [RhymeBook::Pingshui, RhymeBook::Cilin]);
        for book in RhymeBook::SHIPPED {
            assert!(book.is_shipped(), "{} 应随包", book.display_name());
            assert!(book.unavailable_reason().is_none());
        }
    }

    /// 缺书必须是类型化错误，绝不能是「不押韵」。
    #[test]
    fn querying_a_withheld_book_is_a_typed_error_not_an_empty_answer() {
        let err = RhymeBook::Xinyun
            .ensure_available()
            .expect_err("中华新韵未随包，必须报错");
        assert!(matches!(
            err,
            Error::RhymeBookUnavailable {
                book: RhymeBook::Xinyun,
                ..
            }
        ));
        let rendered = format!("{err}");
        assert!(rendered.contains("中华新韵"), "{rendered}");
        // 文案必须说清是「没有这本书」，不能让人读成「查过了不押韵」。
        assert!(rendered.contains("未随包分发"), "{rendered}");
    }

    #[test]
    fn shipped_books_pass_the_availability_gate() {
        for book in RhymeBook::SHIPPED {
            book.ensure_available().expect("随包韵书应通过闸门");
        }
    }

    #[test]
    fn keys_round_trip_and_are_distinct() {
        let mut keys: Vec<&str> = RhymeBook::ALL.iter().map(|b| b.as_key()).collect();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), RhymeBook::ALL.len());
        for book in RhymeBook::ALL {
            assert_eq!(RhymeBook::from_key(book.as_key()), Some(book));
        }
        assert_eq!(RhymeBook::from_key("guangyun"), None);
    }

    #[test]
    fn tone_keys_are_distinct_and_only_level_is_level() {
        let tones = [
            RhymeTone::Level,
            RhymeTone::Rising,
            RhymeTone::Departing,
            RhymeTone::Entering,
            RhymeTone::Oblique,
        ];
        let mut keys: Vec<&str> = tones.iter().map(|t| t.as_key()).collect();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), tones.len());

        assert!(RhymeTone::Level.is_level());
        for tone in [
            RhymeTone::Rising,
            RhymeTone::Departing,
            RhymeTone::Entering,
            RhymeTone::Oblique,
        ] {
            assert!(!tone.is_level(), "{} 不应算平声", tone.display_name());
        }
    }
}
