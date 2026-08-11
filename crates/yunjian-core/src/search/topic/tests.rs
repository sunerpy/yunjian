//! 主题标签检索与作品详情的测试。
//!
//! fixture 建在随仓的黄金契约 fixture（`tests/fixtures/poems.toml`）之上，**不改动它**。
//! `poem_tag` 的行直接取自那份文件的 `tags` 字段——那正是契约对标签的声明。
//! 「签入词表能不能产出这些标签」由 `yunjian-corpus` 的 `tag` 模块验证（依赖方向不允许
//! 本 crate 调用它），这里验证的是「产出之后取得回来」。两侧锚在同一份 fixture 上，
//! 所以任何一侧漂移都会被另一侧发现。
//!
//! 韵书行是本模块自备的极小平水韵子集，**刻意留出两个未收的字**，因为「未知平仄必须
//! 以未知的形式活下来」这条断言需要一个真实的未收字才能成立。

use super::*;
use crate::{CorpusConfig, RhymeConfidence, SCHEMA_VERSION};
use rusqlite::params;
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);

/// 随包 schema 的路径。跨 crate 只读一个文件，不引入依赖。
const CORPUS_SCHEMA_PATH: &str = "../yunjian-corpus/schema.sql";

/// 契约锚定的那首诗。本模块的详情类断言全部围绕它。
const ANCHOR: &str = "fixture:tang-libai-jingyesi";

/// 平仄反查刻意查不到的两个字。
///
/// 选「床」与「低」而不是生僻字：它们是常用字，因此「韵书未收」在真实语料上同样会发生
/// （平水韵只收韵字，不是字典）。用生僻字会让这条断言看起来像边角情形。
const UNCOVERED_CHARACTERS: [&str; 2] = ["床", "低"];

/// 极小平水韵子集：(韵部, 声调, 字)。
const PINGSHUI_ROWS: &[(&str, &str, &str)] = &[
    ("七阳", "level", "光"),
    ("七阳", "level", "霜"),
    ("七阳", "level", "乡"),
    ("八庚", "level", "明"),
    ("十一尤", "level", "头"),
    ("四支", "level", "思"),
    ("四支", "level", "疑"),
    ("一先", "level", "前"),
    ("六月", "entering", "月"),
    ("六语", "rising", "举"),
    ("四寘", "departing", "是"),
    ("四寘", "departing", "地"),
    ("二十三漾", "departing", "望"),
    ("七遇", "departing", "故"),
    // 「上」平仄两读：上声二十二养与去声二十三漾都收它，故它是 `Either`。
    ("二十二养", "rising", "上"),
    ("二十三漾", "departing", "上"),
    // 「空」三读，用于验证 `readings` 把三个声都透出来。
    ("一东", "level", "空"),
    ("一董", "rising", "空"),
    ("一送", "departing", "空"),
];

/// 契约锚定那首诗的韵部归属。
const ANCHOR_RHYME_GROUPS: &[(&str, &str, &str, &str)] = &[(ANCHOR, "pingshui", "七阳", "level")];

/// 一条集评。`work` 为空串时用于验证「缺出处是硬错误」。
struct CommentaryFixture {
    id: &'static str,
    poem_id: &'static str,
    text: &'static str,
    work: &'static str,
    author: &'static str,
    dynasty: &'static str,
    dynasty_raw: &'static str,
    completed_by: i64,
    source_note: &'static str,
}

/// 两条出处完备的集评。文本与出处形态照抄 `corpus/commentary/index.json` 的真实条目。
const COMMENTARIES: &[CommentaryFixture] = &[
    CommentaryFixture {
        id: "fixture-commentary-001",
        poem_id: ANCHOR,
        text: "「床前明月光」四句，妙絕古今，蓋以無意得之。",
        work: "唐诗别裁集",
        author: "沈德潜",
        dynasty: "清",
        dynasty_raw: "清",
        completed_by: 1717,
        source_note: "卷十九・五言絕句；据四部丛刊本，修订号 1234567",
    },
    CommentaryFixture {
        id: "fixture-commentary-002",
        poem_id: ANCHOR,
        text: "太白五言絕，自是天仙口語，此篇尤不假雕琢。",
        work: "沧浪诗话",
        author: "严羽",
        dynasty: "宋",
        dynasty_raw: "宋",
        completed_by: 1245,
        source_note: "卷五・十九・第 20 段；据维基文库校录本，修订号 2329597",
    },
];

/// 只在本模块存在的补充记录。
///
/// 契约 fixture 里没有一首诗含平仄两读的字，而「两读」与「多读但同属仄」必须分得开
/// ——`上` 有上声与去声两个读音却都是仄，`空` 才是真正的平仄两读。少了这一条，
/// [`Tone::Either`] 这一档就没有正例。
const EXTRA_POEM: (&str, &str, &str, &str, &str) = (
    "fixture:tang-wangwei-lucai",
    "鹿柴",
    "王维",
    "空山不见人，但闻人语响。",
    "空山不见人",
);

// ---------------------------------------------------------------- fixture 数据

#[derive(Debug, Deserialize)]
struct SharedFixtures {
    #[serde(rename = "poem")]
    poems: Vec<SharedPoem>,
}

