//! Every other integration test needs all of the crate's features, and
//! compiles to nothing without them. This one fails instead, so that a plain
//! `cargo test` cannot pass by running no tests.

#[cfg(not(all(feature = "derive", feature = "chrono", feature = "uuid")))]
#[test]
fn integration_tests_need_all_features() {
    panic!(
        "the integration tests need `--all-features` (derive, chrono, uuid); run `cargo test --workspace --all-features`, which works on any toolchain"
    )
}
