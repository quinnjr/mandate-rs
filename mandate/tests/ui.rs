#![cfg(all(feature = "derive", feature = "chrono", feature = "uuid"))]
// The `.stderr` snapshots quote compiler diagnostics, which change between
// Rust releases, so this test runs only on the compiler that generated them
// and is ignored on any other. The CI `ui` job passes `--include-ignored`, so
// if its toolchain and the version below ever disagree, the test still runs
// (and fails) instead of being skipped. Keep the two in step.
#[rustversion::attr(
    not(stable(1.99)),
    ignore = "trybuild snapshots are generated on Rust 1.99; run `cargo +1.99 test -p mandate-rs --all-features --test ui`"
)]
#[test]
fn ui() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/ui/fail/*.rs");
    t.pass("tests/ui/pass/*.rs");
}
