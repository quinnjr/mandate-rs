#![cfg(all(feature = "derive", feature = "chrono", feature = "uuid"))]
mod common;
use common::fixture::*;
use mandate::{
    Ability, CheckError, DynResource, EvalError, FieldDef, FieldIdx, Kind, RelationRef, Resource,
    Schema, SubjectResource, ValueRef,
};

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
    assert!(a.check_type(Action::Read, Subject::Post).is_err());
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
    assert!(matches!(e, CheckError::Forbidden(_)));
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
        loaded: Loaded(vec![]),
        id: 1,
        author_id: 7,
        reviewer_id: None,
        title: "t".into(),
        status: Status::Published,
        score: 1.0,
        org: Lazy::NotLoaded,
        reviewer: Lazy::NotLoaded,
        tags: Lazy::NotLoaded,
    };
    assert!(!a.can(Action::Read, &p));
    assert_eq!(
        a.check(Action::Read, &p).unwrap_err(),
        CheckError::Unresolvable(EvalError::NotLoaded {
            path: "tags".into()
        })
    );
    assert!(a.permitted_fields(Action::Read, &p).is_err());
}

struct Imposter;
static IMPOSTER: Schema = Schema::new("Imposter", &[FieldDef::scalar("id", Kind::Int, false)]);
impl Resource for Imposter {
    fn schema() -> &'static Schema {
        &IMPOSTER
    }
    fn as_dyn(&self) -> &dyn DynResource {
        self
    }
}
impl DynResource for Imposter {
    fn resource_schema(&self) -> &'static Schema {
        &IMPOSTER
    }
    fn value(&self, _: FieldIdx) -> ValueRef<'_> {
        ValueRef::NotLoaded
    }
    fn relation(&self, _: FieldIdx) -> RelationRef<'_> {
        RelationRef::NotLoaded
    }
}
impl SubjectResource<Subject> for Imposter {
    const SUBJECT: Subject = Subject::Post;
}

#[test]
fn schema_mismatch_guard() {
    let a = Ab::builder()
        .can(Action::Manage, Subject::All)
        .build()
        .unwrap();
    assert!(!a.can(Action::Read, &Imposter));
    let mismatch = EvalError::SchemaMismatch {
        expected: "Post",
        found: "Imposter",
    };
    assert_eq!(
        a.check(Action::Read, &Imposter).unwrap_err(),
        CheckError::Unresolvable(mismatch.clone())
    );
    assert_eq!(
        a.permitted_fields(Action::Read, &Imposter).unwrap_err(),
        mismatch
    );
}
