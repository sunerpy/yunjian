//! 元数据检索的单元测试。
//!
//! fixture 建在随仓的黄金契约 fixture（`tests/fixtures/poems.toml`）之上，**不改动它**
//! ——那份文件与 `tests/queries.toml` 同为不可变契约。本模块另加六条只在这里存在的记录，
//! 覆盖契约不含的形态：裸词牌、空格分隔的体裁/序号标记、未过白名单的 `·` 题目、
//! 跨朝代 `dynasty_raw`、带残留的作者字段，以及《赤壁》双归属。
//!
//! 派生表 `poem_last_char` **不由本模块写入**，而是由 `CorpusHandle::open` 触发生产代码
//! `derive::build_derived_indexes` 从 `poem.body` 现场派生。因此尾字用例验的是真实派生
//! 结果，而不是测试自己塞进去的答案。

use super::*;
use crate::{CorpusConfig, SCHEMA_VERSION};
use rusqlite::{Connection, params};
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);

// ---------------------------------------------------------------- 契约与 fixture 数据

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

#[derive(Debug, Deserialize)]
struct SharedFixtures {
    #[serde(rename = "variant")]
    variants: Vec<SharedVariant>,
    #[serde(rename = "poem")]
    poems: Vec<SharedPoem>,
}

#[derive(Debug, Deserialize)]
struct SharedVariant {
    from: String,
    to: String,
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
}

/// 只在本模块存在的补充记录。
struct ExtraPoem {
    stable_id: &'static str,
    title: &'static str,
    ci_tune: Option<&'static str>,
    author: &'static str,
    dynasty: &'static str,
    dynasty_raw: &'static str,
    body: &'static str,
    first_line: &'static str,
    note: &'static str,
}

/// 六条补充记录。每一条都对应一个契约覆盖不到的真实上游形态。
const EXTRA_POEMS: &[ExtraPoem] = &[
    ExtraPoem {
        stable_id: "fixture:song-qinguan-wanghaichao",
        title: "望海潮",
        ci_tune: Some("望海潮"),
        author: "秦观",
        dynasty: "宋",
        dynasty_raw: "宋",
        body: "梅英疏淡，冰澌溶泄，东风暗换年华。",
        first_line: "梅英疏淡",
        note: "第一种题目约定：裸词牌。题目与词牌同值，题目等值与词牌等值两条分支必须都能命中。",
    },
    ExtraPoem {
        stable_id: "fixture:tang-libai-gufeng-qiyi",
        title: "古风 其一",
        ci_tune: None,
        author: "李白",
        dynasty: "唐",
        dynasty_raw: "唐",
        body: "大雅久不作，吾衰竟谁陈。",
        first_line: "大雅久不作",
        note: "第三种题目约定：题目后带独立序号/体裁标记，且分隔符是空格而非中点——\
               因为题首没过词牌白名单，入库时不会被改写成中点。查『古风』要能命中。",
    },
    ExtraPoem {
        stable_id: "fixture:unknown-tune-mouti",
        title: "无名调·某题",
        ci_tune: None,
        author: "佚名",
        dynasty: "宋",
        dynasty_raw: "宋",
        body: "此调不在词牌白名单内，故 ci_tune 为空。",
        first_line: "此调不在词牌白名单内",
        note: "白名单的反证：题目里有中点但题首未过白名单，因此 ci_tune 为空。\
               查后半段『某题』不得命中它——尾段分支只对白名单词牌成立。",
    },
    ExtraPoem {
        stable_id: "fixture:tang-hanwo-yishi",
        title: "已凉",
        ci_tune: None,
        author: "韩偓",
        dynasty: "唐",
        dynasty_raw: "唐末宋初",
        body: "碧阑干外绣帘垂，猩色屏风画折枝。",
        first_line: "碧阑干外绣帘垂",
        note: "跨朝代标签：规范键是『唐』而上游原串是『唐末宋初』。结果里两者都要在。",
    },
    ExtraPoem {
        stable_id: "fixture:qing-wangyun-jushi",
        title: "菊石",
        ci_tune: None,
        author: "王筠（1784-1854）",
        dynasty: "清",
        dynasty_raw: "清",
        body: "秋花含蕊向人开，石畔孤根不受埃。",
        first_line: "秋花含蕊向人开",
        note: "上游作者字段混进现代小传的残留形态。入库已按全角左括号截断，但判据是\
               『文本是否前现代』而非字段名，因此不能假定库里一定干净——查『王筠』要命中。",
    },
    ExtraPoem {
        stable_id: "fixture:xianqin-shijing-caiwei",
        title: "采薇",
        ci_tune: None,
        author: "佚名",
        dynasty: "先秦",
        dynasty_raw: "先秦",
        body: "采薇采薇，薇亦作止。曰归曰归，岁亦莫止。",
        first_line: "采薇采薇",
        note: "同一首里『止』出现在第 1 与第 3 句末。尾字检索的结果单位是『诗』而不是\
               『句』，因此它必须只返回一条，且句序号取最小的那个。",
    },
];

