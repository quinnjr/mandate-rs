//! The immutable rule set and its dense index.

use crate::{AbilityBuilder, Action, Rule, Subject};

/// An immutable, indexed set of rules. `Send + Sync`; wrap in `Arc` to share.
#[derive(Clone, Debug)]
pub struct Ability<A, S> {
    rules: Vec<Rule<A, S>>,
    /// `S::COUNT * A::COUNT` cells of rule indices in definition order.
    index: Vec<Vec<u32>>,
}

impl<A: Action, S: Subject> Ability<A, S> {
    /// Starts building an ability.
    pub fn builder() -> AbilityBuilder<A, S> {
        AbilityBuilder::new()
    }

    /// The bound rules in definition order.
    pub fn rules(&self) -> &[Rule<A, S>] {
        &self.rules
    }

    /// Indexes `rules`, expanding `MANAGE` and `ALL` into every cell they cover.
    pub(crate) fn from_rules(rules: Vec<Rule<A, S>>) -> Self {
        let mut index = vec![Vec::new(); S::COUNT * A::COUNT];
        for (i, rule) in rules.iter().enumerate() {
            let (one_a, one_s) = ([rule.action()], [rule.subject()]);
            let actions: &[A] = if A::MANAGE == Some(rule.action()) {
                A::all()
            } else {
                &one_a
            };
            let subjects: &[S] = if S::ALL == Some(rule.subject()) {
                S::all()
            } else {
                &one_s
            };
            for s in subjects {
                for a in actions {
                    index[s.index() * A::COUNT + a.index()].push(i as u32);
                }
            }
        }
        Self { rules, index }
    }

    /// Indices of the rules covering `(a, s)`, in definition order.
    // Until the check API (Task 10) uses it, only tests do.
    #[allow(dead_code)]
    pub(crate) fn cell(&self, a: A, s: S) -> &[u32] {
        &self.index[s.index() * A::COUNT + a.index()]
    }
}

#[cfg(all(test, feature = "derive", feature = "chrono", feature = "uuid"))]
mod tests {
    use crate::Ability;
    use crate::test_fixture::{Action, Subject};

    #[test]
    fn index_expands_wildcards() {
        let a = Ability::<Action, Subject>::builder()
            .cannot(Action::Manage, Subject::All)
            .can(Action::Read, Subject::Post)
            .build()
            .unwrap();
        assert_eq!(a.cell(Action::Read, Subject::Post), [0, 1]);
        assert_eq!(a.cell(Action::Delete, Subject::Org), [0]);
        assert_eq!(a.cell(Action::Manage, Subject::Post), [0]);
    }
}
