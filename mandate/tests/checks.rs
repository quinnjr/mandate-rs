#![cfg(all(feature = "derive", feature = "chrono", feature = "uuid"))]
mod common;
use common::assert_matches;
use common::fixture::*;
use common::imposter::Imposter;
use mandate::{Ability, CheckError, EvalError, FieldRef, Resource};

type Ab = Ability<Action, Subject>;

fn draft(author: i64) -> Post {
    Post {
        author_id: author,
        status: Status::Draft,
        ..post()
    }
}

#[test]
fn later_rules_win() {
    let a = Ab::builder()
        .can(Action::Read, Subject::Post)
        .cannot(Action::Read, Subject::Post)
        .when(Post::STATUS.eq(Status::Draft))
        .can(Action::Read, Subject::Post)
        .when(Post::AUTHOR_ID.eq(7))
        .build()
        .unwrap();
    assert!(a.can(Action::Read, &draft(7)));
    assert!(!a.can(Action::Read, &draft(8)));
    assert!(a.can(Action::Read, &post()));
}

#[test]
fn empty_ability_denies_everything() {
    let a = Ab::builder().build().unwrap();
    assert!(!a.can(Action::Read, &post()));
    assert!(!a.can_type(Action::Read, Subject::Post));
    assert!(!a.can_field(Action::Read, &post(), Post::TITLE));
    assert!(
        a.permitted_fields(Action::Read, &post())
            .unwrap()
            .mask()
            .is_empty()
    );
    let a = Ab::builder()
        .can(Action::Read, Subject::Marker)
        .build()
        .unwrap();
    assert!(a.can(Action::Read, &Marker {}));
    assert!(
        a.permitted_fields(Action::Read, &Marker {})
            .unwrap()
            .mask()
            .is_empty()
    );
}

#[test]
fn wildcards_interleave_by_definition_order() {
    let p = post();
    let a = Ab::builder()
        .cannot(Action::Manage, Subject::All)
        .can(Action::Read, Subject::Post)
        .build()
        .unwrap();
    assert!(a.can(Action::Read, &p));
    assert!(!a.can(Action::Update, &p));
    let a = Ab::builder()
        .can(Action::Read, Subject::Post)
        .cannot(Action::Manage, Subject::All)
        .build()
        .unwrap();
    assert!(!a.can(Action::Read, &p));
}

#[test]
fn field_restrictions() {
    let p = post();
    let a = Ab::builder()
        .can(Action::Read, Subject::Post)
        .fields([Post::TITLE.into()])
        .build()
        .unwrap();
    assert!(a.can(Action::Read, &p));
    assert!(a.can_field(Action::Read, &p, Post::TITLE));
    assert!(!a.can_field(Action::Read, &p, Post::BODY));
    let a = Ab::builder()
        .can(Action::Read, Subject::Post)
        .cannot(Action::Read, Subject::Post)
        .fields([Post::BODY.into()])
        .build()
        .unwrap();
    assert!(a.can(Action::Read, &p));
    assert!(a.can_field(Action::Read, &p, Post::TITLE));
    assert!(!a.can_field(Action::Read, &p, Post::BODY));
}

#[test]
fn field_outside_the_schema_is_denied() {
    let p = post();
    let a = Ab::builder()
        .can(Action::Read, Subject::Post)
        .build()
        .unwrap();
    assert!(a.can_field(Action::Read, &p, Post::TITLE));
    // Hand-built handles past the end of the schema, up to the edges of
    // `FieldMask` (128 fields) and of the index type.
    let n = Post::schema().fields().len() as u16;
    for idx in [n, n + 1, 127, 128, 500, u16::MAX] {
        let missing = FieldRef::<Post>::new(idx);
        assert!(!a.can_field(Action::Read, &p, missing), "#{idx}");
        let e = a.check_field(Action::Read, &p, missing).unwrap_err();
        assert!(
            matches!(
                &e,
                CheckError::Unresolvable(EvalError::InvalidCondition { path, .. })
                    if *path == format!("#{idx}")
            ),
            "{e:?}"
        );
    }
}

#[test]
fn can_type_semantics() {
    let a = Ab::builder()
        .can(Action::Read, Subject::Post)
        .when(Post::AUTHOR_ID.eq(1))
        .build()
        .unwrap();
    assert!(a.can_type(Action::Read, Subject::Post));
    let a = Ab::builder()
        .can(Action::Read, Subject::Post)
        .cannot(Action::Read, Subject::Post)
        .when(Post::AUTHOR_ID.eq(1))
        .build()
        .unwrap();
    assert!(a.can_type(Action::Read, Subject::Post));
    let a = Ab::builder()
        .can(Action::Read, Subject::Post)
        .cannot(Action::Read, Subject::Post)
        .build()
        .unwrap();
    assert!(!a.can_type(Action::Read, Subject::Post));
    let e = a.check_type(Action::Read, Subject::Post).unwrap_err();
    assert!(
        e.action == Action::Read
            && e.subject == Subject::Post
            && e.field.is_none()
            && e.reason.is_none(),
        "{e:?}"
    );
}

