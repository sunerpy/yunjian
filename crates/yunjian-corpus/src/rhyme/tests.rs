//! 韵书导入的验收测试。
//!
//! 夹具是从锁定 revision 的真实资产裁剪出来的：**保留完整的声部/韵部骨架**（所以
//! 「一百零五个韵部」「去声部三十个」在夹具上就能断言），每个韵部只留少量字。
//! 夹具里没有任何被扣留资产的数据。

use super::*;
use std::path::PathBuf;

fn fixture_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/rhyme_book")
}

fn import_fixture() -> RhymeImport {
    import(fixture_root()).expect("夹具应可导入")
}

/// 上游 `Pingshui_Rhyme.json` 的五个声部各自的韵部数（实测于锁定 revision）。
const PINGSHUI_GROUPS_BY_TONE: [(&str, usize); 5] = [
    ("上平声部", 15),
    ("下平声部", 15),
    ("上声部", 28),
    ("去声部", 30),
    ("入声部", 17),
];

/// 平水韵韵部总数（实测）。
///
/// 通行的平水韵是一百零六韵；上游这份**缺上声「三讲」**（键从 `二肿` 直接跳到 `四纸`），
/// 所以实测是一百零五。这个数字写死在断言里是刻意的：上游哪天补上三讲，测试会失败，
/// 于是那是一次被看见的数据变更，而不是悄悄改变了我们对韵部的计数。
const PINGSHUI_GROUP_TOTAL: usize = 105;

/// 词林正韵的部数（实测）。
const CILIN_GROUP_TOTAL: usize = 19;

#[test]
fn pingshui_resolves_the_measured_group_counts() {
    let import = import_fixture();
    let table = import.table(RhymeBook::Pingshui).expect("平水韵随包");

    assert_eq!(
        table.group_count(),
        PINGSHUI_GROUP_TOTAL,
        "平水韵韵部总数与实测不符"
    );

    let by_tone = table.group_count_by_tone_raw();
    for (tone_raw, expected) in PINGSHUI_GROUPS_BY_TONE {
        assert_eq!(
            by_tone.get(tone_raw),
            Some(&expected),
            "{tone_raw} 的韵部数与实测不符"
        );
    }

    // 「平水韵三十韵部」唯一成立的读法：去声部恰好三十个。全书是一百零五个。
    assert_eq!(by_tone.get("去声部"), Some(&30));
    assert_eq!(
        by_tone.values().sum::<usize>(),
        PINGSHUI_GROUP_TOTAL,
        "逐声部之和应等于总数"
    );

    // 缺失的上声三讲：记录为已知的上游缺口，而不是假装它在。
    assert!(!table.groups().contains("三讲"), "上游这份缺上声三讲");
}

#[test]
fn cilin_resolves_nineteen_parts() {
    let import = import_fixture();
    let table = import.table(RhymeBook::Cilin).expect("词林正韵随包");
    assert_eq!(table.group_count(), CILIN_GROUP_TOTAL);

    // 十四部分平仄，五部只有入声（入声独立成部，故无平仄之分）。
    let mut tone_sets: BTreeMap<&str, BTreeSet<RhymeTone>> = BTreeMap::new();
    for entry in table.entries() {
        tone_sets
            .entry(entry.rhyme_group.as_str())
            .or_default()
            .insert(entry.tone);
    }
    let entering_only = tone_sets
        .values()
        .filter(|tones| **tones == BTreeSet::from([RhymeTone::Entering]))
        .count();
    assert_eq!(entering_only, 5, "应有五部只含入声");
    assert_eq!(tone_sets.len() - entering_only, 14, "应有十四部分平仄");

    let tones: BTreeSet<RhymeTone> = table.entries().iter().map(|entry| entry.tone).collect();
    assert!(tones.contains(&RhymeTone::Level));
    assert!(tones.contains(&RhymeTone::Oblique));
    assert!(tones.contains(&RhymeTone::Entering));
    // 词林正韵不区分上去，所以这两个声调不该出现——出现说明有人替上游拆了它没有的信息。
    assert!(!tones.contains(&RhymeTone::Rising));
    assert!(!tones.contains(&RhymeTone::Departing));
}

