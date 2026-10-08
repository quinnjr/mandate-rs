//! The condition AST and its typed builders.

mod cond;
pub(crate) mod eval;
pub(crate) mod fold;

pub use cond::Cond;

use serde::Serialize;

use crate::{FieldIdx, Value};

/// Comparison operator of a [`Condition::Cmp`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
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

/// Text operator of a [`Condition::Str`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum StrOp {
    /// Contains the substring.
    Contains,
    /// Starts with the prefix.
    StartsWith,
    /// Ends with the suffix.
    EndsWith,
}

/// Quantifier of a [`Condition::Rel`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
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
