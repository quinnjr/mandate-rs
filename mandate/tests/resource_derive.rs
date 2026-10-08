#![cfg(all(feature = "derive", feature = "chrono", feature = "uuid"))]
mod common;
use common::fixture::*;
// `DynResource::resource_schema` avoids clashing with `Resource::schema`.
// `Post::schema()` is unambiguous even with `use mandate::*` (see `glob_import_schema`).
use mandate::{
    CardinalityKind, FieldDef, FieldIdx, FieldKind, Kind, RelationRef, RelationSlot, Resource,
    ResourcePtr, Schema, ValueRef,
};

fn scalar(f: &FieldDef) -> (Kind, bool) {
    match f.kind() {
        FieldKind::Scalar { kind, nullable } => (kind, nullable),
        other => panic!("`{}` is not a scalar: {other:?}", f.name()),
    }
}

fn relation(f: &FieldDef) -> (&'static Schema, CardinalityKind, bool) {
    match f.kind() {
        FieldKind::Relation {
            target,
            cardinality,
            nullable,
        } => (target(), cardinality, nullable),
        other => panic!("`{}` is not a relation: {other:?}", f.name()),
    }
}

#[test]
fn post_schema_shape() {
    let s = Post::schema();
    assert_eq!(s.name(), "Post");
    let names: Vec<_> = s.fields().iter().map(|f| f.name()).collect();
    assert_eq!(
        names,
        [
            "id",
            "author_id",
            "reviewer_id",
            "title",
            "body",
            "locked",
            "status",
            "published_at",
            "score",
            "org",
            "reviewer",
            "tags",
            "metadata"
        ]
    );
    let f = s.fields();
    assert_eq!(scalar(&f[1]), (Kind::Int, false));
    assert_eq!(scalar(&f[2]), (Kind::Int, true));
    assert_eq!(scalar(&f[3]), (Kind::String, false));
    assert_eq!(scalar(&f[5]), (Kind::Bool, false));
    assert_eq!(
        scalar(&f[6]),
        (Kind::Enum(&["draft", "published", "archived"]), false)
    );
    assert_eq!(scalar(&f[7]), (Kind::DateTime, true));
    assert_eq!(scalar(&f[8]), (Kind::Float, false));

    let (org, card, nullable) = relation(&f[9]);
    assert_eq!(
        (org.name(), card, nullable),
        ("Org", CardinalityKind::ToOne, false)
    );
    let (user, card, nullable) = relation(&f[10]);
    assert_eq!(
        (user.name(), card, nullable),
        ("User", CardinalityKind::ToOne, true)
    );
    let (tag, card, _) = relation(&f[11]);
    assert_eq!((tag.name(), card), ("Tag", CardinalityKind::ToMany));
    assert!(matches!(f[12].kind(), FieldKind::Opaque));

    assert_eq!(scalar(&Tag::schema().fields()[1]), (Kind::String, true));
}

#[test]
fn consts_match_indices() {
    assert_eq!(Post::ID.idx(), FieldIdx(0));
    assert_eq!(Post::AUTHOR_ID.idx(), FieldIdx(1));
    assert_eq!(Post::TITLE.idx(), FieldIdx(3));
    assert_eq!(Post::STATUS.idx(), FieldIdx(6));
    assert_eq!(Post::SCORE.idx(), FieldIdx(8));
    assert_eq!(Post::ORG.idx(), FieldIdx(9));
    assert_eq!(Post::REVIEWER.idx(), FieldIdx(10));
    assert_eq!(Post::TAGS.idx(), FieldIdx(11));
    assert_eq!(Post::METADATA.idx(), FieldIdx(12));
    assert_eq!(TPost::ID.idx(), FieldIdx(0));
    assert_eq!(TPost::SCORE.idx(), FieldIdx(5));
    assert_eq!(TPost::ORG.idx(), FieldIdx(6));
    assert_eq!(TPost::REVIEWER.idx(), FieldIdx(7));
    assert_eq!(TPost::TAGS.idx(), FieldIdx(8));
    // The handle types carry the declared field type.
    let _: mandate::Field<Post, Option<i64>> = Post::REVIEWER_ID;
    let _: mandate::Rel<Post, Option<User>> = Post::REVIEWER;
    let _: mandate::Opaque<Post> = Post::METADATA;
}

