//! 在指定规模上从**真实上游检出**装配 [`CorpusDbInput`]。
//!
//! 复用 `yunjian-corpus` 的现成流水线，绝不在 xtask 里另写一套入库或质量判据：
//! 入库 → 铸造身份 → 繁简归一 → 质量分析走
//! [`run_pipeline`](yunjian_corpus::quality::run_pipeline)，韵脚表走
//! [`rhyme_foot::derive`](yunjian_corpus::rhyme_foot::derive)。测出来的体积必须是
//! **产物**的体积，所以建库路径也必须是产物的建库路径。

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use anyhow::{Context, Result, bail};
use yunjian_corpus::db::CorpusDbInput;
use yunjian_corpus::ingest::werneror::{
    Bucket, CLASSICAL_BUCKETS, FIXTURE_BUCKETS, buckets_by_file,
};
use yunjian_corpus::model::{CanonicalRecord, Dynasty};
use yunjian_corpus::normalize::Normalizer;
use yunjian_corpus::quality::{Disposition, ReasonCode, run_pipeline};
use yunjian_corpus::rhyme::RhymeImport;
use yunjian_corpus::rhyme_foot::{self, PoemLastCharInput};
use yunjian_corpus::tag::{TagVocabulary, assign_tags};
use yunjian_corpus::{commentary, db::PoemTagRow};

use super::{
    CORPUS_VERSION_FOR_BUILD, InputSource, RhymeConfidenceMeasurement, Scale,
    StructuralCleaningMeasurement,
};

/// 集评种子集在仓库里的位置。与 `xtask commentary-index` 读的是同一个目录。
const COMMENTARY_DIR: &str = "corpus/commentary";

pub(super) fn assemble(
    scale: Scale,
    source: InputSource,
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

    let buckets = resolve_buckets(scale, source)?;
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
    let tags = assemble_tags(&records, &normalized.records, &normalizer)?;
    let commentaries = assemble_commentaries(&records)?;

    Ok(CorpusDbInput {
        shipped_scope: scale.shipped_scope(),
        corpus_version: CORPUS_VERSION_FOR_BUILD.to_owned(),
        source_manifest: manifest_bytes.to_vec(),
        index_verdict: verdict_bytes.to_vec(),
        records,
        normalized_records: normalized.records,
        commentaries,
        rhymes: shippable_rhyme_entries(rhymes),
        poem_rhyme_groups,
        variants: normalized.variant_map.rows(),
        tags,
        quality,
    })
}

/// 按签入的策展词表打标。
///
/// 词表是唯一的标签来源，规则与评审名单都在 `crates/yunjian-corpus/tags.toml` 里。
/// 这里不做任何补充判断——xtask 里再加一条规则就等于让标签有两个事实来源。
fn assemble_tags(
    records: &[CanonicalRecord],
    normalized: &[yunjian_corpus::normalize::NormalizedRecord],
    normalizer: &Normalizer,
) -> Result<Vec<PoemTagRow>> {
    let vocabulary = TagVocabulary::shipped().context("解析签入的标签词表")?;
    let assignment =
        assign_tags(&vocabulary, records, normalized, normalizer).context("构建期打标失败")?;
    tracing::info!(
        rows = assignment.report.rows,
        poems = assignment.report.tagged_poems,
        tags = assignment.report.per_tag.len(),
        "已按策展词表打标"
    );
    Ok(assignment.rows)
}

