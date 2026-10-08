#![cfg(all(feature = "derive", feature = "chrono", feature = "uuid"))]
mod common;
use common::fixture::*;
use mandate::{Kind, LoadError, LoadErrorKind, RuleTemplate, Templates};

type T = Templates<Action, Subject>;

const SPEC_EXAMPLE: &str = r#"{
  "action": "update",
  "subject": "Post",
  "conditions": {
    "author_id": "${user.id}",
    "status": { "$ne": "archived" },
    "published_at": null,
    "org": { "id": "${user.org_id}" },
    "reviewer": { "$isNull": false },
    "tags": { "$some": { "name": "rust" } },
    "$or": [ { "status": "published" }, { "author_id": "${user.id}" } ]
  },
  "fields": ["title", "body"],
  "inverted": false,
  "reason": "Authors edit their own posts"
}"#;

fn rule(json: &str) -> RuleTemplate {
    serde_json::from_str(json).unwrap()
}

fn compile(json: &str) -> Result<T, LoadError> {
    T::compile(&[rule(json)], &["user"])
}

/// Compiles `can read Post when <c>`.
fn cond(c: &str) -> Result<T, LoadError> {
    compile(&format!(
        r#"{{"action":"read","subject":"Post","conditions":{c}}}"#
    ))
}

fn err(c: &str) -> LoadErrorKind {
    cond(c).unwrap_err().kind
}

fn mismatch(expected: Kind, found: &str) -> LoadErrorKind {
    LoadErrorKind::TypeMismatch {
        expected,
        found: found.into(),
    }
}

fn not_allowed(op: &str, kind: Kind) -> LoadErrorKind {
    LoadErrorKind::OperatorNotAllowed {
        op: op.into(),
        kind,
    }
}

#[test]
fn compiles_spec_example() {
    compile(SPEC_EXAMPLE).unwrap();
    // The same rule without the context root is rejected up front.
    let e = T::compile(&[rule(SPEC_EXAMPLE)], &[]).unwrap_err();
    assert_eq!(e.kind, LoadErrorKind::UnknownRoot("user".into()));
    assert_eq!(e.path, "conditions.author_id");
}

