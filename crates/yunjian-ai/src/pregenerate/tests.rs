//! 预生成门禁的用例。
//!
//! 这里验的全是**拒绝**：随包数据集能被分发的理由建立在几条硬约束上，每一条都要有
//! 一个会变红的用例守着，否则约束就只是注释。真实推理不在本文件的验证范围内——
//! 门禁本身与模型是否可达无关，也正因如此它在任何机器上都能验。

use super::{
    ANTHOLOGY_TAGS, DISCLOSURE_MARKERS, LOCAL_CACHE_TABLE, NOT_GENERATED_MARKER,
    OPEN_WEIGHT_LICENSES, OPEN_WEIGHT_PROVIDERS, OpenWeightModel, PregeneratedDataset,
    PregeneratedRecord, SHIPPED_TABLE, closed_api_provider, ensure_disclosure,
    ensure_readable_table, existing_pregenerated_ids, sha256_hex,
};
use crate::cache::{AppreciationCache, ShippedAppreciation};
use crate::genai_provider::ProviderKind;
use crate::provider::{
    APPRECIATION_TEMPLATE_VERSION, Appreciation, AppreciationRequest, ProviderId,
};
use yunjian_core::Error;

const OPEN_MODEL: &str = "deepseek-r1:7b";

fn record(stable_id: &str, text: &str) -> PregeneratedRecord {
    PregeneratedRecord {
        stable_id: stable_id.to_owned(),
        title: "静夜思".to_owned(),
        author: "李白".to_owned(),
        anthology_tags: vec!["唐诗三百首".to_owned()],
        model: OPEN_MODEL.to_owned(),
        model_license: "MIT".to_owned(),
        provider: "ollama".to_owned(),
        generated_at: 1_770_000_000,
        template_version: APPRECIATION_TEMPLATE_VERSION.to_owned(),
        grounding_digest: "a".repeat(64),
        reviewed: false,
        text: text.to_owned(),
    }
}

#[test]
fn open_weight_model_accepts_mit_weights_on_a_local_runtime() {
    let model = OpenWeightModel::new(OPEN_MODEL, "MIT", "ollama").expect("MIT + ollama 应当放行");
    assert_eq!(model.model, OPEN_MODEL);
    assert_eq!(model.model_license, "MIT");
    assert_eq!(model.provider, "ollama");
}

#[test]
fn open_weight_licence_allow_list_is_exactly_mit_and_apache() {
    assert_eq!(OPEN_WEIGHT_LICENSES, ["MIT", "Apache-2.0"]);
    for license in OPEN_WEIGHT_LICENSES {
        OpenWeightModel::new(OPEN_MODEL, license, "ollama")
            .unwrap_or_else(|error| panic!("{license} 应在白名单内：{error}"));
    }
}

#[test]
fn licence_outside_the_allow_list_is_rejected_by_name() {
    for license in [
        "CC-BY-NC-4.0",
        "GPL-3.0",
        "FunASR-Model-Open-Source-License-1.1",
        "UNVERIFIED",
        "",
    ] {
        let error = OpenWeightModel::new(OPEN_MODEL, license, "ollama")
            .expect_err("非白名单许可必须被拒绝");
        assert!(
            matches!(error, Error::PregenerationRejected(ref message) if message.contains(license)),
            "{license} 的拒绝理由应点名该许可，实际：{error}"
        );
    }
}

#[test]
fn dataset_push_rejects_a_record_whose_licence_is_not_open_weight() {
    let mut dataset = PregeneratedDataset::new(true);
    let mut row = record("poem-1", "这是一段赏析。");
    row.model_license = "CC-BY-NC-4.0".to_owned();
    let error = dataset
        .push(row)
        .expect_err("逐条校验必须与配置校验用同一判据");
    assert!(matches!(error, Error::PregenerationRejected(_)), "{error}");
    assert!(dataset.records().is_empty(), "被拒的记录不得留在数据集里");
}

