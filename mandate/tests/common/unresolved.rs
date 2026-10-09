// `Unresolved` is `#[non_exhaustive]`, so tests compare it as
// `(rule_index, placeholder, outcome)` tuples (`inverted` is checked
// separately).

use mandate::{Unresolved, UnresolvedOutcome};

/// The rule index, placeholder and outcome of each of `reported`, in order.
pub fn as_tuples(reported: &[Unresolved]) -> Vec<(usize, &str, UnresolvedOutcome)> {
    reported
        .iter()
        .map(|u| (u.rule_index, u.placeholder.as_str(), u.outcome))
        .collect()
}
