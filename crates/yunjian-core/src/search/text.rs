use crate::{CorpusHandle, Error, QueryPlan, Result, normalize_query, plan_query};
use rusqlite::{Connection, params_from_iter};
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;

/// 单页正文检索允许返回的最大命中数。
pub const TEXT_SEARCH_HARD_CAP: usize = 100;

/// 正文或残句检索请求。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextSearchRequest {
    /// 用户输入的正文或残句。
    pub query: String,
    /// 请求的单页数量；执行时受 [`TEXT_SEARCH_HARD_CAP`] 限制。
    pub limit: usize,
    /// 上一页返回的续页游标。
    pub cursor: Option<String>,
}

impl TextSearchRequest {
    /// 使用默认单页数量创建请求。
    #[must_use]
    pub fn new(query: impl Into<String>) -> Self {
        Self {
            query: query.into(),
            limit: 20,
            cursor: None,
        }
    }
}

/// 高亮范围，使用 Unicode 字符下标而不是 UTF-8 字节下标。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HighlightRange {
    /// 高亮起始字符下标，包含该位置。
    pub start: usize,
    /// 高亮结束字符下标，不包含该位置。
    pub end: usize,
}

/// 一段文本及其字符级高亮范围。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HighlightedSnippet {
    /// 用于展示的命中句。
    pub text: String,
    /// 按字符下标表示的高亮范围。
    pub highlights: Vec<HighlightRange>,
}

/// 一条正文检索命中。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextSearchHit {
    /// 作品稳定标识。
    pub poem_id: String,
    /// 作品题目。
    pub title: String,
    /// 作者名。
    pub author: String,
    /// 朝代规范键。
    pub dynasty: String,
    /// 最佳命中句在作品中的零基序号。
    pub matched_line_index: usize,
    /// 最佳命中句及高亮范围。
    pub snippet: HighlightedSnippet,
}

/// 一页正文检索结果。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchPage {
    /// 本页命中。
    pub hits: Vec<TextSearchHit>,
    /// 当前查询的总命中估计数。
    pub total_estimate: usize,
    /// 续页游标；`None` 表示已到末页。
    pub next_cursor: Option<String>,
    /// 实际执行的物理查询计划。
    pub plan_used: QueryPlan,
}

#[derive(Debug)]
struct Candidate {
    poem_id: String,
    title: String,
    author: String,
    dynasty: String,
    body: String,
    anthology: bool,
}

#[derive(Debug)]
struct RankedHit {
    hit: TextSearchHit,
    exact_line: bool,
    line_initial: bool,
    position: usize,
    prominence: u16,
}

#[derive(Debug, Serialize, Deserialize)]
struct TextCursor {
    query: String,
    offset: usize,
}

#[derive(Debug)]
struct LineMatch {
    line_index: usize,
    text: String,
    start: usize,
    end: usize,
    highlights: Vec<HighlightRange>,
}

impl CorpusHandle {
    /// 按长度敏感的查询计划检索正文并稳定排序。
    pub fn search_text(&self, request: TextSearchRequest) -> Result<SearchPage> {
        if request.limit == 0 {
            return Err(Error::Search("正文检索 limit 必须大于 0".to_owned()));
        }
        let normalized = normalize_query(self, &request.query)?;
        let plan = plan_query(self, &request.query)?;
        let offset = decode_cursor(request.cursor.as_deref(), &normalized)?;
        if matches!(plan, QueryPlan::Empty) {
            if offset != 0 {
                return Err(Error::Search("空查询的 cursor 偏移必须为 0".to_owned()));
            }
            return Ok(SearchPage {
                hits: Vec::new(),
                total_estimate: 0,
                next_cursor: None,
                plan_used: plan,
            });
        }

        let connection = self.connect()?;
        let mut ranked = load_candidates(&connection, &plan)?
            .into_iter()
            .filter_map(|candidate| rank_candidate(candidate, &normalized))
            .collect::<Vec<_>>();
        ranked.sort_by(compare_ranked_hits);
        let total_estimate = ranked.len();
        if offset > total_estimate {
            return Err(Error::Search(format!(
                "正文检索 cursor 偏移 {offset} 超出结果总数 {total_estimate}"
            )));
        }

        let limit = request.limit.min(TEXT_SEARCH_HARD_CAP);
        let end = offset.saturating_add(limit).min(total_estimate);
        let hits = ranked.drain(offset..end).map(|ranked| ranked.hit).collect();
        let next_cursor = (end < total_estimate)
            .then(|| {
                encode_cursor(&TextCursor {
                    query: normalized,
                    offset: end,
                })
            })
            .transpose()?;
        Ok(SearchPage {
            hits,
            total_estimate,
            next_cursor,
            plan_used: plan,
        })
    }
}

