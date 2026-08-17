//! 数据表的测试。
//!
//! 全部不需要模型、声卡或语料库：三份 TSV 都由 `include_str!` 编译期内联，于是「数据是不是
//! 站得住」这个问题在默认构建（`cargo test --workspace`）里就能回答。

use std::collections::BTreeSet;

use super::{
    CITUNE_TSV, CiTunes, Confidence, LexiconError, PhonemeIndex, Polyphones, Poyin, RhythmSource,
    Roster, Syllable, assert_coverage, assert_tune_coverage, compile_overrides,
    evidence_matches_source, located_evidence,
};

// ---------------------------------------------------------------- 依据校验

#[test]
fn empty_evidence_is_rejected() {
    assert_eq!(located_evidence(""), Err("依据为空"));
    assert_eq!(located_evidence("   "), Err("依据为空"));
}

/// 这条守的是「有引用的形状但没有引用的内容」。一句「据某本」听起来像出处，但第三方
/// 无法据它翻到任何一页，所以它必须和空依据一样被拒。
#[test]
fn evidence_without_a_locator_is_rejected() {
    assert_eq!(
        located_evidence("据 chinese_word_rhyme 锁定版转录本"),
        Err("缺卷次/部次/页码/样本量定位符")
    );
}

#[test]
fn evidence_without_an_edition_is_rejected() {
    assert_eq!(
        located_evidence("《平水韵》下平声部 六麻 收「斜」"),
        Err("未说明所据版本（需形如「据…本」）")
    );
}

#[test]
fn located_and_editioned_evidence_passes() {
    assert!(
        located_evidence("《平水韵》下平声部 六麻；据 chinese_word_rhyme 锁定版转录本").is_ok()
    );
    assert!(located_evidence("《唐诗三百首》卷八 七言绝句；据 tags.toml 已评审名单转录本").is_ok());
}

/// 实测类依据用样本量定位，它不含「卷」「部」这类关键字，但同样可复现，所以也要放行。
#[test]
fn sample_size_counts_as_a_locator() {
    assert!(
        located_evidence("《全宋词》念奴娇实测，n=135；据 chinese-poetry 锁定版转录本").is_ok()
    );
}

// ---------------------------------------------------------------- 破读词表

#[test]
fn shipped_poyin_parses() {
    let poyin = Poyin::shipped().expect("随仓破读词表应可解析");
    assert!(
        poyin.rows().len() >= 80,
        "破读词表只有 {} 行，远少于名册引入的多音字数，疑为文件被截断",
        poyin.rows().len()
    );
}

/// 验收条目：每一行的依据非空，且带一个含卷次或页码的定位符。
///
/// 解析本身已经在拒绝不成立的依据，所以这条测试是**冗余的**——而这正是它存在的理由：
/// 有人若把 `Poyin::parse` 里的校验去掉，解析会照常成功，只有这条断言会红。
#[test]
fn every_poyin_row_carries_located_evidence() {
    let poyin = Poyin::shipped().expect("应可解析");
    for row in poyin.rows() {
        assert!(
            located_evidence(&row.evidence).is_ok(),
            "字 {} 的依据不成立：{:?}",
            row.character,
            row.evidence
        );
    }
}

/// 三个黄金破读用例。**读音全部自公有领域韵书推得**，依据写在词表里。
#[test]
fn the_three_golden_readings_come_from_the_lexicon() {
    let poyin = Poyin::shipped().expect("应可解析");
    let cases = [
        ('斜', "远上寒山石径斜，白云生处有人家。", "xiá"),
        ('衰', "少小离家老大回，乡音难改鬓毛衰。", "cuī"),
        ('骑', "一骑红尘妃子笑，无人知是荔枝来。", "jì"),
    ];
    for (character, line, want) in cases {
        let row = poyin
            .reading(character, line)
            .unwrap_or_else(|| panic!("破读词表缺 {character} 在 {line:?} 里的读音"));
        assert_eq!(
            row.pinyin.as_deref(),
            Some(want),
            "{character} 的读音不对；依据：{}",
            row.evidence
        );
    }
}

/// 语境不匹配就不该覆写：「斜」在别的句子里未必读 xiá，词表写的是 `石径斜` 这个片段。
#[test]
fn a_context_row_does_not_leak_to_other_lines() {
    let poyin = Poyin::shipped().expect("应可解析");
    assert!(
        poyin.reading('斜', "月落乌啼霜满天").is_none(),
        "语境限定失效：不含「石径斜」的句子也拿到了破读"
    );
}

