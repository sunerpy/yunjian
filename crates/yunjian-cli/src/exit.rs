//! 退出码与错误分类。
//!
//! # 四个码，界限清楚
//!
//! | 码 | 含义       | 触发条件                                                             |
//! | -- | ---------- | -------------------------------------------------------------------- |
//! | 0  | 成功       | 命令执行完成且有结果                                                 |
//! | 1  | 无结果     | 命令执行完成但结果集为空（含 `show` 指到不存在的作品）                |
//! | 2  | 用法错误   | 参数解析失败、请求本身不成立、请求了未随包的韵书                     |
//! | 3  | 数据不可用 | 语料库缺失、损坏、schema 不兼容、底层 I/O 与数据库故障，或语音模型未就位 |
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
use yunjian_voice::models::ModelError;

/// 取语料失败时的下一步。四个退出码里只有 3 会带它。
pub const FETCH_HINT: &str = "运行 `yunjian corpus fetch` 获取或修复语料库";

/// 取模型失败时的下一步。**刻意与 [`FETCH_HINT`] 不同**：把用户指去取语料是错的引导。
pub const MODEL_FETCH_HINT: &str = "联网后运行 `yunjian models fetch <模型名>` 下载并校验";

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
        // 密钥没配是配置问题，用户自己就能改；建议去取语料是误导，所以是 2 而不是 3。
        // 密钥只存操作系统钥匙串，因此下一步指向钥匙串而不是 `config.toml`。
        Error::AiKeyNotConfigured { .. } => (
            Exit::Usage,
            Failure::new(ErrorCode::Usage, error.to_string())
                .with_hint("把该供应商的 API key 存入操作系统钥匙串后重试；密钥不进 `config.toml`"),
        ),
        // 预生成门禁只在 `xtask pregenerate` 这条构建期路径上触发，用户的 CLI 走不到它。
        // 但它是配置错误而不是语料缺失——归到 3 会让用户被引导去下载语料，而真正要改的是
        // 生成配置或披露文件。所以是 2，且下一步指向那两处。
        Error::PregenerationRejected(_) | Error::PregenerationClosedProvider { .. } => (
            Exit::Usage,
            Failure::new(ErrorCode::Usage, error.to_string()).with_hint(
                "随包赏析数据集只能由开放权重模型生成：检查 `xtask pregenerate` 的 \
                 `--model-license`（只认 MIT 与 Apache-2.0）、`--provider`（须是本地运行时）\
                 与 `dataset/README.md` 的披露段",
            ),
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

/// 把模型侧的失败翻译成退出码与信封里的失败描述。
///
/// 与 [`describe`] 分开而不是塞进 [`Error`]：模型错误不经过 `yunjian_core::Error`，
/// 把它硬转成 `Error::Voice(String)` 会把「命中拒绝名单」与「磁盘满」压成同一个字符串，
/// 于是退出码只能一律归 3——而**被许可门禁拒绝是用法错误**，重试永远不会成功。
///
/// `match` 刻意穷举：[`ModelError`] 新增变体时这里应当编译失败，逼出一次
/// 「它算用法错误还是数据不可用」的判断。
#[must_use]
pub fn describe_model(error: &ModelError) -> (Exit, Failure) {
    match error {
        // 名字打错了，是用法错误；但它不是「拒绝」，报错里已经列出可用的名字。
        ModelError::Unknown { .. } => (
            Exit::Usage,
            Failure::new(ErrorCode::Usage, error.to_string())
                .with_hint("运行 `yunjian models list` 看清单里实际有哪些模型"),
        ),
        // 拒绝名单与许可判定都不是「本机状态不对」，重试与下载都不会让它通过。
        ModelError::Denied { .. } | ModelError::LicenseRefused { .. } => (
            Exit::Usage,
            Failure::new(ErrorCode::ModelRefused, error.to_string())
                .with_hint("只接受 MIT 与 Apache-2.0 的权重；被拒条目与理由见 models/DENYLIST.md"),
        ),
        ModelError::Absent { .. }
        | ModelError::ChecksumMismatch { .. }
        | ModelError::SizeMismatch { .. }
        | ModelError::Download { .. }
        | ModelError::Unpack { .. }
        | ModelError::Io { .. }
        | ModelError::Manifest { .. } => (
            Exit::CorpusUnavailable,
            Failure::new(ErrorCode::ModelUnavailable, error.to_string())
                .with_hint(MODEL_FETCH_HINT),
        ),
    }
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
    fn a_missing_api_key_is_two_and_points_at_the_keychain_not_the_corpus() {
        let (exit, failure) = describe(&Error::AiKeyNotConfigured {
            provider: "openai".to_owned(),
        });
        assert_eq!(exit, Exit::Usage);
        assert_eq!(failure.code, ErrorCode::Usage);
        assert!(
            !failure.render().contains("corpus fetch"),
            "密钥没配与语料无关，不该建议去取语料：{}",
            failure.render()
        );
        assert!(
            failure.render().contains("钥匙串"),
            "必须指向密钥真正的存放处：{}",
            failure.render()
        );
    }

    #[test]
    fn corpus_failure_carries_the_hint_by_construction() {
        let failure = corpus_failure("语料库损坏");
        assert_eq!(failure.code, ErrorCode::CorpusUnavailable);
        assert!(failure.hint.is_some(), "退出 3 必须给出下一步");
    }
}

#[cfg(test)]
mod model_tests {
    use super::{Exit, describe_model};
    use crate::envelope::ErrorCode;
    use yunjian_voice::models::{ModelError, Registry};

    /// 被许可门禁拒绝是**用法错误**，不是「本机数据不对」。
    ///
    /// 这条边界值钱：判成 3 会让脚本以为「再取一次就好」，而拒绝名单命中永远不会通过。
    #[test]
    fn a_refused_license_is_two_and_never_suggests_downloading_again() {
        for error in [
            ModelError::Denied {
                name: "x".to_owned(),
                matched: "matcha-icefall-zh-baker".to_owned(),
                reason: "训练数据集非商用".to_owned(),
            },
            ModelError::LicenseRefused {
                name: "x".to_owned(),
                license: "GPL-3.0".to_owned(),
            },
        ] {
            let (exit, failure) = describe_model(&error);
            assert_eq!(exit, Exit::Usage, "{error}");
            assert_eq!(failure.code, ErrorCode::ModelRefused, "{error}");
            let hint = failure.hint.clone().expect("被拒也要给下一步");
            assert!(
                !hint.contains("models fetch"),
                "重试不会让它通过，不该建议再下载一次：{hint}"
            );
            assert!(hint.contains("DENYLIST"), "要指向拒绝名单以便核对：{hint}");
        }
    }

    #[test]
    fn a_missing_model_is_three_and_points_at_models_fetch_not_the_corpus() {
        let (exit, failure) = describe_model(&ModelError::Absent {
            name: "x".to_owned(),
            dir: std::path::PathBuf::from("/nope"),
            next: "下一步".to_owned(),
        });
        assert_eq!(exit, Exit::CorpusUnavailable);
        assert_eq!(failure.code, ErrorCode::ModelUnavailable);
        let rendered = failure.render();
        assert!(rendered.contains("models fetch"), "{rendered}");
        assert!(
            !rendered.contains("corpus fetch"),
            "模型的问题不该建议去取语料：{rendered}"
        );
    }

    #[test]
    fn an_unknown_name_is_two_and_sends_the_caller_to_the_list_command() {
        let error = Registry::shipped()
            .expect("清单可解析")
            .admit("no-such-model")
            .expect_err("未知名字");
        let (exit, failure) = describe_model(&error);
        assert_eq!(exit, Exit::Usage);
        assert_eq!(failure.code, ErrorCode::Usage);
        assert!(
            failure.hint.expect("要给下一步").contains("models list"),
            "名字打错该去看清单"
        );
    }
}
