//! 三个离线只读工具的协议级契约。
//!
//! 这里断言的是**客户端真正看得到的东西**：`tools/list` 的 JSON 里 annotation 的形状、
//! `tools/call` 结果里 `structuredContent` 与 text block 是否一致、服务端有没有真的把
//! `limit` 封住。会话跑在内存双工上的真实 `rmcp` 客户端与真实服务端之间，所以序列化、
//! schema 生成与参数反序列化全都走完整路径。
//!
//! 工具名单来自 `common` 的两个规范子集；todo 42 已补齐 AI 工具，因此这里同时守住
//! `tools/list` 恰好等于五工具全集，防止漏注册或意外暴露额外工具。

mod common;

use common::{
    ANCHOR, ANCHOR_NEIGHBOUR, EXPECTED_TOOLS_AI, EXPECTED_TOOLS_OFFLINE, Sandbox, Session, args,
    expected_tools_all, first_text, is_valid_tool_name, structured, tool_json, tool_named,
};
use serde_json::{Value, json};
use yunjian_mcp::{SEARCH_LIMIT_MAX, SIMILAR_RESULT_CAP, YunjianServer};

async fn session() -> (Sandbox, Session) {
    let sandbox = Sandbox::new();
    let session = Session::connect(YunjianServer::new(sandbox.core())).await;
    (sandbox, session)
}

#[tokio::test]
async fn the_advertised_list_is_exactly_the_canonical_five_tool_set() {
    let (_sandbox, session) = session().await;
    let tools = session.tools().await;
    let mut advertised: Vec<&str> = tools.iter().map(|tool| tool.name.as_ref()).collect();
    advertised.sort_unstable();
    assert_eq!(
        advertised,
        expected_tools_all(),
        "tools/list 必须恰好暴露规范五工具全集"
    );
    session.shutdown().await;
}

#[tokio::test]
async fn the_canonical_full_set_is_the_union_of_the_two_subsets() {
    let all = expected_tools_all();
    assert_eq!(all.len(), 5, "规范全集应为 5 个工具，实为 {all:?}");
    for name in EXPECTED_TOOLS_OFFLINE
        .iter()
        .chain(EXPECTED_TOOLS_AI.iter())
    {
        assert!(all.contains(name), "{name} 应属于规范全集");
    }
}

#[tokio::test]
async fn every_advertised_tool_name_matches_the_mcp_charset() {
    let (_sandbox, session) = session().await;
    for tool in session.tools().await {
        assert!(
            is_valid_tool_name(&tool.name),
            "工具名 {:?} 不符合 ^[A-Za-z0-9_.-]{{1,128}}$；中文只能放 annotations.title",
            tool.name
        );
    }
    session.shutdown().await;
}

#[tokio::test]
async fn each_offline_tool_declares_read_only_and_a_closed_world_explicitly() {
    let (_sandbox, session) = session().await;
    let tools = session.tools().await;
    for name in EXPECTED_TOOLS_OFFLINE {
        let json = tool_json(tool_named(&tools, name));
        let annotations = json
            .get("annotations")
            .unwrap_or_else(|| panic!("{name} 必须带 annotations；省略会让客户端每次调用都确认"));
        assert_eq!(
            annotations.get("readOnlyHint"),
            Some(&Value::Bool(true)),
            "{name} 必须显式声明 readOnlyHint: true，实际 annotations 为 {annotations}"
        );
        assert_eq!(
            annotations.get("openWorldHint"),
            Some(&Value::Bool(false)),
            "{name} 必须显式声明 openWorldHint: false，实际 annotations 为 {annotations}"
        );
    }
    session.shutdown().await;
}

#[tokio::test]
async fn each_offline_tool_carries_a_chinese_title_and_a_description() {
    let (_sandbox, session) = session().await;
    let tools = session.tools().await;
    for name in EXPECTED_TOOLS_OFFLINE {
        let tool = tool_named(&tools, name);
        let title = tool_json(tool)
            .get("annotations")
            .and_then(|annotations| annotations.get("title").cloned())
            .and_then(|title| title.as_str().map(str::to_owned))
            .unwrap_or_else(|| panic!("{name} 的 annotations 里必须有 title"));
        assert!(
            title.chars().any(|character| character as u32 > 0x2E80),
            "{name} 的 title 应为中文，实为 {title:?}"
        );
        let description = tool
            .description
            .as_deref()
            .unwrap_or_else(|| panic!("{name} 必须有 description"));
        assert!(
            description.chars().count() > 20,
            "{name} 的 description 要说清做什么、何时用，实为 {description:?}"
        );
    }
    session.shutdown().await;
}

