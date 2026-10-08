//! Filter plans: the rows an action is allowed on, as one condition for a
//! database query (spec §8).

use core::fmt;
use core::marker::PhantomData;

use serde::{Serialize, Serializer};

use crate::condition::eval::eval;
use crate::{Condition, EvalError, Resource, Schema};

/// Which rows of `R` an action is allowed on; see
/// [`Ability::access`](crate::Ability::access).
pub enum Access<R> {
    /// No row.
    Denied,
    /// Every row.
    All,
    /// Exactly the rows the plan matches.
    Filter(Plan<R>),
}

/// A backend-neutral row filter over `R`, for adapters to lower to a query.
///
/// The condition is folded and in restricted negation normal form (spec §8):
/// groups are flattened and deduplicated, the only constant left is a
/// relation's `cond: None`, `Every` never appears, and `Not` appears only
/// directly above a [`Condition::Str`] leaf. Adapters still owe the §8 null
/// guards (`Ne`, `NotIn`, and `Not(Str)` also match null on a nullable
/// field) and lower quantifiers and to-one `IsNull`/`IsNotNull` to `EXISTS`.
pub struct Plan<R> {
    condition: Condition,
    _r: PhantomData<fn() -> R>,
}

impl<R> Plan<R> {
    pub(crate) fn new(condition: Condition) -> Self {
        Plan {
            condition,
            _r: PhantomData,
        }
    }

    /// The filter condition; its field indices refer to
    /// [`schema`](Self::schema).
    pub fn condition(&self) -> &Condition {
        &self.condition
    }
}

impl<R: Resource> Plan<R> {
    /// The schema of `R`, for mapping fields to columns.
    pub fn schema(&self) -> &'static Schema {
        R::schema()
    }

    /// The reference evaluator: whether `r` matches the plan.
    ///
    /// For every fully loaded `r`, this is `Ok(true)` exactly when
    /// [`Ability::can`](crate::Ability::can) allows the planned action on `r`.
    pub fn eval(&self, r: &R) -> Result<bool, EvalError> {
        eval(&self.condition, R::schema(), r.as_dyn())
    }
}

/// Serializes the condition.
impl<R> Serialize for Plan<R> {
    fn serialize<Ser: Serializer>(&self, s: Ser) -> Result<Ser::Ok, Ser::Error> {
        self.condition.serialize(s)
    }
}

impl<R> Clone for Plan<R> {
    fn clone(&self) -> Self {
        Plan::new(self.condition.clone())
    }
}

impl<R> PartialEq for Plan<R> {
    fn eq(&self, other: &Self) -> bool {
        self.condition == other.condition
    }
}

impl<R> fmt::Debug for Plan<R> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("Plan").field(&self.condition).finish()
    }
}

impl<R> Clone for Access<R> {
    fn clone(&self) -> Self {
        match self {
            Access::Denied => Access::Denied,
            Access::All => Access::All,
            Access::Filter(p) => Access::Filter(p.clone()),
        }
    }
}

impl<R> PartialEq for Access<R> {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Access::Denied, Access::Denied) | (Access::All, Access::All) => true,
            (Access::Filter(a), Access::Filter(b)) => a == b,
            _ => false,
        }
    }
}

impl<R> fmt::Debug for Access<R> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Access::Denied => f.write_str("Denied"),
            Access::All => f.write_str("All"),
            Access::Filter(p) => f.debug_tuple("Filter").field(p).finish(),
        }
    }
}