#[test]
fn dyn_values_and_relations() {
    let p = post();
    let d = p.as_dyn();
    assert!(std::ptr::eq(d.resource_schema(), Post::schema()));
    assert_eq!(d.value(Post::ID.idx()), ValueRef::Int(1));
    assert_eq!(d.value(Post::REVIEWER_ID.idx()), ValueRef::Null);
    assert_eq!(d.value(Post::TITLE.idx()), ValueRef::Str("Hello"));
    assert_eq!(d.value(Post::LOCKED.idx()), ValueRef::Bool(false));
    assert_eq!(d.value(Post::STATUS.idx()), ValueRef::Str("published"));
    assert_eq!(d.value(Post::PUBLISHED_AT.idx()), ValueRef::Null);
    assert_eq!(d.value(Post::SCORE.idx()), ValueRef::Float(1.0));
    match d.relation(Post::ORG.idx()) {
        RelationRef::One(o) => {
            assert!(std::ptr::eq(o.resource_schema(), Org::schema()));
            assert_eq!(o.value(Org::ID.idx()), ValueRef::Int(3));
        }
        _ => panic!("expected One"),
    }
    assert!(matches!(
        d.relation(Post::REVIEWER.idx()),
        RelationRef::Absent
    ));
    assert!(matches!(d.relation(Post::TAGS.idx()), RelationRef::Many(m) if m.is_empty()));

    // Non-scalar and out-of-range indices never panic.
    for idx in [
        Post::ORG.idx(),
        Post::METADATA.idx(),
        FieldIdx(13),
        FieldIdx(u16::MAX),
    ] {
        assert_eq!(d.value(idx), ValueRef::Null);
    }
    for idx in [
        Post::TITLE.idx(),
        Post::METADATA.idx(),
        FieldIdx(13),
        FieldIdx(u16::MAX),
    ] {
        assert!(matches!(d.relation(idx), RelationRef::Absent));
    }

    let mut p = post();
    p.reviewer = Some(User {
        id: 5,
        name: "Ann".into(),
        email: "ann@example.com".into(),
        posts: vec![],
    });
    p.tags = vec![
        Tag { id: 1, name: None },
        Tag {
            id: 2,
            name: Some("rust".into()),
        },
    ];
    let d = p.as_dyn();
    match d.relation(Post::REVIEWER.idx()) {
        RelationRef::One(u) => assert_eq!(u.value(User::NAME.idx()), ValueRef::Str("Ann")),
        _ => panic!("expected One"),
    }
    match d.relation(Post::TAGS.idx()) {
        RelationRef::Many(m) => {
            assert_eq!(m.len(), 2);
            assert_eq!(
                m.get(0).map(|t| t.value(Tag::NAME.idx())),
                Some(ValueRef::Null)
            );
            assert_eq!(
                m.get(1).map(|t| t.value(Tag::NAME.idx())),
                Some(ValueRef::Str("rust"))
            );
        }
        _ => panic!("expected Many"),
    }
}

#[test]
fn resource_is_its_own_slot_and_pointer() {
    fn slot<S: RelationSlot<Target = Org>>() {}
    slot::<Org>();
    let o = Org {
        id: 3,
        name: "Acme".into(),
    };
    assert!(std::ptr::eq(ResourcePtr::target(&o), &o));
    match RelationSlot::get(&o) {
        RelationRef::One(d) => assert_eq!(d.value(Org::NAME.idx()), ValueRef::Str("Acme")),
        _ => panic!("expected One"),
    }
}

#[test]
fn schema_identity_and_cycles() {
    assert!(std::ptr::eq(Post::schema(), Post::schema()));
    let FieldKind::Relation { target, .. } = Post::schema().fields()[9].kind() else {
        panic!()
    };
    assert!(std::ptr::eq(target(), Org::schema()));
    // Post.reviewer -> User, User.posts -> Post.
    let (user, _, _) = relation(&Post::schema().fields()[10]);
    assert!(std::ptr::eq(user, User::schema()));
    let (posts, card, _) = relation(&User::schema().fields()[3]);
    assert!(std::ptr::eq(posts, Post::schema()));
    assert_eq!(card, CardinalityKind::ToMany);
}

