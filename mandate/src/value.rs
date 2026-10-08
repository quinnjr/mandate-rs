//! Dynamic values and borrowed value references.

use serde::Serialize;

use crate::schema::Kind;

#[cfg(feature = "chrono")]
use chrono::{DateTime, NaiveDate, Timelike, Utc};

/// An owned dynamic scalar value.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(untagged)]
#[non_exhaustive]
pub enum Value {
    /// Boolean.
    Bool(bool),
    /// Signed integer.
    Int(i64),
    /// Float.
    Float(f64),
    /// String (also used for enum variants).
    String(String),
    /// UUID.
    #[cfg(feature = "uuid")]
    Uuid(uuid::Uuid),
    /// UTC timestamp.
    #[cfg(feature = "chrono")]
    DateTime(DateTime<Utc>),
    /// Calendar date.
    #[cfg(feature = "chrono")]
    Date(NaiveDate),
}

impl Value {
    /// Returns `false` only for non-finite floats.
    pub fn is_finite(&self) -> bool {
        match self {
            Value::Float(f) => f.is_finite(),
            _ => true,
        }
    }

    /// Whether this value is acceptable for a field of kind `kind`.
    ///
    /// Strings match both [`Kind::String`] and [`Kind::Enum`].
    pub fn kind_matches(&self, kind: Kind) -> bool {
        match self {
            Value::Bool(_) => kind == Kind::Bool,
            Value::Int(_) => kind == Kind::Int,
            Value::Float(_) => kind == Kind::Float,
            Value::String(_) => matches!(kind, Kind::String | Kind::Enum(_)),
            #[cfg(feature = "uuid")]
            Value::Uuid(_) => kind == Kind::Uuid,
            #[cfg(feature = "chrono")]
            Value::DateTime(_) => kind == Kind::DateTime,
            #[cfg(feature = "chrono")]
            Value::Date(_) => kind == Kind::Date,
        }
    }
}

/// A borrowed view of a field value read from an entity.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum ValueRef<'a> {
    /// Null.
    Null,
    /// The field is not loaded.
    NotLoaded,
    /// Boolean.
    Bool(bool),
    /// Signed integer.
    Int(i64),
    /// Float.
    Float(f64),
    /// Borrowed string.
    Str(&'a str),
    /// UUID.
    #[cfg(feature = "uuid")]
    Uuid(uuid::Uuid),
    /// UTC timestamp.
    #[cfg(feature = "chrono")]
    DateTime(DateTime<Utc>),
    /// Calendar date.
    #[cfg(feature = "chrono")]
    Date(NaiveDate),
}

impl ValueRef<'_> {
    /// Whether this is a value of a field of kind `kind`.
    ///
    /// `Str` matches both [`Kind::String`] and [`Kind::Enum`]; `Null` and
    /// `NotLoaded` match no kind.
    pub fn kind_matches(&self, kind: Kind) -> bool {
        match self {
            ValueRef::Null | ValueRef::NotLoaded => false,
            ValueRef::Bool(_) => kind == Kind::Bool,
            ValueRef::Int(_) => kind == Kind::Int,
            ValueRef::Float(_) => kind == Kind::Float,
            ValueRef::Str(_) => matches!(kind, Kind::String | Kind::Enum(_)),
            #[cfg(feature = "uuid")]
            ValueRef::Uuid(_) => kind == Kind::Uuid,
            #[cfg(feature = "chrono")]
            ValueRef::DateTime(_) => kind == Kind::DateTime,
            #[cfg(feature = "chrono")]
            ValueRef::Date(_) => kind == Kind::Date,
        }
    }
}

/// Truncates a timestamp to microsecond precision.
#[cfg(feature = "chrono")]
pub fn truncate_micros(dt: DateTime<Utc>) -> DateTime<Utc> {
    let nanos = dt.nanosecond() / 1000 * 1000;
    dt.with_nanosecond(nanos).unwrap_or(dt)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn non_finite_values() {
        assert!(!Value::Float(f64::NAN).is_finite());
        assert!(!Value::Float(f64::INFINITY).is_finite());
        assert!(Value::Float(1.0).is_finite() && Value::Int(1).is_finite());
    }

    #[test]
    fn strings_match_string_and_enum_kinds() {
        let v = Value::String("a".into());
        assert!(v.kind_matches(Kind::String));
        assert!(v.kind_matches(Kind::Enum(&["a"])));
        assert!(!v.kind_matches(Kind::Int));
    }

    #[test]
    fn value_refs_match_their_kind() {
        let s = ValueRef::Str("a");
        assert!(s.kind_matches(Kind::String));
        assert!(s.kind_matches(Kind::Enum(&["a"])));
        assert!(!s.kind_matches(Kind::Int));
        assert!(ValueRef::Bool(true).kind_matches(Kind::Bool));
        assert!(ValueRef::Int(1).kind_matches(Kind::Int));
        assert!(!ValueRef::Int(1).kind_matches(Kind::Float));
        assert!(ValueRef::Float(1.0).kind_matches(Kind::Float));
        assert!(!ValueRef::Float(1.0).kind_matches(Kind::Int));
        for kind in [Kind::Bool, Kind::Int, Kind::String, Kind::Enum(&[])] {
            assert!(!ValueRef::Null.kind_matches(kind), "{kind:?}");
            assert!(!ValueRef::NotLoaded.kind_matches(kind), "{kind:?}");
        }
    }

    #[cfg(feature = "uuid")]
    #[test]
    fn uuid_value_refs_match_their_kind() {
        let u = ValueRef::Uuid(uuid::Uuid::nil());
        assert!(u.kind_matches(Kind::Uuid));
        assert!(!u.kind_matches(Kind::String));
    }

    #[cfg(feature = "chrono")]
    #[test]
    fn chrono_value_refs_match_their_kind() {
        let dt = ValueRef::DateTime(DateTime::<Utc>::UNIX_EPOCH);
        assert!(dt.kind_matches(Kind::DateTime));
        assert!(!dt.kind_matches(Kind::Date));
        let d = ValueRef::Date(NaiveDate::default());
        assert!(d.kind_matches(Kind::Date));
        assert!(!d.kind_matches(Kind::DateTime));
    }

    #[cfg(feature = "chrono")]
    #[test]
    fn truncate_micros_drops_nanos() {
        let dt: DateTime<Utc> = "2020-01-15T10:30:00.123456789Z".parse().unwrap();
        assert_eq!(
            truncate_micros(dt).to_rfc3339(),
            "2020-01-15T10:30:00.123456+00:00"
        );
    }
}
