//! 把真机回传的测量行判成 todo 71 的 verdict。
//!
//! # 判据在这里，而不是在设备上
//!
//! 设备侧的 `FullAcceptanceTest` 与 `full-measure.sh` 只报「量到了什么」，一个字的结论都
//! 不下。阈值、必需键与判词都在本模块，理由是：让被测物自己判等于把门禁搬进被测物内部，
//! 而那种门禁在被测物坏掉时会一起坏掉。
//!
//! # 三态如何区分
//!
//! - **NOT EXECUTED**：某个必需键根本没回来，或回来的是 `<key>_unavailable=<原因>`。
//!   这是「没测到」，**不是**产品失败。设备侧刻意把「不存在的值」写成 `_unavailable`
//!   而不是空串，正是为了让这条路存在——空串会被读成「测到了一个空值」，进而记 FAIL，
//!   把一次未到达说成产品坏了。
//! - **FAIL**：必需键全部回来，但值不满足预声明的判据。
//! - **PASS**：必需键全部回来且全部满足。
//!
//! # 判据是执行前冻结的
//!
//! [`CRITERIA`] 的内容在跑第一轮真机之前就已写定并提交。事后按实测值放宽等于把门禁
//! 谈掉；本项目在 PR #103 记过这条判据。

use std::collections::BTreeMap;

use super::super::Verdict;

/// 一条断言的必需测量键与判据。
#[derive(Debug, Clone, Copy)]
pub(crate) struct Criterion {
    /// 断言 id，与 `FULL_DECLARED` 逐字一致。
    pub(crate) id: &'static str,
    /// 必需键。任一缺失或标了 `_unavailable` 即 NOT EXECUTED。
    pub(crate) required: &'static [&'static str],
    /// 判据：全部满足才 PASS。
    pub(crate) checks: &'static [Check],
}

/// 单条判据。
#[derive(Debug, Clone, Copy)]
pub(crate) struct Check {
    pub(crate) key: &'static str,
    pub(crate) rule: Rule,
}

/// 判据的比较方式。
#[derive(Debug, Clone, Copy)]
pub(crate) enum Rule {
    /// 必须是 `true`。
    IsTrue,
    /// 必须是 `false`。
    IsFalse,
    /// 数值必须恰好等于。
    Equals(f64),
    /// 数值必须不小于。
    AtLeast(f64),
    /// 数值必须大于。
    GreaterThan(f64),
    /// 字符串必须包含。
    Contains(&'static str),
}

impl Rule {
    fn satisfied(self, raw: &str) -> bool {
        match self {
            Self::IsTrue => raw.eq_ignore_ascii_case("true"),
            Self::IsFalse => raw.eq_ignore_ascii_case("false"),
            Self::Equals(expected) => raw
                .parse::<f64>()
                .is_ok_and(|value| (value - expected).abs() < f64::EPSILON),
            Self::AtLeast(bound) => raw.parse::<f64>().is_ok_and(|value| value >= bound),
            Self::GreaterThan(bound) => raw.parse::<f64>().is_ok_and(|value| value > bound),
            Self::Contains(needle) => raw.contains(needle),
        }
    }

