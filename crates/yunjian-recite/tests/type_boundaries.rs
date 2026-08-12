#[test]
fn voice_derived_types_cannot_enter_typed_scoring() {
    let cases = trybuild::TestCases::new();
    cases.compile_fail("tests/ui/*.rs");
}