/// 《赤壁》双归属：同一正文挂在杜牧与李商隐名下。
///
/// 这是上游 `chinese-poetry` issue #232 的真实案例，也是 `work_group` 刻意不含作者的
/// 全部理由。两条记录的 `work_group` 相同、作者不同、出处不同。
const DUAL_ATTRIBUTION_BODY: &str = "折戟沉沙铁未销，自将磨洗认前朝。";
const DUAL_ATTRIBUTION: &[(&str, &str, &str)] = &[
    ("fixture:tang-dumu-chibi", "杜牧", "quanTangshi:52-1"),
    (
        "fixture:tang-lishangyin-chibi",
        "李商隐",
        "quanTangshi:539-7",
    ),
];

fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn contract() -> Contract {
    let path = manifest_dir().join("tests/queries.toml");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("读取契约失败 {}：{error}", path.display()));
    toml::from_str(&text).unwrap_or_else(|error| panic!("解析契约失败 {}：{error}", path.display()))
}

fn shared_fixtures() -> SharedFixtures {
    let path = manifest_dir().join("tests/fixtures/poems.toml");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("读取 fixture 失败 {}：{error}", path.display()));
    toml::from_str(&text)
        .unwrap_or_else(|error| panic!("解析 fixture 失败 {}：{error}", path.display()))
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
    build_fixture(true)
}

