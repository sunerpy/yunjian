//! 仅用于打字练习的确定性评分模型。

use crate::align::{AlignOp, Alignment, align_normalized, normalize_text};
use yunjian_core::{CorpusHandle, Result};

const MIN_MATCH_RATIO: f32 = 0.35;
const MAX_CER: f32 = 0.6;

/// 已按语料字形规则归一化的参考诗文。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Poem(String);

impl Poem {
    /// 从诗文原文构造参考输入。
    pub fn new(handle: &CorpusHandle, text: &str) -> Result<Self> {
        normalize_text(handle, text).map(Self)
    }

    /// 返回归一化后的参考文本。
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// 用户主动键入、可进入确定性评分的尝试。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypedAttempt(String);

impl TypedAttempt {
    /// 从用户键入文本构造评分输入。
    pub fn new(handle: &CorpusHandle, text: &str) -> Result<Self> {
        normalize_text(handle, text).map(Self)
    }

    /// 返回归一化后的键入文本。
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// 只供语音诊断或展示使用、不得进入打字评分的偏置识别文本。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BiasedHyp(String);

impl BiasedHyp {
    /// 保存一段语音识别展示文本。
    #[must_use]
    pub fn new(text: String) -> Self {
        Self(text)
    }

    /// 返回展示文本。
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// 一次对齐中各类操作的计数。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct OpsSummary {
    /// 正常匹配数。
    pub normal_count: usize,
    /// 漏读数。
    pub deletion_count: usize,
    /// 增读数。
    pub insertion_count: usize,
    /// 回读片段数。
    pub rerecitation_count: usize,
    /// 替换数。
    pub substitution_count: usize,
}

/// 确定性打字练习分数。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TypedScore {
    /// 未漏读字符所占比例。
    pub completeness: f32,
    /// 按字符错误率计算的严格准确度。
    pub accuracy_strict: f32,
    /// 近音宽容后的准确度；宽容层接入前与严格准确度相同。
    pub accuracy_lenient: f32,
    /// 打字路径没有时序信号，使用中性满值且不表示发音质量。
    pub fluency: f32,
    /// 是否因匹配过低或字符错误率过高而拒绝识别。
    pub is_rejected: bool,
    /// 各类对齐操作的计数。
    pub ops_summary: OpsSummary,
}

/// 用户复诵相对示范音的节奏。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelativeRhythm {
    /// 比示范音更慢。
    Slower,
    /// 与示范音大致相近。
    Similar,
    /// 比示范音更快。
    Faster,
}

/// 不含字准、漏字列表或自动评级的语音练习反馈。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VoicePracticeFeedback {
    /// 是否检测到用户开口。
    pub spoke: bool,
    /// 检测到的停顿次数。
    pub pause_count: usize,
    /// 相对于示范音的节奏。
    pub relative_rhythm: RelativeRhythm,
}

impl VoicePracticeFeedback {
    /// 构造一份仅含活动和节奏信号的反馈。
    #[must_use]
    pub const fn new(spoke: bool, pause_count: usize, relative_rhythm: RelativeRhythm) -> Self {
        Self {
            spoke,
            pause_count,
            relative_rhythm,
        }
    }
}

/// 为一次用户键入尝试评分。
#[must_use]
pub fn score_typed(reference: &Poem, attempt: &TypedAttempt) -> TypedScore {
    score_alignment(align_normalized(reference.as_str(), attempt.as_str()))
}

fn score_alignment(alignment: Alignment) -> TypedScore {
    let ops_summary = summarize_ops(&alignment.ops);
    let reference_len = alignment.reference_len as f32;
    let (completeness, accuracy_strict, match_ratio, cer) = if alignment.reference_len == 0 {
        let empty_attempt = alignment.ops.is_empty();
        let exact = f32::from(empty_attempt);
        (exact, exact, exact, 1.0 - exact)
    } else {
        let completeness = 1.0 - ops_summary.deletion_count as f32 / reference_len;
        let error_count = ops_summary.substitution_count
            + ops_summary.deletion_count
            + ops_summary.insertion_count;
        let cer = error_count as f32 / reference_len;
        (
            completeness,
            1.0 - cer,
            alignment.matched_len as f32 / reference_len,
            cer,
        )
    };
    let completeness = clamp_score(completeness);
    let accuracy_strict = clamp_score(accuracy_strict);
    TypedScore {
        completeness,
        accuracy_strict,
        accuracy_lenient: accuracy_strict,
        fluency: 1.0,
        is_rejected: match_ratio < MIN_MATCH_RATIO || cer > MAX_CER,
        ops_summary,
    }
}

fn summarize_ops(ops: &[AlignOp]) -> OpsSummary {
    let mut summary = OpsSummary::default();
    for op in ops {
        match op {
            AlignOp::Normal { .. } => summary.normal_count += 1,
            AlignOp::Deletion { .. } => summary.deletion_count += 1,
            AlignOp::Insertion { .. } => summary.insertion_count += 1,
            AlignOp::ReRecitation { .. } => summary.rerecitation_count += 1,
            AlignOp::Substitution { .. } => summary.substitution_count += 1,
        }
    }
    summary
}

