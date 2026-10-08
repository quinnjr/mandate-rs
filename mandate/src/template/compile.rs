//! Compiling rule templates against the subjects' schemas (spec §6.2–§6.5).
//!
//! # Compiled representation (consumed by `bind`)
//!
//! - `Templates` holds one `CompiledRule` per template, in source order
//!   (`source_index` is the template's index), and one list of `Slot`s for
//!   every placeholder of every rule; a `SlotId` indexes that list.
//! - A `TCond` mirrors `Condition` node for node, with the same field
//!   indices (relative to the schema in scope; inside `Rel`, to the relation
//!   target's schema). The differences:
//!   - leaf operands are `Operand`s (`Cmp`, `Str`) or `ListOperand`s (`In`,
//!     `NotIn`): either a literal already checked against the field's kind
//!     (`$$` escapes decoded, enum variants checked, UUID/date-time/date
//!     parsed; a `Str` literal is always `Value::String`) or a slot;
//!   - `Rel` always has a condition (`{}` compiles to `And([])`).
//! - Nothing is folded (`build()` folds): empty `$and`/`$or` stay
//!   `And([])`/`Or([])`, `$in: []` stays `In([])`, an object with several keys
//!   is an `And` of them in key order, and an object with one key is that
//!   key's node.
//! - A `Slot` records where its value comes from (`root` and the object keys
//!   `path` below it), the kind its value must have (for a list slot, the kind
//!   of each element), whether it is optional, its JSON path in the template
//!   (for errors and diagnostics), and `negative`: the polarity of the leaf
//!   containing it, flipped by every enclosing `$not` and `$none`.
//! - An unresolved required slot replaces its whole leaf (`Cmp`, `Str`, `In`
//!   or `NotIn`) with false (`Or([])`) when the rule is a `can` rule XOR the
//!   slot is negative, else with true (`And([])`) — spec §6.4. An unresolved
//!   optional slot (listed in `CompiledRule::optional_slots`) drops its rule.
//!
//! # Error choices not fixed by the spec
//!
//! - A condition on an opaque field is `Malformed`: opaque fields have no
//!   kind, so neither `OperatorNotAllowed` nor `TypeMismatch` fits.
//! - A `fields` entry on a rule without a single resource-bound subject is
//!   `UnknownField`, since no schema in scope defines it.
//! - Inside a to-one relation object (a target-scoped condition), an unknown
//!   `$`-key such as `$some` is `UnknownOperator`, as in any condition object;
//!   `UnknownKey` is for to-many relation objects.

use crate::condition::validate::{self, Misuse, Use};
use crate::{
    Action, CardinalityKind, CmpOp, FieldIdx, FieldKind, FieldMask, Kind, LoadError, LoadErrorKind,
    OneOrMany, Quant, RuleTemplate, Schema, StrOp, Subject, TemplateValue, Value,
};

/// Deepest allowed nesting of condition objects below `conditions` (which is
/// level 0). Each `$and`/`$or` element, `$not` operand and relation target or
/// quantifier operand is one level deeper than its parent.
const MAX_DEPTH: usize = 32;

/// Stored rule templates, validated against the subjects' schemas.
///
/// Compile once (at startup, or whenever the stored rules change); binding
/// to a request's context is then cheap.
#[derive(Clone, Debug)]
pub struct Templates<A, S> {
    pub(super) rules: Vec<CompiledRule<A, S>>,
    pub(super) slots: Vec<Slot>,
}

/// Index of a [`Slot`] in [`Templates::slots`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct SlotId(pub(super) usize);

/// One compiled template.
#[derive(Clone, Debug)]
pub(super) struct CompiledRule<A, S> {
    /// Index of the template in the list passed to `compile`.
    pub(super) source_index: usize,
    /// Resolved actions, in source order (never empty).
    pub(super) actions: Vec<A>,
    /// Resolved subjects, in source order (never empty).
    pub(super) subjects: Vec<S>,
    /// Whether this is a `cannot` rule.
    pub(super) inverted: bool,
    /// The condition over the single subject's schema; `None` if the template
    /// has no `conditions`.
    pub(super) cond: Option<TCond>,
    /// The field restriction over the single subject's schema.
    pub(super) fields: Option<FieldMask>,
    /// The template's reason.
    pub(super) reason: Option<String>,
    /// This rule's optional slots; if any is unresolved, the rule is dropped.
    pub(super) optional_slots: Vec<SlotId>,
}

/// A [`Condition`](crate::Condition) whose leaf operands may be placeholders.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum TCond {
    /// `Condition::Cmp`.
    Cmp {
        field: FieldIdx,
        op: CmpOp,
        value: Operand,
    },
    /// `Condition::In`.
    In {
        field: FieldIdx,
        values: ListOperand,
    },
    /// `Condition::NotIn`.
    NotIn {
        field: FieldIdx,
        values: ListOperand,
    },
    /// `Condition::Str`; a literal operand is a `Value::String`.
    Str {
        field: FieldIdx,
        op: StrOp,
        value: Operand,
    },
    /// `Condition::IsNull`.
    IsNull(FieldIdx),
    /// `Condition::IsNotNull`.
    IsNotNull(FieldIdx),
    /// `Condition::And`.
    And(Vec<TCond>),
    /// `Condition::Or`.
    Or(Vec<TCond>),
    /// `Condition::Not`.
    Not(Box<TCond>),
    /// `Condition::Rel`, always with a condition (relative to the target).
    Rel {
        relation: FieldIdx,
        quant: Quant,
        cond: Box<TCond>,
    },
}

