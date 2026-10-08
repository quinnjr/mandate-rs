//! Which conditions a field supports.
//!
//! One set of rules, shared by `Templates::compile` (for stored rules) and
//! `build()` (for code-built rules, whose hand-built field handles may not
//! match the schema).

use core::fmt;

use super::{CmpOp, Condition, Quant, StrOp};
use crate::{CardinalityKind, FieldDef, FieldIdx, FieldKind, Kind, Schema, Value};

/// How a condition node uses a field.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Use {
    /// `Cmp` with this operator.
    Cmp(CmpOp),
    /// `In` or `NotIn`.
    List,
    /// `Str` with this operator.
    Str(StrOp),
    /// `IsNull` or `IsNotNull`.
    NullTest,
    /// `Rel` with this quantifier.
    Rel(Quant),
}

/// Why a field does not support a [`Use`].
#[derive(Clone, Copy, Debug)]
pub(crate) enum Misuse {
    /// Opaque fields cannot appear in conditions.
    Opaque,
    /// A scalar operator (`Cmp`, `In`, `NotIn`, `Str`) on a relation.
    Relation,
    /// A quantifier on a field that is not a relation.
    NotRelation,
    /// The operator is not defined on the field's kind: ordering outside
    /// `Int`/`Float`/`DateTime`/`Date`, text operators outside `String`.
    Operator(Use, Kind),
    /// A null test on anything but a nullable scalar or a nullable to-one
    /// relation.
    NotNullable,
    /// The quantifier does not fit the relation's cardinality.
    Quantifier(Quant, CardinalityKind),
}

impl fmt::Display for Misuse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Misuse::Opaque => f.write_str("opaque fields cannot be used in conditions"),
            Misuse::Relation => {
                f.write_str("a relation field supports only quantifiers and null tests")
            }
            Misuse::NotRelation => f.write_str("a quantifier needs a relation field"),
            Misuse::Operator(Use::Cmp(op), kind) => {
                write!(f, "operator `{op:?}` is not allowed on {kind:?} fields")
            }
            Misuse::Operator(Use::Str(op), kind) => {
                write!(f, "operator `{op:?}` is not allowed on {kind:?} fields")
            }
            Misuse::Operator(u, kind) => write!(f, "{u:?} is not allowed on {kind:?} fields"),
            Misuse::NotNullable => {
                f.write_str("null tests need a nullable scalar or a nullable to-one relation")
            }
            Misuse::Quantifier(quant, cardinality) => {
                write!(
                    f,
                    "quantifier `{quant:?}` does not fit a {cardinality:?} relation"
                )
            }
        }
    }
}

/// Whether ordering (`Lt`, `Lte`, `Gt`, `Gte`) is defined on `kind`. The
/// filter-plan rewrite of a negated ordering relies on it.
fn ordered(kind: Kind) -> bool {
    matches!(kind, Kind::Int | Kind::Float | Kind::DateTime | Kind::Date)
}

/// Checks that a field of kind `field` supports `u`.
pub(crate) fn check(field: FieldKind, u: Use) -> Result<(), Misuse> {
    use CardinalityKind::{ToMany, ToOne};
    match (field, u) {
        (FieldKind::Opaque, _) => Err(Misuse::Opaque),
        (FieldKind::Scalar { nullable, .. }, Use::NullTest) => {
            nullable.then_some(()).ok_or(Misuse::NotNullable)
        }
        (FieldKind::Scalar { .. }, Use::Rel(_)) => Err(Misuse::NotRelation),
        (FieldKind::Scalar { kind, .. }, Use::Cmp(op)) => match op {
            CmpOp::Lt | CmpOp::Lte | CmpOp::Gt | CmpOp::Gte if !ordered(kind) => {
                Err(Misuse::Operator(u, kind))
            }
            _ => Ok(()),
        },
        (FieldKind::Scalar { kind, .. }, Use::Str(_)) if kind != Kind::String => {
            Err(Misuse::Operator(u, kind))
        }
        (FieldKind::Scalar { .. }, Use::List | Use::Str(_)) => Ok(()),
        (
            FieldKind::Relation {
                cardinality,
                nullable,
                ..
            },
            Use::NullTest,
        ) => (nullable && cardinality == ToOne)
            .then_some(())
            .ok_or(Misuse::NotNullable),
        (FieldKind::Relation { cardinality, .. }, Use::Rel(quant)) => match (quant, cardinality) {
            (Quant::One, ToOne) | (Quant::Some | Quant::Every, ToMany) | (Quant::None, _) => Ok(()),
            _ => Err(Misuse::Quantifier(quant, cardinality)),
        },
        (FieldKind::Relation { .. }, Use::Cmp(_) | Use::List | Use::Str(_)) => {
            Err(Misuse::Relation)
        }
    }
}