#[test]
fn every_closed_api_provider_is_rejected_naming_the_open_weight_requirement() {
    let closed = ProviderKind::ALL
        .iter()
        .copied()
        .filter(|kind| !matches!(kind, ProviderKind::Ollama))
        .collect::<Vec<_>>();
    assert_eq!(
        closed.len(),
        ProviderKind::ALL.len() - 1,
        "闭源名单必须由 ProviderKind::ALL 派生，只放行 Ollama"
    );
    for kind in closed {
        assert_eq!(
            closed_api_provider(kind.as_str()),
            Some(kind),
            "{} 应被识别为闭源 API 供应商",
            kind.as_str()
        );
        let error = OpenWeightModel::new(OPEN_MODEL, "MIT", kind.as_str())
            .expect_err("闭源 API 供应商必须中止预生成");
        let Error::PregenerationClosedProvider { ref provider } = error else {
            panic!("应为 PregenerationClosedProvider，实际：{error}");
        };
        assert_eq!(provider, kind.as_str());
        assert!(
            error.to_string().contains("可下载权重"),
            "中止理由必须点明开放权重要求，实际：{error}"
        );
    }
}

#[test]
fn ollama_is_not_treated_as_a_closed_api_provider() {
    assert_eq!(closed_api_provider(ProviderKind::Ollama.as_str()), None);
    assert!(OPEN_WEIGHT_PROVIDERS.contains(&ProviderKind::Ollama.as_str()));
}

#[test]
fn unknown_runtime_is_rejected_even_when_it_is_not_a_known_closed_provider() {
    let error = OpenWeightModel::new(OPEN_MODEL, "MIT", "some-hosted-thing")
        .expect_err("白名单之外的运行时必须被拒绝");
    assert!(
        matches!(error, Error::PregenerationRejected(ref message) if message.contains("some-hosted-thing")),
        "{error}"
    );
}

#[test]
fn reviewed_true_is_rejected_because_no_review_path_exists() {
    let mut dataset = PregeneratedDataset::new(true);
    let mut row = record("poem-1", "这是一段赏析。");
    row.reviewed = true;
    let error = dataset.push(row).expect_err("reviewed 必须恒为 false");
    assert!(
        matches!(error, Error::PregenerationRejected(ref message) if message.contains("reviewed")),
        "{error}"
    );
}

#[test]
fn missing_provenance_fields_are_rejected_field_by_field() {
    for field in ["stable_id", "template_version", "grounding_digest", "text"] {
        let mut dataset = PregeneratedDataset::new(true);
        let mut row = record("poem-1", "这是一段赏析。");
        match field {
            "stable_id" => row.stable_id = String::new(),
            "template_version" => row.template_version = String::new(),
            "grounding_digest" => row.grounding_digest = String::new(),
            _ => row.text = "   ".to_owned(),
        }
        let error = dataset.push(row).expect_err("缺溯源字段必须被拒绝");
        assert!(
            matches!(error, Error::PregenerationRejected(ref message) if message.contains(field)),
            "{field}: {error}"
        );
    }
}

#[test]
fn duplicate_stable_ids_are_rejected_because_the_shipped_table_would_overwrite() {
    let mut dataset = PregeneratedDataset::new(true);
    dataset
        .push(record("poem-1", "第一段。"))
        .expect("首条应当收录");
    let error = dataset
        .push(record("poem-1", "第二段。"))
        .expect_err("重复的 stable_id 必须被拒绝");
    assert!(matches!(error, Error::PregenerationRejected(_)), "{error}");
    assert_eq!(dataset.records().len(), 1);
}

#[test]
fn a_dataset_declaring_no_inference_must_carry_the_marker_in_every_record() {
    let mut declared_not_executed = PregeneratedDataset::new(false);
    let error = declared_not_executed
        .push(record("poem-1", "看起来像模型输出的一段赏析。"))
        .expect_err("声明未执行推理却带真实正文必须被拒绝");
    assert!(matches!(error, Error::PregenerationRejected(_)), "{error}");
    declared_not_executed
        .push(record("poem-1", NOT_GENERATED_MARKER))
        .expect("带未生成标记的记录应当收录");

    let mut declared_executed = PregeneratedDataset::new(true);
    let error = declared_executed
        .push(record("poem-1", NOT_GENERATED_MARKER))
        .expect_err("声明已执行推理却带未生成标记必须被拒绝");
    assert!(matches!(error, Error::PregenerationRejected(_)), "{error}");
}

