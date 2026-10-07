//! Implementation of `#[derive(Action)]`.

use proc_macro2::TokenStream;
use quote::quote;
use syn::{Data, DeriveInput, Error, Fields, Lit};

use crate::naming::apply_rename_all;

pub(crate) fn expand(input: DeriveInput) -> syn::Result<TokenStream> {
    let Data::Enum(data) = &input.data else {
        return Err(Error::new_spanned(
            &input.ident,
            "`Action` can only be derived for enums",
        ));
    };
    if !input.generics.params.is_empty() {
        return Err(Error::new_spanned(
            &input.generics,
            "`Action` does not support generics",
        ));
    }
    let ty = &input.ident;
    let mut idents = Vec::new();
    let mut names: Vec<String> = Vec::new();
    let mut manage: Option<&syn::Ident> = None;
    for v in &data.variants {
        if !matches!(v.fields, Fields::Unit) {
            return Err(Error::new_spanned(
                v,
                "`Action` variants must be unit variants",
            ));
        }
        let mut rename = None;
        let mut is_manage = false;
        for a in v.attrs.iter().filter(|a| a.path().is_ident("action")) {
            a.parse_nested_meta(|m| {
                if m.path.is_ident("manage") {
                    is_manage = true;
                } else if m.path.is_ident("rename") {
                    match m.value()?.parse::<Lit>()? {
                        Lit::Str(s) => rename = Some(s.value()),
                        other => {
                            return Err(Error::new_spanned(other, "expected a string literal"));
                        }
                    }
                } else {
                    return Err(m.error("expected `manage` or `rename = \"...\"`"));
                }
                Ok(())
            })?;
        }
        if is_manage {
            if manage.is_some() {
                return Err(Error::new_spanned(
                    v,
                    "only one variant may be marked `#[action(manage)]`",
                ));
            }
            manage = Some(&v.ident);
        }
        let name = match rename {
            Some(n) => n,
            None => apply_rename_all("snake_case", &v.ident.to_string())
                .map_err(|e| Error::new_spanned(v, e))?,
        };
        if names.contains(&name) {
            return Err(Error::new_spanned(
                v,
                format!("duplicate action name `{name}`"),
            ));
        }
        names.push(name);
        idents.push(&v.ident);
    }
    let count = idents.len();
    let idx = 0..count;
    let manage = match manage {
        Some(m) => quote!(::core::option::Option::Some(Self::#m)),
        None => quote!(::core::option::Option::None),
    };
    Ok(quote! {
        impl ::mandate::Action for #ty {
            const COUNT: usize = #count;
            const MANAGE: ::core::option::Option<Self> = #manage;
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
    })
}
