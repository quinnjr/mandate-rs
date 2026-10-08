//! Binding compiled templates to a request's context (spec §6.3–§6.4).
//!
//! Every slot is resolved and kind-checked up front, so a bad context value
//! fails the bind even in a rule that an unresolved optional placeholder
//! drops; the result does not depend on which rules happen to be dropped.

use std::borrow::Cow;
use std::collections::BTreeMap;

use serde::Serialize;
use serde_json::Value as Json;

use super::compile::{
    JsonScalar, KindError, ListOperand, Operand, Slot, SlotId, TCond, Templates, typed_value,
};
use crate::{
    Action, BindError, BindErrorKind, Condition, Kind, Rule, Subject, Unresolved,
    UnresolvedOutcome, Value,
};

/// Per-request values for template placeholders: named roots (such as
/// `user`), each serialized to JSON once.
///
/// A placeholder `${root.a.b}` reads key `a`, then key `b`, of root `root`.
#[derive(Clone, Debug, Default)]
pub struct Context {
    roots: BTreeMap<String, Json>,
}

impl Context {
    /// A context without roots; add them with [`with`](Self::with).
    pub fn new() -> Self {
        Self::default()
    }

    /// A context without roots, for templates without placeholders (such
    /// as serialized bound rules). The same as [`new`](Self::new).
    pub fn empty() -> Self {
        Self::default()
    }

    /// Adds root `root`, serializing `value` to JSON now. A later value for
    /// the same root replaces the earlier one.
    ///
    /// # Examples
    ///
    /// ```
    /// # use mandate::{Ability, Access, Action, Cond, Resource, Subject};
    /// # #[derive(Clone, Debug, Resource)]
    /// # struct Post { id: i64, author_id: i64, title: String, body: String, locked: bool }
    /// # #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Action)]
    /// # enum Act { Read, Update, #[action(manage)] Manage }
    /// # #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Subject)]
    /// # enum Sub { #[subject(resource = Post)] Post, Dashboard, #[subject(all)] All }
    /// # let mine = Post { id: 1, author_id: 7, title: "Hi".into(), body: "Text".into(), locked: false };
    /// # let theirs = Post { author_id: 8, ..mine.clone() };
    /// # use mandate::Context;
    /// #[derive(serde::Serialize)]
    /// struct User { id: i64, org_id: i64 }
    /// // each root is serialized once, here
    /// let ctx = Context::new().with("user", &User { id: 7, org_id: 3 }).unwrap();
    /// ```
    pub fn with(mut self, root: &str, value: &impl Serialize) -> Result<Self, serde_json::Error> {
        self.roots
            .insert(root.to_owned(), serde_json::to_value(value)?);
        Ok(self)
    }

    /// The value at `root` and then the object keys `path`, or `None` if it
    /// is missing (including a path through a non-object) or `null`.
    fn lookup(&self, root: &str, path: &[String]) -> Option<&Json> {
        let mut v = self.roots.get(root)?;
        for key in path {
            v = v.as_object()?.get(key)?;
        }
        (!v.is_null()).then_some(v)
    }
}

/// Templates bound to one request's context: concrete rules, and the
/// placeholders that could not be resolved.
///
/// Add the rules to an ability with
/// [`AbilityBuilder::extend`](crate::AbilityBuilder::extend). Bound rules are
/// a snapshot for one context; never store them back as templates.
#[derive(Clone, Debug)]
pub struct Bound<A, S> {
    rules: Vec<Rule<A, S>>,
    unresolved: Vec<Unresolved>,
}

impl<A, S> Bound<A, S> {
    /// The bound rules in template order, each template expanded
    /// action-major into one rule per (action, subject) pair. Conditions are
    /// not folded yet; `build()` folds them.
    pub fn rules(&self) -> &[Rule<A, S>] {
        &self.rules
    }

    /// The unresolved placeholders and what binding did instead, in template
    /// order. A rule dropped for an optional placeholder reports only its
    /// unresolved optional placeholders.
    pub fn unresolved(&self) -> &[Unresolved] {
        &self.unresolved
    }

    pub(crate) fn into_rules(self) -> Vec<Rule<A, S>> {
        self.rules
    }
}

/// A slot's value in the context.
enum Resolved {
    /// Missing or `null`.
    Unresolved,
    /// The value of a scalar slot.
    Scalar(Value),
    /// The values of a list slot.
    List(Vec<Value>),
}

