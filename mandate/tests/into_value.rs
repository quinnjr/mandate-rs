#![cfg(all(feature = "derive", feature = "chrono", feature = "uuid"))]
use mandate::{IntoValue, Kind, Nullability, Scalar, ScalarValue, ValueRef};

#[derive(IntoValue, serde::Serialize, Clone, Copy)]
#[serde(rename_all = "lowercase")]
enum Status {
    Draft,
    Published,
    Archived,
}
#[allow(dead_code)]
#[derive(IntoValue, serde::Serialize)]
#[serde(rename_all = "snake_case")]
enum Snake {
    FooBar,
    #[serde(rename = "q")]
    Quux,
}
#[allow(dead_code)]
#[derive(IntoValue)]
enum Plain {
    FooBar,
    #[value(rename = "x")]
    Baz,
}

#[test]
fn variants_follow_serde_naming() {
    assert_eq!(Status::VARIANTS, &["draft", "published", "archived"]);
    assert_eq!(Snake::VARIANTS, &["foo_bar", "q"]);
    assert_eq!(Plain::VARIANTS, &["FooBar", "x"]);
}
#[test]
fn names_match_serde_output() {
    for s in [Status::Draft, Status::Published, Status::Archived] {
        assert_eq!(serde_json::to_value(s).unwrap(), serde_json::json!(s.variant_name()));
    }
}
#[test]
fn enum_is_scalar() {
    assert_eq!(<Status as ScalarValue>::KIND, Kind::Enum(&["draft", "published", "archived"]));
    assert_eq!(Scalar::value_ref(&Status::Draft), ValueRef::Str("draft"));
    assert!(<<Option<Status> as Scalar>::Nullability as Nullability>::NULLABLE);
}
