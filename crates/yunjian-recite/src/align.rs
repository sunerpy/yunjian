//! Unicode 字级背诵对齐。

use yunjian_core::{CorpusHandle, Result, content_chars, normalize_query};

const MIN_RERECITATION_CHARS: usize = 2;

/// 对齐得到的一项朗读行为。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AlignOp {
    /// 参考字与尝试字一致。
    Normal {
        /// 归一化参考文本中的字符位置。
        reference_index: usize,
        /// 归一化尝试文本中的字符位置。
        attempt_index: usize,
        /// 匹配的字符。
        character: char,
    },
    /// 参考文本中的字符没有读出。
    Deletion {
        /// 归一化参考文本中的字符位置。
        reference_index: usize,
        /// 未读出的参考字符。
        reference: char,
    },
    /// 尝试文本多读了一个字符。
    Insertion {
        /// 插入点对应的归一化参考文本位置。
        reference_index: usize,
        /// 归一化尝试文本中的字符位置。
        attempt_index: usize,
        /// 多读出的字符。
        attempt: char,
    },
    /// 回到已经正确读过的参考区间并连续重读。
    ReRecitation {
        /// 被重读区间在归一化参考文本中的起点。
        reference_start: usize,
        /// 被重读区间在归一化参考文本中的开区间终点。
        reference_end: usize,
        /// 重读区间在归一化尝试文本中的起点。
        attempt_start: usize,
        /// 重读区间在归一化尝试文本中的开区间终点。
        attempt_end: usize,
        /// 重读的归一化文本。
        text: String,
    },
    /// 参考字被读成另一个字符。
    Substitution {
        /// 归一化参考文本中的字符位置。
        reference_index: usize,
        /// 归一化尝试文本中的字符位置。
        attempt_index: usize,
        /// 应读字符。
        reference: char,
        /// 实际字符。
        attempt: char,
    },
}

/// 一次完整字级对齐的结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Alignment {
    /// 按尝试过程排列的对齐项。
    pub ops: Vec<AlignOp>,
    /// 正常匹配的参考字符数；回读不重复计入。
    pub matched_len: usize,
    /// 去除标点、空白并应用 `variant_map` 后的参考字符数。
    pub reference_len: usize,
}

/// 对齐参考诗文和一次背诵尝试。
///
/// 两侧文本均先去掉标点和空白，再使用语料库随包的 `variant_map` 做字形改写；
/// 数字等字符保持原样，不执行 inverse text normalization。
pub fn align(handle: &CorpusHandle, reference: &str, attempt: &str) -> Result<Alignment> {
    let reference = normalize(handle, reference)?;
    let attempt = normalize(handle, attempt)?;
    Ok(align_chars(&reference, &attempt))
}

pub(crate) fn normalize_text(handle: &CorpusHandle, text: &str) -> Result<String> {
    Ok(normalize(handle, text)?.into_iter().collect())
}

pub(crate) fn align_normalized(reference: &str, attempt: &str) -> Alignment {
    let reference = reference.chars().collect::<Vec<_>>();
    let attempt = attempt.chars().collect::<Vec<_>>();
    align_chars(&reference, &attempt)
}

fn align_chars(reference: &[char], attempt: &[char]) -> Alignment {
    let raw_ops = edit_script(reference, attempt);
    let ops = detect_rerecitations(raw_ops, reference);
    let matched_len = ops
        .iter()
        .filter(|op| matches!(op, AlignOp::Normal { .. }))
        .count();
    Alignment {
        ops,
        matched_len,
        reference_len: reference.len(),
    }
}

fn normalize(handle: &CorpusHandle, text: &str) -> Result<Vec<char>> {
    let content = content_chars(text).collect::<String>();
    Ok(normalize_query(handle, &content)?.chars().collect())
}

