//! The immutable rule set and its dense index.

use crate::condition::eval::eval;
use crate::condition::fold::Folded;
use crate::fieldset::walk_permitted;
use crate::plan::restrict;
use crate::{
    AbilityBuilder, Access, Action, CheckError, Condition, DynResource, EvalError, FieldIdx,
    FieldMask, FieldPlan, FieldRef, FieldRule, FieldSet, Forbidden, Plan, Resource, Rule, Subject,
    SubjectResource,
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
    ///
    /// # Examples
    ///
    /// ```
    /// # #[cfg(feature = "derive")] {
    /// # use mandate::{Ability, Access, Action, Cond, Resource, Subject};
    /// # #[derive(Clone, Debug, Resource)]
    /// # struct Post { id: i64, author_id: i64, title: String, body: String, locked: bool }
    /// # #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Action)]
    /// # enum Act { Read, Update, #[action(manage)] Manage }
    /// # #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Subject)]
    /// # enum Sub { #[subject(resource = Post)] Post, Dashboard, #[subject(all)] All }
    /// # let mine = Post { id: 1, author_id: 7, title: "Hi".into(), body: "Text".into(), locked: false };
    /// # let theirs = Post { author_id: 8, ..mine.clone() };
    /// let ability = Ability::<Act, Sub>::builder()
    ///     .can(Act::Read, Sub::Post)
    ///     .can(Act::Update, Sub::Post)
    ///         .when(Post::AUTHOR_ID.eq(7))
    ///         .fields([Post::TITLE.into(), Post::BODY.into()])
    ///     .cannot(Act::Update, Sub::Post)
    ///         .when(Post::LOCKED.eq(true))
    ///         .because("Locked posts are read-only")
    ///     .can(Act::Read, Sub::Dashboard)
    ///     .build()
    ///     .unwrap();
    /// assert!(ability.can(Act::Read, &mine));
    /// assert!(ability.can_type(Act::Read, Sub::Dashboard));
    /// # }
    /// ```
    pub fn builder() -> AbilityBuilder<A, S> {
        AbilityBuilder::new()
    }

    /// The bound rules in definition order.
    pub fn rules(&self) -> &[Rule<A, S>] {
        &self.rules
    }

    /// The position of cell `(a, s)` in `index`; `None` if either index is
    /// out of range (only possible with an inconsistent hand-written
    /// `Action`/`Subject` impl, which `build()` rejects).
    fn slot(a: A, s: S) -> Option<usize> {
        let (a, s) = (a.index(), s.index());
        (a < A::COUNT && s < S::COUNT).then(|| s * A::COUNT + a)
    }

    /// Indexes `rules`, expanding `MANAGE` and `ALL` into every cell they cover.
    ///
    /// `build()` has checked that `all()` and `index()` are consistent and
    /// that every rule names listed values; a value with an index out of
    /// range would be skipped (no check can reach its cell either).
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
                    if let Some(cell) = Self::slot(*a, *s).and_then(|k| index.get_mut(k)) {
                        cell.push(i as u32);
                    }
                }
            }
        }
        Self { rules, index }
    }

    /// Indices of the rules covering `(a, s)`, in definition order. Empty
    /// (so every check denies) for a value whose index is out of range.
    pub(crate) fn cell(&self, a: A, s: S) -> &[u32] {
        Self::slot(a, s)
            .and_then(|k| self.index.get(k))
            .map_or(&[], Vec::as_slice)
    }

    /// The first cell (subject-major, in `all()` order) whose rules switch
    /// between `can` and `cannot` more than `max` times, with its count.
    pub(crate) fn alternations_over(&self, max: usize) -> Option<(S, A, usize)> {
        S::all().iter().find_map(|&s| {
            A::all().iter().find_map(|&a| {
                let count = self
                    .cell(a, s)
                    .windows(2)
                    .filter(|w| {
                        self.rules[w[0] as usize].inverted() != self.rules[w[1] as usize].inverted()
                    })
                    .count();
                (count > max).then_some((s, a, count))
            })
        })
    }

    /// Fails closed unless `R`'s schema is the schema of its bound subject (spec §4.2).
    pub(crate) fn guard<R: SubjectResource<S>>() -> Result<(), EvalError> {
        let found = R::schema();
        match R::SUBJECT.schema() {
            Some(expected) if core::ptr::eq(expected, found) => Ok(()),
            Some(expected) => Err(EvalError::SchemaMismatch {
                expected: expected.name(),
                found: found.name(),
            }),
            None => Err(EvalError::SchemaMismatch {
                expected: R::SUBJECT.name(),
                found: found.name(),
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
    ///
    /// A `field` that `R`'s schema does not have (only a hand-built
    /// [`FieldRef`] can name one) fails with
    /// [`EvalError::InvalidCondition`] at path `#<index>`, so the check
    /// denies.
    fn decide<R: SubjectResource<S>>(
        &self,
        action: A,
        resource: &R,
        field: Option<FieldIdx>,
    ) -> Result<Option<&Rule<A, S>>, EvalError> {
        Self::guard::<R>()?;
        if let Some(f) = field
            && R::schema().field(f).is_none()
        {
            return Err(EvalError::InvalidCondition {
                path: format!("#{}", f.0),
            });
        }
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
        R::schema().field(field).map(|d| d.name())
    }

    /// Whether `action` is allowed on `resource`. Fails closed (`false`) on any error.
    ///
    /// # Examples
    ///
    /// ```
    /// # #[cfg(feature = "derive")] {
    /// # use mandate::{Ability, Access, Action, Cond, Resource, Subject};
    /// # #[derive(Clone, Debug, Resource)]
    /// # struct Post { id: i64, author_id: i64, title: String, body: String, locked: bool }
    /// # #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Action)]
    /// # enum Act { Read, Update, #[action(manage)] Manage }
    /// # #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Subject)]
    /// # enum Sub { #[subject(resource = Post)] Post, Dashboard, #[subject(all)] All }
    /// # let mine = Post { id: 1, author_id: 7, title: "Hi".into(), body: "Text".into(), locked: false };
    /// # let theirs = Post { author_id: 8, ..mine.clone() };
    /// # let ability = Ability::<Act, Sub>::builder()
    /// #     .can(Act::Read, Sub::Post)
    /// #     .can(Act::Update, Sub::Post).when(Post::AUTHOR_ID.eq(7))
    /// #     .build().unwrap();
    /// assert!(ability.can(Act::Read, &mine));
    /// assert!(ability.can(Act::Update, &mine));
    /// assert!(!ability.can(Act::Update, &theirs));
    /// # }
    /// ```
    pub fn can<R: SubjectResource<S>>(&self, action: A, resource: &R) -> bool {
        matches!(self.decide(action, resource, None), Ok(Some(r)) if !r.inverted())
    }

    /// Whether `action` is allowed on the `field` of `resource`. Fails closed.
    ///
    /// # Examples
    ///
    /// ```
    /// # #[cfg(feature = "derive")] {
    /// # use mandate::{Ability, Access, Action, Cond, Resource, Subject};
    /// # #[derive(Clone, Debug, Resource)]
    /// # struct Post { id: i64, author_id: i64, title: String, body: String, locked: bool }
    /// # #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Action)]
    /// # enum Act { Read, Update, #[action(manage)] Manage }
    /// # #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Subject)]
    /// # enum Sub { #[subject(resource = Post)] Post, Dashboard, #[subject(all)] All }
    /// # let mine = Post { id: 1, author_id: 7, title: "Hi".into(), body: "Text".into(), locked: false };
    /// # let theirs = Post { author_id: 8, ..mine.clone() };
    /// # let ability = Ability::<Act, Sub>::builder()
    /// #     .can(Act::Read, Sub::Post)
    /// #     .can(Act::Update, Sub::Post).when(Post::AUTHOR_ID.eq(7))
    /// #     .build().unwrap();
    /// assert!(ability.can_field(Act::Update, &mine, Post::TITLE));
    /// assert!(!ability.can_field(Act::Update, &theirs, Post::TITLE));
    /// # }
    /// ```
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
    ///
    /// # Examples
    ///
    /// ```
    /// # #[cfg(feature = "derive")] {
    /// # use mandate::{Ability, Access, Action, Cond, Resource, Subject};
    /// # #[derive(Clone, Debug, Resource)]
    /// # struct Post { id: i64, author_id: i64, title: String, body: String, locked: bool }
    /// # #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Action)]
    /// # enum Act { Read, Update, #[action(manage)] Manage }
    /// # #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Subject)]
    /// # enum Sub { #[subject(resource = Post)] Post, Dashboard, #[subject(all)] All }
    /// # let mine = Post { id: 1, author_id: 7, title: "Hi".into(), body: "Text".into(), locked: false };
    /// # let theirs = Post { author_id: 8, ..mine.clone() };
    /// # let ability = Ability::<Act, Sub>::builder()
    /// #     .can(Act::Read, Sub::Post)
    /// #     .can(Act::Update, Sub::Post).when(Post::AUTHOR_ID.eq(7))
    /// #     .build().unwrap();
    /// assert!(ability.check(Act::Update, &mine).is_ok());
    /// assert!(ability.check(Act::Update, &theirs).is_err());
    /// # }
    /// ```
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
    ///
    /// A field that `R`'s schema does not have (only a hand-built
    /// [`FieldRef`] can name one) is [`CheckError::Unresolvable`] with
    /// [`EvalError::InvalidCondition`] at path `#<index>`.
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
    /// unless `R`'s schema is its subject's, and with
    /// [`EvalError::InvalidCondition`] on a condition the plan cannot express
    /// (unreachable for validated rules).
    ///
    /// # Examples
    ///
    /// ```
    /// # #[cfg(feature = "derive")] {
    /// # use mandate::{Ability, Access, Action, Cond, Resource, Subject};
    /// # #[derive(Clone, Debug, Resource)]
    /// # struct Post { id: i64, author_id: i64, title: String, body: String, locked: bool }
    /// # #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Action)]
    /// # enum Act { Read, Update, #[action(manage)] Manage }
    /// # #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Subject)]
    /// # enum Sub { #[subject(resource = Post)] Post, Dashboard, #[subject(all)] All }
    /// # let mine = Post { id: 1, author_id: 7, title: "Hi".into(), body: "Text".into(), locked: false };
    /// # let theirs = Post { author_id: 8, ..mine.clone() };
    /// # let ability = Ability::<Act, Sub>::builder()
    /// #     .can(Act::Read, Sub::Post)
    /// #     .can(Act::Update, Sub::Post).when(Post::AUTHOR_ID.eq(7))
    /// #     .build().unwrap();
    /// match ability.access::<Post>(Act::Update).unwrap() {
    ///     Access::Filter(plan) => {
    ///         assert!(plan.eval(&mine).unwrap());
    ///         assert!(!plan.eval(&theirs).unwrap());
    ///     }
    ///     other => panic!("expected a filter, got {other:?}"),
    /// }
    /// assert_eq!(ability.access::<Post>(Act::Read).unwrap(), Access::All);
    /// assert_eq!(ability.access::<Post>(Act::Manage).unwrap(), Access::Denied);
    /// # }
    /// ```
    pub fn access<R: SubjectResource<S>>(&self, action: A) -> Result<Access<R>, EvalError> {
        Self::guard::<R>()?;
        let schema = R::schema();
        let f = formula(
            self.applicable(action, R::SUBJECT, None)
                .map(|r| (r.inverted(), r.condition())),
        );
        Ok(match restrict(f, schema)? {
            Folded::True => Access::All,
            Folded::False => Access::Denied,
            Folded::Cond(c) => Access::Filter(Plan::new(c)),
        })
    }

    /// One entry per rule covering `action` on `R`, in definition order, so a
    /// database can compute each condition and [`FieldPlan::permitted`] can
    /// combine them (§7.7).
    ///
    /// # Examples
    ///
    /// ```
    /// # #[cfg(feature = "derive")] {
    /// # use mandate::{Ability, Access, Action, Cond, Resource, Subject};
    /// # #[derive(Clone, Debug, Resource)]
    /// # struct Post { id: i64, author_id: i64, title: String, body: String, locked: bool }
    /// # #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Action)]
    /// # enum Act { Read, Update, #[action(manage)] Manage }
    /// # #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Subject)]
    /// # enum Sub { #[subject(resource = Post)] Post, Dashboard, #[subject(all)] All }
    /// # let mine = Post { id: 1, author_id: 7, title: "Hi".into(), body: "Text".into(), locked: false };
    /// # let theirs = Post { author_id: 8, ..mine.clone() };
    /// # let ability = Ability::<Act, Sub>::builder()
    /// #     .can(Act::Read, Sub::Post)
    /// #     .can(Act::Update, Sub::Post).when(Post::AUTHOR_ID.eq(7))
    /// #     .build().unwrap();
    /// let plan = ability.field_plan::<Post>(Act::Update).unwrap();
    /// assert_eq!(plan.rules.len(), 1);
    /// // the database says whether each rule's condition matched this row
    /// let permitted = plan.permitted(|_rule| true);
    /// assert!(permitted.contains(Post::TITLE));
    /// # }
    /// ```
    pub fn field_plan<R: SubjectResource<S>>(&self, action: A) -> Result<FieldPlan<R>, EvalError> {
        Self::guard::<R>()?;
        let schema = R::schema();
        let rules = self
            .cell(action, R::SUBJECT)
            .iter()
            .map(|&i| {
                let rule = &self.rules[i as usize];
                let restricted = rule
                    .condition()
                    .map(|c| restrict(c.clone(), schema))
                    .transpose()?;
                let cond = match restricted {
                    None | Some(Folded::True) => None,
                    Some(Folded::Cond(c)) => Some(Plan::new(c)),
                    // Unreachable for built rules; evaluates to false.
                    Some(Folded::False) => Some(Plan::new(Condition::constant(false))),
                };
                Ok(FieldRule {
                    inverted: rule.inverted(),
                    fields: rule.fields().map(FieldSet::from_mask),
                    cond,
                })
            })
            .collect::<Result<_, EvalError>>()?;
        Ok(FieldPlan { rules })
    }

    /// The fields of `resource` that `action` is permitted on (§7.4).
    ///
    /// # Examples
    ///
    /// ```
    /// # #[cfg(feature = "derive")] {
    /// # use mandate::{Ability, Access, Action, Cond, Resource, Subject};
    /// # #[derive(Clone, Debug, Resource)]
    /// # struct Post { id: i64, author_id: i64, title: String, body: String, locked: bool }
    /// # #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Action)]
    /// # enum Act { Read, Update, #[action(manage)] Manage }
    /// # #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Subject)]
    /// # enum Sub { #[subject(resource = Post)] Post, Dashboard, #[subject(all)] All }
    /// # let mine = Post { id: 1, author_id: 7, title: "Hi".into(), body: "Text".into(), locked: false };
    /// # let theirs = Post { author_id: 8, ..mine.clone() };
    /// # let ability = Ability::<Act, Sub>::builder()
    /// #     .can(Act::Read, Sub::Post)
    /// #     .can(Act::Update, Sub::Post).when(Post::AUTHOR_ID.eq(7))
    /// #     .build().unwrap();
    /// # let ability = Ability::<Act, Sub>::builder()
    /// #     .can(Act::Update, Sub::Post).when(Post::AUTHOR_ID.eq(7))
    /// #         .fields([Post::TITLE.into(), Post::BODY.into()])
    /// #     .build().unwrap();
    /// let fields = ability.permitted_fields(Act::Update, &mine).unwrap();
    /// let names: Vec<_> = fields.iter().map(|(_, name)| name).collect();
    /// assert_eq!(names, ["title", "body"]);
    /// assert!(ability.permitted_fields(Act::Update, &theirs).unwrap().mask().is_empty());
    /// # }
    /// ```
    pub fn permitted_fields<R: SubjectResource<S>>(
        &self,
        action: A,
        resource: &R,
    ) -> Result<FieldSet<R>, EvalError> {
        Self::guard::<R>()?;
        let dynr: &dyn DynResource = resource.as_dyn();
        let all = FieldMask::all(R::schema().fields().len());
        // The rules whose condition matches, in order; the first evaluation
        // error ends the walk and is returned instead of its result.
        let matched = self
            .cell(action, R::SUBJECT)
            .iter()
            .map(|&i| &self.rules[i as usize])
            .filter_map(|rule| {
                match rule
                    .condition()
                    .map_or(Ok(true), |c| eval(c, R::schema(), dynr))
                {
                    Ok(true) => Some(Ok((rule.inverted(), rule.fields()))),
                    Ok(false) => None,
                    Err(e) => Some(Err(e)),
                }
            });
        Ok(FieldSet::from_mask(walk_permitted(all, matched)?))
    }
}

/// The §7.6 formula over `(inverted, condition)` rules in definition order:
/// from `f = false`, `can c` gives `f = c ∨ f` and `cannot c` gives
/// `f = ¬c ∧ f`, where `c = true` for an unconditional rule.
///
/// A run of like rules `c1..ck` is built as one group, `Or[ck, …, c1, f]` or
/// `And[¬ck, …, ¬c1, f]`, which folds exactly as nesting them one by one, so
/// the depth grows with the alternations between `can` and `cannot`, not with
/// the number of rules. An unconditional rule decides everything before it,
/// so it resets `f` to its constant.
fn formula<'r>(rules: impl IntoIterator<Item = (bool, Option<&'r Condition>)>) -> Condition {
    /// Closes the run `cs` (in definition order) over `f`.
    fn close(mut cs: Vec<Condition>, inverted: bool, f: Condition) -> Condition {
        if cs.is_empty() {
            return f;
        }
        cs.reverse();
        cs.push(f);
        if inverted {
            Condition::And(cs)
        } else {
            Condition::Or(cs)
        }
    }

    let mut f = Condition::constant(false);
    let (mut run, mut run_inverted) = (Vec::new(), false);
    for (inverted, c) in rules {
        if inverted != run_inverted {
            f = close(core::mem::take(&mut run), run_inverted, f);
            run_inverted = inverted;
        }
        match c {
            // `true ∨ f` is true and `¬true ∧ f` is false.
            None => {
                run.clear();
                f = Condition::constant(!inverted);
            }
            Some(c) if inverted => run.push(Condition::Not(Box::new(c.clone()))),
            Some(c) => run.push(c.clone()),
        }
    }
    close(run, run_inverted, f)
}

#[cfg(all(test, feature = "derive", feature = "chrono", feature = "uuid"))]
mod tests {
    use super::formula;
    use crate::condition::fold::fold;
    use crate::test_fixture::{Action, Org, Post, Subject, post};
    use crate::{Ability, CheckError, Condition, EvalError, Quant, Resource, Rule};

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

    /// A rule: whether it is a `cannot`, and its condition.
    type R = (bool, Option<Condition>);

    /// The §7.6 formula as written: one nesting level per rule.
    fn nested(rules: &[R]) -> Condition {
        let mut f = Condition::constant(false);
        for (inverted, c) in rules {
            let c = c.clone().unwrap_or(Condition::constant(true));
            f = if *inverted {
                Condition::And(vec![Condition::Not(Box::new(c)), f])
            } else {
                Condition::Or(vec![c, f])
            };
        }
        f
    }

    #[test]
    fn formula_folds_like_per_rule_nesting() {
        let a = Post::AUTHOR_ID.eq(7).into_condition();
        let b = Post::LOCKED.eq(true).into_condition();
        let kinds: [R; 8] = [
            (false, None),
            (true, None),
            (false, Some(a.clone())),
            (false, Some(b.clone())),
            (false, Some(Condition::Or(vec![a.clone(), b.clone()]))),
            (true, Some(a.clone())),
            (true, Some(b.clone())),
            (true, Some(Condition::And(vec![a, b]))),
        ];
        // Every sequence of up to four rules.
        let mut level: Vec<Vec<R>> = vec![vec![]];
        let mut all = level.clone();
        for _ in 0..4 {
            level = level
                .iter()
                .flat_map(|seq| {
                    kinds.iter().map(|k| {
                        let mut seq = seq.clone();
                        seq.push(k.clone());
                        seq
                    })
                })
                .collect();
            all.extend(level.iter().cloned());
        }
        for seq in all {
            let coalesced = formula(seq.iter().map(|(inverted, c)| (*inverted, c.as_ref())));
            assert_eq!(
                fold(coalesced, Post::schema()),
                fold(nested(&seq), Post::schema()),
                "{seq:?}"
            );
        }
    }

    #[test]
    fn plans_fail_on_a_quantifier_over_a_non_relation_like_checks() {
        // `build()` rejects such a rule, so it is put in directly: a
        // quantifier over `org.name`, which is a scalar.
        let invalid = Condition::Rel {
            relation: Post::ORG.idx(),
            quant: Quant::One,
            cond: Some(Box::new(Condition::Rel {
                relation: Org::NAME.idx(),
                quant: Quant::Some,
                cond: None,
            })),
        };
        let err = EvalError::InvalidCondition {
            path: "org.name".into(),
        };
        let rule =
            |inverted, cond| Rule::new(Action::Read, Subject::Post, inverted, cond, None, None);
        // As a `can`, and as a `cannot` after an unconditional `can`, so the
        // plan rewrites it in either polarity.
        for rules in [
            vec![rule(false, Some(invalid.clone()))],
            vec![rule(false, None), rule(true, Some(invalid.clone()))],
        ] {
            let a = Ability::<Action, Subject>::from_rules(rules);
            assert_eq!(
                a.check(Action::Read, &post()),
                Err(CheckError::Unresolvable(err.clone()))
            );
            assert_eq!(a.access::<Post>(Action::Read), Err(err.clone()));
            assert_eq!(a.field_plan::<Post>(Action::Read).err(), Some(err.clone()));
        }
    }
}
