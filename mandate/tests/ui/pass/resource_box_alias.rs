#[allow(dead_code, unused_imports)]
#[path = "../../common/fixture.rs"]
mod fixture;
use fixture::*;
use mandate::{CardinalityKind, FieldKind, Resource};

type MaybeUser = Option<Box<User>>;

#[derive(Resource)]
struct Doc {
    #[resource(relation)]
    owner: MaybeUser,
    #[resource(relation)]
    editors: Vec<Box<User>>,
    #[resource(rename = "headline")]
    title: String,
}

fn main() {
    let fields = Doc::schema().fields;
    let FieldKind::Relation { target, cardinality, nullable } = fields[Doc::OWNER.idx().0 as usize].kind else {
        panic!("owner is not a relation");
    };
    assert_eq!(cardinality, CardinalityKind::ToOne);
    assert!(nullable);
    assert!(std::ptr::eq(target(), User::schema()));
    let FieldKind::Relation { target, cardinality, .. } = fields[Doc::EDITORS.idx().0 as usize].kind else {
        panic!("editors is not a relation");
    };
    assert_eq!(cardinality, CardinalityKind::ToMany);
    assert!(std::ptr::eq(target(), User::schema()));
    assert_eq!(Doc::schema().index_of("headline"), Some(Doc::TITLE.idx()));
    assert_eq!(Doc::schema().index_of("title"), None);
}
