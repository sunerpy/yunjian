//! `--json` 的稳定输出信封。
//!
//! # 为什么要有信封
//!
//! `--json` 的消费方是脚本与 AI 客户端，它们必须能在**不解析中文文案**的前提下判断
//! 三件事：这次调用成功了吗、结果是空的还是出错了、有没有需要转达给人的降级警告。
//! 把这三件事编码成固定的顶层字段，就是这个信封的全部用途。中文只出现在 `message`
//! 里，供人阅读；机器判断一律看 `status`、`code` 与进程退出码。
//!
//! # 契约
//!
//! stdout 上**恰好一行** JSON，形如：
//!
//! ```json
//! {"schema_version":1,"command":"search","status":"ok","warnings":[],"data":{}}
//! ```
//!
//! | 字段             | 出现时机           | 说明                                            |
//! | ---------------- | ------------------ | ----------------------------------------------- |
//! | `schema_version` | 总是               | [`SCHEMA_VERSION`]；不兼容变更才递增            |
//! | `command`        | 总是               | 子命令的稳定 ASCII 名，如 `search`、`corpus.fetch` |
//! | `status`         | 总是               | `ok` / `empty` / `error`                        |
//! | `warnings`       | 总是（可为空数组） | 降级与退化提示，每条带稳定 `code`               |
//! | `data`           | `status != error`  | 各子命令自有的载荷                              |
//! | `error`          | `status == error`  | 稳定 `code` + 中文 `message` + 可执行 `hint`    |
//!
//! `status` 与退出码一一对应：`ok` → 0，`empty` → 1，`error` → 2 或 3（按
//! [`crate::exit`] 分类）。因此只看退出码就够用，`status` 是给已经拿到 JSON 的调用方
//! 省一次分支。
//!
//! # 兼容性
//!
//! 新增字段不递增 [`SCHEMA_VERSION`]，删除或改变既有字段的含义才递增。新增 `warnings`
//! 或 `error` 的 `code` 取值同样不递增：调用方对未知 code 的正确反应是原样转达
//! `message`，而不是崩溃。

use serde::Serialize;
use serde_json::Value;

/// 信封的 schema 版本。
pub const SCHEMA_VERSION: u32 = 1;

/// 一次调用的三种结局。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    /// 成功且有结果。
    Ok,
    /// 成功执行但结果为空。**不是错误**，退出码 1 用于让 shell 能直接分支。
    Empty,
    /// 未能完成。
    Error,
}

/// 警告的稳定标识。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WarningCode {
    /// 首启派生结构不可用，两字查询退化为全表扫描。
    DerivedUnavailable,
    /// 本次查询走了无索引约束的路径。
    DegradedPlan,
    /// 本页命中被 `--author` / `--dynasty` 过滤后为空，但还有后续页。
    FilteredPageEmpty,
    /// 请求的韵书未随包分发，相关标注为空而不是「没有韵部」。
    RhymeBookUnavailable,
    /// 请求的作用域对该客户端不适用，已按它唯一支持的作用域处理。
    ClientScopeIgnored,
}

/// 一条面向用户的警告。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Warning {
    /// 稳定标识，供机器分支。
    pub code: WarningCode,
    /// 中文说明，供人阅读。
    pub message: String,
}

impl Warning {
    /// 组一条警告。
    #[must_use]
    pub fn new(code: WarningCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

/// 错误的稳定标识。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    /// 参数或请求本身不成立。
    Usage,
    /// 请求了未随包分发的韵书。独立成一档，因为它不是「查过了没有」。
    RhymeBookUnavailable,
    /// 语料库缺失、损坏或无法打开。
    CorpusUnavailable,
    /// 子命令尚未实现。
    NotImplemented,
    /// 目标客户端配置解析失败或结构不符，已拒绝写入。
    ClientConfigInvalid,
    /// 目标客户端配置写入失败。
    ClientConfigWriteFailed,
}

/// 一次失败的完整描述。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Failure {
    /// 稳定标识，供机器分支。
    pub code: ErrorCode,
    /// 中文原因。
    pub message: String,
    /// 下一步该做什么；没有可执行建议时为 `None`，绝不填一句空话。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
}