/// 一个字属于两个韵部时必须产出两行。计划点名的用例：临 在十二侵与二十七沁。
#[test]
fn a_character_in_two_rhyme_groups_yields_two_rows() {
    let import = import_fixture();
    let table = import.table(RhymeBook::Pingshui).expect("平水韵随包");

    let rows: Vec<&RhymeEntry> = table
        .entries()
        .iter()
        .filter(|entry| entry.character == "临")
        .collect();
    assert_eq!(rows.len(), 2, "临 应产出两行，实得 {rows:?}");

    let located: BTreeSet<(RhymeTone, &str)> = rows
        .iter()
        .map(|entry| (entry.tone, entry.rhyme_group.as_str()))
        .collect();
    assert!(located.contains(&(RhymeTone::Level, "十二侵")));
    assert!(located.contains(&(RhymeTone::Departing, "二十七沁")));
}

/// 相反嵌套的两个解析器必须产出同一种行，且不变量一致。
#[test]
fn reversed_nesting_produces_equivalent_logical_output() {
    let import = import_fixture();
    let pingshui = import.table(RhymeBook::Pingshui).expect("平水韵随包");
    let cilin = import.table(RhymeBook::Cilin).expect("词林正韵随包");

    for table in [pingshui, cilin] {
        assert!(!table.entries().is_empty());
        for entry in table.entries() {
            assert_eq!(entry.book, table.book, "行上的书别与表不一致");
            assert!(!entry.rhyme_group.is_empty(), "韵部名不得为空");
            assert!(!entry.character.is_empty(), "字不得为空");
            assert!(!entry.tone_raw.is_empty(), "上游原始声部键必须保留");
        }
    }

    // 同一个字在两本书里各自定位到本书的韵部：平水韵取自「声部 -> 韵部」的内层，
    // 词林正韵取自「部 -> 声」的外层。方向相反，产出的行形状相同。
    let dong_pingshui: BTreeSet<(RhymeTone, &str)> = pingshui
        .entries()
        .iter()
        .filter(|entry| entry.character == "东")
        .map(|entry| (entry.tone, entry.rhyme_group.as_str()))
        .collect();
    let dong_cilin: BTreeSet<(RhymeTone, &str)> = cilin
        .entries()
        .iter()
        .filter(|entry| entry.character == "东")
        .map(|entry| (entry.tone, entry.rhyme_group.as_str()))
        .collect();

    assert_eq!(dong_pingshui, BTreeSet::from([(RhymeTone::Level, "一东")]));
    assert_eq!(dong_cilin, BTreeSet::from([(RhymeTone::Level, "第一部")]));

    // 两本书的韵部命名空间不重叠，所以「不指定韵书」的查询不可能被当成有意义的问题。
    let pingshui_groups = pingshui.groups();
    let cilin_groups = cilin.groups();
    let shared: Vec<&&str> = pingshui_groups.intersection(&cilin_groups).collect();
    assert!(shared.is_empty(), "两本书的韵部名不应重叠：{shared:?}");
}