#[test]
fn dataset_json_is_a_top_level_array_carrying_every_required_field() {
    let mut dataset = PregeneratedDataset::new(true);
    dataset
        .push(record("poem-1", "这是一段赏析。"))
        .expect("应当收录");
    let json = dataset.to_json().expect("序列化应当成功");
    let parsed: serde_json::Value = serde_json::from_str(&json).expect("产物应当是合法 JSON");
    let array = parsed
        .as_array()
        .expect("顶层必须是数组，jq all(.[]; …) 遍历它");
    assert_eq!(array.len(), 1);
    let entry = &array[0];
    for field in [
        "stable_id",
        "model",
        "model_license",
        "provider",
        "generated_at",
        "template_version",
        "grounding_digest",
        "reviewed",
    ] {
        assert!(entry.get(field).is_some(), "缺字段 {field}");
    }
    assert_eq!(entry["reviewed"], serde_json::Value::Bool(false));
    assert!(
        entry.get("content_hash").is_none(),
        "数据集不得键在 content_hash 上"
    );
}

#[test]
fn records_convert_into_shipped_rows_matching_the_shipped_table_columns() {
    let mut dataset = PregeneratedDataset::new(true);
    dataset.push(record("poem-2", "乙。")).expect("应当收录");
    dataset.push(record("poem-1", "甲。")).expect("应当收录");
    let rows = dataset.to_shipped();
    assert_eq!(
        rows.iter()
            .map(|row| row.stable_id.as_str())
            .collect::<Vec<_>>(),
        ["poem-1", "poem-2"],
        "种子行按 stable_id 有序，便于逐次构建做差分"
    );
    assert_eq!(rows[0].model_license, "MIT");
    assert_eq!(rows[0].template_version, APPRECIATION_TEMPLATE_VERSION);
}

#[test]
fn seed_rows_are_accepted_by_the_shipped_cache_and_then_hit_for_another_provider() {
    let directory = tempdir();
    let cache = AppreciationCache::open(&directory, "test-corpus", 8).expect("打开缓存应当成功");
    let detail = fixture_detail();
    let request = AppreciationRequest::new(detail, "some-users-model");

    let mut dataset = PregeneratedDataset::new(true);
    let mut row = record(&request.poem().poem.stable_id, "这是随包赏析。");
    row.grounding_digest = request.grounding_digest().to_owned();
    dataset.push(row).expect("应当收录");

    for seed in dataset.to_shipped() {
        cache.insert_shipped(&seed).expect("导入种子应当成功");
    }

    let provider = ProviderId::new("deepseek").expect("供应商标识合法");
    let hit = cache
        .lookup(&request, &provider)
        .expect("查找应当成功")
        .expect("用户配的是另一家 provider，随包层仍应命中");
    assert_eq!(hit.appreciation.text, "这是随包赏析。");
}

#[test]
fn pregeneration_refuses_to_read_the_local_cache_table() {
    ensure_readable_table(SHIPPED_TABLE).expect("随包层允许读取");
    let error = ensure_readable_table(LOCAL_CACHE_TABLE).expect_err("用户自费层必须被拒绝");
    assert!(
        matches!(error, Error::PregenerationRejected(ref message)
            if message.contains(LOCAL_CACHE_TABLE) && message.contains("用户")),
        "{error}"
    );
}

