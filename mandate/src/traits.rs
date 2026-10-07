//! Core resource traits.

use core::fmt::Debug;

use crate::{FieldIdx, RelationRef, Schema, ValueRef};

/// A statically described resource type.
pub trait Resource: 'static {
    /// The static schema describing this resource type.
    fn schema() -> &'static Schema;
    /// Views this value as a dynamic resource.
    fn as_dyn(&self) -> &dyn DynResource;
}

/// Object-safe, schema-driven access to a resource's fields.
pub trait DynResource {
    /// The schema of this resource.
    fn resource_schema(&self) -> &'static Schema;
    /// The value of a scalar field.
    fn value(&self, field: FieldIdx) -> ValueRef<'_>;
    /// The state of a relation field.
    fn relation(&self, field: FieldIdx) -> RelationRef<'_>;
}

/// Reports which scalar fields of a partially loaded resource are loaded.
pub trait LoadState {
    /// Whether the named scalar field is loaded.
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
