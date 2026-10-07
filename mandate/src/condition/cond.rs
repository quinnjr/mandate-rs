//! Typed condition builders.

use core::fmt;
use core::marker::PhantomData;
use core::ops::Not;

use super::{CmpOp, Condition, Quant, StrOp};
use crate::{
    Field, Nullable, Ordered, Rel, RelationSlot, Scalar, ScalarValue, Textual, ToMany, ToOne, Value,
};

/// A condition on resource `R`, built through the typed field handles.
pub struct Cond<R>(Condition, PhantomData<fn() -> R>);

impl<R> Cond<R> {
    fn new(c: Condition) -> Self {
        Self(c, PhantomData)
    }

    /// The underlying AST.
    pub fn condition(&self) -> &Condition {
        &self.0
    }

    /// Consumes the builder, returning the AST.
    pub fn into_condition(self) -> Condition {
        self.0
    }

    /// Both conditions hold.
    pub fn and(self, other: Cond<R>) -> Cond<R> {
        Self::new(Condition::And(vec![self.0, other.0]))
    }

    /// At least one condition holds.
    pub fn or(self, other: Cond<R>) -> Cond<R> {
        Self::new(Condition::Or(vec![self.0, other.0]))
    }

    /// All conditions hold (`true` when empty).
    pub fn all(conds: impl IntoIterator<Item = Cond<R>>) -> Cond<R> {
        Self::new(Condition::And(conds.into_iter().map(|c| c.0).collect()))
    }

    /// At least one condition holds (`false` when empty).
    pub fn any(conds: impl IntoIterator<Item = Cond<R>>) -> Cond<R> {
        Self::new(Condition::Or(conds.into_iter().map(|c| c.0).collect()))
    }
}

impl<R> Clone for Cond<R> {
    fn clone(&self) -> Self {
        Self::new(self.0.clone())
    }
}

impl<R> fmt::Debug for Cond<R> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("Cond").field(&self.0).finish()
    }
}

impl<R> PartialEq for Cond<R> {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

impl<R> Not for Cond<R> {
    type Output = Cond<R>;
    fn not(self) -> Cond<R> {
        Self::new(Condition::Not(Box::new(self.0)))
    }
}

fn to_value<T: Scalar>(v: impl Into<T::Inner>) -> Value {
    ScalarValue::to_value(&v.into())
}

impl<R, T: Scalar> Field<R, T> {
    fn cmp(self, op: CmpOp, v: impl Into<T::Inner>) -> Cond<R> {
        Cond::new(Condition::Cmp {
            field: self.idx(),
            op,
            value: to_value::<T>(v),
        })
    }

    fn values<V: Into<T::Inner>>(vs: impl IntoIterator<Item = V>) -> Vec<Value> {
        vs.into_iter().map(to_value::<T>).collect()
    }

    fn text(self, op: StrOp, v: impl Into<String>) -> Cond<R> {
        Cond::new(Condition::Str {
            field: self.idx(),
            op,
            value: v.into(),
        })
    }

    /// The field equals `v`.
    pub fn eq(self, v: impl Into<T::Inner>) -> Cond<R> {
        self.cmp(CmpOp::Eq, v)
    }

    /// The field does not equal `v`.
    pub fn ne(self, v: impl Into<T::Inner>) -> Cond<R> {
        self.cmp(CmpOp::Ne, v)
    }

    /// The field equals one of `vs`.
    pub fn is_in<V: Into<T::Inner>>(self, vs: impl IntoIterator<Item = V>) -> Cond<R> {
        Cond::new(Condition::In {
            field: self.idx(),
            values: Self::values(vs),
        })
    }

    /// The field equals none of `vs`.
    pub fn not_in<V: Into<T::Inner>>(self, vs: impl IntoIterator<Item = V>) -> Cond<R> {
        Cond::new(Condition::NotIn {
            field: self.idx(),
            values: Self::values(vs),
        })
    }

    /// The field is less than `v`.
    pub fn lt(self, v: impl Into<T::Inner>) -> Cond<R>
    where
        T::Inner: Ordered,
    {
        self.cmp(CmpOp::Lt, v)
    }

    /// The field is less than or equal to `v`.
    pub fn lte(self, v: impl Into<T::Inner>) -> Cond<R>
    where
        T::Inner: Ordered,
    {
        self.cmp(CmpOp::Lte, v)
    }

    /// The field is greater than `v`.
    pub fn gt(self, v: impl Into<T::Inner>) -> Cond<R>
    where
        T::Inner: Ordered,
    {
        self.cmp(CmpOp::Gt, v)
    }

    /// The field is greater than or equal to `v`.
    pub fn gte(self, v: impl Into<T::Inner>) -> Cond<R>
    where
        T::Inner: Ordered,
    {
        self.cmp(CmpOp::Gte, v)
    }

    /// The field contains `v`.
    pub fn contains(self, v: impl Into<String>) -> Cond<R>
    where
        T::Inner: Textual,
    {
        self.text(StrOp::Contains, v)
    }

    /// The field starts with `v`.
    pub fn starts_with(self, v: impl Into<String>) -> Cond<R>
    where
        T::Inner: Textual,
    {
        self.text(StrOp::StartsWith, v)
    }

    /// The field ends with `v`.
    pub fn ends_with(self, v: impl Into<String>) -> Cond<R>
    where
        T::Inner: Textual,
    {
        self.text(StrOp::EndsWith, v)
    }

    /// The field is null.
    pub fn is_null(self) -> Cond<R>
    where
        T: Scalar<Nullability = Nullable>,
    {
        Cond::new(Condition::IsNull(self.idx()))
    }

    /// The field is not null.
    pub fn is_not_null(self) -> Cond<R>
    where
        T: Scalar<Nullability = Nullable>,
    {
        Cond::new(Condition::IsNotNull(self.idx()))
    }
}

impl<R, T: RelationSlot> Rel<R, T> {
    fn quantified(self, quant: Quant, c: Cond<T::Target>) -> Cond<R> {
        Cond::new(Condition::Rel {
            relation: self.idx(),
            quant,
            cond: Some(Box::new(c.0)),
        })
    }

    /// The to-one target exists and matches `c`.
    pub fn then(self, c: Cond<T::Target>) -> Cond<R>
    where
        T: RelationSlot<Cardinality = ToOne>,
    {
        self.quantified(Quant::One, c)
    }

    /// The nullable to-one relation is absent.
    pub fn is_null(self) -> Cond<R>
    where
        T: RelationSlot<Cardinality = ToOne, Nullability = Nullable>,
    {
        Cond::new(Condition::IsNull(self.idx()))
    }

    /// The nullable to-one relation is present.
    pub fn is_not_null(self) -> Cond<R>
    where
        T: RelationSlot<Cardinality = ToOne, Nullability = Nullable>,
    {
        Cond::new(Condition::IsNotNull(self.idx()))
    }

    /// At least one related element matches `c`.
    pub fn some(self, c: Cond<T::Target>) -> Cond<R>
    where
        T: RelationSlot<Cardinality = ToMany>,
    {
        self.quantified(Quant::Some, c)
    }

    /// Every related element matches `c`.
    pub fn every(self, c: Cond<T::Target>) -> Cond<R>
    where
        T: RelationSlot<Cardinality = ToMany>,
    {
        self.quantified(Quant::Every, c)
    }

    /// No related element matches `c`.
    pub fn none(self, c: Cond<T::Target>) -> Cond<R>
    where
        T: RelationSlot<Cardinality = ToMany>,
    {
        self.quantified(Quant::None, c)
    }
}
