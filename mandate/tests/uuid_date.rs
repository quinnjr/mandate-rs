#![cfg(all(feature = "derive", feature = "chrono", feature = "uuid"))]
//! End-to-end coverage of the `Uuid` and `Date` kinds: stored templates
//! compile their text forms (spec §6.2: a hyphenated UUID, a `YYYY-MM-DD`
//! date) and bind context values by the same rules, and `can`, `check` and
//! the filter plan evaluate them, null dates included (§5.3).
//!
//! Serialization round trips are in `rule_serialize.rs`; the generated
//! cases in `properties.rs` cover both kinds too.

mod common;
use chrono::NaiveDate;
use common::assert_matches;
use common::fixture::*;
use common::templates::{
    assert_invalid, assert_mismatch, assert_not_allowed, cond, cond_err, not_date, not_uuid,
};
use mandate::{
    Ability, Access, BindError, BindErrorKind, CheckError, CmpOp, Cond, Condition, Context, Kind,
    LoadErrorKind, RuleTemplate, Templates, Value,
};
use serde::Serialize;
use serde_json::json;
use uuid::Uuid;

type T = Templates<Action, Subject>;
type Ab = Ability<Action, Subject>;

/// [`OWNER`] in the hyphenated form.
const OWNER_TEXT: &str = "67e55044-10b1-426f-9247-bb680e5fe0c8";

/// Another owner, and its hyphenated form.
const OTHER: Uuid = Uuid::from_u128(0x0123_4567_89ab_cdef_0123_4567_89ab_cdef);
const OTHER_TEXT: &str = "01234567-89ab-cdef-0123-456789abcdef";

fn date(s: &str) -> NaiveDate {
    s.parse().expect("a valid date")
}

/// The forms of [`OWNER`] that templates and contexts reject (§6.2):
/// simple (32 hex digits), braced, and URN.
fn other_forms() -> [String; 3] {
    [
        OWNER.simple().to_string(),
        OWNER.braced().to_string(),
        OWNER.urn().to_string(),
    ]
}

/// The condition `can read Post when <c>` binds to, with `user` as the
/// `user` root. `c` must compile and every placeholder must resolve.
fn bound(c: &str, user: &impl Serialize) -> Result<Condition, BindError> {
    let ctx = Context::new().with("user", user).expect("serializable");
    let b = cond(c).unwrap_or_else(|e| panic!("{c}: {e}")).bind(&ctx)?;
    assert!(b.unresolved().is_empty(), "{c}: {:?}", b.unresolved());
    Ok(b.rules()[0].condition().expect("a condition").clone())
}

fn cmp(field: mandate::FieldIdx, op: CmpOp, value: Value) -> Condition {
    Condition::Cmp { field, op, value }
}

