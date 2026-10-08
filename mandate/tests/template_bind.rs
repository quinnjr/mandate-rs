#![cfg(all(feature = "derive", feature = "chrono", feature = "uuid"))]
mod common;
use common::fixture::*;
use mandate::{
    BindError, BindErrorKind, Bound, CmpOp, Condition, Context, FieldIdx, Kind, Quant,
    RuleTemplate, Templates, Unresolved, UnresolvedOutcome, Value,
};
use serde_json::json;

type T = Templates<Action, Subject>;
type Ab = mandate::Ability<Action, Subject>;

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

const TITLE: FieldIdx = FieldIdx(3);
const AUTHOR_ID: FieldIdx = FieldIdx(1);
const STATUS: FieldIdx = FieldIdx(6);
const TAGS: FieldIdx = FieldIdx(11);

fn templates(json: &str) -> T {
    let raw: Vec<RuleTemplate> = serde_json::from_str(json).unwrap();
    T::compile(&raw, &["user"]).unwrap()
}

fn user(value: serde_json::Value) -> Context {
    Context::new().with("user", &value).unwrap()
}

/// Binds `[<rules>]` with `user` as the `user` root.
fn bind(rules: &str, u: serde_json::Value) -> Bound<Action, Subject> {
    templates(&format!("[{rules}]")).bind(&user(u)).unwrap()
}

