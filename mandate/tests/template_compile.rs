#![cfg(all(feature = "derive", feature = "chrono", feature = "uuid"))]
mod common;
use common::assert_matches;
use common::fixture::*;
use common::spec::SPEC_EXAMPLE;
use common::templates::{
    assert_invalid, assert_load_err, assert_mismatch, assert_not_allowed, cond, not_date_time,
};
use mandate::{Context, Kind, LoadError, LoadErrorKind, RuleTemplate, Templates};

type T = Templates<Action, Subject>;

fn rule(json: &str) -> RuleTemplate {
    serde_json::from_str(json).unwrap()
}

fn compile(json: &str) -> Result<T, LoadError> {
    T::compile(&[rule(json)], &["user"])
}

fn err(c: &str) -> LoadErrorKind {
    cond(c).unwrap_err().kind
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

    for (c, path, expected, found) in [
        (
            r#"{"author_id":"seven"}"#,
            "conditions.author_id",
            Kind::Int,
            "\"seven\"",
        ),
        (r#"{"locked":1}"#, "conditions.locked", Kind::Bool, "1"),
        (
            r#"{"title":true}"#,
            "conditions.title",
            Kind::String,
            "true",
        ),
        (
            r#"{"author_id":[1]}"#,
            "conditions.author_id",
            Kind::Int,
            "array",
        ),
        (
            r#"{"title":{"$contains":5}}"#,
            "conditions.title.$contains",
            Kind::String,
            "5",
        ),
        (
            r#"{"reviewer":{"$isNull":"yes"}}"#,
            "conditions.reviewer.$isNull",
            Kind::Bool,
            "\"yes\"",
        ),
    ] {
        assert_mismatch(c, path, expected, found);
    }

    assert_not_allowed(
        r#"{"title":{"$lt":"a"}}"#,
        "conditions.title.$lt",
        "$lt",
        Kind::String,
    );
    assert_not_allowed(
        r#"{"author_id":{"$contains":"1"}}"#,
        "conditions.author_id.$contains",
        "$contains",
        Kind::Int,
    );
    assert_not_allowed(
        r#"{"locked":{"$gte":true}}"#,
        "conditions.locked.$gte",
        "$gte",
        Kind::Bool,
    );
    assert_matches!(
        err(r#"{"status":{"$gt":"draft"}}"#),
        OperatorNotAllowed {
            kind: Kind::Enum(_),
            ..
        }
    );
    assert_matches!(
        err(r#"{"status":{"$startsWith":"d"}}"#),
        OperatorNotAllowed {
            kind: Kind::Enum(_),
            ..
        }
    );

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

    // The reason starts with our text and ends with chrono's own message.
    for (c, path, text) in [
        (
            r#"{"published_at":"yesterday"}"#,
            "conditions.published_at",
            "yesterday",
        ),
        (
            r#"{"published_at":{"$gt":"2024-13-01T00:00:00Z"}}"#,
            "conditions.published_at.$gt",
            "2024-13-01T00:00:00Z",
        ),
    ] {
        assert_invalid(c, path, |r| r.starts_with(&not_date_time(text)));
    }

    for (c, path, reason) in [
        (
            r#"{"author_id":"${user.id"}"#,
            "conditions.author_id",
            "invalid placeholder `${user.id`",
        ),
        (
            r#"{"title":"${}"}"#,
            "conditions.title",
            "invalid placeholder `${}`",
        ),
        (
            r#"{"title":"${user..id}"}"#,
            "conditions.title",
            "invalid placeholder `${user..id}`",
        ),
        (
            r#"{"author_id":{"$in":["${user.a}"]}}"#,
            "conditions.author_id.$in[0]",
            "a list element cannot be a placeholder; use a placeholder for the whole list",
        ),
        (
            r#"{"author_id":{"$in":5}}"#,
            "conditions.author_id.$in",
            "expected an array or a placeholder, found 5",
        ),
        (
            r#"{"author_id":{}}"#,
            "conditions.author_id",
            "empty operator object",
        ),
        (
            r#"{"$and":{}}"#,
            "conditions.$and",
            "`$and` expects an array of conditions",
        ),
        (
            r#"{"$not":[]}"#,
            "conditions.$not",
            "expected a condition object, found array",
        ),
        (
            r#"{"org":5}"#,
            "conditions.org",
            "a relation expects null or an object, found 5",
        ),
        (
            r#"{"tags":{}}"#,
            "conditions.tags",
            "a to-many relation needs `$some`, `$every` or `$none`",
        ),
        // Opaque fields expose no operators.
        (
            r#"{"metadata":{}}"#,
            "conditions.metadata",
            "`metadata` is an opaque field and cannot be used in conditions",
        ),
    ] {
        assert_load_err(c, path, |k| *k == Malformed(reason.into()));
    }

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

/// `$startsWith` and `$endsWith` compile to the conditions of the typed
/// builders.
#[test]
fn text_operators_compile_like_the_builders() {
    for (c, want) in [
        (
            r#"{"title":{"$startsWith":"He"}}"#,
            Post::TITLE.starts_with("He"),
        ),
        (
            r#"{"title":{"$endsWith":"lo"}}"#,
            Post::TITLE.ends_with("lo"),
        ),
    ] {
        let bound = cond(c).unwrap().bind(&Context::empty()).unwrap();
        assert_eq!(
            bound.rules()[0].condition(),
            Some(&want.into_condition()),
            "{c}"
        );
    }
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
    assert_mismatch(
        r#"{"author_id":1.5}"#,
        "conditions.author_id",
        Kind::Int,
        "1.5",
    );
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

/// `"conditions": null` and `"fields": null` mean the same as leaving the
/// key out (as in CASL): an unconditional rule on every field.
#[test]
fn null_conditions_and_fields_mean_omitted() {
    let omitted = rule(r#"{"action":"read","subject":"Post"}"#);
    let null = rule(r#"{"action":"read","subject":"Post","conditions":null,"fields":null}"#);
    assert_eq!(null, omitted);
    assert!(null.conditions.is_none() && null.fields.is_none());
    // So they are allowed where conditions and fields are not.
    for subject in ["All", "Dashboard"] {
        let json =
            format!(r#"{{"action":"read","subject":"{subject}","conditions":null,"fields":null}}"#);
        compile(&json).unwrap();
    }
    // Unlike an empty list or a non-object condition.
    assert_eq!(
        compile(r#"{"action":"read","subject":"Post","fields":[]}"#)
            .unwrap_err()
            .kind,
        LoadErrorKind::Empty("fields")
    );
    let e = compile(r#"{"action":"read","subject":"Post","conditions":[]}"#).unwrap_err();
    assert_eq!(
        (e.rule_index, e.path.as_str(), e.kind),
        (
            0,
            "conditions",
            LoadErrorKind::Malformed("expected a condition object, found array".into())
        )
    );
}