fn clamp_score(score: f32) -> f32 {
    score.clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::{Connection, params};
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};
    use yunjian_core::{CorpusConfig, CorpusHandle, SCHEMA_VERSION};

    static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);

    struct Fixture {
        dir: PathBuf,
        handle: CorpusHandle,
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    fn fixture() -> Fixture {
        let dir = std::env::temp_dir().join(format!(
            "yunjian-recite-score-{}-{}",
            std::process::id(),
            NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("创建评分 fixture 目录");
        let path = dir.join("corpus.db");
        write_fixture(&path);
        let handle = CorpusHandle::open(&CorpusConfig {
            path: Some(path),
            data_dir: dir.clone(),
            archive: None,
        })
        .expect("打开评分 fixture");
        Fixture { dir, handle }
    }

    fn write_fixture(path: &Path) {
        let connection = Connection::open(path).expect("创建评分 fixture 数据库");
        connection
            .execute_batch(
                "CREATE TABLE poem(stable_id TEXT PRIMARY KEY NOT NULL, body TEXT NOT NULL);
                 CREATE TABLE variant_map(
                     src_char TEXT PRIMARY KEY NOT NULL,
                     dst_char TEXT NOT NULL
                 ) WITHOUT ROWID;
                 CREATE TABLE corpus_meta(
                     singleton INTEGER PRIMARY KEY NOT NULL CHECK(singleton=1),
                     schema_version INTEGER NOT NULL,
                     corpus_version TEXT NOT NULL,
                     built_at TEXT NOT NULL,
                     poem_count INTEGER NOT NULL,
                     index_detail_mode TEXT NOT NULL,
                     derived_indexes TEXT NOT NULL,
                     shipped_scope TEXT NOT NULL,
                     integrity_check TEXT NOT NULL
                 );",
            )
            .expect("创建评分 fixture schema");
        connection
            .execute(
                "INSERT INTO variant_map(src_char, dst_char) VALUES (?1, ?2)",
                params!["國", "国"],
            )
            .expect("写 variant_map");
        connection
            .execute(
                "INSERT INTO corpus_meta VALUES
                 (1, ?1, 'fixture-v1', '2026-08-12T00:00:00Z', 0, 'full',
                  'first_launch', '10k', 'ok')",
                [SCHEMA_VERSION],
            )
            .expect("写 corpus_meta");
        connection.close().expect("关闭评分 fixture 数据库");
    }

    fn poem(handle: &CorpusHandle, text: &str) -> Poem {
        Poem::new(handle, text).expect("构造参考诗文")
    }

    fn attempt(handle: &CorpusHandle, text: &str) -> TypedAttempt {
        TypedAttempt::new(handle, text).expect("构造键入尝试")
    }

    #[test]
    fn verbatim_reference_is_a_perfect_score_with_zero_errors() {
        let fixture = fixture();
        let reference = poem(
            &fixture.handle,
            "床前明月光，疑是地上霜。举头望明月，低头思故乡。",
        );
        let score = score_typed(
            &reference,
            &attempt(
                &fixture.handle,
                "床前明月光，疑是地上霜。举头望明月，低头思故乡。",
            ),
        );

        assert_eq!(score.completeness, 1.0);
        assert_eq!(score.accuracy_strict, 1.0);
        assert_eq!(score.accuracy_lenient, 1.0);
        assert_eq!(score.fluency, 1.0);
        assert!(!score.is_rejected);
        assert_eq!(score.ops_summary.deletion_count, 0);
        assert_eq!(score.ops_summary.insertion_count, 0);
        assert_eq!(score.ops_summary.rerecitation_count, 0);
        assert_eq!(score.ops_summary.substitution_count, 0);
        assert_eq!(score.ops_summary.normal_count, 20);
    }

    #[test]
    fn deleting_one_line_counts_each_missing_character() {
        let fixture = fixture();
        let reference = poem(
            &fixture.handle,
            "床前明月光，疑是地上霜。举头望明月，低头思故乡。",
        );
        let score = score_typed(
            &reference,
            &attempt(&fixture.handle, "床前明月光，疑是地上霜。低头思故乡。"),
        );

        assert_eq!(score.ops_summary.deletion_count, 5);
        assert_eq!(score.completeness, 0.75);
        assert_eq!(score.accuracy_strict, 0.75);
        assert_eq!(score.accuracy_lenient, 0.75);
        assert!(!score.is_rejected);
    }

    #[test]
    fn an_unrelated_poem_is_rejected_instead_of_looking_plausible() {
        let fixture = fixture();
        let reference = poem(&fixture.handle, "床前明月光疑是地上霜");
        let score = score_typed(
            &reference,
            &attempt(&fixture.handle, "白日依山尽黄河入海流"),
        );

        assert!(score.is_rejected);
        assert_eq!(score.accuracy_strict, 0.0);
    }

    #[test]
    fn either_rejection_threshold_is_sufficient() {
        let low_match = score_alignment(Alignment {
            ops: vec![AlignOp::Normal {
                reference_index: 0,
                attempt_index: 0,
                character: '山',
            }],
            matched_len: 1,
            reference_len: 4,
        });
        assert!(low_match.is_rejected);

        let high_cer = score_alignment(Alignment {
            ops: vec![
                AlignOp::Normal {
                    reference_index: 0,
                    attempt_index: 0,
                    character: '山',
                },
                AlignOp::Insertion {
                    reference_index: 1,
                    attempt_index: 1,
                    attempt: '水',
                },
                AlignOp::Insertion {
                    reference_index: 1,
                    attempt_index: 2,
                    attempt: '水',
                },
            ],
            matched_len: 1,
            reference_len: 3,
        });
        assert!(high_cer.is_rejected);
    }

    #[test]
    fn every_numeric_component_is_clamped_to_the_unit_interval() {
        let score = score_alignment(Alignment {
            ops: (0..8)
                .map(|attempt_index| AlignOp::Insertion {
                    reference_index: 0,
                    attempt_index,
                    attempt: '错',
                })
                .collect(),
            matched_len: 0,
            reference_len: 1,
        });

        for component in [
            score.completeness,
            score.accuracy_strict,
            score.accuracy_lenient,
            score.fluency,
        ] {
            assert!((0.0..=1.0).contains(&component));
        }
        assert_eq!(score.accuracy_strict, 0.0);
    }

    #[test]
    fn adding_deletions_never_increases_a_score_component() {
        let fixture = fixture();
        let reference_text = "床前明月光疑是地上霜";
        let reference = poem(&fixture.handle, reference_text);
        let characters = reference_text.chars().collect::<Vec<_>>();
        let mut previous = score_typed(&reference, &attempt(&fixture.handle, reference_text));

        for remaining in (0..characters.len()).rev() {
            let text = characters[..remaining].iter().collect::<String>();
            let current = score_typed(&reference, &attempt(&fixture.handle, &text));
            assert!(current.completeness <= previous.completeness);
            assert!(current.accuracy_strict <= previous.accuracy_strict);
            assert!(current.accuracy_lenient <= previous.accuracy_lenient);
            assert!(current.fluency <= previous.fluency);
            previous = current;
        }
    }

    #[test]
    fn voice_feedback_exposes_only_activity_pauses_and_relative_rhythm() {
        let feedback = VoicePracticeFeedback::new(true, 2, RelativeRhythm::Similar);

        assert!(feedback.spoke);
        assert_eq!(feedback.pause_count, 2);
        assert_eq!(feedback.relative_rhythm, RelativeRhythm::Similar);
    }

    #[test]
    fn source_guards_keep_voice_derived_text_out_of_typed_scoring() {
        let source = include_str!("score.rs");
        assert!(source.contains(
            "pub fn score_typed(reference: &Poem, attempt: &TypedAttempt) -> TypedScore"
        ));
        assert!(!source.contains(concat!("impl Into<", "TypedAttempt>")));

        let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(2)
            .expect("定位 workspace root");
        let crates = workspace.join("crates");
        let distributed_sources = rust_sources(&crates);
        let compact = distributed_sources
            .iter()
            .map(|(_, source)| source.split_whitespace().collect::<String>())
            .collect::<String>();
        for forbidden in [
            concat!("impl", "From<BiasedHyp>", "for", "TypedAttempt"),
            concat!("impl", "Into<TypedAttempt>", "for", "BiasedHyp"),
            concat!("impl", "Deref", "for", "BiasedHyp"),
            concat!("impl", "AsRef<TypedAttempt>", "for", "BiasedHyp"),
            concat!("impl", "From<VoicePracticeFeedback>", "for", "TypedScore"),
            concat!("impl", "From<TypedScore>", "for", "VoicePracticeFeedback"),
        ] {
            assert!(
                !compact.contains(forbidden),
                "发现被禁止的类型通路：{forbidden}"
            );
        }

        for relative in [
            "crates/yunjian-voice",
            "crates/yunjian-app",
            "crates/yunjian-mobile",
            "app",
        ] {
            let root = workspace.join(relative);
            for (path, source) in rust_sources(&root) {
                assert!(
                    !source.contains("TypedAttempt::new"),
                    "ASR/应用路径不得构造 TypedAttempt：{}",
                    path.display()
                );
            }
        }
    }

    fn rust_sources(root: &Path) -> Vec<(PathBuf, String)> {
        if !root.exists() {
            return Vec::new();
        }
        let mut pending = vec![root.to_path_buf()];
        let mut sources = Vec::new();
        while let Some(path) = pending.pop() {
            if path.is_dir() {
                for entry in std::fs::read_dir(&path).expect("读取源码目录") {
                    pending.push(entry.expect("读取源码条目").path());
                }
            } else if path.extension().is_some_and(|extension| extension == "rs") {
                let source = std::fs::read_to_string(&path).expect("读取 Rust 源码");
                sources.push((path, source));
            }
        }
        sources
    }
}
