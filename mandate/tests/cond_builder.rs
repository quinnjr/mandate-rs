#![cfg(all(feature = "derive", feature = "chrono", feature = "uuid"))]
mod common;
use common::fixture::*;
use mandate::{CmpOp, Cond, Condition, FieldIdx, Quant, StrOp, Value};

#[test]
fn scalar_ops_build_ast() {
    assert_eq!(Post::AUTHOR_ID.eq(5).condition(), &Condition::Cmp { field: FieldIdx(1), op: CmpOp::Eq, value: Value::Int(5) });
    assert_eq!(Post::TITLE.contains("x").condition(), &Condition::Str { field: FieldIdx(3), op: StrOp::Contains, value: "x".into() });
    assert_eq!(Post::REVIEWER_ID.is_null().condition(), &Condition::IsNull(FieldIdx(2)));
    assert_eq!(Post::STATUS.is_in([Status::Draft, Status::Archived]).condition(),
        &Condition::In { field: FieldIdx(6), values: vec![Value::String("draft".into()), Value::String("archived".into())] });
    assert_eq!(Post::TITLE.eq("hello").condition(), &Condition::Cmp { field: FieldIdx(3), op: CmpOp::Eq, value: Value::String("hello".into()) });
}

#[test]
fn relation_ops_build_ast() {
    assert_eq!(Post::ORG.then(Org::ID.eq(1)).into_condition(), Condition::Rel { relation: FieldIdx(9), quant: Quant::One,
        cond: Some(Box::new(Condition::Cmp { field: FieldIdx(0), op: CmpOp::Eq, value: Value::Int(1) })) });
    assert!(matches!(Post::TAGS.none(Tag::NAME.eq("x")).into_condition(), Condition::Rel { relation: FieldIdx(11), quant: Quant::None, .. }));
    assert_eq!(Post::REVIEWER.is_not_null().into_condition(), Condition::IsNotNull(FieldIdx(10)));
}

#[test]
fn combinators() {
    let (a, b) = (Post::LOCKED.eq(true), Post::SCORE.gt(1.5));
    assert_eq!(a.clone().and(b.clone()).into_condition(), Condition::And(vec![a.clone().into_condition(), b.clone().into_condition()]));
    assert_eq!(a.clone().or(b.clone()).into_condition(), Condition::Or(vec![a.clone().into_condition(), b.into_condition()]));
    assert_eq!((!a.clone()).into_condition(), Condition::Not(Box::new(a.into_condition())));
    assert_eq!(Cond::<Post>::all([]).into_condition(), Condition::And(vec![]));
    assert_eq!(Cond::<Post>::any([]).into_condition(), Condition::Or(vec![]));
}
