//! The rule builder: groups of rules validated and folded at `build()`.

use std::borrow::Cow;

use crate::condition::fold::{Folded, fold};
use crate::{
    Ability, Action, Bound, BuildError, Cond, Condition, FieldIdx, FieldMask, FieldRef, Resource,
    Rule, Schema, Subject,
};

/// Converts one or many actions into a list.
pub trait IntoActions<A> {
    /// The actions as a vector.
    fn into_vec(self) -> Vec<A>;
}

/// Converts one or many subjects into a list.
pub trait IntoSubjects<S> {
    /// The subjects as a vector.
    fn into_vec(self) -> Vec<S>;
}

macro_rules! into_list {
    ($trait:ident, $bound:ident) => {
        impl<T: $bound> $trait<T> for T {
            fn into_vec(self) -> Vec<T> {
                vec![self]
            }
        }
        impl<T: $bound, const N: usize> $trait<T> for [T; N] {
            fn into_vec(self) -> Vec<T> {
                Vec::from(self)
            }
        }
        impl<T: $bound> $trait<T> for &[T] {
            fn into_vec(self) -> Vec<T> {
                self.to_vec()
            }
        }
        impl<T: $bound> $trait<T> for Vec<T> {
            fn into_vec(self) -> Vec<T> {
                self
            }
        }
    };
}
into_list!(IntoActions, Action);
into_list!(IntoSubjects, Subject);

