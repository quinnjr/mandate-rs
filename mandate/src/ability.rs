//! The immutable rule set and its dense index.

use crate::condition::eval::eval;
use crate::condition::fold::{Folded, fold};
use crate::condition::nnf::nnf;
use crate::{
    AbilityBuilder, Access, Action, CheckError, Condition, DynResource, EvalError, FieldIdx,
    FieldMask, FieldRef, FieldSet, Forbidden, Plan, Resource, Rule, Subject, SubjectResource,
};

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
    pub(crate) fn cell(&self, a: A, s: S) -> &[u32] {
        &self.index[s.index() * A::COUNT + a.index()]
    }

    /// Fails closed unless `R`'s schema is the schema of its bound subject (spec §4.2).
    pub(crate) fn guard<R: SubjectResource<S>>() -> Result<(), EvalError> {
        let found = R::schema();
        match R::SUBJECT.schema() {
            Some(expected) if core::ptr::eq(expected, found) => Ok(()),
            Some(expected) => Err(EvalError::SchemaMismatch {
                expected: expected.name,
                found: found.name,
            }),
            None => Err(EvalError::SchemaMismatch {
                expected: R::SUBJECT.name(),
                found: found.name,
            }),
        }
    }

    /// Rules covering `(a, s)`, in definition order, after the §7.2 restriction filter.
    fn applicable(
        &self,
        a: A,
        s: S,
        field: Option<FieldIdx>,
    ) -> impl DoubleEndedIterator<Item = &Rule<A, S>> {
        self.cell(a, s)
            .iter()
            .map(|&i| &self.rules[i as usize])
            .filter(move |r| match field {
                Some(f) => r.fields().is_none_or(|m| m.contains(f)),
                None => !r.inverted() || r.fields().is_none(),
            })
    }

    /// The rule that decides an instance check (§7.1), if any.
    fn decide<R: SubjectResource<S>>(
        &self,
        action: A,
        resource: &R,
        field: Option<FieldIdx>,
    ) -> Result<Option<&Rule<A, S>>, EvalError> {
        Self::guard::<R>()?;
        let dynr = resource.as_dyn();
        for rule in self.applicable(action, R::SUBJECT, field).rev() {
            let hit = match rule.condition() {
                None => true,
                Some(c) => eval(c, R::schema(), dynr)?,
            };
            if hit {
                return Ok(Some(rule));
            }
        }
        Ok(None)
    }

    fn verdict(
        &self,
        action: A,
        subject: S,
        field: Option<&'static str>,
        decided: Option<&Rule<A, S>>,
    ) -> Result<(), Forbidden<A, S>> {
        match decided {
            Some(r) if !r.inverted() => Ok(()),
            _ => Err(Forbidden {
                action,
                subject,
                field,
                reason: decided
                    .and_then(|r| r.reason())
                    .map(|t| std::borrow::Cow::Owned(t.to_owned())),
            }),
        }
    }

    fn field_name<R: Resource>(field: FieldIdx) -> Option<&'static str> {
        R::schema().field(field).map(|d| d.name)
    }

    /// Whether `action` is allowed on `resource`. Fails closed (`false`) on any error.
    pub fn can<R: SubjectResource<S>>(&self, action: A, resource: &R) -> bool {
        matches!(self.decide(action, resource, None), Ok(Some(r)) if !r.inverted())
    }

    /// Whether `action` is allowed on the `field` of `resource`. Fails closed.
    pub fn can_field<R: SubjectResource<S>>(
        &self,
        action: A,
        resource: &R,
        field: impl Into<FieldRef<R>>,
    ) -> bool {
        let f = field.into().idx();
        matches!(self.decide(action, resource, Some(f)), Ok(Some(r)) if !r.inverted())
    }

    /// Like [`can`](Self::can), but explains a denial or an evaluation failure.
    pub fn check<R: SubjectResource<S>>(
        &self,
        action: A,
        resource: &R,
    ) -> Result<(), CheckError<A, S>> {
        let decided = self
            .decide(action, resource, None)
            .map_err(CheckError::Unresolvable)?;
        Ok(self.verdict(action, R::SUBJECT, None, decided)?)
    }

    /// Like [`can_field`](Self::can_field), but explains a denial or an evaluation failure.
    pub fn check_field<R: SubjectResource<S>>(
        &self,
        action: A,
        resource: &R,
        field: impl Into<FieldRef<R>>,
    ) -> Result<(), CheckError<A, S>> {
        let f = field.into().idx();
        let decided = self
            .decide(action, resource, Some(f))
            .map_err(CheckError::Unresolvable)?;
        Ok(self.verdict(action, R::SUBJECT, Self::field_name::<R>(f), decided)?)
    }

    /// The rule that decides a type-level check (§7.3), if any.
    fn decide_type(&self, action: A, subject: S) -> Option<&Rule<A, S>> {
        self.applicable(action, subject, None)
            .rev()
            .find(|r| r.condition().is_none() || !r.inverted())
    }

    /// Whether `action` is possibly allowed on some instance of `subject`.
    pub fn can_type(&self, action: A, subject: S) -> bool {
        matches!(self.decide_type(action, subject), Some(r) if !r.inverted())
    }

    /// Like [`can_type`](Self::can_type), but explains a denial.
    pub fn check_type(&self, action: A, subject: S) -> Result<(), Forbidden<A, S>> {
        self.verdict(action, subject, None, self.decide_type(action, subject))
    }

    /// The rows of `R` that `action` is allowed on, as a filter for a
    /// database query (§7.6, §8).
    ///
    /// A fully loaded `r` is in the result exactly when [`can`](Self::can)
    /// allows `action` on it. Fails closed with [`EvalError::SchemaMismatch`]
    /// unless `R`'s schema is its subject's.
    pub fn access<R: SubjectResource<S>>(&self, action: A) -> Result<Access<R>, EvalError> {
        Self::guard::<R>()?;
        let schema = R::schema();
        // The §7.6 formula; an unconditional rule's `true` resets `f`.
        let mut f = Condition::Or(vec![]);
        for rule in self.applicable(action, R::SUBJECT, None) {
            let c = rule.condition().cloned().unwrap_or(Condition::And(vec![]));
            f = if rule.inverted() {
                Condition::And(vec![Condition::Not(Box::new(c)), f])
            } else {
                Condition::Or(vec![c, f])
            };
        }
        let restricted = match fold(f, schema) {
            Folded::Cond(c) => fold(nnf(c, schema), schema),
            constant => constant,
        };
        Ok(match restricted {
            Folded::True => Access::All,
            Folded::False => Access::Denied,
            Folded::Cond(c) => Access::Filter(Plan::new(c)),
        })
    }

    /// The fields of `resource` that `action` is permitted on (§7.4).
    pub fn permitted_fields<R: SubjectResource<S>>(
        &self,
        action: A,
        resource: &R,
    ) -> Result<FieldSet<R>, EvalError> {
        Self::guard::<R>()?;
        let dynr: &dyn DynResource = resource.as_dyn();
        let all = FieldMask::all(R::schema().fields.len());
        let mut set = FieldMask::default();
        for &i in self.cell(action, R::SUBJECT) {
            let rule = &self.rules[i as usize];
            if let Some(c) = rule.condition() {
                if !eval(c, R::schema(), dynr)? {
                    continue;
                }
            }
            let fields = rule.fields().unwrap_or(all);
            if rule.inverted() {
                for f in fields.iter() {
                    set.remove(f);
                }
            } else {
                set = set.union(fields);
            }
        }
        Ok(FieldSet::from_mask(set))
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