#[test]
fn readings_in_a_line_are_returned_in_order() {
    let poyin = Poyin::shipped().expect("应可解析");
    let hits = poyin.readings_in("远上寒山石径斜，白云生处有人家。");
    assert_eq!(
        hits.iter().map(|(c, _)| *c).collect::<Vec<_>>(),
        vec!['斜'],
        "本句只应命中「斜」"
    );
}

#[test]
fn engine_default_rows_do_not_override() {
    let poyin = Poyin::shipped().expect("应可解析");
    let ledger = poyin
        .rows()
        .iter()
        .filter(|row| row.confidence == Confidence::EngineDefault)
        .count();
    assert!(ledger > 0, "处置台账不应为空");
    assert_eq!(
        poyin.override_count() + ledger,
        poyin.rows().len(),
        "每一行要么覆写要么登记处置，没有第三种"
    );
}

/// `pinyin` 与 `confidence` 必须互相一致，否则读者无法判断一行到底生效不生效。
#[test]
fn a_row_claiming_to_override_must_give_a_reading() {
    let text = "字\tcontext\tpinyin\t依据\tconfidence\n\
                斜\t石径斜\t-\t《平水韵》下平声部 六麻；据某锁定版转录本\trhyme_attested\n";
    let error = Poyin::parse(text).expect_err("覆写行没给拼音，应拒绝");
    assert!(
        matches!(error, LexiconError::BadField { .. }),
        "得到 {error:?}"
    );
}

#[test]
fn an_engine_default_row_must_not_give_a_reading() {
    let text = "字\tcontext\tpinyin\t依据\tconfidence\n\
                斜\t*\txiá\t《平水韵》下平声部 六麻；据某锁定版转录本\tengine_default\n";
    let error = Poyin::parse(text).expect_err("不覆写却给了拼音，应拒绝");
    assert!(
        matches!(error, LexiconError::BadField { .. }),
        "得到 {error:?}"
    );
}

#[test]
fn an_unlocated_row_is_rejected_with_its_line_number() {
    let text = "字\tcontext\tpinyin\t依据\tconfidence\n\
                斜\t石径斜\txiá\t据某锁定版转录本\trhyme_attested\n";
    let error = Poyin::parse(text).expect_err("依据无定位符，应拒绝");
    match error {
        LexiconError::Unlocated { line, reason, .. } => {
            assert_eq!(line, 2);
            assert_eq!(reason, "缺卷次/部次/页码/样本量定位符");
        }
        other => panic!("得到 {other:?}"),
    }
}

// ---------------------------------------------------------------- 词谱句式表

#[test]
fn shipped_citune_table_parses_and_every_row_is_located() {
    let tunes = CiTunes::shipped().expect("随仓句式表应可解析");
    assert!(!tunes.is_empty(), "句式表不应为空");
    for tune in ["念奴娇", "水调歌头"] {
        let row = tunes.get(tune).unwrap_or_else(|| panic!("缺词牌 {tune}"));
        assert!(
            located_evidence(&row.evidence).is_ok(),
            "{tune} 的依据不成立：{:?}",
            row.evidence
        );
        assert!(row.pattern.len() > 1, "{tune} 的句式只有一句，疑为解析错误");
    }
}

/// **v1 的诚实声明由测试守着。** 仓库内没有任何公有领域词谱（唯一一份 `Ci_Tunes.json` 已被
/// todo 15 扣留），所以宣称词谱权威的行数必须是 0。哪天真的有了词谱数据，这条会红，
/// 提醒改的人同时更新 `citune_rhythm.tsv` 顶部那段声明——那段声明不该悄悄过期。
#[test]
fn v1_claims_no_citune_authority() {
    let tunes = CiTunes::shipped().expect("应可解析");
    assert_eq!(
        tunes.citune_authority_count(),
        0,
        "有行声称词谱权威，但仓库内没有公有领域词谱数据；\
         若已引入，请同步更新 data/citune_rhythm.tsv 顶部的声明"
    );
}