/// The operand of `Cmp` or `Str`.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Operand {
    /// A literal of the field's kind.
    Lit(Value),
    /// A scalar placeholder.
    Slot(SlotId),
}

/// The operand of `In` or `NotIn`.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum ListOperand {
    /// Literals of the field's kind (possibly none).
    Lit(Vec<Value>),
    /// A whole-list placeholder.
    Slot(SlotId),
}

/// A placeholder occurrence (spec §6.3–§6.4).
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Slot {
    /// Index of the template containing the placeholder.
    pub(super) rule_index: usize,
    /// The context root (first segment).
    pub(super) root: String,
    /// Object keys below the root (the remaining segments; may be empty).
    pub(super) path: Vec<String>,
    /// Written `${…?}`: if unresolved, the rule is dropped.
    pub(super) optional: bool,
    /// The kind the value must have (of each element, for a list slot).
    pub(super) kind: Kind,
    /// A whole-list placeholder of `$in`/`$nin` (the value must be an array).
    pub(super) list: bool,
    /// The leaf's polarity: flipped by each enclosing `$not` and `$none`.
    pub(super) negative: bool,
    /// Where the placeholder is in the template, e.g. `conditions.org.id`.
    pub(super) json_path: String,
}

impl<A: Action, S: Subject> Templates<A, S> {
    /// Resolves action, subject, and field names and validates every
    /// condition against the subject's schema (spec §6.2–§6.3).
    ///
    /// `roots` lists the context roots placeholders may name (`${root.…}`).
    /// Fails on the first invalid template; nothing is silently ignored.
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
    /// // a typo is an error, never silently ignored
    /// let typo: Vec<RuleTemplate> = serde_json::from_str(
    ///     r#"[{"action": "update", "subject": "Post", "conditions": {"autor_id": 1}}]"#,
    /// ).unwrap();
    /// assert!(Templates::<Act, Sub>::compile(&typo, &["user"]).is_err());
    /// ```
    pub fn compile(raw: &[RuleTemplate], roots: &[&str]) -> Result<Self, LoadError> {
        let mut slots = Vec::new();
        let mut rules = Vec::with_capacity(raw.len());
        for (rule_index, template) in raw.iter().enumerate() {
            rules.push(compile_rule(rule_index, template, roots, &mut slots)?);
        }
        Ok(Self { rules, slots })
    }
}

fn compile_rule<A: Action, S: Subject>(
    rule_index: usize,
    t: &RuleTemplate,
    roots: &[&str],
    slots: &mut Vec<Slot>,
) -> Result<CompiledRule<A, S>, LoadError> {
    let err = |path: &str, kind| LoadError {
        rule_index,
        path: path.to_owned(),
        kind,
    };
    let actions = names(
        rule_index,
        &t.action,
        "action",
        A::from_name,
        LoadErrorKind::UnknownAction,
    )?;
    let subjects = names(
        rule_index,
        &t.subject,
        "subject",
        S::from_name,
        LoadErrorKind::UnknownSubject,
    )?;
    // Conditions and field restrictions need a single resource-bound subject.
    let schema = match subjects.as_slice() {
        [s] if S::ALL != Some(*s) => s.schema(),
        _ => None,
    };

    let mut c = Compiler {
        rule_index,
        roots,
        slots,
        optional_slots: Vec::new(),
    };
    let cond = match &t.conditions {
        None => None,
        Some(v) => {
            let schema =
                schema.ok_or_else(|| err("conditions", LoadErrorKind::ConditionsNotAllowed))?;
            Some(c.object(v, schema, "conditions", 0, false)?)
        }
    };

    let fields = match &t.fields {
        None => None,
        Some(fs) if fs.is_empty() => return Err(err("fields", LoadErrorKind::Empty("fields"))),
        Some(fs) => {
            let mut mask = FieldMask::default();
            for (i, name) in fs.iter().enumerate() {
                let idx = schema.and_then(|s| s.index_of(name)).ok_or_else(|| {
                    err(
                        &format!("fields[{i}]"),
                        LoadErrorKind::UnknownField(name.clone()),
                    )
                })?;
                mask.insert(idx);
            }
            Some(mask)
        }
    };

    Ok(CompiledRule {
        source_index: rule_index,
        actions,
        subjects,
        inverted: t.inverted,
        cond,
        fields,
        reason: t.reason.clone(),
        optional_slots: c.optional_slots,
    })
}

/// Resolves an `action` or `subject` value (`key`) to a non-empty list.
fn names<T>(
    rule_index: usize,
    v: &OneOrMany,
    key: &'static str,
    lookup: impl Fn(&str) -> Option<T>,
    unknown: impl Fn(String) -> LoadErrorKind,
) -> Result<Vec<T>, LoadError> {
    let resolve = |name: &String, path: String| {
        lookup(name).ok_or_else(|| LoadError {
            rule_index,
            path,
            kind: unknown(name.clone()),
        })
    };
    match v {
        OneOrMany::One(name) => Ok(vec![resolve(name, key.to_owned())?]),
        OneOrMany::Many(list) if list.is_empty() => Err(LoadError {
            rule_index,
            path: key.to_owned(),
            kind: LoadErrorKind::Empty(key),
        }),
        OneOrMany::Many(list) => list
            .iter()
            .enumerate()
            .map(|(i, name)| resolve(name, format!("{key}[{i}]")))
            .collect(),
    }
}

