//! 在指定规模上从**真实上游检出**装配 [`CorpusDbInput`]。
//!
//! 复用 `yunjian-corpus` 的现成流水线，绝不在 xtask 里另写一套入库或质量判据：
//! 入库 → 铸造身份 → 繁简归一 → 质量分析走
//! [`run_pipeline`](yunjian_corpus::quality::run_pipeline)，韵脚表走
//! [`rhyme_foot::derive`](yunjian_corpus::rhyme_foot::derive)。测出来的体积必须是
//! **产物**的体积，所以建库路径也必须是产物的建库路径。

use std::collections::BTreeSet;
use std::path::Path;

use anyhow::{Context, Result, bail};
use yunjian_corpus::db::CorpusDbInput;
use yunjian_corpus::ingest::werneror::{Bucket, CLASSICAL_BUCKETS};
use yunjian_corpus::model::{CanonicalRecord, Dynasty};
use yunjian_corpus::normalize::Normalizer;
use yunjian_corpus::quality::{Disposition, run_pipeline};
use yunjian_corpus::rhyme::RhymeImport;
use yunjian_corpus::rhyme_foot::{self, PoemLastCharInput};

use super::{CORPUS_VERSION_FOR_BUILD, Scale};

pub(super) fn assemble(
    scale: Scale,
    chinese_poetry_dir: &Path,
    werneror_dir: &Path,
    rhymes: &RhymeImport,
    manifest_bytes: &[u8],
    verdict_bytes: &[u8],
) -> Result<CorpusDbInput> {
    for dir in [chinese_poetry_dir, werneror_dir] {
        if !dir.is_dir() {
            bail!(
                "上游检出目录不存在：{}。按 corpus/sources.toml 的锁定 revision 检出后重跑。",
                dir.display()
            );
        }
    }

    let buckets = resolve_buckets(scale)?;
    let outcome = run_pipeline(
        chinese_poetry_dir,
        werneror_dir,
        &buckets,
        Vec::new(),
        CORPUS_VERSION_FOR_BUILD,
    )
    .context("质量流水线失败")?;

    let mut records = outcome.shippable;
    if scale.tang_song_only() {
        records.retain(|record| matches!(record.dynasty, Dynasty::Tang | Dynasty::Song));
    }
    records.sort_by(|left, right| left.stable_id.cmp(&right.stable_id));
    if let Some(limit) = scale.truncate_to() {
        records.truncate(limit);
    }
    if records.is_empty() {
        bail!("规模 {} 筛出零条记录，无法建库", scale.key());
    }

    // 守恒是由 `build_database` 强制的：`shipped` 处置的 stable_id 集合必须与 poem
    // 集合逐一相等。本子命令按规模裁剪了记录，因此台账也要按同一个集合重算——
    // 把被裁掉的记录改判为 `excluded`，而不是把它们从台账里删掉（删掉会让
    // `dispositions.len() == input_rows` 失败，那条守恒正是为了让记录无处静默消失）。
    let kept: BTreeSet<&str> = records
        .iter()
        .map(|record| record.stable_id.as_str())
        .collect();
    let mut quality = outcome.report;
    for row in &mut quality.dispositions {
        if row.disposition == Disposition::Shipped {
            let still_kept = row.stable_id.as_deref().is_some_and(|id| kept.contains(id));
            if !still_kept {
                row.disposition = Disposition::Excluded;
            }
        }
    }
    let shipped = quality
        .dispositions
        .iter()
        .filter(|row| row.disposition == Disposition::Shipped)
        .count();
    let quarantined = quality
        .dispositions
        .iter()
        .filter(|row| row.disposition == Disposition::Quarantined)
        .count();
    quality.counts.shipped = shipped;
    quality.counts.quarantined = quarantined;
    quality.counts.excluded = quality.dispositions.len() - shipped - quarantined;
    quality.poem_count = shipped;
    // 缺陷可以指向被裁掉的记录，但 `check_cross_report_integrity` 要求每个非空
    // stable_id 都在台账里——台账保留了全部行，所以这一条自然成立，无需改 findings。
    quality.check_conservation()?;
    quality.check_cross_report_integrity()?;

    let normalizer = Normalizer::new()?;
    let normalized = normalizer.normalize(&records)?;
    let poem_rhyme_groups = derive_rhyme_groups(&records, &normalized.records, rhymes)?;

    Ok(CorpusDbInput {
        shipped_scope: scale.shipped_scope(),
        corpus_version: CORPUS_VERSION_FOR_BUILD.to_owned(),
        source_manifest: manifest_bytes.to_vec(),
        index_verdict: verdict_bytes.to_vec(),
        records,
        normalized_records: normalized.records,
        commentaries: Vec::new(),
        rhymes: shippable_rhyme_entries(rhymes),
        poem_rhyme_groups,
        variants: normalized.variant_map.rows(),
        tags: Vec::new(),
        quality,
    })
}

/// 随包的韵书行：平水韵与词林正韵。
///
/// 中华新韵未随包（[`RhymeBook::ensure_available`] 拦在前面），所以这里只取两本，
/// 而不是遍历 `RhymeBook::ALL`——后者会在新韵上拿到错误。
fn shippable_rhyme_entries(rhymes: &RhymeImport) -> Vec<yunjian_corpus::rhyme::RhymeEntry> {
    let mut entries = rhymes.pingshui.entries().to_vec();
    entries.extend_from_slice(rhymes.cilin.entries());
    entries
}

fn resolve_buckets(scale: Scale) -> Result<Vec<Bucket>> {
    let wanted = scale.werneror_buckets();
    if wanted.is_empty() {
        return Ok(CLASSICAL_BUCKETS.to_vec());
    }
    wanted
        .iter()
        .map(|file| {
            CLASSICAL_BUCKETS
                .iter()
                .find(|bucket| bucket.file == *file)
                .copied()
                .with_context(|| format!("古典白名单里没有分桶 {file}"))
        })
        .collect()
}

fn derive_rhyme_groups(
    records: &[CanonicalRecord],
    normalized: &[yunjian_corpus::normalize::NormalizedRecord],
    rhymes: &RhymeImport,
) -> Result<Vec<yunjian_corpus::db::PoemRhymeGroupRow>> {
    let mut inputs = Vec::new();
    for (record, norm) in records.iter().zip(normalized.iter()) {
        for (line_index, line) in norm.body_lines.iter().enumerate() {
            let Some(character) = last_character(line) else {
                continue;
            };
            inputs.push(PoemLastCharInput {
                poem_id: record.stable_id.clone(),
                work_group: record.work_group.clone(),
                genre: record.genre,
                line_index,
                character: character.to_string(),
            });
        }
    }
    Ok(rhyme_foot::derive(&inputs, rhymes)?.rows)
}

fn last_character(line: &str) -> Option<char> {
    line.chars()
        .rev()
        .find(|character| !character.is_whitespace() && !is_punctuation(*character))
}

fn is_punctuation(character: char) -> bool {
    character.is_ascii_punctuation()
        || matches!(
            character,
            '，' | '。'
                | '、'
                | '；'
                | '：'
                | '？'
                | '！'
                | '“'
                | '”'
                | '‘'
                | '’'
                | '（'
                | '）'
                | '《'
                | '》'
                | '〈'
                | '〉'
                | '【'
                | '】'
                | '〔'
                | '〕'
                | '—'
                | '…'
                | '·'
        )
}