#[derive(Debug, Deserialize)]
struct SharedPoem {
    stable_id: String,
    title: String,
    author: String,
    dynasty: String,
    ci_tune: String,
    body: String,
    first_line: String,
    last_chars: Vec<String>,
    tags: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct Contract {
    #[serde(rename = "query")]
    queries: Vec<ContractEntry>,
}

#[derive(Debug, Deserialize)]
struct ContractEntry {
    id: String,
    query: String,
    class: String,
    expect_top_id: String,
    expect_min_hits: usize,
}

fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn shared_fixtures() -> SharedFixtures {
    let path = manifest_dir().join("tests/fixtures/poems.toml");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("读取 fixture 失败 {}：{error}", path.display()));
    toml::from_str(&text)
        .unwrap_or_else(|error| panic!("解析 fixture 失败 {}：{error}", path.display()))
}

fn contract() -> Contract {
    let path = manifest_dir().join("tests/queries.toml");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("读取契约失败 {}：{error}", path.display()));
    toml::from_str(&text).unwrap_or_else(|error| panic!("解析契约失败 {}：{error}", path.display()))
}

// ---------------------------------------------------------------- fixture 语料库

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
    build_fixture(None)
}

/// `blank_citation_field` 指定要清空的出处列名，供「缺出处必须报错」的用例使用。
fn build_fixture(blank_citation_field: Option<&str>) -> Fixture {
    let dir = std::env::temp_dir().join(format!(
        "yunjian-topic-{}-{}",
        std::process::id(),
        NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("创建主题 fixture 目录");
    let path = dir.join("corpus.db");
    write_fixture(&path, blank_citation_field);
    let handle = CorpusHandle::open(&CorpusConfig {
        path: Some(path),
        data_dir: dir.clone(),
        archive: None,
    })
    .expect("打开主题 fixture");
    Fixture { dir, handle }
}

fn write_fixture(path: &Path, blank_citation_field: Option<&str>) {
    let connection = Connection::open(path).expect("创建主题 fixture 数据库");
    // 列、CHECK 与索引名逐字取自 `crates/yunjian-corpus/schema.sql`：索引名进
    // `EXPLAIN QUERY PLAN` 的断言，名字对不上断言就验的不是生产索引；CHECK 照抄是为了
    // 让 fixture 无法表达随包 schema 拒绝的形态（例如 1912 年之后成书的「集评」）。
    connection
        .execute_batch(
            "CREATE TABLE author(name TEXT PRIMARY KEY NOT NULL) WITHOUT ROWID;
             CREATE TABLE poem(
                 stable_id TEXT PRIMARY KEY NOT NULL,
                 content_hash TEXT NOT NULL,
                 source_locator TEXT NOT NULL UNIQUE,
                 source_locator_kind TEXT NOT NULL CHECK (source_locator_kind IN ('native', 'positional')),
                 genre TEXT NOT NULL,
                 title TEXT NOT NULL,
                 title_raw TEXT NOT NULL,
                 ci_tune TEXT,
                 author TEXT NOT NULL REFERENCES author(name),
                 dynasty TEXT NOT NULL,
                 dynasty_raw TEXT NOT NULL,
                 body TEXT NOT NULL,
                 body_original TEXT NOT NULL,
                 script TEXT NOT NULL,
                 first_line TEXT NOT NULL,
                 last_chars TEXT NOT NULL CHECK (json_valid(last_chars)),
                 line_count INTEGER NOT NULL,
                 char_count INTEGER NOT NULL,
                 provenance_source TEXT NOT NULL,
                 provenance_revision TEXT NOT NULL,
                 provenance_kind TEXT NOT NULL CHECK (provenance_kind IN ('原文', '集评-PD', 'AI')),
                 provenance_license TEXT NOT NULL,
                 provenance_license_class TEXT NOT NULL CHECK (provenance_license_class IN ('public_domain', 'permissive')),
                 work_group TEXT NOT NULL,
                 edition_group TEXT NOT NULL
             );
             CREATE TABLE commentary(
                 id TEXT PRIMARY KEY NOT NULL,
                 poem_id TEXT NOT NULL REFERENCES poem(stable_id),
                 text TEXT NOT NULL,
                 citation_work TEXT NOT NULL,
                 citation_author TEXT NOT NULL,
                 citation_dynasty TEXT NOT NULL,
                 citation_dynasty_raw TEXT NOT NULL,
                 citation_work_completed_by INTEGER NOT NULL CHECK (citation_work_completed_by < 1912),
                 citation_source_note TEXT NOT NULL
             ) WITHOUT ROWID;
             CREATE TABLE rhyme(
                 rhyme_book TEXT NOT NULL CHECK (rhyme_book IN ('pingshui', 'cilin', 'xinyun')),
                 rhyme_group TEXT NOT NULL,
                 tone TEXT NOT NULL CHECK (tone IN ('level', 'rising', 'departing', 'entering', 'oblique')),
                 tone_raw TEXT NOT NULL,
                 character TEXT NOT NULL,
                 PRIMARY KEY (rhyme_book, rhyme_group, tone, character)
             ) WITHOUT ROWID;
             CREATE TABLE poem_rhyme_group(
                 poem_id TEXT NOT NULL REFERENCES poem(stable_id),
                 rhyme_book TEXT NOT NULL CHECK (rhyme_book IN ('pingshui', 'cilin', 'xinyun')),
                 rhyme_group TEXT NOT NULL,
                 tone TEXT NOT NULL CHECK (tone IN ('level', 'rising', 'departing', 'entering', 'oblique')),
                 confidence TEXT NOT NULL CHECK (confidence IN ('resolved_by_vote', 'unambiguous', 'unresolved')),
                 PRIMARY KEY (poem_id, rhyme_book, rhyme_group, tone)
             ) WITHOUT ROWID;
             CREATE TABLE tag(name TEXT PRIMARY KEY NOT NULL) WITHOUT ROWID;
             CREATE TABLE poem_tag(
                 poem_id TEXT NOT NULL REFERENCES poem(stable_id),
                 tag TEXT NOT NULL REFERENCES tag(name),
                 PRIMARY KEY (poem_id, tag)
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
             );
             CREATE INDEX poem_work_group_idx ON poem(work_group);
             CREATE INDEX poem_rhyme_group_idx ON poem_rhyme_group(rhyme_book, rhyme_group, poem_id);
             CREATE INDEX poem_tag_idx ON poem_tag(tag, poem_id);
             CREATE INDEX rhyme_character_idx ON rhyme(rhyme_book, character);
             CREATE INDEX commentary_poem_idx ON commentary(poem_id);",
        )
        .expect("创建主题 fixture schema");

    let fixtures = shared_fixtures();
    for poem in &fixtures.poems {
        connection
            .execute(
                "INSERT OR IGNORE INTO author(name) VALUES (?1)",
                params![poem.author],
            )
            .expect("写作者");
        connection
            .execute(
                "INSERT INTO poem(stable_id, content_hash, source_locator, source_locator_kind, \
                 genre, title, title_raw, ci_tune, author, dynasty, dynasty_raw, body, \
                 body_original, script, first_line, last_chars, line_count, char_count, \
                 provenance_source, provenance_revision, provenance_kind, provenance_license, \
                 provenance_license_class, work_group, edition_group) \
                 VALUES (?1, ?2, ?3, 'native', 'shi', ?4, ?4, ?5, ?6, ?7, ?7, ?8, ?8, \
                 'simplified', ?9, ?10, ?11, ?12, 'chinese-poetry', 'rev-abc123', '原文', 'MIT', \
                 'permissive', ?13, ?14)",
                params![
                    poem.stable_id,
                    format!("hash-{}", poem.stable_id),
                    format!("locator:{}", poem.stable_id),
                    poem.title,
                    (!poem.ci_tune.is_empty()).then(|| poem.ci_tune.clone()),
                    poem.author,
                    poem.dynasty,
                    poem.body,
                    poem.first_line,
                    serde_json::to_string(&poem.last_chars).expect("序列化 last_chars"),
                    crate::split_metrical_lines(&poem.body).count() as i64,
                    crate::text::content_chars(&poem.body).count() as i64,
                    format!("wg-{}", poem.title),
                    format!("eg-{}-{}", poem.author, poem.title),
                ],
            )
            .expect("写诗");
    }

    let (extra_id, extra_title, extra_author, extra_body, extra_first_line) = EXTRA_POEM;
    let extra_last_chars = ["人", "响"];
    connection
        .execute(
            "INSERT OR IGNORE INTO author(name) VALUES (?1)",
            params![extra_author],
        )
        .expect("写补充作者");
    connection
        .execute(
            "INSERT INTO poem(stable_id, content_hash, source_locator, source_locator_kind, \
             genre, title, title_raw, ci_tune, author, dynasty, dynasty_raw, body, \
             body_original, script, first_line, last_chars, line_count, char_count, \
             provenance_source, provenance_revision, provenance_kind, provenance_license, \
             provenance_license_class, work_group, edition_group) \
             VALUES (?1, 'hash-extra', 'locator:extra', 'native', 'shi', ?2, ?2, NULL, ?3, \
             '唐', '唐', ?4, ?4, 'simplified', ?5, ?6, 2, ?7, 'chinese-poetry', 'rev-abc123', \
             '原文', 'MIT', 'permissive', 'wg-鹿柴', 'eg-王维-鹿柴')",
            params![
                extra_id,
                extra_title,
                extra_author,
                extra_body,
                extra_first_line,
                serde_json::to_string(&extra_last_chars).expect("序列化补充 last_chars"),
                crate::text::content_chars(extra_body).count() as i64,
            ],
        )
        .expect("写补充诗");

    // 标签取自 fixture 的 `tags` 字段，即契约对标签的声明。
    let mut declared: Vec<&str> = fixtures
        .poems
        .iter()
        .flat_map(|poem| poem.tags.iter().map(String::as_str))
        .collect();
    declared.sort_unstable();
    declared.dedup();
    for name in &declared {
        connection
            .execute("INSERT INTO tag(name) VALUES (?1)", params![name])
            .expect("写标签");
    }
    for poem in &fixtures.poems {
        for tag in &poem.tags {
            connection
                .execute(
                    "INSERT INTO poem_tag(poem_id, tag) VALUES (?1, ?2)",
                    params![poem.stable_id, tag],
                )
                .expect("写标签关联");
        }
    }

    for (group, tone, character) in PINGSHUI_ROWS {
        connection
            .execute(
                "INSERT INTO rhyme(rhyme_book, rhyme_group, tone, tone_raw, character) \
                 VALUES ('pingshui', ?1, ?2, ?2, ?3)",
                params![group, tone, character],
            )
            .expect("写韵书行");
    }
    for (poem_id, book, group, tone) in ANCHOR_RHYME_GROUPS {
        connection
            .execute(
                "INSERT INTO poem_rhyme_group(poem_id, rhyme_book, rhyme_group, tone, confidence) \
                 VALUES (?1, ?2, ?3, ?4, 'unambiguous')",
                params![poem_id, book, group, tone],
            )
            .expect("写韵部归属");
    }

    for entry in COMMENTARIES {
        let blank = |field: &str, value: &'static str| -> String {
            if blank_citation_field == Some(field) && entry.id == COMMENTARIES[0].id {
                String::new()
            } else {
                value.to_owned()
            }
        };
        connection
            .execute(
                "INSERT INTO commentary(id, poem_id, text, citation_work, citation_author, \
                 citation_dynasty, citation_dynasty_raw, citation_work_completed_by, \
                 citation_source_note) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    entry.id,
                    entry.poem_id,
                    entry.text,
                    blank("citation_work", entry.work),
                    blank("citation_author", entry.author),
                    blank("citation_dynasty", entry.dynasty),
                    entry.dynasty_raw,
                    entry.completed_by,
                    blank("citation_source_note", entry.source_note),
                ],
            )
            .expect("写集评");
    }

    connection
        .execute(
            "INSERT INTO corpus_meta VALUES (1, ?1, 'topic-fixture-v1', \
             '2026-08-11T00:00:00Z', ?2, 'full', 'first_launch', '10k', 'ok')",
            params![SCHEMA_VERSION, fixtures.poems.len() as i64],
        )
        .expect("写 corpus_meta");
    connection.close().expect("关闭主题 fixture 数据库");
}