    fn describe(self) -> String {
        match self {
            Self::IsTrue => "== true".to_owned(),
            Self::IsFalse => "== false".to_owned(),
            Self::Equals(value) => format!("== {value}"),
            Self::AtLeast(value) => format!(">= {value}"),
            Self::GreaterThan(value) => format!("> {value}"),
            Self::Contains(needle) => format!("包含 `{needle}`"),
        }
    }
}

const fn check(key: &'static str, rule: Rule) -> Check {
    Check { key, rule }
}

/// **执行前冻结**的十条判据。顺序与 `FULL_DECLARED` 一致。
pub(crate) const CRITERIA: &[Criterion] = &[
    Criterion {
        id: "install_and_launch",
        required: &["root_rendered", "tab_clickable", "crashed", "device_model"],
        checks: &[
            check("root_rendered", Rule::IsTrue),
            check("tab_clickable", Rule::IsTrue),
            check("crashed", Rule::IsFalse),
        ],
    },
    Criterion {
        id: "corpus_first_run_materialization",
        required: &[
            "progress_shown",
            "stage_count",
            "corpus_present",
            "atomic_install",
            "residual_temp_files",
            "duration_seconds",
            "crashed",
        ],
        checks: &[
            check("progress_shown", Rule::IsTrue),
            // 三段是「下载、校验与原子物化」各至少露一次脸的下限。少于三段说明界面
            // 只报了其中一部分，而判据问的是三件事都显示。
            check("stage_count", Rule::AtLeast(3.0)),
            check("corpus_present", Rule::IsTrue),
            check("atomic_install", Rule::IsTrue),
            check("residual_temp_files", Rule::Equals(0.0)),
            check("crashed", Rule::IsFalse),
        ],
    },
    Criterion {
        id: "two_char_search_returns_results",
        required: &["query", "query_char_count", "hits"],
        checks: &[
            check("query_char_count", Rule::Equals(2.0)),
            check("query", Rule::Contains("明月")),
            // 判据原文是「返回至少一条结果」。
            check("hits", Rule::AtLeast(1.0)),
        ],
    },
    Criterion {
        id: "reading_view_citations_and_ai_appreciation",
        // 两类数据的覆盖集互不相交（随包赏析 16 首、集评 394 首，实测交集 0），所以设备侧
        // 打开两首诗分别验证，两个 id 都要回传供复核。判据没有放宽：两样仍须真的显示。
        required: &[
            "commentary_poem_id",
            "commentary_count",
            "citation_shown",
            "appreciation_poem_id",
            "appreciation_shown",
            "disclosure_says_unreviewed",
            "api_key_configured",
        ],
        checks: &[
            check("commentary_count", Rule::AtLeast(1.0)),
            check("citation_shown", Rule::IsTrue),
            check("appreciation_shown", Rule::IsTrue),
            // 「明确标注、未经人工审校」——标注必须真的说出那句话。
            check("disclosure_says_unreviewed", Rule::IsTrue),
            // 随包赏析不得依赖 API key。桌面端 `shipped_appreciation_without_key`
            // 同一条要求。
            check("api_key_configured", Rule::IsFalse),
        ],
    },
    Criterion {
        id: "typed_recitation_scores_correctly",
        required: &[
            "answer_equals_reference",
            "completeness",
            "accuracy_strict",
            "normal_count",
            "deletion_count",
            "insertion_count",
            "substitution_count",
            "rejected",
        ],
        checks: &[
            // 提交与原文逐字相同的答案，所以评分是确定性的：完整度与严格准确都必须是 1，
            // 三类错误计数都必须是 0。「与输入一致」这条判据因此可精确比对，而不是
            // 「看起来差不多」。
            check("answer_equals_reference", Rule::IsTrue),
            check("completeness", Rule::Equals(1.0)),
            check("accuracy_strict", Rule::Equals(1.0)),
            check("normal_count", Rule::AtLeast(1.0)),
            check("deletion_count", Rule::Equals(0.0)),
            check("insertion_count", Rule::Equals(0.0)),
            check("substitution_count", Rule::Equals(0.0)),
            check("rejected", Rule::IsFalse),
        ],
    },
    Criterion {
        id: "voice_recitation_round_succeeds_end_to_end",
        required: &[
            "native_voice_enabled",
            "record_audio_granted",
            "model_dir_present",
            "spoke",
            "total_ms",
            "auto_graded",
        ],
        checks: &[
            check("native_voice_enabled", Rule::IsTrue),
            check("record_audio_granted", Rule::IsTrue),
            check("model_dir_present", Rule::IsTrue),
            check("total_ms", Rule::GreaterThan(0.0)),
            // 2026-08-11 裁决：CER 实测 77.01%，v1 是 `guided_practice`，不做自动评分。
            // 这条 check 是那个裁决的可执行形态——哪天有人把识别结果接进评分，它变红。
            check("auto_graded", Rule::IsFalse),
        ],
    },
    Criterion {
        id: "voice_permission_denied_degrades",
        // **刻意不检查 `record_audio_granted == false`。** 撤销一个已授予的运行时权限
        // 必然重启持有它的进程（`pm revoke` 与 `UiAutomation.revokeRuntimePermission`
        // 两条路径真机实测都产生 `permissions revoked` 那一刀），而 instrumentation
        // 跑在应用进程里，于是整轮以 `Process crashed.` 结束——那条断言永远拿不到结果。
        //
        // 改用 `appops deny android:record_audio`：它拒的是**操作**，`checkSelfPermission`
        // 仍报已授予，但 `AudioRecord` 读到静音流，且不重启进程。判据因此问的是
        // 「采集真的拿不到数据时产品怎么做」，比问「权限位是什么」更贴近用户处境。
        required: &[
            "revoke_executed",
            "revoke_path",
            "degraded_reason",
            "reason_names_capture_denial",
            "fallback_to_typing_shown",
            "crashed",
        ],
        checks: &[
            check("revoke_executed", Rule::IsTrue),
            // 「显示具体原因」：原因里要点名采集被拒，一句「语音不可用」不算。
            check("reason_names_capture_denial", Rule::IsTrue),
            check("fallback_to_typing_shown", Rule::IsTrue),
            check("crashed", Rule::IsFalse),
        ],
    },
    Criterion {
        id: "chinese_ime_prefilled_field_visible",
        required: &[
            "default_ime",
            "field_prefilled",
            "append_preserved_existing",
            "entered_text_present",
            "input_visible",
            "input_bottom_screen_px",
        ],
        checks: &[
            check("field_prefilled", Rule::IsTrue),
            // 「向已有内容的字段输入」：原有内容必须还在，输入的新字也必须在。
            check("append_preserved_existing", Rule::IsTrue),
            check("entered_text_present", Rule::IsTrue),
            // 「键盘不遮挡输入框」：输入框底边必须落在屏幕上一个正的坐标。
            check("input_visible", Rule::IsTrue),
            check("input_bottom_screen_px", Rule::GreaterThan(0.0)),
        ],
    },
    Criterion {
        id: "background_return_preserves_layout",
        required: &[
            "went_background",
            "root_present_after_return",
            "tabs_present_after_return",
            "layout_preserved",
            "blank_screen",
        ],
        checks: &[
            check("went_background", Rule::IsTrue),
            check("root_present_after_return", Rule::IsTrue),
            check("tabs_present_after_return", Rule::IsTrue),
            check("layout_preserved", Rule::IsTrue),
            check("blank_screen", Rule::IsFalse),
        ],
    },
    Criterion {
        id: "app_exits_cleanly",
        required: &["activity_destroyed", "orphan_process_count", "crashed"],
        checks: &[
            check("activity_destroyed", Rule::IsTrue),
            // instrumentation 自己跑在应用进程里，所以「剩 1 个」是正常的；
            // `orphan_process_count` 已经把那一个减掉了。
            check("orphan_process_count", Rule::Equals(0.0)),
            check("crashed", Rule::IsFalse),
        ],
    },
];

/// 一条断言的所有测量键。
pub(crate) type Measurements = BTreeMap<String, String>;

/// 按断言 id 归组的测量值，外加标了不可用的键。
#[derive(Debug, Default)]
pub(crate) struct MeasurementSet {
    values: BTreeMap<String, Measurements>,
    unavailable: BTreeMap<String, BTreeMap<String, String>>,
}

impl MeasurementSet {
    /// 解析设备回传日志。
    ///
    /// 只认两种行：
    ///
    /// ```text
    /// YUNJIAN-FULL <assertion> <key>=<value>
    /// YUNJIAN-FULL <assertion> <key>_unavailable=<reason>
    /// ```
    ///
    /// 其他行一概忽略——回传日志里混着 adb、gradle 与 logcat 的输出，宽松解析会把
    /// 别人的等号当成测量值。
    pub(crate) fn parse(log: &str) -> Self {
        let mut set = Self::default();
        for line in log.lines() {
            let Some(rest) = line
                .split_once("YUNJIAN-FULL ")
                .map(|(_, rest)| rest.trim())
            else {
                continue;
            };
            let Some((assertion, pair)) = rest.split_once(' ') else {
                continue;
            };
            let Some((key, value)) = pair.split_once('=') else {
                continue;
            };
            let assertion = assertion.trim().to_owned();
            let key = key.trim();
            let value = value.trim().to_owned();
            if let Some(base) = key.strip_suffix("_unavailable") {
                set.unavailable
                    .entry(assertion)
                    .or_default()
                    .insert(base.to_owned(), value);
            } else {
                set.values
                    .entry(assertion)
                    .or_default()
                    .insert(key.to_owned(), value);
            }
        }
        set
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.values.is_empty() && self.unavailable.is_empty()
    }

