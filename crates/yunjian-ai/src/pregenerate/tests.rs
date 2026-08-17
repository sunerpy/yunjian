//! 预生成门禁的用例。
//!
//! 这里验的全是**拒绝**：随包数据集能被分发的理由建立在几条硬约束上，每一条都要有
//! 一个会变红的用例守着，否则约束就只是注释。真实推理不在本文件的验证范围内——
//! 门禁本身与模型是否可达无关，也正因如此它在任何机器上都能验。

use super::{
    ANTHOLOGY_TAGS, DATASET_SCHEMA_VERSION, DISCLOSURE_MARKERS, DatasetManifest, LOCAL_CACHE_TABLE,
    MIN_APPRECIATION_CHARS, NOT_GENERATED_MARKER, OPEN_WEIGHT_LICENSES, OPEN_WEIGHT_PROVIDERS,
    OpenWeightModel, PregeneratedDataset, PregeneratedRecord, ReleaseExpectation, SHIPPED_TABLE,
    closed_api_provider, ensure_disclosure, ensure_readable_table, ensure_releasable,
    existing_pregenerated_ids, sha256_hex,
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

// ------------------------------------------------------ 发布门禁 ensure_releasable
//
// 这一组**只吃合成输入**。发布门禁必须在任何机器上都能被证明会红，不该依赖一次真实推理
// 或一份 631 MB 的语料——那样它的每个分支就只有在发版当天才被走到，而那是最不该出错的时刻。

#[test]
fn the_release_gate_accepts_a_seed_that_really_came_from_an_open_weight_run() {
    let seed = releasable_seed();
    ensure_releasable(&seed.manifest, &seed.records, &seed.expectation())
        .expect("一份真跑出来的种子必须能发布");
}

#[test]
fn the_release_gate_rejects_a_placeholder_seed_naming_the_degradation() {
    let mut seed = releasable_seed();
    seed.manifest.generation_executed = false;
    seed.manifest.model_digest = None;
    seed.manifest.not_executed_reason = Some("未提供 --endpoint".to_owned());
    for record in &mut seed.records {
        record.text = NOT_GENERATED_MARKER.to_owned();
    }

    let error = release_error(&seed);
    assert!(
        error.contains("generation_executed=false") && error.contains("未提供 --endpoint"),
        "占位种子的拒绝理由要点名真因（降级本身），实际：{error}"
    );
}

#[test]
fn a_manifest_claiming_execution_while_carrying_a_degradation_reason_is_rejected() {
    let mut seed = releasable_seed();
    seed.manifest.not_executed_reason = Some("未提供 --endpoint".to_owned());
    let error = release_error(&seed);
    assert!(
        error.contains("两者只能有一个成立"),
        "同时声明已执行与未执行原因的清单是被改过的，应点名这一点：{error}"
    );
}

#[test]
fn a_seed_without_a_weight_digest_cannot_be_released() {
    let mut seed = releasable_seed();
    seed.manifest.model_digest = None;
    let error = release_error(&seed);
    assert!(
        error.contains("model_digest"),
        "缺权重摘要必须点名该字段：{error}"
    );

    for shape in ["sha256:aaaa", "AA", &"z".repeat(64), &"a".repeat(63)] {
        let mut seed = releasable_seed();
        seed.manifest.model_digest = Some(shape.to_owned());
        let error = release_error(&seed);
        assert!(
            error.contains("十六进制"),
            "`{shape}` 不是合法权重摘要，拒绝理由应说明形状要求：{error}"
        );
    }
}

#[test]
fn a_text_hiding_the_marker_inside_a_paragraph_is_rejected_even_though_push_allows_it() {
    let mut seed = releasable_seed();
    let padding = "补".repeat(MIN_APPRECIATION_CHARS);
    seed.records[0].text = format!("这一首的赏析暂缺。{NOT_GENERATED_MARKER}后续补齐。{padding}");

    // 生成期门禁只比逐字相等，所以它放这条过——这正是发布侧要补的那道判据。
    let mut generation = PregeneratedDataset::new(true);
    generation
        .push(seed.records[0].clone())
        .expect("`push` 只拦逐字相等的标记，因此它会放这条过");

    let error = release_error(&seed);
    assert!(
        error.contains("含未生成标记"),
        "夹带标记的正文必须被发布门禁拦下：{error}"
    );
}

#[test]
fn a_stub_shorter_than_the_floor_is_rejected() {
    let mut seed = releasable_seed();
    seed.records[0].text = "略".to_owned();

    let mut generation = PregeneratedDataset::new(true);
    generation
        .push(seed.records[0].clone())
        .expect("`push` 只拦空正文，一个字的存根它放过");

    let error = release_error(&seed);
    assert!(
        error.contains(&MIN_APPRECIATION_CHARS.to_string()),
        "存根的拒绝理由要点名字数下界：{error}"
    );
}

#[test]
fn pasting_one_paragraph_across_every_record_is_rejected() {
    let mut seed = releasable_seed();
    let single = seed.records[0].text.clone();
    for record in &mut seed.records {
        record.text = single.clone();
    }

    let error = release_error(&seed);
    assert!(
        error.contains("逐字相同"),
        "把一段话复制成整份数据集是最省事的伪造形态，必须被点名：{error}"
    );
}

#[test]
fn padding_one_sentence_out_to_the_length_floor_is_rejected() {
    let mut seed = releasable_seed();
    seed.records[0].text = "这首作品意境深远，语言凝练，值得反复品读。".repeat(6);
    let error = release_error(&seed);
    assert!(
        error.contains("重复 6 次"),
        "重复凑长度的模板必须被点名并说出倍数：{error}"
    );
}

/// 真种子必须过得了自我重复这条判据，而模板伪造必须过不了。
///
/// **两侧都要断言。** 只验「伪造被拒」不足以说明这条判据可用——一个把所有输入都判红的检查
/// 也满足那句话，而它会在发版当天红掉真产物。实测数字：真种子 16 条的重复倍数全是 1。
#[test]
fn the_repetition_check_separates_real_output_from_a_padded_template() {
    let real = "上片写关河的开阔，下片折回人事的局促，两种尺度相撞，苍凉便从对比里生出来。\
                词牌本身宜于铺叙，作者却在换头处收住，留出的空白比写满更能承住那份沉重。";
    assert_eq!(
        super::self_repetition(real),
        None,
        "一段真实赏析不该被判成重复拼接"
    );
    assert_eq!(super::self_repetition(&"甲乙丙".repeat(6)), Some(6));
    assert_eq!(super::self_repetition("甲"), None, "单字不构成重复");
    assert_eq!(
        super::self_repetition(""),
        None,
        "空串在别处被拦，这里不误判"
    );
}

#[test]
fn a_grounding_digest_that_does_not_recompute_from_the_released_corpus_is_rejected() {
    let mut seed = releasable_seed();
    seed.records[1].grounding_digest = "b".repeat(64);

    let error = release_error(&seed);
    assert!(
        error.contains("重算") && error.contains(&seed.records[1].stable_id),
        "事实块摘要重算不上必须点名那一条：{error}"
    );
}

#[test]
fn a_record_outside_the_released_coverage_set_is_rejected() {
    let mut seed = releasable_seed();
    seed.records[0].stable_id = "poem-不在覆盖集".to_owned();

    let error = release_error(&seed);
    assert!(
        error.contains("不在本次待发布语料解析出的覆盖集里"),
        "覆盖集之外的记录必须被拒：{error}"
    );
}

#[test]
fn a_seed_covering_only_part_of_the_released_coverage_set_is_rejected() {
    let mut seed = releasable_seed();
    let dropped = seed.records.pop().expect("夹具至少三条").stable_id;
    seed.manifest.record_count = seed.records.len();

    let error = release_error(&seed);
    assert!(
        error.contains(&dropped),
        "缺覆盖的种子要点名缺的是哪一首：{error}"
    );
}

#[test]
fn version_and_digest_mismatches_are_each_rejected_by_name() {
    let mut seed = releasable_seed();
    seed.manifest.corpus_version = "9.9.9".to_owned();
    assert!(
        release_error(&seed).contains("corpus_version"),
        "语料版本对不上必须点名"
    );

    let mut seed = releasable_seed();
    seed.manifest.template_version = "0.0.1".to_owned();
    assert!(
        release_error(&seed).contains("模板"),
        "模板版本对不上必须点名"
    );

    let mut seed = releasable_seed();
    seed.manifest.appreciations_sha256 = "c".repeat(64);
    assert!(
        release_error(&seed).contains("清单描述的不是这个文件"),
        "种子文件摘要对不上必须点名"
    );

    let mut seed = releasable_seed();
    seed.manifest.record_count += 1;
    assert!(
        release_error(&seed).contains("实有"),
        "条数对不上必须点名实际条数"
    );

    let mut seed = releasable_seed();
    seed.manifest.schema_version = DATASET_SCHEMA_VERSION + 1;
    assert!(
        release_error(&seed).contains("schema"),
        "跨 schema 的种子必须被拒"
    );
}

#[test]
fn the_release_gate_still_runs_the_per_record_generation_gates() {
    let mut seed = releasable_seed();
    seed.records[0].reviewed = true;
    assert!(
        release_error(&seed).contains("reviewed"),
        "发布门禁必须继续走生成期那道逐条判据，而不是另抄一份"
    );

    let mut seed = releasable_seed();
    seed.records[0].provider = "anthropic".to_owned();
    seed.records[0].model_license = "MIT".to_owned();
    let error = release_error(&seed);
    assert!(
        error.contains("anthropic"),
        "闭源供应商的记录必须被拒并点名：{error}"
    );
}

#[test]
fn a_record_whose_provenance_disagrees_with_the_manifest_is_rejected() {
    let mut seed = releasable_seed();
    seed.records[0].model = "qwen2.5:7b".to_owned();
    let error = release_error(&seed);
    assert!(
        error.contains("qwen2.5:7b") && error.contains("model"),
        "逐条溯源与清单不一致必须点名：{error}"
    );
}

// ---------------------------------------------------------------- 测试辅助

/// 一份「发布侧应当放行」的合成种子，以及它对应的重算结果。
struct ReleasableSeed {
    manifest: DatasetManifest,
    records: Vec<PregeneratedRecord>,
    grounding: std::collections::BTreeMap<String, String>,
    /// 种子字节的实测摘要。**独立于清单自述的那一行**，否则改清单永远比得上自己。
    seed_sha256: String,
}

impl ReleasableSeed {
    fn expectation(&self) -> ReleaseExpectation<'_> {
        ReleaseExpectation {
            corpus_version: "0.1.0",
            template_version: APPRECIATION_TEMPLATE_VERSION,
            grounding: &self.grounding,
            seed_sha256: &self.seed_sha256,
        }
    }

    /// 按当前记录填出「本该由待发布语料算出」的那一组值。**只在构造夹具时调用。**
    ///
    /// 用例里不得再调：改完 `stable_id` 或 `grounding_digest` 再重算，等于把用例想验的那处
    /// 偏差抹掉，于是三条断言会在门禁完全正常的情况下变绿——我写这份夹具时正是先掉进了这里。
    fn reseal(&mut self) {
        self.grounding = self
            .records
            .iter()
            .map(|record| (record.stable_id.clone(), record.grounding_digest.clone()))
            .collect();
    }
}