fn ids(page: &MetaPage) -> Vec<&str> {
    page.hits.iter().map(|hit| hit.stable_id.as_str()).collect()
}

// ---------------------------------------------------------------- 标签检索

#[test]
fn tag_membership_matches_the_checked_in_vocabulary_projection() {
    let fixture = fixture();
    let handle = &fixture.handle;
    let shared = shared_fixtures();

    for poem in &shared.poems {
        for tag in &poem.tags {
            let page = browse_by_tag(handle, tag, None)
                .unwrap_or_else(|error| panic!("按标签 {tag} 浏览失败：{error}"));
            assert!(
                ids(&page).contains(&poem.stable_id.as_str()),
                "{} 声明了标签 {tag}，但按该标签浏览没有它：{:?}",
                poem.stable_id,
                ids(&page)
            );
            assert_eq!(
                page.hits
                    .iter()
                    .find(|hit| hit.stable_id == poem.stable_id)
                    .map(|hit| hit.matched_on),
                Some(MetaMatch::Tag),
                "标签命中必须标为 Tag"
            );
        }
    }

    // 反向：按标签取回的每一首，都必须真的声明了它。少了会漏，多了会把别的诗塞进
    // 一个主题里，两种坏法都不会报错。
    for summary in list_tags(handle).expect("列标签") {
        let page = browse_by_tag(handle, &summary.name, None).expect("按标签浏览");
        assert_eq!(
            page.hits.len(),
            summary.poem_count,
            "标签 {} 的计数与浏览结果不一致",
            summary.name
        );
        for hit in &page.hits {
            let declared = shared
                .poems
                .iter()
                .find(|poem| poem.stable_id == hit.stable_id)
                .map(|poem| poem.tags.contains(&summary.name))
                .unwrap_or(false);
            assert!(
                declared,
                "{} 出现在标签 {} 下，但它没有声明这个标签",
                hit.stable_id, summary.name
            );
        }
    }
}

