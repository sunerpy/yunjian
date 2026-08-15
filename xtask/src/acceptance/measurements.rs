//! 把 Device Farm 设备侧回传的测量行解析成判据结果。
//!
//! # 为什么需要这一层
//!
//! 在此之前，`mobile::build_report` 把四条判据**硬编码**成 `NOT EXECUTED`：无论远端
//! 真机上发生了什么，报告都一个样。于是 harness 无法区分「没跑」与「跑了但测不到」，
//! 也无法在测量值齐备时给出 `PASS`。本模块补上那条通路，且只补通路——
//! **它不会在缺少测量值时替设备下结论**。
//!
//! # 设备侧协议
//!
//! `.aws/devicefarm/spike-measure.sh` 每测到一个值就打印一行：
//!
//! ```text
//! YUNJIAN-MEASURE <criterion_id> <key>=<value>
//! ```
//!
//! 测不到的项打印另一种前缀，并**必须**带上缺什么：
//!
//! ```text
//! YUNJIAN-MEASURE-UNAVAILABLE <criterion_id> <key> reason=<why>
//! ```
//!
//! 两种前缀刻意不同：如果不可用项复用同一前缀而把值写成空串，那么「没测到」与
//! 「测到的是空串」在解析后不可区分，而前者不该影响 verdict、后者应当判 FAIL。
//!
//! # 判据的 PASS 需要什么
//!
//! 一条判据只有在它**预声明的全部 `required_measurements` 都拿到非空实测值**，
//! 且阈值满足时才是 `PASS`。任一必需项落在 `UNAVAILABLE` 里，结果是 `NOT EXECUTED`
//! 而不是 `FAIL`——这是 `problems.md` 反复记录的那条：未执行不等于失败，把它记成
//! 失败会让 `uniffi_native` 被一个没测过的结论选中。

use std::collections::BTreeMap;

use serde_json::Value;

use super::Verdict;

const MEASURE_PREFIX: &str = "YUNJIAN-MEASURE ";
const UNAVAILABLE_PREFIX: &str = "YUNJIAN-MEASURE-UNAVAILABLE ";

/// 单条判据从设备侧收到的东西。
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct DeviceMeasurements {
    /// 实测到的键值对。
    pub(crate) values: BTreeMap<String, String>,
    /// 明确报告为测不到的键，以及设备给出的原因。
    pub(crate) unavailable: BTreeMap<String, String>,
}

impl DeviceMeasurements {
    fn record(&mut self, key: &str, value: &str) {
        self.values.insert(key.to_owned(), value.to_owned());
    }

    fn mark_unavailable(&mut self, key: &str, reason: &str) {
        self.unavailable.insert(key.to_owned(), reason.to_owned());
    }
}

/// 按判据 id 分组的全部设备侧测量。
pub(crate) type MeasurementsByCriterion = BTreeMap<String, DeviceMeasurements>;

/// 解析设备侧日志。无法识别的行一律忽略——设备日志里混着 adb、Gradle 与系统输出，
/// 对未知行报错只会让整份日志因为一行噪声而不可用。
pub(crate) fn parse_measurements(log: &str) -> MeasurementsByCriterion {
    let mut out = MeasurementsByCriterion::new();
    for line in log.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix(UNAVAILABLE_PREFIX) {
            let mut parts = rest.split_whitespace();
            let (Some(criterion), Some(key)) = (parts.next(), parts.next()) else {
                continue;
            };
            let reason = parts
                .next()
                .and_then(|token| token.strip_prefix("reason="))
                .unwrap_or("unspecified");
            out.entry(criterion.to_owned())
                .or_default()
                .mark_unavailable(key, reason);
        } else if let Some(rest) = line.strip_prefix(MEASURE_PREFIX) {
            let mut parts = rest.splitn(2, char::is_whitespace);
            let (Some(criterion), Some(pair)) = (parts.next(), parts.next()) else {
                continue;
            };
            let Some((key, value)) = pair.split_once('=') else {
                continue;
            };
            out.entry(criterion.to_owned())
                .or_default()
                .record(key.trim(), value.trim());
        }
    }
    out
}

