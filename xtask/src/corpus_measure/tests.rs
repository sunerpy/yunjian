use super::*;

/// 一份最小的合法报告。各测试从它出发只改一处，因此每个断言都能指认「正是这一处
/// 让校验失败」，而不是让一份处处可疑的报告碰巧被拒。
fn valid_report() -> MeasuredReport {
    MeasuredReport {
        schema_version: 1,
        budget: Budget {
            artifact_gzip_bytes: DEFAULT_ARTIFACT_BUDGET_MIB * 1024 * 1024,
            p95_ms: DEFAULT_P95_BUDGET_MS,
            declared_by: "方案 todo 20".to_owned(),
        },
        environment: Environment {
            reference_machine: "linux/x86_64，32 逻辑核".to_owned(),
            cpu_model: "Intel(R) Xeon(R) 6975P-C".to_owned(),
            memory_total_kib: 64 * 1024 * 1024,
            disk_kind: "nvme0n1=SSD/NVMe".to_owned(),
            sqlite_version: "3.53.2".to_owned(),
            page_size: 4096,
            index_detail_mode: "full".to_owned(),
            ngram_aux_enabled: true,
            repeats_per_query: 25,
            warmup_per_query: WARMUP,
        },
        scales: vec![ScaleRow {
            scale: "10k".to_owned(),
            scope: "唐宋前 1 万首".to_owned(),
            state: MeasurementState::Measured,
            blocked_reason: None,
            measurement: Some(valid_measurement()),
        }],
        verdict: Verdict {
            within_budget: true,
            full_scale_measured: false,
            largest_measured_scale: "10k".to_owned(),
            summary: "已实测 1 个规模，预算内。".to_owned(),
            mitigation: None,
            dominant_table: None,
        },
    }
}

fn valid_measurement() -> Measurement {
    Measurement {
        poem_count: 10_000,
        input_rows: 12_000,
        raw_text_bytes: 3_000_000,
        poem_table_bytes: 4_096_000,
        poem_fts_bytes: 8_192_000,
        ngram_table_bytes: 20_480_000,
        ngram_rows: 900_000,
        table_bytes: vec![
            TableBytes {
                name: "ngram".to_owned(),
                bytes: 20_480_000,
                share_of_file: 0.5389,
            },
            TableBytes {
                name: "poem_fts_data".to_owned(),
                bytes: 8_192_000,
                share_of_file: 0.2156,
            },
        ],
        fts_to_poem_ratio: 2.0,
        index_to_raw_ratio: 9.56,
        bytes_before_vacuum: 40_000_000,
        bytes_after_vacuum: 38_000_000,
        gzip_bytes: 12_000_000,
        gzip_ratio: 0.3158,
        build_seconds: 42.5,
        queries: (0..REPRESENTATIVE_QUERY_COUNT)
            .map(|index| QueryMeasurement {
                id: format!("probe_{index}"),
                kind: "等值".to_owned(),
                sql_shape: "SELECT stable_id FROM poem WHERE author = ?1".to_owned(),
                hits: 12,
                p50_ms: 0.05,
                p95_ms: 0.09,
                explain_query_plan: vec!["SEARCH poem USING INDEX poem_author_idx".to_owned()],
            })
            .collect(),
        worst_p95_ms: 0.09,
        within_p95_budget: true,
        within_artifact_budget: true,
    }
}

fn error_of(report: &MeasuredReport) -> String {
    format!("{:#}", report.validate().expect_err("这份报告应当被拒绝"))
}

#[test]
fn a_fully_measured_report_passes_validation() {
    valid_report()
        .validate()
        .expect("一份每个字段都是实测值的报告必须通过校验");
}

#[test]
fn the_parser_rejects_a_placeholder_anywhere_in_the_report() {
    // 逐个占位符都要被拒，而不只是其中一个：写报告的人可能用任意一种写法糊过去。
    for placeholder in PLACEHOLDERS {
        let mut report = valid_report();
        report.verdict.summary = format!("体积{placeholder}，等实测");
        let message = error_of(&report);
        assert!(
            message.contains(placeholder) && message.contains("只接受实测值"),
            "占位符 `{placeholder}` 必须被拒绝，实际错误：{message}"
        );
    }
}

#[test]
fn the_parser_rejects_a_measured_row_missing_its_measurement() {
    let mut report = valid_report();
    report.scales[0].measurement = None;
    let message = error_of(&report);
    assert!(
        message.contains("标为 Measured 但没有测量值"),
        "缺测量值必须被拒绝，实际错误：{message}"
    );
}

