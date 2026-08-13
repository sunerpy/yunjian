//! 「不做读音判定」的门禁。
//!
//! 2026-08-11 裁决明令语音路径不得报告或暗示 声韵 / 调型 / 发音 评分。这条约束靠三道
//! 检查守住，缺一道都留下真实缺口：
//!
//! 1. **穷尽字段清单**：把 [`SessionScore`] 与 [`RhythmInputs`] 的公开字段逐个写死在下面
//!    的解构里。往这两个类型里加任何字段都会让本文件编译失败，作者必须回来面对这条约束。
//! 2. **grep 守卫**：扫 `crates/yunjian-voice/src/session.rs` 的标识符与文档，禁止出现
//!    发音评分类词汇的**肯定用法**。
//! 3. **标签断言**：界面上这个指标只能叫「节奏连贯度」。
//!
//! 前两道互补而不重复：字段清单挡的是「加了个 `phone_score` 字段」，grep 挡的是
//! 「把连贯度的文档写成读音是否标准」。

use yunjian_voice::session::{COHERENCE_LABEL, RhythmInputs, SessionScore};

/// 任何形式的读音评分都不许出现在语音路径上。`fluency` 也在内：那是打字路径里那个中性
/// 满值字段的名字，语音侧刻意改叫 coherence，混用会让两条路径的语义悄悄合流。
const FORBIDDEN: [&str; 10] = [
    "phone_score",
    "tone_score",
    "pronunciation",
    "声韵",
    "调型",
    "发音评分",
    "读音评分",
    "字准",
    "accuracy",
    "completeness",
];

/// 本门禁自己的文件名。`session.rs` 的文档指向它是**引用**而非声称，因此不算违规。
const GUARD_FILE: &str = "no_pronunciation_scoring";

fn session_source() -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/session.rs");
    std::fs::read_to_string(&path).expect("读取 session.rs")
}

#[test]
fn the_public_score_type_has_no_pronunciation_field() {
    let inputs = RhythmInputs::from_parts(0.0, 0, 1.0);
    let config = yunjian_core::VoiceSessionConfig::default();
    let score = SessionScore {
        feedback: yunjian_recite::VoicePracticeFeedback::new(
            true,
            0,
            yunjian_recite::RelativeRhythm::Similar,
        ),
        coherence: yunjian_voice::session::coherence(&inputs, &config),
        inputs,
        lines_attempted: 1,
        prompt_count: 0,
    };

    let SessionScore {
        feedback,
        coherence,
        inputs,
        lines_attempted,
        prompt_count,
    } = score;

    let yunjian_recite::VoicePracticeFeedback {
        spoke,
        pause_count,
        relative_rhythm,
    } = feedback;
    assert!(spoke);
    assert_eq!(pause_count, 0);
    assert_eq!(relative_rhythm, yunjian_recite::RelativeRhythm::Similar);
    assert!((coherence.value() - 1.0).abs() < 1e-9);
    assert_eq!(lines_attempted, 1);
    assert_eq!(prompt_count, 0);

    assert!(inputs.gap_variance_ms2().abs() < f64::EPSILON);
    assert_eq!(inputs.long_pause_count(), 0);
    assert!((inputs.duration_ratio() - 1.0).abs() < f64::EPSILON);
}

#[test]
fn the_session_source_never_claims_pronunciation_scoring() {
    let source = session_source();
    for line in source.lines() {
        let trimmed = line.trim();
        let is_negation = trimmed.starts_with("//")
            && (trimmed.contains("不")
                || trimmed.contains("never")
                || trimmed.contains("no ")
                || trimmed.contains("NOT")
                || trimmed.contains(GUARD_FILE));
        if is_negation {
            continue;
        }
        for needle in FORBIDDEN {
            assert!(
                !line.contains(needle),
                "session.rs 出现了读音评分类词汇 `{needle}`，且不在否定语境里：{line}"
            );
        }
    }
}

#[test]
fn the_metric_has_exactly_one_user_facing_name() {
    assert_eq!(COHERENCE_LABEL, "节奏连贯度");
    for needle in ["流畅度", "发音", "标准"] {
        assert!(
            !COHERENCE_LABEL.contains(needle),
            "界面标签不得含 `{needle}`"
        );
    }
}

#[test]
fn the_three_rhythm_inputs_are_the_only_way_into_coherence() {
    let source = session_source();
    let signature = source
        .lines()
        .find(|line| line.contains("pub fn coherence("))
        .expect("session.rs 必须有 coherence 的签名");
    assert!(
        signature.contains("&RhythmInputs") && signature.contains("&VoiceSessionConfig"),
        "coherence 只能看见 RhythmInputs 与配置，实际签名：{signature}"
    );
    assert!(
        !signature.contains("str") && !signature.contains("Hyp"),
        "coherence 的签名里不得出现任何转写类型：{signature}"
    );
}
