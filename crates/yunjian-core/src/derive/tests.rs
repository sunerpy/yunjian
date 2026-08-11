use super::*;
use rusqlite::params;

fn corpus(poems: &[(&str, &str)]) -> Connection {
    corpus_with_mode(poems, "full")
}

fn corpus_with_mode(poems: &[(&str, &str)], detail_mode: &str) -> Connection {
    let connection = Connection::open_in_memory().expect("打开内存库");
    connection
        .execute_batch(
            "CREATE TABLE poem (
                 stable_id TEXT PRIMARY KEY NOT NULL,
                 body TEXT NOT NULL
             );
             CREATE TABLE corpus_meta (
                 singleton INTEGER PRIMARY KEY NOT NULL CHECK (singleton = 1),
                 index_detail_mode TEXT NOT NULL
             );",
        )
        .expect("建 fixture 表");
    connection
        .execute(
            "INSERT INTO corpus_meta(singleton, index_detail_mode) VALUES (1, ?1)",
            params![detail_mode],
        )
        .expect("写裁决形态");
    for (stable_id, body) in poems {
        connection
            .execute(
                "INSERT INTO poem(stable_id, body) VALUES (?1, ?2)",
                params![stable_id, body],
            )
            .expect("写 fixture");
    }
    connection
}

fn grams_of(connection: &Connection, stable_id: &str) -> Vec<String> {
    let mut statement = connection
        .prepare("SELECT gram FROM ngram WHERE stable_id=?1 ORDER BY gram")
        .expect("prepare");
    statement
        .query_map(params![stable_id], |row| row.get::<_, String>(0))
        .expect("query")
        .collect::<rusqlite::Result<Vec<_>>>()
        .expect("collect")
}

#[test]
fn derived_grams_cover_one_and_two_characters_and_cross_line_boundaries() {
    let mut connection = corpus(&[("a", "床前明月光，\n疑是地上霜。")]);
    build_derived_indexes(&mut connection).expect("首启构建");
    let grams = grams_of(&connection, "a");
    for expected in ["床", "明", "床前", "明月", "月光"] {
        assert!(grams.iter().any(|g| g == expected), "缺候选 {expected}");
    }
    // 「光疑」跨句：换行与逗号都被滤掉，两字候选因此跨句连续——用户查「明月」时
    // 不关心它是否恰好落在同一句里。
    assert!(
        grams.iter().any(|g| g == "光疑"),
        "两字候选必须跨句边界，否则整句检索会在句读处断掉"
    );
    assert!(!grams.iter().any(|g| g.contains('，') || g.contains('\n')));
}

#[test]
fn grams_are_deduplicated_per_poem_but_not_across_poems() {
    let mut connection = corpus(&[("a", "明明明"), ("b", "明")]);
    build_derived_indexes(&mut connection).expect("首启构建");
    assert_eq!(grams_of(&connection, "a"), vec!["明", "明明"]);
    assert_eq!(grams_of(&connection, "b"), vec!["明"]);
}

#[test]
fn last_chars_are_derived_per_line_with_punctuation_stripped() {
    let mut connection = corpus(&[("a", "床前明月光，疑是地上霜。举头望明月，低头思故乡。")]);
    build_derived_indexes(&mut connection).expect("首启构建");
    let mut statement = connection
        .prepare("SELECT line_index, ch FROM poem_last_char WHERE poem_id='a' ORDER BY line_index")
        .expect("prepare");
    let rows = statement
        .query_map([], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })
        .expect("query")
        .collect::<rusqlite::Result<Vec<_>>>()
        .expect("collect");
    assert_eq!(rows, vec![(0, "霜".to_owned()), (1, "乡".to_owned())]);
    assert!(rows.iter().all(|(_, character)| character != "月"));
}

#[test]
fn rhyme_and_metrical_splitters_have_deliberately_different_comma_semantics() {
    let body = "床前明月光，疑是地上霜。举头望明月，低头思故乡。";
    let rhyme_feet = split_rhyme_feet(body)
        .filter_map(last_character)
        .collect::<Vec<_>>();
    let metrical_lines = split_metrical_lines(body).collect::<Vec<_>>();

    assert_eq!(rhyme_feet, vec!['霜', '乡']);
    assert_eq!(
        metrical_lines,
        ["床前明月光", "疑是地上霜", "举头望明月", "低头思故乡"]
    );
}