/// Binds a single `can read Post when <c>` rule.
fn bind_can(c: &str, u: serde_json::Value) -> Bound<Action, Subject> {
    bind(
        &format!(r#"{{"action":"read","subject":"Post","conditions":{c}}}"#),
        u,
    )
}

fn bind_err(c: &str, u: serde_json::Value) -> BindError {
    templates(&format!(
        r#"[{{"action":"read","subject":"Post","conditions":{c}}}]"#
    ))
    .bind(&user(u))
    .unwrap_err()
}

fn build(bound: Bound<Action, Subject>) -> Ab {
    Ab::builder().extend(bound).build().unwrap()
}

fn unresolved(rule_index: usize, placeholder: &str, outcome: UnresolvedOutcome) -> Unresolved {
    Unresolved {
        rule_index,
        placeholder: placeholder.into(),
        outcome,
    }
}

fn eq(field: FieldIdx, value: Value) -> Condition {
    Condition::Cmp {
        field,
        op: CmpOp::Eq,
        value,
    }
}

fn s(v: &str) -> Value {
    Value::String(v.into())
}

#[test]
fn binds_spec_example() {
    let ctx = Context::new()
        .with("user", &json!({"id": 7, "org_id": 3}))
        .unwrap();
    let bound = templates(&format!("[{SPEC_EXAMPLE}]")).bind(&ctx).unwrap();
    assert!(bound.unresolved().is_empty());
    assert_eq!(bound.rules().len(), 1);

    let got = build(bound);
    let want = Ab::builder()
        .can(Action::Update, Subject::Post)
        .when(
            Post::AUTHOR_ID
                .eq(7)
                .and(Post::STATUS.ne(Status::Archived))
                .and(Post::PUBLISHED_AT.is_null())
                .and(Post::ORG.then(Org::ID.eq(3)))
                .and(Post::REVIEWER.is_not_null())
                .and(Post::TAGS.some(Tag::NAME.eq("rust")))
                .and(Post::STATUS.eq(Status::Published).or(Post::AUTHOR_ID.eq(7))),
        )
        .fields([Post::TITLE.into(), Post::BODY.into()])
        .because("Authors edit their own posts")
        .build()
        .unwrap();
    assert_eq!(got.rules(), want.rules());
}

#[test]
fn rules_expand_action_major_and_stay_unfolded() {
    let bound = bind(
        r#"{"action":["read","update"],"subject":["Post","Dashboard"],"reason":"r"},
           {"action":"read","subject":"Post","inverted":true,
            "conditions":{"$and":[],"title":"x"},"fields":["title"]}"#,
        json!({}),
    );
    let got: Vec<_> = bound
        .rules()
        .iter()
        .map(|r| (r.action(), r.subject(), r.inverted(), r.reason()))
        .collect();
    assert_eq!(
        got,
        [
            (Action::Read, Subject::Post, false, Some("r")),
            (Action::Read, Subject::Dashboard, false, Some("r")),
            (Action::Update, Subject::Post, false, Some("r")),
            (Action::Update, Subject::Dashboard, false, Some("r")),
            (Action::Read, Subject::Post, true, None),
        ]
    );
    let last = &bound.rules()[4];
    assert_eq!(
        last.condition(),
        Some(&Condition::And(vec![
            Condition::And(vec![]),
            eq(TITLE, s("x"))
        ]))
    );
    assert_eq!(last.fields().unwrap().iter().collect::<Vec<_>>(), [TITLE]);
}

#[test]
fn literal_escapes() {
    let bound = templates(
        r#"[{"action":"read","subject":"Post","conditions":{"title":"$$5"}},
            {"action":"read","subject":"Post","conditions":{"title":"$5"}}]"#,
    )
    .bind(&Context::empty())
    .unwrap();
    assert_eq!(bound.rules()[0].condition(), Some(&eq(TITLE, s("$5"))));
    assert_eq!(bound.rules()[1].condition(), Some(&eq(TITLE, s("$5"))));

    // Context values are data, not template strings: they are never unescaped.
    let bound = bind_can(r#"{"title":"${user.title}"}"#, json!({"title": "$$5"}));
    assert_eq!(bound.rules()[0].condition(), Some(&eq(TITLE, s("$$5"))));
}

#[test]
fn unresolved_required_in_can_drops_rule() {
    let bound = bind_can(
        r#"{"reviewer_id":"${user.manager_id}"}"#,
        json!({"manager_id": null}),
    );
    assert_eq!(
        bound.unresolved(),
        [unresolved(
            0,
            "user.manager_id",
            UnresolvedOutcome::LeafFalse
        )]
    );
    assert_eq!(bound.rules()[0].condition(), Some(&Condition::Or(vec![])));
    assert!(build(bound).rules().is_empty());

    // A missing root is unresolved too.
    let bound = templates(
        r#"[{"action":"read","subject":"Post","conditions":{"reviewer_id":"${user.manager_id}"}}]"#,
    )
    .bind(&Context::empty())
    .unwrap();
    assert_eq!(bound.unresolved()[0].outcome, UnresolvedOutcome::LeafFalse);
    assert!(!build(bound).can(Action::Read, &post()));
}

#[test]
fn unresolved_required_in_cannot_denies() {
    let bound = bind(
        r#"{"action":"read","subject":"Post"},
           {"action":"read","subject":"Post","inverted":true,
            "conditions":{"author_id":"${user.blocked}"}}"#,
        json!({"id": 1}),
    );
    assert_eq!(
        bound.unresolved(),
        [unresolved(1, "user.blocked", UnresolvedOutcome::LeafTrue)]
    );
    let ability = build(bound);
    let rule = &ability.rules()[1];
    assert!(rule.inverted());
    assert_eq!(rule.condition(), None);
    assert!(!ability.can(Action::Read, &post()));
}

#[test]
fn polarity_flips_under_not_and_none() {
    let bound = bind_can(r#"{"$not":{"author_id":"${user.x}"}}"#, json!({}));
    assert_eq!(
        bound.unresolved(),
        [unresolved(0, "user.x", UnresolvedOutcome::LeafTrue)]
    );
    assert!(build(bound).rules().is_empty());

    let bound = bind_can(r#"{"tags":{"$none":{"name":"${user.tag}"}}}"#, json!({}));
    assert_eq!(
        bound.unresolved(),
        [unresolved(0, "user.tag", UnresolvedOutcome::LeafTrue)]
    );
    assert_eq!(
        build(bound).rules()[0].condition(),
        Some(&Condition::Rel {
            relation: TAGS,
            quant: Quant::None,
            cond: None
        })
    );

    // A double flip restores the polarity; in a `cannot` rule it inverts again.
    let bound = bind_can(
        r#"{"$not":{"tags":{"$none":{"name":"${user.tag}"}}}}"#,
        json!({}),
    );
    assert_eq!(bound.unresolved()[0].outcome, UnresolvedOutcome::LeafFalse);
    let bound = bind(
        r#"{"action":"read","subject":"Post","inverted":true,
            "conditions":{"$not":{"author_id":"${user.x}"}}}"#,
        json!({}),
    );
    // ¬false: the prohibition becomes unconditional.
    assert_eq!(bound.unresolved()[0].outcome, UnresolvedOutcome::LeafFalse);
    assert_eq!(build(bound).rules()[0].condition(), None);
}

#[test]
fn optional_placeholder_drops_only_its_rule() {
    let rules = r#"{"action":"read","subject":"Post"},
        {"action":"read","subject":"Post","inverted":true,
         "conditions":{"author_id":"${user.blocked?}","title":"${user.title}"}}"#;

    let bound = bind(rules, json!({}));
    // Only the optional placeholder is reported: the rule is gone.
    assert_eq!(
        bound.unresolved(),
        [unresolved(
            1,
            "user.blocked",
            UnresolvedOutcome::RuleDropped
        )]
    );
    assert_eq!(bound.rules().len(), 1);
    assert!(!bound.rules()[0].inverted());
    assert!(build(bound).can(Action::Read, &post()));

    let bound = bind(rules, json!({"blocked": 7, "title": "Hello"}));
    assert!(bound.unresolved().is_empty());
    assert!(!build(bound).can(Action::Read, &post()));
}

#[test]
fn list_placeholders() {
    let c = r#"{"status":{"$in":"${user.statuses}"}}"#;
    let bound = bind_can(c, json!({"statuses": ["draft"]}));
    assert_eq!(
        bound.rules()[0].condition(),
        Some(&Condition::In {
            field: STATUS,
            values: vec![s("draft")]
        })
    );
    assert!(!build(bound).can(Action::Read, &post()));

    let bound = bind_can(c, json!({"statuses": ["draft", "published"]}));
    assert!(build(bound).can(Action::Read, &post()));

    let bound = bind_can(c, json!({"statuses": []}));
    assert!(bound.unresolved().is_empty());
    assert_eq!(
        bound.rules()[0].condition(),
        Some(&Condition::In {
            field: STATUS,
            values: vec![]
        })
    );
    assert!(build(bound).rules().is_empty());

    let bound = bind_can(
        r#"{"author_id":{"$nin":"${user.ids}"}}"#,
        json!({"ids": [1, 2]}),
    );
    assert_eq!(
        bound.rules()[0].condition(),
        Some(&Condition::NotIn {
            field: AUTHOR_ID,
            values: vec![Value::Int(1), Value::Int(2)]
        })
    );

    // An unresolved list placeholder replaces its leaf like a scalar one.
    let bound = bind_can(c, json!({}));
    assert_eq!(
        bound.unresolved(),
        [unresolved(0, "user.statuses", UnresolvedOutcome::LeafFalse)]
    );
}

#[test]
fn placeholder_paths_walk_objects() {
    let c = r#"{"org":{"id":"${user.org.id}"}}"#;
    let bound = bind_can(c, json!({"org": {"id": 3}}));
    assert!(bound.unresolved().is_empty());
    assert!(build(bound).can(Action::Read, &post()));

    // A path through a non-object is missing, like an absent key.
    for u in [
        json!({"org": 3}),
        json!({"org": [{"id": 3}]}),
        json!({"org": null}),
    ] {
        let bound = bind_can(c, u);
        assert_eq!(
            bound.unresolved(),
            [unresolved(0, "user.org.id", UnresolvedOutcome::LeafFalse)]
        );
    }

    // A placeholder may name the root itself; a later `with` replaces a root.
    let t =
        templates(r#"[{"action":"read","subject":"Post","conditions":{"author_id":"${user}"}}]"#);
    let ctx = Context::new()
        .with("user", &1)
        .unwrap()
        .with("user", &7)
        .unwrap();
    assert!(build(t.bind(&ctx).unwrap()).can(Action::Read, &post()));
}

#[test]
fn bind_errors() {
    use BindErrorKind::*;

    let e = bind_err(r#"{"author_id":"${user.id}"}"#, json!({"id": "seven"}));
    assert_eq!(
        e,
        BindError {
            rule_index: 0,
            path: "conditions.author_id".into(),
            kind: TypeMismatch {
                expected: Kind::Int,
                found: "\"seven\"".into()
            }
        }
    );
    assert_eq!(
        e.to_string(),
        "rule 0 at `conditions.author_id`: expected Int, found \"seven\""
    );
    assert_eq!(
        bind_err(
            r#"{"status":"${user.status}"}"#,
            json!({"status": "Published"})
        )
        .kind,
        UnknownVariant("Published".into())
    );
    assert!(matches!(
        bind_err(
            r#"{"published_at":{"$lt":"${user.since}"}}"#,
            json!({"since": "yesterday"})
        )
        .kind,
        InvalidValue(_)
    ));

    // Kinds follow the compile-time literal rules.
    for (c, u, expected, found) in [
        (
            r#"{"author_id":"${user.v}"}"#,
            json!({"v": 1.5}),
            Kind::Int,
            "1.5",
        ),
        (
            r#"{"author_id":"${user.v}"}"#,
            json!({"v": [1]}),
            Kind::Int,
            "array",
        ),
        (
            r#"{"locked":"${user.v}"}"#,
            json!({"v": 1}),
            Kind::Bool,
            "1",
        ),
        (
            r#"{"title":"${user.v}"}"#,
            json!({"v": {}}),
            Kind::String,
            "object",
        ),
        (
            r#"{"title":{"$contains":"${user.v}"}}"#,
            json!({"v": true}),
            Kind::String,
            "true",
        ),
        (
            r#"{"reviewer_id":{"$in":"${user.v}"}}"#,
            json!({"v": 1}),
            Kind::Int,
            "1",
        ),
    ] {
        assert_eq!(
            bind_err(c, u).kind,
            TypeMismatch {
                expected,
                found: found.into()
            },
            "{c}"
        );
    }
    let bound = bind_can(r#"{"score":"${user.v}"}"#, json!({"v": 2}));
    assert_eq!(
        bound.rules()[0].condition(),
        Some(&eq(FieldIdx(8), Value::Float(2.0)))
    );
    let bound = bind_can(
        r#"{"published_at":{"$gte":"${user.v}"}}"#,
        json!({"v": "2024-01-15T12:30:00+02:00"}),
    );
    assert_eq!(
        bound.rules()[0].condition(),
        Some(&Condition::Cmp {
            field: FieldIdx(7),
            op: CmpOp::Gte,
            value: Value::DateTime("2024-01-15T10:30:00Z".parse().unwrap())
        })
    );

    // List elements: their path gets an index; null is never a list element.
    let e = bind_err(
        r#"{"status":{"$in":"${user.statuses}"}}"#,
        json!({"statuses": ["draft", null]}),
    );
    assert_eq!(e.path, "conditions.status.$in[1]");
    assert_eq!(e.kind, InvalidValue("null in list".into()));
    let e = bind_err(
        r#"{"status":{"$nin":"${user.statuses}"}}"#,
        json!({"statuses": ["draft", "Published"]}),
    );
    assert_eq!(e.path, "conditions.status.$nin[1]");
    assert_eq!(e.kind, UnknownVariant("Published".into()));

    // A bad value fails the bind even in a rule an optional placeholder drops.
    let e = templates(
        r#"[{"action":"read","subject":"Post"},
            {"action":"read","subject":"Post","conditions":
              {"author_id":"${user.missing?}","title":"${user.id}"}}]"#,
    )
    .bind(&user(json!({"id": 7})))
    .unwrap_err();
    assert_eq!((e.rule_index, e.path.as_str()), (1, "conditions.title"));
}

#[test]
fn extend_preserves_definition_order() {
    let deny = || {
        bind(
            r#"{"action":"read","subject":"Post","inverted":true}"#,
            json!({}),
        )
    };

    let a = Ab::builder()
        .can(Action::Read, Subject::Post)
        .extend(deny())
        .build()
        .unwrap();
    assert!(!a.can(Action::Read, &post()));

    let a = Ab::builder()
        .extend(deny())
        .can(Action::Read, Subject::Post)
        .build()
        .unwrap();
    assert!(a.can(Action::Read, &post()));

    // Extended rules sit between the groups around them.
    let a = Ab::builder()
        .can(Action::Read, Subject::Post)
        .extend(deny())
        .can(Action::Update, Subject::Post)
        .extend(bind(r#"{"action":"delete","subject":"Post"}"#, json!({})))
        .build()
        .unwrap();
    let got: Vec<_> = a
        .rules()
        .iter()
        .map(|r| (r.action(), r.inverted()))
        .collect();
    assert_eq!(
        got,
        [
            (Action::Read, false),
            (Action::Read, true),
            (Action::Update, false),
            (Action::Delete, false),
        ]
    );
}

/// `"conditions": null` binds an unconditional rule and `"fields": null` a
/// rule on every field, exactly like omitting them.
#[test]
fn null_conditions_and_fields_bind_like_omitted() {
    let omitted = bind(r#"{"action":"update","subject":"Post"}"#, json!({}));
    let null = bind(
        r#"{"action":"update","subject":"Post","conditions":null,"fields":null}"#,
        json!({}),
    );
    assert_eq!(null.rules(), omitted.rules());
    let rule = &null.rules()[0];
    assert!(rule.condition().is_none() && rule.fields().is_none());

    let a = build(null);
    let p = post();
    assert!(a.can(Action::Update, &p));
    assert!(a.can_field(Action::Update, &p, Post::METADATA));
    assert_eq!(
        a.permitted_fields(Action::Update, &p).unwrap().mask(),
        mandate::FieldMask::all(13)
    );
    // A null-conditions `cannot` denies unconditionally.
    let a = build(bind(
        r#"{"action":"update","subject":"Post"},
           {"action":"update","subject":"Post","inverted":true,"conditions":null}"#,
        json!({}),
    ));
    assert!(!a.can(Action::Update, &p));
}