/// A pending rule group, validated at `build()`.
struct Group<A, S> {
    actions: Vec<A>,
    subjects: Vec<S>,
    inverted: bool,
    conds: Vec<(Condition, &'static Schema)>,
    fields: Vec<(Vec<FieldIdx>, &'static Schema)>,
    reason: Option<Cow<'static, str>>,
}

/// One step of the definition, in definition order.
enum Entry<A, S> {
    /// A `can`/`cannot` group.
    Group(Group<A, S>),
    /// Rules added by `extend`, already expanded; each condition is over its
    /// subject's schema.
    Bound(Vec<Rule<A, S>>),
}

/// Collects rule groups and builds an [`Ability`].
pub struct AbilityBuilder<A, S> {
    entries: Vec<Entry<A, S>>,
}

impl<A: Action, S: Subject> AbilityBuilder<A, S> {
    pub(crate) fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    /// Adds rules bound from stored templates, after the rules defined so
    /// far and before any defined later. `build()` folds them like the
    /// others.
    pub fn extend(mut self, bound: Bound<A, S>) -> Self {
        self.entries.push(Entry::Bound(bound.into_rules()));
        self
    }

    /// Starts a group of `can` rules.
    pub fn can(
        self,
        actions: impl IntoActions<A>,
        subjects: impl IntoSubjects<S>,
    ) -> GroupBuilder<A, S> {
        GroupBuilder::start(self.entries, false, actions.into_vec(), subjects.into_vec())
    }

    /// Starts a group of `cannot` rules.
    pub fn cannot(
        self,
        actions: impl IntoActions<A>,
        subjects: impl IntoSubjects<S>,
    ) -> GroupBuilder<A, S> {
        GroupBuilder::start(self.entries, true, actions.into_vec(), subjects.into_vec())
    }

    /// Validates, folds, expands, and indexes every group.
    pub fn build(self) -> Result<Ability<A, S>, BuildError> {
        build(self.entries)
    }
}

/// The group under construction; `when`, `fields`, `because` apply to every
/// (action, subject) pair in it.
pub struct GroupBuilder<A, S> {
    done: Vec<Entry<A, S>>,
    cur: Group<A, S>,
}

impl<A: Action, S: Subject> GroupBuilder<A, S> {
    fn start(done: Vec<Entry<A, S>>, inverted: bool, actions: Vec<A>, subjects: Vec<S>) -> Self {
        let cur = Group {
            actions,
            subjects,
            inverted,
            conds: Vec::new(),
            fields: Vec::new(),
            reason: None,
        };
        Self { done, cur }
    }

    /// Adds a condition; a repeat call ANDs with the earlier ones.
    pub fn when<R: Resource>(mut self, c: Cond<R>) -> Self {
        self.cur.conds.push((c.into_condition(), R::schema()));
        self
    }

    /// Restricts the group to these fields; a repeat call unions the fields.
    pub fn fields<R: Resource>(mut self, fs: impl IntoIterator<Item = FieldRef<R>>) -> Self {
        self.cur
            .fields
            .push((fs.into_iter().map(FieldRef::idx).collect(), R::schema()));
        self
    }

    /// Sets the denial reason; the last call wins.
    pub fn because(mut self, r: impl Into<Cow<'static, str>>) -> Self {
        self.cur.reason = Some(r.into());
        self
    }

    fn finish(mut self) -> Vec<Entry<A, S>> {
        self.done.push(Entry::Group(self.cur));
        self.done
    }

    /// Finishes this group, then adds rules bound from stored templates
    /// (see [`AbilityBuilder::extend`]).
    pub fn extend(self, bound: Bound<A, S>) -> AbilityBuilder<A, S> {
        AbilityBuilder {
            entries: self.finish(),
        }
        .extend(bound)
    }

    /// Finishes this group and starts a `can` group.
    pub fn can(
        self,
        actions: impl IntoActions<A>,
        subjects: impl IntoSubjects<S>,
    ) -> GroupBuilder<A, S> {
        GroupBuilder::start(
            self.finish(),
            false,
            actions.into_vec(),
            subjects.into_vec(),
        )
    }

    /// Finishes this group and starts a `cannot` group.
    pub fn cannot(
        self,
        actions: impl IntoActions<A>,
        subjects: impl IntoSubjects<S>,
    ) -> GroupBuilder<A, S> {
        GroupBuilder::start(self.finish(), true, actions.into_vec(), subjects.into_vec())
    }

    /// Validates, folds, expands, and indexes every group.
    pub fn build(self) -> Result<Ability<A, S>, BuildError> {
        build(self.finish())
    }
}

/// Deepest allowed nesting of `And`/`Or`/`Not`/`Rel` in a rule condition
/// (see [`BuildError::TooDeep`]).
const MAX_DEPTH: usize = 64;

/// Most `can`/`cannot` switches allowed among the rules of one cell (see
/// [`BuildError::TooManyAlternations`]).
const MAX_ALTERNATIONS: usize = 256;

/// The nesting depth of `c`: every `And`, `Or`, `Not` and `Rel` is one level,
/// leaves are at depth 0. Iterative, so any depth is measured safely.
fn depth(c: &Condition) -> usize {
    let mut max = 0;
    let mut stack = vec![(c, 0)];
    while let Some((c, d)) = stack.pop() {
        let d = match c {
            Condition::And(cs) | Condition::Or(cs) => {
                stack.extend(cs.iter().map(|c| (c, d + 1)));
                d + 1
            }
            Condition::Not(c) => {
                stack.push((c, d + 1));
                d + 1
            }
            Condition::Rel { cond, .. } => {
                stack.extend(cond.as_deref().map(|c| (c, d + 1)));
                d + 1
            }
            Condition::Cmp { .. }
            | Condition::In { .. }
            | Condition::NotIn { .. }
            | Condition::Str { .. }
            | Condition::IsNull(_)
            | Condition::IsNotNull(_) => d,
        };
        max = max.max(d);
    }
    max
}

/// Rejects a condition nested deeper than [`MAX_DEPTH`]. Runs before any
/// recursive pass over the condition.
fn check_depth(c: &Condition, subject: &'static str) -> Result<(), BuildError> {
    match depth(c) {
        d if d > MAX_DEPTH => Err(BuildError::TooDeep { subject, depth: d }),
        _ => Ok(()),
    }
}

/// Rejects non-finite float operands anywhere in `c`.
fn check_finite(c: &Condition) -> Result<(), BuildError> {
    let bad = |v: &crate::Value| {
        if v.is_finite() {
            Ok(())
        } else {
            Err(BuildError::InvalidValue {
                reason: format!("non-finite float operand {v:?}"),
            })
        }
    };
    match c {
        Condition::Cmp { value, .. } => bad(value),
        Condition::In { values, .. } | Condition::NotIn { values, .. } => {
            values.iter().try_for_each(bad)
        }
        Condition::And(cs) | Condition::Or(cs) => cs.iter().try_for_each(check_finite),
        Condition::Not(c) => check_finite(c),
        Condition::Rel { cond, .. } => cond.as_deref().map_or(Ok(()), check_finite),
        Condition::Str { .. } | Condition::IsNull(_) | Condition::IsNotNull(_) => Ok(()),
    }
}

/// Validates a group, then returns its folded condition and field mask.
/// `None` for the condition means unconditional; `Ok(None)` overall means the
/// group's rules are dropped.
#[allow(clippy::type_complexity)]
fn resolve<A: Action, S: Subject>(
    g: &mut Group<A, S>,
) -> Result<Option<(Option<Condition>, Option<FieldMask>)>, BuildError> {
    if g.actions.is_empty() {
        return Err(BuildError::Empty { what: "actions" });
    }
    if g.subjects.is_empty() {
        return Err(BuildError::Empty { what: "subjects" });
    }
    if g.fields.iter().any(|(f, _)| f.is_empty()) {
        return Err(BuildError::Empty { what: "fields" });
    }
    let first = g.subjects[0];

    if !g.fields.is_empty() {
        for (_, fs) in &g.fields {
            for s in &g.subjects {
                if !s.schema().is_some_and(|ss| std::ptr::eq(ss, *fs)) {
                    return Err(BuildError::ForeignField {
                        subject: s.name(),
                        field_schema: fs.name(),
                    });
                }
            }
        }
    }
    let mask = (!g.fields.is_empty()).then(|| {
        let mut m = FieldMask::default();
        g.fields
            .iter()
            .flat_map(|(f, _)| f)
            .for_each(|i| m.insert(*i));
        m
    });

    if g.conds.is_empty() {
        return Ok(Some((None, mask)));
    }
    let subject_schema = match (g.subjects.len(), first.schema()) {
        (1, Some(schema)) if S::ALL != Some(first) => schema,
        _ => {
            return Err(BuildError::ConditionsNotAllowed {
                subject: first.name(),
            });
        }
    };
    for (c, cs) in &g.conds {
        if !std::ptr::eq(*cs, subject_schema) {
            return Err(BuildError::SubjectMismatch {
                subject: first.name(),
                condition_schema: cs.name(),
            });
        }
        check_depth(c, first.name())?;
        check_finite(c)?;
    }
    let mut conds: Vec<Condition> = std::mem::take(&mut g.conds)
        .into_iter()
        .map(|(c, _)| c)
        .collect();
    let combined = if conds.len() == 1 {
        conds.remove(0)
    } else {
        Condition::And(conds)
    };
    Ok(match fold(combined, subject_schema) {
        Folded::False => None,
        Folded::True => Some((None, mask)),
        Folded::Cond(c) => Some((Some(c), mask)),
    })
}

/// Folds the condition of a rule added by `extend`; `None` drops the rule.
///
/// Its condition is already over its subject's schema (templates are
/// compiled against it), so only the checks that need no typed condition
/// remain.
fn fold_bound<A: Action, S: Subject>(mut r: Rule<A, S>) -> Result<Option<Rule<A, S>>, BuildError> {
    let Some(c) = r.condition_mut().take() else {
        return Ok(Some(r));
    };
    let subject = r.subject();
    let schema = match subject.schema() {
        Some(schema) if S::ALL != Some(subject) => schema,
        _ => {
            return Err(BuildError::ConditionsNotAllowed {
                subject: subject.name(),
            });
        }
    };
    check_depth(&c, subject.name())?;
    check_finite(&c)?;
    Ok(match fold(c, schema) {
        Folded::False => None,
        Folded::True => Some(r),
        Folded::Cond(c) => {
            *r.condition_mut() = Some(c);
            Some(r)
        }
    })
}

fn build<A: Action, S: Subject>(entries: Vec<Entry<A, S>>) -> Result<Ability<A, S>, BuildError> {
    let mut rules = Vec::new();
    for entry in entries {
        let mut g = match entry {
            Entry::Group(g) => g,
            Entry::Bound(bound) => {
                for r in bound {
                    rules.extend(fold_bound(r)?);
                }
                continue;
            }
        };
        let Some((cond, fields)) = resolve(&mut g)? else {
            continue;
        };
        for &a in &g.actions {
            for &s in &g.subjects {
                rules.push(Rule::new(
                    a,
                    s,
                    g.inverted,
                    cond.clone(),
                    fields,
                    g.reason.clone(),
                ));
            }
        }
    }
    let ability = Ability::from_rules(rules);
    if let Some((subject, action, count)) = ability.alternations_over(MAX_ALTERNATIONS) {
        return Err(BuildError::TooManyAlternations {
            subject: subject.name(),
            action: action.name(),
            count,
        });
    }
    Ok(ability)
}