#[test]
fn an_unknown_tag_is_an_error_that_lists_what_does_exist() {
    let fixture = fixture();
    let error = browse_by_tag(&fixture.handle, "无此标签", None)
        .expect_err("未登记的标签必须报错，不得返回空页");
    let text = error.to_string();
    assert!(text.contains("无此标签"), "错误要点名那个标签：{text}");
    assert!(text.contains("思乡"), "错误要列出现有标签：{text}");
    assert!(
        text.contains("构建期"),
        "错误要说明标签的来源，否则调用方会以为可以运行时新建：{text}"
    );
}

#[test]
fn the_tag_contract_entries_pass_against_the_production_entry_point() {
    let fixture = fixture();
    let handle = &fixture.handle;
    let mut covered = 0;
    for entry in &contract().queries {
        if entry.class != "tag_query" {
            continue;
        }
        covered += 1;
        let page = browse_by_tag(handle, &entry.query, None)
            .unwrap_or_else(|error| panic!("{}: 执行契约查询失败：{error}", entry.id));
        let hits = ids(&page);
        assert!(
            hits.len() >= entry.expect_min_hits,
            "{}: 标签「{}」只命中 {} 首（{hits:?}），低于契约下界 {}",
            entry.id,
            entry.query,
            hits.len(),
            entry.expect_min_hits
        );
        assert!(
            hits.contains(&entry.expect_top_id.as_str()),
            "{}: 契约的锚 {} 不在命中集合里：{hits:?}",
            entry.id,
            entry.expect_top_id
        );
    }
    assert_eq!(
        covered, 2,
        "契约里 tag_query 类共 2 条，实际跑了 {covered} 条——契约被改动过就要同步这个数"
    );
}

