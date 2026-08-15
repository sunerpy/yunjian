//! 报告解析器断言。
//!
//! 这些用例守的不是被测产品，而是**这份报告本身可信**：没有未声明的断言、没有空白
//! verdict、IME 那条一定在且一定有明确裁决、`all_pass` 是布尔且语义严格。
//!
//! 为什么值得为一份报告写测试：报告会被终验消费。一份能悄悄少掉几条断言、或者让
//! `NOT EXECUTED` 算进 `all_pass` 的报告，比没有报告更坏——它会让人以为验过了。

use super::*;

/// 造一份全 PASS 的收集结果，供各条断言复用。
fn all_pass_collector() -> Collector {
    let mut collector = Collector::new();
    for declared in DECLARED {
        collector
            .record(
                declared.id,
                Verdict::Pass,
                "测试桩：已执行并通过",
                None,
                // 要求截图的条目必须给一张，否则 build_report 会拒——那条守卫本身由
                // `passing_ui_assertion_without_a_screenshot_is_rejected` 验证。
                declared
                    .needs_screenshot
                    .then(|| format!("desktop-qa/{}.png", declared.id)),
            )
            .expect("记录已声明的断言应当成功");
    }
    collector
}

#[test]
fn declared_ids_are_unique() {
    let mut ids: Vec<&str> = DECLARED.iter().map(|d| d.id).collect();
    ids.sort_unstable();
    let before = ids.len();
    ids.dedup();
    assert_eq!(before, ids.len(), "断言 id 必须唯一，否则报告会有两行同名");
}

#[test]
fn ime_assertion_is_declared() {
    // 方案点名要求这条存在。它是这个应用最可能对着自己的受众坏掉的方式
    // （Windows WebView2 + TSF 在一个已有内容的输入框首次聚焦时冻住 CJK 输入法）。
    assert!(
        DECLARED.iter().any(|d| d.id == "ime_prefilled_search_box"),
        "必须声明中文输入法往已有内容检索框输入这条断言"
    );
}

#[test]
fn voice_success_and_degradation_are_separate_assertions() {
    // 写成「成功或降级」会被一个语音从未工作过的构建满足，所以必须是两条。
    assert!(
        DECLARED
            .iter()
            .any(|d| d.id == "voice_round_succeeds_end_to_end")
    );
    assert!(
        DECLARED
            .iter()
            .any(|d| d.id == "voice_degradation_states_reason")
    );
}

#[test]
fn recording_an_undeclared_assertion_is_rejected() {
    let mut collector = Collector::new();
    let error = collector
        .record("not_a_real_assertion", Verdict::Pass, "桩", None, None)
        .expect_err("未声明的断言必须被拒绝");
    assert!(
        error.to_string().contains("未在 DECLARED 里声明"),
        "错误信息应指出它未被声明：{error}"
    );
}

#[test]
fn recording_the_same_assertion_twice_is_rejected() {
    let mut collector = Collector::new();
    collector
        .record(DECLARED[0].id, Verdict::Pass, "桩", None, None)
        .expect("第一次记录应当成功");
    let error = collector
        .record(DECLARED[0].id, Verdict::Fail, "桩", None, None)
        .expect_err("重复记录必须被拒绝");
    assert!(
        error.to_string().contains("两次"),
        "错误信息应指出重复：{error}"
    );
}

#[test]
fn blank_detail_is_rejected() {
    let mut collector = Collector::new();
    let error = collector
        .record(DECLARED[0].id, Verdict::Pass, "   ", None, None)
        .expect_err("空白依据必须被拒绝");
    assert!(
        error.to_string().contains("detail 为空"),
        "错误信息应指出依据为空：{error}"
    );
}

#[test]
fn not_executed_without_executable_condition_is_rejected() {
    let mut collector = Collector::new();
    let error = collector
        .record(DECLARED[0].id, Verdict::NotExecuted, "跑不了", None, None)
        .expect_err("未执行必须附可执行条件");
    assert!(
        error.to_string().contains("什么条件下能跑"),
        "错误信息应要求可执行条件：{error}"
    );
}