/// Compiles the conditions of one template.
struct Compiler<'a> {
    rule_index: usize,
    roots: &'a [&'a str],
    /// Slots of every template compiled so far.
    slots: &'a mut Vec<Slot>,
    /// This template's optional slots.
    optional_slots: Vec<SlotId>,
}

/// A JSON string in operand position, interpreted per spec §6.3.
enum Text<'s> {
    /// A literal string, `$$` escape already decoded.
    Literal(&'s str),
    /// `${root.a.b}` or `${root.a.b?}`.
    Placeholder(Placeholder<'s>),
}

struct Placeholder<'s> {
    root: &'s str,
    path: Vec<&'s str>,
    optional: bool,
}

/// A parsed scalar operand.
enum Parsed<'s> {
    Lit(Value),
    Placeholder(Placeholder<'s>),
}

impl Compiler<'_> {
    fn err(&self, path: &str, kind: LoadErrorKind) -> LoadError {
        LoadError {
            rule_index: self.rule_index,
            path: path.to_owned(),
            kind,
        }
    }

    fn malformed(&self, path: &str, reason: impl Into<String>) -> LoadError {
        self.err(path, LoadErrorKind::Malformed(reason.into()))
    }

    /// The error for a field use that [`validate::check`] rejects; `op` is
    /// the template operator, if any.
    fn misuse(&self, path: &str, op: &str, m: Misuse) -> LoadError {
        match m {
            Misuse::Operator(_, kind) => self.err(
                path,
                LoadErrorKind::OperatorNotAllowed {
                    op: op.to_owned(),
                    kind,
                },
            ),
            Misuse::NotNullable => self.err(path, LoadErrorKind::NullNotAllowed),
            Misuse::Opaque | Misuse::Relation | Misuse::NotRelation | Misuse::Quantifier(..) => {
                self.malformed(path, m.to_string())
            }
        }
    }

    /// A condition object over `schema`: field keys and `$and`/`$or`/`$not`,
    /// AND-ed in key order. `negative` is the polarity of its position.
    fn object(
        &mut self,
        v: &TemplateValue,
        schema: &'static Schema,
        path: &str,
        depth: usize,
        negative: bool,
    ) -> Result<TCond, LoadError> {
        if depth > MAX_DEPTH {
            return Err(self.malformed(path, "nesting too deep"));
        }
        let TemplateValue::Object(entries) = v else {
            return Err(self.malformed(
                path,
                format!("expected a condition object, found {}", describe(v)),
            ));
        };
        let mut out = Vec::with_capacity(entries.len());
        for (key, value) in entries {
            let path = format!("{path}.{key}");
            out.push(match key.as_str() {
                "$and" | "$or" => {
                    let TemplateValue::Array(items) = value else {
                        return Err(self
                            .malformed(&path, format!("`{key}` expects an array of conditions")));
                    };
                    let mut children = Vec::with_capacity(items.len());
                    for (i, item) in items.iter().enumerate() {
                        let path = format!("{path}[{i}]");
                        children.push(self.object(item, schema, &path, depth + 1, negative)?);
                    }
                    if key == "$and" {
                        TCond::And(children)
                    } else {
                        TCond::Or(children)
                    }
                }
                "$not" => TCond::Not(Box::new(self.object(
                    value,
                    schema,
                    &path,
                    depth + 1,
                    !negative,
                )?)),
                op if op.starts_with('$') => {
                    return Err(self.err(&path, LoadErrorKind::UnknownOperator(op.to_owned())));
                }
                name => self.field(name, value, schema, &path, depth, negative)?,
            });
        }
        Ok(and(out))
    }

    /// The value of field key `name` in a condition object over `schema`.
    fn field(
        &mut self,
        name: &str,
        v: &TemplateValue,
        schema: &'static Schema,
        path: &str,
        depth: usize,
        negative: bool,
    ) -> Result<TCond, LoadError> {
        let Some((idx, def)) = schema
            .index_of(name)
            .and_then(|idx| schema.field(idx).map(|def| (idx, def)))
        else {
            return Err(self.err(path, LoadErrorKind::UnknownField(name.to_owned())));
        };
        match def.kind() {
            FieldKind::Scalar { kind, nullable } => {
                self.scalar(idx, kind, nullable, v, path, negative)
            }
            FieldKind::Opaque => Err(self.malformed(
                path,
                format!("`{name}` is an opaque field and cannot be used in conditions"),
            )),
            FieldKind::Relation {
                target,
                cardinality,
                nullable,
            } => self.relation(idx, target, cardinality, nullable, v, path, depth, negative),
        }
    }

    /// A scalar field's value: a bare operand (`$eq`) or an operator object.
    fn scalar(
        &mut self,
        field: FieldIdx,
        kind: Kind,
        nullable: bool,
        v: &TemplateValue,
        path: &str,
        negative: bool,
    ) -> Result<TCond, LoadError> {
        let TemplateValue::Object(ops) = v else {
            return self.op("$eq", field, kind, nullable, v, path, negative);
        };
        if ops.is_empty() {
            return Err(self.malformed(path, "empty operator object"));
        }
        let mut out = Vec::with_capacity(ops.len());
        for (op, operand) in ops {
            let path = format!("{path}.{op}");
            out.push(self.op(op, field, kind, nullable, operand, &path, negative)?);
        }
        Ok(and(out))
    }

    /// One scalar operator applied to `v`.
    #[allow(clippy::too_many_arguments)]
    fn op(
        &mut self,
        op: &str,
        field: FieldIdx,
        kind: Kind,
        nullable: bool,
        v: &TemplateValue,
        path: &str,
        negative: bool,
    ) -> Result<TCond, LoadError> {
        let def = FieldKind::Scalar { kind, nullable };
        // The operator's use of the field, checked by the rules `build()`
        // applies to code-built conditions.
        let check = |u| validate::check(def, u).map_err(|m| self.misuse(path, op, m));
        match op {
            "$eq" | "$ne" if matches!(v, TemplateValue::Null) => {
                self.null_test(field, def, op == "$eq", path)
            }
            "$eq" | "$ne" | "$lt" | "$lte" | "$gt" | "$gte" => {
                let cmp = match op {
                    "$eq" => CmpOp::Eq,
                    "$ne" => CmpOp::Ne,
                    "$lt" => CmpOp::Lt,
                    "$lte" => CmpOp::Lte,
                    "$gt" => CmpOp::Gt,
                    _ => CmpOp::Gte,
                };
                check(Use::Cmp(cmp))?;
                Ok(TCond::Cmp {
                    field,
                    op: cmp,
                    value: self.operand(kind, v, path, negative)?,
                })
            }
            "$contains" | "$startsWith" | "$endsWith" => {
                let text_op = match op {
                    "$contains" => StrOp::Contains,
                    "$startsWith" => StrOp::StartsWith,
                    _ => StrOp::EndsWith,
                };
                check(Use::Str(text_op))?;
                Ok(TCond::Str {
                    field,
                    op: text_op,
                    value: self.operand(kind, v, path, negative)?,
                })
            }
            "$in" => {
                check(Use::List)?;
                Ok(TCond::In {
                    field,
                    values: self.list(kind, v, path, negative)?,
                })
            }
            "$nin" => {
                check(Use::List)?;
                Ok(TCond::NotIn {
                    field,
                    values: self.list(kind, v, path, negative)?,
                })
            }
            "$isNull" => {
                let null = self.flag(v, path)?;
                self.null_test(field, def, null, path)
            }
            _ => Err(self.err(path, LoadErrorKind::UnknownOperator(op.to_owned()))),
        }
    }

    /// A to-one or to-many relation field's value.
    #[allow(clippy::too_many_arguments)]
    fn relation(
        &mut self,
        relation: FieldIdx,
        target: fn() -> &'static Schema,
        cardinality: CardinalityKind,
        nullable: bool,
        v: &TemplateValue,
        path: &str,
        depth: usize,
        negative: bool,
    ) -> Result<TCond, LoadError> {
        let def = FieldKind::Relation {
            target,
            cardinality,
            nullable,
        };
        let target = target();
        let to_one = cardinality == CardinalityKind::ToOne;
        let entries = match v {
            TemplateValue::Object(entries) => entries,
            TemplateValue::Null => return self.null_test(relation, def, true, path),
            _ => {
                let expected = if to_one {
                    "null or an object"
                } else {
                    "an object"
                };
                return Err(self.malformed(
                    path,
                    format!("a relation expects {expected}, found {}", describe(v)),
                ));
            }
        };
        if to_one {
            if !entries.iter().any(|(k, _)| k == "$isNull" || k == "$none") {
                // A condition scoped to the target.
                let cond = self.object(v, target, path, depth + 1, negative)?;
                return self.rel(relation, def, Quant::One, cond, path);
            }
            let [(key, value)] = entries.as_slice() else {
                return Err(self.err(path, LoadErrorKind::MixedRelationObject));
            };
            let path = format!("{path}.{key}");
            if key == "$isNull" {
                let null = self.flag(value, &path)?;
                return self.null_test(relation, def, null, &path);
            }
            let cond = self.object(value, target, &path, depth + 1, !negative)?;
            return self.rel(relation, def, Quant::None, cond, &path);
        }
        if let Some((key, _)) = entries
            .iter()
            .find(|(k, _)| !matches!(k.as_str(), "$some" | "$every" | "$none"))
        {
            return Err(self.err(
                &format!("{path}.{key}"),
                LoadErrorKind::UnknownKey(key.clone()),
            ));
        }
        let [(key, value)] = entries.as_slice() else {
            return Err(if entries.is_empty() {
                self.malformed(
                    path,
                    "a to-many relation needs `$some`, `$every` or `$none`",
                )
            } else {
                self.err(path, LoadErrorKind::MixedRelationObject)
            });
        };
        let (quant, negative) = match key.as_str() {
            "$some" => (Quant::Some, negative),
            "$every" => (Quant::Every, negative),
            _ => (Quant::None, !negative),
        };
        let path = format!("{path}.{key}");
        let cond = self.object(value, target, &path, depth + 1, negative)?;
        self.rel(relation, def, quant, cond, &path)
    }

    /// `Rel{quant, cond}` on `relation`. The quantifier always fits the
    /// cardinality here (the keys allowed depend on it); it is checked by
    /// the rules `build()` applies to code-built conditions all the same.
    fn rel(
        &self,
        relation: FieldIdx,
        def: FieldKind,
        quant: Quant,
        cond: TCond,
        path: &str,
    ) -> Result<TCond, LoadError> {
        validate::check(def, Use::Rel(quant)).map_err(|m| self.misuse(path, "", m))?;
        Ok(TCond::Rel {
            relation,
            quant,
            cond: Box::new(cond),
        })
    }

    /// `IsNull` (`null`) or `IsNotNull` on a nullable scalar or to-one relation.
    fn null_test(
        &self,
        field: FieldIdx,
        def: FieldKind,
        null: bool,
        path: &str,
    ) -> Result<TCond, LoadError> {
        validate::check(def, Use::NullTest).map_err(|m| self.misuse(path, "", m))?;
        Ok(if null {
            TCond::IsNull(field)
        } else {
            TCond::IsNotNull(field)
        })
    }

    /// The literal boolean operand of `$isNull`.
    fn flag(&self, v: &TemplateValue, path: &str) -> Result<bool, LoadError> {
        match v {
            TemplateValue::Bool(b) => Ok(*b),
            TemplateValue::Null => Err(self.err(path, LoadErrorKind::NullNotAllowed)),
            _ => Err(self.err(
                path,
                LoadErrorKind::TypeMismatch {
                    expected: Kind::Bool,
                    found: describe(v),
                },
            )),
        }
    }

    /// A scalar operand: a literal of `kind` or a placeholder.
    fn operand(
        &mut self,
        kind: Kind,
        v: &TemplateValue,
        path: &str,
        negative: bool,
    ) -> Result<Operand, LoadError> {
        Ok(match self.parse(kind, v, path)? {
            Parsed::Lit(value) => Operand::Lit(value),
            Parsed::Placeholder(p) => Operand::Slot(self.slot(p, kind, false, negative, path)?),
        })
    }

    /// The operand of `$in`/`$nin`: an array of literals, or a whole-list
    /// placeholder.
    fn list(
        &mut self,
        kind: Kind,
        v: &TemplateValue,
        path: &str,
        negative: bool,
    ) -> Result<ListOperand, LoadError> {
        match v {
            TemplateValue::Array(items) => {
                let mut values = Vec::with_capacity(items.len());
                for (i, item) in items.iter().enumerate() {
                    let path = format!("{path}[{i}]");
                    match self.parse(kind, item, &path)? {
                        Parsed::Lit(value) => values.push(value),
                        Parsed::Placeholder(_) => {
                            return Err(self.malformed(
                                &path,
                                "a list element cannot be a placeholder; use a placeholder for the whole list",
                            ));
                        }
                    }
                }
                Ok(ListOperand::Lit(values))
            }
            TemplateValue::Null => Err(self.err(path, LoadErrorKind::NullNotAllowed)),
            TemplateValue::String(s) => match classify(s) {
                Ok(Text::Placeholder(p)) => {
                    Ok(ListOperand::Slot(self.slot(p, kind, true, negative, path)?))
                }
                Ok(Text::Literal(_)) => Err(self.malformed(
                    path,
                    format!("expected an array or a placeholder, found {}", describe(v)),
                )),
                Err(kind) => Err(self.err(path, kind)),
            },
            _ => Err(self.malformed(
                path,
                format!("expected an array or a placeholder, found {}", describe(v)),
            )),
        }
    }

    /// Parses a scalar operand of `kind`; `null` is never an operand.
    fn parse<'v>(
        &self,
        kind: Kind,
        v: &'v TemplateValue,
        path: &str,
    ) -> Result<Parsed<'v>, LoadError> {
        let mismatch = || {
            self.err(
                path,
                LoadErrorKind::TypeMismatch {
                    expected: kind,
                    found: describe(v),
                },
            )
        };
        let scalar = match v {
            TemplateValue::Null => return Err(self.err(path, LoadErrorKind::NullNotAllowed)),
            TemplateValue::Bool(b) => JsonScalar::Bool(*b),
            TemplateValue::Number(n) => JsonScalar::Number(n),
            TemplateValue::String(s) => match classify(s).map_err(|k| self.err(path, k))? {
                Text::Placeholder(p) => return Ok(Parsed::Placeholder(p)),
                Text::Literal(text) => JsonScalar::String(text),
            },
            TemplateValue::Array(_) | TemplateValue::Object(_) => return Err(mismatch()),
        };
        match typed_value(kind, scalar) {
            Ok(value) => Ok(Parsed::Lit(value)),
            Err(KindError::TypeMismatch) => Err(mismatch()),
            Err(KindError::UnknownVariant(name)) => {
                Err(self.err(path, LoadErrorKind::UnknownVariant(name)))
            }
            Err(KindError::InvalidValue(reason)) => {
                Err(self.err(path, LoadErrorKind::InvalidValue(reason)))
            }
        }
    }

    /// Records a placeholder slot after checking its root.
    fn slot(
        &mut self,
        p: Placeholder<'_>,
        kind: Kind,
        list: bool,
        negative: bool,
        path: &str,
    ) -> Result<SlotId, LoadError> {
        if !self.roots.contains(&p.root) {
            return Err(self.err(path, LoadErrorKind::UnknownRoot(p.root.to_owned())));
        }
        let id = SlotId(self.slots.len());
        self.slots.push(Slot {
            rule_index: self.rule_index,
            root: p.root.to_owned(),
            path: p.path.into_iter().map(str::to_owned).collect(),
            optional: p.optional,
            kind,
            list,
            negative,
            json_path: path.to_owned(),
        });
        if p.optional {
            self.optional_slots.push(id);
        }
        Ok(id)
    }
}

