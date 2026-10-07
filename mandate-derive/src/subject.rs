//! Implementation of `#[derive(Subject)]`.

use proc_macro2::TokenStream;
use quote::quote;
use syn::{Data, DeriveInput, Error, Fields, Lit, Type};

pub(crate) fn expand(input: DeriveInput) -> syn::Result<TokenStream> {
    let Data::Enum(data) = &input.data else {
        return Err(Error::new_spanned(
            &input.ident,
            "`Subject` can only be derived for enums",
        ));
    };
    if !input.generics.params.is_empty() {
        return Err(Error::new_spanned(
            &input.generics,
            "`Subject` does not support generics",
        ));
    }
    let ty = &input.ident;
    let mut idents = Vec::new();
    let mut names: Vec<String> = Vec::new();
    let mut schemas = Vec::new();
    let mut bindings = Vec::new();
    let mut all: Option<&syn::Ident> = None;
    for v in &data.variants {
        if !matches!(v.fields, Fields::Unit) {
            return Err(Error::new_spanned(
                v,
                "`Subject` variants must be unit variants",
            ));
        }
        let mut rename = None;
        let mut is_all = false;
        let mut resource: Option<Type> = None;
        for a in v.attrs.iter().filter(|a| a.path().is_ident("subject")) {
            a.parse_nested_meta(|m| {
                if m.path.is_ident("all") {
                    is_all = true;
                } else if m.path.is_ident("rename") {
                    match m.value()?.parse::<Lit>()? {
                        Lit::Str(s) => rename = Some(s.value()),
                        other => {
                            return Err(Error::new_spanned(other, "expected a string literal"));
                        }
                    }
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
        let name = rename.unwrap_or_else(|| v.ident.to_string());
        if names.contains(&name) {
            return Err(Error::new_spanned(
                v,
                format!("duplicate subject name `{name}`"),
            ));
        }
        names.push(name);
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
        idents.push(id);
    }
    let count = idents.len();
    let idx = 0..count;
    let all = match all {
        Some(a) => quote!(::core::option::Option::Some(Self::#a)),
        None => quote!(::core::option::Option::None),
    };
    Ok(quote! {
        impl ::mandate::Subject for #ty {
            const COUNT: usize = #count;
            const ALL: ::core::option::Option<Self> = #all;
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
            fn schema(self) -> ::core::option::Option<&'static ::mandate::Schema> {
                match self { #(Self::#idents => #schemas,)* }
            }
        }
        #(#bindings)*
    })
}