fn edit_script(reference: &[char], attempt: &[char]) -> Vec<AlignOp> {
    let columns = attempt.len() + 1;
    let mut costs = vec![0_usize; (reference.len() + 1) * columns];
    for reference_index in 0..=reference.len() {
        costs[reference_index * columns] = reference_index;
    }
    for (attempt_index, cost) in costs.iter_mut().take(attempt.len() + 1).enumerate() {
        *cost = attempt_index;
    }
    for reference_index in 1..=reference.len() {
        for attempt_index in 1..=attempt.len() {
            let diagonal = costs[(reference_index - 1) * columns + attempt_index - 1]
                + usize::from(reference[reference_index - 1] != attempt[attempt_index - 1]);
            let deletion = costs[(reference_index - 1) * columns + attempt_index] + 1;
            let insertion = costs[reference_index * columns + attempt_index - 1] + 1;
            costs[reference_index * columns + attempt_index] =
                diagonal.min(deletion).min(insertion);
        }
    }

    let mut reference_index = reference.len();
    let mut attempt_index = attempt.len();
    let mut reversed = Vec::with_capacity(reference.len().max(attempt.len()));
    while reference_index > 0 || attempt_index > 0 {
        let current = costs[reference_index * columns + attempt_index];
        if reference_index > 0 && attempt_index > 0 {
            let reference_char = reference[reference_index - 1];
            let attempt_char = attempt[attempt_index - 1];
            let diagonal = costs[(reference_index - 1) * columns + attempt_index - 1];
            if reference_char != attempt_char && current == diagonal + 1 {
                reversed.push(AlignOp::Substitution {
                    reference_index: reference_index - 1,
                    attempt_index: attempt_index - 1,
                    reference: reference_char,
                    attempt: attempt_char,
                });
                reference_index -= 1;
                attempt_index -= 1;
                continue;
            }
        }
        if attempt_index > 0 && current == costs[reference_index * columns + attempt_index - 1] + 1
        {
            reversed.push(AlignOp::Insertion {
                reference_index,
                attempt_index: attempt_index - 1,
                attempt: attempt[attempt_index - 1],
            });
            attempt_index -= 1;
            continue;
        }
        if reference_index > 0 && attempt_index > 0 {
            let reference_char = reference[reference_index - 1];
            let attempt_char = attempt[attempt_index - 1];
            if reference_char == attempt_char
                && current == costs[(reference_index - 1) * columns + attempt_index - 1]
            {
                reversed.push(AlignOp::Normal {
                    reference_index: reference_index - 1,
                    attempt_index: attempt_index - 1,
                    character: reference_char,
                });
                reference_index -= 1;
                attempt_index -= 1;
                continue;
            }
        }
        reversed.push(AlignOp::Deletion {
            reference_index: reference_index - 1,
            reference: reference[reference_index - 1],
        });
        reference_index -= 1;
    }
    reversed.reverse();
    reversed
}

fn detect_rerecitations(raw_ops: Vec<AlignOp>, reference: &[char]) -> Vec<AlignOp> {
    let mut ops = Vec::with_capacity(raw_ops.len());
    let mut matched_reference = vec![false; reference.len()];
    let mut cursor = 0;
    while cursor < raw_ops.len() {
        match &raw_ops[cursor] {
            AlignOp::Normal {
                reference_index, ..
            } => {
                matched_reference[*reference_index] = true;
                ops.push(raw_ops[cursor].clone());
                cursor += 1;
            }
            AlignOp::Insertion {
                reference_index,
                attempt_index,
                ..
            } => {
                let insertion_reference_index = *reference_index;
                let first_attempt_index = *attempt_index;
                let mut end = cursor + 1;
                while let Some(AlignOp::Insertion {
                    reference_index,
                    attempt_index,
                    ..
                }) = raw_ops.get(end)
                {
                    if *reference_index != insertion_reference_index
                        || *attempt_index != first_attempt_index + end - cursor
                    {
                        break;
                    }
                    end += 1;
                }
                fold_insertion_run(
                    &raw_ops[cursor..end],
                    reference,
                    &matched_reference,
                    insertion_reference_index,
                    &mut ops,
                );
                cursor = end;
            }
            _ => {
                ops.push(raw_ops[cursor].clone());
                cursor += 1;
            }
        }
    }
    ops
}