fn load_candidates(connection: &Connection, plan: &QueryPlan) -> Result<Vec<Candidate>> {
    const COLUMNS: &str = "p.stable_id, p.title, p.author, p.dynasty, p.body, EXISTS(SELECT 1 FROM poem_tag AS pt WHERE pt.poem_id = p.stable_id AND pt.tag IN ('唐诗三百首', '宋词三百首', '千家诗', '古诗文名篇'))";
    let (sql, bindings) = match plan {
        QueryPlan::NgramCandidates { gram, like_pattern } => (
            format!(
                "SELECT {COLUMNS} FROM ngram AS n JOIN poem AS p ON p.stable_id = n.stable_id WHERE n.gram = ?1 AND p.body LIKE ?2"
            ),
            vec![gram.clone(), like_pattern.clone()],
        ),
        QueryPlan::Match { expression } => (
            format!(
                "SELECT {COLUMNS} FROM poem_fts AS f JOIN poem AS p ON p.rowid = f.rowid WHERE poem_fts MATCH ?1"
            ),
            vec![expression.clone()],
        ),
        QueryPlan::Like { pattern } => (
            format!(
                "SELECT {COLUMNS} FROM poem_fts AS f JOIN poem AS p ON p.rowid = f.rowid WHERE f.body LIKE ?1"
            ),
            vec![pattern.clone()],
        ),
        QueryPlan::FullScan { pattern, .. } => (
            format!("SELECT {COLUMNS} FROM poem AS p WHERE p.body LIKE ?1"),
            vec![pattern.clone()],
        ),
        QueryPlan::Empty => return Ok(Vec::new()),
        QueryPlan::Meta { .. } => {
            return Err(Error::Search("正文检索不能执行元数据查询计划".to_owned()));
        }
    };
    let mut statement = connection.prepare(&sql)?;
    let candidates = statement
        .query_map(params_from_iter(bindings), |row| {
            Ok(Candidate {
                poem_id: row.get(0)?,
                title: row.get(1)?,
                author: row.get(2)?,
                dynasty: row.get(3)?,
                body: row.get(4)?,
                anthology: row.get(5)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(candidates)
}

fn rank_candidate(candidate: Candidate, query: &str) -> Option<RankedHit> {
    let matched = best_line_match(&candidate.body, query)?;
    let prominence = static_prominence(&candidate.author, candidate.anthology);
    Some(RankedHit {
        exact_line: matched.start == 0 && matched.end == matched.text.chars().count(),
        line_initial: matched.start == 0,
        position: matched.start,
        prominence,
        hit: TextSearchHit {
            poem_id: candidate.poem_id,
            title: candidate.title,
            author: candidate.author,
            dynasty: candidate.dynasty,
            matched_line_index: matched.line_index,
            snippet: HighlightedSnippet {
                text: matched.text,
                highlights: matched.highlights,
            },
        },
    })
}

fn compare_ranked_hits(left: &RankedHit, right: &RankedHit) -> Ordering {
    right
        .exact_line
        .cmp(&left.exact_line)
        .then_with(|| right.line_initial.cmp(&left.line_initial))
        .then_with(|| left.position.cmp(&right.position))
        .then_with(|| right.prominence.cmp(&left.prominence))
        .then_with(|| left.hit.poem_id.cmp(&right.hit.poem_id))
}

fn best_line_match(body: &str, pattern: &str) -> Option<LineMatch> {
    let pattern_chars = pattern.chars().collect::<Vec<_>>();
    let mut best = split_lines(body)
        .filter_map(|(line_index, text)| {
            find_pattern_match(&text, &pattern_chars).map(|(start, end, highlights)| LineMatch {
                line_index,
                text,
                start,
                end,
                highlights,
            })
        })
        .min_by(compare_line_matches);
    if best.is_none() {
        best = fallback_line_match(body, pattern);
    }
    best
}

fn compare_line_matches(left: &LineMatch, right: &LineMatch) -> Ordering {
    let left_exact = left.start == 0 && left.end == left.text.chars().count();
    let right_exact = right.start == 0 && right.end == right.text.chars().count();
    right_exact
        .cmp(&left_exact)
        .then_with(|| (right.start == 0).cmp(&(left.start == 0)))
        .then_with(|| left.start.cmp(&right.start))
        .then_with(|| left.line_index.cmp(&right.line_index))
}

fn split_lines(body: &str) -> impl Iterator<Item = (usize, String)> + '_ {
    body.split(['\n', '，', '。', '！', '？', '；'])
        .filter_map(|line| {
            let trimmed = line.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_owned())
        })
        .enumerate()
}

fn find_pattern_match(text: &str, pattern: &[char]) -> Option<(usize, usize, Vec<HighlightRange>)> {
    let text = text.chars().collect::<Vec<_>>();
    for start in 0..=text.len() {
        let mut highlights = Vec::new();
        if let Some(end) = match_pattern_at(&text, pattern, start, 0, &mut highlights) {
            return Some((start, end, merge_highlights(highlights)));
        }
    }
    None
}

fn match_pattern_at(
    text: &[char],
    pattern: &[char],
    text_index: usize,
    pattern_index: usize,
    highlights: &mut Vec<HighlightRange>,
) -> Option<usize> {
    let Some(pattern_char) = pattern.get(pattern_index).copied() else {
        return Some(text_index);
    };
    match pattern_char {
        '%' => {
            let next_pattern = pattern_index + 1;
            for next_text in text_index..=text.len() {
                let checkpoint = highlights.len();
                if let Some(end) =
                    match_pattern_at(text, pattern, next_text, next_pattern, highlights)
                {
                    return Some(end);
                }
                highlights.truncate(checkpoint);
            }
            None
        }
        '_' => (text_index < text.len()).then_some(()).and_then(|()| {
            match_pattern_at(text, pattern, text_index + 1, pattern_index + 1, highlights)
        }),
        literal if text.get(text_index) == Some(&literal) => {
            highlights.push(HighlightRange {
                start: text_index,
                end: text_index + 1,
            });
            let matched =
                match_pattern_at(text, pattern, text_index + 1, pattern_index + 1, highlights);
            if matched.is_none() {
                highlights.pop();
            }
            matched
        }
        _ => None,
    }
}

fn merge_highlights(highlights: Vec<HighlightRange>) -> Vec<HighlightRange> {
    let mut merged: Vec<HighlightRange> = Vec::new();
    for range in highlights {
        if let Some(last) = merged.last_mut()
            && last.end == range.start
        {
            last.end = range.end;
            continue;
        }
        merged.push(range);
    }
    merged
}

fn fallback_line_match(body: &str, pattern: &str) -> Option<LineMatch> {
    let literal = pattern
        .split(['%', '_'])
        .filter(|part| !part.is_empty())
        .max_by_key(|part| part.chars().count())?;
    split_lines(body).find_map(|(line_index, text)| {
        let start = char_position(&text, literal)?;
        let end = start + literal.chars().count();
        Some(LineMatch {
            line_index,
            text,
            start,
            end,
            highlights: vec![HighlightRange { start, end }],
        })
    })
}

fn char_position(text: &str, needle: &str) -> Option<usize> {
    let byte_position = text.find(needle)?;
    Some(text[..byte_position].chars().count())
}

fn static_prominence(author: &str, anthology: bool) -> u16 {
    let author_score = match author {
        "李白" | "杜甫" | "苏轼" => 100,
        "王维" | "白居易" | "李商隐" | "杜牧" | "辛弃疾" | "李清照" => 90,
        "王昌龄" | "孟浩然" | "王之涣" | "柳宗元" | "王安石" | "陆游" => 80,
        _ => 0,
    };
    author_score + u16::from(anthology) * 1_000
}

fn decode_cursor(cursor: Option<&str>, normalized_query: &str) -> Result<usize> {
    let Some(cursor) = cursor else {
        return Ok(0);
    };
    let bytes = decode_hex(cursor)?;
    let cursor: TextCursor = serde_json::from_slice(&bytes)
        .map_err(|error| Error::Search(format!("正文检索 cursor 无法解析：{error}")))?;
    if cursor.query != normalized_query {
        return Err(Error::Search(
            "正文检索 cursor 与当前归一化查询不匹配".to_owned(),
        ));
    }
    Ok(cursor.offset)
}

fn encode_cursor(cursor: &TextCursor) -> Result<String> {
    let bytes = serde_json::to_vec(cursor)
        .map_err(|error| Error::Search(format!("正文检索 cursor 无法编码：{error}")))?;
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write;
        write!(&mut encoded, "{byte:02x}")
            .map_err(|error| Error::Search(format!("正文检索 cursor 无法编码：{error}")))?;
    }
    Ok(encoded)
}

fn decode_hex(encoded: &str) -> Result<Vec<u8>> {
    if !encoded.len().is_multiple_of(2) {
        return Err(Error::Search("正文检索 cursor 长度非法".to_owned()));
    }
    encoded
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let pair = std::str::from_utf8(pair)
                .map_err(|error| Error::Search(format!("正文检索 cursor 非法：{error}")))?;
            u8::from_str_radix(pair, 16)
                .map_err(|error| Error::Search(format!("正文检索 cursor 非法：{error}")))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CorpusConfig, SCHEMA_VERSION};
    use rusqlite::{Connection, params};
    use std::collections::BTreeSet;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);

    #[derive(Clone)]
    struct FixturePoem {
        id: String,
        title: String,
        author: String,
        dynasty: String,
        body: String,
        anthology: bool,
    }

    impl FixturePoem {
        fn new(id: &str, author: &str, body: &str) -> Self {
            Self {
                id: id.to_owned(),
                title: format!("题-{id}"),
                author: author.to_owned(),
                dynasty: "唐".to_owned(),
                body: body.to_owned(),
                anthology: false,
            }
        }

        fn anthology(mut self) -> Self {
            self.anthology = true;
            self
        }
    }

    struct Fixture {
        dir: PathBuf,
        handle: CorpusHandle,
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    fn fixture(poems: &[FixturePoem]) -> Fixture {
        let dir = std::env::temp_dir().join(format!(
            "yunjian-text-search-{}-{}",
            std::process::id(),
            NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("创建正文检索 fixture 目录");
        let path = dir.join("corpus.db");
        write_fixture(&path, poems);
        let handle = CorpusHandle::open(&CorpusConfig {
            path: Some(path),
            data_dir: dir.clone(),
            archive: None,
        })
        .expect("打开正文检索 fixture");
        Fixture { dir, handle }
    }

    fn write_fixture(path: &Path, poems: &[FixturePoem]) {
        let mut connection = Connection::open(path).expect("创建正文检索 fixture 数据库");
        connection
            .execute_batch(
                "CREATE TABLE poem(
                     stable_id TEXT PRIMARY KEY NOT NULL,
                     title TEXT NOT NULL,
                     author TEXT NOT NULL,
                     dynasty TEXT NOT NULL,
                     body TEXT NOT NULL
                 );
                 CREATE TABLE poem_tag(
                     poem_id TEXT NOT NULL,
                     tag TEXT NOT NULL,
                     PRIMARY KEY(poem_id, tag)
                 ) WITHOUT ROWID;
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
            .expect("创建正文检索 fixture schema");
        let transaction = connection.transaction().expect("开始 fixture 事务");
        for poem in poems {
            transaction
                .execute(
                    "INSERT INTO poem(stable_id, title, author, dynasty, body)
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![poem.id, poem.title, poem.author, poem.dynasty, poem.body],
                )
                .expect("写 fixture 诗");
            if poem.anthology {
                transaction
                    .execute(
                        "INSERT INTO poem_tag(poem_id, tag) VALUES (?1, '唐诗三百首')",
                        [&poem.id],
                    )
                    .expect("写 fixture 选本标签");
            }
        }
        transaction
            .execute(
                "INSERT INTO corpus_meta VALUES
                 (1, ?1, 'text-fixture-v1', '2026-08-11T00:00:00Z', ?2, 'full',
                  'first_launch', '10k', 'ok')",
                params![SCHEMA_VERSION, poems.len() as i64],
            )
            .expect("写 fixture 元数据");
        transaction.commit().expect("提交 fixture 事务");
        connection.close().expect("关闭 fixture 数据库");
    }

    fn request(query: &str, limit: usize, cursor: Option<String>) -> TextSearchRequest {
        TextSearchRequest {
            query: query.to_owned(),
            limit,
            cursor,
        }
    }

    #[test]
    fn ranks_domain_signals_and_builds_unicode_highlights_in_rust() {
        let fixture = fixture(&[
            FixturePoem::new("z-exact", "佚名", "明月"),
            FixturePoem::new("y-initial", "佚名", "明月照我"),
            FixturePoem::new("x-position-one", "佚名", "看明月照我"),
            FixturePoem::new("a-anthology", "佚名", "床前明月光").anthology(),
            FixturePoem::new("z-famous", "李白", "床前明月光"),
            FixturePoem::new("a-unknown", "佚名", "床前明月光"),
            FixturePoem::new("a-tie", "王维", "床前明月光"),
            FixturePoem::new("b-tie", "王维", "床前明月光"),
        ]);

        let page = fixture
            .handle
            .search_text(request("明月", 20, None))
            .expect("检索明月");
        let ids = page
            .hits
            .iter()
            .map(|hit| hit.poem_id.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            ids,
            [
                "z-exact",
                "y-initial",
                "x-position-one",
                "a-anthology",
                "z-famous",
                "a-tie",
                "b-tie",
                "a-unknown",
            ]
        );
        assert_eq!(page.plan_used.contract_name(), "Ngram");
        assert_eq!(page.hits[3].matched_line_index, 0);
        assert_eq!(page.hits[3].snippet.text, "床前明月光");
        assert_eq!(
            page.hits[3].snippet.highlights,
            [HighlightRange { start: 2, end: 4 }]
        );
    }

    #[test]
    fn exact_line_selection_reports_the_best_matching_line_index() {
        let fixture = fixture(&[FixturePoem::new(
            "multi-line",
            "李白",
            "前句明月在。明月光。明月。",
        )]);
        let page = fixture
            .handle
            .search_text(request("明月", 20, None))
            .expect("检索多句正文");
        let hit = page.hits.first().expect("应命中多句正文");
        assert_eq!(hit.matched_line_index, 2);
        assert_eq!(hit.snippet.text, "明月");
        assert_eq!(
            hit.snippet.highlights,
            [HighlightRange { start: 0, end: 2 }]
        );
    }

    #[test]
    fn poem_id_is_the_final_deterministic_tiebreak() {
        let ranked = |poem_id: &str| RankedHit {
            hit: TextSearchHit {
                poem_id: poem_id.to_owned(),
                title: "同题".to_owned(),
                author: "王维".to_owned(),
                dynasty: "唐".to_owned(),
                matched_line_index: 0,
                snippet: HighlightedSnippet {
                    text: "明月".to_owned(),
                    highlights: vec![HighlightRange { start: 0, end: 2 }],
                },
            },
            exact_line: true,
            line_initial: true,
            position: 0,
            prominence: 90,
        };
        assert_eq!(
            compare_ranked_hits(&ranked("a"), &ranked("b")),
            std::cmp::Ordering::Less
        );
    }

    #[test]
    fn hard_cap_cursor_and_tiebreak_page_thousands_without_gaps() {
        let poems = (0..1_205)
            .rev()
            .map(|index| FixturePoem::new(&format!("fixture:{index:04}"), "佚名", "月"))
            .collect::<Vec<_>>();
        let fixture = fixture(&poems);

        let first = fixture
            .handle
            .search_text(request("月", 10_000, None))
            .expect("检索第一页");
        assert_eq!(first.hits.len(), TEXT_SEARCH_HARD_CAP);
        assert_eq!(first.total_estimate, poems.len());
        assert!(first.next_cursor.is_some(), "超出硬上限必须给 cursor");

        let repeated = fixture
            .handle
            .search_text(request("月", 10_000, None))
            .expect("重复检索第一页");
        assert_eq!(first.hits, repeated.hits, "相同输入的排序必须逐项相同");
        assert_eq!(first.next_cursor, repeated.next_cursor);

        let mut all_ids = Vec::new();
        let mut cursor = None;
        loop {
            let page = fixture
                .handle
                .search_text(request("月", 10_000, cursor))
                .expect("翻页检索");
            all_ids.extend(page.hits.into_iter().map(|hit| hit.poem_id));
            cursor = page.next_cursor;
            if cursor.is_none() {
                break;
            }
        }
        assert_eq!(all_ids.len(), poems.len());
        assert_eq!(
            all_ids.iter().collect::<BTreeSet<_>>().len(),
            poems.len(),
            "cursor 翻页不得重复"
        );
        assert_eq!(
            all_ids,
            (0..1_205)
                .map(|index| format!("fixture:{index:04}"))
                .collect::<Vec<_>>(),
            "poem_id tiebreak 必须让分页无跳项且跨运行稳定"
        );
    }

    #[test]
    fn cursor_is_bound_to_the_normalized_query() {
        let fixture = fixture(&[
            FixturePoem::new("a", "李白", "明月光"),
            FixturePoem::new("b", "杜甫", "明月照"),
        ]);
        let first = fixture
            .handle
            .search_text(request("明月", 1, None))
            .expect("生成 cursor");
        let error = fixture
            .handle
            .search_text(request("月光", 1, first.next_cursor))
            .expect_err("cursor 不得跨查询复用");
        assert!(matches!(error, Error::Search(_)));
    }

    #[test]
    fn handle_can_search_from_a_worker_thread() {
        let fixture = fixture(&[FixturePoem::new("worker", "李白", "床前明月光")]);
        let handle = fixture.handle.clone();
        let page = std::thread::spawn(move || handle.search_text(TextSearchRequest::new("明月光")))
            .join()
            .expect("worker 不应 panic")
            .expect("worker 检索应成功");
        assert_eq!(page.hits[0].poem_id, "worker");
        assert_eq!(page.plan_used.contract_name(), "Match");
    }
}
