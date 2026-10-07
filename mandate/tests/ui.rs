#![cfg(all(feature = "derive", feature = "chrono", feature = "uuid"))]
#[test]
fn ui() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/ui/fail/*.rs");
}
