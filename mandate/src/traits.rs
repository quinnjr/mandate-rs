//! Core resource traits.

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
    fn schema(&self) -> &'static Schema;
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