impl Failure {
    /// 组一条不带建议的失败。
    #[must_use]
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            hint: None,
        }
    }

    /// 附上可执行的下一步。
    #[must_use]
    pub fn with_hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }

    /// 人类可读的单行渲染，用于 stderr 日志。
    #[must_use]
    pub fn render(&self) -> String {
        match &self.hint {
            Some(hint) => format!("{}；{hint}", self.message),
            None => self.message.clone(),
        }
    }
}

/// `--json` 写到 stdout 的那一行。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Envelope {
    /// 见 [`SCHEMA_VERSION`]。
    pub schema_version: u32,
    /// 子命令的稳定 ASCII 名。
    pub command: &'static str,
    /// 本次调用的结局。
    pub status: Status,
    /// 降级与退化提示；无警告时是空数组而不是省略，省略会让调用方要写两套取值逻辑。
    pub warnings: Vec<Warning>,
    /// 子命令载荷。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
    /// 失败详情。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<Failure>,
}

impl Envelope {
    /// 成功且有结果。
    #[must_use]
    pub fn ok(command: &'static str, data: Value) -> Self {
        Self::payload(command, Status::Ok, data)
    }

    /// 成功但结果为空。
    #[must_use]
    pub fn empty(command: &'static str, data: Value) -> Self {
        Self::payload(command, Status::Empty, data)
    }