#[test]
fn an_unknown_source_value_is_rejected() {
    let text = "词牌\t句式\t来源\t依据\n念奴娇\t4-5\t钦定词谱\t卷一；据某本\n";
    let error = CiTunes::parse(text).expect_err("来源列不认识的值应拒绝");
    assert!(
        matches!(error, LexiconError::BadField { .. }),
        "得到 {error:?}"
    );
}

#[test]
fn a_zero_width_clause_is_rejected() {
    let text = "词牌\t句式\t来源\t依据\n念奴娇\t4-0-5\tcorpus_modal\t众数句式 n=135 实测；据某锁定版转录本\n";
    assert!(matches!(
        CiTunes::parse(text),
        Err(LexiconError::BadField { .. })
    ));
}

// ------------------------------------------------ 依据类型与所声明来源必须一致

/// **这条钉住的是「依据合格」与「依据类型正确」的区别。** 随仓每一行都已过
/// `located_evidence`，但那只证明能被翻到；这条要求实测行看起来就是实测、词谱行看起来就是
/// 词谱，两者不可互换措辞。
#[test]
fn every_shipped_row_evidence_matches_its_declared_source() {
    let tunes = CiTunes::shipped().expect("随仓句式表应可解析");
    for tune in tunes.tunes() {
        let row = tunes.get(&tune).expect("刚枚举出来的词牌应存在");
        assert_eq!(
            evidence_matches_source(row.source, &row.evidence),
            Ok(()),
            "{tune} 的依据与来源 {} 不符：{:?}",
            row.source.as_str(),
            row.evidence
        );
    }
}

/// **防洗白的那一半。** 把《全宋词》实测众数写成「《钦定词谱》卷五页三」能过
/// `located_evidence`（卷与页都是合格定位符），却把统计推断冒充成了词谱权威。
#[test]
fn a_modal_row_dressed_up_as_a_citune_citation_is_rejected() {
    let laundered = "《钦定词谱》卷五页三；据某影印本";
    assert_eq!(located_evidence(laundered), Ok(()));

    let text = format!("词牌\t句式\t来源\t依据\n念奴娇\t4-5\tcorpus_modal\t{laundered}\n");
    match CiTunes::parse(&text).expect_err("洗白的实测依据应被拒") {
        LexiconError::ProvenanceMismatch {
            declared, reason, ..
        } => {
            assert_eq!(declared, "corpus_modal");
            assert_eq!(reason, "实测依据引了词谱书名，等于把统计推断冒充词谱权威");
        }
        other => panic!("得到 {other:?}"),
    }
}

/// 只写卷页不写书名也不行：光有卷页说不清是哪部书的卷页。
#[test]
fn a_modal_row_claiming_a_volume_locator_is_rejected() {
    let text =
        "词牌\t句式\t来源\t依据\n念奴娇\t4-5\tcorpus_modal\t卷五页三 n=135 实测；据某影印本\n";
    match CiTunes::parse(text).expect_err("实测依据写卷页应被拒") {
        LexiconError::ProvenanceMismatch { reason, .. } => {
            assert_eq!(reason, "实测依据写了卷次或页码，等于把统计推断冒充词谱权威");
        }
        other => panic!("得到 {other:?}"),
    }
}

/// 反方向同样要拦：一条声称词谱权威的行必须真的给出书名、卷次与页码。
#[test]
fn a_citune_row_must_carry_a_work_name_a_volume_and_a_page() {
    for (evidence, want) in [
        ("《全宋词》卷五页三；据某影印本", "词谱依据未写出词谱书名"),
        ("《钦定词谱》页三；据某影印本", "词谱依据缺卷次"),
        ("《钦定词谱》卷五；据某影印本", "词谱依据缺页码"),
        (
            "《钦定词谱》卷五页三 n=135 实测；据某影印本",
            "词谱依据里出现实测口径，来源类型自相矛盾",
        ),
    ] {
        assert_eq!(
            evidence_matches_source(RhythmSource::CiTune, evidence),
            Err(want),
            "{evidence:?} 应因 {want} 被拒"
        );
    }
    assert_eq!(
        evidence_matches_source(RhythmSource::CiTune, "《钦定词谱》卷五页三；据某影印本"),
        Ok(())
    );
}

/// `char_count` 与 `punctuation` 是运行期推得的，不是数据行能声明的来源。
#[test]
fn runtime_only_sources_cannot_be_declared_by_a_row() {
    for source in [RhythmSource::CharCount, RhythmSource::Punctuation] {
        assert_eq!(
            evidence_matches_source(source, "《钦定词谱》卷五页三；据某影印本"),
            Err("该来源是运行期推得的切分方式，不能作为表内行的来源")
        );
    }
}