fn fold_insertion_run(
    run: &[AlignOp],
    reference: &[char],
    matched_reference: &[bool],
    reference_limit: usize,
    output: &mut Vec<AlignOp>,
) {
    let inserted = run
        .iter()
        .filter_map(|op| match op {
            AlignOp::Insertion { attempt, .. } => Some(*attempt),
            _ => None,
        })
        .collect::<Vec<_>>();
    let limit = reference_limit.min(reference.len());
    let columns = limit + 1;
    let mut common_prefix = vec![0_usize; (inserted.len() + 1) * columns];
    for inserted_index in (0..inserted.len()).rev() {
        for reference_index in (0..limit).rev() {
            if matched_reference[reference_index]
                && inserted[inserted_index] == reference[reference_index]
            {
                common_prefix[inserted_index * columns + reference_index] =
                    1 + common_prefix[(inserted_index + 1) * columns + reference_index + 1];
            }
        }
    }

    let mut inserted_index = 0;
    while inserted_index < inserted.len() {
        let best = (0..limit)
            .map(|reference_start| {
                (
                    reference_start,
                    common_prefix[inserted_index * columns + reference_start],
                )
            })
            .filter(|(_, length)| *length >= MIN_RERECITATION_CHARS)
            .max_by_key(|(reference_start, length)| (*length, *reference_start));
        if let Some((reference_start, length)) = best {
            let attempt_start = match &run[inserted_index] {
                AlignOp::Insertion { attempt_index, .. } => *attempt_index,
                _ => unreachable!("插入区间只含 insertion"),
            };
            output.push(AlignOp::ReRecitation {
                reference_start,
                reference_end: reference_start + length,
                attempt_start,
                attempt_end: attempt_start + length,
                text: inserted[inserted_index..inserted_index + length]
                    .iter()
                    .collect(),
            });
            inserted_index += length;
        } else {
            output.push(run[inserted_index].clone());
            inserted_index += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::{Connection, params};
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};
    use yunjian_core::{CorpusConfig, SCHEMA_VERSION};

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
            "yunjian-recite-align-{}-{}",
            std::process::id(),
            NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("创建对齐 fixture 目录");
        let path = dir.join("corpus.db");
        write_fixture(&path);
        let handle = CorpusHandle::open(&CorpusConfig {
            path: Some(path),
            data_dir: dir.clone(),
            archive: None,
        })
        .expect("打开对齐 fixture");
        Fixture { dir, handle }
    }

    fn write_fixture(path: &Path) {
        let connection = Connection::open(path).expect("创建对齐 fixture 数据库");
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
            .expect("创建对齐 fixture schema");
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
        connection.close().expect("关闭对齐 fixture 数据库");
    }

    #[test]
    fn identical_text_is_all_normal_after_normalization() {
        let fixture = fixture();
        let alignment = align(
            &fixture.handle,
            "國破山河在，城春草木深。",
            "国破山河在\n城春草木深",
        )
        .expect("对齐相同文本");

        assert_eq!(alignment.reference_len, 10);
        assert_eq!(alignment.matched_len, 10);
        assert_eq!(alignment.ops.len(), 10);
        assert!(
            alignment
                .ops
                .iter()
                .all(|op| matches!(op, AlignOp::Normal { .. }))
        );
    }

    #[test]
    fn missing_line_is_exactly_deletions_for_that_line() {
        let fixture = fixture();
        let alignment = align(
            &fixture.handle,
            "床前明月光，疑是地上霜。举头望明月，低头思故乡。",
            "床前明月光，疑是地上霜。低头思故乡。",
        )
        .expect("对齐漏读行");

        let deleted = alignment
            .ops
            .iter()
            .filter_map(|op| match op {
                AlignOp::Deletion { reference, .. } => Some(*reference),
                _ => None,
            })
            .collect::<String>();
        assert_eq!(deleted, "举头望明月");
        assert_eq!(alignment.matched_len, 15);
    }

    #[test]
    fn extra_character_is_one_insertion() {
        let fixture = fixture();
        let alignment = align(&fixture.handle, "床前明月光", "床前明明月光").expect("对齐增读");

        assert_eq!(
            alignment
                .ops
                .iter()
                .filter(|op| matches!(op, AlignOp::Insertion { .. }))
                .count(),
            1
        );
        assert_eq!(alignment.matched_len, 5);
    }

    #[test]
    fn wrong_character_is_one_substitution() {
        let fixture = fixture();
        let alignment = align(&fixture.handle, "床前明月光", "床前明月霜").expect("对齐替换");

        assert_eq!(
            alignment
                .ops
                .iter()
                .filter(|op| matches!(op, AlignOp::Substitution { .. }))
                .count(),
            1
        );
        assert_eq!(alignment.matched_len, 4);
    }

    #[test]
    fn backing_up_four_characters_is_one_rerecitation() {
        let fixture = fixture();
        let alignment = align(
            &fixture.handle,
            "床前明月光疑是地上霜",
            "床前明月床前明月光疑是地上霜",
        )
        .expect("对齐回读");

        let rereads = alignment
            .ops
            .iter()
            .filter_map(|op| match op {
                AlignOp::ReRecitation { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(rereads, ["床前明月"]);
        assert_eq!(
            alignment
                .ops
                .iter()
                .filter(|op| matches!(op, AlignOp::Insertion { .. } | AlignOp::Deletion { .. }))
                .count(),
            0
        );
        assert_eq!(alignment.matched_len, 10);
    }

    #[test]
    fn inverse_text_normalization_stays_off() {
        let fixture = fixture();
        let alignment = align(&fixture.handle, "一", "1").expect("对齐中文数字与阿拉伯数字");

        assert!(matches!(
            alignment.ops.as_slice(),
            [AlignOp::Substitution {
                reference: '一',
                attempt: '1',
                ..
            }]
        ));
        assert_eq!(alignment.matched_len, 0);
    }
}