#[test]
fn permitted_fields_walk() {
    let a = Ab::builder()
        .can(Action::Read, Subject::Post)
        .cannot(Action::Read, Subject::Post)
        .fields([Post::BODY.into()])
        .when(Post::STATUS.eq(Status::Draft))
        .build()
        .unwrap();
    let n = Post::schema().fields().len();
    let d = a.permitted_fields(Action::Read, &draft(7)).unwrap();
    assert_eq!(d.mask().len(), n - 1);
    assert!(!d.contains(Post::BODY) && d.contains(Post::TITLE));
    assert!(d.iter().all(|(_, name)| name != "body"));
    let names: Vec<_> = d.iter().map(|(i, _)| i.0).collect();
    assert!(names.windows(2).all(|w| w[0] < w[1]));
    let p = a.permitted_fields(Action::Read, &post()).unwrap();
    assert_eq!(p.mask().len(), n);
    assert_eq!(p.iter().count(), n);
}

#[test]
fn check_reason_and_display() {
    let a = Ab::builder()
        .can(Action::Manage, Subject::Post)
        .cannot(Action::Delete, Subject::Post)
        .when(Post::STATUS.eq(Status::Published))
        .because("Published posts cannot be deleted")
        .build()
        .unwrap();
    let e = a.check(Action::Delete, &post()).unwrap_err();
    assert_eq!(
        e.to_string(),
        "cannot delete Post: Published posts cannot be deleted"
    );
    assert_matches!(e, CheckError::Forbidden(_));
    assert!(a.check(Action::Delete, &draft(7)).is_ok());
    let e = a.check_field(Action::Read, &post(), Post::BODY);
    assert!(e.is_ok());
    let a = Ab::builder().build().unwrap();
    let e = a
        .check_field(Action::Update, &post(), Post::BODY)
        .unwrap_err();
    assert_eq!(e.to_string(), "cannot update Post.body");
}

#[test]
fn fail_closed_on_unloaded() {
    let a = Ability::<Action, TSubject>::builder()
        .can(Action::Read, TSubject::TPost)
        .cannot(Action::Read, TSubject::TPost)
        .when(TPost::TAGS.some(TTag::ID.eq(1)))
        .build()
        .unwrap();
    let p = TPost {
        title: "t".into(),
        org: Lazy::NotLoaded,
        reviewer: Lazy::NotLoaded,
        tags: Lazy::NotLoaded,
        ..loaded_tpost()
    };
    assert!(!a.can(Action::Read, &p));
    let e = a.check(Action::Read, &p).unwrap_err();
    assert!(
        matches!(
            &e,
            CheckError::Unresolvable(EvalError::NotLoaded { path, .. }) if path == "tags"
        ),
        "{e:?}"
    );
    let e = a.permitted_fields(Action::Read, &p).unwrap_err();
    assert!(
        matches!(&e, EvalError::NotLoaded { path, .. } if path == "tags"),
        "{e:?}"
    );
}

#[test]
fn schema_mismatch_guard() {
    let a = Ab::builder()
        .can(Action::Manage, Subject::All)
        .build()
        .unwrap();
    assert!(!a.can(Action::Read, &Imposter));
    let mismatch = |e: &EvalError| {
        matches!(
            e,
            EvalError::SchemaMismatch {
                expected: "Post",
                found: "Imposter",
                ..
            }
        )
    };
    assert_matches!(
        a.check(Action::Read, &Imposter),
        Err(CheckError::Unresolvable(e)) if mismatch(&e)
    );
    assert_matches!(
        a.permitted_fields(Action::Read, &Imposter),
        Err(e) if mismatch(&e)
    );
    let id = FieldRef::<Imposter>::new(0);
    assert!(!a.can_field(Action::Read, &Imposter, id));
    assert_matches!(
        a.check_field(Action::Read, &Imposter, id),
        Err(CheckError::Unresolvable(e)) if mismatch(&e)
    );
    // `.err()`: a `Projection<Imposter>` is not `Debug`.
    assert_matches!(
        a.projection::<Imposter>(Action::Read).err(),
        Some(e) if mismatch(&e)
    );
    assert_matches!(
        a.field_plan::<Imposter>(Action::Read),
        Err(e) if mismatch(&e)
    );
}
