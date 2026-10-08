//! Implementation of `#[derive(Resource)]`.

use proc_macro2::{Literal, TokenStream};
use quote::{format_ident, quote, quote_spanned};
use syn::{Attribute, Data, DeriveInput, Error, Fields, Ident, LitStr, Type, spanned::Spanned};

/// Mirrors `mandate::MAX_FIELDS`.
const MAX_FIELDS: usize = 128;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Role {
    Scalar,
    Relation,
    Opaque,
    Skip,
}

struct FieldAttrs {
    role: Role,
    rename: Option<LitStr>,
}

/// Reads `#[resource(relation | opaque | skip | rename = "...")]` on a field.
fn field_attrs(attrs: &[Attribute]) -> syn::Result<FieldAttrs> {
    let mut role = None;
    let mut rename = None;
    for a in attrs.iter().filter(|a| a.path().is_ident("resource")) {
        a.parse_nested_meta(|m| {
            let r = if m.path.is_ident("relation") {
                Role::Relation
            } else if m.path.is_ident("opaque") {
                Role::Opaque
            } else if m.path.is_ident("skip") {
                Role::Skip
            } else if m.path.is_ident("rename") {
                if rename.is_some() {
                    return Err(m.error("duplicate `rename`"));
                }
                rename = Some(m.value()?.parse::<LitStr>()?);
                return Ok(());
            } else {
                return Err(m.error(
                    "unknown `resource` field attribute; expected `relation`, `opaque`, `skip`, or `rename`",
                ));
            };
            if role.is_some() {
                return Err(m.error("a field may have only one of `relation`, `opaque`, `skip`"));
            }
            role = Some(r);
            Ok(())
        })?;
    }
    let role = role.unwrap_or(Role::Scalar);
    if let (Role::Skip, Some(lit)) = (role, &rename) {
        return Err(Error::new(
            lit.span(),
            "`skip` cannot be combined with `rename`",
        ));
    }
    Ok(FieldAttrs { role, rename })
}

/// Reads `#[resource(load_state = field)]` on the struct.
fn load_state_attr(attrs: &[Attribute]) -> syn::Result<Option<Ident>> {
    let mut out = None;
    for a in attrs.iter().filter(|a| a.path().is_ident("resource")) {
        a.parse_nested_meta(|m| {
            if !m.path.is_ident("load_state") {
                return Err(m.error("unknown `resource` struct attribute; expected `load_state`"));
            }
            if out.is_some() {
                return Err(m.error("duplicate `load_state`"));
            }
            out = Some(m.value()?.parse::<Ident>()?);
            Ok(())
        })?;
    }
    Ok(out)
}

/// Strips a raw-identifier prefix.
fn unraw(ident: &Ident) -> String {
    let s = ident.to_string();
    s.strip_prefix("r#").map(str::to_owned).unwrap_or(s)
}

struct SchemaField<'a> {
    ident: &'a Ident,
    ty: &'a Type,
    role: Role,
    name: String,
}

