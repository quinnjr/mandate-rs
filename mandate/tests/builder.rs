#![cfg(all(feature = "derive", feature = "chrono", feature = "uuid"))]
mod common;
use common::fixture::*;
use mandate::{BuildError, Cond, FieldIdx, FieldRef};

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

#[test]
fn ability_is_send_sync() {
    fn assert<T: Send + Sync>() {}
    assert::<Ab>();
}