#[tokio::test]
async fn each_offline_tool_declares_an_output_schema() {
    let (_sandbox, session) = session().await;
    let tools = session.tools().await;
    for name in EXPECTED_TOOLS_OFFLINE {
        let json = tool_json(tool_named(&tools, name));
        assert!(
            json.get("outputSchema").is_some_and(Value::is_object),
            "{name} 应由 Json<T> 生成 outputSchema，实际为 {:?}",
            json.get("outputSchema")
        );
        assert!(
            json.get("inputSchema").is_some_and(Value::is_object),
            "{name} 应有 inputSchema"
        );
    }
    session.shutdown().await;
}

#[tokio::test]
async fn every_result_carries_structured_content_and_a_matching_text_block() {
    let (_sandbox, session) = session().await;
    let calls: Vec<(&'static str, Value)> = vec![
        ("search_poem", args(vec![("query", json!("明月"))])),
        ("explain_poem", args(vec![("poem_id", json!(ANCHOR))])),
        ("find_similar_poem", args(vec![("poem_id", json!(ANCHOR))])),
    ];
    for (name, arguments) in calls {
        let result = session.call(name, arguments).await;
        assert_eq!(result.is_error, Some(false), "{name} 应成功返回");
        let structured_value = structured(&result);
        let text = first_text(&result);
        let parsed: Value = serde_json::from_str(&text)
            .unwrap_or_else(|error| panic!("{name} 的 text block 应是 JSON：{error}\n{text}"));
        assert_eq!(
            parsed, structured_value,
            "{name} 的 text block 必须与 structuredContent 解析出同一个值"
        );
    }
    session.shutdown().await;
}

#[tokio::test]
async fn an_oversized_limit_is_clamped_rather_than_rejected() {
    let (_sandbox, session) = session().await;
    let result = session
        .call(
            "search_poem",
            args(vec![("query", json!("明月")), ("limit", json!(9999))]),
        )
        .await;
    assert_eq!(
        result.is_error,
        Some(false),
        "超限的 limit 应截断而不是报错"
    );
    let structured_value = structured(&result);
    assert_eq!(
        structured_value["limit"],
        json!(SEARCH_LIMIT_MAX),
        "limit 应被截断到 {SEARCH_LIMIT_MAX}，实为 {}",
        structured_value["limit"]
    );
    assert_eq!(structured_value["limit_clamped"], json!(true));
    assert!(
        structured_value["notes"]
            .as_array()
            .is_some_and(|notes| notes
                .iter()
                .any(|note| note.as_str().is_some_and(|note| note.contains("截断")))),
        "截断必须在 notes 里说出来：{structured_value}"
    );
    session.shutdown().await;
}

#[tokio::test]
async fn the_default_limit_applies_when_the_caller_omits_it() {
    let (_sandbox, session) = session().await;
    let structured_value = structured(
        &session
            .call("search_poem", args(vec![("query", json!("明月"))]))
            .await,
    );
    assert_eq!(structured_value["limit"], json!(10));
    assert_eq!(structured_value["limit_clamped"], json!(false));
    session.shutdown().await;
}

#[tokio::test]
async fn a_two_character_query_returns_results() {
    let (_sandbox, session) = session().await;
    let structured_value = structured(
        &session
            .call("search_poem", args(vec![("query", json!("明月"))]))
            .await,
    );
    let hits = structured_value["hits"]
        .as_array()
        .expect("hits 应为数组")
        .clone();
    assert!(
        !hits.is_empty(),
        "两字查询「明月」应有命中：{structured_value}"
    );
    for hit in &hits {
        assert!(
            hit["poem_id"].as_str().is_some_and(|id| !id.is_empty()),
            "每条命中都要带可用的 poem_id：{hit}"
        );
        assert!(
            hit["snippet"].as_str().is_some_and(|text| !text.is_empty()),
            "每条命中都要带命中行：{hit}"
        );
    }
    session.shutdown().await;
}