impl<A: Action, S: Subject> Templates<A, S> {
    /// Substitutes the context's values for the placeholders (spec §6.3–§6.4).
    ///
    /// A placeholder is unresolved when its path is missing from `ctx` or is
    /// `null`. That never fails the bind:
    /// - a required placeholder's leaf becomes the constant that grants the
    ///   least access: `false` in a `can` rule and `true` in a `cannot` rule,
    ///   the other way round under an odd number of `$not`s and `$none`s;
    /// - an optional placeholder (`${…?}`) drops its whole rule.
    ///
    /// Each one is reported in [`Bound::unresolved`].
    ///
    /// Fails if a resolved value does not have the kind its field requires,
    /// checked as template literals are at compile time (an enum value must
    /// be a variant name; a list placeholder takes an array without `null`s).
    ///
    /// # Examples
    ///
    /// ```
    /// # use mandate::{Ability, Access, Action, Cond, Resource, Subject};
    /// # #[derive(Clone, Debug, Resource)]
    /// # struct Post { id: i64, author_id: i64, title: String, body: String, locked: bool }
    /// # #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Action)]
    /// # enum Act { Read, Update, #[action(manage)] Manage }
    /// # #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Subject)]
    /// # enum Sub { #[subject(resource = Post)] Post, Dashboard, #[subject(all)] All }
    /// # let mine = Post { id: 1, author_id: 7, title: "Hi".into(), body: "Text".into(), locked: false };
    /// # let theirs = Post { author_id: 8, ..mine.clone() };
    /// # use mandate::{Context, RuleTemplate, Templates};
    /// let stored = r#"[{"action": "update", "subject": "Post",
    ///                   "conditions": {"author_id": "${user.id}"}}]"#;
    /// let raw: Vec<RuleTemplate> = serde_json::from_str(stored).unwrap();
    /// let templates = Templates::<Act, Sub>::compile(&raw, &["user"]).unwrap();
    /// let ctx = Context::new().with("user", &serde_json::json!({"id": 7})).unwrap();
    /// let bound = templates.bind(&ctx).unwrap();
    /// assert!(bound.unresolved().is_empty());
    /// let ability = Ability::<Act, Sub>::builder().extend(bound).build().unwrap();
    /// assert!(ability.can(Act::Update, &mine));
    /// assert!(!ability.can(Act::Update, &theirs));
    /// ```
    pub fn bind(&self, ctx: &Context) -> Result<Bound<A, S>, BindError> {
        let values = self
            .slots
            .iter()
            .map(|slot| resolve(slot, ctx))
            .collect::<Result<Vec<_>, _>>()?;
        let mut rules = Vec::new();
        let mut unresolved = Vec::new();
        for rule in &self.rules {
            let mut dropped = false;
            for id in &rule.optional_slots {
                if let Resolved::Unresolved = values[id.0] {
                    unresolved.push(report(&self.slots[id.0], UnresolvedOutcome::RuleDropped));
                    dropped = true;
                }
            }
            if dropped {
                continue;
            }
            let mut binder = Binder {
                slots: &self.slots,
                values: &values,
                can: !rule.inverted,
                rule_index: rule.source_index,
                unresolved: &mut unresolved,
            };
            let cond = rule.cond.as_ref().map(|c| binder.cond(c)).transpose()?;
            let reason = rule.reason.clone().map(Cow::Owned);
            for &a in &rule.actions {
                for &s in &rule.subjects {
                    rules.push(Rule::new(
                        a,
                        s,
                        rule.inverted,
                        cond.clone(),
                        rule.fields,
                        reason.clone(),
                    ));
                }
            }
        }
        Ok(Bound { rules, unresolved })
    }
}

/// Looks up `slot` in `ctx` and checks its value's kind.
fn resolve(slot: &Slot, ctx: &Context) -> Result<Resolved, BindError> {
    let Some(v) = ctx.lookup(&slot.root, &slot.path) else {
        return Ok(Resolved::Unresolved);
    };
    let err = |path: String, kind| BindError {
        rule_index: slot.rule_index,
        path,
        kind,
    };
    if !slot.list {
        return context_value(slot.kind, v)
            .map(Resolved::Scalar)
            .map_err(|kind| err(slot.json_path.clone(), kind));
    }
    let Json::Array(items) = v else {
        return Err(err(slot.json_path.clone(), mismatch(slot.kind, v)));
    };
    let mut values = Vec::with_capacity(items.len());
    for (i, item) in items.iter().enumerate() {
        let value = if item.is_null() {
            Err(BindErrorKind::InvalidValue("null in list".to_owned()))
        } else {
            context_value(slot.kind, item)
        };
        values.push(value.map_err(|kind| err(format!("{}[{i}]", slot.json_path), kind))?);
    }
    Ok(Resolved::List(values))
}

/// Converts a context value to a value of `kind`, as compile converts a
/// template literal.
fn context_value(kind: Kind, v: &Json) -> Result<Value, BindErrorKind> {
    let scalar = match v {
        Json::Bool(b) => JsonScalar::Bool(*b),
        Json::Number(n) => JsonScalar::Number(n),
        Json::String(s) => JsonScalar::String(s),
        Json::Null | Json::Array(_) | Json::Object(_) => return Err(mismatch(kind, v)),
    };
    typed_value(kind, scalar).map_err(|e| match e {
        KindError::TypeMismatch => mismatch(kind, v),
        KindError::UnknownVariant(name) => BindErrorKind::UnknownVariant(name),
        KindError::InvalidValue(reason) => BindErrorKind::InvalidValue(reason),
    })
}

