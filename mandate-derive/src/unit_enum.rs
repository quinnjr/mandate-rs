//! The parts `#[derive(Action)]` and `#[derive(Subject)]` share: a
//! non-generic enum of unit variants, each with a unique rule name, and the
//! dense-index methods generated from them.

use proc_macro2::TokenStream;
use quote::quote;
use syn::meta::ParseNestedMeta;
use syn::punctuated::Punctuated;
use syn::token::Comma;
use syn::{Data, DeriveInput, Error, Fields, Ident, Lit, Variant};

/// The variants of `input`, which must be a non-generic enum; `derive`
/// names the derive (`"Action"`) in errors. Check each variant with
/// [`check_unit`].
pub(crate) fn variants<'a>(
    input: &'a DeriveInput,
    derive: &str,
) -> syn::Result<&'a Punctuated<Variant, Comma>> {
    let Data::Enum(data) = &input.data else {
        return Err(Error::new_spanned(
            &input.ident,
            format!("`{derive}` can only be derived for enums"),
        ));
    };
    if !input.generics.params.is_empty() {
        return Err(Error::new_spanned(
            &input.generics,
            format!("`{derive}` does not support generics"),
        ));
    }
    Ok(&data.variants)
}

/// Fails unless `v` is a unit variant.
pub(crate) fn check_unit(v: &Variant, derive: &str) -> syn::Result<()> {
    if matches!(v.fields, Fields::Unit) {
        Ok(())
    } else {
        Err(Error::new_spanned(
            v,
            format!("`{derive}` variants must be unit variants"),
        ))
    }
}

/// The string literal of a `rename = "..."` key.
pub(crate) fn rename_value(m: &ParseNestedMeta<'_>) -> syn::Result<String> {
    match m.value()?.parse::<Lit>()? {
        Lit::Str(s) => Ok(s.value()),
        other => Err(Error::new_spanned(other, "expected a string literal")),
    }
}

/// The variants of the enum with their rule names, in declaration order.
pub(crate) struct Named<'a> {
    /// What a name names in errors: `"action"` or `"subject"`.
    what: &'static str,
    idents: Vec<&'a Ident>,
    names: Vec<String>,
}

impl<'a> Named<'a> {
    pub(crate) fn new(what: &'static str) -> Self {
        Self {
            what,
            idents: Vec::new(),
            names: Vec::new(),
        }
    }

    /// Adds `v` under `name`, which no earlier variant may have.
    pub(crate) fn push(&mut self, v: &'a Variant, name: String) -> syn::Result<()> {
        if self.names.contains(&name) {
            return Err(Error::new_spanned(
                v,
                format!("duplicate {} name `{name}`", self.what),
            ));
        }
        self.names.push(name);
        self.idents.push(&v.ident);
        Ok(())
    }

    /// The variants in declaration order.
    pub(crate) fn idents(&self) -> &[&'a Ident] {
        &self.idents
    }

    /// `COUNT` and the `index`, `name`, `from_name` and `all` items of the
    /// `Action`/`Subject` impl: indices are declaration order.
    pub(crate) fn dense_items(&self) -> TokenStream {
        let (idents, names) = (&self.idents, &self.names);
        let count = idents.len();
        let idx = 0..count;
        quote! {
            const COUNT: usize = #count;
            fn index(self) -> usize {
                match self { #(Self::#idents => #idx,)* }
            }
            fn name(self) -> &'static str {
                match self { #(Self::#idents => #names,)* }
            }
            fn from_name(name: &str) -> ::core::option::Option<Self> {
                match name {
                    #(#names => ::core::option::Option::Some(Self::#idents),)*
                    _ => ::core::option::Option::None,
                }
            }
            fn all() -> &'static [Self] {
                &[#(Self::#idents),*]
            }
        }
    }
}