#[test]
fn fill_remaining_covers_every_declared_assertion() {
    // 「提前退出不会被误当成成功」的机制本身。
    let mut collector = Collector::new();
    collector
        .record(DECLARED[0].id, Verdict::Pass, "桩", None, None)
        .expect("记录应当成功");
    collector
        .fill_remaining("harness 提前结束", "重跑")
        .expect("补齐应当成功");
    let outcomes = collector.finish();
    assert_eq!(
        outcomes.len(),
        DECLARED.len(),
        "补齐之后每条声明都必须有一行"
    );
    assert_eq!(
        outcomes
            .iter()
            .filter(|o| o.verdict == Verdict::NotExecuted)
            .count(),
        DECLARED.len() - 1,
        "除已记录的那条，其余都应是未执行"
    );
}

#[test]
fn report_has_no_unlisted_assertion_and_no_blank_verdict() {
    let report = build_report(
        &repo_root(),
        Platform::Linux,
        "desktop",
        stub_session(),
        all_pass_collector(),
    )
    .expect("构造报告应当成功");

    assert_eq!(
        report.assertions.len(),
        DECLARED.len(),
        "报告的断言条数必须等于声明条数"
    );
    for outcome in &report.assertions {
        assert!(
            DECLARED.iter().any(|d| d.id == outcome.id),
            "报告里出现了未声明的断言 `{}`",
            outcome.id
        );
        assert!(
            !outcome.detail.trim().is_empty(),
            "断言 `{}` 的依据为空",
            outcome.id
        );
    }
    // 序列化后再查一遍：verdict 必须是三个字面量之一，不存在空字符串。
    let encoded = serde_json::to_string(&report).expect("序列化应当成功");
    let parsed: serde_json::Value = serde_json::from_str(&encoded).expect("解析应当成功");
    let assertions = parsed["assertions"]
        .as_array()
        .expect("assertions 应当是数组");
    for entry in assertions {
        let verdict = entry["verdict"].as_str().expect("verdict 应当是字符串");
        assert!(
            matches!(verdict, "PASS" | "FAIL" | "NOT EXECUTED"),
            "verdict 只能取三种字面量，实际 `{verdict}`"
        );
    }
}

#[test]
fn ime_assertion_carries_an_explicit_verdict_in_the_report() {
    let report = build_report(
        &repo_root(),
        Platform::Linux,
        "desktop",
        stub_session(),
        all_pass_collector(),
    )
    .expect("构造报告应当成功");
    let ime = report
        .assertions
        .iter()
        .find(|o| o.id == "ime_prefilled_search_box")
        .expect("报告里必须有 IME 那条断言");
    assert!(
        matches!(
            ime.verdict,
            Verdict::Pass | Verdict::Fail | Verdict::NotExecuted
        ),
        "IME 那条必须有明确裁决"
    );
    assert!(!ime.detail.trim().is_empty(), "IME 那条的依据不能为空");
}

#[test]
fn all_pass_is_a_bool_and_false_when_anything_is_not_executed() {
    let mut collector = all_pass_collector();
    // 把最后一条改成未执行：重建一个只差一条的收集结果。
    collector.outcomes.pop();
    let last = DECLARED.last().expect("断言集不能为空");
    collector
        .record(
            last.id,
            Verdict::NotExecuted,
            "桩：未执行",
            Some("桩条件".to_owned()),
            None,
        )
        .expect("记录应当成功");

    let report = build_report(
        &repo_root(),
        Platform::Linux,
        "desktop",
        stub_session(),
        collector,
    )
    .expect("构造报告应当成功");

    let encoded = serde_json::to_value(&report).expect("序列化应当成功");
    assert!(
        encoded["all_pass"].is_boolean(),
        "all_pass 必须是布尔值，终验会消费它"
    );
    assert!(
        !report.all_pass,
        "只要有一条未执行，all_pass 就必须为假——否则会被读成「三平台都过了」"
    );
    assert_eq!(report.not_executed, 1);
}

