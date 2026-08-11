use criterion::{Criterion, criterion_group, criterion_main};
use rusqlite::{Connection, params};
use std::hint::black_box;
use std::path::{Path, PathBuf};
use yunjian_core::{
    AuthorSearchRequest, CorpusConfig, CorpusHandle, FirstLineSearchRequest, RhymeBook,
    RhymeGroupSearchRequest, TagBrowseRequest, TextSearchRequest, ToneFilter, Yunjian,
};

struct Fixture {
    directory: PathBuf,
    path: PathBuf,
    api: Yunjian,
}

impl Fixture {
    fn new() -> Self {
        let directory = std::env::temp_dir().join(format!(
            "yunjian-representative-bench-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).expect("创建 Criterion fixture 目录");
        let path = directory.join("corpus.db");
        write_fixture(&path);
        let corpus = CorpusHandle::open(&Self::config_for(&directory, &path))
            .expect("打开 Criterion fixture");
        Self {
            directory,
            path,
            api: Yunjian::new(corpus),
        }
    }

    fn config(&self) -> CorpusConfig {
        Self::config_for(&self.directory, &self.path)
    }

    fn config_for(directory: &Path, path: &Path) -> CorpusConfig {
        CorpusConfig {
            path: Some(path.to_owned()),
            data_dir: directory.to_owned(),
            archive: None,
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}

fn write_fixture(path: &Path) {
    let mut connection = Connection::open(path).expect("创建 Criterion fixture 数据库");
    connection
        .execute_batch(include_str!("../../yunjian-corpus/schema.sql"))
        .expect("创建 Criterion fixture schema");
    let transaction = connection
        .transaction()
        .expect("开始 Criterion fixture 事务");
    for author in ["李白", "王安石", "苏轼"] {
        transaction
            .execute("INSERT INTO author(name) VALUES (?1)", [author])
            .expect("写 Criterion 作者");
    }
    let poems = [
        (
            "fixture:jingyesi",
            "静夜思",
            None,
            "李白",
            "唐",
            "床前明月光，疑是地上霜。举头望明月，低头思故乡。",
            "床前明月光",
            r#"["光","霜","月","乡"]"#,
            "group:jingyesi",
        ),
        (
            "fixture:bochuanguazhou",
            "泊船瓜洲",
            None,
            "王安石",
            "宋",
            "京口瓜洲一水间，钟山只隔数重山。春风又绿江南岸，明月何时照我还。",
            "京口瓜洲一水间",
            r#"["间","山","岸","还"]"#,
            "group:bochuanguazhou",
        ),
        (
            "fixture:shuidiaogetou",
            "水调歌头·明月几时有",
            Some("水调歌头"),
            "苏轼",
            "宋",
            "明月几时有，把酒问青天。不知天上宫阙，今夕是何年。",
            "明月几时有",
            r#"["有","天","阙","年"]"#,
            "group:shuidiaogetou",
        ),
    ];
    for (id, title, tune, author, dynasty, body, first_line, last_chars, work_group) in poems {
        transaction
            .execute(
                "INSERT INTO poem(
                    stable_id, content_hash, source_locator, source_locator_kind, genre,
                    title, title_raw, ci_tune, author, dynasty, dynasty_raw, body,
                    body_original, script, first_line, last_chars, line_count, char_count,
                    provenance_source, provenance_revision, provenance_kind,
                    provenance_license, provenance_license_class, work_group, edition_group
                 ) VALUES (
                    ?1, ?2, ?3, 'native', ?4, ?5, ?5, ?6, ?7, ?8, ?8, ?9,
                    ?9, 'simplified', ?10, ?11, 4, ?12, 'criterion', 'fixture',
                    '原文', 'public-domain', 'public_domain', ?13, ?14
                 )",
                params![
                    id,
                    format!("hash:{id}"),
                    format!("source:{id}"),
                    if tune.is_some() { "ci" } else { "shi" },
                    title,
                    tune,
                    author,
                    dynasty,
                    body,
                    first_line,
                    last_chars,
                    body.chars()
                        .filter(|character| !character.is_ascii_punctuation())
                        .count() as i64,
                    work_group,
                    format!("edition:{id}"),
                ],
            )
            .expect("写 Criterion 作品");
    }
    transaction
        .execute("INSERT INTO tag(name) VALUES ('思乡')", [])
        .expect("写 Criterion 标签");
    transaction
        .execute(
            "INSERT INTO poem_tag(poem_id, tag) VALUES ('fixture:jingyesi', '思乡')",
            [],
        )
        .expect("写 Criterion 作品标签");
    for character in ["光", "霜"] {
        transaction
            .execute(
                "INSERT INTO rhyme(rhyme_book, rhyme_group, tone, tone_raw, character)
                 VALUES ('pingshui', '七阳', 'level', '下平', ?1)",
                [character],
            )
            .expect("写 Criterion 韵书字");
    }
    transaction
        .execute(
            "INSERT INTO poem_rhyme_group(
                poem_id, rhyme_book, rhyme_group, tone, confidence
             ) VALUES ('fixture:jingyesi', 'pingshui', '七阳', 'level', 'unambiguous')",
            [],
        )
        .expect("写 Criterion 作品韵部");
    transaction
        .execute(
            "INSERT INTO corpus_meta(
                singleton, schema_version, corpus_version, built_at,
                source_manifest_sha256, poem_count, finding_count, input_row_count,
                index_detail_mode, derived_indexes, shipped_scope,
                builder_sqlite_version, integrity_check
             ) VALUES (
                1, ?1, 'criterion-v1', '2026-08-11T00:00:00Z', ?2, 3, 0, 3,
                'none', 'first_launch', '10k', 'fixture', 'ok'
             )",
            params![yunjian_core::SCHEMA_VERSION, "0".repeat(64)],
        )
        .expect("写 Criterion 语料元数据");
    transaction.commit().expect("提交 Criterion fixture");
}

fn representative_queries(criterion: &mut Criterion) {
    let fixture = Fixture::new();

    criterion.bench_function("two_char_ngram", |bench| {
        bench.iter(|| {
            black_box(
                fixture
                    .api
                    .search_text(TextSearchRequest::new(black_box("明月")))
                    .expect("两字查询"),
            )
        });
    });
    criterion.bench_function("three_char_match", |bench| {
        bench.iter(|| {
            black_box(
                fixture
                    .api
                    .search_text(TextSearchRequest::new(black_box("明月光")))
                    .expect("三字查询"),
            )
        });
    });
    criterion.bench_function("full_line_like", |bench| {
        bench.iter(|| {
            black_box(
                fixture
                    .api
                    .search_text(TextSearchRequest::new(black_box("床前明月光")))
                    .expect("整句查询"),
            )
        });
    });
    criterion.bench_function("author_lookup", |bench| {
        bench.iter(|| {
            black_box(
                fixture
                    .api
                    .find_by_author(AuthorSearchRequest {
                        query: black_box("李白").to_owned(),
                        cursor: None,
                    })
                    .expect("作者查询"),
            )
        });
    });
    criterion.bench_function("first_line_prefix", |bench| {
        bench.iter(|| {
            black_box(
                fixture
                    .api
                    .find_by_first_line(FirstLineSearchRequest {
                        prefix: black_box("床前").to_owned(),
                        cursor: None,
                    })
                    .expect("首句前缀查询"),
            )
        });
    });
    criterion.bench_function("rhyme_group_join", |bench| {
        bench.iter(|| {
            black_box(
                fixture
                    .api
                    .find_by_rhyme_group(RhymeGroupSearchRequest {
                        book: RhymeBook::Pingshui,
                        rhyme_group: black_box("七阳").to_owned(),
                        tone: ToneFilter::Any,
                    })
                    .expect("韵部查询"),
            )
        });
    });
    criterion.bench_function("tag_filter", |bench| {
        bench.iter(|| {
            black_box(
                fixture
                    .api
                    .browse_by_tag(TagBrowseRequest {
                        tag: black_box("思乡").to_owned(),
                        cursor: None,
                    })
                    .expect("标签查询"),
            )
        });
    });
    criterion.bench_function("cold_open_first_query", |bench| {
        bench.iter(|| {
            let api = Yunjian::new(
                CorpusHandle::open(black_box(&fixture.config())).expect("冷启动打开 fixture"),
            );
            black_box(
                api.search_text(TextSearchRequest::new("明月"))
                    .expect("冷启动首查"),
            )
        });
    });
}

fn criterion_config() -> Criterion {
    Criterion::default().output_directory(Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/benchmark-data"
    )))
}

criterion_group! {
    name = benches;
    config = criterion_config();
    targets = representative_queries
}
criterion_main!(benches);