#[test]
fn resume_ignores_rows_the_user_paid_for_and_only_skips_shipped_rows() {
    let directory = tempdir();
    let cache = AppreciationCache::open(&directory, "test-corpus", 8).expect("打开缓存应当成功");
    let detail = fixture_detail();
    let request = AppreciationRequest::new(detail, "users-model");
    let user_paid_id = request.poem().poem.stable_id.clone();

    let provider = ProviderId::new("deepseek").expect("供应商标识合法");
    cache
        .store_completed(
            &request,
            &Appreciation {
                text: "用户自费生成的赏析。".to_owned(),
                model: "users-model".to_owned(),
                provider,
                generated_at: 1_770_000_001,
                template_version: APPRECIATION_TEMPLATE_VERSION.to_owned(),
                grounding_digest: request.grounding_digest().to_owned(),
                usage: None,
            },
        )
        .expect("写入用户层应当成功");

    cache
        .insert_shipped(&ShippedAppreciation {
            stable_id: "shipped-only".to_owned(),
            template_version: APPRECIATION_TEMPLATE_VERSION.to_owned(),
            model: OPEN_MODEL.to_owned(),
            model_license: "MIT".to_owned(),
            grounding_digest: "b".repeat(64),
            text: "随包赏析。".to_owned(),
            generated_at: 1_770_000_002,
        })
        .expect("写入随包层应当成功");

    let done = existing_pregenerated_ids(cache.path(), APPRECIATION_TEMPLATE_VERSION)
        .expect("续跑读取应当成功");
    assert!(done.contains("shipped-only"), "随包层已有的作品应当被跳过");
    assert!(
        !done.contains(&user_paid_id),
        "用户自费生成的 `{user_paid_id}` 绝不能被当成已完成——它是用户的 Output，\
         不属于随包数据集"
    );
}

#[test]
fn disclosure_gate_requires_every_marker() {
    let full = "This dataset is AI-generated. 未经领域专家审校，可能编造典故或错置作者，\
                未经独立核实不得依赖。Do not use it to train competing models.";
    ensure_disclosure(full).expect("完整披露应当放行");

    for missing in DISCLOSURE_MARKERS {
        let stripped = full.replace(missing, "");
        let error = ensure_disclosure(&stripped).expect_err(&format!("缺 {missing} 时必须拒绝"));
        assert!(
            matches!(error, Error::PregenerationRejected(ref message) if message.contains(missing)),
            "缺 {missing} 的拒绝理由应点名它，实际：{error}"
        );
    }
}

#[test]
fn coverage_target_is_declared_as_the_four_anthology_tags() {
    assert_eq!(
        ANTHOLOGY_TAGS,
        ["唐诗三百首", "宋词三百首", "千家诗", "古诗文名篇"],
        "覆盖集必须是显式声明的选本集合，不是全语料"
    );
    let vocabulary = yunjian_corpus_anthology_names();
    for tag in ANTHOLOGY_TAGS {
        assert!(
            vocabulary.contains(&tag.to_owned()),
            "`{tag}` 必须是语料策展词表里已声明的选本标签"
        );
    }
}

#[test]
fn sha256_matches_the_reference_digest_of_the_empty_input() {
    assert_eq!(
        sha256_hex(b""),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
}

// ---------------------------------------------------------------- 测试辅助

/// 读取策展词表里 `kind = "anthology"` 的标签名。
///
/// 刻意重新解析 `tags.toml` 而不是依赖 `yunjian-corpus`：`yunjian-ai` 不依赖它，
/// 为一条断言引入一整个 crate 依赖不划算，而这里只需要标签名。
fn yunjian_corpus_anthology_names() -> Vec<String> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../yunjian-corpus/tags.toml");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("读取 {} 失败：{error}", path.display()));
    let value: toml::Value = toml::from_str(&text).expect("词表应当是合法 TOML");
    value["tag"]
        .as_array()
        .expect("`tag` 应当是数组")
        .iter()
        .filter(|entry| entry["kind"].as_str() == Some("anthology"))
        .map(|entry| {
            entry["name"]
                .as_str()
                .expect("标签名应当是字符串")
                .to_owned()
        })
        .collect()
}

fn tempdir() -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!(
        "yunjian-pregen-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path).expect("创建临时目录应当成功");
    path
}

fn fixture_detail() -> yunjian_core::PoemDetail {
    crate::provider::tests::fixture_detail()
}