#[test]
fn the_parser_rejects_a_zero_valued_measurement_field() {
    // 零是「忘了填」最常见的形态：字段在，数字是默认值。它必须与真实测量区分开。
    let mut report = valid_report();
    report.scales[0]
        .measurement
        .as_mut()
        .unwrap()
        .poem_fts_bytes = 0;
    let message = error_of(&report);
    assert!(
        message.contains("poem_fts_bytes") && message.contains("不是实测出来的"),
        "零值测量必须被拒绝，实际错误：{message}"
    );
}

#[test]
fn the_parser_rejects_a_not_measured_row_without_a_blocking_reason() {
    let mut report = valid_report();
    report.scales.push(ScaleRow {
        scale: "full".to_owned(),
        scope: "全量".to_owned(),
        state: MeasurementState::NotMeasured,
        blocked_reason: None,
        measurement: None,
    });
    let message = error_of(&report);
    assert!(
        message.contains("没写阻塞原因"),
        "未测但无原因必须被拒绝，实际错误：{message}"
    );
}

#[test]
fn a_not_measured_row_with_a_reason_is_a_successful_report() {
    // 「10k 实测 + full 诚实标未测」是本 todo 认可的成功产出，不能被校验器当成缺陷。
    let mut report = valid_report();
    report.scales.push(ScaleRow {
        scale: "full".to_owned(),
        scope: "全量约 85 万首".to_owned(),
        state: MeasurementState::NotMeasured,
        blocked_reason: Some("磁盘剩余空间不足，需要约 40 GiB 可用空间".to_owned()),
        measurement: None,
    });
    report.validate().expect("带阻塞原因的未测规模是合法产出");
}

#[test]
fn the_parser_rejects_a_report_with_zero_measured_scales() {
    let mut report = valid_report();
    report.scales = vec![ScaleRow {
        scale: "full".to_owned(),
        scope: "全量".to_owned(),
        state: MeasurementState::NotMeasured,
        blocked_reason: Some("网络不可用".to_owned()),
        measurement: None,
    }];
    let message = error_of(&report);
    assert!(
        message.contains("零实测"),
        "全部未测的报告不构成产出，实际错误：{message}"
    );
}

#[test]
fn the_parser_rejects_a_missing_reference_machine() {
    let mut report = valid_report();
    report.environment.cpu_model = "   ".to_owned();
    let message = error_of(&report);
    assert!(
        message.contains("参考机配置无法解读"),
        "缺参考机配置必须被拒绝，实际错误：{message}"
    );
}

#[test]
fn the_parser_rejects_fewer_than_eight_representative_queries() {
    let mut report = valid_report();
    report.scales[0].measurement.as_mut().unwrap().queries.pop();
    let message = error_of(&report);
    assert!(
        message.contains("方案要求 8 条代表性查询"),
        "少于八条查询必须被拒绝，实际错误：{message}"
    );
}

#[test]
fn the_parser_rejects_a_content_probe_that_hit_nothing() {
    // 这条钉住 10k 首轮实测暴露的真实缺陷：写死的整句在繁体底本上一条不中，
    // p95 记成 0.011 ms 并被当成「很快」。零命中的正文探针必须让报告失败。
    let mut report = valid_report();
    let queries = &mut report.scales[0].measurement.as_mut().unwrap().queries;
    queries[0].id = "full_line_like".to_owned();
    queries[0].hits = 0;
    let message = error_of(&report);
    assert!(
        message.contains("零命中") && message.contains("空结果集的速度"),
        "零命中的正文探针必须被拒绝，实际错误：{message}"
    );
}

#[test]
fn a_metadata_probe_may_hit_nothing_only_when_it_declares_the_table_is_empty() {
    // 标签表在当前构建阶段确实没有数据来源，所以零命中是合法的——但必须自己声明，
    // 否则「表是空的」与「查询写错了」在报告里长得一模一样。
    let mut report = valid_report();
    let queries = &mut report.scales[0].measurement.as_mut().unwrap().queries;
    queries[0].id = "tag_filter".to_owned();
    queries[0].hits = 0;
    queries[0].kind = "标签过滤".to_owned();
    let message = error_of(&report);
    assert!(
        message.contains("没有声明表为空"),
        "未声明的零命中必须被拒绝，实际错误：{message}"
    );

    report.scales[0].measurement.as_mut().unwrap().queries[0].kind =
        "标签过滤（规范化多对多表），poem_tag 表为空，本条为零命中基线".to_owned();
    report
        .validate()
        .expect("声明了表为空的元数据探针零命中是合法的");
}

#[test]
fn the_parser_rejects_a_query_without_an_explain_query_plan() {
    let mut report = valid_report();
    report.scales[0].measurement.as_mut().unwrap().queries[0]
        .explain_query_plan
        .clear();
    let message = error_of(&report);
    assert!(
        message.contains("缺 EXPLAIN QUERY PLAN"),
        "缺 EQP 必须被拒绝，实际错误：{message}"
    );
}