    pub(crate) fn get(&self, assertion: &str, key: &str) -> Option<&str> {
        self.values
            .get(assertion)
            .and_then(|values| values.get(key))
            .map(String::as_str)
    }

    fn unavailable_reason(&self, assertion: &str, key: &str) -> Option<&str> {
        self.unavailable
            .get(assertion)
            .and_then(|keys| keys.get(key))
            .map(String::as_str)
    }

    /// 整套 harness 都没跑起来时设备侧写的原因。
    pub(crate) fn harness_unavailable(&self, assertion: &str) -> Option<&str> {
        self.unavailable_reason(assertion, "harness")
    }

    /// 设备身份三项。宿主侧据此判「是不是物理设备」。
    pub(crate) fn device_identity(&self) -> DeviceIdentity {
        DeviceIdentity {
            model: self
                .get("device_identity", "model")
                .unwrap_or_default()
                .to_owned(),
            os_build: self
                .get("device_identity", "os_build")
                .unwrap_or_default()
                .to_owned(),
            hardware: self
                .get("device_identity", "ro_hardware")
                .unwrap_or_default()
                .to_owned(),
            qemu: self
                .get("device_identity", "ro_kernel_qemu")
                .unwrap_or_default()
                .to_owned(),
            fingerprint: self
                .get("device_identity", "fingerprint")
                .unwrap_or_default()
                .to_owned(),
        }
    }