#[test]
fn a_tag_page_is_capped_and_carries_a_cursor_only_when_more_remain() {
    let fixture = fixture();
    let page = browse_by_tag(&fixture.handle, "月", None).expect("按标签浏览");
    assert!(page.hits.len() <= TAG_PAGE_LIMIT);
    assert!(
        page.next_cursor.is_none(),
        "fixture 规模远小于单页上限，不该给出续页游标"
    );
    assert_eq!(page.normalized, "月");
}

// ---------------------------------------------------------------- 作品详情

#[test]
fn poem_detail_returns_the_poem_its_author_and_its_provenance() {
    let fixture = fixture();
    let detail = poem_detail(&fixture.handle, ANCHOR).expect("取详情");

    assert_eq!(detail.poem.stable_id, ANCHOR);
    assert_eq!(detail.poem.title, "静夜思");
    assert_eq!(detail.poem.author, "李白");
    assert_eq!(detail.poem.dynasty.canonical, "唐");
    assert_eq!(detail.poem.last_chars, vec!["霜", "乡"]);
    assert_eq!(detail.author.name, "李白");
    assert!(
        detail.author.poem_count >= 2,
        "fixture 里李白有多首，作者记录应如实计数：{}",
        detail.author.poem_count
    );
    assert!(
        detail
            .author
            .dynasties
            .iter()
            .any(|label| label.canonical == "唐"),
        "作者记录要带朝代：{:?}",
        detail.author.dynasties
    );

    assert_eq!(detail.provenance.kind, "原文");
    assert_eq!(detail.provenance.license_class, "permissive");
    assert!(
        !detail.provenance.source_locator.is_empty(),
        "溯源必须带上游定位符，否则这条记录无法回到上游核对"
    );
    assert!(!detail.provenance.revision.is_empty());
}

#[test]
fn poem_detail_carries_the_curated_tags_and_the_rhyme_group_with_its_confidence() {
    let fixture = fixture();
    let detail = poem_detail(&fixture.handle, ANCHOR).expect("取详情");
    assert_eq!(detail.tags, vec!["思乡".to_owned(), "月".to_owned()]);

    assert_eq!(detail.rhyme_groups.len(), 1);
    let membership = &detail.rhyme_groups[0];
    assert_eq!(membership.book, RhymeBook::Pingshui);
    assert_eq!(membership.group, "七阳");
    assert_eq!(membership.tone, RhymeTone::Level);
    assert_eq!(membership.confidence, RhymeConfidence::Unambiguous);
    assert!(membership.is_positive_claim(), "unambiguous 必须是肯定判断");
}

#[test]
fn poem_detail_lists_work_group_siblings_without_repeating_the_poem_itself() {
    let fixture = fixture();
    let detail = poem_detail(&fixture.handle, ANCHOR).expect("取详情");
    assert!(
        detail
            .work_group_siblings
            .iter()
            .all(|sibling| sibling.stable_id != ANCHOR),
        "兄弟项里不得含本篇自己"
    );
    assert!(
        detail.attribution_conflict.is_none(),
        "fixture 里《静夜思》只挂李白一人，不该报归属冲突"
    );
}

#[test]
fn an_unknown_poem_id_is_a_named_error_rather_than_an_empty_detail() {
    let fixture = fixture();
    let error =
        poem_detail(&fixture.handle, "fixture:does-not-exist").expect_err("未知 id 必须报错");
    assert!(
        error.to_string().contains("fixture:does-not-exist"),
        "错误要点名那个 id：{error}"
    );
}

// ---------------------------------------------------------------- 集评与出处

#[test]
fn every_commentary_entry_comes_back_with_a_non_empty_citation() {
    let fixture = fixture();
    let detail = poem_detail(&fixture.handle, ANCHOR).expect("取详情");
    assert_eq!(detail.commentaries.len(), COMMENTARIES.len());
    for entry in &detail.commentaries {
        assert!(!entry.id.is_empty());
        assert!(!entry.text.trim().is_empty(), "{}: 评语正文为空", entry.id);
        let citation = &entry.citation;
        assert!(
            !citation.work.trim().is_empty(),
            "{}: 出处著作为空",
            entry.id
        );
        assert!(!citation.author.trim().is_empty(), "{}: 评者为空", entry.id);
        assert!(
            !citation.dynasty.canonical.trim().is_empty(),
            "{}: 评者朝代为空",
            entry.id
        );
        assert!(
            !citation.source_note.trim().is_empty(),
            "{}: 定位与版本为空——引用无法复核",
            entry.id
        );
        assert!(
            citation.work_completed_by < 1912,
            "{}: 成书年 {} 不早于 1912，不是前现代著作",
            entry.id,
            citation.work_completed_by
        );
    }
}

