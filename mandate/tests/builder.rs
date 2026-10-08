#![cfg(all(feature = "derive", feature = "chrono", feature = "uuid"))]
mod common;
use common::fixture::*;
use mandate::{BuildError, Cond, Field, FieldIdx, FieldRef, Rel};

type Ab = mandate::Ability<Action, Subject>;

#[test]
fn group_modifiers_apply_to_every_pair() {
    let a = Ab::builder()
        .can([Action::Read, Action::Update], Subject::Post)
        .when(Post::AUTHOR_ID.eq(1))
        .fields([Post::TITLE.into()])
        .build()
        .unwrap();
    assert_eq!(a.rules().len(), 2);
    assert!(
        a.rules()
            .iter()
            .all(|r| r.condition().is_some() && r.fields().unwrap().contains(FieldIdx(3)))
    );
}

#[test]
fn repeat_modifiers_combine() {
    let a = Ab::builder()
        .cannot(Action::Read, Subject::Post)
        .when(Post::AUTHOR_ID.eq(1))
        .when(Post::LOCKED.eq(true))
        .fields([Post::TITLE.into()])
        .fields([Post::SCORE.into()])
        .because("a")
        .because("b")
        .build()
        .unwrap();
    let r = &a.rules()[0];
    assert!(r.inverted());
    assert_eq!(r.reason(), Some("b"));
    let f = r.fields().unwrap();
    assert!(f.contains(FieldIdx(3)) && f.contains(FieldIdx(8)));
    assert_eq!(
        r.condition().cloned(),
        Some(mandate::Condition::And(vec![
            Post::AUTHOR_ID.eq(1).into_condition(),
            Post::LOCKED.eq(true).into_condition()
        ]))
    );
}

#[test]
fn expansion_is_action_major() {
    let a = Ab::builder()
        .can(
            [Action::Read, Action::Update],
            [Subject::Post, Subject::Org],
        )
        .build()
        .unwrap();
    let got: Vec<_> = a
        .rules()
        .iter()
        .map(|r| (r.action(), r.subject()))
        .collect();
    assert_eq!(
        got,
        vec![
            (Action::Read, Subject::Post),
            (Action::Read, Subject::Org),
            (Action::Update, Subject::Post),
            (Action::Update, Subject::Org)
        ]
    );
}

#[test]
fn into_actions_forms() {
    let list = vec![Action::Read, Action::Delete];
    let a = Ab::builder()
        .can(&list[..], Subject::Dashboard)
        .can(list, [Subject::Dashboard])
        .build()
        .unwrap();
    assert_eq!(a.rules().len(), 4);
}

#[test]
fn folding_at_build() {
    let a = Ab::builder()
        .can(Action::Read, Subject::Post)
        .when(Cond::<Post>::all([]))
        .build()
        .unwrap();
    assert_eq!(a.rules()[0].condition(), None);
    let a = Ab::builder()
        .can(Action::Read, Subject::Post)
        .when(Post::AUTHOR_ID.is_in(Vec::<i64>::new()))
        .build()
        .unwrap();
    assert!(a.rules().is_empty());
}

#[test]
fn build_errors() {
    let err = |r: Result<Ab, BuildError>| r.unwrap_err();
    assert_eq!(
        err(Ab::builder()
            .can(Action::Read, Subject::Post)
            .when(Org::ID.eq(1))
            .build()),
        BuildError::SubjectMismatch {
            subject: "Post",
            condition_schema: "Org"
        }
    );
    for s in [
        Ab::builder()
            .can(Action::Read, Subject::All)
            .when(Post::AUTHOR_ID.eq(1))
            .build(),
        Ab::builder()
            .can(Action::Read, Subject::Dashboard)
            .when(Post::AUTHOR_ID.eq(1))
            .build(),
        Ab::builder()
            .can(Action::Read, [Subject::Post, Subject::Org])
            .when(Post::AUTHOR_ID.eq(1))
            .build(),
    ] {
        assert!(matches!(err(s), BuildError::ConditionsNotAllowed { .. }));
    }
    assert!(matches!(
        err(Ab::builder()
            .can(Action::Read, Subject::Post)
            .fields([Org::NAME.into()])
            .build()),
        BuildError::ForeignField {
            subject: "Post",
            field_schema: "Org"
        }
    ));
    assert_eq!(
        err(Ab::builder()
            .can(Vec::<Action>::new(), Subject::Post)
            .build()),
        BuildError::Empty { what: "actions" }
    );
    assert_eq!(
        err(Ab::builder()
            .can(Action::Read, Vec::<Subject>::new())
            .build()),
        BuildError::Empty { what: "subjects" }
    );
    assert_eq!(
        err(Ab::builder()
            .can(Action::Read, Subject::Post)
            .fields(Vec::<FieldRef<Post>>::new())
            .build()),
        BuildError::Empty { what: "fields" }
    );
    for v in [f64::NAN, f64::INFINITY] {
        assert!(matches!(
            err(Ab::builder()
                .can(Action::Read, Subject::Post)
                .when(Post::SCORE.eq(v))
                .build()),
            BuildError::InvalidValue { .. }
        ));
    }
}