#[test]
fn all_pass_is_true_only_when_everything_passed() {
    let report = build_report(
        &repo_root(),
        Platform::Linux,
        "desktop",
        stub_session(),
        all_pass_collector(),
    )
    .expect("构造报告应当成功");
    assert!(report.all_pass, "全 PASS 时 all_pass 应为真");
    assert_eq!(report.failed, 0);
    assert_eq!(report.not_executed, 0);
    assert_eq!(report.executed_pass, DECLARED.len());
}

#[test]
fn markdown_lists_every_assertion_and_flags_unexecuted_platforms() {
    let report = build_report(
        &repo_root(),
        Platform::Linux,
        "desktop",
        stub_session(),
        all_pass_collector(),
    )
    .expect("构造报告应当成功");
    let markdown = render_markdown(&report);
    for declared in DECLARED {
        assert!(
            markdown.contains(declared.id),
            "人读报告里缺少断言 `{}`",
            declared.id
        );
    }
    assert!(
        markdown.contains("未执行的平台"),
        "人读报告必须显著标出哪些平台未执行"
    );
    assert!(
        markdown.contains("windows") && markdown.contains("macos"),
        "未执行的平台一节必须逐个点名"
    );
}

#[test]
fn passing_ui_assertion_without_a_screenshot_is_rejected() {
    // 这条证明上一条不是一句空话：判 PASS 却拿不出图的 UI 断言必须让报告构造失败。
    let mut collector = Collector::new();
    for declared in DECLARED {
        let screenshot = if declared.id == "app_launches" {
            None
        } else {
            declared
                .needs_screenshot
                .then(|| format!("desktop-qa/{}.png", declared.id))
        };
        collector
            .record(declared.id, Verdict::Pass, "测试桩", None, screenshot)
            .expect("记录应当成功");
    }
    let error = build_report(
        &repo_root(),
        Platform::Linux,
        "desktop",
        stub_session(),
        collector,
    )
    .expect_err("缺图的 PASS 必须让报告构造失败");
    assert!(
        error.to_string().contains("app_launches"),
        "错误信息应点名缺图的那条：{error}"
    );
}

#[test]
fn every_ui_assertion_requires_a_screenshot() {
    // 「界面上确实是这样」这句话只有图能作证。凡是 DOM 或 OS 通道的断言都要求截图。
    for declared in DECLARED {
        match declared.channel {
            Channel::WebDriver | Channel::OsHarness => assert!(
                declared.needs_screenshot,
                "UI 断言 `{}` 必须要求截图",
                declared.id
            ),
            Channel::Process => {}
        }
    }
}

#[test]
fn platform_parsing_accepts_the_five_documented_values() {
    assert_eq!(Platform::parse("win").expect("win"), Platform::Windows);
    assert_eq!(Platform::parse("mac").expect("mac"), Platform::MacOs);
    assert_eq!(Platform::parse("linux").expect("linux"), Platform::Linux);
    assert_eq!(
        Platform::parse("android").expect("android"),
        Platform::Android
    );
    assert_eq!(Platform::parse("ios").expect("ios"), Platform::Ios);
    Platform::parse("plan9").expect_err("真正未知的平台必须被拒绝");
}