#[test]
fn a_busted_budget_without_a_mitigation_is_rejected() {
    // 这条是「门禁是真的而不是装饰」的机制保证：超预算且不给出路的报告写不出去。
    let mut report = valid_report();
    report.verdict.within_budget = false;
    report.verdict.mitigation = None;
    let message = error_of(&report);
    assert!(
        message.contains("没有指名缓解措施"),
        "超预算无措施必须被拒绝，实际错误：{message}"
    );
}

#[test]
fn a_busted_artifact_budget_selects_the_tang_song_default_mitigation() {
    // 唐宋在预算内、全量超预算 —— 这正是「限制默认集」能解决的情形。
    let rows = vec![
        scale_row("tang-song", 300_000, 200 * 1024 * 1024, true, true),
        scale_row("full", 850_000, 400 * 1024 * 1024, false, true),
    ];
    let verdict = decide(&rows, DEFAULT_ARTIFACT_BUDGET_MIB * 1024 * 1024);
    assert!(!verdict.within_budget, "全量超预算时结论必须为假");
    let mitigation = verdict.mitigation.expect("超预算必须指名缓解措施");
    assert_eq!(mitigation.id, MITIGATION_TANG_SONG_DEFAULT);
    assert!(
        mitigation.statement.contains("不丢任何一首诗"),
        "该措施的关键性质是不删记录，必须写在措施里：{}",
        mitigation.statement
    );
}

#[test]
fn an_absurd_budget_flips_the_verdict_and_still_names_a_mitigation() {
    // 失败场景：预算设成荒谬的 1 MB。连唐宋也装不下，于是「限制默认集」解决不了问题，
    // 必须如实说明并把决定交回给人 —— 而不是悄悄提高预算。
    let rows = vec![
        scale_row("tang-song", 300_000, 200 * 1024 * 1024, false, true),
        scale_row("full", 850_000, 400 * 1024 * 1024, false, true),
    ];
    let verdict = decide(&rows, 1024 * 1024);
    assert!(!verdict.within_budget);
    let mitigation = verdict.mitigation.expect("必须指名缓解措施");
    assert_eq!(mitigation.id, MITIGATION_NO_SUBSET_FITS);
    assert!(
        mitigation.statement.contains("不擅自提高预算"),
        "子命令不得自行放宽预算，这一点必须写明：{}",
        mitigation.statement
    );
}

#[test]
fn a_p95_bust_alone_also_flips_the_verdict() {
    // 体积达标但延迟超标同样要翻假：两条预算是并列的硬门槛，不是二选一。
    let rows = vec![scale_row("full", 850_000, 100 * 1024 * 1024, true, false)];
    let verdict = decide(&rows, DEFAULT_ARTIFACT_BUDGET_MIB * 1024 * 1024);
    assert!(!verdict.within_budget, "延迟超标必须让结论为假");
    assert!(
        verdict.summary.contains("超延迟预算"),
        "结论必须点明是延迟超标：{}",
        verdict.summary
    );
}

#[test]
fn a_verdict_within_budget_records_whether_the_shipping_scale_was_measured() {
    // 只测了 10k 就宣布「预算内」是危险的：结论必须自带覆盖范围。
    let rows = vec![scale_row("10k", 10_000, 12 * 1024 * 1024, true, true)];
    let verdict = decide(&rows, DEFAULT_ARTIFACT_BUDGET_MIB * 1024 * 1024);
    assert!(verdict.within_budget);
    assert!(!verdict.full_scale_measured);
    assert!(
        verdict.summary.contains("发布规模尚未实测"),
        "未测发布规模时结论必须自我限定范围：{}",
        verdict.summary
    );
}

#[test]
fn a_busted_verdict_names_the_table_that_dominates_the_file() {
    // 实测显示占大头的是 ngram（约 76%）而不是正文（9.5%）。结论必须点名这一项，
    // 否则 todo 21 会去优化那个只占 9.5% 的项，而预算根本不是它撑爆的。
    let rows = vec![scale_row("full", 896_127, 3351 * 1024 * 1024, false, true)];
    let verdict = decide(&rows, DEFAULT_ARTIFACT_BUDGET_MIB * 1024 * 1024);
    let dominant = verdict
        .dominant_table
        .as_ref()
        .expect("超预算的结论必须带上占字节最多的表");
    assert_eq!(dominant.name, "ngram");
    assert!(
        verdict.summary.contains("ngram") && verdict.summary.contains("不是正文"),
        "结论必须点名主项并说明缩小集合削不掉它：{}",
        verdict.summary
    );
}

