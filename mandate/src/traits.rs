//! Core resource traits.

use core::fmt::Debug;

use crate::{FieldIdx, RelationRef, Schema, ValueRef};

/// A statically described resource type.
pub trait Resource: 'static {
    /// The static schema describing this resource type.
    ///
    /// Must return the same `static` on every call, and a different one for
    /// every type: a schema's address is its identity, compared with
    /// [`ptr::eq`](core::ptr::eq) (never by name) to check that a resource,
    /// a rule and a relation target agree. A schema built anew per call, or
    /// shared between types, makes those checks fail closed or pass wrongly.
    fn schema() -> &'static Schema;
    /// Views this value as a dynamic resource.
    fn as_dyn(&self) -> &dyn DynResource;
}

/// Object-safe, schema-driven access to a resource's fields.
///
/// Field indices are those of [`resource_schema`](Self::resource_schema).
/// Evaluation trusts these answers to fail closed, so an implementation
/// must keep to this contract:
///
/// - [`ValueRef::Null`] only for a loaded, nullable scalar that is null;
/// - a loaded scalar's [`ValueRef`] variant is that of the field's
///   [`Kind`](crate::Kind) (`Str` for `String` and `Enum`); evaluation
///   fails with [`EvalError::InvalidCondition`](crate::EvalError::InvalidCondition)
///   on any other;
/// - [`RelationRef::Absent`] only for a loaded to-one relation without a
///   target;
/// - [`ValueRef::NotLoaded`] / [`RelationRef::NotLoaded`] for data that was
///   not loaded (or was loaded only in part, such as a filtered to-many),
///   and for every index the type cannot answer: one outside its schema, or
///   a scalar index asked for a relation and the other way round.
///
/// Reading unanswerable data as null or absent would fail open: `Ne`,
/// `NotIn`, `Every` and `None` hold on it, so a `cannot` rule could stop
/// applying.
pub trait DynResource {
    /// The schema of this resource; the same `static` as
    /// [`Resource::schema`] for the type.
    fn resource_schema(&self) -> &'static Schema;
    /// The value of a scalar field; `NotLoaded` for an index that is not a
    /// loaded scalar of the schema.
    fn value(&self, field: FieldIdx) -> ValueRef<'_>;
    /// The state of a relation field; `NotLoaded` for an index that is not
    /// a loaded relation of the schema.
    fn relation(&self, field: FieldIdx) -> RelationRef<'_>;
}

/// Reports which scalar fields of a partially loaded resource are loaded.
///
/// Named by `#[resource(load_state = field)]`: before reading a scalar
/// field, the derived [`DynResource`] asks the tracker, and reports
/// [`ValueRef::NotLoaded`](crate::ValueRef::NotLoaded) (so checks fail
/// closed) when it answers `false`.
pub trait LoadState {
    /// Whether the scalar field called `field` is loaded.
    ///
    /// `field` is the field's schema name, the name rules use: the Rust
    /// field name, or its `#[resource(rename = "...")]`. Track the set of
    /// fields that were loaded and answer `false` for any name not in it,
    /// including names you do not recognise, so a mismatch fails closed.
    fn scalar_loaded(&self, field: &'static str) -> bool;
}

/// A user-defined set of actions, normally a fieldless enum deriving `Action`.
pub trait Action: Copy + Eq + Debug + Send + Sync + 'static {
    /// The number of actions.
    const COUNT: usize;
    /// The variant marked `#[action(manage)]`, which matches every action.
    const MANAGE: Option<Self>;
    /// The dense index of this action, in `0..COUNT`.
    fn index(self) -> usize;
    /// The name of this action as used in rules.
    fn name(self) -> &'static str;
    /// Looks an action up by its exact, case-sensitive name.
    fn from_name(name: &str) -> Option<Self>;
    /// Every action, in declaration order.
    fn all() -> &'static [Self];
}

/// A user-defined set of subjects, normally a fieldless enum deriving `Subject`.
pub trait Subject: Copy + Eq + Debug + Send + Sync + 'static {
    /// The number of subjects.
    const COUNT: usize;
    /// The variant marked `#[subject(all)]`, which matches every subject.
    const ALL: Option<Self>;
    /// The dense index of this subject, in `0..COUNT`.
    fn index(self) -> usize;
    /// The name of this subject as used in rules.
    fn name(self) -> &'static str;
    /// Looks a subject up by its exact, case-sensitive name.
    fn from_name(name: &str) -> Option<Self>;
    /// Every subject, in declaration order.
    fn all() -> &'static [Self];
    /// The schema of the bound resource, for `#[subject(resource = T)]` variants.
    fn schema(self) -> Option<&'static Schema>;
}

/// The binding of a resource type to a subject variant.
///
/// Generated only by `#[derive(Subject)]`.
pub trait SubjectResource<S: Subject>: Resource {
    /// The subject variant bound to this resource type.
    const SUBJECT: S;
}
