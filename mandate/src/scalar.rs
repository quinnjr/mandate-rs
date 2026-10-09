//! Scalar traits connecting Rust field types to [`Kind`] and [`ValueRef`].

use std::borrow::Cow;

use crate::schema::Kind;
use crate::value::{Value, ValueRef};

#[cfg(feature = "chrono")]
use chrono::{DateTime, NaiveDate, Utc};

/// Type-level nullability marker.
pub trait Nullability {
    /// Whether the field may be null.
    const NULLABLE: bool;
}

/// Marker: the field is never null.
pub enum NonNull {}

/// Marker: the field may be null.
pub enum Nullable {}

impl Nullability for NonNull {
    const NULLABLE: bool = false;
}

impl Nullability for Nullable {
    const NULLABLE: bool = true;
}

/// A non-null scalar type with a fixed [`Kind`].
pub trait ScalarValue {
    /// The scalar kind.
    const KIND: Kind;
    /// Converts to an owned [`Value`].
    fn to_value(&self) -> Value;
    /// Borrows as a [`ValueRef`].
    fn value_ref(&self) -> ValueRef<'_>;
}

/// A field type: a scalar, possibly optional.
pub trait Scalar {
    /// The underlying non-null scalar.
    type Inner: ScalarValue;
    /// Whether the field may be null.
    type Nullability: Nullability;
    /// Borrows as a [`ValueRef`] (`Null` for `None`).
    fn value_ref(&self) -> ValueRef<'_>;
}

/// Scalars supporting ordering comparisons.
pub trait Ordered: ScalarValue {}

/// Scalars supporting text operators.
pub trait Textual: ScalarValue {}

/// Implemented by string-backed enums to expose their variant names.
pub trait IntoValue {
    /// All variant names.
    const VARIANTS: &'static [&'static str];
    /// Name of this variant.
    fn variant_name(&self) -> &'static str;
}

macro_rules! scalar_impl {
    ($t:ty, $kind:expr, $variant:ident, |$s:ident| $conv:expr, $vr:ident) => {
        impl ScalarValue for $t {
            const KIND: Kind = $kind;
            fn to_value(&self) -> Value {
                let $s = self;
                Value::$variant($conv)
            }
            fn value_ref(&self) -> ValueRef<'_> {
                let $s = self;
                ValueRef::$vr($conv)
            }
        }
        impl Scalar for $t {
            type Inner = $t;
            type Nullability = NonNull;
            fn value_ref(&self) -> ValueRef<'_> {
                <$t as ScalarValue>::value_ref(self)
            }
        }
    };
}

macro_rules! int_impl {
    ($($t:ty),*) => {$(
        scalar_impl!($t, Kind::Int, Int, |s| i64::from(*s), Int);
        impl Ordered for $t {}
    )*};
}

int_impl!(i8, i16, i32, i64, u8, u16, u32);

scalar_impl!(bool, Kind::Bool, Bool, |s| *s, Bool);
scalar_impl!(f32, Kind::Float, Float, |s| f64::from(*s), Float);
scalar_impl!(f64, Kind::Float, Float, |s| *s, Float);
impl Ordered for f32 {}
impl Ordered for f64 {}

impl ScalarValue for String {
    const KIND: Kind = Kind::String;
    fn to_value(&self) -> Value {
        Value::String(self.clone())
    }
    fn value_ref(&self) -> ValueRef<'_> {
        ValueRef::Str(self)
    }
}
impl Scalar for String {
    type Inner = String;
    type Nullability = NonNull;
    fn value_ref(&self) -> ValueRef<'_> {
        ValueRef::Str(self)
    }
}
impl Textual for String {}

impl Scalar for &'static str {
    type Inner = String;
    type Nullability = NonNull;
    fn value_ref(&self) -> ValueRef<'_> {
        ValueRef::Str(self)
    }
}

impl Scalar for Cow<'static, str> {
    type Inner = String;
    type Nullability = NonNull;
    fn value_ref(&self) -> ValueRef<'_> {
        ValueRef::Str(self)
    }
}

#[cfg(feature = "uuid")]
scalar_impl!(uuid::Uuid, Kind::Uuid, Uuid, |s| *s, Uuid);

#[cfg(feature = "chrono")]
scalar_impl!(DateTime<Utc>, Kind::DateTime, DateTime, |s| *s, DateTime);
#[cfg(feature = "chrono")]
scalar_impl!(NaiveDate, Kind::Date, Date, |s| *s, Date);
#[cfg(feature = "chrono")]
impl Ordered for DateTime<Utc> {}
#[cfg(feature = "chrono")]
impl Ordered for NaiveDate {}

impl<T: Scalar<Nullability = NonNull>> Scalar for Option<T> {
    type Inner = T::Inner;
    type Nullability = Nullable;
    fn value_ref(&self) -> ValueRef<'_> {
        match self {
            Some(v) => Scalar::value_ref(v),
            None => ValueRef::Null,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scalar_kinds_and_nullability() {
        assert_eq!(<i32 as ScalarValue>::KIND, Kind::Int);
        assert_eq!(<f32 as ScalarValue>::KIND, Kind::Float);
        assert_eq!(<String as ScalarValue>::KIND, Kind::String);
        const _: () = assert!(<<Option<i64> as Scalar>::Nullability as Nullability>::NULLABLE);
        const _: () = assert!(!<<i64 as Scalar>::Nullability as Nullability>::NULLABLE);
    }

    #[test]
    fn value_refs() {
        assert_eq!(Scalar::value_ref(&None::<i64>), ValueRef::Null);
        assert_eq!(Scalar::value_ref(&Some(5i64)), ValueRef::Int(5));
        assert_eq!(Scalar::value_ref(&"x"), ValueRef::Str("x"));
        assert_eq!(ScalarValue::to_value(&u32::MAX), Value::Int(4_294_967_295));
    }
}