/// 一条判据的裁决与它的依据。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CriterionOutcome {
    pub(crate) verdict: Verdict,
    pub(crate) detail: String,
    pub(crate) measurement: BTreeMap<&'static str, Value>,
}

/// 判据阈值。每条都只看它自己声明的键，不看别的判据的值。
type Threshold = fn(&BTreeMap<String, String>) -> Result<(), String>;

/// 依据设备侧测量裁决一条判据。
///
/// 三种结果的边界在这里，而不是散在调用方：
///
/// - 必需项**全部**拿到非空实测值且阈值满足 → `PASS`
/// - 必需项全部拿到但阈值不满足 → `FAIL`（这是产品失败，会强制选型 `uniffi_native`）
/// - 任一必需项缺失或被设备标为不可用 → `NOT EXECUTED`
pub(crate) fn judge(
    required: &[&'static str],
    measured: Option<&DeviceMeasurements>,
    threshold: Threshold,
) -> CriterionOutcome {
    let empty = DeviceMeasurements::default();
    let measured = measured.unwrap_or(&empty);

    let mut measurement = BTreeMap::new();
    for key in required {
        let value = measured
            .values
            .get(*key)
            .map_or(Value::Null, |raw| typed(raw));
        measurement.insert(*key, value);
    }

    let mut missing = Vec::new();
    for key in required {
        match measured.values.get(*key) {
            Some(value) if !value.trim().is_empty() => {}
            _ => {
                let reason = measured.unavailable.get(*key).map_or_else(
                    || "设备侧未回传".to_owned(),
                    |why| format!("设备侧报告 {why}"),
                );
                missing.push(format!("{key}（{reason}）"));
            }
        }
    }

    if !missing.is_empty() {
        return CriterionOutcome {
            verdict: Verdict::NotExecuted,
            detail: format!(
                "NOT EXECUTED：真机已回传部分测量值，但以下必需项仍缺：{}。缺项不记 FAIL——未执行不是产品失败",
                missing.join("；")
            ),
            measurement,
        };
    }

    match threshold(&measured.values) {
        Ok(()) => CriterionOutcome {
            verdict: Verdict::Pass,
            detail: "PASS：必需测量值齐备且满足预声明阈值".to_owned(),
            measurement,
        },
        Err(why) => CriterionOutcome {
            verdict: Verdict::Fail,
            detail: format!("FAIL：测量值齐备但未达阈值——{why}"),
            measurement,
        },
    }
}

/// 数字与布尔在 JSON 里保持原类型，其余按字符串。报告要能被机器消费，
/// 把 `16000` 写成 `"16000"` 会让下游断言不得不做二次解析。
fn typed(raw: &str) -> Value {
    if let Ok(flag) = raw.parse::<bool>() {
        return Value::Bool(flag);
    }
    if let Ok(number) = raw.parse::<i64>() {
        return Value::Number(number.into());
    }
    if let Ok(number) = raw.parse::<f64>()
        && let Some(number) = serde_json::Number::from_f64(number)
    {
        return Value::Number(number);
    }
    Value::String(raw.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r"
noise before
YUNJIAN-MEASURE microphone_capture device_model=Pixel 8
YUNJIAN-MEASURE microphone_capture sample_rate_hz=16000
YUNJIAN-MEASURE microphone_capture channel_count=1
YUNJIAN-MEASURE microphone_capture rms=0.0031
YUNJIAN-MEASURE-UNAVAILABLE corpus_materialization sha256_verified reason=needs_in_app_production_fetch
random adb output = not a measurement
";

    #[test]
    fn parses_values_and_unavailable_keys_into_separate_buckets() {
        let parsed = parse_measurements(SAMPLE);
        let mic = parsed
            .get("microphone_capture")
            .expect("应当解析出麦克风判据");
        assert_eq!(
            mic.values.get("sample_rate_hz").map(String::as_str),
            Some("16000")
        );
        assert_eq!(
            mic.values.get("device_model").map(String::as_str),
            Some("Pixel 8"),
            "带空格的值必须完整保留，不能在第一个空格处截断"
        );
        assert!(
            mic.unavailable.is_empty(),
            "实测项不应落进不可用桶：{:?}",
            mic.unavailable
        );
        let corpus = parsed
            .get("corpus_materialization")
            .expect("应当解析出语料判据");
        assert_eq!(
            corpus
                .unavailable
                .get("sha256_verified")
                .map(String::as_str),
            Some("needs_in_app_production_fetch"),
            "不可用项必须带上设备给出的原因"
        );
        assert!(corpus.values.is_empty());
    }

    #[test]
    fn unknown_lines_never_abort_the_parse() {
        let parsed = parse_measurements("完全无关的一行\nYUNJIAN-MEASURE a b=1\n又一行噪声");
        assert_eq!(parsed.len(), 1, "噪声行应被忽略而不是让整份日志失效");
    }

    fn always_ok(_: &BTreeMap<String, String>) -> Result<(), String> {
        Ok(())
    }

    fn always_bad(_: &BTreeMap<String, String>) -> Result<(), String> {
        Err("阈值故意不满足".to_owned())
    }

    #[test]
    fn a_missing_required_key_is_not_executed_not_failed() {
        let parsed = parse_measurements(SAMPLE);
        let outcome = judge(
            &["sample_rate_hz", "permission_plugin"],
            parsed.get("microphone_capture"),
            always_ok,
        );
        assert_eq!(
            outcome.verdict,
            Verdict::NotExecuted,
            "缺必需项必须是 NOT EXECUTED；记成 FAIL 会让选型被一个没测过的结论决定"
        );
        assert!(
            outcome.detail.contains("permission_plugin"),
            "必须点名缺哪一项：{}",
            outcome.detail
        );
        assert_eq!(
            outcome.measurement.get("permission_plugin"),
            Some(&Value::Null),
            "缺失项在报告里写 null，不能省略这个键"
        );
    }

    #[test]
    fn an_unavailable_key_carries_the_device_reason_into_the_detail() {
        let parsed = parse_measurements(SAMPLE);
        let outcome = judge(
            &["sha256_verified"],
            parsed.get("corpus_materialization"),
            always_ok,
        );
        assert_eq!(outcome.verdict, Verdict::NotExecuted);
        assert!(
            outcome.detail.contains("needs_in_app_production_fetch"),
            "不可用原因必须出现在报告细节里，否则读者不知道下一步做什么：{}",
            outcome.detail
        );
    }

    #[test]
    fn complete_measurements_pass_or_fail_purely_by_the_threshold() {
        let parsed = parse_measurements(SAMPLE);
        let keys = ["sample_rate_hz", "channel_count", "rms"];
        let pass = judge(&keys, parsed.get("microphone_capture"), always_ok);
        assert_eq!(pass.verdict, Verdict::Pass);
        let fail = judge(&keys, parsed.get("microphone_capture"), always_bad);
        assert_eq!(
            fail.verdict,
            Verdict::Fail,
            "测量值齐备而阈值不满足才是真的产品失败"
        );
        assert!(fail.detail.contains("阈值故意不满足"));
    }

    #[test]
    fn numbers_and_booleans_keep_their_json_types() {
        let parsed = parse_measurements(
            "YUNJIAN-MEASURE x count=42\nYUNJIAN-MEASURE x ratio=0.5\nYUNJIAN-MEASURE x flag=true\nYUNJIAN-MEASURE x name=Pixel 8",
        );
        let outcome = judge(
            &["count", "ratio", "flag", "name"],
            parsed.get("x"),
            always_ok,
        );
        assert_eq!(outcome.measurement.get("count"), Some(&Value::from(42)));
        assert_eq!(outcome.measurement.get("ratio"), Some(&Value::from(0.5)));
        assert_eq!(outcome.measurement.get("flag"), Some(&Value::Bool(true)));
        assert_eq!(
            outcome.measurement.get("name"),
            Some(&Value::from("Pixel 8"))
        );
    }

    #[test]
    fn no_measurements_at_all_is_still_not_executed() {
        let outcome = judge(&["anything"], None, always_ok);
        assert_eq!(outcome.verdict, Verdict::NotExecuted);
        assert!(outcome.detail.contains("设备侧未回传"));
    }
}