    /// 失败。
    #[must_use]
    pub fn failed(command: &'static str, failure: Failure) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            command,
            status: Status::Error,
            warnings: Vec::new(),
            data: None,
            error: Some(failure),
        }
    }

    /// 挂上警告列表。
    #[must_use]
    pub fn with_warnings(mut self, warnings: Vec<Warning>) -> Self {
        self.warnings = warnings;
        self
    }

    /// 序列化成写往 stdout 的那一行。
    ///
    /// 单行而不是 pretty：`--json` 的输出要能直接进 JSON Lines 管道，而 `jq` 两种都读。
    /// 序列化理论上不会失败（载荷里没有非字符串键，也没有 `NaN`），但这里不 `unwrap`
    /// ——一次 CLI 输出不值得用 panic 换取一行代码。
    #[must_use]
    pub fn to_json_line(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|error| {
            let message = format!("信封序列化失败：{error}");
            let fallback = Self::failed(
                self.command,
                Failure::new(ErrorCode::Usage, message).with_hint("请把该输出连同命令一并反馈"),
            );
            serde_json::to_string(&fallback)
                .unwrap_or_else(|_| String::from(r#"{"schema_version":1,"status":"error"}"#))
        })
    }

    fn payload(command: &'static str, status: Status, data: Value) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            command,
            status,
            warnings: Vec::new(),
            data: Some(data),
            error: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Envelope, ErrorCode, Failure, Status, Warning, WarningCode};
    use serde_json::{Value, json};

    fn parse(line: &str) -> Value {
        serde_json::from_str(line).expect("信封必须是合法 JSON")
    }

    #[test]
    fn success_envelope_carries_the_four_always_present_keys() {
        let line = Envelope::ok("search", json!({"hits": []})).to_json_line();
        let value = parse(&line);
        let object = value.as_object().expect("信封是 JSON 对象");
        for key in ["schema_version", "command", "status", "warnings", "data"] {
            assert!(object.contains_key(key), "缺少字段 {key}：{line}");
        }
        assert_eq!(object["schema_version"], json!(1));
        assert_eq!(object["status"], json!("ok"));
        // 空警告必须是 `[]` 而不是省略：否则调用方要为「没有 warnings 键」写第二条分支。
        assert_eq!(object["warnings"], json!([]));
        assert!(
            !object.contains_key("error"),
            "成功信封不得带 error：{line}"
        );
    }

    #[test]
    fn empty_status_is_not_an_error_and_keeps_its_payload() {
        let line = Envelope::empty("search", json!({"hits": []})).to_json_line();
        let value = parse(&line);
        assert_eq!(value["status"], json!("empty"));
        assert!(value.get("data").is_some(), "空结果仍要带 data：{line}");
        assert!(value.get("error").is_none(), "空结果不是错误：{line}");
    }

    #[test]
    fn failure_envelope_omits_data_and_keeps_a_stable_code() {
        let failure = Failure::new(ErrorCode::CorpusUnavailable, "语料库不存在")
            .with_hint("运行 `yunjian corpus fetch`");
        let line = Envelope::failed("search", failure).to_json_line();
        let value = parse(&line);
        assert_eq!(value["status"], json!("error"));
        assert_eq!(value["error"]["code"], json!("corpus_unavailable"));
        assert_eq!(value["error"]["hint"], json!("运行 `yunjian corpus fetch`"));
        assert!(value.get("data").is_none(), "失败信封不得带 data：{line}");
    }

    #[test]
    fn hint_is_omitted_rather_than_null_when_absent() {
        let line = Envelope::failed("mcp", Failure::new(ErrorCode::NotImplemented, "尚未实现"))
            .to_json_line();
        let value = parse(&line);
        assert!(
            value["error"].get("hint").is_none(),
            "没有建议时应省略 hint 而不是写 null：{line}"
        );
    }

    #[test]
    fn every_status_warning_and_error_code_serializes_to_snake_case_ascii() {
        for status in [Status::Ok, Status::Empty, Status::Error] {
            let rendered = serde_json::to_string(&status).expect("序列化 status");
            assert!(
                rendered.trim_matches('"').chars().all(is_snake_ascii),
                "status 取值必须是 snake_case ASCII：{rendered}"
            );
        }
        for code in [
            WarningCode::DerivedUnavailable,
            WarningCode::DegradedPlan,
            WarningCode::FilteredPageEmpty,
            WarningCode::RhymeBookUnavailable,
            WarningCode::ClientScopeIgnored,
        ] {
            let rendered = serde_json::to_string(&code).expect("序列化 warning code");
            assert!(
                rendered.trim_matches('"').chars().all(is_snake_ascii),
                "warning code 必须是 snake_case ASCII：{rendered}"
            );
        }
        for code in [
            ErrorCode::Usage,
            ErrorCode::RhymeBookUnavailable,
            ErrorCode::CorpusUnavailable,
            ErrorCode::NotImplemented,
            ErrorCode::ClientConfigInvalid,
            ErrorCode::ClientConfigWriteFailed,
        ] {
            let rendered = serde_json::to_string(&code).expect("序列化 error code");
            assert!(
                rendered.trim_matches('"').chars().all(is_snake_ascii),
                "error code 必须是 snake_case ASCII：{rendered}"
            );
        }
    }

    #[test]
    fn warnings_survive_serialization_with_code_and_message() {
        let envelope = Envelope::ok("search", json!({})).with_warnings(vec![Warning::new(
            WarningCode::DegradedPlan,
            "索引无法约束本次查询",
        )]);
        let value = parse(&envelope.to_json_line());
        assert_eq!(value["warnings"][0]["code"], json!("degraded_plan"));
        assert_eq!(
            value["warnings"][0]["message"],
            json!("索引无法约束本次查询")
        );
    }

    #[test]
    fn failure_renders_message_and_hint_for_stderr() {
        let failure = Failure::new(ErrorCode::Usage, "游标无效").with_hint("去掉 --cursor 重试");
        assert_eq!(failure.render(), "游标无效；去掉 --cursor 重试");
        assert_eq!(
            Failure::new(ErrorCode::Usage, "游标无效").render(),
            "游标无效"
        );
    }

    fn is_snake_ascii(character: char) -> bool {
        character.is_ascii_lowercase() || character == '_'
    }
}