/// **这条守的是本项目栽过的那个坑：解释一条规则的文字命中这条规则。**
/// `citune_rhythm.tsv` 顶部的声明里逐字写着「《钦定词谱》卷 X 页 Y」当反例，如果解析或校验
/// 把注释行也读成数据行，那段声明本身就会被当成一条词谱依据——校验照绿，而表其实是空的。
#[test]
fn the_header_comment_block_is_not_read_as_a_row() {
    assert!(
        CITUNE_TSV.contains("《钦定词谱》"),
        "顶部声明应仍带那个反例，否则这条测试失去意义"
    );

    let tunes = CiTunes::shipped().expect("随仓句式表应可解析");
    assert_eq!(tunes.len(), 2, "表内应恰有 2 个数据行，注释行不计入");
    for tune in tunes.tunes() {
        let row = tunes.get(&tune).expect("刚枚举出来的词牌应存在");
        assert!(
            !row.evidence.contains("词谱"),
            "{tune} 的依据引了词谱书名，但仓库内没有公有领域词谱：{:?}",
            row.evidence
        );
    }
}

// ------------------------------------------------------------ 词牌覆盖闭合

/// **覆盖是闭合而不是百分比。** 分母是名册里出现的词牌——方案要求覆盖宋词三百首所含词牌，
/// 而该选本的收录名单没有任何随包资产，仓库能知道的成员就是名册标注的那些。
#[test]
fn tune_coverage_over_the_roster_is_closed() {
    let tunes = CiTunes::shipped().expect("应可解析");
    let roster = Roster::shipped().expect("应可解析");

    let in_roster: BTreeSet<String> = roster
        .entries()
        .iter()
        .filter_map(|entry| entry.ci_tune.clone())
        .collect();
    assert_eq!(
        in_roster,
        tunes.tunes(),
        "名册词牌集与句式表词牌集必须相等：多出来的没有依据，少掉的没有句读"
    );
    assert_eq!(assert_tune_coverage(&tunes, &roster), Ok(()));
}

/// 名册加一支表外词牌就必须变红，否则这条闭合检查是装饰。
#[test]
fn a_roster_tune_absent_from_the_table_is_a_coverage_gap() {
    let tunes = CiTunes::shipped().expect("应可解析");
    let roster = Roster::parse(
        "id\t选本\t作者\t题目\t词牌\t正文\t依据\n\
         t1\t宋词三百首\t李清照\t声声慢·寻寻觅觅\t声声慢\t寻寻觅觅，冷冷清清。\
         \t《宋词三百首》卷一；据某锁定版转录本\n",
    )
    .expect("名册应可解析");

    match assert_tune_coverage(&tunes, &roster).expect_err("表外词牌应被报为缺口") {
        LexiconError::TuneCoverageGap { missing } => {
            assert_eq!(missing, BTreeSet::from(["声声慢".to_owned()]));
        }
        other => panic!("得到 {other:?}"),
    }
}

// ---------------------------------------------------------------- 名册与覆盖

#[test]
fn shipped_roster_parses_and_every_row_is_located() {
    let roster = Roster::shipped().expect("随仓名册应可解析");
    assert_eq!(roster.entries().len(), 22, "名册应为 22 首");
    for entry in roster.entries() {
        assert!(
            located_evidence(&entry.evidence).is_ok(),
            "{} 《{}》的依据不成立：{:?}",
            entry.author,
            entry.title,
            entry.evidence
        );
    }
}

/// 名册记录的选本归属必须与 `tags.toml` 的人工评审名单一致：8 首。
///
/// 这个数字**故意钉死**。它是「v1 不是两个选本的完整覆盖」这句诚实声明的量化形式：
/// 谁把它改大而没有同时扩充名册与破读表，这条就会红。
#[test]
fn anthology_membership_matches_the_reviewed_list() {
    let roster = Roster::shipped().expect("应可解析");
    assert_eq!(roster.in_anthology("唐诗三百首"), 6);
    assert_eq!(roster.in_anthology("宋词三百首"), 2);
}

