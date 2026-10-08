// `Unresolved` is `#[non_exhaustive]`: code outside the crate cannot build
// one, even with every current field, so fields can be added without a
// breaking change.
fn main() {
    let _ = mandate::Unresolved {
        rule_index: 0,
        placeholder: String::new(),
        outcome: mandate::UnresolvedOutcome::RuleDropped,
        inverted: false,
    };
}