/// Several AND-ed conditions; a single one stands for itself.
fn and(children: Vec<TCond>) -> TCond {
    match <[TCond; 1]>::try_from(children) {
        Ok([only]) => only,
        Err(children) => TCond::And(children),
    }
}

/// Interprets a string operand by its prefix (spec §6.3).
fn classify(s: &str) -> Result<Text<'_>, LoadErrorKind> {
    let Some(rest) = s.strip_prefix('$') else {
        return Ok(Text::Literal(s));
    };
    if rest.starts_with('$') {
        return Ok(Text::Literal(rest));
    }
    let Some(body) = rest.strip_prefix('{') else {
        return Ok(Text::Literal(s));
    };
    let malformed = || LoadErrorKind::Malformed(format!("invalid placeholder `{s}`"));
    let body = body.strip_suffix('}').ok_or_else(malformed)?;
    let (body, optional) = match body.strip_suffix('?') {
        Some(body) => (body, true),
        None => (body, false),
    };
    if body
        .split('.')
        .any(|seg| seg.is_empty() || seg.contains(['{', '}', '?']))
    {
        return Err(malformed());
    }
    let mut segments = body.split('.');
    let root = segments.next().ok_or_else(malformed)?;
    Ok(Text::Placeholder(Placeholder {
        root,
        path: segments.collect(),
        optional,
    }))
}