#[test]
fn uuid_literals_are_hyphenated() {
    let owner = Post::OWNER.idx();
    let none = json!({});
    assert_eq!(
        bound(&format!(r#"{{"owner":"{OWNER_TEXT}"}}"#), &none),
        Ok(cmp(owner, CmpOp::Eq, Value::Uuid(OWNER)))
    );
    // Hex digits are case-insensitive; the form is still the hyphenated one.
    let upper = OWNER_TEXT.to_uppercase();
    assert_eq!(
        bound(&format!(r#"{{"owner":"{upper}"}}"#), &none),
        Ok(cmp(owner, CmpOp::Eq, Value::Uuid(OWNER)))
    );
    assert_eq!(
        bound(
            &format!(r#"{{"owner":{{"$ne":"{OWNER_TEXT}","$in":["{OTHER_TEXT}"]}}}}"#),
            &none
        ),
        Ok(Condition::And(vec![
            cmp(owner, CmpOp::Ne, Value::Uuid(OWNER)),
            Condition::In {
                field: owner,
                values: vec![Value::Uuid(OTHER)],
            },
        ]))
    );

    // The simple, braced and URN forms are rejected, as literals, operands
    // and list elements.
    for text in other_forms() {
        let reason = not_uuid(&text);
        let is = |r: &str| r == reason;
        assert_invalid(&format!(r#"{{"owner":"{text}"}}"#), "conditions.owner", is);
        assert_invalid(
            &format!(r#"{{"owner":{{"$ne":"{text}"}}}}"#),
            "conditions.owner.$ne",
            is,
        );
        assert_invalid(
            &format!(r#"{{"owner":{{"$nin":["{OWNER_TEXT}","{text}"]}}}}"#),
            "conditions.owner.$nin[1]",
            is,
        );
    }
    assert_invalid(r#"{"owner":"not-a-uuid"}"#, "conditions.owner", |r| {
        r == not_uuid("not-a-uuid")
    });

    // Not a string; no ordering or text operators; never null.
    assert_mismatch(r#"{"owner":1}"#, "conditions.owner", Kind::Uuid, "1");
    for op in ["$lt", "$lte", "$gt", "$gte", "$contains", "$startsWith"] {
        assert_not_allowed(
            &format!(r#"{{"owner":{{"{op}":"{OWNER_TEXT}"}}}}"#),
            &format!("conditions.owner.{op}"),
            op,
            Kind::Uuid,
        );
    }
    let e = cond_err(r#"{"owner":null}"#);
    assert_eq!(
        (e.kind, e.path.as_str()),
        (LoadErrorKind::NullNotAllowed, "conditions.owner")
    );
}

#[test]
fn uuid_placeholders_bind() {
    #[derive(Serialize)]
    struct User {
        id: Uuid,
        teams: Vec<Uuid>,
    }
    // A `Uuid` serializes to its hyphenated form, which binds back to it.
    let user = User {
        id: OWNER,
        teams: vec![OTHER, OWNER],
    };
    let owner = Post::OWNER.idx();
    assert_eq!(
        bound(r#"{"owner":"${user.id}"}"#, &user),
        Ok(cmp(owner, CmpOp::Eq, Value::Uuid(OWNER)))
    );
    assert_eq!(
        bound(r#"{"owner":{"$ne":"${user.id}"}}"#, &user),
        Ok(cmp(owner, CmpOp::Ne, Value::Uuid(OWNER)))
    );
    assert_eq!(
        bound(r#"{"owner":{"$in":"${user.teams}"}}"#, &user),
        Ok(Condition::In {
            field: owner,
            values: vec![Value::Uuid(OTHER), Value::Uuid(OWNER)],
        })
    );
    assert_eq!(
        bound(r#"{"owner":"${user.id}"}"#, &json!({"id": OTHER_TEXT})),
        Ok(cmp(owner, CmpOp::Eq, Value::Uuid(OTHER)))
    );

    // Context strings follow the literal rules.
    for text in other_forms() {
        let e = bound(r#"{"owner":"${user.id}"}"#, &json!({"id": text})).unwrap_err();
        assert!(
            matches!(
                &e,
                BindError { rule_index: 0, path, kind: BindErrorKind::InvalidValue(r), .. }
                    if path == "conditions.owner" && *r == not_uuid(&text)
            ),
            "{text}: {e:?}"
        );
        let e = bound(
            r#"{"owner":{"$in":"${user.teams}"}}"#,
            &json!({"teams": [OWNER_TEXT, text]}),
        )
        .unwrap_err();
        assert!(
            matches!(
                &e,
                BindError { rule_index: 0, path, kind: BindErrorKind::InvalidValue(r), .. }
                    if path == "conditions.owner.$in[1]" && *r == not_uuid(&text)
            ),
            "{text}: {e:?}"
        );
    }
    let e = bound(r#"{"owner":"${user.id}"}"#, &json!({"id": 7})).unwrap_err();
    assert!(
        matches!(
            &e,
            BindError {
                rule_index: 0,
                path,
                kind: BindErrorKind::TypeMismatch { expected: Kind::Uuid, found, .. },
                ..
            } if path == "conditions.owner" && found == "7"
        ),
        "{e:?}"
    );
}

#[test]
fn date_literals_are_yyyy_mm_dd() {
    let due = Post::DUE.idx();
    let none = json!({});
    assert_eq!(
        bound(r#"{"due":{"$lt":"2026-01-01"}}"#, &none),
        Ok(cmp(due, CmpOp::Lt, Value::Date(date("2026-01-01"))))
    );
    assert_eq!(
        bound(r#"{"due":"2024-02-29"}"#, &none),
        Ok(cmp(due, CmpOp::Eq, Value::Date(date("2024-02-29"))))
    );
    assert_eq!(
        bound(r#"{"due":{"$nin":["2024-02-29","2025-01-01"]}}"#, &none),
        Ok(Condition::NotIn {
            field: due,
            values: vec![
                Value::Date(date("2024-02-29")),
                Value::Date(date("2025-01-01"))
            ],
        })
    );
    // `due` is nullable: `null` is a null test.
    assert_eq!(bound(r#"{"due":null}"#, &none), Ok(Condition::IsNull(due)));
    assert_eq!(
        bound(r#"{"due":{"$eq":null}}"#, &none),
        Ok(Condition::IsNull(due))
    );
    assert_eq!(
        bound(r#"{"due":{"$ne":null}}"#, &none),
        Ok(Condition::IsNotNull(due))
    );
    assert_eq!(
        bound(r#"{"due":{"$isNull":false}}"#, &none),
        Ok(Condition::IsNotNull(due))
    );

    // Strings that are not dates, as literals, operands and list elements.
    for text in [
        "2026-13-45",
        "2023-02-29",
        "yesterday",
        "",
        "2026-01-01T00:00:00Z",
    ] {
        let is = |r: &str| r.starts_with(&not_date(text));
        assert_invalid(&format!(r#"{{"due":"{text}"}}"#), "conditions.due", is);
        assert_invalid(
            &format!(r#"{{"due":{{"$gte":"{text}"}}}}"#),
            "conditions.due.$gte",
            is,
        );
        assert_invalid(
            &format!(r#"{{"due":{{"$in":["2026-01-01","{text}"]}}}}"#),
            "conditions.due.$in[1]",
            is,
        );
    }

    // Not a string; no text operators.
    assert_mismatch(
        r#"{"due":20260101}"#,
        "conditions.due",
        Kind::Date,
        "20260101",
    );
    for op in ["$contains", "$startsWith", "$endsWith"] {
        assert_not_allowed(
            &format!(r#"{{"due":{{"{op}":"2026"}}}}"#),
            &format!("conditions.due.{op}"),
            op,
            Kind::Date,
        );
    }
}

#[test]
fn date_placeholders_bind() {
    #[derive(Serialize)]
    struct User {
        since: NaiveDate,
        holidays: Vec<NaiveDate>,
    }
    // A `NaiveDate` serializes to `YYYY-MM-DD`, which binds back to it.
    let user = User {
        since: date("2026-01-01"),
        holidays: vec![date("2025-12-25"), date("2026-01-01")],
    };
    let due = Post::DUE.idx();
    assert_eq!(
        bound(r#"{"due":{"$gte":"${user.since}"}}"#, &user),
        Ok(cmp(due, CmpOp::Gte, Value::Date(date("2026-01-01"))))
    );
    assert_eq!(
        bound(r#"{"due":{"$nin":"${user.holidays}"}}"#, &user),
        Ok(Condition::NotIn {
            field: due,
            values: vec![
                Value::Date(date("2025-12-25")),
                Value::Date(date("2026-01-01"))
            ],
        })
    );

    // Context strings follow the literal rules.
    for text in ["2026-13-45", "yesterday", "2026-01-01T00:00:00Z"] {
        let e = bound(
            r#"{"due":{"$gte":"${user.since}"}}"#,
            &json!({"since": text}),
        )
        .unwrap_err();
        assert!(
            matches!(
                &e,
                BindError { rule_index: 0, path, kind: BindErrorKind::InvalidValue(r), .. }
                    if path == "conditions.due.$gte" && r.starts_with(&not_date(text))
            ),
            "{text}: {e:?}"
        );
    }
    let e = bound(r#"{"due":"${user.since}"}"#, &json!({"since": 20260101})).unwrap_err();
    assert!(
        matches!(
            &e,
            BindError {
                rule_index: 0,
                path,
                kind: BindErrorKind::TypeMismatch { expected: Kind::Date, found, .. },
                ..
            } if path == "conditions.due" && found == "20260101"
        ),
        "{e:?}"
    );
}

/// Whether `can read Post when c` allows reading `p`, after checking that
/// `check` and the filter plan give the same answer.
#[track_caller]
fn allows(c: &Cond<Post>, p: &Post) -> bool {
    let a = Ab::builder()
        .can(Action::Read, Subject::Post)
        .when(c.clone())
        .build()
        .expect("valid condition");
    let can = a.can(Action::Read, p);
    match a.check(Action::Read, p) {
        Ok(()) => assert!(can, "{c:?}"),
        Err(CheckError::Forbidden(_)) => assert!(!can, "{c:?}"),
        Err(e) => panic!("{c:?}: {e}"),
    }
    let planned = match a.access::<Post>(Action::Read).expect("Post's schema") {
        Access::Denied => false,
        Access::All => true,
        Access::Filter(plan) => plan.eval(p).expect("fully loaded"),
    };
    assert_eq!(planned, can, "{c:?}: the plan disagrees with `can`");
    can
}

#[test]
fn uuid_conditions_evaluate() {
    let mine = post();
    assert_eq!(mine.owner, OWNER);
    let theirs = Post {
        owner: OTHER,
        ..post()
    };
    for (c, on_mine, on_theirs) in [
        (Post::OWNER.eq(OWNER), true, false),
        (Post::OWNER.eq(Uuid::nil()), false, false),
        (Post::OWNER.ne(OWNER), false, true),
        (Post::OWNER.is_in([OTHER, OWNER]), true, true),
        (Post::OWNER.is_in([OTHER]), false, true),
        (Post::OWNER.is_in(Vec::<Uuid>::new()), false, false),
        (Post::OWNER.not_in([OWNER]), false, true),
        (Post::OWNER.not_in([OWNER, OTHER]), false, false),
        (!Post::OWNER.eq(OWNER), false, true),
    ] {
        assert_eq!(allows(&c, &mine), on_mine, "{c:?} on mine");
        assert_eq!(allows(&c, &theirs), on_theirs, "{c:?} on theirs");
    }
}

#[test]
fn date_conditions_evaluate() {
    let (dec31, jan1) = (date("2025-12-31"), date("2026-01-01"));
    let due = |d: Option<NaiveDate>| Post { due: d, ..post() };
    for (c, on_dec31, on_jan1, on_none) in [
        (Post::DUE.lt(jan1), true, false, false),
        (Post::DUE.lte(jan1), true, true, false),
        (Post::DUE.gt(dec31), false, true, false),
        (Post::DUE.gte(jan1), false, true, false),
        (Post::DUE.eq(jan1), false, true, false),
        // §5.3: on a null value only `Ne`, `NotIn` and `IsNull` hold.
        (Post::DUE.ne(jan1), true, false, true),
        (Post::DUE.is_in([dec31]), true, false, false),
        (Post::DUE.not_in([dec31]), false, true, true),
        (Post::DUE.is_null(), false, false, true),
        (Post::DUE.is_not_null(), true, true, false),
        // Two-valued logic: negation flips the null case too.
        (!Post::DUE.lt(jan1), false, true, true),
        (!Post::DUE.gte(jan1), true, false, true),
        (
            Post::DUE.gte(dec31).and(Post::DUE.lt(jan1)),
            true,
            false,
            false,
        ),
    ] {
        assert_eq!(
            allows(&c, &due(Some(dec31))),
            on_dec31,
            "{c:?} on 2025-12-31"
        );
        assert_eq!(allows(&c, &due(Some(jan1))), on_jan1, "{c:?} on 2026-01-01");
        assert_eq!(allows(&c, &due(None)), on_none, "{c:?} on no date");
    }
}

/// A stored policy on both kinds, bound from typed context values, decides
/// checks end to end.
#[test]
fn stored_templates_decide_on_uuid_and_date() {
    let raw: Vec<RuleTemplate> = serde_json::from_str(
        r#"[
          {"action":"update","subject":"Post","conditions":{"owner":"${user.id}"}},
          {"action":"update","subject":"Post","inverted":true,
           "conditions":{"due":{"$lt":"${user.today}"}},"reason":"overdue"}
        ]"#,
    )
    .unwrap();
    #[derive(Serialize)]
    struct User {
        id: Uuid,
        today: NaiveDate,
    }
    let ctx = Context::new()
        .with(
            "user",
            &User {
                id: OWNER,
                today: date("2026-10-08"),
            },
        )
        .unwrap();
    let bound = T::compile(&raw, &["user"]).unwrap().bind(&ctx).unwrap();
    assert!(bound.unresolved().is_empty());
    let a = Ab::builder().extend(bound).build().unwrap();

    // Owned and without a due date, or due today: allowed.
    assert!(a.can(Action::Update, &post()));
    let today = Post {
        due: Some(date("2026-10-08")),
        ..post()
    };
    assert!(a.can(Action::Update, &today));
    // Someone else's: not allowed.
    let theirs = Post {
        owner: OTHER,
        ..post()
    };
    assert_matches!(
        a.check(Action::Update, &theirs),
        Err(CheckError::Forbidden(f)) if f.reason.is_none()
    );
    // Overdue: denied with the reason.
    let overdue = Post {
        due: Some(date("2026-10-07")),
        ..post()
    };
    assert_matches!(
        a.check(Action::Update, &overdue),
        Err(CheckError::Forbidden(f)) if f.reason.as_deref() == Some("overdue")
    );
    // The filter plan agrees.
    let Access::Filter(plan) = a.access::<Post>(Action::Update).unwrap() else {
        panic!("expected a filter plan")
    };
    for (p, expected) in [
        (&post(), true),
        (&today, true),
        (&theirs, false),
        (&overdue, false),
    ] {
        assert_eq!(plan.eval(p), Ok(expected), "{p:?}");
    }
}
