//! `mandate-rs`: a CASL-style authorization library.
#![deny(missing_docs)]
#![forbid(unsafe_code)]

extern crate self as mandate;

pub mod scalar;
pub mod schema;
pub mod value;

pub use scalar::{
    IntoValue, NonNull, Nullability, Nullable, Ordered, Scalar, ScalarValue, Textual,
};
pub use schema::{CardinalityKind, FieldDef, FieldIdx, FieldKind, Kind, MAX_FIELDS, Schema};
#[cfg(feature = "chrono")]
pub use value::truncate_micros;
pub use value::{Value, ValueRef};
#[cfg(feature = "derive")]
pub use mandate_derive::IntoValue;