#[test]
fn mobile_spike_predeclares_four_measurable_criteria() {
    let declared = mobile::DECLARED;
    assert_eq!(declared.len(), 4, "移动端选型必须恰好由四项预声明判据决定");
    assert_eq!(
        declared
            .iter()
            .map(|criterion| criterion.id)
            .collect::<Vec<_>>(),
        [
            "microphone_capture",
            "corpus_materialization",
            "chinese_ime",
            "ios_testflight_submission",
        ]
    );
    for criterion in declared {
        assert!(!criterion.threshold.trim().is_empty(), "每项必须声明阈值");
        assert!(
            criterion.required_measurements.contains(&"device_model")
                && criterion.required_measurements.contains(&"os_build"),
            "判据 `{}` 必须记录物理设备型号与 OS build",
            criterion.id
        );
        assert!(
            !criterion.required_measurements.is_empty(),
            "判据 `{}` 必须声明机器可读测量字段",
            criterion.id
        );
    }
    let microphone = &declared[0].required_measurements;
    for key in ["sample_rate_hz", "channel_count", "rms"] {
        assert!(microphone.contains(&key), "麦克风判据缺少 `{key}`");
    }
}

#[test]
fn unavailable_mobile_hardware_is_not_executed_and_keeps_selection_undetermined() {
    let report = mobile::build_unexecuted_report(Platform::Android);
    assert_eq!(report.verdict, mobile::SelectionVerdict::Undetermined);
    assert_eq!(report.criteria.len(), 4);
    for (criterion, declared) in report.criteria.iter().zip(mobile::DECLARED) {
        assert_eq!(criterion.verdict, Verdict::NotExecuted);
        assert_eq!(criterion.threshold, declared.threshold);
        assert!(
            !criterion.executable_when.trim().is_empty(),
            "NOT EXECUTED 必须给出可执行条件"
        );
        for key in declared.required_measurements {
            assert!(
                criterion.measurement.contains_key(*key),
                "判据 `{}` 缺少机器可读测量字段 `{key}`",
                criterion.id
            );
            assert!(
                criterion.measurement[*key].is_null(),
                "未实测时 `{key}` 必须是 null，不能编造值"
            );
        }
    }
}

#[test]
fn mobile_selection_is_tauri_only_for_four_passes_and_native_for_any_failure() {
    use mobile::SelectionVerdict::{TauriMobile, Undetermined, UniffiNative};

    assert_eq!(
        mobile::selection_verdict(&[Verdict::Pass; 4]),
        TauriMobile,
        "只有四项全 PASS 才能选择 Tauri mobile"
    );
    assert_eq!(
        mobile::selection_verdict(&[
            Verdict::Pass,
            Verdict::NotExecuted,
            Verdict::Pass,
            Verdict::Pass,
        ]),
        Undetermined,
        "未执行不是产品失败，也不能冒充已完成选型"
    );
    assert_eq!(
        mobile::selection_verdict(&[
            Verdict::NotExecuted,
            Verdict::Fail,
            Verdict::NotExecuted,
            Verdict::NotExecuted,
        ]),
        UniffiNative,
        "任一 FAIL 必须强制选择 UniFFI native"
    );
}

#[test]
fn mobile_report_serializes_the_three_stable_selection_literals() {
    assert_eq!(
        serde_json::to_value(mobile::SelectionVerdict::TauriMobile).expect("serialize"),
        "tauri_mobile"
    );
    assert_eq!(
        serde_json::to_value(mobile::SelectionVerdict::UniffiNative).expect("serialize"),
        "uniffi_native"
    );
    assert_eq!(
        serde_json::to_value(mobile::SelectionVerdict::Undetermined).expect("serialize"),
        "undetermined"
    );
}

#[test]
fn foreign_platforms_report_a_reason_and_an_executable_condition() {
    for platform in [Platform::Windows, Platform::MacOs] {
        let (reason, when) = foreign_platform_gap(platform);
        assert!(!reason.is_empty(), "{platform:?} 必须有原因");
        assert!(!when.is_empty(), "{platform:?} 必须有可执行条件");
    }
}

fn stub_session() -> SessionFacts {
    SessionFacts {
        display_kind: "virtual".to_owned(),
        display: Some(":99".to_owned()),
        window_manager: Some("Openbox".to_owned()),
        webdriver: WebDriverProbe {
            attempted: true,
            succeeded: false,
            detail: "测试桩".to_owned(),
        },
        audio_input_devices: 0,
    }
}