#[test]
fn invalid_parts_are_not_folded_away() {
    // The invalid condition sits next to a constant that would absorb it.
    let c = Post::SCORE.eq(f64::NAN).or(Cond::<Post>::all([]));
    assert!(matches!(
        Ab::builder()
            .can(Action::Read, Subject::Post)
            .when(c)
            .build(),
        Err(BuildError::InvalidValue { .. })
    ));
}

/// The path and reason of the `InvalidField` error of `can read Post when c`.
fn invalid(c: Cond<Post>) -> (String, String) {
    match Ab::builder()
        .can(Action::Read, Subject::Post)
        .when(c)
        .build()
    {
        Err(BuildError::InvalidField {
            subject: "Post",
            path,
            reason,
        }) => (path, reason),
        other => panic!("expected InvalidField, got {other:?}"),
    }
}

#[test]
fn hand_built_handles_are_checked_against_the_schema() {
    let cases: Vec<(Cond<Post>, &str, &str)> = vec![
        // An `i64` handle on the `String` title.
        (
            Field::<Post, i64>::new(3).lt(5),
            "title",
            "`Lt` is not allowed on String",
        ),
        (Field::<Post, i64>::new(3).eq(5), "title", "does not fit"),
        (
            Field::<Post, i64>::new(200).eq(1),
            "#200",
            "no field with index 200",
        ),
        (
            Field::<Post, String>::new(6).eq("deleted"),
            "status",
            "does not fit",
        ),
        (
            Field::<Post, String>::new(6).contains("x"),
            "status",
            "`Contains` is not allowed on Enum",
        ),
        (
            Field::<Post, Option<i64>>::new(1).is_null(),
            "author_id",
            "nullable",
        ),
        (Field::<Post, String>::new(12).eq("x"), "metadata", "opaque"),
        (Field::<Post, i64>::new(9).eq(1), "org", "only quantifiers"),
        (
            Rel::<Post, Vec<Tag>>::new(9).some(Tag::ID.eq(1)),
            "org",
            "`Some` does not fit a ToOne relation",
        ),
        (
            Rel::<Post, Org>::new(3).then(Org::ID.eq(1)),
            "title",
            "needs a relation",
        ),
        (
            Rel::<Post, Option<User>>::new(9).is_null(),
            "org",
            "nullable",
        ),
        // Inside relations, against the target's schema.
        (
            Post::ORG.then(Field::<Org, i64>::new(5).eq(1)),
            "org.#5",
            "`Org` has no field with index 5",
        ),
        (
            Post::ORG.then(Field::<Org, i64>::new(1).gt(1)),
            "org.name",
            "`Gt` is not allowed on String",
        ),
        (
            Post::TAGS.some(Field::<Tag, i64>::new(1).eq(1)),
            "tags.name",
            "does not fit",
        ),
        // Inside groups and negations.
        (
            !(Post::ID.eq(1).and(Field::<Post, i64>::new(3).eq(5))),
            "title",
            "does not fit",
        ),
    ];
    for (c, path, reason) in cases {
        let shown = format!("{c:?}");
        let (p, r) = invalid(c);
        assert_eq!(p, path, "{shown}");
        assert!(r.contains(reason), "{shown}: {r}");
    }
    // Hand-built handles that match the schema are fine.
    assert!(
        Ab::builder()
            .can(Action::Read, Subject::Post)
            .when(Field::<Post, i64>::new(1).gte(7))
            .when(Rel::<Post, Option<User>>::new(10).is_null())
            .fields([FieldRef::<Post>::new(12)])
            .build()
            .is_ok()
    );
}

#[test]
fn hand_built_field_lists_are_checked_against_the_schema() {
    assert_eq!(
        Ab::builder()
            .can(Action::Read, Subject::Post)
            .fields([Post::TITLE.into(), FieldRef::new(13)])
            .build()
            .unwrap_err(),
        BuildError::InvalidField {
            subject: "Post",
            path: "#13".into(),
            reason: "`Post` has no field with index 13".into(),
        }
    );
}

#[test]
fn ability_is_send_sync() {
    fn assert<T: Send + Sync>() {}
    assert::<Ab>();
}
