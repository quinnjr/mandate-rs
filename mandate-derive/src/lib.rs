//! Derive macros for `mandate-rs`.

mod into_value;
mod naming;

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