#[test]
fn shipped_polyphone_index_parses() {
    let polyphones = Polyphones::shipped().expect("随仓多音字索引应可解析");
    assert!(
        polyphones.characters().len() > 1_500,
        "多音字索引只有 {} 字，疑为文件被截断",
        polyphones.characters().len()
    );
    for character in ['骑', '思', '还', '深'] {
        assert!(polyphones.contains(character), "{character} 应判为多音字");
    }
}

/// 覆盖闭合：名册正文里每一个多音字都在破读词表里有一行。
///
/// 候选集来自 `polyphone_index.tsv`，与词表是两份互相独立的数据，因此这条断言不是循环的。
#[test]
fn coverage_over_the_roster_is_closed() {
    let poyin = Poyin::shipped().expect("应可解析");
    let roster = Roster::shipped().expect("应可解析");
    let polyphones = Polyphones::shipped().expect("应可解析");
    assert_coverage(&poyin, &roster, polyphones.characters()).expect("名册覆盖应闭合");
}

/// 反向验证上一条真的会红：拿一个空词表去查，缺口必须被列出来。
#[test]
fn coverage_gap_is_reported_with_the_missing_characters() {
    let roster = Roster::shipped().expect("应可解析");
    let polyphones = Polyphones::shipped().expect("应可解析");
    let error = assert_coverage(&Poyin::default(), &roster, polyphones.characters())
        .expect_err("空词表必须报缺口");
    match error {
        LexiconError::CoverageGap { missing } => {
            assert!(missing.contains(&'骑'), "缺口里应含「骑」");
            assert!(
                missing.len() > 50,
                "缺口只有 {} 字，疑为名册没被读到",
                missing.len()
            );
        }
        other => panic!("得到 {other:?}"),
    }
}

/// 名册外的多音字不该被要求覆盖：覆盖是相对名册闭合，不是相对整部韵书。
#[test]
fn characters_outside_the_roster_are_not_demanded() {
    let poyin = Poyin::shipped().expect("应可解析");
    let roster = Roster::shipped().expect("应可解析");
    let mut candidates = BTreeSet::new();
    candidates.insert('龘');
    assert_coverage(&poyin, &roster, &candidates).expect("名册外的字不该被要求");
}

// ---------------------------------------------------------------- 切分依据标识

#[test]
fn rhythm_source_identifiers_are_stable() {
    assert_eq!(RhythmSource::CharCount.as_str(), "char_count");
    assert_eq!(RhythmSource::CiTune.as_str(), "citune");
    assert_eq!(RhythmSource::CorpusModal.as_str(), "corpus_modal");
    assert_eq!(RhythmSource::Punctuation.as_str(), "punctuation");
}

/// 只有词谱才算词谱权威。实测众数与标点回落都不算——界面据这个判据决定怎么措辞。
#[test]
fn only_citune_claims_authority() {
    assert!(RhythmSource::CiTune.claims_citune_authority());
    for source in [
        RhythmSource::CharCount,
        RhythmSource::CorpusModal,
        RhythmSource::Punctuation,
    ] {
        assert!(
            !source.claims_citune_authority(),
            "{source:?} 不该宣称词谱权威"
        );
    }
}

// ---------------------------------------------------------------- 拼音与音素

#[test]
fn tone_marks_and_trailing_digits_are_both_accepted() {
    assert_eq!(
        Syllable::parse("xiá").expect("应可解析"),
        Syllable {
            base: "xia".to_owned(),
            tone: 2
        }
    );
    assert_eq!(
        Syllable::parse("xia2").expect("应可解析"),
        Syllable {
            base: "xia".to_owned(),
            tone: 2
        }
    );
}

#[test]
fn a_toneless_syllable_is_neutral() {
    assert_eq!(Syllable::parse("de").expect("应可解析").tone, 5);
}

#[test]
fn v_is_normalised_to_u_umlaut() {
    assert_eq!(Syllable::parse("lv4").expect("应可解析").base, "lü");
    assert_eq!(Syllable::parse("lǜ").expect("应可解析").base, "lü");
}

#[test]
fn a_malformed_pinyin_is_rejected() {
    assert!(Syllable::parse("").is_err());
    assert!(Syllable::parse("xiá2").is_err(), "两个调号应拒绝");
    assert!(Syllable::parse("xia9").is_err(), "调号越界应拒绝");
    assert!(Syllable::parse("石径斜").is_err(), "汉字不是拼音");
}