fn scale_row(
    scale: &str,
    poem_count: usize,
    gzip_bytes: u64,
    within_artifact: bool,
    within_p95: bool,
) -> ScaleRow {
    let mut measurement = valid_measurement();
    measurement.poem_count = poem_count;
    measurement.gzip_bytes = gzip_bytes;
    measurement.within_artifact_budget = within_artifact;
    measurement.within_p95_budget = within_p95;
    measurement.worst_p95_ms = if within_p95 { 0.09 } else { 420.0 };
    ScaleRow {
        scale: scale.to_owned(),
        scope: format!("{scale} 规模"),
        state: MeasurementState::Measured,
        blocked_reason: None,
        measurement: Some(measurement),
    }
}

#[test]
fn the_markdown_marks_not_measured_scales_verbatim() {
    // 报告的可信度全靠这一点：未测的规模在人读的表里必须一眼可辨，不能是空白单元格。
    let mut report = valid_report();
    report.scales.push(ScaleRow {
        scale: "full".to_owned(),
        scope: "全量约 85 万首".to_owned(),
        state: MeasurementState::NotMeasured,
        blocked_reason: Some("磁盘空间不足".to_owned()),
        measurement: None,
    });
    let markdown = render_markdown(&report);
    assert!(markdown.contains("NOT MEASURED"));
    assert!(markdown.contains("磁盘空间不足"));
    assert!(markdown.contains("### 未实测的规模与阻塞原因"));
}

#[test]
fn load_rejects_a_placeholder_report_read_from_disk() {
    // 校验必须在**文件读取路径**上生效，而不只在内存里的结构上：todo 21 拿到的是
    // 一个磁盘上的 JSON，如果只有 `validate()` 有门禁而 `load()` 没有，那道门就绕开了。
    let dir = std::env::temp_dir().join(format!("yunjian-measure-load-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("measurements.json");

    let good = valid_report();
    std::fs::write(&path, serde_json::to_string_pretty(&good).unwrap()).unwrap();
    let loaded = MeasuredReport::load(&path).expect("合法报告必须能读回");
    assert_eq!(loaded.scales.len(), good.scales.len());
    assert!(loaded.verdict.within_budget);

    let mut bad = valid_report();
    bad.scales[0].scope = "全量语料（估算）".to_owned();
    std::fs::write(&path, serde_json::to_string_pretty(&bad).unwrap()).unwrap();
    let message = format!(
        "{:#}",
        MeasuredReport::load(&path).expect_err("含占位符的报告必须在读取时就被拒")
    );
    assert!(
        message.contains("估算"),
        "读取路径必须拒绝占位符，实际错误：{message}"
    );

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn the_declared_budget_is_two_hundred_fifty_mib() {
    // 预算是方案事先声明的数字，不许在实现里悄悄改动。
    assert_eq!(DEFAULT_ARTIFACT_BUDGET_MIB, 250);
    assert_eq!(DEFAULT_ARTIFACT_BUDGET_MIB * 1024 * 1024, 250 * 1024 * 1024);
    assert_eq!(DEFAULT_P95_BUDGET_MS, 150.0);
}

#[test]
fn the_scale_keys_parse_and_round_trip() {
    for scale in ALL_SCALES {
        assert_eq!(Scale::parse(scale.key()).unwrap(), scale);
    }
    assert!(Scale::parse("300k").is_err());
}

#[test]
fn only_the_tang_song_scales_restrict_dynasties_and_truncate() {
    assert!(Scale::Sample10k.tang_song_only());
    assert!(Scale::TangSong.tang_song_only());
    assert!(!Scale::Full.tang_song_only());
    assert_eq!(Scale::Sample10k.truncate_to(), Some(10_000));
    assert_eq!(Scale::TangSong.truncate_to(), None);
    assert_eq!(Scale::Full.truncate_to(), None);
    // 全量必须读全部古典白名单分桶，用空列表表达「不缩小范围」。
    assert!(Scale::Full.werneror_buckets().is_empty());
    assert!(!Scale::TangSong.werneror_buckets().is_empty());
}

#[test]
fn gzip_size_counts_bytes_without_writing_a_file() {
    let path =
        std::env::temp_dir().join(format!("yunjian-measure-gzip-{}.bin", std::process::id()));
    // 高度可压缩的输入：断言压缩后显著小于原文，证明确实走了压缩而不是原样计数。
    std::fs::write(&path, "明月".repeat(50_000)).unwrap();
    let raw = std::fs::metadata(&path).unwrap().len();
    let compressed = gzip_size(&path).unwrap();
    std::fs::remove_file(&path).unwrap();
    assert!(compressed > 0);
    assert!(
        compressed < raw / 10,
        "重复正文应当被显著压缩：raw={raw} gzip={compressed}"
    );
    assert!(
        !std::env::temp_dir()
            .join(format!(
                "yunjian-measure-gzip-{}.bin.gz",
                std::process::id()
            ))
            .exists(),
        "量体积不应落下 .gz 文件"
    );
}