/// `with_first_line_index = false` 时刻意漏建 `poem_first_line_idx`，供计划检查的
/// 可证伪对照使用。
fn build_fixture(with_first_line_index: bool) -> Fixture {
    let dir = std::env::temp_dir().join(format!(
        "yunjian-meta-{}-{}",
        std::process::id(),
        NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("创建元数据 fixture 目录");
    let path = dir.join("corpus.db");
    write_fixture(&path, with_first_line_index);
    let handle = CorpusHandle::open(&CorpusConfig {
        path: Some(path),
        data_dir: dir.clone(),
        archive: None,
    })
    .expect("打开元数据 fixture");
    assert!(
        handle.derived().is_ready(),
        "fixture 的首启派生必须成功，否则 poem_last_char 不存在：{:?}",
        handle.derived()
    );
    Fixture { dir, handle }
}

fn write_fixture(path: &Path, with_first_line_index: bool) {
    let connection = Connection::open(path).expect("创建元数据 fixture 数据库");
    // 列与索引逐字取自 `crates/yunjian-corpus/schema.sql`：索引名进 `EXPLAIN QUERY PLAN`
    // 的断言，名字对不上断言就验的不是生产索引。
    connection
        .execute_batch(
            "CREATE TABLE poem(
                 stable_id TEXT PRIMARY KEY NOT NULL,
                 genre TEXT NOT NULL,
                 title TEXT NOT NULL,
                 title_raw TEXT NOT NULL,
                 ci_tune TEXT,
                 author TEXT NOT NULL,
                 dynasty TEXT NOT NULL,
                 dynasty_raw TEXT NOT NULL,
                 body TEXT NOT NULL,
                 first_line TEXT NOT NULL,
                 last_chars TEXT NOT NULL CHECK (json_valid(last_chars)),
                 line_count INTEGER NOT NULL,
                 char_count INTEGER NOT NULL,
                 source_locator TEXT NOT NULL UNIQUE,
                 provenance_source TEXT NOT NULL,
                 provenance_revision TEXT NOT NULL,
                 work_group TEXT NOT NULL,
                 edition_group TEXT NOT NULL
             );
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
             CREATE INDEX poem_author_idx ON poem(author);
             CREATE INDEX poem_dynasty_idx ON poem(dynasty);
             CREATE INDEX poem_title_idx ON poem(title);
             CREATE INDEX poem_ci_tune_idx ON poem(ci_tune);
             CREATE INDEX poem_work_group_idx ON poem(work_group);",
        )
        .expect("创建元数据 fixture schema");
    if with_first_line_index {
        connection
            .execute_batch("CREATE INDEX poem_first_line_idx ON poem(first_line);")
            .expect("创建首句索引");
    }

    let fixtures = shared_fixtures();
    let mut count = 0_i64;
    for poem in &fixtures.poems {
        insert_poem(
            &connection,
            &poem.stable_id,
            &poem.title,
            (!poem.ci_tune.is_empty()).then(|| poem.ci_tune.clone()),
            &poem.author,
            &poem.dynasty,
            &poem.dynasty,
            &poem.body,
            &poem.first_line,
            &format!("fixture:{}", poem.stable_id),
            "golden-fixture",
        );
        count += 1;
    }
    for extra in EXTRA_POEMS {
        assert!(
            extra.note.chars().count() >= 10,
            "补充记录 {} 必须写清它为什么存在",
            extra.stable_id
        );
        insert_poem(
            &connection,
            extra.stable_id,
            extra.title,
            extra.ci_tune.map(str::to_owned),
            extra.author,
            extra.dynasty,
            extra.dynasty_raw,
            extra.body,
            extra.first_line,
            &format!("fixture:{}", extra.stable_id),
            "meta-fixture",
        );
        count += 1;
    }
    for (stable_id, author, locator) in DUAL_ATTRIBUTION {
        insert_poem(
            &connection,
            stable_id,
            "赤壁",
            None,
            author,
            "唐",
            "唐",
            DUAL_ATTRIBUTION_BODY,
            "折戟沉沙铁未销",
            locator,
            "chinese-poetry",
        );
        count += 1;
    }
    for variant in &fixtures.variants {
        connection
            .execute(
                "INSERT INTO variant_map(src_char, dst_char) VALUES (?1, ?2)",
                params![variant.from, variant.to],
            )
            .expect("写 variant_map");
    }
    connection
        .execute(
            "INSERT INTO corpus_meta VALUES \
             (1, ?1, 'meta-fixture-v1', '2026-08-11T00:00:00Z', ?2, 'full', \
              'first_launch', '10k', 'ok')",
            params![SCHEMA_VERSION, count],
        )
        .expect("写 corpus_meta");
    connection.close().expect("关闭元数据 fixture 数据库");
}

#[allow(clippy::too_many_arguments)]
fn insert_poem(
    connection: &Connection,
    stable_id: &str,
    title: &str,
    ci_tune: Option<String>,
    author: &str,
    dynasty: &str,
    dynasty_raw: &str,
    body: &str,
    first_line: &str,
    source_locator: &str,
    provenance_source: &str,
) {
    let lines: Vec<&str> = body
        .split(['\n', '，', '。', '！', '？', '；'])
        .filter(|line| !line.is_empty())
        .collect();
    let last_chars: Vec<String> = lines
        .iter()
        .filter_map(|line| line.chars().last().map(String::from))
        .collect();
    let genre = if ci_tune.is_some() { "ci" } else { "shi" };
    connection
        .execute(
            "INSERT INTO poem(stable_id, genre, title, title_raw, ci_tune, author, dynasty, \
             dynasty_raw, body, first_line, last_chars, line_count, char_count, source_locator, \
             provenance_source, provenance_revision, work_group, edition_group) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18)",
            params![
                stable_id,
                genre,
                title,
                title,
                ci_tune,
                author,
                dynasty,
                dynasty_raw,
                body,
                first_line,
                serde_json::to_string(&last_chars).expect("序列化 last_chars"),
                lines.len() as i64,
                body.chars().filter(|c| !crate::is_punctuation(*c)).count() as i64,
                source_locator,
                provenance_source,
                "fixture-rev",
                group_key(body),
                group_key(&format!("{author}{body}")),
            ],
        )
        .expect("写 fixture 诗");
}

/// fixture 用的分组键。
///
/// 真实语料的 `work_group` 是 `blake3(去空白去标点正文)[:12]`，这里只需要它的那条**性质**：
/// 同一正文得同一个键，且**与作者无关**。因此用一个不引依赖的 FNV-1a 折叠出同样宽度的
/// 十六进制串即可——被测的是「冲突可被检出」，不是哈希算法本身。
fn group_key(seed: &str) -> String {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in seed
        .chars()
        .filter(|c| !c.is_whitespace() && !crate::is_punctuation(*c))
        .flat_map(|c| {
            let mut buffer = [0_u8; 4];
            c.encode_utf8(&mut buffer);
            buffer.into_iter().take(c.len_utf8()).collect::<Vec<_>>()
        })
    {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")[..12].to_owned()
}

// ---------------------------------------------------------------- 计划检查

/// 生产 SQL 的 `EXPLAIN QUERY PLAN` 逐行。
fn explain(connection: &Connection, sql: &str, binds: usize) -> Vec<String> {
    let placeholders: Vec<Value> = (0..binds).map(|_| Value::Text("x".to_owned())).collect();
    let mut statement = connection
        .prepare(&format!("EXPLAIN QUERY PLAN {sql}"))
        .unwrap_or_else(|error| panic!("prepare explain 失败：{error}\nSQL: {sql}"));
    statement
        .query_map(params_from_iter(placeholders), |row| {
            row.get::<_, String>(3)
        })
        .expect("执行 explain")
        .collect::<rusqlite::Result<Vec<_>>>()
        .expect("收集 explain")
}

/// 基表别名。它们出现在 `SCAN` 行上且不带 `USING` 时，说明退化成了整表扫描。
const BASE_ALIASES: [&str; 4] = ["poem", "p", "l", "poem_last_char"];

/// 计划必须走 B-tree 且不碰 FTS。
///
/// 写成返回 `Result` 而不是直接 `assert!`：可证伪对照
/// （[`dropping_the_first_line_index_makes_the_plan_inspection_fail`]）要断言这套判据
/// **真的会红**，而 `assert!` 版本无法在测试内部被检验。
fn check_btree_plan(lines: &[String], required_index: &str) -> std::result::Result<(), String> {
    if let Some(line) = lines.iter().find(|line| line.contains("poem_fts")) {
        return Err(format!("元数据检索不得引用 poem_fts：{line}"));
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

fn assert_btree_plan(label: &str, lines: &[String], required_index: &str) {
    if let Err(reason) = check_btree_plan(lines, required_index) {
        panic!("{label} 的查询计划不合格：{reason}\n计划：{lines:?}");
    }
}

fn ids(page: &MetaPage) -> Vec<&str> {
    page.hits.iter().map(|hit| hit.stable_id.as_str()).collect()
}

fn hit<'a>(page: &'a MetaPage, stable_id: &str) -> &'a MetaHit {
    page.hits
        .iter()
        .find(|hit| hit.stable_id == stable_id)
        .unwrap_or_else(|| panic!("{stable_id} 不在命中集合里：{:?}", ids(page)))
}

// ---------------------------------------------------------------- 题目：三种约定

#[test]
fn the_three_upstream_title_conventions_all_resolve() {
    let fixture = fixture();
    let handle = &fixture.handle;

    // 约定一：裸词牌。题目与词牌同值，等值分支就够。
    let bare = find_by_title(handle, "望海潮", None).expect("查裸词牌");
    assert_eq!(
        ids(&bare),
        vec!["fixture:song-qinguan-wanghaichao"],
        "裸词牌必须命中同名词作"
    );
    assert_eq!(
        hit(&bare, "fixture:song-qinguan-wanghaichao").matched_on,
        MetaMatch::Title
    );

    // 约定二：词牌·题目。整串、前半段、后半段三种输入都要命中同一首。
    let combined = find_by_title(handle, "念奴娇·赤壁怀古", None).expect("查合成题目");
    assert_eq!(
        ids(&combined),
        vec!["fixture:song-sushi-niannujiao-chibihuaigu"]
    );
    assert_eq!(
        hit(&combined, "fixture:song-sushi-niannujiao-chibihuaigu").matched_on,
        MetaMatch::Title
    );

    let tune = find_by_title(handle, "念奴娇", None).expect("查词牌");
    assert!(
        ids(&tune).contains(&"fixture:song-sushi-niannujiao-chibihuaigu"),
        "『念奴娇』必须命中《念奴娇·赤壁怀古》：{:?}",
        ids(&tune)
    );
    assert_eq!(
        hit(&tune, "fixture:song-sushi-niannujiao-chibihuaigu").matched_on,
        MetaMatch::CiTune,
        "词牌命中要标成 CiTune，这一条同时是白名单校验"
    );

    let tail = find_by_title(handle, "赤壁怀古", None).expect("查合成题目后半段");
    assert!(
        ids(&tail).contains(&"fixture:song-sushi-niannujiao-chibihuaigu"),
        "『赤壁怀古』必须命中《念奴娇·赤壁怀古》：{:?}",
        ids(&tail)
    );
    assert_eq!(
        hit(&tail, "fixture:song-sushi-niannujiao-chibihuaigu").matched_on,
        MetaMatch::TitleTail
    );

    // 约定三：题目 + 独立标记，分隔符是空格。
    let marker = find_by_title(handle, "古风", None).expect("查带标记的题目");
    assert_eq!(ids(&marker), vec!["fixture:tang-libai-gufeng-qiyi"]);
    assert_eq!(
        hit(&marker, "fixture:tang-libai-gufeng-qiyi").matched_on,
        MetaMatch::TitleHead,
        "空格形态只能由前缀分支命中：题首没过白名单，ci_tune 是空的"
    );
}

#[test]
fn the_tune_half_is_validated_against_the_allow_list() {
    let fixture = fixture();
    let handle = &fixture.handle;

    // 题目里有中点，但题首未过词牌白名单（入库因此没有写 ci_tune）。
    let head = find_by_title(handle, "无名调", None).expect("查未过白名单的题首");
    assert_eq!(ids(&head), vec!["fixture:unknown-tune-mouti"]);
    assert_eq!(
        hit(&head, "fixture:unknown-tune-mouti").matched_on,
        MetaMatch::TitleHead,
        "未过白名单的题首只能算题目组分，不得被当成词牌"
    );
    assert_eq!(
        hit(&head, "fixture:unknown-tune-mouti").ci_tune,
        None,
        "它的 ci_tune 必须是空的，否则这条反证不成立"
    );

    // 后半段分支只对白名单词牌成立，所以查『某题』不得命中它。
    let tail = find_by_title(handle, "某题", None).expect("查未过白名单的题尾");
    assert!(
        !ids(&tail).contains(&"fixture:unknown-tune-mouti"),
        "题首不在白名单时，后半段不得被当作可检索的题目组分：{:?}",
        ids(&tail)
    );

    // 反面对照：白名单内的词牌，后半段就能命中。
    let allowed = find_by_title(handle, "明月几时有", None).expect("查白名单词牌的题尾");
    assert!(
        ids(&allowed).contains(&"fixture:song-sushi-shuidiaogetou-mingyue"),
        "『明月几时有』应经白名单词牌『水调歌头』命中：{:?}",
        ids(&allowed)
    );
}

#[test]
fn title_branches_are_mutually_exclusive_so_a_poem_is_never_returned_twice() {
    let fixture = fixture();
    for query in ["望海潮", "念奴娇", "赤壁怀古", "古风", "静夜思"] {
        let page = find_by_title(&fixture.handle, query, None).expect("查题目");
        let mut seen = ids(&page);
        let before = seen.len();
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(
            seen.len(),
            before,
            "查询「{query}」返回了重复的 stable_id：{:?}",
            ids(&page)
        );
    }
}

// ---------------------------------------------------------------- 作者与归属冲突

#[test]
fn a_conflicting_attribution_returns_both_authors_with_their_sources() {
    let fixture = fixture();
    let handle = &fixture.handle;

    let detail = author_detail(handle, "杜牧", None).expect("查杜牧详情");
    assert_eq!(detail.poem_count, 1);
    assert_eq!(ids(&detail.page), vec!["fixture:tang-dumu-chibi"]);

    let conflicts = &detail.attribution_conflicts;
    assert_eq!(
        conflicts.len(),
        1,
        "《赤壁》双归属必须被报成一处冲突：{conflicts:?}"
    );
    let conflict = &conflicts[0];
    assert_eq!(
        conflict.authors(),
        vec!["李商隐", "杜牧"],
        "两个归属都要在，不得静默选一个"
    );
    assert_eq!(conflict.attributions.len(), 2);
    for attribution in &conflict.attributions {
        assert!(
            !attribution.source_locator.is_empty()
                && !attribution.provenance_source.is_empty()
                && !attribution.provenance_revision.is_empty(),
            "每个归属都必须带出处，否则冲突无法核对：{attribution:?}"
        );
    }
    assert!(
        !conflict.work_group.is_empty(),
        "冲突必须带上分组键，调用方才能自己展开"
    );

    // 从另一位作者查，同一处冲突必须同样可见——冲突不依赖从哪一侧进入。
    let mirror = author_detail(handle, "李商隐", None).expect("查李商隐详情");
    assert_eq!(mirror.attribution_conflicts.len(), 1);
    assert_eq!(
        mirror.attribution_conflicts[0].work_group,
        conflict.work_group
    );

    // work_group 刻意不含作者：两条记录同组正是冲突可检出的机制本身。
    let attributions =
        find_work_group_attributions(handle, &conflict.work_group).expect("列出分组归属");
    assert_eq!(attributions.len(), 2);
}

#[test]
fn author_lookup_tolerates_a_dirty_upstream_author_value() {
    let fixture = fixture();
    let page = find_by_author(&fixture.handle, "王筠", None).expect("查带残留的作者");
    assert_eq!(ids(&page), vec!["fixture:qing-wangyun-jushi"]);
    let found = hit(&page, "fixture:qing-wangyun-jushi");
    assert_eq!(
        found.matched_on,
        MetaMatch::AuthorPrefix,
        "残留值只能由前缀分支命中；等值分支匹配不上它"
    );
    assert_eq!(
        found.author, "王筠（1784-1854）",
        "作者值必须原样返回，不得替调用方清洗掉"
    );

    let detail = author_detail(&fixture.handle, "王筠", None).expect("查带残留的作者详情");
    assert_eq!(
        detail.matched_names,
        vec!["王筠（1784-1854）".to_owned()],
        "实际命中的 author 值要单独列出来，调用方才知道自己拿到的是什么"
    );
}

#[test]
fn exact_author_matches_rank_before_prefix_matches() {
    let fixture = fixture();
    let detail = author_detail(&fixture.handle, "李白", None).expect("查李白详情");
    assert!(
        detail.poem_count >= 6,
        "李白应有多首：{}",
        detail.poem_count
    );
    assert!(
        detail
            .page
            .hits
            .iter()
            .all(|hit| hit.matched_on == MetaMatch::Author),
        "『李白』在 fixture 里是干净的等值命中"
    );
    assert_eq!(detail.matched_names, vec!["李白".to_owned()]);
    assert_eq!(
        detail.dynasties,
        vec![DynastyLabel {
            canonical: "唐".to_owned(),
            raw: "唐".to_owned()
        }]
    );
}

// ---------------------------------------------------------------- 朝代

#[test]
fn dynasty_results_expose_both_the_canonical_key_and_the_raw_label() {
    let fixture = fixture();
    let page = browse_by_dynasty(&fixture.handle, "唐", None).expect("浏览唐");
    let crossover = hit(&page, "fixture:tang-hanwo-yishi");
    assert_eq!(
        crossover.dynasty,
        DynastyLabel {
            canonical: "唐".to_owned(),
            raw: "唐末宋初".to_owned()
        },
        "跨朝代标签的原串不得被归一化掉"
    );
    assert!(
        page.hits.iter().all(|hit| hit.dynasty.canonical == "唐"),
        "浏览结果必须全是该朝代"
    );
}

#[test]
fn an_unknown_dynasty_key_is_an_error_rather_than_an_empty_page() {
    let fixture = fixture();
    let error =
        browse_by_dynasty(&fixture.handle, "民国", None).expect_err("语料里没有的朝代键必须报错");
    let rendered = error.to_string();
    assert!(
        rendered.contains("民国") && rendered.contains("唐"),
        "报错要说清缺的是哪个键、现有哪些键：{rendered}"
    );
}

// ---------------------------------------------------------------- 首句与尾字

#[test]
fn first_line_prefix_matches_the_precomputed_column() {
    let fixture = fixture();
    let handle = &fixture.handle;

    let page = find_by_first_line(handle, "床前", None).expect("查首句前缀");
    assert_eq!(ids(&page), vec!["fixture:tang-libai-jingyesi"]);
    assert_eq!(
        hit(&page, "fixture:tang-libai-jingyesi").matched_on,
        MetaMatch::FirstLinePrefix
    );

    let whole = find_by_first_line(handle, "床前明月光", None).expect("查整句首句");
    assert_eq!(
        hit(&whole, "fixture:tang-libai-jingyesi").matched_on,
        MetaMatch::FirstLine,
        "整句首句要走等值分支并排在前"
    );

    // 首句检索不得混进正文命中：《水调歌头·明月几时有》的首句是「明月几时有」，
    // 而《静夜思》正文里也有「明月」——两者不能串。
    let ambiguous = find_by_first_line(handle, "明月", None).expect("查歧义首句前缀");
    assert_eq!(
        ids(&ambiguous),
        vec!["fixture:song-sushi-shuidiaogetou-mingyue"]
    );
}

#[test]
fn last_char_search_reads_the_derived_table_and_counts_poems_not_lines() {
    let fixture = fixture();
    let handle = &fixture.handle;

    let page = find_by_last_char(handle, "光", None).expect("查尾字");
    assert_eq!(ids(&page), vec!["fixture:tang-libai-jingyesi"]);
    assert_eq!(
        hit(&page, "fixture:tang-libai-jingyesi").matched_line_index,
        Some(0),
        "「光」是《静夜思》的第 0 句末字"
    );

    // 《枫桥夜泊》正文有「霜」但不在句末：这一条区分「句末字」与「正文含此字」。
    let shuang = find_by_last_char(handle, "霜", None).expect("查句末霜");
    assert_eq!(ids(&shuang), vec!["fixture:tang-libai-jingyesi"]);
    assert!(
        !ids(&shuang).contains(&"fixture:tang-zhangji-fengqiaoyebo"),
        "「霜」在《枫桥夜泊》里不在句末，不得命中"
    );

    // 《采薇》第 1、3 句末字都是「止」，结果单位是「诗」，故只返回一条且取最小句序号。
    let repeated = find_by_last_char(handle, "止", None).expect("查重复出现的句末字");
    assert_eq!(ids(&repeated), vec!["fixture:xianqin-shijing-caiwei"]);
    assert_eq!(
        hit(&repeated, "fixture:xianqin-shijing-caiwei").matched_line_index,
        Some(1)
    );

    assert!(
        find_by_last_char(handle, "明月", None).is_err(),
        "尾字检索只接受单字"
    );
}

// ---------------------------------------------------------------- 计划检查

#[test]
fn every_metadata_query_uses_a_btree_index_and_never_touches_the_fts_table() {
    let fixture = fixture();
    let connection = fixture.handle.connect().expect("打开只读连接");
    let cases: [(&str, String, usize, &str); 10] = [
        ("题目等值", title_exact_sql(), 3, "poem_title_idx"),
        ("词牌等值", ci_tune_exact_sql(), 4, "poem_ci_tune_idx"),
        ("题目前半段", title_head_sql(), 5, "poem_title_idx"),
        ("题目后半段", title_tail_sql(), 3, "poem_title_idx"),
        ("作者等值", author_exact_sql(), 3, "poem_author_idx"),
        ("作者前缀", author_prefix_sql(), 5, "poem_author_idx"),
        ("朝代", dynasty_sql(), 3, "poem_dynasty_idx"),
        (
            "首句前缀",
            first_line_prefix_sql(),
            5,
            "poem_first_line_idx",
        ),
        ("尾字", last_char_sql(), 3, "poem_last_char_idx"),
        ("分组归属", work_group_sql(), 1, "poem_work_group_idx"),
    ];
    for (label, sql, binds, index) in cases {
        let lines = explain(&connection, &sql, binds);
        assert_btree_plan(label, &lines, index);
    }
    // 首句等值走的是同一张索引，单独列出以免它被漏掉。
    let lines = explain(&connection, &first_line_exact_sql(), 3);
    assert_btree_plan("首句等值", &lines, "poem_first_line_idx");
}

/// 可证伪对照：漏建 `poem_first_line_idx` 时，上面那条计划断言必须变红。
///
/// 没有这一条，[`every_metadata_query_uses_a_btree_index_and_never_touches_the_fts_table`]
/// 就可能只是一句恒真的空话。
#[test]
fn dropping_the_first_line_index_makes_the_plan_inspection_fail() {
    let fixture = build_fixture(false);
    let connection = fixture.handle.connect().expect("打开只读连接");
    for sql in [first_line_prefix_sql(), first_line_exact_sql()] {
        let binds = if sql.contains("?5") { 5 } else { 3 };
        let lines = explain(&connection, &sql, binds);
        let verdict = check_btree_plan(&lines, "poem_first_line_idx");
        assert!(
            verdict.is_err(),
            "漏建首句索引后计划检查竟然通过了，说明它没在验索引使用：{lines:?}"
        );
    }
    // 同一份 fixture 上其它分支照旧合格，证明失败是索引缺失导致的，不是判据太严。
    let lines = explain(&connection, &title_exact_sql(), 3);
    assert_btree_plan("题目等值", &lines, "poem_title_idx");
}

// ---------------------------------------------------------------- 分页与确定性

#[test]
fn paging_through_the_cursor_skips_nothing_and_repeats_nothing() {
    let fixture = fixture();
    let handle = &fixture.handle;
    let all = browse_by_dynasty(handle, "唐", None).expect("浏览唐");
    assert!(all.hits.len() >= 4, "唐诗 fixture 太少，撑不起分页断言");
    assert!(
        all.next_cursor.is_none(),
        "fixture 规模低于上限，首页不该有续页游标"
    );

    // 从第一条之后续读，逐条走完，断言序列与一次取全完全一致。
    let mut walked = Vec::new();
    let mut cursor = Some(format!("0:{}", all.hits[0].stable_id));
    walked.push(all.hits[0].stable_id.clone());
    while let Some(current) = cursor {
        let page = browse_by_dynasty(handle, "唐", Some(&current)).expect("续页");
        if page.hits.is_empty() {
            break;
        }
        walked.push(page.hits[0].stable_id.clone());
        cursor = Some(format!("0:{}", page.hits[0].stable_id));
        if walked.len() > all.hits.len() {
            panic!("续页产生了多余的行：{walked:?}");
        }
    }
    assert_eq!(
        walked,
        all.hits
            .iter()
            .map(|hit| hit.stable_id.clone())
            .collect::<Vec<_>>(),
        "逐条续页的序列必须与一次取全一致"
    );
}

#[test]
fn identical_input_yields_identical_ordering_across_runs() {
    let fixture = fixture();
    let first = browse_by_dynasty(&fixture.handle, "唐", None).expect("第一次浏览");
    let second = browse_by_dynasty(&fixture.handle, "唐", None).expect("第二次浏览");
    assert_eq!(first, second, "同一输入两次调用的结果必须逐字段相同");
    assert!(
        first
            .hits
            .windows(2)
            .all(|pair| pair[0].stable_id < pair[1].stable_id),
        "同一分支内必须按 stable_id 严格升序，跨运行才稳定"
    );
}

#[test]
fn a_malformed_cursor_is_rejected_instead_of_being_ignored() {
    let fixture = fixture();
    for bad in ["", "唐", "0:", "x:abc"] {
        assert!(
            browse_by_dynasty(&fixture.handle, "唐", Some(bad)).is_err(),
            "非法游标 {bad:?} 必须报错而不是被当成首页"
        );
    }
}

#[test]
fn punctuation_only_input_returns_an_empty_page_without_querying() {
    let fixture = fixture();
    let handle = &fixture.handle;
    for page in [
        find_by_title(handle, "，。！", None).expect("题目"),
        find_by_author(handle, "，。！", None).expect("作者"),
        find_by_first_line(handle, "，。！", None).expect("首句"),
        find_by_last_char(handle, "，。！", None).expect("尾字"),
    ] {
        assert!(page.hits.is_empty() && page.next_cursor.is_none());
        assert!(page.normalized.is_empty());
    }
}

#[test]
fn prefix_upper_bound_is_exclusive_and_survives_the_surrogate_gap() {
    // 末字抬一个码位：前 U+524D -> 剎 U+524E，白 U+767D -> 百 U+767E。
    assert_eq!(prefix_upper_bound("床前").as_deref(), Some("床剎"));
    assert_eq!(prefix_upper_bound("李白").as_deref(), Some("李百"));
    // U+D7FF 的下一个码位落在 UTF-16 代理区，必须跳到 U+E000。
    assert_eq!(
        prefix_upper_bound("\u{d7ff}").as_deref(),
        Some("\u{e000}"),
        "上界不得落进 char 不允许的代理区"
    );
    // 整串到顶时没有上界；调用方据此只用等值分支。
    assert_eq!(prefix_upper_bound(&char::MAX.to_string()), None);
    assert_eq!(prefix_upper_bound(""), None);
    // 抬末字时要退位：`a` + `char::MAX` 的上界是 `b`。
    assert_eq!(
        prefix_upper_bound(&format!("a{}", char::MAX)).as_deref(),
        Some("b")
    );
}

// ---------------------------------------------------------------- 黄金契约

/// 契约里五类元数据条目全部由**生产函数**满足。
///
/// 与 `tests/golden_queries.rs` 的区别：那里用的是一份刻意与生产代码解耦的参考实现，
/// 只证明契约可满足；这里调用的是真正会上线的 `find_by_*`，证明生产代码满足契约。
/// 契约文件仍然只有一份，本用例读它而不复制它。
#[test]
fn the_five_metadata_contract_classes_pass_against_the_production_functions() {
    let fixture = fixture();
    let handle = &fixture.handle;
    let contract = contract();
    let mut covered = 0;
    for entry in &contract.queries {
        let page = match entry.class.as_str() {
            "title_lookup" | "ci_tune_lookup" | "ci_tune_title_lookup" => {
                find_by_title(handle, &entry.query, None)
            }
            "first_line_prefix" => find_by_first_line(handle, &entry.query, None),
            "last_char_lookup" => find_by_last_char(handle, &entry.query, None),
            _ => continue,
        }
        .unwrap_or_else(|error| panic!("{}: 执行契约查询失败：{error}", entry.id));
        covered += 1;
        let hits = ids(&page);
        assert!(
            hits.len() >= entry.expect_min_hits,
            "{}: 查询「{}」只命中 {} 首（{hits:?}），低于契约下界 {}",
            entry.id,
            entry.query,
            hits.len(),
            entry.expect_min_hits
        );
        // 断言锚在命中集合里，而不是排在第一条——与 `tests/golden_queries.rs` 的
        // `check()` 同一判据。元数据检索没有领域权重可言：`流` 在《登鹳雀楼》与
        // 《黄鹤楼送孟浩然之广陵》里同样是句末字，凭空规定哪首更靠前就是编一个排序。
        assert!(
            hits.contains(&entry.expect_top_id.as_str()),
            "{}: 契约的锚 {} 不在命中集合里：{hits:?}",
            entry.id,
            entry.expect_top_id
        );
    }
    assert_eq!(
        covered, 10,
        "五类元数据条目共 10 条，实际跑了 {covered} 条——契约被改动过就要同步这个数"
    );
}