/// 缺出处的集评必须是类型化错误，而不是一个空字段。
///
/// 四个必填出处字段逐个清空各验一次：只验其中一个，另外三个的检查可以被删掉而测试仍绿。
#[test]
fn a_commentary_missing_any_citation_field_is_a_typed_error_naming_it() {
    for field in [
        "citation_work",
        "citation_author",
        "citation_dynasty",
        "citation_source_note",
    ] {
        let fixture = build_fixture(Some(field));
        match poem_detail(&fixture.handle, ANCHOR) {
            Err(Error::CommentaryCitationMissing { missing_field, .. }) => {
                assert_eq!(missing_field, field, "错误要点名被清空的那个字段");
            }
            Err(other) => panic!("{field} 为空时应报 CommentaryCitationMissing，实际是 {other:?}"),
            Ok(detail) => panic!(
                "{field} 为空时 poem_detail 必须报错，实际返回了 {} 条集评",
                detail.commentaries.len()
            ),
        }
    }
}

#[test]
fn the_citation_error_names_the_offending_commentary_and_field() {
    let fixture = build_fixture(Some("citation_source_note"));
    let error = poem_detail(&fixture.handle, ANCHOR).expect_err("缺出处必须报错");
    match &error {
        Error::CommentaryCitationMissing {
            commentary_id,
            poem_id,
            missing_field,
        } => {
            assert_eq!(commentary_id, COMMENTARIES[0].id);
            assert_eq!(poem_id, ANCHOR);
            assert_eq!(*missing_field, "citation_source_note");
        }
        other => panic!("必须是 CommentaryCitationMissing，实际是 {other:?}"),
    }
    let text = error.to_string();
    assert!(text.contains(COMMENTARIES[0].id), "{text}");
    assert!(text.contains("citation_source_note"), "{text}");
}

// ---------------------------------------------------------------- 平仄

#[test]
fn unknown_tone_positions_survive_as_unknown_and_are_never_rendered_as_level() {
    let fixture = fixture();
    let detail = poem_detail(&fixture.handle, ANCHOR).expect("取详情");
    let tones = &detail.tones;
    assert_eq!(tones.book, RhymeBook::Pingshui);
    assert_eq!(tones.lines.len(), 4, "《静夜思》四句");

    let mut unknown_characters = Vec::new();
    for line in &tones.lines {
        for cell in &line.cells {
            if cell.tone == Tone::Unknown {
                unknown_characters.push(cell.character.clone());
                assert!(
                    !cell.tone.is_level(),
                    "{} 的平仄未知，is_level 必须为 false",
                    cell.character
                );
                assert!(
                    cell.readings.is_empty(),
                    "{} 未知却带了读音，说明归并逻辑错了",
                    cell.character
                );
                assert_eq!(cell.tone.marker(), '？');
                assert_eq!(cell.tone.as_key(), "unknown");
            }
        }
    }
    unknown_characters.sort();
    let mut expected: Vec<String> = UNCOVERED_CHARACTERS
        .iter()
        .map(|character| (*character).to_owned())
        .collect();
    expected.sort();
    assert_eq!(
        unknown_characters, expected,
        "韵书未收的字必须且只能是这两个"
    );
    assert_eq!(tones.unknown_count, UNCOVERED_CHARACTERS.len());
    assert!(tones.has_unknown());

    // 展示串里未知位置写 `？`，绝不写 `平`。
    let display = tones.display();
    assert!(display.contains('？'), "展示串必须体现未知：{display}");
    assert_eq!(
        display.chars().filter(|c| *c == '？').count(),
        UNCOVERED_CHARACTERS.len(),
        "展示串里的未知个数要与统计一致：{display}"
    );
}

/// 「多个读音」不等于「平仄两读」。
///
/// 「上」在上声二十二养与去声二十三漾各有一读，但两读都是仄，所以它的平仄是确定的。
/// 把它报成 [`Tone::Either`] 会让一个本可判定的位置变成不可判定，格律分析随之失去依据。
#[test]
fn a_character_whose_readings_are_all_oblique_stays_oblique_rather_than_ambiguous() {
    let fixture = fixture();
    let detail = poem_detail(&fixture.handle, ANCHOR).expect("取详情");
    let cell = detail
        .tones
        .lines
        .iter()
        .flat_map(|line| &line.cells)
        .find(|cell| cell.character == "上")
        .expect("《静夜思》第二句有「上」");
    assert_eq!(
        cell.readings,
        vec!["departing".to_owned(), "rising".to_owned()],
        "两个读音都要透出来"
    );
    assert_eq!(cell.tone, Tone::Oblique, "上声与去声都是仄，平仄并不含糊");
    assert!(!cell.tone.is_level());
    assert_eq!(detail.tones.either_count, 0, "《静夜思》里没有平仄两读的字");
}