/// 逐字反向索引是从平水韵反转推导出来的，不是引第三方仓库来的。
#[test]
fn character_reverse_index_is_derived_by_inverting_the_rhyme_table() {
    let import = import_fixture();
    let table = import.table(RhymeBook::Pingshui).expect("平水韵随包");
    let index = import.index(RhymeBook::Pingshui).expect("平水韵随包");

    assert_eq!(index.book(), Some(RhymeBook::Pingshui));
    assert_eq!(
        index.len(),
        table.distinct_characters(),
        "索引的字数应等于韵书里的不同字数"
    );

    // 被拒绝的 jkak/pingShuiYun 的记录形状是
    // `"临": [["去","二十七沁",""], ["平","十二侵",""]]`；推导结果必须等价。
    let lin = index.lookup("临");
    assert_eq!(
        lin,
        &[
            (RhymeTone::Level, "十二侵".to_owned()),
            (RhymeTone::Departing, "二十七沁".to_owned()),
        ],
        "临 的推导结果应等价于被拒仓库的记录形状"
    );
    assert!(index.is_tone_ambiguous("临"), "临 平去两读");

    // 反转必须完备：韵书里的每一行都能在索引里找回来。
    for entry in table.entries() {
        let found = index.lookup(&entry.character);
        assert!(
            found.contains(&(entry.tone, entry.rhyme_group.clone())),
            "索引丢了 {} -> ({}, {})",
            entry.character,
            entry.tone.display_name(),
            entry.rhyme_group
        );
    }

    // 不在韵书里的字返回空切片，这是有效的否定答案，与「韵书缺失」不同。
    assert!(index.lookup("〇").is_empty());
}

/// 上游同一韵部内重复列出同一个字时，索引不得出现重复的（声调, 韵部）。
#[test]
fn within_group_duplicates_are_counted_and_deduplicated() {
    let import = import_fixture();
    let table = import.table(RhymeBook::Pingshui).expect("平水韵随包");
    let index = import.index(RhymeBook::Pingshui).expect("平水韵随包");

    assert!(
        table.duplicate_entries() > 0,
        "夹具刻意保留了同韵部内的重复条目"
    );

    let dong = index.lookup("东");
    let distinct: BTreeSet<&(RhymeTone, String)> = dong.iter().collect();
    assert_eq!(
        dong.len(),
        distinct.len(),
        "索引里出现了重复的（声调, 韵部）"
    );
    assert_eq!(dong, &[(RhymeTone::Level, "一东".to_owned())]);
}

/// 声调维度以反向索引为准，`Word_Tune.json` 只作交叉核对；分歧只记录不改写。
#[test]
fn tone_table_is_cross_checked_not_trusted() {
    let import = import_fixture();
    let check = &import.tone_cross_check;

    assert!(check.declared_rows > 0);
    assert!(
        check.declared_only.is_empty(),
        "平仄表出现了韵书里没有的字：{:?}",
        check.declared_only
    );

    // 「空」在上平一东（平）与上声一董、去声一送（仄）都出现，确实平仄两读，
    // 而上游平仄表标成「仄」。采信上游会把「空山不见人」判为出律，所以以索引为准。
    let kong = import
        .lookup(RhymeBook::Pingshui, "空")
        .expect("平水韵随包");
    assert!(
        kong.iter().any(|(tone, _)| tone.is_level()),
        "空 应有平声归属"
    );
    assert!(
        kong.iter().any(|(tone, _)| !tone.is_level()),
        "空 应有仄声归属"
    );
    assert!(
        import
            .index(RhymeBook::Pingshui)
            .expect("平水韵随包")
            .is_tone_ambiguous("空")
    );

    let divergence = check
        .divergences
        .iter()
        .find(|row| row.character == "空")
        .expect("空 应被记为一处分歧");
    assert_eq!(divergence.declared, DeclaredTune::Oblique);
    assert!(divergence.derived.len() > 1);
}

/// **缺书必须是类型化错误，绝不能是空结果集。**
///
/// 这是本 todo 最关键的一条：静默把缺数据报成否定答案，就是对格律的虚假陈述。
#[test]
fn querying_xinyun_returns_a_typed_error_and_never_an_empty_result() {
    let import = import_fixture();

    for attempt in [
        import.table(RhymeBook::Xinyun).err(),
        import.index(RhymeBook::Xinyun).err(),
        import.lookup(RhymeBook::Xinyun, "东").err(),
    ] {
        let err = attempt.expect("对中华新韵的查询必须报错，而不是返回空结果");
        assert!(
            matches!(
                err,
                Error::RhymeBookUnavailable {
                    book: RhymeBook::Xinyun,
                    ..
                }
            ),
            "错误类型不对，调用方无法区分「缺书」与「不押韵」：{err:?}"
        );
    }

    // 对照：随包的两本书同样的调用返回数据。若上面那组返回了空切片而不是错误，
    // 调用方看到的就是与「这个字不在韵书里」完全一样的形状。
    for book in RhymeBook::SHIPPED {
        let rows = import.lookup(book, "东").expect("随包韵书应返回数据");
        assert!(!rows.is_empty(), "{} 里应查到 东", book.display_name());
    }
}