#[tokio::test]
async fn search_reports_that_author_and_dynasty_only_filter_the_current_page() {
    let (_sandbox, session) = session().await;
    let structured_value = structured(
        &session
            .call(
                "search_poem",
                args(vec![("query", json!("明月")), ("author", json!("李白"))]),
            )
            .await,
    );
    for hit in structured_value["hits"].as_array().expect("hits 应为数组") {
        assert_eq!(hit["author"], json!("李白"), "过滤后不该出现其它作者");
    }
    assert!(
        structured_value["notes"]
            .as_array()
            .is_some_and(|notes| notes
                .iter()
                .any(|note| note.as_str().is_some_and(|note| note.contains("当前页")))),
        "只过滤本页这件事必须说出来，否则用户会读成语料里没有：{structured_value}"
    );
    session.shutdown().await;
}

#[tokio::test]
async fn explain_returns_a_citation_for_every_commentary_entry() {
    let (_sandbox, session) = session().await;
    let structured_value = structured(
        &session
            .call("explain_poem", args(vec![("poem_id", json!(ANCHOR))]))
            .await,
    );
    let commentaries = structured_value["commentaries"]
        .as_array()
        .expect("commentaries 应为数组")
        .clone();
    assert!(
        !commentaries.is_empty(),
        "锚定作品应带集评：{structured_value}"
    );
    for entry in &commentaries {
        let citation = entry
            .get("citation")
            .unwrap_or_else(|| panic!("集评 {} 缺 citation：{entry}", entry["id"]));
        for field in ["work", "author", "dynasty", "source_note"] {
            assert!(
                citation[field]
                    .as_str()
                    .is_some_and(|value| !value.is_empty()),
                "集评 {} 的出处字段 {field} 不得为空：{citation}",
                entry["id"]
            );
        }
        assert!(
            citation["work_completed_by"]
                .as_u64()
                .is_some_and(|year| year > 0),
            "集评 {} 的成书年份必须有值：{citation}",
            entry["id"]
        );
    }
    session.shutdown().await;
}

#[tokio::test]
async fn explain_keeps_unknown_tones_as_unknown_instead_of_calling_them_level() {
    let (_sandbox, session) = session().await;
    let structured_value = structured(
        &session
            .call("explain_poem", args(vec![("poem_id", json!(ANCHOR))]))
            .await,
    );
    let tones = &structured_value["tones"];
    assert_eq!(
        tones["has_unknown"],
        json!(true),
        "fixture 韵书刻意未收「床」「低」"
    );
    assert!(
        tones["unknown_count"]
            .as_u64()
            .is_some_and(|count| count > 0),
        "未知平仄的字数应大于 0：{tones}"
    );
    let unknown_cells: Vec<&Value> = tones["lines"]
        .as_array()
        .expect("lines 应为数组")
        .iter()
        .flat_map(|line| line["cells"].as_array().expect("cells 应为数组"))
        .filter(|cell| cell["tone"] == json!("unknown"))
        .collect();
    assert!(
        !unknown_cells.is_empty(),
        "未收字必须以 tone=unknown 的形式活下来，而不是被当成平声：{tones}"
    );
    session.shutdown().await;
}

#[tokio::test]
async fn explain_reports_rhyme_groups_per_book_with_a_confidence() {
    let (_sandbox, session) = session().await;
    let structured_value = structured(
        &session
            .call("explain_poem", args(vec![("poem_id", json!(ANCHOR))]))
            .await,
    );
    let groups = structured_value["rhyme_groups"]
        .as_array()
        .expect("rhyme_groups 应为数组")
        .clone();
    assert!(
        !groups.is_empty(),
        "锚定作品应有韵部归属：{structured_value}"
    );
    for group in &groups {
        assert_eq!(group["book"], json!("pingshui"));
        assert!(group["group"].as_str().is_some_and(|name| !name.is_empty()));
        assert!(
            ["unambiguous", "resolved_by_vote", "unresolved"]
                .contains(&group["confidence"].as_str().unwrap_or_default()),
            "confidence 必须是三个稳定键之一：{group}"
        );
    }
    session.shutdown().await;
}

