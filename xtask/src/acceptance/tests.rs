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
fn close_and_exit_assertions_preserve_the_tray_contract() {
    let close = DECLARED
        .iter()
        .find(|declared| declared.id == "control_close_works")
        .expect("必须声明关闭按钮断言");
    let exit = DECLARED
        .iter()
        .find(|declared| declared.id == "app_exits_cleanly")
        .expect("必须声明正常退出断言");

    assert!(close.what.contains("隐藏到托盘") && close.what.contains("进程继续运行"));
    assert!(exit.what.contains("托盘菜单") && exit.what.contains("退出码 0"));
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

/// 一份让判据①②③全部 PASS 的设备侧日志。判据④刻意不给测量值：用户已决定不做 iOS 提交。
///
/// `overrides` 按 `("<criterion> <key>", "<value>")` 覆盖单个测量值，于是每条断言只需说清
/// 它改了哪一个量，不必各自维护一份完整日志。
fn android_device_log(overrides: &[(&str, &str)]) -> String {
    let mut lines: Vec<(String, String)> = [
        ("microphone_capture device_model", "Pixel 8"),
        ("microphone_capture os_build", "15/35"),
        ("microphone_capture sample_rate_hz", "16000"),
        ("microphone_capture channel_count", "1"),
        ("microphone_capture rms", "0.0031"),
        (
            "microphone_capture permission_plugin",
            "record_audio_granted+modify_audio_settings_granted",
        ),
        ("corpus_materialization device_model", "Pixel 8"),
        ("corpus_materialization os_build", "15/35"),
        ("corpus_materialization artifact_bytes", "223113374"),
        ("corpus_materialization sha256_verified", "true"),
        ("corpus_materialization duration_seconds", "41.2"),
        ("corpus_materialization atomic_install", "true"),
        ("corpus_materialization crashed", "false"),
        (
            "corpus_materialization production_path",
            "yunjian_core::assets::AssetResolver::{discover,new}+sync",
        ),
        ("chinese_ime device_model", "Pixel 8"),
        ("chinese_ime os_build", "15/35"),
        ("chinese_ime target_sdk", "35"),
        ("chinese_ime edge_to_edge", "true"),
        ("chinese_ime entered_text", "云笺"),
        ("chinese_ime keyboard_overlap_px", "0"),
        ("chinese_ime input_visible", "true"),
        ("chinese_ime visual_viewport_updated", "true"),
    ]
    .iter()
    .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
    .collect();
    for (key, value) in overrides {
        let slot = lines
            .iter_mut()
            .find(|(existing, _)| existing == key)
            .unwrap_or_else(|| panic!("覆盖的测量键 `{key}` 不在基线日志里，写错了就该红"));
        slot.1 = (*value).to_owned();
    }
    lines
        .iter()
        .map(|(key, value)| format!("YUNJIAN-MEASURE {key}={value}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn verdict_of(criterion: &str, report: &mobile::MobileReport) -> Verdict {
    report
        .criteria
        .iter()
        .find(|item| item.id == criterion)
        .unwrap_or_else(|| panic!("报告里没有判据 `{criterion}`"))
        .verdict
}

#[test]
fn three_android_criteria_can_reach_pass_from_a_real_device_log() {
    let report = mobile::build_report_from_device_log(Platform::Android, &android_device_log(&[]));
    for criterion in [
        "microphone_capture",
        "corpus_materialization",
        "chinese_ime",
    ] {
        assert_eq!(
            verdict_of(criterion, &report),
            Verdict::Pass,
            "判据 `{criterion}` 在测量值齐备且达阈值时必须能拿到 PASS，否则门禁根本不可执行：{:?}",
            report
                .criteria
                .iter()
                .find(|item| item.id == criterion)
                .map(|item| &item.detail)
        );
    }
    assert_eq!(
        verdict_of("ios_testflight_submission", &report),
        Verdict::NotExecuted,
        "用户已决定不做 iOS 提交，它既不该是 PASS 也不该是 FAIL"
    );
    mobile::validate_consistency(&report).expect("一致性校验必须通过");
}

#[test]
fn three_passes_plus_the_out_of_scope_ios_criterion_still_yields_undetermined() {
    let report = mobile::build_report_from_device_log(Platform::Android, &android_device_log(&[]));
    assert_eq!(
        report.verdict,
        mobile::SelectionVerdict::Undetermined,
        "一条范围外的判据既不是 FAIL 也不是 PASS，顶层只能是 undetermined"
    );
    assert!(
        report.verdict_rationale.contains("判据④"),
        "顶层裁决必须说明它为何停在这里：{}",
        report.verdict_rationale
    );
    assert!(
        !report.verdict_rationale.trim().is_empty()
            && report.verdict_rationale.contains("undetermined")
    );
}

#[test]
fn the_rationale_argues_the_verdict_the_report_actually_carries() {
    // 一份写着 `uniffi_native` 却在解释 `undetermined` 为何合理的报告，比没有说明更糟：
    // 读者会以为门禁还没有结论。所以文案必须随裁决走。
    let failed = mobile::build_report_from_device_log(
        Platform::Android,
        &android_device_log(&[("corpus_materialization crashed", "true")]),
    );
    assert_eq!(failed.verdict, mobile::SelectionVerdict::UniffiNative);
    assert!(
        failed.verdict_rationale.contains("uniffi_native")
            && !failed.verdict_rationale.contains("故顶层保持 undetermined"),
        "FAIL 的报告不能拿 undetermined 的理由充数：{}",
        failed.verdict_rationale
    );

    let undetermined =
        mobile::build_report_from_device_log(Platform::Android, &android_device_log(&[]));
    assert_eq!(undetermined.verdict, mobile::SelectionVerdict::Undetermined);
    assert!(
        undetermined.verdict_rationale.contains("undetermined"),
        "undetermined 的报告必须说清它为何还没有结论：{}",
        undetermined.verdict_rationale
    );
}

#[test]
fn the_targetsdk_the_build_actually_shipped_is_a_failure_not_a_relaxed_threshold() {
    let report = mobile::build_report_from_device_log(
        Platform::Android,
        &android_device_log(&[("chinese_ime target_sdk", "36")]),
    );
    assert_eq!(
        verdict_of("chinese_ime", &report),
        Verdict::Fail,
        "判据声明的是 targetSdk 35；实测 36 必须判 FAIL，而不是把阈值放宽到 >= 35"
    );
    assert_eq!(
        report.verdict,
        mobile::SelectionVerdict::UniffiNative,
        "任一 FAIL 必须强制 uniffi_native"
    );
    mobile::validate_consistency(&report).expect("FAIL 也要能通过一致性校验");
}

#[test]
fn an_injected_failure_makes_tauri_mobile_unreachable() {
    for (key, value) in [
        ("microphone_capture rms", "0"),
        ("microphone_capture sample_rate_hz", "48000"),
        ("microphone_capture channel_count", "2"),
        (
            "microphone_capture permission_plugin",
            "record_audio_denied+modify_audio_settings_granted",
        ),
        ("corpus_materialization sha256_verified", "false"),
        ("corpus_materialization duration_seconds", "72.5"),
        ("corpus_materialization atomic_install", "false"),
        ("corpus_materialization crashed", "true"),
        ("chinese_ime keyboard_overlap_px", "184"),
        ("chinese_ime edge_to_edge", "false"),
        ("chinese_ime input_visible", "false"),
        ("chinese_ime visual_viewport_updated", "false"),
    ] {
        let report = mobile::build_report_from_device_log(
            Platform::Android,
            &android_device_log(&[(key, value)]),
        );
        assert_eq!(
            report.verdict,
            mobile::SelectionVerdict::UniffiNative,
            "把 `{key}` 设成 `{value}` 后选型必须落到 uniffi_native，门禁不能被谈过去"
        );
        assert_ne!(
            report.verdict,
            mobile::SelectionVerdict::TauriMobile,
            "存在 FAIL 时 tauri_mobile 必须不可达"
        );
        mobile::validate_consistency(&report).expect("注入失败后仍须自洽");
    }
}

#[test]
fn a_zero_overlap_outside_edge_to_edge_does_not_earn_a_pass() {
    let report = mobile::build_report_from_device_log(
        Platform::Android,
        &android_device_log(&[("chinese_ime edge_to_edge", "false")]),
    );
    let criterion = report
        .criteria
        .iter()
        .find(|item| item.id == "chinese_ime")
        .expect("chinese_ime");
    assert_eq!(criterion.verdict, Verdict::Fail);
    assert!(
        criterion.detail.contains("edge-to-edge"),
        "必须点明失败在边到边这一项，否则读者会以为是遮挡量的问题：{}",
        criterion.detail
    );
}

#[test]
fn a_missing_device_measurement_is_still_not_executed_rather_than_failed() {
    // 只把一项换成 UNAVAILABLE，其余保持可通过：结果必须是 NOT EXECUTED 而不是 FAIL，
    // 否则一次「设备没测到」会替产品背一个失败，并把选型推向 uniffi_native。
    let log = format!(
        "{}\nYUNJIAN-MEASURE-UNAVAILABLE chinese_ime entered_text reason=soft_keyboard_never_appeared",
        android_device_log(&[]).replace("YUNJIAN-MEASURE chinese_ime entered_text=云笺\n", "")
    );
    let report = mobile::build_report_from_device_log(Platform::Android, &log);
    assert_eq!(
        verdict_of("chinese_ime", &report),
        Verdict::NotExecuted,
        "缺一项必需测量值只能是 NOT EXECUTED"
    );
    assert_eq!(report.verdict, mobile::SelectionVerdict::Undetermined);
}

#[test]
fn every_amended_threshold_is_recorded_in_the_report_text() {
    let report = mobile::build_unexecuted_report(Platform::Android);
    let microphone = &report.criteria[0];
    assert!(
        microphone.threshold.contains("permission_plugin =="),
        "判据①的权限路径阈值必须写在声明里，不能只活在代码里：{}",
        microphone.threshold
    );
    for fragment in ["sample_rate_hz == 16000", "channel_count == 1", "rms > 0"] {
        assert!(
            microphone.threshold.contains(fragment),
            "采集参数阈值不得在修订中被削弱，缺 `{fragment}`"
        );
    }
    let ime = report
        .criteria
        .iter()
        .find(|item| item.id == "chinese_ime")
        .expect("chinese_ime");
    assert!(
        ime.threshold.contains("target_sdk == 35"),
        "判据③的 targetSdk 阈值必须仍是等于 35：{}",
        ime.threshold
    );
    assert!(
        ime.threshold.contains("edge_to_edge == true"),
        "新增的边到边必需项必须写进声明：{}",
        ime.threshold
    );
}

#[test]
fn mobile_full_predeclares_every_real_device_assertion() {
    assert_eq!(
        mobile::FULL_DECLARED
            .iter()
            .map(|assertion| assertion.id)
            .collect::<Vec<_>>(),
        [
            "install_and_launch",
            "corpus_first_run_materialization",
            "two_char_search_returns_results",
            "reading_view_citations_and_ai_appreciation",
            "typed_recitation_scores_correctly",
            "voice_recitation_round_succeeds_end_to_end",
            "voice_permission_denied_degrades",
            "chinese_ime_prefilled_field_visible",
            "background_return_preserves_layout",
            "app_exits_cleanly",
        ],
        "todo 71 的十项真机断言必须在执行前完整冻结"
    );
    for assertion in mobile::FULL_DECLARED {
        assert!(assertion.needs_screenshot, "每项真机断言都必须要求截图");
        assert!(
            !assertion.exact_command.trim().is_empty(),
            "断言 `{}` 必须给出自动化执行命令",
            assertion.id
        );
        assert!(
            !assertion.executable_when.trim().is_empty(),
            "断言 `{}` 必须声明物理设备前置条件",
            assertion.id
        );
    }
}

#[test]
fn unavailable_mobile_devices_produce_a_complete_auditable_full_report() {
    let report = mobile::build_unexecuted_full_report();
    assert!(
        !report.all_pass,
        "存在 NOT EXECUTED 时 all_pass 必须为 false"
    );
    assert_eq!(report.platforms.len(), 2, "报告必须同时覆盖 Android 与 iOS");
    assert!(!report.app_version.trim().is_empty());
    assert!(!report.commit_sha.trim().is_empty());

    for platform in &report.platforms {
        assert!(!platform.physical_device_used);
        assert!(platform.device_model.starts_with("NOT EXECUTED:"));
        assert!(platform.os_version.starts_with("NOT EXECUTED:"));
        assert_eq!(platform.assertions.len(), mobile::FULL_DECLARED.len());
        for (actual, declared) in platform.assertions.iter().zip(mobile::FULL_DECLARED) {
            assert_eq!(actual.id, declared.id);
            assert_eq!(actual.verdict, Verdict::NotExecuted);
            assert!(!actual.detail.trim().is_empty());
            assert!(!actual.executable_when.trim().is_empty());
            assert!(!actual.exact_command.trim().is_empty());
            assert!(actual.screenshot.is_none(), "未执行时不得伪造截图");
        }
    }

    let encoded = serde_json::to_string(&report).expect("serialize full report");
    mobile::validate_full_report_json(&encoded).expect("完整的未执行报告也必须通过结构校验");
}

#[test]
fn mobile_full_parser_rejects_an_unlisted_assertion_and_blank_verdict() {
    let report = mobile::build_unexecuted_full_report();
    let mut value = serde_json::to_value(report).expect("serialize full report");
    value["platforms"][0]["assertions"][0]["id"] = "injected_unlisted_assertion".into();
    let error = mobile::validate_full_report_json(&value.to_string())
        .expect_err("未声明 assertion id 必须被解析器拒绝");
    assert!(error.to_string().contains("未声明"), "{error}");

    let mut value = serde_json::to_value(mobile::build_unexecuted_full_report())
        .expect("serialize full report");
    value["platforms"][0]["assertions"][0]["verdict"] = "".into();
    mobile::validate_full_report_json(&value.to_string()).expect_err("空 verdict 必须被解析器拒绝");
}

#[test]
fn generated_mobile_full_report_passes_the_same_parser() {
    let report_dir = repo_root().join("docs/reports");
    let mut reports = std::fs::read_dir(&report_dir)
        .expect("报告目录必须存在")
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("mobile-qa-") && name.ends_with(".json"))
        })
        .collect::<Vec<_>>();
    reports.sort();
    let path = reports.last().expect("必须先生成 mobile-qa JSON 报告");
    let encoded = std::fs::read_to_string(path).expect("必须能读取 mobile-qa JSON 报告");
    mobile::validate_full_report_json(&encoded)
        .unwrap_or_else(|error| panic!("生成报告 {} 校验失败：{error}", path.display()));
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
        audio_capture: "测试桩".to_owned(),
    }
}

