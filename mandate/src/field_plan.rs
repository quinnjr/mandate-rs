//! Per-rule field plans: field masks computed by the database (spec §7.7).

use core::fmt;

use crate::{FieldMask, FieldSet, Plan, Resource};

/// The rules covering one action on `R`, in definition order, for adapters
/// that evaluate rule conditions in the database; see
/// [`Ability::field_plan`](crate::Ability::field_plan).
pub struct FieldPlan<R> {
    /// One entry per rule, in definition order.
    pub rules: Vec<FieldRule<R>>,
}

/// One rule of a [`FieldPlan`].
pub struct FieldRule<R> {
    /// Whether the rule is a `cannot`.
    pub inverted: bool,
    /// The fields the rule names; `None` means every field.
    pub fields: Option<FieldSet<R>>,
    /// The rule's condition; `None` means it always matches.
    pub cond: Option<Plan<R>>,
}

impl<R: Resource> FieldPlan<R> {
    /// The permitted fields of a row, where `matched(i)` says whether rule
    /// `i`'s condition holds for it (rules without a condition always match).
    ///
    /// For every fully loaded row this equals
    /// [`Ability::permitted_fields`](crate::Ability::permitted_fields).
    pub fn permitted(&self, matched: impl Fn(usize) -> bool) -> FieldSet<R> {
        let all = FieldMask::all(R::schema().fields.len());
        let mut set = FieldMask::default();
        for (i, rule) in self.rules.iter().enumerate() {
            if rule.cond.is_some() && !matched(i) {
                continue;
            }
            let fields = rule.fields.map_or(all, |f| f.mask());
            if rule.inverted {
                for f in fields.iter() {
                    set.remove(f);
                }
            } else {
                set = set.union(fields);
            }
        }
        FieldSet::from_mask(set)
    }
}

impl<R> Clone for FieldPlan<R> {
    fn clone(&self) -> Self {
        FieldPlan {
            rules: self.rules.clone(),
        }
    }
}

impl<R> fmt::Debug for FieldPlan<R> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FieldPlan")
            .field("rules", &self.rules)
            .finish()
    }
}

impl<R> Clone for FieldRule<R> {
    fn clone(&self) -> Self {
        FieldRule {
            inverted: self.inverted,
            fields: self.fields,
            cond: self.cond.clone(),
        }
    }
}

impl<R> fmt::Debug for FieldRule<R> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FieldRule")
            .field("inverted", &self.inverted)
            .field("fields", &self.fields)
            .field("cond", &self.cond)
            .finish()
    }
}
