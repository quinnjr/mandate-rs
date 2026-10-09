#![cfg(all(feature = "derive", feature = "chrono", feature = "uuid"))]
mod common;
use common::fixture::*;
use mandate::{Ability, FieldIdx, FieldMask, RelationProjection, Resource};

type Ab = Ability<Action, Subject>;

fn mask(ix: &[u16]) -> FieldMask {
    let mut m = FieldMask::default();
    for &i in ix {
        m.insert(FieldIdx(i));
    }
    m
}

/// A [`RelationProjection`] (which is `#[non_exhaustive]`) as plain data,
/// with the target schema by name.
#[derive(Debug, PartialEq)]
struct Dep {
    relation: FieldIdx,
    target: &'static str,
    fields: FieldMask,
    relations: Vec<Dep>,
}

/// `r` as [`Dep`]s, recursively.
fn deps(r: &[RelationProjection]) -> Vec<Dep> {
    r.iter()
        .map(|r| Dep {
            relation: r.relation,
            target: r.target.name(),
            fields: r.fields,
            relations: deps(&r.relations),
        })
        .collect()
}

#[test]
fn condition_dependencies_are_fetched() {
    let a = Ab::builder()
        .can(Action::Read, Subject::Post)
        .fields([Post::TITLE.into(), Post::BODY.into()])
        .cannot(Action::Read, Subject::Post)
        .fields([Post::BODY.into()])
        .when(Post::STATUS.eq(Status::Draft))
        .build()
        .unwrap();
    let p = a.projection::<Post>(Action::Read).unwrap();
    assert_eq!(p.fields.mask(), mask(&[3, 4, 6]));
    assert!(p.relations.is_empty());
}

#[test]
fn relation_dependencies_carry_only_read_fields() {
    let a = Ab::builder()
        .can(Action::Read, Subject::Post)
        .cannot(Action::Read, Subject::Post)
        .when(Post::REVIEWER.then(User::EMAIL.ends_with("@corp")))
        .build()
        .unwrap();
    let p = a.projection::<Post>(Action::Read).unwrap();
    assert_eq!(
        deps(&p.relations),
        [Dep {
            relation: Post::REVIEWER.idx(),
            target: "User",
            fields: mask(&[User::EMAIL.idx().0]),
            relations: vec![],
        }]
    );
}

#[test]
fn permitted_relations_are_not_loaded() {
    let a = Ab::builder()
        .can(Action::Read, Subject::Post)
        .build()
        .unwrap();
    let p = a.projection::<Post>(Action::Read).unwrap();
    assert_eq!(
        p.fields.mask(),
        mask(&[0, 1, 2, 3, 4, 5, 6, 7, 8, 12, 13, 14])
    );
    assert!(p.relations.is_empty());
}

#[test]
fn unconditional_cannot_removes() {
    let a = Ab::builder()
        .can(Action::Read, Subject::Post)
        .cannot(Action::Read, Subject::Post)
        .fields([Post::BODY.into()])
        .build()
        .unwrap();
    let p = a.projection::<Post>(Action::Read).unwrap();
    assert!(!p.fields.contains(Post::BODY));
    assert!(p.fields.contains(Post::TITLE));
}

#[test]
fn cyclic_paths_terminate() {
    let a = Ab::builder()
        .can(Action::Read, Subject::Post)
        .cannot(Action::Read, Subject::Post)
        .when(Post::REVIEWER.then(User::POSTS.some(Post::LOCKED.eq(true))))
        .build()
        .unwrap();
    let p = a.projection::<Post>(Action::Read).unwrap();
    assert_eq!(
        deps(&p.relations),
        [Dep {
            relation: Post::REVIEWER.idx(),
            target: "User",
            fields: FieldMask::default(),
            relations: vec![Dep {
                relation: User::POSTS.idx(),
                target: "Post",
                fields: mask(&[Post::LOCKED.idx().0]),
                relations: vec![],
            }],
        }]
    );
}