#[test]
fn the_fts_table_is_built_from_the_mode_recorded_in_corpus_meta() {
    // 裁决的牙齿：`corpus_meta.index_detail_mode` 是构建期把实测结论刻进工件的结果，
    // 首启必须照它建。改掉那一列就改掉了运行时真正建出来的索引形态。
    for mode in ["full", "column"] {
        let mut connection = corpus_with_mode(&[("a", "床前明月光")], mode);
        build_derived_indexes(&mut connection).expect("首启构建");
        let ddl: String = connection
            .query_row(
                "SELECT sql FROM sqlite_schema WHERE type='table' AND name='poem_fts'",
                [],
                |row| row.get(0),
            )
            .expect("read ddl");
        assert!(ddl.contains(&format!("detail={mode}")), "实际 DDL：{ddl}");
        assert!(ddl.contains("content='poem'"), "必须是 external-content");
    }
}

#[test]
fn an_illegal_recorded_detail_mode_is_rejected_before_anything_is_built() {
    let mut connection = corpus_with_mode(&[("a", "床前明月光")], "trigram");
    let error = build_derived_indexes(&mut connection).expect_err("非法形态必须被拒");
    assert!(
        error.to_string().contains("detail 模式非法"),
        "unexpected: {error}"
    );
    assert!(
        !derived_indexes_present(&connection).expect("探测"),
        "被拒的构建不得留下半成品"
    );
}

#[test]
fn the_external_content_fts_leaves_no_shadow_content_table() {
    // 影子内容表是正文的第二份副本，实测可以超过文件的一半。首启构建同样不能引入它。
    let mut connection = corpus(&[("a", "床前明月光")]);
    build_derived_indexes(&mut connection).expect("首启构建");
    let shadow: i64 = connection
        .query_row(
            "SELECT count(*) FROM pragma_table_list WHERE name='poem_fts_content'",
            [],
            |row| row.get(0),
        )
        .expect("probe shadow");
    assert_eq!(shadow, 0);
}

#[test]
fn build_is_idempotent_and_reports_measured_counts_per_step() {
    let mut connection = corpus(&[("a", "床前明月光"), ("b", "国破山河在")]);
    let first = build_derived_indexes(&mut connection).expect("首次构建");
    assert_eq!(first.poems, 2);
    assert!(first.grams > 0);
    assert_eq!(first.last_chars, 2);
    assert!(first.elapsed >= first.ngram_elapsed);

    let rows: i64 = connection
        .query_row("SELECT count(*) FROM ngram", [], |row| row.get(0))
        .expect("count");
    assert_eq!(rows, i64::try_from(first.grams).expect("行数"));

    let second = build_derived_indexes(&mut connection).expect("重复构建");
    assert_eq!(second.grams, first.grams, "重复构建必须幂等，不得累加");
    assert_eq!(second.last_chars, first.last_chars);
}

#[test]
fn absent_structures_are_reported_before_any_query_runs() {
    let connection = corpus(&[("a", "床前明月光")]);
    assert!(!derived_indexes_present(&connection).expect("探测"));
    let error = verify_derived_indexes(&connection).expect_err("未构建时必须报错");
    assert!(
        error.to_string().contains("首启派生结构不完整"),
        "unexpected: {error}"
    );
}

#[test]
fn a_table_without_its_covering_index_counts_as_not_built() {
    let mut connection = corpus(&[("a", "床前明月光")]);
    build_derived_indexes(&mut connection).expect("首启构建");
    connection
        .execute_batch("DROP INDEX ngram_gram_idx;")
        .expect("只删索引");
    assert!(
        !derived_indexes_present(&connection).expect("探测"),
        "只有表没有覆盖索引时两字查询会全表扫描，等同于没建"
    );
}

#[test]
fn a_poem_whose_body_is_only_punctuation_is_not_expected_to_derive_anything() {
    // 实测唐宋集合里有 176 首的正文就是一个 `。`（上游空记录）。它们派生不出候选与
    // 尾字，那是正确行为——把总首数当期望值会让首启构建在真实语料上必然失败。
    let mut connection = corpus(&[("a", "床前明月光"), ("empty", "。")]);
    let stats = build_derived_indexes(&mut connection).expect("含空记录时首启仍应成功");
    assert_eq!(stats.poems, 2, "统计口径是全部诗条");
    verify_derived_indexes(&connection).expect("空正文的诗不参与覆盖判据");
    let covered: i64 = connection
        .query_row(
            "SELECT count(*) FROM (SELECT DISTINCT stable_id FROM ngram)",
            [],
            |row| row.get(0),
        )
        .expect("count");
    assert_eq!(covered, 1, "只有有正文的那一首进候选表");
}

