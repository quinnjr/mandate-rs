//! Derive macros for `mandate-rs`.

mod into_value;
mod naming;
mod resource;

use proc_macro::TokenStream;
use syn::{DeriveInput, parse_macro_input};

/// Derives `IntoValue`, `ScalarValue` and `Scalar` for a unit-only enum.
///
/// Variant names follow serde: `#[value(rename)]` takes precedence over
/// `#[serde(rename)]`, then `#[serde(rename_all)]`, then the identifier.
#[proc_macro_derive(IntoValue, attributes(value))]
pub fn derive_into_value(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    into_value::expand(input).unwrap_or_else(syn::Error::into_compile_error).into()
}

/// Derives `Resource`, `DynResource`, `RelationSlot` and `ResourcePtr` for a
/// non-generic struct with named fields, plus a typed `pub const` handle per
/// schema field (`Post::AUTHOR_ID` for `author_id`).
///
/// Schema fields are numbered in declaration order. Unmarked fields must
/// implement `Scalar`. Field attributes:
///
/// - `#[resource(relation)]`: the field type implements `RelationSlot`;
/// - `#[resource(opaque)]`: any type, visible to field permissions only;
/// - `#[resource(skip)]`: not part of the schema;
/// - `#[resource(rename = "name")]`: the name used in rules (must not start
///   with `$`; names must be unique).
///
/// The struct attribute `#[resource(load_state = field)]` names a field
/// implementing `LoadState`. That field is not part of the schema; scalar
/// reads ask it, by schema name, whether the field was loaded, and report
/// `ValueRef::NotLoaded` when it was not.
#[proc_macro_derive(Resource, attributes(resource))]
pub fn derive_resource(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    resource::expand(input)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}
