#![cfg(all(feature = "derive", feature = "chrono", feature = "uuid"))]
//! Behaviours mirrored from CASL's own test suite, plus the documented
//! deviation from `rulesToQuery`.
mod common;
use common::fixture::*;
use mandate::{Ability, Access};

type Ab = Ability<Action, Subject>;

fn org() -> Org {
    Org {
        id: 3,
        name: "Acme".into(),
    }
}

#[test]
fn allows_matching_action_and_subject() {
    let a = Ab::builder()
        .can(Action::Read, Subject::Post)
        .build()
        .unwrap();
    assert!(a.can(Action::Read, &post()));
    assert!(!a.can(Action::Update, &post()));
    assert!(!a.can(Action::Read, &org()));
}

#[test]
fn inverted_rule_overrides_earlier_can() {
    let a = Ab::builder()
        .can(Action::Read, Subject::Post)
        .cannot(Action::Read, Subject::Post)
        .build()
        .unwrap();
    assert!(!a.can(Action::Read, &post()));
}

#[test]
fn later_rule_takes_precedence() {
    let a = Ab::builder()
        .cannot(Action::Read, Subject::Post)
        .can(Action::Read, Subject::Post)
        .build()
        .unwrap();
    assert!(a.can(Action::Read, &post()));
}

#[test]
fn manage_matches_any_action() {
    let a = Ab::builder()
        .can(Action::Manage, Subject::Post)
        .build()
        .unwrap();
    for act in [
        Action::Read,
        Action::Create,
        Action::Update,
        Action::Delete,
        Action::Manage,
    ] {
        assert!(a.can(act, &post()));
    }
    assert!(!a.can(Action::Read, &org()));
}

#[test]
fn all_matches_any_subject() {
    let a = Ab::builder()
        .can(Action::Read, Subject::All)
        .build()
        .unwrap();
    assert!(a.can(Action::Read, &post()));
    assert!(a.can(Action::Read, &org()));
    assert!(a.can_type(Action::Read, Subject::Dashboard));
    assert!(!a.can(Action::Update, &post()));
}

#[test]
fn can_with_fields_allows_whole_subject_check() {
    let a = Ab::builder()
        .can(Action::Read, Subject::Post)
        .fields([Post::TITLE.into()])
        .build()
        .unwrap();
    assert!(a.can(Action::Read, &post()));
    assert!(a.can_field(Action::Read, &post(), Post::TITLE));
    assert!(!a.can_field(Action::Read, &post(), Post::BODY));
}

#[test]
fn cannot_with_fields_ignored_without_field() {
    let a = Ab::builder()
        .can(Action::Read, Subject::Post)
        .cannot(Action::Read, Subject::Post)
        .fields([Post::BODY.into()])
        .build()
        .unwrap();
    assert!(a.can(Action::Read, &post()));
    assert!(a.can_field(Action::Read, &post(), Post::TITLE));
    assert!(!a.can_field(Action::Read, &post(), Post::BODY));
}

#[test]
fn conditional_can_allows_type_check() {
    let a = Ab::builder()
        .can(Action::Read, Subject::Post)
        .when(Post::AUTHOR_ID.eq(99))
        .build()
        .unwrap();
    assert!(a.can_type(Action::Read, Subject::Post));
    assert!(!a.can(Action::Read, &post()));
}

#[test]
fn conditional_cannot_does_not_deny_type_check() {
    let a = Ab::builder()
        .can(Action::Read, Subject::Post)
        .cannot(Action::Read, Subject::Post)
        .when(Post::LOCKED.eq(true))
        .build()
        .unwrap();
    assert!(a.can_type(Action::Read, Subject::Post));
    assert!(a.can(Action::Read, &post()));
    let locked = Post {
        locked: true,
        ..post()
    };
    assert!(!a.can(Action::Read, &locked));
}

#[test]
fn permitted_fields_order_semantics() {
    let names = |a: &Ab| -> Vec<&'static str> {
        a.permitted_fields(Action::Read, &post())
            .unwrap()
            .iter()
            .map(|(_, n)| n)
            .collect()
    };
    let a = Ab::builder()
        .can(Action::Read, Subject::Post)
        .fields([Post::TITLE.into(), Post::BODY.into()])
        .cannot(Action::Read, Subject::Post)
        .fields([Post::BODY.into()])
        .build()
        .unwrap();
    assert_eq!(names(&a), ["title"]);
    // a later `can` re-grants what an earlier `cannot` removed
    let a = Ab::builder()
        .can(Action::Read, Subject::Post)
        .fields([Post::TITLE.into(), Post::BODY.into()])
        .cannot(Action::Read, Subject::Post)
        .fields([Post::BODY.into()])
        .can(Action::Read, Subject::Post)
        .fields([Post::BODY.into()])
        .build()
        .unwrap();
    assert_eq!(names(&a), ["title", "body"]);
}

#[test]
fn deviation_exact_query_formula() {
    // can A; cannot B; can C. A row matching B and C (but not A) is decided by
    // the last matching rule, C, so it is allowed. CASL's `rulesToQuery` would
    // exclude it; mandate's plan matches `can` exactly.
    let a = Ab::builder()
        .can(Action::Read, Subject::Post)
        .when(Post::STATUS.eq(Status::Published))
        .cannot(Action::Read, Subject::Post)
        .when(Post::LOCKED.eq(true))
        .can(Action::Read, Subject::Post)
        .when(Post::AUTHOR_ID.eq(7))
        .build()
        .unwrap();
    let row = Post {
        status: Status::Draft,
        locked: true,
        author_id: 7,
        ..post()
    };
    assert!(a.can(Action::Read, &row));
    let Access::Filter(plan) = a.access::<Post>(Action::Read).unwrap() else {
        panic!("expected a filter");
    };
    assert!(plan.eval(&row).unwrap());
    // and a row matching only B stays excluded by both
    let only_b = Post {
        status: Status::Draft,
        locked: true,
        author_id: 8,
        ..post()
    };
    assert!(!a.can(Action::Read, &only_b));
    assert!(!plan.eval(&only_b).unwrap());
}