#[test]
fn a_character_read_in_both_level_and_oblique_is_either_and_carries_all_its_readings() {
    let fixture = fixture();
    let detail = poem_detail(&fixture.handle, EXTRA_POEM.0).expect("取补充诗的详情");
    let cell = detail
        .tones
        .lines
        .iter()
        .flat_map(|line| &line.cells)
        .find(|cell| cell.character == "空")
        .expect("《鹿柴》首句有「空」");
    assert_eq!(
        cell.tone,
        Tone::Either,
        "「空」在上平一东（平）、上声一董与去声一送（仄）都收，是平仄两读"
    );
    assert!(
        !cell.tone.is_level(),
        "两读不得算作确定的平声：格律判断只能建立在确定的平上"
    );
    assert_eq!(
        cell.readings,
        vec![
            "departing".to_owned(),
            "level".to_owned(),
            "rising".to_owned()
        ],
        "三个读音都要透出来，否则用户无从判断该按哪个读"
    );
    assert_eq!(cell.tone.marker(), '多');
    assert_eq!(detail.tones.either_count, 1);
}

#[test]
fn known_tones_are_derived_from_the_rhyme_book_not_guessed() {
    let fixture = fixture();
    let detail = poem_detail(&fixture.handle, ANCHOR).expect("取详情");
    let tone_of = |character: &str| {
        detail
            .tones
            .lines
            .iter()
            .flat_map(|line| &line.cells)
            .find(|cell| cell.character == character)
            .map(|cell| cell.tone)
            .unwrap_or_else(|| panic!("正文里没有 {character}"))
    };
    assert_eq!(tone_of("光"), Tone::Level, "光在下平七阳，平");
    assert_eq!(tone_of("月"), Tone::Oblique, "月在入声六月，入归仄");
    assert_eq!(tone_of("是"), Tone::Oblique, "是在去声四寘，仄");
    assert_eq!(tone_of("举"), Tone::Oblique, "举在上声六语，仄");
    assert!(tone_of("光").is_level());
    assert!(!tone_of("月").is_level());
}

#[test]
fn the_upstream_undetermined_markers_map_to_unknown_not_to_level() {
    // 上游平仄表用 `？` 与 `○` 表示未定。它们必须与「韵书未收」落在同一档，否则同一个
    // 「不知道」会因为来源不同而被渲染成两种东西。
    for marker in ['？', '○', '?'] {
        assert_eq!(
            Tone::from_marker(marker),
            Some(Tone::Unknown),
            "{marker} 必须是未知"
        );
    }
    assert_eq!(Tone::from_marker('平'), Some(Tone::Level));
    assert_eq!(Tone::from_marker('仄'), Some(Tone::Oblique));
    assert_eq!(Tone::from_marker('多'), Some(Tone::Either));
    assert_eq!(Tone::from_marker('，'), None, "标点不是平仄，由调用方跳过");
    assert!(!Tone::Unknown.is_level());
    assert!(!Tone::Either.is_level());
}

#[test]
fn tone_lines_and_rhyme_feet_keep_their_distinct_boundaries() {
    // 平仄按逗号切格律行，韵脚只按句号等句末标点切；这里刻意证明两套边界不会再次被
    // “统一”。《静夜思》因此有 4 个平仄行，但只有「霜/乡」两个韵脚候选。
    let fixture = fixture();
    let detail = poem_detail(&fixture.handle, ANCHOR).expect("取详情");
    assert_eq!(detail.poem.line_count, 4);
    assert_eq!(detail.poem.last_chars, ["霜", "乡"]);
    assert_eq!(detail.tones.lines.len(), 4);
    let metrical_ends = detail
        .tones
        .lines
        .iter()
        .map(|line| line.cells.last().map(|cell| cell.character.as_str()))
        .collect::<Vec<_>>();
    assert_eq!(
        metrical_ends,
        [Some("光"), Some("霜"), Some("月"), Some("乡")]
    );
    for (index, line) in detail.tones.lines.iter().enumerate() {
        assert_eq!(
            line.line_index as usize, index,
            "格律行序号必须连续且从 0 起"
        );
    }
}

// ---------------------------------------------------------------- 查询计划

const BASE_ALIASES: [&str; 5] = ["poem", "p", "t", "commentary", "rhyme"];

/// 一条待检查的计划：标签、SQL 与它的绑定值。
type PlanCase = (&'static str, &'static str, Vec<Box<dyn rusqlite::ToSql>>);

fn plan_of(connection: &Connection, sql: &str, binds: &[&dyn rusqlite::ToSql]) -> Vec<String> {
    let mut statement = connection
        .prepare(&format!("EXPLAIN QUERY PLAN {sql}"))
        .unwrap_or_else(|error| panic!("准备计划查询失败：{error}\nSQL: {sql}"));
    let rows = statement
        .query_map(binds, |row| row.get::<_, String>(3))
        .expect("执行计划查询");
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .expect("读取计划行")
}

fn check_btree_plan(lines: &[String], required_index: &str) -> std::result::Result<(), String> {
    if let Some(line) = lines.iter().find(|line| line.contains("poem_fts")) {
        return Err(format!("主题检索不得引用 poem_fts：{line}"));
    }
    if !lines.iter().any(|line| line.contains(required_index)) {
        return Err(format!("计划里没有 {required_index}：{lines:?}"));
    }
    for line in lines {
        for alias in BASE_ALIASES {
            if line.contains(&format!("SCAN {alias}")) && !line.contains("USING") {
                return Err(format!("退化成基表扫描：{line}"));
            }
        }
    }
    Ok(())
}

