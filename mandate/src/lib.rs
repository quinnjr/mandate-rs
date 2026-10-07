//! `mandate-rs`: a CASL-style authorization library.
#![deny(missing_docs)]
#![forbid(unsafe_code)]

extern crate self as mandate;

pub mod field;
pub mod fieldset;
pub mod relation;
pub mod scalar;
pub mod schema;
pub mod traits;
pub mod value;

pub use field::{Field, FieldRef, Opaque, Rel};
pub use fieldset::{FieldMask, FieldSet};
#[cfg(feature = "derive")]
pub use mandate_derive::IntoValue;
pub use relation::{Cardinality, DynMany, RelationRef, RelationSlot, ResourcePtr, ToMany, ToOne};
pub use scalar::{
    IntoValue, NonNull, Nullability, Nullable, Ordered, Scalar, ScalarValue, Textual,
};
pub use schema::{CardinalityKind, FieldDef, FieldIdx, FieldKind, Kind, MAX_FIELDS, Schema};
pub use traits::{DynResource, LoadState, Resource};
#[cfg(feature = "chrono")]
pub use value::truncate_micros;
pub use value::{Value, ValueRef};