fn releasable_seed() -> ReleasableSeed {
    let texts = [
        "这一首以明月起兴，由光影落到乡思，二十字里只用寻常景物便把羁旅之感托住。\
         起句写所见，承句写所疑，转合两句才把身在异乡这件事说出来，次序本身就是情绪。\
         语言浅到近乎口语，而正是这份浅让它在千余年里一直被记住。",
        "上片写关河的开阔，下片折回人事的局促，两种尺度相撞，苍凉便从对比里生出来。\
         词牌本身宜于铺叙，作者却在换头处收住，留出的空白比写满更能承住那份沉重。\
         结句不着一字议论，情绪却已经压到最低处，这是词体特有的收法。",
        "全篇以问句收束，答案留在字外；不作断语正是它比同题诸作耐读的地方。\
         前六句层层设景，末二句忽然抽身发问，读者被推到与作者同一个位置上去。\
         设景与发问之间没有过渡，那道断裂本身就是这首诗的结构。",
    ];
    let mut records = Vec::new();
    for (index, text) in texts.iter().enumerate() {
        let mut row = record(&format!("poem-{index}"), text);
        row.grounding_digest = format!("{index:064}").replace('0', "a");
        records.push(row);
    }
    let manifest = DatasetManifest {
        schema_version: DATASET_SCHEMA_VERSION,
        template_version: APPRECIATION_TEMPLATE_VERSION.to_owned(),
        coverage_tags: ANTHOLOGY_TAGS.iter().map(|tag| (*tag).to_owned()).collect(),
        coverage_selector: "reviewed_roster".to_owned(),
        record_count: records.len(),
        model: OPEN_MODEL.to_owned(),
        model_license: "MIT".to_owned(),
        provider: "ollama".to_owned(),
        model_digest: Some("d".repeat(64)),
        generation_executed: true,
        not_executed_reason: None,
        appreciations_sha256: "e".repeat(64),
        corpus_version: "0.1.0".to_owned(),
        built_at: 1_770_000_000,
    };
    let seed_sha256 = manifest.appreciations_sha256.clone();
    let mut seed = ReleasableSeed {
        manifest,
        records,
        grounding: std::collections::BTreeMap::new(),
        seed_sha256,
    };
    seed.reseal();
    seed
}

fn release_error(seed: &ReleasableSeed) -> String {
    ensure_releasable(&seed.manifest, &seed.records, &seed.expectation())
        .expect_err("这份种子应当被发布门禁拒绝")
        .to_string()
}

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