/// 装载并解析历代集评。
///
/// **结构与出处的缺陷是致命的，诗篇定不到一首是不致命的。** 前者（缺出处、非前现代著作、
/// 现代体裁标记……）与本次构建选了哪个规模无关，是我们自己维护的种子集出了问题；后者
/// 在抽样规模上必然大量出现——一条评宋词的集评在只含唐诗的规模里当然找不到诗。把两者
/// 混为一谈会导致抽样构建全部失败，或者更糟：为了让抽样跑通而把出处缺陷也放过。
fn assemble_commentaries(records: &[CanonicalRecord]) -> Result<Vec<commentary::CommentaryRecord>> {
    let dir = crate::index_spike::repo_root()?.join(COMMENTARY_DIR);
    let seeds = commentary::load_seeds(&dir)
        .with_context(|| format!("读取集评种子集 {}", dir.display()))?;
    let outcome = commentary::ingest(&seeds, records).context("集评入库失败")?;

    let (unresolved, defects): (Vec<_>, Vec<_>) = outcome
        .rejections
        .iter()
        .partition(|rejection| is_poem_resolution_failure(rejection.reason));
    let ambiguous = unresolved
        .iter()
        .filter(|rejection| rejection.reason == commentary::RejectionReason::PoemAmbiguous)
        .collect::<Vec<_>>();
    if !ambiguous.is_empty() {
        // 歧义条目走 `emit`（终端报告）而**不是** tracing：`xtask` 没装 subscriber，
        // 同文件里那几条 `tracing::info!` 实际上一行都不会出现。而这几条必须被看见——
        // 它们与「评宋词的集评在唐诗规模里找不到诗」不同：那一类随规模自愈，这一类是
        // 真实的上游重出，随包规模上就存在，每一条都意味着一条已转录的集评没有随包。
        crate::verify_sources::emit(&format!(
            "集评有 {} 条定不到唯一诗篇，本次不随包：{}",
            ambiguous.len(),
            ambiguous
                .iter()
                .map(|rejection| rejection.entry_id.as_str())
                .collect::<Vec<_>>()
                .join("、")
        ));
    }
    if !defects.is_empty() {
        let detail = defects
            .iter()
            .map(|rejection| {
                format!(
                    "{}（{}）：{}",
                    rejection.entry_id,
                    rejection.reason.as_key(),
                    rejection.detail
                )
            })
            .collect::<Vec<_>>()
            .join("；");
        bail!("集评种子集有 {} 条结构或出处缺陷：{detail}", defects.len());
    }
    tracing::info!(
        accepted = outcome.records.len(),
        out_of_scope = unresolved.len() - ambiguous.len(),
        ambiguous = ambiguous.len(),
        seeds = seeds.len(),
        "已关联历代集评"
    );
    Ok(outcome.records)
}