#[test]
fn every_topic_query_uses_an_index_and_none_touches_the_fts_table() {
    let fixture = fixture();
    let connection = fixture.handle.connect().expect("开连接");
    // 前提检查：`poem_fts` 必须**确实存在**（`CorpusHandle::open` 的首启派生会建它），
    // 否则「计划里没有 poem_fts」是一句空话——一个不存在的对象当然不会出现在计划里。
    let fts_tables: i64 = connection
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE type='table' AND name='poem_fts'",
            [],
            |row| row.get(0),
        )
        .expect("查 poem_fts 是否存在");
    assert_eq!(
        fts_tables, 1,
        "首启派生应已建出 poem_fts；它不存在时下面的断言证明不了任何事"
    );

    let cases: [PlanCase; 5] = [
        (
            "标签作品",
            TAG_POEMS_SQL,
            vec![
                Box::new("思乡".to_owned()),
                Box::new(String::new()),
                Box::new(50_i64),
            ],
        ),
        (
            "标签存在",
            TAG_EXISTS_SQL,
            vec![Box::new("思乡".to_owned())],
        ),
        (
            "作品详情",
            POEM_DETAIL_SQL,
            vec![Box::new(ANCHOR.to_owned())],
        ),
        (
            "逐字声调",
            CHARACTER_TONES_SQL,
            vec![Box::new("pingshui".to_owned()), Box::new("光".to_owned())],
        ),
        (
            "作品集评",
            COMMENTARY_SQL,
            vec![Box::new(ANCHOR.to_owned())],
        ),
    ];
    // `tag` 与 `poem_rhyme_group` 是 `WITHOUT ROWID` 表，计划里写作 `USING PRIMARY KEY`；
    // `poem` 是普通 rowid 表，它的 `TEXT PRIMARY KEY` 由 SQLite 建成
    // `sqlite_autoindex_poem_1`。两种写法不能互换，所以逐条写实测到的那一个。
    let required = [
        "poem_tag_idx",
        "PRIMARY KEY",
        "sqlite_autoindex_poem_1",
        "rhyme_character_idx",
        "commentary_poem_idx",
    ];
    for ((label, sql, binds), index) in cases.iter().zip(required) {
        let refs: Vec<&dyn rusqlite::ToSql> = binds.iter().map(AsRef::as_ref).collect();
        let plan = plan_of(&connection, sql, &refs);
        if let Err(reason) = check_btree_plan(&plan, index) {
            panic!("{label} 的查询计划不合格：{reason}\n计划：{plan:?}");
        }
    }

    let plan = plan_of(
        &connection,
        POEM_RHYME_GROUPS_SQL,
        &[&ANCHOR as &dyn rusqlite::ToSql],
    );
    if let Err(reason) = check_btree_plan(&plan, "PRIMARY KEY") {
        panic!("韵部归属的查询计划不合格：{reason}\n计划：{plan:?}");
    }
}

/// 计划断言的可证伪对照：拿掉 `poem_tag_idx`，标签检索必须被判为不合格。
#[test]
fn dropping_the_tag_index_makes_the_plan_inspection_fail() {
    let fixture = fixture();
    let connection = fixture.handle.connect().expect("开连接");
    let binds: [&dyn rusqlite::ToSql; 3] = [&"思乡", &"", &50_i64];
    let before = plan_of(&connection, TAG_POEMS_SQL, &binds);
    assert!(check_btree_plan(&before, "poem_tag_idx").is_ok());

    // 只在一个可写的副本上做，不动 fixture 的只读语料库。
    let scratch = fixture.dir.join("scratch.db");
    std::fs::copy(fixture.handle.path(), &scratch).expect("复制语料库");
    let writable = Connection::open(&scratch).expect("打开可写副本");
    writable
        .execute_batch("DROP INDEX poem_tag_idx")
        .expect("拿掉索引");
    let after = plan_of(&writable, TAG_POEMS_SQL, &binds);
    assert!(
        check_btree_plan(&after, "poem_tag_idx").is_err(),
        "索引拿掉之后计划检查必须变红，否则它证明不了任何事：{after:?}"
    );
}

// ---------------------------------------------------------------- 索引名守卫

/// 断言的索引名必须与随包 schema 一致。
///
/// 否则 `EXPLAIN` 断言只证明了 fixture 自造的索引被用上了：随包 schema 改名之后，
/// 真实语料上退化成扫表而本模块的测试全绿。
#[test]
fn the_asserted_index_names_match_the_shipped_schema() {
    let path = manifest_dir().join(CORPUS_SCHEMA_PATH);
    let schema = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("读取随包 schema 失败 {}：{error}", path.display()));
    for (name, columns) in [
        ("poem_tag_idx", "poem_tag(tag, poem_id)"),
        ("rhyme_character_idx", "rhyme(rhyme_book, character)"),
        ("commentary_poem_idx", "commentary(poem_id)"),
        (
            "poem_rhyme_group_idx",
            "poem_rhyme_group(rhyme_book, rhyme_group, poem_id)",
        ),
    ] {
        let expected = format!("CREATE INDEX {name} ON {columns};");
        assert!(
            schema.contains(&expected),
            "随包 schema 里没有 `{expected}`；索引名或列序改了，断言必须同步"
        );
    }
}
