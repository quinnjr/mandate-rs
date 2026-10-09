#![cfg(all(feature = "derive", feature = "chrono", feature = "uuid"))]

use mandate::{OneOrMany, RuleTemplate, TemplateValue};

#[test]
fn duplicate_keys_rejected() {
    let e = serde_json::from_str::<RuleTemplate>(
        r#"{"action":"read","subject":"Post","conditions":{"status":"draft","status":"archived"}}"#,
    )
    .unwrap_err();
    assert!(e.to_string().contains("duplicate key `status`"));
    let e = serde_json::from_str::<RuleTemplate>(
        r#"{"action":"read","subject":"Post","conditions":{"a":{"x":1,"x":2}}}"#,
    )
    .unwrap_err();
    assert!(e.to_string().contains("duplicate key `x`"));
    // The same key at different levels is fine.
    serde_json::from_str::<RuleTemplate>(
        r#"{"action":"read","subject":"Post","conditions":{"a":{"a":1}}}"#,
    )
    .unwrap();
}

#[test]
fn key_order_preserved_and_round_trips() {
    let s = r#"{"action":"read","subject":"Post","conditions":{"b":1,"a":{"$ne":2}}}"#;
    let t: RuleTemplate = serde_json::from_str(s).unwrap();
    assert_eq!(serde_json::to_string(&t).unwrap(), s);
}

#[test]
fn defaults_and_unknown_fields() {
    let t: RuleTemplate =
        serde_json::from_str(r#"{"action":["read","update"],"subject":"Post"}"#).unwrap();
    assert!(!t.inverted);
    assert_eq!(
        t.action,
        OneOrMany::Many(vec!["read".into(), "update".into()])
    );
    assert_eq!(t.subject, OneOrMany::One("Post".into()));
    assert!(t.conditions.is_none() && t.fields.is_none() && t.reason.is_none());
    assert!(
        serde_json::from_str::<RuleTemplate>(r#"{"action":"a","subject":"b","bogus":1}"#).is_err()
    );
}

#[test]
fn all_value_kinds_round_trip() {
    let s = r#"{"action":"a","subject":"b","conditions":{"n":null,"t":true,"i":-3,"u":18446744073709551615,"f":1.5,"s":"x","l":[1,"y",{"k":[]}]},"fields":["a","b"],"inverted":true,"reason":"no"}"#;
    let t: RuleTemplate = serde_json::from_str(s).unwrap();
    assert_eq!(serde_json::to_string(&t).unwrap(), s);
    assert!(matches!(t.conditions, Some(TemplateValue::Object(_))));
}