#[test]
fn load_state_reports_not_loaded() {
    let t = TPost {
        loaded: Loaded(vec!["author_id"]),
        id: 1,
        author_id: 7,
        reviewer_id: None,
        title: "T".into(),
        status: Status::Draft,
        score: 0.5,
        org: Lazy::NotLoaded,
        reviewer: Lazy::Loaded(None),
        tags: Lazy::Loaded(vec![]),
    };
    let d = t.as_dyn();
    assert_eq!(d.value(FieldIdx(1)), ValueRef::NotLoaded);
    assert_eq!(d.value(FieldIdx(0)), ValueRef::Int(1));
    assert_eq!(d.value(FieldIdx(4)), ValueRef::Str("draft"));
    let s = TPost::schema();
    assert_eq!(s.index_of("loaded"), None);
    assert_eq!(s.fields().len(), 9);
    assert!(matches!(d.relation(FieldIdx(6)), RelationRef::NotLoaded));
    assert!(matches!(d.relation(FieldIdx(7)), RelationRef::Absent));
    assert!(matches!(d.relation(FieldIdx(8)), RelationRef::Many(m) if m.is_empty()));

    // Lazy slots classify like the slot they wrap.
    let (org, card, nullable) = relation(&s.fields()[6]);
    assert_eq!(
        (org.name(), card, nullable),
        ("TOrg", CardinalityKind::ToOne, false)
    );
    let (user, card, nullable) = relation(&s.fields()[7]);
    assert_eq!(
        (user.name(), card, nullable),
        ("TUser", CardinalityKind::ToOne, true)
    );
    let (tag, card, _) = relation(&s.fields()[8]);
    assert_eq!((tag.name(), card), ("TTag", CardinalityKind::ToMany));

    // A loaded relation whose target has unloaded scalars.
    let t = TPost {
        org: Lazy::Loaded(TOrg {
            loaded: Loaded(vec!["name"]),
            id: 3,
            name: String::new(),
        }),
        ..t
    };
    match t.as_dyn().relation(TPost::ORG.idx()) {
        RelationRef::One(o) => {
            assert_eq!(o.value(TOrg::ID.idx()), ValueRef::Int(3));
            assert_eq!(o.value(TOrg::NAME.idx()), ValueRef::NotLoaded);
        }
        _ => panic!("expected One"),
    }
}

#[test]
fn fieldless_resource() {
    assert!(Marker::schema().fields().is_empty());
    let m = Marker {};
    assert_eq!(m.as_dyn().value(FieldIdx(0)), ValueRef::Null);
    assert!(matches!(
        m.as_dyn().relation(FieldIdx(0)),
        RelationRef::Absent
    ));
}

#[derive(Resource)]
#[resource(load_state = loaded)]
struct Renamed {
    loaded: Loaded,
    r#type: String,
    #[resource(rename = "headline")]
    title: String,
}

#[test]
fn raw_idents_and_renames() {
    let s = Renamed::schema();
    assert_eq!(s.name(), "Renamed");
    assert_eq!(s.index_of("type"), Some(Renamed::TYPE.idx()));
    assert_eq!(s.index_of("headline"), Some(Renamed::TITLE.idx()));
    assert_eq!(s.index_of("title"), None);
    // Load state is queried by schema (rule) name.
    let r = Renamed {
        loaded: Loaded(vec!["headline"]),
        r#type: "a".into(),
        title: "b".into(),
    };
    assert_eq!(r.as_dyn().value(Renamed::TYPE.idx()), ValueRef::Str("a"));
    assert_eq!(r.as_dyn().value(Renamed::TITLE.idx()), ValueRef::NotLoaded);
}

#[test]
fn glob_import_schema() {
    mod glob {
        use super::Post;
        use mandate::*;
        pub fn run() -> bool {
            std::ptr::eq(Post::schema(), Post::schema())
        }
    }
    assert!(glob::run());
}