    /// 本次回传的截图文件名与字节数。
    pub(crate) fn screenshot_for(&self, assertion: &str) -> Option<String> {
        self.values.get(assertion).and_then(|values| {
            values
                .iter()
                .filter(|(key, _)| key.starts_with("screenshot_") && !key.ends_with("_bytes"))
                .map(|(_, name)| name.clone())
                .next()
        })
    }

    /// 判定一条断言。
    pub(crate) fn evaluate(&self, criterion: &Criterion) -> Evaluation {
        if let Some(reason) = self.harness_unavailable(criterion.id) {
            return Evaluation {
                verdict: Verdict::NotExecuted,
                detail: format!("NOT EXECUTED：设备侧 harness 未跑起来（{reason}）"),
            };
        }
        let Some(values) = self.values.get(criterion.id) else {
            if let Some(reasons) = self.unavailable.get(criterion.id) {
                let listed = reasons
                    .iter()
                    .map(|(key, reason)| format!("{key}={reason}"))
                    .collect::<Vec<_>>()
                    .join("、");
                return Evaluation {
                    verdict: Verdict::NotExecuted,
                    detail: format!("NOT EXECUTED：设备侧逐项标注不可用（{listed}）"),
                };
            }
            return Evaluation {
                verdict: Verdict::NotExecuted,
                detail: "NOT EXECUTED：回传日志里没有这条断言的任何测量值".to_owned(),
            };
        };

        let mut missing = Vec::new();
        for key in criterion.required {
            if let Some(reason) = self.unavailable_reason(criterion.id, key) {
                missing.push(format!("{key} 不可用（{reason}）"));
            } else if !values.contains_key(*key) {
                missing.push(format!("{key} 未回传"));
            }
        }
        if !missing.is_empty() {
            return Evaluation {
                verdict: Verdict::NotExecuted,
                detail: format!(
                    "NOT EXECUTED：必需测量值缺失——{}；已回传 {}",
                    missing.join("、"),
                    render_values(values),
                ),
            };
        }

        let mut violations = Vec::new();
        for rule in criterion.checks {
            let raw = values.get(rule.key).map(String::as_str).unwrap_or_default();
            if !rule.rule.satisfied(raw) {
                violations.push(format!(
                    "{}={raw} 不满足 {}",
                    rule.key,
                    rule.rule.describe()
                ));
            }
        }
        if violations.is_empty() {
            Evaluation {
                verdict: Verdict::Pass,
                detail: format!("PASS：实测 {}", render_values(values)),
            }
        } else {
            Evaluation {
                verdict: Verdict::Fail,
                detail: format!(
                    "FAIL：{}；实测 {}",
                    violations.join("；"),
                    render_values(values)
                ),
            }
        }
    }
}

/// 一次判定的结果。
#[derive(Debug)]
pub(crate) struct Evaluation {
    pub(crate) verdict: Verdict,
    pub(crate) detail: String,
}

/// 回传的设备身份。
#[derive(Debug, Default)]
pub(crate) struct DeviceIdentity {
    pub(crate) model: String,
    pub(crate) os_build: String,
    pub(crate) hardware: String,
    pub(crate) qemu: String,
    pub(crate) fingerprint: String,
}

impl DeviceIdentity {
    /// 是不是物理设备。
    ///
    /// # 为什么按属性判而不是信配置
    ///
    /// Device Farm 的公共设备池确实是真机，但「配置里写的是真机池」与「这次回传来自
    /// 真机」是两件事——PR #99 记过「池里有真机 ≠ 真机上装过云笺」。模拟器的
    /// `ro.hardware` 给 `goldfish` / `ranchu`，`ro.kernel.qemu` 给 `1`，
    /// fingerprint 里带 `generic` 或 `sdk_gphone`。按属性判，冒充要连改四项。
    ///
    /// 身份三项任一为空时返回 `false`：拿不到身份就不能声称是物理设备。
    pub(crate) fn is_physical(&self) -> bool {
        if self.model.trim().is_empty()
            || self.os_build.trim().is_empty()
            || self.fingerprint.trim().is_empty()
        {
            return false;
        }
        if self.qemu.trim() == "1" {
            return false;
        }
        let hardware = self.hardware.to_ascii_lowercase();
        if EMULATOR_HARDWARE
            .iter()
            .any(|marker| hardware.contains(marker))
        {
            return false;
        }
        let fingerprint = self.fingerprint.to_ascii_lowercase();
        !EMULATOR_FINGERPRINTS
            .iter()
            .any(|marker| fingerprint.contains(marker))
    }