#[tokio::test]
async fn explain_returns_work_group_alternatives_and_provenance() {
    let (_sandbox, session) = session().await;
    let structured_value = structured(
        &session
            .call("explain_poem", args(vec![("poem_id", json!(ANCHOR))]))
            .await,
    );
    assert!(
        structured_value["work_group_alternatives"].is_array(),
        "替代项必须存在，哪怕是空数组：{structured_value}"
    );
    let provenance = &structured_value["provenance"];
    for field in ["source_locator", "source", "revision", "license"] {
        assert!(
            provenance[field]
                .as_str()
                .is_some_and(|value| !value.is_empty()),
            "溯源字段 {field} 不得为空：{provenance}"
        );
    }
    assert!(
        structured_value["disclosure"]
            .as_str()
            .is_some_and(|text| text.contains("不含 AI 生成内容")),
        "结果必须声明自己不含 AI 生成内容：{structured_value}"
    );
    session.shutdown().await;
}

#[tokio::test]
async fn explain_rejects_an_unknown_poem_id_as_a_visible_tool_error() {
    let (_sandbox, session) = session().await;
    let result = session
        .call(
            "explain_poem",
            args(vec![("poem_id", json!("no-such-poem"))]),
        )
        .await;
    assert_eq!(result.is_error, Some(true), "找不到的 id 应是工具级错误");
    let structured_value = structured(&result);
    assert_eq!(structured_value["code"], json!("poem_not_found"));
    session.shutdown().await;
}

#[tokio::test]
async fn similar_scores_are_auditable_and_capped() {
    let (_sandbox, session) = session().await;
    let structured_value = structured(
        &session
            .call("find_similar_poem", args(vec![("poem_id", json!(ANCHOR))]))
            .await,
    );

    let weights = &structured_value["weights"];
    assert_eq!(weights["shared_tags"], json!(0.4));
    assert_eq!(weights["same_rhyme_group"], json!(0.25));
    assert_eq!(weights["same_ci_tune"], json!(0.2));
    assert_eq!(weights["character_overlap"], json!(0.15));
    assert!(
        structured_value["method"]
            .as_str()
            .is_some_and(|method| method.contains("非 embedding")),
        "方法说明必须点明不是 embedding：{structured_value}"
    );

    let matches = structured_value["matches"]
        .as_array()
        .expect("matches 应为数组")
        .clone();
    assert!(
        !matches.is_empty(),
        "锚定作品应能找到相关作品：{structured_value}"
    );
    assert!(
        matches.len() <= SIMILAR_RESULT_CAP,
        "结果条数应封在 {SIMILAR_RESULT_CAP} 以内，实为 {}",
        matches.len()
    );

    let mut previous = f64::MAX;
    let mut work_groups: Vec<&str> = Vec::new();
    for entry in &matches {
        assert_ne!(
            entry["poem_id"],
            json!(ANCHOR),
            "基准作品自身不该出现在结果里"
        );
        let score = entry["score"].as_f64().expect("score 应为数值");
        assert!(
            (0.0..=1.0).contains(&score),
            "得分应落在 [0,1]，实为 {score}"
        );
        assert!(score <= previous, "结果应按得分降序：{matches:?}");
        previous = score;

        let components = &entry["components"];
        let sum = [
            "shared_tags",
            "same_rhyme_group",
            "same_ci_tune",
            "character_overlap",
        ]
        .iter()
        .map(|key| components[*key].as_f64().expect("分量应为数值"))
        .sum::<f64>();
        assert!(
            (sum - score).abs() < 1e-9,
            "得分必须等于四项分量之和，否则无从复核：{entry}"
        );

        let work_group = entry["work_group"].as_str().expect("work_group 应为字符串");
        assert!(
            !work_groups.contains(&work_group),
            "同一 work_group 只应保留一条：{work_group}"
        );
        work_groups.push(work_group);
    }
    session.shutdown().await;
}

