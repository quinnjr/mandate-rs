//! Implementation of `#[derive(IntoValue)]`.

use proc_macro2::TokenStream;
use quote::quote;
use syn::{Attribute, Data, DeriveInput, Error, Expr, Fields, Lit, Token, spanned::Spanned};

use crate::naming::apply_rename_all;

/// Reads a string literal from `= "lit"` or `(serialize = "lit", ...)`.
fn string_arg(meta: &syn::meta::ParseNestedMeta<'_>) -> syn::Result<Option<String>> {
    if meta.input.peek(Token![=]) {
        let lit: Lit = meta.value()?.parse()?;
        match lit {
            Lit::Str(s) => Ok(Some(s.value())),
            other => Err(Error::new(other.span(), "expected a string literal")),
        }
    } else if meta.input.peek(syn::token::Paren) {
        let mut ser = None;
        meta.parse_nested_meta(|inner| {
            if inner.path.is_ident("serialize") {
                ser = string_arg(&inner)?;
            } else if inner.input.peek(Token![=]) {
                inner.value()?.parse::<Expr>()?;
            }
            Ok(())
        })?;
        Ok(ser)
    } else {
        Ok(None)
    }
}

/// Skips the argument of an attribute key we do not care about.
fn skip_arg(meta: &syn::meta::ParseNestedMeta<'_>) -> syn::Result<()> {
    if meta.input.peek(Token![=]) {
        meta.value()?.parse::<Expr>()?;
    } else if meta.input.peek(syn::token::Paren) {
        let content;
        syn::parenthesized!(content in meta.input);
        content.parse::<TokenStream>()?;
    }
    Ok(())
}

#[derive(Default)]
struct Attrs {
    /// `#[value(rename = ..)]`
    value_rename: Option<String>,
    /// `#[serde(rename = ..)]`
    serde_rename: Option<String>,
    /// `#[serde(rename_all = ..)]`
    rename_all: Option<String>,
}

fn read_attrs(attrs: &[Attribute]) -> syn::Result<Attrs> {
    let mut out = Attrs::default();
    for a in attrs {
        if a.path().is_ident("value") {
            a.parse_nested_meta(|m| {
                if m.path.is_ident("rename") {
                    out.value_rename = string_arg(&m)?;
                    Ok(())
                } else {
                    Err(m.error("unknown `value` attribute; expected `rename`"))
                }
            })?;
        } else if a.path().is_ident("serde") {
            a.parse_nested_meta(|m| {
                if m.path.is_ident("rename") {
                    out.serde_rename = string_arg(&m)?;
                    Ok(())
                } else if m.path.is_ident("rename_all") {
                    out.rename_all = string_arg(&m)?;
                    Ok(())
                } else {
                    skip_arg(&m)
                }
            })?;
        }
    }
    Ok(out)
}

pub(crate) fn expand(input: DeriveInput) -> syn::Result<TokenStream> {
    if !input.generics.params.is_empty() {
        return Err(Error::new(
            input.generics.span(),
            "`IntoValue` cannot be derived for generic enums",
        ));
    }
    let Data::Enum(data) = &input.data else {
        return Err(Error::new(
            input.ident.span(),
            "`IntoValue` can only be derived for enums",
        ));
    };
    let container = read_attrs(&input.attrs)?;
    let rule = container.rename_all.as_deref();

    let mut names: Vec<String> = Vec::new();
    let mut idents = Vec::new();
    for v in &data.variants {
        if !matches!(v.fields, Fields::Unit) {
            return Err(Error::new(
                v.span(),
                "`IntoValue` supports only unit variants",
            ));
        }
        let at = read_attrs(&v.attrs)?;
        let raw = v.ident.to_string();
        let raw = raw.strip_prefix("r#").unwrap_or(&raw).to_owned();
        let name = if let Some(n) = at.value_rename.or(at.serde_rename) {
            n
        } else if let Some(rule) = rule {
            apply_rename_all(rule, &raw).map_err(|e| Error::new(v.span(), e))?
        } else {
            raw
        };
        if name.starts_with('$') {
            return Err(Error::new(
                v.span(),
                format!("value name `{name}` must not start with `$`"),
            ));
        }
        if names.contains(&name) {
            return Err(Error::new(
                v.span(),
                format!("duplicate value name `{name}`"),
            ));
        }
        names.push(name);
        idents.push(&v.ident);
    }

    let ty = &input.ident;
    Ok(quote! {
        impl ::mandate::IntoValue for #ty {
            const VARIANTS: &'static [&'static str] = &[#(#names),*];
            fn variant_name(&self) -> &'static str {
                match self {
                    #(Self::#idents => #names,)*
                }
            }
        }
        impl ::mandate::ScalarValue for #ty {
            const KIND: ::mandate::Kind = ::mandate::Kind::Enum(<Self as ::mandate::IntoValue>::VARIANTS);
            fn to_value(&self) -> ::mandate::Value {
                ::mandate::Value::String(::core::convert::From::from(
                    <Self as ::mandate::IntoValue>::variant_name(self),
                ))
            }
            fn value_ref(&self) -> ::mandate::ValueRef<'_> {
                ::mandate::ValueRef::Str(<Self as ::mandate::IntoValue>::variant_name(self))
            }
        }
        impl ::mandate::Scalar for #ty {
            type Inner = Self;
            type Nullability = ::mandate::NonNull;
            fn value_ref(&self) -> ::mandate::ValueRef<'_> {
                <Self as ::mandate::ScalarValue>::value_ref(self)
            }
        }
    })
}