/// Whether `v` is a value of `kind`: of the same kind, and for an enum one
/// of its variants.
pub(crate) fn operand_fits(kind: Kind, v: &Value) -> bool {
    match (kind, v) {
        (Kind::Enum(variants), Value::String(s)) => variants.contains(&s.as_str()),
        (Kind::Enum(_), _) => false,
        _ => v.kind_matches(kind),
    }
}

/// An invalid field use found by [`fields`]: the dotted path to the field
/// (`#<index>` for an index the schema does not have) and the reason.
pub(crate) struct Invalid {
    pub(crate) path: String,
    pub(crate) reason: String,
}

/// Checks every field use in `c`, whose indices refer to `schema`: the
/// field exists, supports the use ([`check`]), and the operands fit its kind
/// ([`operand_fits`]). Recurses into relations with the target's schema;
/// `c` must be of bounded depth.
pub(crate) fn fields(c: &Condition, schema: &'static Schema) -> Result<(), Invalid> {
    fields_at(c, schema, "")
}

fn fields_at(c: &Condition, schema: &'static Schema, prefix: &str) -> Result<(), Invalid> {
    // The field at `idx`, after checking that it supports `u` with `operands`.
    let used = |idx: FieldIdx, u: Use, operands: &[Value]| -> Result<&'static FieldDef, Invalid> {
        let Some(def) = schema.field(idx) else {
            return Err(Invalid {
                path: format!("{prefix}#{}", idx.0),
                reason: format!("`{}` has no field with index {}", schema.name(), idx.0),
            });
        };
        let invalid = |reason: String| Invalid {
            path: format!("{prefix}{}", def.name()),
            reason,
        };
        check(def.kind(), u).map_err(|m| invalid(m.to_string()))?;
        if let FieldKind::Scalar { kind, .. } = def.kind()
            && let Some(v) = operands.iter().find(|v| !operand_fits(kind, v))
        {
            return Err(invalid(format!("operand {v:?} does not fit {kind:?}")));
        }
        Ok(def)
    };
    match c {
        Condition::Cmp { field, op, value } => {
            used(*field, Use::Cmp(*op), core::slice::from_ref(value)).map(drop)
        }
        Condition::In { field, values } | Condition::NotIn { field, values } => {
            used(*field, Use::List, values).map(drop)
        }
        Condition::Str { field, op, .. } => used(*field, Use::Str(*op), &[]).map(drop),
        Condition::IsNull(field) | Condition::IsNotNull(field) => {
            used(*field, Use::NullTest, &[]).map(drop)
        }
        Condition::And(cs) | Condition::Or(cs) => {
            cs.iter().try_for_each(|c| fields_at(c, schema, prefix))
        }
        Condition::Not(c) => fields_at(c, schema, prefix),
        Condition::Rel {
            relation,
            quant,
            cond,
        } => {
            let def = used(*relation, Use::Rel(*quant), &[])?;
            match (def.kind(), cond) {
                (FieldKind::Relation { target, .. }, Some(c)) => {
                    fields_at(c, target(), &format!("{prefix}{}.", def.name()))
                }
                _ => Ok(()),
            }
        }
    }
}