#[tokio::test]
async fn similar_by_axis_restricts_candidates_without_changing_the_weights() {
    let (_sandbox, session) = session().await;
    let all = structured(
        &session
            .call("find_similar_poem", args(vec![("poem_id", json!(ANCHOR))]))
            .await,
    );
    let themed = structured(
        &session
            .call(
                "find_similar_poem",
                args(vec![("poem_id", json!(ANCHOR)), ("by", json!("theme"))]),
            )
            .await,
    );

    assert_eq!(themed["requested_axis"], json!("theme"));
    assert_eq!(themed["axes_used"], json!(["theme"]));
    assert_eq!(
        all["axes_used"],
        json!(["theme", "rhyme", "tune", "author", "dynasty"]),
        "缺省时应取全部轴的并集"
    );
    assert_eq!(
        themed["weights"], all["weights"],
        "by 只限定候选来源，不得改变打分口径"
    );

    let themed_ids: Vec<&str> = themed["matches"]
        .as_array()
        .expect("matches 应为数组")
        .iter()
        .map(|entry| entry["poem_id"].as_str().unwrap_or_default())
        .collect();
    let all_ids: Vec<&str> = all["matches"]
        .as_array()
        .expect("matches 应为数组")
        .iter()
        .map(|entry| entry["poem_id"].as_str().unwrap_or_default())
        .collect();
    for id in &themed_ids {
        assert!(
            all_ids.contains(id),
            "单轴候选必须是全轴候选的子集：{id} 不在 {all_ids:?} 里"
        );
    }
    session.shutdown().await;
}

#[tokio::test]
async fn similar_excludes_the_most_frequent_characters_from_the_overlap_term() {
    let (_sandbox, session) = session().await;
    let structured_value = structured(
        &session
            .call("find_similar_poem", args(vec![("poem_id", json!(ANCHOR))]))
            .await,
    );
    assert!(
        structured_value["excluded_frequent_chars"]
            .as_u64()
            .is_some_and(|count| count > 0),
        "高频字表应真的算出来了，否则共享一个「不」就能造出相似：{structured_value}"
    );
    session.shutdown().await;
}

#[tokio::test]
async fn a_poem_sharing_a_tag_and_a_rhyme_group_scores_above_one_sharing_neither() {
    let (_sandbox, session) = session().await;
    let structured_value = structured(
        &session
            .call("find_similar_poem", args(vec![("poem_id", json!(ANCHOR))]))
            .await,
    );
    let matches = structured_value["matches"]
        .as_array()
        .expect("matches 应为数组")
        .clone();
    let neighbour = matches
        .iter()
        .find(|entry| entry["poem_id"] == json!(ANCHOR_NEIGHBOUR))
        .unwrap_or_else(|| {
            panic!("{ANCHOR_NEIGHBOUR} 与锚定作品共享标签「思乡」，应出现在结果里：{matches:?}")
        });
    assert!(
        neighbour["components"]["shared_tags"]
            .as_f64()
            .unwrap_or_default()
            > 0.0,
        "共享标签的分量应大于 0：{neighbour}"
    );
    assert!(
        neighbour["matched_axes"]
            .as_array()
            .is_some_and(|axes| axes.contains(&json!("theme"))),
        "命中轴应包含 theme：{neighbour}"
    );
    session.shutdown().await;
}

#[tokio::test]
async fn a_missing_corpus_yields_a_structured_error_from_every_offline_tool() {
    let session = Session::connect(YunjianServer::without_corpus()).await;
    let calls: Vec<(&'static str, Value)> = vec![
        ("search_poem", args(vec![("query", json!("明月"))])),
        ("explain_poem", args(vec![("poem_id", json!(ANCHOR))])),
        ("find_similar_poem", args(vec![("poem_id", json!(ANCHOR))])),
    ];
    for (name, arguments) in calls {
        let result = session.call(name, arguments).await;
        assert_eq!(result.is_error, Some(true), "{name} 缺语料应是工具级错误");
        let structured_value = structured(&result);
        assert_eq!(structured_value["code"], json!("corpus_missing"));
        assert!(
            structured_value["hint"]
                .as_str()
                .is_some_and(|hint| hint.contains("yunjian corpus fetch")),
            "{name} 的缺语料提示必须点名获取命令：{structured_value}"
        );
    }
    session.shutdown().await;
}