/// 这份小词典的形态与 MeloTTS 中文包完全一致（实测每个单字条目都是 `字 声母 韵母 调 调`）。
const FAKE_LEXICON: &str = "\
斜 x ie 2 2
霞 x ia 2 2
石 sh iii 2 2
径 j ing 4 4
安 AA an 1 1
绿 l v 4 4
一溜歪斜 i → l j o u → w a i → x ie 2 2
";

#[test]
fn only_single_character_entries_are_indexed() {
    let index = PhonemeIndex::parse(FAKE_LEXICON);
    assert_eq!(index.len(), 6, "多字条目不该进索引");
    assert_eq!(index.character('斜'), Some("x ie 2 2"));
}

/// 无声母音节的伪声母要丢掉，否则 `AA an` 会被拼成 `aaan`，目标读音就永远查不到。
#[test]
fn pseudo_initials_are_dropped_when_recovering_a_syllable() {
    let index = PhonemeIndex::parse(FAKE_LEXICON);
    let syllable = Syllable::parse("ān").expect("应可解析");
    assert_eq!(index.syllable(&syllable), Some("AA an 1 1"));
}

#[test]
fn the_lexicon_v_is_indexed_as_u_umlaut() {
    let index = PhonemeIndex::parse(FAKE_LEXICON);
    let syllable = Syllable::parse("lǜ").expect("应可解析");
    assert_eq!(index.syllable(&syllable), Some("l v 4 4"));
}

/// 覆写词条是**多字**的：这样「石径斜」读 xiá，而单独一个「斜」字仍走默认读音——这正是
/// 破读表用语境列限定读音的意思。
#[test]
fn an_override_is_a_multi_character_entry_borrowing_a_homophone() {
    let poyin = Poyin::parse(
        "字\tcontext\tpinyin\t依据\tconfidence\n\
         斜\t石径斜\txiá\t《平水韵》下平声部 六麻；据某锁定版转录本\trhyme_attested\n",
    )
    .expect("应可解析");
    let overrides =
        compile_overrides(&poyin, &PhonemeIndex::parse(FAKE_LEXICON)).expect("应可编译");
    assert_eq!(overrides.len(), 1);
    assert_eq!(overrides[0].word, "石径斜");
    assert_eq!(
        overrides[0].line(),
        "石径斜 sh iii 2 2 j ing 4 4 x ia 2 2",
        "被破读的字取 霞 的音素，其余取各自默认音素"
    );
}

/// 一个音素都不许手写，所以引擎音系里表达不出来的读音必须报错而不是跳过：
/// 静默失效的破读听起来和没有破读表一模一样。
#[test]
fn an_unreachable_reading_is_an_error_not_a_silent_skip() {
    let poyin = Poyin::parse(
        "字\tcontext\tpinyin\t依据\tconfidence\n\
         斜\t石径斜\txiǎ\t《平水韵》下平声部 六麻；据某锁定版转录本\trhyme_attested\n",
    )
    .expect("应可解析");
    let problems =
        compile_overrides(&poyin, &PhonemeIndex::parse(FAKE_LEXICON)).expect_err("应报错");
    assert!(problems[0].contains("没有同音条目"), "{problems:?}");
}

#[test]
fn a_context_character_missing_from_the_lexicon_is_an_error() {
    let poyin = Poyin::parse(
        "字\tcontext\tpinyin\t依据\tconfidence\n\
         斜\t小径斜\txiá\t《平水韵》下平声部 六麻；据某锁定版转录本\trhyme_attested\n",
    )
    .expect("应可解析");
    let problems =
        compile_overrides(&poyin, &PhonemeIndex::parse(FAKE_LEXICON)).expect_err("应报错");
    assert!(problems[0].contains("不在引擎词典里"), "{problems:?}");
}

#[test]
fn ledger_rows_produce_no_overrides() {
    let poyin = Poyin::parse(
        "字\tcontext\tpinyin\t依据\tconfidence\n\
         斜\t*\t-\t《平水韵》下平声部 六麻；据某锁定版转录本\tengine_default\n",
    )
    .expect("应可解析");
    let overrides =
        compile_overrides(&poyin, &PhonemeIndex::parse(FAKE_LEXICON)).expect("应可编译");
    assert!(overrides.is_empty(), "不覆写的行不该产出词条");
}