    /// 判为非物理设备时说清哪一项触发。
    pub(crate) fn rejection(&self) -> String {
        if self.model.trim().is_empty() || self.fingerprint.trim().is_empty() {
            return "设备身份未回传（model 或 fingerprint 为空）".to_owned();
        }
        if self.qemu.trim() == "1" {
            return "ro.kernel.qemu=1，这是模拟器".to_owned();
        }
        let hardware = self.hardware.to_ascii_lowercase();
        if let Some(marker) = EMULATOR_HARDWARE
            .iter()
            .find(|marker| hardware.contains(**marker))
        {
            return format!("ro.hardware={hardware} 含模拟器标记 `{marker}`");
        }
        let fingerprint = self.fingerprint.to_ascii_lowercase();
        if let Some(marker) = EMULATOR_FINGERPRINTS
            .iter()
            .find(|marker| fingerprint.contains(**marker))
        {
            return format!("fingerprint 含模拟器标记 `{marker}`");
        }
        String::new()
    }
}

const EMULATOR_HARDWARE: &[&str] = &["goldfish", "ranchu", "vbox", "cuttlefish", "gce_x86"];
const EMULATOR_FINGERPRINTS: &[&str] = &["generic", "sdk_gphone", "emulator", "cuttlefish"];

fn render_values(values: &Measurements) -> String {
    // 长值截短：一次判词里塞进整段正文会让报告那一行长到没人读，而没人读的判词与没有
    // 判词等价。PR #108 在桌面报告里记过同一件事。
    values
        .iter()
        .map(|(key, value)| {
            let shown: String = value.chars().take(48).collect();
            let suffix = if value.chars().count() > 48 {
                "…"
            } else {
                ""
            };
            format!("{key}={shown}{suffix}")
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn criterion(id: &str) -> &'static Criterion {
        CRITERIA
            .iter()
            .find(|criterion| criterion.id == id)
            .expect("判据必须存在")
    }

    #[test]
    fn criteria_cover_every_declared_assertion_in_order() {
        let declared = super::super::FULL_DECLARED
            .iter()
            .map(|declared| declared.id)
            .collect::<Vec<_>>();
        let judged = CRITERIA
            .iter()
            .map(|criterion| criterion.id)
            .collect::<Vec<_>>();
        assert_eq!(
            declared, judged,
            "每条预声明断言都必须有判据，且顺序一致；否则某条断言会永远拿不到 verdict"
        );
    }

    #[test]
    fn every_criterion_requires_the_keys_its_checks_read() {
        for criterion in CRITERIA {
            for check in criterion.checks {
                assert!(
                    criterion.required.contains(&check.key),
                    "断言 `{}` 的判据读 `{}`，但没把它列进 required；\
                     那会让缺这个键时判成 FAIL 而不是 NOT EXECUTED",
                    criterion.id,
                    check.key
                );
            }
        }
    }

    #[test]
    fn parse_only_accepts_the_declared_prefix() {
        let set = MeasurementSet::parse(
            "noise key=value\n\
             YUNJIAN-FULL install_and_launch root_rendered=true\n\
             I/adb: something=else\n\
             YUNJIAN-FULL install_and_launch crashed=false\n",
        );
        assert_eq!(set.get("install_and_launch", "root_rendered"), Some("true"));
        assert_eq!(set.get("install_and_launch", "crashed"), Some("false"));
        assert_eq!(
            set.get("install_and_launch", "key"),
            None,
            "没有前缀的行不得进入测量集，否则别人的等号会被当成测量值"
        );
    }

    #[test]
    fn a_missing_required_key_is_not_executed_rather_than_fail() {
        let set = MeasurementSet::parse("YUNJIAN-FULL install_and_launch root_rendered=true\n");
        let evaluation = set.evaluate(criterion("install_and_launch"));
        assert_eq!(
            evaluation.verdict,
            Verdict::NotExecuted,
            "必需键缺失是「没测到」，把它记成 FAIL 等于把未执行说成产品失败"
        );
        assert!(evaluation.detail.contains("tab_clickable 未回传"));
    }

    #[test]
    fn an_explicitly_unavailable_key_is_not_executed_with_its_reason() {
        let set = MeasurementSet::parse(
            "YUNJIAN-FULL voice_recitation_round_succeeds_end_to_end native_voice_enabled=true\n\
             YUNJIAN-FULL voice_recitation_round_succeeds_end_to_end spoke_unavailable=asr_weights_not_downloaded_on_device\n",
        );
        let evaluation = set.evaluate(criterion("voice_recitation_round_succeeds_end_to_end"));
        assert_eq!(evaluation.verdict, Verdict::NotExecuted);
        assert!(
            evaluation
                .detail
                .contains("asr_weights_not_downloaded_on_device"),
            "不可用的原因必须原样带进判词，否则读者不知道下一步该做什么：{}",
            evaluation.detail
        );
    }

    #[test]
    fn all_keys_present_but_violating_a_threshold_is_fail() {
        let set = MeasurementSet::parse(
            "YUNJIAN-FULL two_char_search_returns_results query=明月\n\
             YUNJIAN-FULL two_char_search_returns_results query_char_count=2\n\
             YUNJIAN-FULL two_char_search_returns_results hits=0\n",
        );
        let evaluation = set.evaluate(criterion("two_char_search_returns_results"));
        assert_eq!(
            evaluation.verdict,
            Verdict::Fail,
            "键都在但值不达标是真失败，不能记成未执行"
        );
        assert!(evaluation.detail.contains("hits=0"));
    }

    #[test]
    fn all_keys_present_and_satisfying_is_pass() {
        let set = MeasurementSet::parse(
            "YUNJIAN-FULL two_char_search_returns_results query=明月\n\
             YUNJIAN-FULL two_char_search_returns_results query_char_count=2\n\
             YUNJIAN-FULL two_char_search_returns_results hits=7\n",
        );
        let evaluation = set.evaluate(criterion("two_char_search_returns_results"));
        assert_eq!(evaluation.verdict, Verdict::Pass);
        assert!(evaluation.detail.contains("hits=7"));
    }

    #[test]
    fn a_perfect_typed_answer_must_score_exactly_one() {
        let base = "YUNJIAN-FULL typed_recitation_scores_correctly answer_equals_reference=true\n\
             YUNJIAN-FULL typed_recitation_scores_correctly normal_count=20\n\
             YUNJIAN-FULL typed_recitation_scores_correctly deletion_count=0\n\
             YUNJIAN-FULL typed_recitation_scores_correctly insertion_count=0\n\
             YUNJIAN-FULL typed_recitation_scores_correctly substitution_count=0\n\
             YUNJIAN-FULL typed_recitation_scores_correctly rejected=false\n\
             YUNJIAN-FULL typed_recitation_scores_correctly accuracy_strict=1.0\n";
        let pass = MeasurementSet::parse(&format!(
            "{base}YUNJIAN-FULL typed_recitation_scores_correctly completeness=1.0\n"
        ));
        assert_eq!(
            pass.evaluate(criterion("typed_recitation_scores_correctly"))
                .verdict,
            Verdict::Pass
        );
        let fail = MeasurementSet::parse(&format!(
            "{base}YUNJIAN-FULL typed_recitation_scores_correctly completeness=0.98\n"
        ));
        assert_eq!(
            fail.evaluate(criterion("typed_recitation_scores_correctly"))
                .verdict,
            Verdict::Fail,
            "逐字相同的答案必须得满分；0.98 说明评分或归一化有问题"
        );
    }

    #[test]
    fn auto_grading_voice_flips_the_ruling_to_fail() {
        let measurements = "YUNJIAN-FULL voice_recitation_round_succeeds_end_to_end native_voice_enabled=true\n\
             YUNJIAN-FULL voice_recitation_round_succeeds_end_to_end record_audio_granted=true\n\
             YUNJIAN-FULL voice_recitation_round_succeeds_end_to_end model_dir_present=true\n\
             YUNJIAN-FULL voice_recitation_round_succeeds_end_to_end spoke=true\n\
             YUNJIAN-FULL voice_recitation_round_succeeds_end_to_end total_ms=3000\n";
        let pass = MeasurementSet::parse(&format!(
            "{measurements}YUNJIAN-FULL voice_recitation_round_succeeds_end_to_end auto_graded=false\n"
        ));
        assert_eq!(
            pass.evaluate(criterion("voice_recitation_round_succeeds_end_to_end"))
                .verdict,
            Verdict::Pass
        );
        let fail = MeasurementSet::parse(&format!(
            "{measurements}YUNJIAN-FULL voice_recitation_round_succeeds_end_to_end auto_graded=true\n"
        ));
        assert_eq!(
            fail.evaluate(criterion("voice_recitation_round_succeeds_end_to_end"))
                .verdict,
            Verdict::Fail,
            "2026-08-11 裁决禁止语音自动评分（CER 实测 77.01%）；把它接进评分必须变红"
        );
    }

    #[test]
    fn emulator_markers_are_rejected_as_physical_devices() {
        let physical = MeasurementSet::parse(
            "YUNJIAN-FULL device_identity model=Pixel 8\n\
             YUNJIAN-FULL device_identity os_build=15/35\n\
             YUNJIAN-FULL device_identity ro_hardware=zuma\n\
             YUNJIAN-FULL device_identity ro_kernel_qemu=unset\n\
             YUNJIAN-FULL device_identity fingerprint=google/shiba/shiba:15/AP4A.250105.002/12> \n",
        )
        .device_identity();
        assert!(
            physical.is_physical(),
            "真机不该被拒：{}",
            physical.rejection()
        );

        for (label, log) in [
            (
                "qemu",
                "YUNJIAN-FULL device_identity model=sdk\n\
                 YUNJIAN-FULL device_identity os_build=15/35\n\
                 YUNJIAN-FULL device_identity ro_hardware=zuma\n\
                 YUNJIAN-FULL device_identity ro_kernel_qemu=1\n\
                 YUNJIAN-FULL device_identity fingerprint=x/y/z\n",
            ),
            (
                "goldfish",
                "YUNJIAN-FULL device_identity model=sdk\n\
                 YUNJIAN-FULL device_identity os_build=15/35\n\
                 YUNJIAN-FULL device_identity ro_hardware=goldfish\n\
                 YUNJIAN-FULL device_identity ro_kernel_qemu=unset\n\
                 YUNJIAN-FULL device_identity fingerprint=x/y/z\n",
            ),
            (
                "sdk_gphone fingerprint",
                "YUNJIAN-FULL device_identity model=sdk\n\
                 YUNJIAN-FULL device_identity os_build=15/35\n\
                 YUNJIAN-FULL device_identity ro_hardware=zuma\n\
                 YUNJIAN-FULL device_identity ro_kernel_qemu=unset\n\
                 YUNJIAN-FULL device_identity fingerprint=google/sdk_gphone64_arm64/x:15\n",
            ),
            ("空身份", ""),
        ] {
            let identity = MeasurementSet::parse(log).device_identity();
            assert!(
                !identity.is_physical(),
                "`{label}` 必须被判成非物理设备，否则模拟器结果能冒充真机"
            );
            assert!(
                !identity.rejection().is_empty(),
                "`{label}` 被拒时必须说清哪一项触发"
            );
        }
    }
}
