#![cfg(all(feature = "derive", feature = "chrono", feature = "uuid"))]
use mandate::{
    Cardinality, CardinalityKind, DynResource, FieldDef, FieldIdx, FieldMask, Kind, Nullability,
    RelationRef, RelationSlot, Resource, ResourcePtr, Schema, ValueRef,
};

struct Node {
    id: i64,
    children: Vec<Node>,
    parent: Option<Box<Node>>,
}

static NODE_SCHEMA: Schema = Schema::new(
    "Node",
    &[
        FieldDef::scalar("id", Kind::Int, false),
        FieldDef::relation(
            "children",
            <Node as Resource>::schema,
            CardinalityKind::ToMany,
            false,
        ),
        FieldDef::relation(
            "parent",
            <Node as Resource>::schema,
            CardinalityKind::ToOne,
            true,
        ),
    ],
);

impl Resource for Node {
    fn schema() -> &'static Schema {
        &NODE_SCHEMA
    }
    fn as_dyn(&self) -> &dyn DynResource {
        self
    }
}

impl ResourcePtr for Node {
    type Target = Node;
    fn target(&self) -> &Node {
        self
    }
}

impl DynResource for Node {
    fn resource_schema(&self) -> &'static Schema {
        &NODE_SCHEMA
    }
    fn value(&self, field: FieldIdx) -> ValueRef<'_> {
        match field.0 {
            0 => ValueRef::Int(self.id),
            _ => ValueRef::NotLoaded,
        }
    }
    fn relation(&self, field: FieldIdx) -> RelationRef<'_> {
        match field.0 {
            1 => RelationSlot::get(&self.children),
            2 => RelationSlot::get(&self.parent),
            _ => RelationRef::NotLoaded,
        }
    }
}

fn leaf(id: i64) -> Node {
    Node {
        id,
        children: vec![],
        parent: None,
    }
}

#[test]
fn option_box_slot_absent_and_one() {
    let n = leaf(1);
    assert!(matches!(n.relation(FieldIdx(2)), RelationRef::Absent));
    let n = Node {
        id: 1,
        children: vec![],
        parent: Some(Box::new(leaf(9))),
    };
    match n.relation(FieldIdx(2)) {
        RelationRef::One(d) => assert_eq!(d.value(FieldIdx(0)), ValueRef::Int(9)),
        _ => panic!("expected One"),
    }
}

#[test]
fn vec_slot_many() {
    let n = Node {
        id: 0,
        children: vec![leaf(1), leaf(2)],
        parent: None,
    };
    match n.relation(FieldIdx(1)) {
        RelationRef::Many(m) => {
            assert_eq!(m.len(), 2);
            assert_eq!(
                m.get(1).map(|d| d.value(FieldIdx(0))),
                Some(ValueRef::Int(2))
            );
            assert!(m.get(2).is_none());
        }
        _ => panic!("expected Many"),
    }
}

#[test]
fn slot_type_level_facts() {
    fn target_is_node<S: RelationSlot<Target = Node>>() {}
    target_is_node::<Option<Box<Node>>>();
    target_is_node::<Vec<Box<Node>>>();
    const { assert!(<<Option<Box<Node>> as RelationSlot>::Nullability as Nullability>::NULLABLE) };
    assert_eq!(
        <<Vec<Node> as RelationSlot>::Cardinality as Cardinality>::KIND,
        CardinalityKind::ToMany
    );
}

#[test]
fn field_mask_ops() {
    let mut m = FieldMask::default();
    assert!(m.is_empty());
    for i in [0u16, 5, 127] {
        m.insert(FieldIdx(i));
    }
    assert!(m.contains(FieldIdx(5)) && m.contains(FieldIdx(127)));
    assert_eq!(m.len(), 3);
    m.remove(FieldIdx(5));
    assert!(!m.contains(FieldIdx(5)));
    assert_eq!(
        m.iter().collect::<Vec<_>>(),
        vec![FieldIdx(0), FieldIdx(127)]
    );
    let all = FieldMask::all(3);
    assert_eq!(all.len(), 3);
    assert!(!m.is_subset(&all));
    let mut s = FieldMask::default();
    s.insert(FieldIdx(0));
    assert!(s.is_subset(&all) && s.is_subset(&m));
    assert_eq!(s.union(m), m);
    assert_eq!(FieldMask::all(128).len(), 128);
}