#[test]
fn relation_entries_merge_and_sort() {
    let a = Ab::builder()
        .can(Action::Read, Subject::Post)
        .cannot(Action::Read, Subject::Post)
        .when(
            Post::REVIEWER
                .then(User::NAME.eq("a"))
                .and(Post::ORG.then(Org::ID.eq(1))),
        )
        .cannot(Action::Read, Subject::Post)
        .when(Post::REVIEWER.then(User::EMAIL.eq("b")))
        .build()
        .unwrap();
    let p = a.projection::<Post>(Action::Read).unwrap();
    let rels: Vec<_> = p
        .relations
        .iter()
        .map(|r| (r.relation.0, r.fields))
        .collect();
    assert_eq!(rels, vec![(9, mask(&[0])), (10, mask(&[1, 2]))]);
}

#[test]
fn projection_of_fieldless_resource() {
    let a = Ab::builder()
        .can(Action::Read, Subject::Marker)
        .build()
        .unwrap();
    let p = a.projection::<Marker>(Action::Read).unwrap();
    assert!(p.fields.mask().is_empty());
    assert!(p.relations.is_empty());
}

#[test]
fn projection_suffices_for_checks() {
    use mandate::FieldKind;
    let a = Ability::<Action, TSubject>::builder()
        .can(Action::Read, TSubject::TPost)
        .fields([TPost::TITLE.into()])
        .can(Action::Read, TSubject::TPost)
        .fields([TPost::TITLE.into()])
        .when(TPost::TAGS.some(TTag::ID.eq(1)))
        .cannot(Action::Read, TSubject::TPost)
        .fields([TPost::TITLE.into()])
        .when(TPost::AUTHOR_ID.eq(7))
        .build()
        .unwrap();
    let p = a.projection::<TPost>(Action::Read).unwrap();
    let schema = TPost::schema();
    let not_loaded: Vec<&'static str> = schema
        .fields()
        .iter()
        .enumerate()
        .filter(|(i, d)| {
            matches!(d.kind(), FieldKind::Scalar { .. })
                && !p.fields.mask().contains(FieldIdx(*i as u16))
        })
        .map(|(_, d)| d.name())
        .collect();
    assert!(not_loaded.contains(&"score"));
    assert!(!not_loaded.contains(&"author_id"));
    assert_eq!(p.relations.len(), 1);
    assert_eq!(p.relations[0].relation, TPost::TAGS.idx());
    let t = TPost {
        loaded: Loaded(not_loaded),
        title: "t".into(),
        org: Lazy::NotLoaded,
        reviewer: Lazy::NotLoaded,
        tags: Lazy::Loaded(vec![TTag {
            loaded: Loaded(vec!["name"]),
            id: 1,
            name: None,
            author: Lazy::Loaded(None),
        }]),
        ..loaded_tpost()
    };
    assert!(a.permitted_fields(Action::Read, &t).is_ok());
    assert!(a.can(Action::Read, &t));
}

#[test]
fn relation_null_tests_are_relation_dependencies() {
    let a = Ab::builder()
        .can(Action::Read, Subject::Post)
        .cannot(Action::Read, Subject::Post)
        .when(Post::REVIEWER.is_null())
        .build()
        .unwrap();
    let p = a.projection::<Post>(Action::Read).unwrap();
    assert!(!p.fields.contains(Post::REVIEWER));
    assert_eq!(
        deps(&p.relations),
        [Dep {
            relation: Post::REVIEWER.idx(),
            target: "User",
            fields: FieldMask::default(),
            relations: vec![],
        }]
    );
}

#[test]
fn relation_null_test_check_runs_on_projection() {
    let a = Ability::<Action, TSubject>::builder()
        .can(Action::Read, TSubject::TPost)
        .fields([TPost::TITLE.into()])
        .cannot(Action::Read, TSubject::TPost)
        .when(TPost::REVIEWER.is_null())
        .build()
        .unwrap();
    let p = a.projection::<TPost>(Action::Read).unwrap();
    assert_eq!(p.relations.len(), 1);
    assert_eq!(p.relations[0].relation, TPost::REVIEWER.idx());
    // Only `title` (the permitted field) is fetched.
    let t = TPost {
        loaded: Loaded(vec![
            "id",
            "author_id",
            "reviewer_id",
            "status",
            "score",
            "owner",
            "due",
        ]),
        title: "t".into(),
        org: Lazy::NotLoaded,
        reviewer: Lazy::Loaded(None),
        tags: Lazy::NotLoaded,
        ..loaded_tpost()
    };
    assert!(a.permitted_fields(Action::Read, &t).is_ok());
    assert!(!a.can(Action::Read, &t));
}
