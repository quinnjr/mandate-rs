//! The condition AST and its typed builders.

mod cond;
pub(crate) mod deps;
pub(crate) mod eval;
pub(crate) mod fold;
pub(crate) mod limits;
pub(crate) mod nnf;
pub(crate) mod validate;

pub use cond::Cond;

use serde::Serialize;

use crate::{FieldIdx, Value};

/// Comparison operator of a [`Condition::Cmp`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub enum CmpOp {
    /// Equal.
    Eq,
    /// Not equal.
    Ne,
    /// Less than.
    Lt,
    /// Less than or equal.
    Lte,
    /// Greater than.
    Gt,
    /// Greater than or equal.
    Gte,
}

/// The template key of `$in` (a [`Condition::In`]; spec §6.2).
pub(crate) const IN_KEY: &str = "$in";
/// The template key of `$nin` (a [`Condition::NotIn`]; spec §6.2).
pub(crate) const NIN_KEY: &str = "$nin";
/// The template key of `$isNull` (a [`Condition::IsNull`] or
/// [`Condition::IsNotNull`]; spec §6.2).
pub(crate) const IS_NULL_KEY: &str = "$isNull";

/// Implements `template_key` and `from_template_key` for an operator enum
/// from one list of `Variant => "key"` pairs, so the two directions cannot
/// drift apart. `template_key` matches exhaustively, so a new variant without
/// a key does not compile.
macro_rules! template_keys {
    ($ty:ident { $($variant:ident => $key:literal),+ $(,)? }) => {
        impl $ty {
            /// The operator's key in a template condition object (spec §6.2).
            pub(crate) fn template_key(self) -> &'static str {
                match self {
                    $($ty::$variant => $key,)+
                }
            }

            /// The operator whose [`template_key`](Self::template_key) is
            /// `key`.
            pub(crate) fn from_template_key(key: &str) -> Option<Self> {
                match key {
                    $($key => Some($ty::$variant),)+
                    _ => None,
                }
            }
        }
    };
}

template_keys!(CmpOp {
    Eq => "$eq",
    Ne => "$ne",
    Lt => "$lt",
    Lte => "$lte",
    Gt => "$gt",
    Gte => "$gte",
});

/// Text operator of a [`Condition::Str`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub enum StrOp {
    /// Contains the substring.
    Contains,
    /// Starts with the prefix.
    StartsWith,
    /// Ends with the suffix.
    EndsWith,
}

template_keys!(StrOp {
    Contains => "$contains",
    StartsWith => "$startsWith",
    EndsWith => "$endsWith",
});

/// Quantifier of a [`Condition::Rel`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub enum Quant {
    /// The to-one target matches.
    One,
    /// At least one to-many element matches.
    Some,
    /// Every to-many element matches.
    Every,
    /// No element (or no target) matches.
    None,
}

/// An untyped condition over a resource schema.
///
/// Field indices are relative to the schema in scope; inside
/// [`Condition::Rel`] they refer to the relation target's schema.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[non_exhaustive]
pub enum Condition {
    /// Compares a scalar field with a value.
    Cmp {
        /// The field.
        field: FieldIdx,
        /// The operator.
        op: CmpOp,
        /// The right-hand value.
        value: Value,
    },
    /// The field equals one of the values.
    In {
        /// The field.
        field: FieldIdx,
        /// The candidate values.
        values: Vec<Value>,
    },
    /// The field equals none of the values.
    NotIn {
        /// The field.
        field: FieldIdx,
        /// The excluded values.
        values: Vec<Value>,
    },
    /// A text operator on a string field.
    Str {
        /// The field.
        field: FieldIdx,
        /// The operator.
        op: StrOp,
        /// The operand.
        value: String,
    },
    /// The field (scalar or nullable to-one relation) is null.
    IsNull(FieldIdx),
    /// The field (scalar or nullable to-one relation) is not null.
    IsNotNull(FieldIdx),
    /// All children hold.
    And(Vec<Condition>),
    /// At least one child holds.
    Or(Vec<Condition>),
    /// The child does not hold.
    Not(Box<Condition>),
    /// A quantified condition on a relation; `cond: None` means true.
    Rel {
        /// The relation field.
        relation: FieldIdx,
        /// The quantifier.
        quant: Quant,
        /// The condition on the related resource(s), relative to the target schema.
        cond: Option<Box<Condition>>,
    },
}

impl Condition {
    /// The constant `b`. The AST has no constant node: true is the empty
    /// `And` and false the empty `Or`, the groups that fold to them.
    pub(crate) fn constant(b: bool) -> Condition {
        if b {
            Condition::And(Vec::new())
        } else {
            Condition::Or(Vec::new())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The operator keys of spec §6.2, written out rather than taken from
    /// the macro's list.
    const CMP_KEYS: [(CmpOp, &str); 6] = [
        (CmpOp::Eq, "$eq"),
        (CmpOp::Ne, "$ne"),
        (CmpOp::Lt, "$lt"),
        (CmpOp::Lte, "$lte"),
        (CmpOp::Gt, "$gt"),
        (CmpOp::Gte, "$gte"),
    ];
    const STR_KEYS: [(StrOp, &str); 3] = [
        (StrOp::Contains, "$contains"),
        (StrOp::StartsWith, "$startsWith"),
        (StrOp::EndsWith, "$endsWith"),
    ];

    #[test]
    fn template_keys_match_the_spec_both_ways() {
        for (op, key) in CMP_KEYS {
            assert_eq!(op.template_key(), key);
            assert_eq!(CmpOp::from_template_key(key), Some(op), "{key}");
            assert_eq!(StrOp::from_template_key(key), None, "{key}");
        }
        for (op, key) in STR_KEYS {
            assert_eq!(op.template_key(), key);
            assert_eq!(StrOp::from_template_key(key), Some(op), "{key}");
            assert_eq!(CmpOp::from_template_key(key), None, "{key}");
        }
    }

    #[test]
    fn other_keys_are_no_operator() {
        for key in ["$in", "$nin", "$isNull", "$EQ", ""] {
            assert_eq!(CmpOp::from_template_key(key), None, "{key}");
            assert_eq!(StrOp::from_template_key(key), None, "{key}");
        }
    }
}