/// 被扣留的资产在本模块里没有读取路径。
#[test]
fn withheld_assets_are_absent_from_the_shippable_set() {
    for withheld in WITHHELD_ASSETS {
        assert!(
            !SHIPPED_ASSETS.contains(&withheld),
            "{withheld} 被扣留，不得进入可分发白名单"
        );
    }
    // 计划点名的两个资产必须在扣留表里。
    assert!(WITHHELD_ASSETS.contains(&"data/Ci_Tunes.json"));
    assert!(WITHHELD_ASSETS.contains(&"data/Xinyun_Rhyme.json"));

    // 即便有人把扣留资产的路径直接传进来，也拿不到数据。
    let err =
        read_asset(&fixture_root(), "data/Ci_Tunes.json").expect_err("扣留资产不得有读取路径");
    assert!(format!("{err}").contains("拒绝读取"), "{err}");
}

/// `sources.toml` 必须把两个被点名的资产记为 `license_class = "unverified"` 且不可分发。
///
/// 直接读清单而不是相信记忆：许可判定的唯一真相在那个文件里，代码里的常量表只是它的镜像。
#[test]
fn sources_manifest_marks_both_named_assets_unverified_and_unshippable() {
    #[derive(Deserialize)]
    struct Manifest {
        #[serde(rename = "source")]
        sources: Vec<Source>,
    }
    #[derive(Deserialize)]
    struct Source {
        name: String,
        git_rev: String,
        assets: Vec<Asset>,
    }
    #[derive(Deserialize)]
    struct Asset {
        path: String,
        license_class: String,
        shippable: bool,
    }

    let manifest_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus/sources.toml");
    let text = std::fs::read_to_string(&manifest_path).expect("读取 corpus/sources.toml");
    let manifest: Manifest = toml::from_str(&text).expect("解析 corpus/sources.toml");

    let source = manifest
        .sources
        .iter()
        .find(|source| source.name == SOURCE_NAME)
        .expect("清单里应有韵书来源");
    assert_eq!(
        source.git_rev, SOURCE_REV,
        "导入代码与清单锁定的 revision 必须一致"
    );

    for named in ["data/Ci_Tunes.json", "data/Xinyun_Rhyme.json"] {
        let asset = source
            .assets
            .iter()
            .find(|asset| asset.path == named)
            .unwrap_or_else(|| panic!("清单里应有 {named}"));
        assert_eq!(
            asset.license_class, "unverified",
            "{named} 的授权链未核实，必须记为 unverified"
        );
        assert!(!asset.shippable, "{named} 不得标为可分发");
    }

    // 白名单里的三个资产在清单里都必须是可分发的，否则代码与清单已经脱节。
    for shipped in SHIPPED_ASSETS {
        let asset = source
            .assets
            .iter()
            .find(|asset| asset.path == shipped)
            .unwrap_or_else(|| panic!("清单里应有 {shipped}"));
        assert!(
            asset.shippable,
            "{shipped} 在清单里不可分发，与代码白名单矛盾"
        );
        assert_ne!(asset.license_class, "unverified");
    }
}

/// 上游多出一个声部键必须是硬错误，不能被当成一个新声调收下。
#[test]
fn an_unregistered_top_level_key_fails_the_parse() {
    let raw = r#"{
      "上平声部": {"一东": ["东"]},
      "下平声部": {"一先": ["先"]},
      "上声部": {"一董": ["董"]},
      "去声部": {"一送": ["送"]},
      "入声部": {"入声一屋": ["屋"]},
      "新增声部": {"一新": ["新"]}
    }"#;
    let err = parse_pingshui(raw, "内联夹具").expect_err("未登记的顶层键必须报错");
    assert!(format!("{err}").contains("新增声部"), "{err}");
}

