//! Implementation of `#[derive(Action)]`.

use proc_macro2::TokenStream;
use quote::quote;
use syn::{DeriveInput, Error};

use crate::naming::{apply_rename_all, unraw};
use crate::unit_enum::{Named, check_unit, rename_value, variants};

pub(crate) fn expand(input: DeriveInput) -> syn::Result<TokenStream> {
    let variants = variants(&input, "Action")?;
    let ty = &input.ident;
    let mut named = Named::new("action");
    let mut manage: Option<&syn::Ident> = None;
    for v in variants {
        check_unit(v, "Action")?;
        let mut rename = None;
        let mut is_manage = false;
        for a in v.attrs.iter().filter(|a| a.path().is_ident("action")) {
            a.parse_nested_meta(|m| {
                if m.path.is_ident("manage") {
                    is_manage = true;
                } else if m.path.is_ident("rename") {
                    rename = Some(rename_value(&m)?);
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
            None => apply_rename_all("snake_case", &unraw(&v.ident))
                .map_err(|e| Error::new_spanned(v, e))?,
        };
        named.push(v, name)?;
    }
    let dense = named.dense_items();
    let manage = match manage {
        Some(m) => quote!(::core::option::Option::Some(Self::#m)),
        None => quote!(::core::option::Option::None),
    };
    Ok(quote! {
        impl ::mandate::Action for #ty {
            const MANAGE: ::core::option::Option<Self> = #manage;
            #dense
        }
    })
}

#[cfg(test)]
mod tests {
    use syn::{DeriveInput, parse_quote};

    #[test]
    fn raw_identifiers_are_unrawed() {
        let input: DeriveInput = parse_quote! {
            enum E { r#match, r#LoopAround, #[action(rename = "r#kept")] r#Other }
        };
        let expanded = super::expand(input).unwrap().to_string();
        // Rule names appear in the expansion as string literals.
        let names = |name: &str| expanded.contains(&format!("{name:?}"));
        assert!(names("match") && !names("r#match"));
        assert!(names("loop_around"));
        // An explicit rename is taken as written.
        assert!(names("r#kept"));
    }
}