pub(crate) fn expand(input: DeriveInput) -> syn::Result<TokenStream> {
    if !input.generics.params.is_empty() {
        return Err(Error::new(
            input.generics.span(),
            "`Resource` cannot be derived for generic structs",
        ));
    }
    let named = match &input.data {
        Data::Struct(s) if !matches!(s.fields, Fields::Unnamed(_)) => &s.fields,
        _ => {
            return Err(Error::new(
                input.ident.span(),
                "`Resource` can only be derived for structs with named fields",
            ));
        }
    };

    let load_state = load_state_attr(&input.attrs)?;
    if let Some(ls) = &load_state
        && !named.iter().any(|f| f.ident.as_ref() == Some(ls))
    {
        return Err(Error::new(
            ls.span(),
            format!("`load_state` names `{ls}`, which is not a field of this struct"),
        ));
    }

    let mut fields: Vec<SchemaField<'_>> = Vec::new();
    for f in named {
        let Some(ident) = &f.ident else { continue };
        let attrs = field_attrs(&f.attrs)?;
        if attrs.role == Role::Skip || load_state.as_ref() == Some(ident) {
            continue;
        }
        let (name, span) = match &attrs.rename {
            Some(lit) => (lit.value(), lit.span()),
            None => (unraw(ident), ident.span()),
        };
        if name.starts_with('$') {
            return Err(Error::new(
                span,
                format!("field name `{name}` must not start with `$`"),
            ));
        }
        if fields.iter().any(|g| g.name == name) {
            return Err(Error::new(span, format!("duplicate field name `{name}`")));
        }
        fields.push(SchemaField {
            ident,
            ty: &f.ty,
            role: attrs.role,
            name,
        });
    }
    if fields.len() > MAX_FIELDS {
        return Err(Error::new(
            input.ident.span(),
            format!(
                "`{}` has {} schema fields; at most {MAX_FIELDS} are allowed",
                input.ident,
                fields.len()
            ),
        ));
    }

    let res = &input.ident;
    let schema_name = unraw(res);
    let mut consts = Vec::new();
    let mut defs = Vec::new();
    let mut value_arms = Vec::new();
    let mut relation_arms = Vec::new();
    let mut asserts = Vec::new();
    for (i, f) in fields.iter().enumerate() {
        let idx = Literal::usize_unsuffixed(i);
        let konst = format_ident!("{}", unraw(f.ident).to_uppercase(), span = f.ident.span());
        let (ident, ty, name) = (f.ident, f.ty, &f.name);
        let span = ty.span();
        let doc = format!("Handle to the `{name}` field (index {i}).");
        match f.role {
            Role::Scalar => {
                consts.push(quote! {
                    #[doc = #doc]
                    pub const #konst: ::mandate::Field<#res, #ty> = ::mandate::Field::new(#idx);
                });
                defs.push(quote_spanned! {span=>
                    ::mandate::FieldDef::scalar(
                        #name,
                        <<#ty as ::mandate::Scalar>::Inner as ::mandate::ScalarValue>::KIND,
                        <<#ty as ::mandate::Scalar>::Nullability as ::mandate::Nullability>::NULLABLE,
                    )
                });
                let read =
                    quote_spanned! {span=> <#ty as ::mandate::Scalar>::value_ref(&self.#ident) };
                value_arms.push(match &load_state {
                    Some(ls) => quote! {
                        #idx => if ::mandate::LoadState::scalar_loaded(&self.#ls, #name) {
                            #read
                        } else {
                            ::mandate::ValueRef::NotLoaded
                        }
                    },
                    None => quote! { #idx => #read },
                });
                asserts.push(quote_spanned! {span=>
                    const _: () = {
                        fn __assert<T: ::mandate::Scalar>() {}
                        fn __check() {
                            __assert::<#ty>();
                        }
                    };
                });
            }
            Role::Relation => {
                consts.push(quote! {
                    #[doc = #doc]
                    pub const #konst: ::mandate::Rel<#res, #ty> = ::mandate::Rel::new(#idx);
                });
                defs.push(quote_spanned! {span=>
                    ::mandate::FieldDef::relation(
                        #name,
                        <<#ty as ::mandate::RelationSlot>::Target as ::mandate::Resource>::schema,
                        <<#ty as ::mandate::RelationSlot>::Cardinality as ::mandate::Cardinality>::KIND,
                        <<#ty as ::mandate::RelationSlot>::Nullability as ::mandate::Nullability>::NULLABLE,
                    )
                });
                relation_arms.push(quote_spanned! {span=>
                    #idx => <#ty as ::mandate::RelationSlot>::get(&self.#ident)
                });
            }
            Role::Opaque => {
                consts.push(quote! {
                    #[doc = #doc]
                    pub const #konst: ::mandate::Opaque<#res> = ::mandate::Opaque::new(#idx);
                });
                defs.push(quote! { ::mandate::FieldDef::opaque(#name) });
            }
            Role::Skip => {}
        }
    }

    Ok(quote! {
        #(#asserts)*

        impl #res {
            #(#consts)*
        }

        #[automatically_derived]
        impl ::mandate::Resource for #res {
            fn schema() -> &'static ::mandate::Schema {
                static SCHEMA: ::mandate::Schema =
                    ::mandate::Schema::new(#schema_name, &[#(#defs),*]);
                &SCHEMA
            }
            fn as_dyn(&self) -> &dyn ::mandate::DynResource {
                self
            }
        }

        #[automatically_derived]
        impl ::mandate::DynResource for #res {
            fn resource_schema(&self) -> &'static ::mandate::Schema {
                <Self as ::mandate::Resource>::schema()
            }
            fn value(&self, field: ::mandate::FieldIdx) -> ::mandate::ValueRef<'_> {
                match field.0 {
                    #(#value_arms,)*
                    _ => ::mandate::ValueRef::Null,
                }
            }
            fn relation(&self, field: ::mandate::FieldIdx) -> ::mandate::RelationRef<'_> {
                match field.0 {
                    #(#relation_arms,)*
                    _ => ::mandate::RelationRef::Absent,
                }
            }
        }

        #[automatically_derived]
        impl ::mandate::RelationSlot for #res {
            type Target = Self;
            type Cardinality = ::mandate::ToOne;
            type Nullability = ::mandate::NonNull;
            fn get(&self) -> ::mandate::RelationRef<'_> {
                ::mandate::RelationRef::One(self)
            }
        }

        #[automatically_derived]
        impl ::mandate::ResourcePtr for #res {
            type Target = Self;
            fn target(&self) -> &Self {
                self
            }
        }
    })
}