#[test]
fn a_missing_pingshui_tone_section_fails_the_parse() {
    let raw = r#"{"上平声部": {"一东": ["东"]}}"#;
    let err = parse_pingshui(raw, "内联夹具").expect_err("缺声部键必须报错");
    assert!(format!("{err}").contains("下平声部"), "{err}");
}

/// 词林正韵出现未知声键时不得猜一个声调——猜就是编造格律。
#[test]
fn an_unknown_cilin_tone_key_fails_the_parse() {
    let raw = r#"{"第一部": {"上声": ["董"]}}"#;
    let err = parse_cilin(raw, "内联夹具").expect_err("未知声键必须报错");
    let rendered = format!("{err}");
    assert!(rendered.contains("上声"), "{rendered}");
    assert!(rendered.contains("声调不可推测"), "{rendered}");
}

#[test]
fn an_empty_asset_fails_loudly_rather_than_yielding_zero_rows() {
    let raw = r#"{
      "上平声部": {}, "下平声部": {}, "上声部": {}, "去声部": {}, "入声部": {}
    }"#;
    let err = parse_pingshui(raw, "内联夹具").expect_err("空资产必须报错");
    assert!(format!("{err}").contains("不得空吞"), "{err}");
}

#[test]
fn a_bad_tone_value_fails_the_parse() {
    let err = parse_tone_table(r#"{"东": "中"}"#, "内联夹具").expect_err("非法平仄值必须报错");
    assert!(format!("{err}").contains("只接受 平 / 仄 / 多"), "{err}");
}

/// 完整上游检出上的实测数字。默认跳过，需要锁定 revision 的检出。
#[test]
#[ignore = "需要锁定 revision 的完整上游检出，由 YUNJIAN_CHINESE_WORD_RHYME_DIR 指定"]
fn full_checkout_matches_the_measured_totals() {
    let Ok(dir) = std::env::var("YUNJIAN_CHINESE_WORD_RHYME_DIR") else {
        panic!("请设置 YUNJIAN_CHINESE_WORD_RHYME_DIR 指向锁定 revision 的检出");
    };
    let import = import(dir).expect("完整检出应可导入");

    let pingshui = import.table(RhymeBook::Pingshui).expect("平水韵随包");
    assert_eq!(pingshui.group_count(), PINGSHUI_GROUP_TOTAL);
    assert_eq!(pingshui.entries().len(), 10_671, "平水韵字条数（含重复）");
    assert_eq!(pingshui.distinct_characters(), 8_232, "平水韵不同字数");

    let cilin = import.table(RhymeBook::Cilin).expect("词林正韵随包");
    assert_eq!(cilin.group_count(), CILIN_GROUP_TOTAL);
    assert_eq!(cilin.entries().len(), 5_575, "词林正韵字条数");
    assert_eq!(cilin.distinct_characters(), 5_037, "词林正韵不同字数");

    // 上游同一韵部内重复列出同一个字的条目数。去重前 10671 条里有 293 条是这种重复。
    assert_eq!(
        pingshui.duplicate_entries(),
        293,
        "平水韵同韵部内重复条目数"
    );

    let index = import.index(RhymeBook::Pingshui).expect("平水韵随包");
    assert_eq!(index.len(), 8_232, "反向索引字数");
    // 归属多个**不同**（声调, 韵部）的字数。按上游原始条目列表长度数会得到 1992，
    // 那个数字把同韵部内的重复也算成了跨韵部，是错的。
    assert_eq!(index.polyphone_count(), 1_823, "跨韵部字数");

    let check = &import.tone_cross_check;
    assert_eq!(check.declared_rows, 8_232, "Word_Tune 行数");
    assert_eq!(check.divergence_count(), 157, "与平仄表的分歧数");
    assert!(check.declared_only.is_empty());
    assert!(check.index_only.is_empty());
}