#[test]
fn a_partially_covered_structure_is_rejected() {
    let mut connection = corpus(&[("a", "床前明月光"), ("b", "国破山河在")]);
    build_derived_indexes(&mut connection).expect("首启构建");
    connection
        .execute("DELETE FROM ngram WHERE stable_id='b'", [])
        .expect("删掉一首的候选");
    let error = verify_derived_indexes(&connection).expect_err("漏首必须被拒");
    assert!(error.to_string().contains("覆盖"), "unexpected: {error}");
}

#[test]
fn a_missing_last_char_row_is_rejected_too() {
    let mut connection = corpus(&[("a", "床前明月光"), ("b", "国破山河在")]);
    build_derived_indexes(&mut connection).expect("首启构建");
    connection
        .execute("DELETE FROM poem_last_char WHERE poem_id='b'", [])
        .expect("删掉一首的句尾字");
    let error = verify_derived_indexes(&connection).expect_err("漏首必须被拒");
    assert!(
        error.to_string().contains("poem_last_char"),
        "错误必须点名是哪张结构：{error}"
    );
}

#[test]
fn a_detail_mode_drifting_from_corpus_meta_is_rejected() {
    let mut connection = corpus(&[("a", "床前明月光")]);
    build_derived_indexes(&mut connection).expect("首启构建");
    connection
        .execute("UPDATE corpus_meta SET index_detail_mode='column'", [])
        .expect("篡改记录的形态");
    let error = verify_derived_indexes(&connection).expect_err("形态漂移必须被拒");
    let message = error.to_string();
    assert!(
        message.contains("column") && message.contains("full"),
        "错误必须同时报出两个形态：{message}"
    );
}

#[test]
fn two_character_lookup_uses_the_covering_index_after_first_launch_build() {
    let mut connection = corpus(&[("a", "床前明月光"), ("b", "明月松间照")]);
    build_derived_indexes(&mut connection).expect("首启构建");
    let mut statement = connection
        .prepare(
            "EXPLAIN QUERY PLAN
             SELECT n.stable_id FROM ngram AS n WHERE n.gram = ?1 ORDER BY n.stable_id",
        )
        .expect("prepare explain");
    let plan = statement
        .query_map(params!["明月"], |row| row.get::<_, String>(3))
        .expect("query explain")
        .collect::<rusqlite::Result<Vec<_>>>()
        .expect("collect explain");
    assert!(
        plan.iter()
            .any(|line| line.contains("USING COVERING INDEX ngram_gram_idx")),
        "首启构建出来的候选表必须走覆盖索引，否则等于没有加速：{plan:?}"
    );
}

#[test]
fn the_last_char_lookup_uses_its_index_after_first_launch_build() {
    let mut connection = corpus(&[("a", "床前明月光"), ("b", "明月松间照")]);
    build_derived_indexes(&mut connection).expect("首启构建");
    let mut statement = connection
        .prepare("EXPLAIN QUERY PLAN SELECT poem_id FROM poem_last_char WHERE ch = ?1")
        .expect("prepare explain");
    let plan = statement
        .query_map(params!["光"], |row| row.get::<_, String>(3))
        .expect("query explain")
        .collect::<rusqlite::Result<Vec<_>>>()
        .expect("collect explain");
    assert!(
        plan.iter().any(|line| line.contains("poem_last_char_idx")),
        "尾字检索必须走索引：{plan:?}"
    );
}

#[test]
fn an_interrupted_build_leaves_no_half_populated_structure() {
    let mut connection = corpus(&[("a", "床前明月光")]);
    build_derived_indexes(&mut connection).expect("先建一次");
    // 模拟「构建中途进程死掉」：事务未提交即回滚。灌数据在事务里，所以回滚后
    // 不会留下一张只灌了一半的表。
    let transaction = connection.transaction().expect("开事务");
    transaction
        .execute("DELETE FROM ngram", [])
        .expect("事务内清空");
    transaction.rollback().expect("回滚");
    verify_derived_indexes(&connection).expect("回滚后原结构仍然完整可用");
}
