//! 退出码与错误分类。
//!
//! # 四个码，界限清楚
//!
//! | 码 | 含义       | 触发条件                                                             |
//! | -- | ---------- | -------------------------------------------------------------------- |
//! | 0  | 成功       | 命令执行完成且有结果                                                 |
//! | 1  | 无结果     | 命令执行完成但结果集为空（含 `show` 指到不存在的作品）                |
//! | 2  | 用法错误   | 参数解析失败、请求本身不成立、请求了未随包的韵书                     |
//! | 3  | 语料不可用 | 语料库缺失、损坏、schema 不兼容，或读取语料时的底层 I/O 与数据库故障 |
//!
//! 1 与 3 的区别是产品语义上的：**1 说「我查过了，没有」，3 说「我没法查」**。把「语料
//! 缺失」压成 0 条结果是这条边界上最容易犯、也最贵的错——脚本会把它当成「诗库里没有
//! 李白」，而正确反应是去取语料。
//!
//! 2 与 3 的区别是「谁该改」：2 是调用方改命令，3 是调用方补语料。因此**未随包的韵书
//! 是 2 而不是 3**——`yunjian corpus fetch` 取不来中华新韵，那是一条许可判定，不是一份
//! 缺失的文件。
//!
//! 除用法类之外的失败一律归 3，包括理论上不该在本 CLI 出现的 AI / 语音 / 背诵错误。
//! 这不是偷懒：方案只给了四个码，而这些失败的共同点是「本机状态不对，命令本身没写错」，
//! 与 3 的语义一致。真正需要区分时，`--json` 的 `error.code` 比退出码精细。

use crate::envelope::{ErrorCode, Failure};
use yunjian_core::Error;

/// 取语料失败时的下一步。四个退出码里只有 3 会带它。
pub const FETCH_HINT: &str = "运行 `yunjian corpus fetch` 获取或修复语料库";

/// 进程退出码。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Exit {
    /// 0：成功且有结果。
    Success,
    /// 1：执行成功，结果为空。
    NoResults,
    /// 2：用法错误。与 `clap` 自身的解析失败退出码一致。
    Usage,
    /// 3：语料不可用。
    CorpusUnavailable,
}

impl Exit {
    /// 交给 `std::process::exit` 的数值。
    #[must_use]
    pub const fn code(self) -> i32 {
        match self {
            Self::Success => 0,
            Self::NoResults => 1,
            Self::Usage => 2,
            Self::CorpusUnavailable => 3,
        }
    }
}

/// 把核心错误翻译成退出码与信封里的失败描述。
///
/// `match` 刻意穷举而不写 `_ =>`：[`Error`] 将来新增变体时这里应当编译失败，逼出一次
/// 「它算用法错误还是语料不可用」的判断，而不是被通配分支静默归到 3。
#[must_use]
pub fn describe(error: &Error) -> (Exit, Failure) {
    match error {
        Error::RhymeBookUnavailable { .. } => (
            Exit::Usage,
            Failure::new(ErrorCode::RhymeBookUnavailable, error.to_string())
                .with_hint("改用随包的 `pingshui`（诗）或 `cilin`（词）"),
        ),
        Error::Search(_) | Error::Config(_) => (
            Exit::Usage,
            Failure::new(ErrorCode::Usage, error.to_string()),
        ),
        Error::Corpus(_)
        | Error::CommentaryCitationMissing { .. }
        | Error::Io(_)
        | Error::Db(_)
        | Error::Ai(_)
        | Error::Voice(_)
        | Error::Recite(_) => (Exit::CorpusUnavailable, corpus_failure(error.to_string())),
    }
}

/// 组一条「语料不可用」的失败，附带取语料的下一步。
#[must_use]
pub fn corpus_failure(message: impl Into<String>) -> Failure {
    Failure::new(ErrorCode::CorpusUnavailable, message).with_hint(FETCH_HINT)
}

#[cfg(test)]
mod tests {
    use super::{Exit, corpus_failure, describe};
    use crate::envelope::ErrorCode;
    use yunjian_core::{Error, RhymeBook};

    #[test]
    fn the_four_codes_are_exactly_zero_one_two_three() {
        assert_eq!(Exit::Success.code(), 0);
        assert_eq!(Exit::NoResults.code(), 1);
        assert_eq!(Exit::Usage.code(), 2);
        assert_eq!(Exit::CorpusUnavailable.code(), 3);
    }

    #[test]
    fn a_missing_corpus_is_three_and_always_names_the_fetch_command() {
        let (exit, failure) = describe(&Error::Corpus("语料库 /nonexistent 不存在".to_owned()));
        assert_eq!(exit, Exit::CorpusUnavailable);
        assert_eq!(failure.code, ErrorCode::CorpusUnavailable);
        assert!(
            failure.render().contains("corpus fetch"),
            "退出 3 的文案必须点名 `yunjian corpus fetch`：{}",
            failure.render()
        );
    }

    #[test]
    fn an_unshipped_rhyme_book_is_a_usage_error_not_a_missing_corpus() {
        let error = RhymeBook::Xinyun
            .ensure_available()
            .expect_err("中华新韵未随包");
        let (exit, failure) = describe(&error);
        // 取语料取不来一条许可判定，所以这里绝不能是 3。
        assert_eq!(exit, Exit::Usage);
        assert_eq!(failure.code, ErrorCode::RhymeBookUnavailable);
        assert!(
            !failure.render().contains("corpus fetch"),
            "许可判定不该建议去取语料：{}",
            failure.render()
        );
    }

    #[test]
    fn a_bad_request_is_two_and_a_data_defect_is_three() {
        assert_eq!(
            describe(&Error::Search("正文检索 limit 必须大于 0".to_owned())).0,
            Exit::Usage
        );
        assert_eq!(
            describe(&Error::Config("解析 TOML 失败".to_owned())).0,
            Exit::Usage
        );
        assert_eq!(
            describe(&Error::CommentaryCitationMissing {
                commentary_id: "c-1".to_owned(),
                poem_id: "p-1".to_owned(),
                missing_field: "citation_work",
            })
            .0,
            Exit::CorpusUnavailable
        );
        assert_eq!(
            describe(&Error::Io(std::io::Error::other("磁盘错误"))).0,
            Exit::CorpusUnavailable
        );
    }

    #[test]
    fn corpus_failure_carries_the_hint_by_construction() {
        let failure = corpus_failure("语料库损坏");
        assert_eq!(failure.code, ErrorCode::CorpusUnavailable);
        assert!(failure.hint.is_some(), "退出 3 必须给出下一步");
    }
}
