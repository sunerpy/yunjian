use super::*;
use crate::{CorpusConfig, SCHEMA_VERSION};
use rusqlite::{Connection, params};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);

/// 随包 schema 的路径。跨 crate 只读一个文件，不引入依赖——`yunjian-core` 不能依赖
/// `yunjian-corpus`，但两者共用数据库里那几个稳定键，所以守卫直接读源文件。
const CORPUS_SCHEMA_PATH: &str = "../yunjian-corpus/schema.sql";

struct Fixture {
    dir: PathBuf,
    handle: CorpusHandle,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// 韵书行：(韵书, 韵部, 声调, 上游原始声键, 字)。
///
/// 形态刻意与真实语料一致：`rhyme_group` 是韵书自身的分组键（`一东`，**不带**声部前缀），
/// 声调另存一列。用户输入的「下平七阳」在检索侧剥前缀，而不是在数据里拼成整串——
/// 拼整串的合成库会让「前缀剥离」这条逻辑测不出来。
const PINGSHUI_ROWS: &[(&str, &str, &str, &str)] = &[
    ("一东", "level", "上平声部", "东"),
    ("一东", "level", "上平声部", "同"),
    ("一东", "level", "上平声部", "中"),
    ("一东", "level", "上平声部", "空"),
    ("二冬", "level", "上平声部", "冬"),
    ("二冬", "level", "上平声部", "松"),
    ("一董", "rising", "上声部", "董"),
    ("一董", "rising", "上声部", "空"),
    ("一送", "departing", "去声部", "送"),
    ("一送", "departing", "去声部", "空"),
    ("七阳", "level", "下平声部", "阳"),
    ("七阳", "level", "下平声部", "光"),
    ("七阳", "level", "下平声部", "霜"),
    ("七阳", "level", "下平声部", "乡"),
    ("十二侵", "level", "下平声部", "深"),
    ("十二侵", "level", "下平声部", "心"),
    ("十二侵", "level", "下平声部", "金"),
    ("十二侵", "level", "下平声部", "簪"),
];

/// 词林正韵把平水韵的一东与二冬并进第一部，上去两声并成「仄声」。
const CILIN_ROWS: &[(&str, &str, &str, &str)] = &[
    ("第一部", "level", "平声", "东"),
    ("第一部", "level", "平声", "同"),
    ("第一部", "level", "平声", "中"),
    ("第一部", "level", "平声", "冬"),
    ("第一部", "level", "平声", "松"),
    ("第一部", "oblique", "仄声", "董"),
    ("第一部", "oblique", "仄声", "送"),
];

/// 作品的韵部归属：(作品, 韵书, 韵部, 声调, 可信度)。
const GROUP_ROWS: &[(&str, &str, &str, &str, &str)] = &[
    (
        "fixture:tang-libai-jingyesi",
        "pingshui",
        "七阳",
        "level",
        "unambiguous",
    ),
    (
        "fixture:tang-dufu-chunwang",
        "pingshui",
        "十二侵",
        "level",
        "resolved_by_vote",
    ),
    (
        "fixture:unresolved-kongshan",
        "pingshui",
        "一东",
        "level",
        "unresolved",
    ),
    (
        "fixture:unresolved-kongshan",
        "pingshui",
        "一董",
        "rising",
        "unresolved",
    ),
    (
        "fixture:tang-libai-jingyesi-pair",
        "pingshui",
        "一东",
        "level",
        "unambiguous",
    ),
    (
        "fixture:song-ci-diyibu",
        "cilin",
        "第一部",
        "level",
        "unambiguous",
    ),
];

const POEMS: &[(&str, &str, &str)] = &[
    ("fixture:tang-libai-jingyesi", "静夜思", "李白"),
    ("fixture:tang-dufu-chunwang", "春望", "杜甫"),
    ("fixture:unresolved-kongshan", "鹿柴", "王维"),
    ("fixture:tang-libai-jingyesi-pair", "题西林壁", "苏轼"),
    ("fixture:song-ci-diyibu", "水调歌头·明月几时有", "苏轼"),
];

fn fixture() -> Fixture {
    let dir = std::env::temp_dir().join(format!(
        "yunjian-rhyme-search-{}-{}",
        std::process::id(),
        NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("创建韵部检索 fixture 目录");
    let path = dir.join("corpus.db");
    write_fixture(&path);
    let handle = CorpusHandle::open(&CorpusConfig {
        path: Some(path),
        data_dir: dir.clone(),
        archive: None,
    })
    .expect("打开韵部检索 fixture");
    Fixture { dir, handle }
}

fn write_fixture(path: &Path) {
    let connection = Connection::open(path).expect("创建韵部检索 fixture 数据库");
    connection
        .execute_batch(
            "CREATE TABLE poem(
                 stable_id TEXT PRIMARY KEY NOT NULL,
                 body TEXT NOT NULL,
                 title TEXT NOT NULL,
                 author TEXT NOT NULL
             );
             CREATE TABLE rhyme(
                 rhyme_book TEXT NOT NULL,
                 rhyme_group TEXT NOT NULL,
                 tone TEXT NOT NULL,
                 tone_raw TEXT NOT NULL,
                 character TEXT NOT NULL,
                 PRIMARY KEY (rhyme_book, rhyme_group, tone, character)
             ) WITHOUT ROWID;
             CREATE TABLE poem_rhyme_group(
                 poem_id TEXT NOT NULL REFERENCES poem(stable_id),
                 rhyme_book TEXT NOT NULL,
                 rhyme_group TEXT NOT NULL,
                 tone TEXT NOT NULL,
                 confidence TEXT NOT NULL,
                 PRIMARY KEY (poem_id, rhyme_book, rhyme_group, tone)
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
             CREATE INDEX poem_rhyme_group_idx ON poem_rhyme_group(rhyme_book, rhyme_group, poem_id);
             CREATE INDEX rhyme_character_idx ON rhyme(rhyme_book, character);",
        )
        .expect("创建韵部检索 fixture schema");

    for (stable_id, title, author) in POEMS {
        connection
            .execute(
                "INSERT INTO poem(stable_id, body, title, author) VALUES (?1, ?2, ?3, ?4)",
                params![stable_id, "床前明月光，疑是地上霜。", title, author],
            )
            .expect("写 fixture 诗");
    }
    for (book, rows) in [("pingshui", PINGSHUI_ROWS), ("cilin", CILIN_ROWS)] {
        for (rhyme_group, tone, tone_raw, character) in rows {
            connection
                .execute(
                    "INSERT INTO rhyme(rhyme_book, rhyme_group, tone, tone_raw, character) \
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![book, rhyme_group, tone, tone_raw, character],
                )
                .expect("写 fixture 韵书行");
        }
    }
    for (poem_id, book, rhyme_group, tone, confidence) in GROUP_ROWS {
        connection
            .execute(
                "INSERT INTO poem_rhyme_group(poem_id, rhyme_book, rhyme_group, tone, confidence) \
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![poem_id, book, rhyme_group, tone, confidence],
            )
            .expect("写 fixture 韵部归属");
    }
    connection
        .execute(
            "INSERT INTO corpus_meta VALUES \
             (1, ?1, 'fixture-v1', '2026-08-11T00:00:00Z', ?2, 'full', 'first_launch', '10k', 'ok')",
            params![SCHEMA_VERSION, POEMS.len() as i64],
        )
        .expect("写 corpus_meta");
    connection.close().expect("关闭韵部检索 fixture 数据库");
}

fn shared_groups(answer: &RhymeAnswer) -> Vec<(&str, RhymeTone)> {
    answer
        .shared
        .iter()
        .map(|group| (group.rhyme_group.as_str(), group.tone))
        .collect()
}

fn hit_ids(matches: &RhymeGroupMatches) -> Vec<&str> {
    matches
        .hits
        .iter()
        .map(|hit| hit.poem_id.as_str())
        .collect()
}

fn explain(connection: &Connection, sql: &str, binds: &[Option<&str>]) -> Vec<String> {
    let mut statement = connection
        .prepare(&format!("EXPLAIN QUERY PLAN {sql}"))
        .expect("prepare explain");
    statement
        .query_map(rusqlite::params_from_iter(binds), |row| {
            row.get::<_, String>(3)
        })
        .expect("执行 explain")
        .collect::<rusqlite::Result<Vec<_>>>()
        .expect("收集 explain")
}

/// 一东里的两个字在平水韵下相押。
#[test]
fn two_characters_in_one_dong_rhyme_under_pingshui() {
    let fixture = fixture();
    let answer =
        do_these_rhyme(&fixture.handle, &['东', '同'], RhymeBook::Pingshui).expect("判断押韵");
    assert_eq!(answer.verdict(), RhymeVerdict::Rhyme);
    assert_eq!(shared_groups(&answer), [("一东", RhymeTone::Level)]);
    assert!(answer.not_in_book.is_empty(), "{answer:?}");
}

/// 同一对字在两本韵书下给出不同答案：东与冬在平水韵分属一东与二冬，在词林正韵同归第一部。
///
/// 这条是「韵书必须是显式维度」的实证：若 `book` 有隐式默认，两个答案里必然有一个是错的，
/// 而调用方无从知道自己拿到了哪一个。
#[test]
fn one_pair_gets_book_dependent_answers() {
    let fixture = fixture();
    let pair = ['东', '冬'];

    let pingshui = do_these_rhyme(&fixture.handle, &pair, RhymeBook::Pingshui).expect("平水韵判断");
    assert_eq!(pingshui.verdict(), RhymeVerdict::NoRhyme);
    assert!(pingshui.shared.is_empty(), "{pingshui:?}");
    // 否定是真的否定：两个字都在平水韵里，不是查不到。
    assert!(pingshui.not_in_book.is_empty(), "{pingshui:?}");

    let cilin = do_these_rhyme(&fixture.handle, &pair, RhymeBook::Cilin).expect("词林正韵判断");
    assert_eq!(cilin.verdict(), RhymeVerdict::Rhyme);
    assert_eq!(shared_groups(&cilin), [("第一部", RhymeTone::Level)]);
}

/// 被扣留的韵书必须是类型化错误，绝不能表现为空结果集或「不押韵」。
#[test]
fn a_withheld_book_is_an_error_never_an_empty_answer() {
    let fixture = fixture();

    let answer = do_these_rhyme(&fixture.handle, &['东', '同'], RhymeBook::Xinyun);
    let groups = rhyme_groups_of(&fixture.handle, '东', RhymeBook::Xinyun);
    let matches = find_by_rhyme_group(&fixture.handle, RhymeBook::Xinyun, "一东", ToneFilter::Any);

    for (label, error) in [
        ("do_these_rhyme", answer.err()),
        ("rhyme_groups_of", groups.err()),
        ("find_by_rhyme_group", matches.err()),
    ] {
        let error = error.unwrap_or_else(|| panic!("{label} 对中华新韵必须报错而不是返回结果"));
        assert!(
            matches!(
                error,
                Error::RhymeBookUnavailable {
                    book: RhymeBook::Xinyun,
                    ..
                }
            ),
            "{label} 的错误必须是 RhymeBookUnavailable：{error:?}"
        );
        let rendered = format!("{error}");
        assert!(rendered.contains("中华新韵"), "{label}: {rendered}");
        assert!(rendered.contains("未随包分发"), "{label}: {rendered}");
        assert!(
            !rendered.contains("不押韵"),
            "{label} 不得把缺书说成不押韵：{rendered}"
        );
    }
}

/// `unresolved` 的韵脚不产生肯定判断，但也不被丢弃。
#[test]
fn unresolved_feet_never_produce_a_positive_claim() {
    let fixture = fixture();
    let matches = find_by_rhyme_group(
        &fixture.handle,
        RhymeBook::Pingshui,
        "一东",
        ToneFilter::Any,
    )
    .expect("检索一东");

    assert_eq!(hit_ids(&matches), ["fixture:tang-libai-jingyesi-pair"]);
    assert!(
        matches
            .hits
            .iter()
            .all(|hit| hit.confidence.is_positive_claim()),
        "{matches:?}"
    );
    assert_eq!(
        matches
            .unresolved
            .iter()
            .map(|hit| hit.poem_id.as_str())
            .collect::<Vec<_>>(),
        ["fixture:unresolved-kongshan"],
        "未消歧的作品必须保留在候选里：既不算命中，也不算不存在"
    );
    assert!(
        matches
            .unresolved
            .iter()
            .all(|hit| hit.confidence == RhymeConfidence::Unresolved),
        "{matches:?}"
    );
}

/// 可信度逐条透出，UI 才能区分投票解出的与本就无歧义的。
#[test]
fn confidence_travels_with_every_hit() {
    let fixture = fixture();
    for (group, expected) in [
        ("七阳", RhymeConfidence::Unambiguous),
        ("十二侵", RhymeConfidence::ResolvedByVote),
    ] {
        let matches =
            find_by_rhyme_group(&fixture.handle, RhymeBook::Pingshui, group, ToneFilter::Any)
                .expect("检索韵部");
        let hit = matches.hits.first().expect("该韵部应有命中");
        assert_eq!(hit.confidence, expected, "{group}");
        assert!(!hit.title.is_empty() && !hit.author.is_empty(), "{hit:?}");
    }
}

/// 用户输入的声部前缀被剥掉，声调仍由 `tone` 参数承载。
#[test]
fn a_tone_section_prefix_is_stripped_and_tone_still_filters() {
    let fixture = fixture();
    let prefixed = find_by_rhyme_group(
        &fixture.handle,
        RhymeBook::Pingshui,
        "下平七阳",
        ToneFilter::Any,
    )
    .expect("带前缀检索");
    let bare = find_by_rhyme_group(
        &fixture.handle,
        RhymeBook::Pingshui,
        "七阳",
        ToneFilter::Any,
    )
    .expect("不带前缀检索");
    assert_eq!(prefixed, bare);
    assert_eq!(prefixed.rhyme_group, "七阳");

    let level = find_by_rhyme_group(
        &fixture.handle,
        RhymeBook::Pingshui,
        "七阳",
        ToneFilter::Only(RhymeTone::Level),
    )
    .expect("按平声筛选");
    assert_eq!(hit_ids(&level), ["fixture:tang-libai-jingyesi"]);

    let rising = find_by_rhyme_group(
        &fixture.handle,
        RhymeBook::Pingshui,
        "七阳",
        ToneFilter::Only(RhymeTone::Rising),
    )
    .expect("按上声筛选");
    assert!(rising.hits.is_empty(), "{rising:?}");
}

/// 拿词的韵部去查平水韵，得到的是书选错了的报错，而不是一个「查无此诗」的空集。
#[test]
fn a_ci_group_queried_against_pingshui_names_the_book_mismatch() {
    let fixture = fixture();
    let error = find_by_rhyme_group(
        &fixture.handle,
        RhymeBook::Pingshui,
        "第一部",
        ToneFilter::Any,
    )
    .expect_err("平水韵没有第一部，必须报错");
    let rendered = format!("{error}");
    assert!(rendered.contains("平水韵"), "{rendered}");
    assert!(rendered.contains("词林正韵"), "{rendered}");
    assert!(rendered.contains("第一部"), "{rendered}");

    // 反向仍然成立：词林正韵里没有平水韵的韵部名。
    let reverse = find_by_rhyme_group(&fixture.handle, RhymeBook::Cilin, "七阳", ToneFilter::Any)
        .expect_err("词林正韵没有七阳");
    assert!(format!("{reverse}").contains("词林正韵"), "{reverse}");
}

/// 多音字按任一读音都能相押，不需要事先猜它读哪个音。
#[test]
fn a_polyphone_matches_on_any_of_its_readings() {
    let fixture = fixture();
    let readings = rhyme_groups_of(&fixture.handle, '空', RhymeBook::Pingshui).expect("查空的读音");
    assert_eq!(
        readings
            .iter()
            .map(|group| (group.rhyme_group.as_str(), group.tone))
            .collect::<Vec<_>>(),
        [
            ("一东", RhymeTone::Level),
            ("一董", RhymeTone::Rising),
            ("一送", RhymeTone::Departing),
        ]
    );

    for (partner, group, tone) in [
        ('东', "一东", RhymeTone::Level),
        ('董', "一董", RhymeTone::Rising),
        ('送', "一送", RhymeTone::Departing),
    ] {
        let answer = do_these_rhyme(&fixture.handle, &['空', partner], RhymeBook::Pingshui)
            .expect("多音字判断");
        assert_eq!(answer.verdict(), RhymeVerdict::Rhyme, "{partner}");
        assert_eq!(shared_groups(&answer), [(group, tone)], "{partner}");
    }
}

/// 韵书未收的字让结论变成不确定，而不是「不押韵」。
#[test]
fn an_uncollected_character_is_indeterminate_not_negative() {
    let fixture = fixture();
    assert!(
        rhyme_groups_of(&fixture.handle, '囧', RhymeBook::Pingshui)
            .expect("查未收字")
            .is_empty()
    );

    let answer =
        do_these_rhyme(&fixture.handle, &['东', '囧'], RhymeBook::Pingshui).expect("含未收字判断");
    assert_eq!(answer.verdict(), RhymeVerdict::Indeterminate);
    assert_eq!(answer.not_in_book, ['囧']);
    assert!(answer.shared.is_empty(), "{answer:?}");
}

/// 同部异声不构成押韵，但要能解释这个否定是怎么来的。
#[test]
fn same_group_but_different_tone_explains_its_negative() {
    let fixture = fixture();
    let answer =
        do_these_rhyme(&fixture.handle, &['东', '董'], RhymeBook::Cilin).expect("同部异声");
    assert_eq!(answer.verdict(), RhymeVerdict::NoRhyme);
    assert!(answer.shared.is_empty(), "{answer:?}");
    assert_eq!(answer.same_group_other_tone, ["第一部"]);
}

/// 一个字自己不能构成押韵判断。
#[test]
fn a_single_character_cannot_be_asked_to_rhyme() {
    let fixture = fixture();
    let error = do_these_rhyme(&fixture.handle, &['东'], RhymeBook::Pingshui)
        .expect_err("单字不足以判断押韵");
    assert!(format!("{error}").contains("至少需要两个字"), "{error}");
}

/// 两条 SQL 都走索引，且都不引用 FTS 表。
#[test]
fn queries_use_indexes_and_never_touch_the_full_text_table() {
    let fixture = fixture();
    let connection = fixture.handle.connect().expect("打开只读连接");

    // 断言前提：派生结构确实建出了 `poem_fts`，否则「计划里没有 poem_fts」是空话。
    let fts_tables: i64 = connection
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE type='table' AND name='poem_fts'",
            [],
            |row| row.get(0),
        )
        .expect("查 poem_fts 是否存在");
    assert_eq!(fts_tables, 1, "fixture 必须建出 poem_fts，断言才有意义");

    let cases = [
        (
            "韵部取作品",
            RHYME_GROUP_POEMS_SQL,
            vec![Some("pingshui"), Some("一东"), None],
            "poem_rhyme_group_idx",
        ),
        (
            "逐字取韵部",
            CHARACTER_RHYME_GROUPS_SQL,
            vec![Some("pingshui"), Some("东")],
            "rhyme_character_idx",
        ),
    ];
    for (label, sql, binds, index) in cases {
        let lines = explain(&connection, sql, &binds);
        assert!(
            lines.iter().any(|line| line.contains(index)),
            "{label} 必须走 {index}：{lines:?}"
        );
        assert!(
            lines.iter().all(|line| !line.contains("poem_fts")),
            "{label} 不得引用 poem_fts：{lines:?}"
        );
        assert!(
            lines.iter().all(|line| !line.contains("SCAN")),
            "{label} 不得出现全表扫描：{lines:?}"
        );
    }
}

/// 三个入口的 `book` 参数都是具体类型且必填。
///
/// 函数指针的类型标注就是断言：谁把 `book` 改成 `Option`、加上默认值或换成泛型
/// `impl Into<_>`，这里立刻编译不过。模块文档里另有一对 doctest 从调用点证明同一件事。
#[test]
fn every_entry_point_requires_an_explicit_book() {
    let _: fn(&CorpusHandle, RhymeBook, &str, ToneFilter) -> Result<RhymeGroupMatches> =
        find_by_rhyme_group;
    let _: fn(&CorpusHandle, &[char], RhymeBook) -> Result<RhymeAnswer> = do_these_rhyme;
    let _: fn(&CorpusHandle, char, RhymeBook) -> Result<Vec<RhymeGroupRef>> = rhyme_groups_of;
}

/// 两条 SQL 里不得出现 FTS 的痕迹。
#[test]
fn the_sql_never_mentions_the_full_text_table() {
    for sql in [RHYME_GROUP_POEMS_SQL, CHARACTER_RHYME_GROUPS_SQL] {
        assert!(!sql.contains("poem_fts"), "{sql}");
        assert!(!sql.contains("MATCH"), "{sql}");
    }
}

/// 本模块依赖的两个索引必须与随包 schema 同名同列。
///
/// 没有这条守卫，`EXPLAIN QUERY PLAN` 断言只证明 fixture 自己造的索引被用上了；
/// 随包 schema 改了索引名或列序，检索会在真实语料上退化成扫描而测试全绿。
#[test]
fn the_indexes_this_module_relies_on_exist_in_the_shipped_schema() {
    let schema =
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(CORPUS_SCHEMA_PATH))
            .expect("读取随包 schema");
    for definition in [
        "CREATE INDEX poem_rhyme_group_idx ON poem_rhyme_group(rhyme_book, rhyme_group, poem_id)",
        "CREATE INDEX rhyme_character_idx ON rhyme(rhyme_book, character)",
    ] {
        assert!(
            schema.contains(definition),
            "随包 schema 缺少 `{definition}`"
        );
    }
}