/// 这条拒绝是「定不到那首诗」而不是「种子集自己坏了」吗？
///
/// 两种取值都是解析结果而非种子缺陷，所以都不该中止构建：
///
/// - `PoemUnresolved`：本次规模里没有那首诗。抽样规模上必然大量出现。
/// - `PoemAmbiguous`：同作者同题同起句匹配到多首而正文互不相同，
///   [`resolve`](commentary::PoemIndex::resolve) 按设计拒绝猜。随包唐宋规模上实测 6 条，
///   全部是**同一首词的两个上游版本差一个字**（如「步转迥廊」与「步转回廊」），
///   `work_group` 与 `edition_group` 都由正文算出，因此都无法把它们并成一组。
///
/// 把后者算成致命缺陷的直接后果是：随包工件永远建不出来，`commentary` 表永远是 0 行——
/// 这正是 08-11 那份工件三张表全空的成因之一。
fn is_poem_resolution_failure(reason: commentary::RejectionReason) -> bool {
    matches!(
        reason,
        commentary::RejectionReason::PoemUnresolved | commentary::RejectionReason::PoemAmbiguous
    )
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

/// 本次要读的分桶。
///
/// 规模声明的是**真实上游**上的分桶清单；fixture 目录只提供其中的一小部分
/// （[`FIXTURE_BUCKETS`]，其余在锁定 revision 上动辄十几 MB，刻意不签入）。所以对
/// fixture 输入必须求交集——不裁剪就会去要一个从来没打算存在的文件，把输入选择
/// 错误伪装成数据缺失。裁剪只会缩小「读哪些」，白名单仍然是唯一的许可判据。
fn resolve_buckets(scale: Scale, source: InputSource) -> Result<Vec<Bucket>> {
    let declared: Vec<&str> = match scale.werneror_buckets() {
        [] => CLASSICAL_BUCKETS.iter().map(|bucket| bucket.file).collect(),
        wanted => wanted.to_vec(),
    };
    let wanted: Vec<&str> = match source {
        InputSource::Upstream => declared,
        InputSource::Fixture => declared
            .into_iter()
            .filter(|file| FIXTURE_BUCKETS.contains(file))
            .collect(),
    };
    if wanted.is_empty() {
        bail!(
            "规模 {} 在 {} 上没有任何可读分桶",
            scale.key(),
            source.label()
        );
    }
    buckets_by_file(&wanted).map_err(Into::into)
}

fn derive_rhyme_groups(
    records: &[CanonicalRecord],
    normalized: &[yunjian_corpus::normalize::NormalizedRecord],
    rhymes: &RhymeImport,
) -> Result<Vec<yunjian_corpus::db::PoemRhymeGroupRow>> {
    let mut inputs = Vec::new();
    for (record, norm) in records.iter().zip(normalized.iter()) {
        for (line_index, line) in yunjian_core::split_rhyme_feet(&norm.body).enumerate() {
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

pub(super) fn measure_structural_cleaning(
    input: &CorpusDbInput,
    rhymes: &RhymeImport,
) -> Result<StructuralCleaningMeasurement> {
    let mut forms = BTreeMap::new();
    let mut is_yuefu = 0;
    for record in &input.records {
        let classification = yunjian_corpus::form::classify(record)?;
        *forms
            .entry(classification.form.as_str().to_owned())
            .or_insert(0) += 1;
        is_yuefu += usize::from(classification.is_yuefu);
    }

    Ok(StructuralCleaningMeasurement {
        quality_scope_input_rows: input.quality.input_rows,
        structure_scope_poems: input.records.len(),
        placeholder_body: input.quality.finding_count(ReasonCode::PlaceholderBody),
        glued_lines: input.quality.finding_count(ReasonCode::GluedLines),
        forms,
        is_yuefu,
        rhyme_before: measure_rhyme_confidence(input, rhymes, true)?,
        rhyme_after: measure_rhyme_confidence(input, rhymes, false)?,
    })
}

fn measure_rhyme_confidence(
    input: &CorpusDbInput,
    rhymes: &RhymeImport,
    legacy_separators: bool,
) -> Result<RhymeConfidenceMeasurement> {
    let normalized = input
        .normalized_records
        .iter()
        .map(|record| (record.stable_id.as_str(), record))
        .collect::<BTreeMap<_, _>>();
    let mut feet = Vec::new();
    for record in &input.records {
        let norm = normalized
            .get(record.stable_id.as_str())
            .with_context(|| format!("缺少归一记录：{}", record.stable_id))?;
        let fragments = if legacy_separators {
            norm.body
                .split(['\n', '，', '。', '！', '？', '；'])
                .collect::<Vec<_>>()
        } else {
            yunjian_core::split_rhyme_feet(&norm.body).collect::<Vec<_>>()
        };
        for (line_index, line) in fragments.into_iter().enumerate() {
            let Some(character) = last_character(line) else {
                continue;
            };
            feet.push(PoemLastCharInput {
                poem_id: record.stable_id.clone(),
                work_group: record.work_group.clone(),
                genre: record.genre,
                line_index,
                character: character.to_string(),
            });
        }
    }
    let output = rhyme_foot::derive(&feet, rhymes)?;
    let stats = output.stats();
    let unresolved_ratio = stats.unresolved_ratio();
    Ok(RhymeConfidenceMeasurement {
        rows: stats
            .rows_by_confidence
            .into_iter()
            .map(|(confidence, count)| (confidence.as_str().to_owned(), count))
            .collect(),
        poems: stats
            .poems_by_confidence
            .into_iter()
            .map(|(confidence, count)| (confidence.as_str().to_owned(), count))
            .collect(),
        analyzed_poems: stats.analyzed_poems,
        unresolved_poems: stats.unresolved_poems,
        unresolved_ratio,
    })
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
