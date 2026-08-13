//! 双路解码的类型边界：两路假设都进不了打字评分。
//!
//! 与 `crates/yunjian-recite/tests/ui/` 里那三条互补而不重复：那边守的是背诵内核**自己**
//! 导出的类型，这边守的是**语音生产路径实际产出的值**——`UnbiasedAsrHyp` 与
//! `biased_hyp()` 的返回值。2026-08-11 裁决作废了「无偏置即可评分」这条语义，因此
//! 无偏置那一路同样要被拦住，而不是只拦偏置那一路。

#[test]
fn neither_decode_pass_can_enter_typed_scoring() {
    let cases = trybuild::TestCases::new();
    cases.compile_fail("tests/ui/*.rs");
}
