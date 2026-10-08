//! Implementation of `#[derive(Subject)]`.

use proc_macro2::TokenStream;
use quote::quote;
use syn::{DeriveInput, Error, Type};

use crate::naming::unraw;
use crate::unit_enum::{Named, check_unit, rename_value, variants};

pub(crate) fn expand(input: DeriveInput) -> syn::Result<TokenStream> {
    let variants = variants(&input, "Subject")?;
    let ty = &input.ident;
    let mut named = Named::new("subject");
    let mut schemas = Vec::new();
    let mut bindings = Vec::new();
    let mut all: Option<&syn::Ident> = None;
    for v in variants {
        check_unit(v, "Subject")?;
        let mut rename = None;
        let mut is_all = false;
        let mut resource: Option<Type> = None;
        for a in v.attrs.iter().filter(|a| a.path().is_ident("subject")) {
            a.parse_nested_meta(|m| {
                if m.path.is_ident("all") {
                    is_all = true;
                } else if m.path.is_ident("rename") {
                    rename = Some(rename_value(&m)?);
                } else if m.path.is_ident("resource") {
                    resource = Some(m.value()?.parse()?);
                } else {
                    return Err(m.error("expected `all`, `resource = Type` or `rename = \"...\"`"));
                }
                Ok(())
            })?;
        }
        if is_all {
            if resource.is_some() {
                return Err(Error::new_spanned(
                    v,
                    "`#[subject(all)]` cannot be combined with `resource`",
                ));
            }
            if all.is_some() {
                return Err(Error::new_spanned(
                    v,
                    "only one variant may be marked `#[subject(all)]`",
                ));
            }
            all = Some(&v.ident);
        }
        named.push(v, rename.unwrap_or_else(|| unraw(&v.ident)))?;
        let id = &v.ident;
        match resource {
            Some(t) => {
                schemas.push(quote!(::core::option::Option::Some(
                    <#t as ::mandate::Resource>::schema()
                )));
                bindings.push(quote! {
                    impl ::mandate::SubjectResource<#ty> for #t {
                        const SUBJECT: #ty = #ty::#id;
                    }
                });
            }
            None => schemas.push(quote!(::core::option::Option::None)),
        }
    }
    let dense = named.dense_items();
    let idents = named.idents();
    let all = match all {
        Some(a) => quote!(::core::option::Option::Some(Self::#a)),
        None => quote!(::core::option::Option::None),
    };
    Ok(quote! {
        impl ::mandate::Subject for #ty {
            const ALL: ::core::option::Option<Self> = #all;
            #dense
            fn schema(self) -> ::core::option::Option<&'static ::mandate::Schema> {
                match self { #(Self::#idents => #schemas,)* }
            }
        }
        #(#bindings)*
    })
}

#[cfg(test)]
mod tests {
    use syn::{DeriveInput, parse_quote};

    #[test]
    fn raw_identifiers_are_unrawed() {
        let input: DeriveInput = parse_quote! {
            enum E { r#match, #[subject(rename = "r#kept")] r#Other }
        };
        let expanded = super::expand(input).unwrap().to_string();
        // Rule names appear in the expansion as string literals.
        let names = |name: &str| expanded.contains(&format!("{name:?}"));
        assert!(names("match") && !names("r#match"));
        // An explicit rename is taken as written.
        assert!(names("r#kept"));
    }
}
