use super::*;
use crate::model::{Dynasty, LicenseClass, Provenance, ProvenanceKind, Script, SourceLocatorKind};

fn record(title: &str, body: &str) -> CanonicalRecord {
    CanonicalRecord {
        stable_id: "fixture".to_owned(),
        content_hash: "0000000000000000".to_owned(),
        work_group: "000000000000".to_owned(),
        edition_group: "000000000000".to_owned(),
        source_locator: "fixture:form".to_owned(),
        source_locator_kind: SourceLocatorKind::Native,
        genre: Genre::Shi,
        title: title.to_owned(),
        title_raw: title.to_owned(),
        ci_tune: None,
        author: "无名氏".to_owned(),
        dynasty: Dynasty::Tang,
        dynasty_raw: "唐".to_owned(),
        body_lines: vec![body.to_owned()],
        body_original: body.to_owned(),
        script: Script::Simplified,
        provenance: Provenance {
            source_name: "fixture".to_owned(),
            source_rev: "fixture".to_owned(),
            license: "public-domain".to_owned(),
            license_class: LicenseClass::PublicDomain,
            kind: ProvenanceKind::Original,
        },
    }
}

#[test]
fn fixed_length_structures_map_to_the_four_regulated_forms() {
    let wujue = record("五绝", "一二三四五，一二三四五。一二三四五，一二三四五。");
    let qilv = record(
        "七律",
        "一二三四五六七，一二三四五六七。一二三四五六七，一二三四五六七。\
         一二三四五六七，一二三四五六七。一二三四五六七，一二三四五六七。",
    );
    assert_eq!(classify(&wujue).expect("五绝分类").form, Form::Wujue);
    assert_eq!(classify(&qilv).expect("七律分类").form, Form::Qilv);
}

#[test]
fn unequal_line_lengths_are_irregular_and_a_ci_tune_has_priority() {
    let irregular = record("杂诗", "一二三四五，一二三四五六。一二三四五，一二三四五。");
    assert_eq!(
        classify(&irregular).expect("杂诗分类").form,
        Form::Irregular
    );

    let mut ci = irregular;
    ci.ci_tune = Some("念奴娇".to_owned());
    assert_eq!(classify(&ci).expect("词分类").form, Form::Ci);
}

#[test]
fn yuefu_is_an_independent_prefix_flag_and_never_matches_a_substring() {
    let yuefu = record("将进酒·君不见", "君不见黄河之水天上来，奔流到海不复回。");
    let classification = classify(&yuefu).expect("乐府分类");
    assert_eq!(classification.form, Form::Irregular);
    assert!(classification.is_yuefu);

    let not_yuefu = record(
        "黄鹤楼",
        "昔人已乘黄鹤去，此地空余黄鹤楼。黄鹤一去不复返，白云千载空悠悠。\
         晴川历历汉阳树，芳草萋萋鹦鹉洲。日暮乡关何处是，烟波江上使人愁。",
    );
    let classification = classify(&not_yuefu).expect("黄鹤楼分类");
    assert_eq!(classification.form, Form::Qilv);
    assert!(!classification.is_yuefu);

    let substring = record(
        "送友人将进酒楼",
        "一二三四五，一二三四五。一二三四五，一二三四五。",
    );
    assert!(!classify(&substring).expect("子串分类").is_yuefu);
}