#[test]
fn load_error_kinds() {
    use LoadErrorKind::*;

    let e = compile(r#"{"action":"updat","subject":"Post"}"#).unwrap_err();
    assert_eq!(
        (e.kind, e.path.as_str()),
        (UnknownAction("updat".into()), "action")
    );
    let e = compile(r#"{"action":["read","updat"],"subject":"Post"}"#).unwrap_err();
    assert_eq!(
        (e.kind, e.path.as_str()),
        (UnknownAction("updat".into()), "action[1]")
    );
    let e = compile(r#"{"action":"read","subject":"Posts"}"#).unwrap_err();
    assert_eq!(
        (e.kind, e.path.as_str()),
        (UnknownSubject("Posts".into()), "subject")
    );

    assert_eq!(err(r#"{"author":1}"#), UnknownField("author".into()));
    assert_eq!(err(r#"{"org":{"nme":"x"}}"#), UnknownField("nme".into()));
    assert_eq!(
        err(r#"{"title":{"$regex":"x"}}"#),
        UnknownOperator("$regex".into())
    );
    assert_eq!(err(r#"{"$xor":[]}"#), UnknownOperator("$xor".into()));
    assert_eq!(err(r#"{"tags":{"$any":{}}}"#), UnknownKey("$any".into()));
    assert_eq!(err(r#"{"tags":{"name":"x"}}"#), UnknownKey("name".into()));
    assert_eq!(
        err(r#"{"author_id":"${org.id}"}"#),
        UnknownRoot("org".into())
    );
    assert_eq!(
        err(r#"{"status":"Publshed"}"#),
        UnknownVariant("Publshed".into())
    );
    assert_eq!(
        err(r#"{"status":{"$in":["draft","Publshed"]}}"#),
        UnknownVariant("Publshed".into())
    );

    assert_eq!(
        err(r#"{"author_id":"seven"}"#),
        mismatch(Kind::Int, "\"seven\"")
    );
    assert_eq!(err(r#"{"locked":1}"#), mismatch(Kind::Bool, "1"));
    assert_eq!(err(r#"{"title":true}"#), mismatch(Kind::String, "true"));
    assert_eq!(err(r#"{"author_id":[1]}"#), mismatch(Kind::Int, "array"));
    assert_eq!(
        err(r#"{"title":{"$contains":5}}"#),
        mismatch(Kind::String, "5")
    );
    assert_eq!(
        err(r#"{"reviewer":{"$isNull":"yes"}}"#),
        mismatch(Kind::Bool, "\"yes\"")
    );

    assert_eq!(
        err(r#"{"title":{"$lt":"a"}}"#),
        not_allowed("$lt", Kind::String)
    );
    assert_eq!(
        err(r#"{"author_id":{"$contains":"1"}}"#),
        not_allowed("$contains", Kind::Int)
    );
    assert_eq!(
        err(r#"{"locked":{"$gte":true}}"#),
        not_allowed("$gte", Kind::Bool)
    );
    assert!(matches!(
        err(r#"{"status":{"$gt":"draft"}}"#),
        OperatorNotAllowed {
            kind: Kind::Enum(_),
            ..
        }
    ));
    assert!(matches!(
        err(r#"{"status":{"$startsWith":"d"}}"#),
        OperatorNotAllowed {
            kind: Kind::Enum(_),
            ..
        }
    ));

    assert_eq!(err(r#"{"author_id":null}"#), NullNotAllowed);
    assert_eq!(err(r#"{"author_id":{"$eq":null}}"#), NullNotAllowed);
    assert_eq!(err(r#"{"author_id":{"$isNull":false}}"#), NullNotAllowed);
    assert_eq!(err(r#"{"reviewer_id":{"$in":[null]}}"#), NullNotAllowed);
    assert_eq!(err(r#"{"reviewer_id":{"$lt":null}}"#), NullNotAllowed);
    assert_eq!(err(r#"{"org":null}"#), NullNotAllowed);
    assert_eq!(err(r#"{"org":{"$isNull":true}}"#), NullNotAllowed);
    assert_eq!(err(r#"{"tags":null}"#), NullNotAllowed);

    assert_eq!(
        err(r#"{"reviewer":{"$isNull":false,"name":"x"}}"#),
        MixedRelationObject
    );
    assert_eq!(
        err(r#"{"reviewer":{"name":"x","$none":{}}}"#),
        MixedRelationObject
    );
    assert_eq!(
        err(r#"{"tags":{"$some":{},"$every":{}}}"#),
        MixedRelationObject
    );

    for subject in [r#"["Post","Org"]"#, r#""All""#, r#""Dashboard""#] {
        let e = compile(&format!(
            r#"{{"action":"read","subject":{subject},"conditions":{{}}}}"#
        ))
        .unwrap_err();
        assert_eq!(
            (e.kind, e.path.as_str()),
            (ConditionsNotAllowed, "conditions")
        );
    }

    let e = compile(r#"{"action":[],"subject":"Post"}"#).unwrap_err();
    assert_eq!((e.kind, e.path.as_str()), (Empty("action"), "action"));
    let e = compile(r#"{"action":"read","subject":[]}"#).unwrap_err();
    assert_eq!((e.kind, e.path.as_str()), (Empty("subject"), "subject"));
    let e = compile(r#"{"action":"read","subject":"Post","fields":[]}"#).unwrap_err();
    assert_eq!((e.kind, e.path.as_str()), (Empty("fields"), "fields"));

    let e = compile(r#"{"action":"read","subject":"Post","fields":["title","nope"]}"#).unwrap_err();
    assert_eq!(
        (e.kind, e.path.as_str()),
        (UnknownField("nope".into()), "fields[1]")
    );
    // Field restrictions need a single resource-bound subject.
    let e = compile(r#"{"action":"read","subject":"Dashboard","fields":["title"]}"#).unwrap_err();
    assert_eq!(
        (e.kind, e.path.as_str()),
        (UnknownField("title".into()), "fields[0]")
    );

    assert!(matches!(
        err(r#"{"published_at":"yesterday"}"#),
        InvalidValue(_)
    ));
    assert!(matches!(
        err(r#"{"published_at":{"$gt":"2024-13-01T00:00:00Z"}}"#),
        InvalidValue(_)
    ));

    assert!(matches!(err(r#"{"author_id":"${user.id"}"#), Malformed(_)));
    assert!(matches!(err(r#"{"title":"${}"}"#), Malformed(_)));
    assert!(matches!(err(r#"{"title":"${user..id}"}"#), Malformed(_)));
    assert!(matches!(
        err(r#"{"author_id":{"$in":["${user.a}"]}}"#),
        Malformed(_)
    ));
    assert!(matches!(err(r#"{"author_id":{"$in":5}}"#), Malformed(_)));
    assert!(matches!(err(r#"{"author_id":{}}"#), Malformed(_)));
    assert!(matches!(err(r#"{"$and":{}}"#), Malformed(_)));
    assert!(matches!(err(r#"{"$not":[]}"#), Malformed(_)));
    assert!(matches!(err(r#"{"org":5}"#), Malformed(_)));
    assert!(matches!(err(r#"{"tags":{}}"#), Malformed(_)));
    // Opaque fields expose no operators.
    assert!(matches!(err(r#"{"metadata":{}}"#), Malformed(_)));

    // Paths are dotted from `conditions`, with array indices.
    let e = cond(r#"{"$or":[{},{"author":1}]}"#).unwrap_err();
    assert_eq!(e.path, "conditions.$or[1].author");
    assert_eq!(
        e.to_string(),
        "rule 0 at `conditions.$or[1].author`: unknown field `author`"
    );
    let e = cond(r#"{"tags":{"$some":{"name":{"$in":["a",null]}}}}"#).unwrap_err();
    assert_eq!(e.path, "conditions.tags.$some.name.$in[1]");
    let e = T::compile(
        &[
            rule(r#"{"action":"read","subject":"Post"}"#),
            rule(r#"{"action":"read","subject":"Post","conditions":{"status":{"$ne":"x"}}}"#),
        ],
        &[],
    )
    .unwrap_err();
    assert_eq!(
        (e.rule_index, e.path.as_str()),
        (1, "conditions.status.$ne")
    );
}

#[test]
fn int_fields_reject_non_i64_numbers() {
    let int_mismatch = |k: LoadErrorKind| {
        matches!(
            k,
            LoadErrorKind::TypeMismatch {
                expected: Kind::Int,
                ..
            }
        )
    };
    for n in ["1.5", "1e3", "9223372036854775808", "-9223372036854775809"] {
        assert!(int_mismatch(err(&format!(r#"{{"author_id":{n}}}"#))), "{n}");
        assert!(
            int_mismatch(err(&format!(r#"{{"reviewer_id":{{"$in":[1,{n}]}}}}"#))),
            "{n}"
        );
        assert!(
            int_mismatch(err(&format!(r#"{{"id":{{"$lt":{n}}}}}"#))),
            "{n}"
        );
    }
    assert_eq!(err(r#"{"author_id":1.5}"#), mismatch(Kind::Int, "1.5"));
    cond(r#"{"author_id":9223372036854775807}"#).unwrap();
    cond(r#"{"author_id":-9223372036854775808}"#).unwrap();
    // Float fields accept any JSON number.
    cond(r#"{"score":{"$gt":1,"$lt":1.5e300}}"#).unwrap();
}

#[test]
fn nesting_depth_limit() {
    let nested = |n: usize| {
        format!(
            "{}{}{}",
            r#"{"$not":"#.repeat(n),
            r#"{"locked":true}"#,
            "}".repeat(n)
        )
    };
    cond(&nested(32)).unwrap();
    assert_eq!(
        err(&nested(33)),
        LoadErrorKind::Malformed("nesting too deep".into())
    );
    // Logical arrays and relation objects count as levels too.
    let or = |n: usize| format!("{}{}{}", r#"{"$or":["#.repeat(n), "{}", "]}".repeat(n));
    cond(&or(32)).unwrap();
    assert_eq!(
        err(&or(33)),
        LoadErrorKind::Malformed("nesting too deep".into())
    );
    // Post.reviewer (to-one) → User.posts ($some) → Post: two levels per step.
    let rel = |n: usize| {
        format!(
            "{}{}{}",
            r#"{"reviewer":{"posts":{"$some":"#.repeat(n),
            "{}",
            "}}}".repeat(n)
        )
    };
    cond(&rel(16)).unwrap();
    let e = cond(&rel(17)).unwrap_err();
    assert_eq!(e.kind, LoadErrorKind::Malformed("nesting too deep".into()));
    // The 33rd level is the 17th `reviewer` target object.
    assert_eq!(e.path.matches(".reviewer").count(), 17);
    assert!(e.path.ends_with(".$some.reviewer"), "{}", e.path);
}

#[test]
fn accepted_forms() {
    for c in [
        r#"{"title":"$$5"}"#,
        r#"{"title":"$5"}"#,
        r#"{"title":"$${x}"}"#,
        r#"{"title":{"$in":["$$5","$${x}"]}}"#,
        r#"{"reviewer":null}"#,
        r#"{"reviewer":{"$isNull":true}}"#,
        r#"{"reviewer":{"$isNull":false}}"#,
        r#"{"reviewer":{"$not":{"name":"x"}}}"#,
        r#"{"reviewer":{"$none":{"name":"x"}}}"#,
        r#"{"org":{"$none":{}}}"#,
        r#"{"org":{}}"#,
        r#"{"tags":{"$some":{}}}"#,
        r#"{"tags":{"$every":{"name":null}}}"#,
        r#"{"tags":{"$none":{"name":{"$startsWith":"x"}}}}"#,
        r#"{"reviewer_id":{"$ne":null}}"#,
        r#"{"reviewer_id":{"$eq":null}}"#,
        r#"{"reviewer_id":{"$isNull":true}}"#,
        r#"{"author_id":"${user.id?}"}"#,
        r#"{"author_id":{"$nin":"${user.blocked}"}}"#,
        r#"{"status":{"$in":["draft","published"]}}"#,
        r#"{"status":{"$in":[]}}"#,
        r#"{"published_at":{"$gte":"2024-01-15T10:30:00.123456789+02:00"}}"#,
        r#"{"title":{"$contains":"${user.name}","$endsWith":"!"}}"#,
        r#"{"reviewer":{"posts":{"$some":{"id":"${user.id}"}}}}"#,
        r#"{"$and":[],"$or":[],"$not":{}}"#,
        r#"{}"#,
    ] {
        cond(c).unwrap_or_else(|e| panic!("{c}: {e}"));
    }
    // Rules without conditions on wildcard and type-only subjects.
    T::compile(
        &[
            rule(r#"{"action":"manage","subject":"All"}"#),
            rule(r#"{"action":["read","update"],"subject":["Dashboard","Post"],"inverted":true,"reason":"r"}"#),
            rule(r#"{"action":"read","subject":"Post","fields":["title","org","metadata"]}"#),
        ],
        &[],
    )
    .unwrap();
}