/// 拆除被托管进程时必须连它的子进程一起收干净。
///
/// 这条守的是一个**实测过的**泄漏：`Child::kill` 发的是 SIGKILL，捕不到，于是
/// `tauri-driver` 没有机会回收自己的 `WebKitWebDriver` 子进程——那个孤儿会继续监听端口，
/// 而它正是启动被测应用的那一方，于是下一轮的判词写成「麦克风被别的程序独占」或
/// 「driver 起来后没有监听端口」，两句都指着没坏的东西。
///
/// 用 `sh -c` 造一棵同形态的进程树而不是真起 `tauri-driver`：要守的是
/// [`Background`] 的拆除语义，而它与那个具体程序无关；真起 driver 还会让这条用例
/// 依赖宿主机装了什么。
#[test]
fn dropping_a_background_process_reaps_its_child() {
    let marker = std::env::temp_dir().join(format!("yunjian-reap-{}", std::process::id()));
    let _ = fs::remove_file(&marker);

    // 父进程收到 SIGTERM 时先把子进程杀掉再退——`tauri-driver` 就是这个形态。
    // 子进程留一个文件当活着的证据，好让「有没有被收掉」变成一个可观测的事实。
    let script = format!(
        "trap 'kill $CHILD 2>/dev/null; exit 0' TERM; \
         sh -c 'touch {marker}; while true; do sleep 0.2; done' & \
         CHILD=$!; wait $CHILD",
        marker = marker.display()
    );
    let mut command = Command::new("sh");
    command.arg("-c").arg(&script);
    let process = Background::spawn("reap-probe", &mut command).expect("起探针进程");

    let appeared = Background::wait_until(Duration::from_secs(10), || marker.is_file());
    assert!(appeared, "探针的子进程没有起来，这条用例无从判定");

    drop(process);

    let gone = Background::wait_until(Duration::from_secs(10), || !marker_owner_alive(&marker));
    let _ = fs::remove_file(&marker);
    assert!(
        gone,
        "拆除被托管进程后它的子进程仍活着：说明发的是 SIGKILL，\
         被托管进程来不及回收自己的子进程"
    );
}

/// 还有没有进程持有那棵探针进程树。
///
/// 按命令行里的 marker 路径认领进程，而不是按进程名：探针是 `sh`，宿主机上到处都是 `sh`。
fn marker_owner_alive(marker: &std::path::Path) -> bool {
    let needle = marker.display().to_string();
    let Ok(entries) = fs::read_dir("/proc") else {
        return false;
    };
    entries.flatten().any(|entry| {
        entry.file_name().to_string_lossy().parse::<u32>().is_ok()
            && fs::read_to_string(entry.path().join("cmdline"))
                .is_ok_and(|line| line.contains(&needle))
    })
}