/// A JSON scalar operand: a template literal (`$$` escape decoded) or a
/// context value (taken verbatim).
pub(super) enum JsonScalar<'v> {
    /// A boolean.
    Bool(bool),
    /// A number.
    Number(&'v serde_json::Number),
    /// A string.
    String(&'v str),
}

/// Why a [`JsonScalar`] is not a value of some kind.
pub(super) enum KindError {
    /// The JSON type does not fit the kind (or an integer kind's number is
    /// not an `i64`).
    TypeMismatch,
    /// A string that is not one of the enum's variants.
    UnknownVariant(String),
    /// A string that does not parse as the kind; the reason.
    InvalidValue(String),
}

/// Converts a JSON scalar to a value of `kind`. Template literals (at
/// compile) and context values (at bind) follow these same rules: `Int`
/// takes numbers that are `i64`s, `Float` any number, enums only their
/// variant names, and `Uuid`/`DateTime`/`Date` parse their string forms.
pub(super) fn typed_value(kind: Kind, v: JsonScalar<'_>) -> Result<Value, KindError> {
    match (kind, v) {
        (Kind::Bool, JsonScalar::Bool(b)) => Ok(Value::Bool(b)),
        (Kind::Int, JsonScalar::Number(n)) => {
            n.as_i64().map(Value::Int).ok_or(KindError::TypeMismatch)
        }
        (Kind::Float, JsonScalar::Number(n)) => {
            n.as_f64().map(Value::Float).ok_or(KindError::TypeMismatch)
        }
        (Kind::String, JsonScalar::String(s)) => Ok(Value::String(s.to_owned())),
        (Kind::Enum(variants), JsonScalar::String(s)) if variants.contains(&s) => {
            Ok(Value::String(s.to_owned()))
        }
        (Kind::Enum(_), JsonScalar::String(s)) => Err(KindError::UnknownVariant(s.to_owned())),
        (Kind::Uuid, JsonScalar::String(s)) => parse_uuid(s).map_err(KindError::InvalidValue),
        (Kind::DateTime, JsonScalar::String(s)) => {
            parse_datetime(s).map_err(KindError::InvalidValue)
        }
        (Kind::Date, JsonScalar::String(s)) => parse_date(s).map_err(KindError::InvalidValue),
        _ => Err(KindError::TypeMismatch),
    }
}

/// A short description of a JSON value for `TypeMismatch::found`.
fn describe(v: &TemplateValue) -> String {
    match v {
        TemplateValue::Null => "null".to_owned(),
        TemplateValue::Bool(b) => b.to_string(),
        TemplateValue::Number(n) => n.to_string(),
        TemplateValue::String(s) => format!("{s:?}"),
        TemplateValue::Array(_) => "array".to_owned(),
        TemplateValue::Object(_) => "object".to_owned(),
    }
}

/// Parses a hyphenated UUID (the only 36-character form).
#[cfg(feature = "uuid")]
fn parse_uuid(s: &str) -> Result<Value, String> {
    match uuid::Uuid::parse_str(s) {
        Ok(u) if s.len() == 36 => Ok(Value::Uuid(u)),
        _ => Err(format!("{s:?} is not a hyphenated UUID")),
    }
}

/// Parses an RFC 3339 timestamp.
#[cfg(feature = "chrono")]
fn parse_datetime(s: &str) -> Result<Value, String> {
    chrono::DateTime::parse_from_rfc3339(s)
        .map(|dt| Value::DateTime(dt.with_timezone(&chrono::Utc)))
        .map_err(|e| format!("{s:?} is not an RFC 3339 date-time: {e}"))
}

/// Parses a `YYYY-MM-DD` date.
#[cfg(feature = "chrono")]
fn parse_date(s: &str) -> Result<Value, String> {
    chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d")
        .map(Value::Date)
        .map_err(|e| format!("{s:?} is not a YYYY-MM-DD date: {e}"))
}

// Without the feature no derived schema has fields of these kinds, but a
// hand-written one may.
#[cfg(not(feature = "uuid"))]
fn parse_uuid(_: &str) -> Result<Value, String> {
    Err("UUID values need the `uuid` feature".to_owned())
}

#[cfg(not(feature = "chrono"))]
fn parse_datetime(_: &str) -> Result<Value, String> {
    Err("date-time values need the `chrono` feature".to_owned())
}

#[cfg(not(feature = "chrono"))]
fn parse_date(_: &str) -> Result<Value, String> {
    Err("date values need the `chrono` feature".to_owned())
}

#[cfg(all(test, feature = "derive", feature = "chrono", feature = "uuid"))]
mod tests {
    use super::*;
    use crate::test_fixture::{Action as Act, Subject as Subj};

    fn compile(json: &str) -> Templates<Act, Subj> {
        let raw: Vec<RuleTemplate> = serde_json::from_str(json).unwrap();
        Templates::compile(&raw, &["user"]).unwrap()
    }

    /// The condition of a single `can read Post` rule.
    fn cond(c: &str) -> TCond {
        let t = compile(&format!(
            r#"[{{"action":"read","subject":"Post","conditions":{c}}}]"#
        ));
        t.rules[0].cond.clone().unwrap()
    }

    fn cmp(field: u16, op: CmpOp, value: Value) -> TCond {
        TCond::Cmp {
            field: FieldIdx(field),
            op,
            value: Operand::Lit(value),
        }
    }

    fn slot_cmp(field: u16, slot: usize) -> TCond {
        TCond::Cmp {
            field: FieldIdx(field),
            op: CmpOp::Eq,
            value: Operand::Slot(SlotId(slot)),
        }
    }

    fn rel(relation: u16, quant: Quant, cond: TCond) -> TCond {
        TCond::Rel {
            relation: FieldIdx(relation),
            quant,
            cond: Box::new(cond),
        }
    }

    fn s(v: &str) -> Value {
        Value::String(v.into())
    }

    #[test]
    fn spec_example_representation() {
        let t = compile(
            r#"[{
              "action": "update",
              "subject": "Post",
              "conditions": {
                "author_id": "${user.id}",
                "status": { "$ne": "archived" },
                "published_at": null,
                "org": { "id": "${user.org_id}" },
                "reviewer": { "$isNull": false },
                "tags": { "$some": { "name": "rust" } },
                "$or": [ { "status": "published" }, { "author_id": "${user.id}" } ]
              },
              "fields": ["title", "body"],
              "reason": "Authors edit their own posts"
            }]"#,
        );
        let r = &t.rules[0];
        assert_eq!(r.source_index, 0);
        assert_eq!(r.actions, [Act::Update]);
        assert_eq!(r.subjects, [Subj::Post]);
        assert!(!r.inverted);
        assert_eq!(r.reason.as_deref(), Some("Authors edit their own posts"));
        assert_eq!(
            r.fields.unwrap().iter().collect::<Vec<_>>(),
            [FieldIdx(3), FieldIdx(4)]
        );
        assert!(r.optional_slots.is_empty());
        assert_eq!(
            r.cond,
            Some(TCond::And(vec![
                slot_cmp(1, 0),
                cmp(6, CmpOp::Ne, s("archived")),
                TCond::IsNull(FieldIdx(7)),
                rel(9, Quant::One, slot_cmp(0, 1)),
                TCond::IsNotNull(FieldIdx(10)),
                rel(11, Quant::Some, cmp(1, CmpOp::Eq, s("rust"))),
                TCond::Or(vec![cmp(6, CmpOp::Eq, s("published")), slot_cmp(1, 2)]),
            ]))
        );
        let slot = |path: &str, json_path: &str| Slot {
            rule_index: 0,
            root: "user".into(),
            path: vec![path.into()],
            optional: false,
            kind: Kind::Int,
            list: false,
            negative: false,
            json_path: json_path.into(),
        };
        assert_eq!(
            t.slots,
            [
                slot("id", "conditions.author_id"),
                slot("org_id", "conditions.org.id"),
                slot("id", "conditions.$or[1].author_id"),
            ]
        );
    }

    #[test]
    fn slots_record_polarity_lists_and_optionality() {
        let t = compile(
            r#"[
              {"action":"read","subject":"Post","conditions":{
                "$not": {"$and": [{"author_id": "${user.a}"}, {"tags": {"$none": {"name": "${user.b}"}}}]},
                "reviewer": {"$none": {"$not": {"name": "${user.c}"}}},
                "org": {"$not": {"id": "${user.d}"}},
                "tags": {"$every": {"name": {"$in": "${user.e?}"}}},
                "$or": [{"reviewer": {"name": {"$contains": "${user.f.g}"}}}]
              }},
              {"action":"read","subject":"Post","inverted":true,
               "conditions":{"reviewer_id": {"$nin": "${user?}"}}}
            ]"#,
        );
        let summary: Vec<_> = t
            .slots
            .iter()
            .map(|s| {
                (
                    s.rule_index,
                    s.path.join("."),
                    s.negative,
                    s.list,
                    s.optional,
                    s.kind,
                )
            })
            .collect();
        assert_eq!(
            summary,
            [
                (0, "a".into(), true, false, false, Kind::Int),
                (0, "b".into(), false, false, false, Kind::String),
                (0, "c".into(), false, false, false, Kind::String),
                (0, "d".into(), true, false, false, Kind::Int),
                (0, "e".into(), false, true, true, Kind::String),
                (0, "f.g".into(), false, false, false, Kind::String),
                (1, String::new(), false, true, true, Kind::Int),
            ]
        );
        assert_eq!(
            t.slots[1].json_path,
            "conditions.$not.$and[1].tags.$none.name"
        );
        assert_eq!(t.slots[4].json_path, "conditions.tags.$every.name.$in");
        assert_eq!(t.rules[0].optional_slots, [SlotId(4)]);
        assert_eq!(t.rules[1].optional_slots, [SlotId(6)]);
        assert_eq!(t.rules[1].source_index, 1);
        assert_eq!(
            t.rules[1].cond,
            Some(TCond::NotIn {
                field: FieldIdx(2),
                values: ListOperand::Slot(SlotId(6)),
            })
        );
    }

    #[test]
    fn literals_are_typed_and_nothing_is_folded() {
        assert_eq!(
            cond(
                r#"{"$and":[],"$or":[],"title":"$$5","body":"$5","status":{"$in":[]},
                    "reviewer_id":{"$nin":[1,2]},"score":2,
                    "published_at":{"$lt":"2024-01-15T12:30:00+02:00"},"tags":{"$some":{}}}"#
            ),
            TCond::And(vec![
                TCond::And(vec![]),
                TCond::Or(vec![]),
                cmp(3, CmpOp::Eq, s("$5")),
                cmp(4, CmpOp::Eq, s("$5")),
                TCond::In {
                    field: FieldIdx(6),
                    values: ListOperand::Lit(vec![]),
                },
                TCond::NotIn {
                    field: FieldIdx(2),
                    values: ListOperand::Lit(vec![Value::Int(1), Value::Int(2)]),
                },
                cmp(8, CmpOp::Eq, Value::Float(2.0)),
                cmp(
                    7,
                    CmpOp::Lt,
                    Value::DateTime("2024-01-15T10:30:00Z".parse().unwrap())
                ),
                rel(11, Quant::Some, TCond::And(vec![])),
            ])
        );
        assert_eq!(
            cond(r#"{"score":{"$gt":1,"$lte":5}}"#),
            TCond::And(vec![
                cmp(8, CmpOp::Gt, Value::Float(1.0)),
                cmp(8, CmpOp::Lte, Value::Float(5.0)),
            ])
        );
        assert_eq!(
            cond(r#"{"title":{"$startsWith":"${user.prefix}"}}"#),
            TCond::Str {
                field: FieldIdx(3),
                op: StrOp::StartsWith,
                value: Operand::Slot(SlotId(0)),
            }
        );
        assert_eq!(
            cond(r#"{"locked":true}"#),
            cmp(5, CmpOp::Eq, Value::Bool(true))
        );
        assert_eq!(cond(r#"{"reviewer":null}"#), TCond::IsNull(FieldIdx(10)));
        assert_eq!(
            cond(r#"{"reviewer_id":{"$ne":null}}"#),
            TCond::IsNotNull(FieldIdx(2))
        );
        assert_eq!(
            cond(r#"{"org":{"$none":{}}}"#),
            rel(9, Quant::None, TCond::And(vec![]))
        );
        assert_eq!(
            cond(r#"{"reviewer":{"$not":{"name":"x"}}}"#),
            rel(
                10,
                Quant::One,
                TCond::Not(Box::new(cmp(1, CmpOp::Eq, s("x"))))
            )
        );
    }

    #[test]
    fn uuid_and_date_strings_parse_strictly() {
        let u = "67e55044-10b1-426f-9247-bb680e5fe0c8";
        assert_eq!(parse_uuid(u), Ok(Value::Uuid(u.parse().unwrap())));
        assert!(parse_uuid("67e5504410b1426f9247bb680e5fe0c8").is_err());
        assert!(parse_uuid("{67e55044-10b1-426f-9247-bb680e5fe0c8}").is_err());
        assert_eq!(
            parse_date("2024-02-29"),
            Ok(Value::Date("2024-02-29".parse().unwrap()))
        );
        assert!(parse_date("2023-02-29").is_err());
        assert!(parse_date("2024-02-29T00:00:00Z").is_err());
        assert!(parse_datetime("2024-02-29").is_err());
    }
}
