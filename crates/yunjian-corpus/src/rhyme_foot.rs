use crate::ingest::corpus_error;
use crate::model::Genre;
use crate::quality::{Finding, ReasonCode};
use crate::rhyme::RhymeImport;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use yunjian_core::Result;
use yunjian_core::rhyme::{RhymeBook, RhymeTone};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RhymeConfidence {
    ResolvedByVote,
    Unambiguous,
    Unresolved,
}

impl RhymeConfidence {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ResolvedByVote => "resolved_by_vote",
            Self::Unambiguous => "unambiguous",
            Self::Unresolved => "unresolved",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct PoemRhymeGroupRow {
    pub poem_id: String,
    pub rhyme_book: RhymeBook,
    pub rhyme_group: String,
    pub tone: RhymeTone,
    pub confidence: RhymeConfidence,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PoemLastCharInput {
    pub poem_id: String,
    pub work_group: String,
    pub genre: Genre,
    pub line_index: usize,
    pub character: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RhymeFootOutput {
    pub rows: Vec<PoemRhymeGroupRow>,
    pub findings: Vec<Finding>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RhymeFootStats {
    pub rows_by_confidence: BTreeMap<RhymeConfidence, usize>,
    pub poems_by_confidence: BTreeMap<RhymeConfidence, usize>,
    pub analyzed_poems: usize,
    pub unresolved_poems: usize,
}

impl RhymeFootStats {
    pub fn unresolved_ratio(&self) -> f64 {
        if self.analyzed_poems == 0 {
            0.0
        } else {
            self.unresolved_poems as f64 / self.analyzed_poems as f64
        }
    }
}

impl RhymeFootOutput {
    pub fn stats(&self) -> RhymeFootStats {
        let mut stats = RhymeFootStats::default();
        let mut poem_confidences: BTreeMap<&str, BTreeSet<RhymeConfidence>> = BTreeMap::new();
        for row in &self.rows {
            *stats.rows_by_confidence.entry(row.confidence).or_default() += 1;
            poem_confidences
                .entry(&row.poem_id)
                .or_default()
                .insert(row.confidence);
        }
        let unresolved_ids = self
            .findings
            .iter()
            .filter(|finding| finding.reason_code == ReasonCode::RhymeUnresolved)
            .filter_map(|finding| finding.stable_id.as_deref())
            .collect::<BTreeSet<_>>();
        for poem_id in &unresolved_ids {
            poem_confidences
                .entry(poem_id)
                .or_default()
                .insert(RhymeConfidence::Unresolved);
        }
        for confidences in poem_confidences.values() {
            for confidence in confidences {
                *stats.poems_by_confidence.entry(*confidence).or_default() += 1;
            }
        }
        stats.analyzed_poems = poem_confidences.len();
        stats.unresolved_poems = unresolved_ids.len();
        stats
    }
}

struct FootCandidates<'a> {
    character: &'a str,
    candidates: BTreeSet<(String, RhymeTone)>,
}

pub fn derive(inputs: &[PoemLastCharInput], rhymes: &RhymeImport) -> Result<RhymeFootOutput> {
    let mut by_poem: BTreeMap<&str, Vec<&PoemLastCharInput>> = BTreeMap::new();
    for input in inputs {
        by_poem.entry(&input.poem_id).or_default().push(input);
    }

    let mut output = RhymeFootOutput::default();
    for poem_inputs in by_poem.values_mut() {
        poem_inputs.sort_by_key(|input| input.line_index);
        derive_poem(poem_inputs, rhymes, &mut output)?;
    }
    output.rows.sort();
    output.findings.sort();
    Ok(output)
}

fn derive_poem(
    inputs: &[&PoemLastCharInput],
    rhymes: &RhymeImport,
    output: &mut RhymeFootOutput,
) -> Result<()> {
    let first = inputs
        .first()
        .ok_or_else(|| corpus_error("韵脚推导收到空作品"))?;
    let Some(book) = rhyme_book_for(first.genre) else {
        return Ok(());
    };
    validate_poem_inputs(inputs, first)?;

    let index = rhymes.index(book)?;
    let feet = inputs
        .iter()
        .map(|input| FootCandidates {
            character: &input.character,
            candidates: index
                .lookup(&input.character)
                .iter()
                .map(|(tone, group)| (group.clone(), *tone))
                .collect(),
        })
        .collect::<Vec<_>>();
    let all_groups = feet
        .iter()
        .flat_map(|foot| foot.candidates.iter().map(|(group, _)| group.clone()))
        .collect::<BTreeSet<_>>();
    let votes = count_group_votes(&feet);
    let winners = leading_groups(&votes);
    let selected = if all_groups.len() == 1 {
        all_groups.first().cloned()
    } else if winners.len() == 1 && votes.get(&winners[0]).is_some_and(|count| *count >= 2) {
        Some(winners[0].clone())
    } else {
        None
    };

    match selected.as_deref() {
        Some(group) => {
            let confidence = if feet.iter().any(|foot| {
                let groups = foot
                    .candidates
                    .iter()
                    .map(|(candidate, _)| candidate)
                    .collect::<BTreeSet<_>>();
                groups.len() > 1 && groups.iter().any(|candidate| candidate.as_str() == group)
            }) {
                RhymeConfidence::ResolvedByVote
            } else {
                RhymeConfidence::Unambiguous
            };
            append_rows_for_groups(
                output,
                first,
                book,
                &feet,
                BTreeSet::from([group.to_owned()]),
                confidence,
            );
        }
        None => append_rows_for_groups(
            output,
            first,
            book,
            &feet,
            all_groups,
            RhymeConfidence::Unresolved,
        ),
    }

    let missing = feet
        .iter()
        .filter(|foot| foot.candidates.is_empty())
        .map(|foot| foot.character)
        .collect::<BTreeSet<_>>();
    if selected.is_none() || !missing.is_empty() {
        output.findings.push(Finding {
            stable_id: Some(first.poem_id.clone()),
            work_group: Some(first.work_group.clone()),
            reason_code: ReasonCode::RhymeUnresolved,
            detail: unresolved_detail(book, &votes, &missing),
            source: "rhyme-foot".to_owned(),
        });
    }
    Ok(())
}

fn rhyme_book_for(genre: Genre) -> Option<RhymeBook> {
    match genre {
        Genre::Shi => Some(RhymeBook::Pingshui),
        Genre::Ci => Some(RhymeBook::Cilin),
        Genre::Qu | Genre::Fu | Genre::Wen => None,
    }
}

fn validate_poem_inputs(inputs: &[&PoemLastCharInput], first: &PoemLastCharInput) -> Result<()> {
    let mut line_indexes = BTreeSet::new();
    for input in inputs {
        if input.genre != first.genre || input.work_group != first.work_group {
            return Err(corpus_error(format!(
                "作品 {} 的 poem_last_char 元数据不一致",
                first.poem_id
            )));
        }
        if !line_indexes.insert(input.line_index) {
            return Err(corpus_error(format!(
                "作品 {} 的 poem_last_char 出现重复 line_index {}",
                first.poem_id, input.line_index
            )));
        }
    }
    Ok(())
}

fn count_group_votes(feet: &[FootCandidates<'_>]) -> BTreeMap<String, usize> {
    let mut votes = BTreeMap::new();
    for foot in feet {
        let groups = foot
            .candidates
            .iter()
            .map(|(group, _)| group)
            .collect::<BTreeSet<_>>();
        if let Some(group) = (groups.len() == 1).then(|| *groups.first().expect("长度已检查"))
        {
            *votes.entry(group.clone()).or_default() += 1;
        }
    }
    votes
}

fn leading_groups(votes: &BTreeMap<String, usize>) -> Vec<String> {
    let Some(max_votes) = votes.values().copied().max() else {
        return Vec::new();
    };
    votes
        .iter()
        .filter(|(_, count)| **count == max_votes)
        .map(|(group, _)| group.clone())
        .collect()
}

fn append_rows_for_groups(
    output: &mut RhymeFootOutput,
    poem: &PoemLastCharInput,
    book: RhymeBook,
    feet: &[FootCandidates<'_>],
    groups: BTreeSet<String>,
    confidence: RhymeConfidence,
) {
    let candidates = feet
        .iter()
        .flat_map(|foot| foot.candidates.iter())
        .filter(|(group, _)| groups.contains(group))
        .cloned()
        .collect::<BTreeSet<_>>();
    output.rows.extend(
        candidates
            .into_iter()
            .map(|(rhyme_group, tone)| PoemRhymeGroupRow {
                poem_id: poem.poem_id.clone(),
                rhyme_book: book,
                rhyme_group,
                tone,
                confidence,
            }),
    );
}

fn unresolved_detail(
    book: RhymeBook,
    votes: &BTreeMap<String, usize>,
    missing: &BTreeSet<&str>,
) -> String {
    let vote_text = votes
        .iter()
        .map(|(group, count)| format!("{group}={count}"))
        .collect::<Vec<_>>()
        .join("、");
    let missing_text = missing.iter().copied().collect::<Vec<_>>().join("、");
    format!(
        "{}韵脚未能唯一消歧；候选票数 [{}]；韵书未收字 [{}]",
        book.display_name(),
        vote_text,
        missing_text
    )
}

#[cfg(test)]
mod tests;