fn mismatch(expected: Kind, v: &Json) -> BindErrorKind {
    let found = match v {
        Json::Null => "null".to_owned(),
        Json::Bool(b) => b.to_string(),
        Json::Number(n) => n.to_string(),
        Json::String(s) => format!("{s:?}"),
        Json::Array(_) => "array".to_owned(),
        Json::Object(_) => "object".to_owned(),
    };
    BindErrorKind::TypeMismatch { expected, found }
}

/// The diagnostic for unresolved `slot`.
fn report(slot: &Slot, outcome: UnresolvedOutcome) -> Unresolved {
    let mut placeholder = slot.root.clone();
    for key in &slot.path {
        placeholder.push('.');
        placeholder.push_str(key);
    }
    Unresolved {
        rule_index: slot.rule_index,
        placeholder,
        outcome,
    }
}

/// Converts one rule's `TCond` to a `Condition`, given its slots' values.
struct Binder<'a> {
    slots: &'a [Slot],
    values: &'a [Resolved],
    /// Whether the rule is a `can` rule.
    can: bool,
    rule_index: usize,
    unresolved: &'a mut Vec<Unresolved>,
}

impl Binder<'_> {
    fn cond(&mut self, c: &TCond) -> Result<Condition, BindError> {
        if let Some(id) = leaf_slot(c) {
            if let Resolved::Unresolved = self.values[id.0] {
                return Ok(self.constant(id));
            }
        }
        Ok(match c {
            TCond::Cmp { field, op, value } => Condition::Cmp {
                field: *field,
                op: *op,
                value: self.scalar(value)?,
            },
            TCond::Str { field, op, value } => match self.scalar(value)? {
                Value::String(value) => Condition::Str {
                    field: *field,
                    op: *op,
                    value,
                },
                _ => return Err(self.misfit()),
            },
            TCond::In { field, values } => Condition::In {
                field: *field,
                values: self.list(values)?,
            },
            TCond::NotIn { field, values } => Condition::NotIn {
                field: *field,
                values: self.list(values)?,
            },
            TCond::IsNull(field) => Condition::IsNull(*field),
            TCond::IsNotNull(field) => Condition::IsNotNull(*field),
            TCond::And(cs) => Condition::And(self.all(cs)?),
            TCond::Or(cs) => Condition::Or(self.all(cs)?),
            TCond::Not(c) => Condition::Not(Box::new(self.cond(c)?)),
            TCond::Rel {
                relation,
                quant,
                cond,
            } => Condition::Rel {
                relation: *relation,
                quant: *quant,
                cond: Some(Box::new(self.cond(cond)?)),
            },
        })
    }

    fn all(&mut self, cs: &[TCond]) -> Result<Vec<Condition>, BindError> {
        cs.iter().map(|c| self.cond(c)).collect()
    }

    /// The constant replacing the leaf of unresolved required slot `id`:
    /// false if the rule is a `can` rule XOR the leaf is negative, else true
    /// (spec §6.4).
    fn constant(&mut self, id: SlotId) -> Condition {
        let slot = &self.slots[id.0];
        let (cond, outcome) = if self.can != slot.negative {
            (Condition::Or(vec![]), UnresolvedOutcome::LeafFalse)
        } else {
            (Condition::And(vec![]), UnresolvedOutcome::LeafTrue)
        };
        self.unresolved.push(report(slot, outcome));
        cond
    }

    fn scalar(&self, o: &Operand) -> Result<Value, BindError> {
        match o {
            Operand::Lit(v) => Ok(v.clone()),
            Operand::Slot(id) => match &self.values[id.0] {
                Resolved::Scalar(v) => Ok(v.clone()),
                _ => Err(self.misfit()),
            },
        }
    }

    fn list(&self, o: &ListOperand) -> Result<Vec<Value>, BindError> {
        match o {
            ListOperand::Lit(vs) => Ok(vs.clone()),
            ListOperand::Slot(id) => match &self.values[id.0] {
                Resolved::List(vs) => Ok(vs.clone()),
                _ => Err(self.misfit()),
            },
        }
    }

    /// An operand that does not fit its leaf: a text operator's non-string
    /// operand, or a resolved slot of the wrong shape. `compile` and
    /// `resolve` never produce one; failing the bind keeps such a bug from
    /// changing access.
    fn misfit(&self) -> BindError {
        BindError {
            rule_index: self.rule_index,
            path: "conditions".to_owned(),
            kind: BindErrorKind::InvalidValue("operand does not fit its operator".to_owned()),
        }
    }
}

/// The slot of a leaf whose operand is a placeholder.
fn leaf_slot(c: &TCond) -> Option<SlotId> {
    match c {
        TCond::Cmp {
            value: Operand::Slot(id),
            ..
        }
        | TCond::Str {
            value: Operand::Slot(id),
            ..
        }
        | TCond::In {
            values: ListOperand::Slot(id),
            ..
        }
        | TCond::NotIn {
            values: ListOperand::Slot(id),
            ..
        } => Some(*id),
        _ => None,
    }
}
