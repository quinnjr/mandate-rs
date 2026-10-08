//! Constant folding and canonical form (spec §5.1).
//!
//! Folding replaces constant subconditions and quantifiers over constants
//! with their value; canonical form flattens and deduplicates groups so that
//! equivalent rule sets compare and serialise identically. Both preserve the
//! §5.3 semantics exactly.

use super::{Condition, Quant};
#[cfg(feature = "chrono")]
use crate::truncate_micros;
use crate::{CardinalityKind, FieldIdx, FieldKind, Schema, Value};

/// A folded condition: a constant, or a non-constant condition in canonical
/// form.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Folded {
    /// Always holds.
    True,
    /// Never holds.
    False,
    /// Depends on the resource.
    Cond(Condition),
}

/// Folds constants out of `c` and puts it in canonical form; `schema` is the
/// schema `c`'s field indices refer to. Idempotent.
///
/// `DateTime` operands are truncated to microseconds (§5.2).
pub(crate) fn fold(c: Condition, schema: &'static Schema) -> Folded {
    match c {
        Condition::Cmp { field, op, value } => Folded::Cond(Condition::Cmp {
            field,
            op,
            value: operand(value),
        }),
        Condition::In { values, .. } if values.is_empty() => Folded::False,
        Condition::In { field, values } => Folded::Cond(Condition::In {
            field,
            values: values.into_iter().map(operand).collect(),
        }),
        Condition::NotIn { values, .. } if values.is_empty() => Folded::True,
        Condition::NotIn { field, values } => Folded::Cond(Condition::NotIn {
            field,
            values: values.into_iter().map(operand).collect(),
        }),
        c @ (Condition::Str { .. } | Condition::IsNull(_) | Condition::IsNotNull(_)) => {
            Folded::Cond(c)
        }
        Condition::And(cs) => group(cs, true, schema),
        Condition::Or(cs) => group(cs, false, schema),
        Condition::Not(c) => match fold(*c, schema) {
            Folded::True => Folded::False,
            Folded::False => Folded::True,
            // A canonical child is never itself a double negation.
            Folded::Cond(Condition::Not(c)) => Folded::Cond(*c),
            Folded::Cond(c) => Folded::Cond(Condition::Not(Box::new(c))),
        },
        Condition::Rel {
            relation,
            quant,
            cond,
        } => rel(relation, quant, cond, schema),
    }
}

/// Folds the children of an `And` (`and`) or `Or` (`!and`) group.
///
/// The group's identity (true for `And`, false for `Or`) is dropped and its
/// absorbing constant decides it. Children of the same kind are flattened
/// into it, and later duplicates of a canonical child are removed, keeping
/// the first; neither changes which child decides a short-circuit.
fn group(children: Vec<Condition>, and: bool, schema: &'static Schema) -> Folded {
    let mut out: Vec<Condition> = Vec::with_capacity(children.len());
    for child in children {
        let child = match fold(child, schema) {
            Folded::Cond(c) => c,
            Folded::True if and => continue,
            Folded::False if !and => continue,
            absorbing => return absorbing,
        };
        match (and, child) {
            (true, Condition::And(cs)) | (false, Condition::Or(cs)) => {
                for c in cs {
                    push_unique(&mut out, c);
                }
            }
            (_, c) => push_unique(&mut out, c),
        }
    }
    match out.len() {
        0 if and => Folded::True,
        0 => Folded::False,
        1 => Folded::Cond(out.swap_remove(0)),
        _ if and => Folded::Cond(Condition::And(out)),
        _ => Folded::Cond(Condition::Or(out)),
    }
}

fn push_unique(out: &mut Vec<Condition>, c: Condition) {
    if !out.contains(&c) {
        out.push(c);
    }
}

/// Folds a quantifier, recursing into its condition with the target schema.
fn rel(
    relation: FieldIdx,
    quant: Quant,
    cond: Option<Box<Condition>>,
    schema: &'static Schema,
) -> Folded {
    let Some(FieldKind::Relation {
        target,
        cardinality,
        nullable,
    }) = schema.field(relation).map(|def| def.kind())
    else {
        // Not a relation (unreachable after validation): there is no target
        // schema to fold the condition against.
        return Folded::Cond(Condition::Rel {
            relation,
            quant,
            cond,
        });
    };
    let quant = match cond.map(|c| fold(*c, target())) {
        Some(Folded::Cond(c)) => {
            return Folded::Cond(Condition::Rel {
                relation,
                quant,
                cond: Some(Box::new(c)),
            });
        }
        None | Some(Folded::True) => quant,
        // No related row satisfies the condition.
        Some(Folded::False) => match quant {
            Quant::One | Quant::Some => return Folded::False,
            Quant::None => return Folded::True,
            // Every row fails: true exactly when there is no row.
            Quant::Every => Quant::None,
        },
    };
    // The condition is true, so only whether a row exists matters. A
    // non-nullable to-one is never assumed to have its row (§5.1), and a
    // to-many may be empty, so those stay quantifiers.
    let nullable_to_one = nullable && cardinality == CardinalityKind::ToOne;
    match quant {
        Quant::Every => Folded::True,
        Quant::One if nullable_to_one => Folded::Cond(Condition::IsNotNull(relation)),
        Quant::None if nullable_to_one => Folded::Cond(Condition::IsNull(relation)),
        quant => Folded::Cond(Condition::Rel {
            relation,
            quant,
            cond: None,
        }),
    }
}

/// Normalises an operand: `DateTime`s are truncated to microseconds (§5.2).
fn operand(v: Value) -> Value {
    #[cfg(feature = "chrono")]
    if let Value::DateTime(dt) = v {
        return Value::DateTime(truncate_micros(dt));
    }
    v
}

#[cfg(all(test, feature = "derive", feature = "chrono", feature = "uuid"))]
mod tests {
    use chrono::{DateTime, Utc};

    use super::{Folded, fold};
    use crate::condition::eval::eval;
    use crate::test_fixture::*;
    use crate::{CmpOp, Condition, EvalError, FieldIdx, Quant, Resource, Value};

    use Condition::{And, IsNotNull, IsNull, Not, Or};

    /// Inputs paired with their expected fold.
    type Cases = Vec<(Condition, Folded)>;

    // Post fields (see the fixture).
    const TITLE: FieldIdx = FieldIdx(3);
    const PUBLISHED_AT: FieldIdx = FieldIdx(7);
    /// To-one, non-nullable (→ Org).
    const ORG: FieldIdx = FieldIdx(9);
    /// To-one, nullable (→ User).
    const REVIEWER: FieldIdx = FieldIdx(10);
    /// To-many (→ Tag).
    const TAGS: FieldIdx = FieldIdx(11);
    /// `User.posts`: to-many (→ Post).
    const USER_POSTS: FieldIdx = FieldIdx(3);

    /// The constant true.
    fn t() -> Condition {
        And(vec![])
    }

    /// The constant false.
    fn f() -> Condition {
        Or(vec![])
    }

    /// Distinct non-constant Post leaves.
    fn a() -> Condition {
        Post::AUTHOR_ID.eq(7).into_condition()
    }

    fn b() -> Condition {
        Post::LOCKED.eq(true).into_condition()
    }

    fn c() -> Condition {
        Post::TITLE.contains("ell").into_condition()
    }

    /// A Tag leaf.
    fn tag_id(id: i64) -> Condition {
        Tag::ID.eq(id).into_condition()
    }

    fn not(c: Condition) -> Condition {
        Not(Box::new(c))
    }

    fn rel(relation: FieldIdx, quant: Quant, cond: Option<Condition>) -> Condition {
        Condition::Rel {
            relation,
            quant,
            cond: cond.map(Box::new),
        }
    }

    fn cond(c: Condition) -> Folded {
        Folded::Cond(c)
    }

    fn fold_post(c: Condition) -> Folded {
        fold(c, Post::schema())
    }

    fn check(cases: Cases) {
        for (input, expected) in cases {
            assert_eq!(fold_post(input.clone()), expected, "folding {input:?}");
        }
    }

    fn dt(s: &str) -> DateTime<Utc> {
        s.parse().unwrap()
    }

    /// `And([])` / `Or([])` → `t` / `f`.
    fn empty_groups() -> Cases {
        vec![(t(), Folded::True), (f(), Folded::False)]
    }

    /// `And` containing `f` / `Or` containing `t` → `f` / `t`; other
    /// constant children are dropped.
    fn constant_children() -> Cases {
        vec![
            (And(vec![a(), f(), b()]), Folded::False),
            (Or(vec![a(), t(), b()]), Folded::True),
            (And(vec![t(), a(), t()]), cond(a())),
            (Or(vec![f(), a(), f()]), cond(a())),
            (And(vec![t(), a(), b()]), cond(And(vec![a(), b()]))),
            (Or(vec![a(), f(), b()]), cond(Or(vec![a(), b()]))),
            (And(vec![t(), t()]), Folded::True),
            (Or(vec![f(), f()]), Folded::False),
            // Constants produced by folding a child.
            (And(vec![a(), Or(vec![b(), f()]), not(t())]), Folded::False),
            (Or(vec![a(), not(f())]), Folded::True),
            (And(vec![Or(vec![b(), t()]), a()]), cond(a())),
        ]
    }

    /// `Not(t)` / `Not(f)` → `f` / `t`.
    fn negated_constants() -> Cases {
        vec![
            (not(t()), Folded::False),
            (not(f()), Folded::True),
            (not(And(vec![a(), f()])), Folded::True),
            (not(Or(vec![t(), a()])), Folded::False),
        ]
    }

    /// `In([])` / `NotIn([])` → `f` / `t`.
    fn empty_lists() -> Cases {
        let none = Vec::<i64>::new;
        vec![
            (
                Post::AUTHOR_ID.is_in(none()).into_condition(),
                Folded::False,
            ),
            (
                Post::AUTHOR_ID.not_in(none()).into_condition(),
                Folded::True,
            ),
            // Also on a nullable field, where the instance may be null.
            (
                Post::REVIEWER_ID.is_in(none()).into_condition(),
                Folded::False,
            ),
            (
                Post::REVIEWER_ID.not_in(none()).into_condition(),
                Folded::True,
            ),
            // Non-empty lists are kept.
            (
                Post::AUTHOR_ID.is_in([7]).into_condition(),
                cond(Post::AUTHOR_ID.is_in([7]).into_condition()),
            ),
            (
                Post::AUTHOR_ID.not_in([7]).into_condition(),
                cond(Post::AUTHOR_ID.not_in([7]).into_condition()),
            ),
        ]
    }

    /// `Rel{q, t}` → stored as `cond: None`.
    fn true_relation_conditions() -> Cases {
        vec![
            (
                rel(TAGS, Quant::Some, Some(t())),
                cond(rel(TAGS, Quant::Some, None)),
            ),
            (
                rel(TAGS, Quant::None, Some(t())),
                cond(rel(TAGS, Quant::None, None)),
            ),
            (
                rel(ORG, Quant::One, Some(t())),
                cond(rel(ORG, Quant::One, None)),
            ),
            (
                rel(ORG, Quant::None, Some(t())),
                cond(rel(ORG, Quant::None, None)),
            ),
            // A condition that only folds to true.
            (
                rel(TAGS, Quant::Some, Some(Or(vec![tag_id(1), not(f())]))),
                cond(rel(TAGS, Quant::Some, None)),
            ),
        ]
    }

    /// `Rel{One, None}` on a nullable to-one → `IsNotNull(rel)`.
    fn present_nullable_to_one() -> Cases {
        vec![
            (
                rel(REVIEWER, Quant::One, Some(t())),
                cond(IsNotNull(REVIEWER)),
            ),
            (rel(REVIEWER, Quant::One, None), cond(IsNotNull(REVIEWER))),
        ]
    }

    /// `Rel{None, None}` on a nullable to-one → `IsNull(rel)`.
    fn absent_nullable_to_one() -> Cases {
        vec![
            (rel(REVIEWER, Quant::None, None), cond(IsNull(REVIEWER))),
            (
                rel(REVIEWER, Quant::None, Some(t())),
                cond(IsNull(REVIEWER)),
            ),
        ]
    }

    /// `Rel{Every, None}` → `t`.
    fn every_with_true() -> Cases {
        vec![
            (rel(TAGS, Quant::Every, None), Folded::True),
            (rel(TAGS, Quant::Every, Some(t())), Folded::True),
        ]
    }

    /// `Rel{One, f}` / `Rel{Some, f}` → `f`.
    fn one_or_some_with_false() -> Cases {
        vec![
            (rel(ORG, Quant::One, Some(f())), Folded::False),
            (rel(REVIEWER, Quant::One, Some(f())), Folded::False),
            (rel(TAGS, Quant::Some, Some(f())), Folded::False),
            (
                rel(TAGS, Quant::Some, Some(And(vec![tag_id(1), f()]))),
                Folded::False,
            ),
        ]
    }

    /// `Rel{None, f}` → `t`.
    fn none_with_false() -> Cases {
        vec![
            (rel(TAGS, Quant::None, Some(f())), Folded::True),
            (rel(ORG, Quant::None, Some(f())), Folded::True),
            (rel(REVIEWER, Quant::None, Some(f())), Folded::True),
        ]
    }

    /// `Rel{Every, f}` → `Rel{None, None}`, then the rows above.
    fn every_with_false() -> Cases {
        vec![
            (
                rel(TAGS, Quant::Every, Some(f())),
                cond(rel(TAGS, Quant::None, None)),
            ),
            (
                rel(TAGS, Quant::Every, Some(not(t()))),
                cond(rel(TAGS, Quant::None, None)),
            ),
            // `Every` is to-many only, but the result must still be canonical.
            (
                rel(REVIEWER, Quant::Every, Some(f())),
                cond(IsNull(REVIEWER)),
            ),
        ]
    }

    /// Non-nullable to-ones are never assumed present, and to-manys may be
    /// empty.
    fn unfolded_presence() -> Cases {
        [
            rel(ORG, Quant::One, None),
            rel(ORG, Quant::None, None),
            rel(TAGS, Quant::Some, None),
            rel(TAGS, Quant::None, None),
        ]
        .into_iter()
        .map(|c| (c.clone(), cond(c)))
        .collect()
    }

    fn canonical() -> Cases {
        vec![
            (
                And(vec![a(), And(vec![b(), a()])]),
                cond(And(vec![a(), b()])),
            ),
            (Or(vec![c()]), cond(c())),
            (not(not(c())), cond(c())),
            (And(vec![a()]), cond(a())),
            (
                Or(vec![a(), Or(vec![b(), Or(vec![c(), a()])])]),
                cond(Or(vec![a(), b(), c()])),
            ),
            // Groups of the other kind are kept, and order is preserved.
            (
                Or(vec![b(), And(vec![c(), a()])]),
                cond(Or(vec![b(), And(vec![c(), a()])])),
            ),
            (
                And(vec![b(), Or(vec![a()]), c()]),
                cond(And(vec![b(), a(), c()])),
            ),
            (And(vec![a(), a()]), cond(a())),
            (
                Or(vec![And(vec![a(), b()]), And(vec![a(), b()])]),
                cond(And(vec![a(), b()])),
            ),
            (not(not(not(c()))), cond(not(c()))),
            (not(And(vec![not(not(a()))])), cond(not(a()))),
            // Duplicates are found after canonicalising the children.
            (And(vec![a(), not(not(a()))]), cond(a())),
            (
                And(vec![Or(vec![a(), b()]), Or(vec![a(), Or(vec![b()])])]),
                cond(Or(vec![a(), b()])),
            ),
        ]
    }

    /// Inner conditions are folded against the relation's target schema.
    fn relation_scopes() -> Cases {
        vec![
            (
                rel(
                    TAGS,
                    Quant::Some,
                    Some(And(vec![tag_id(1), t(), tag_id(1)])),
                ),
                cond(rel(TAGS, Quant::Some, Some(tag_id(1)))),
            ),
            // Index 3 is `User.posts` here, not `Post.title`.
            (
                rel(
                    REVIEWER,
                    Quant::One,
                    Some(rel(USER_POSTS, Quant::Every, Some(f()))),
                ),
                cond(rel(
                    REVIEWER,
                    Quant::One,
                    Some(rel(USER_POSTS, Quant::None, None)),
                )),
            ),
            (
                rel(
                    REVIEWER,
                    Quant::One,
                    Some(rel(
                        USER_POSTS,
                        Quant::Some,
                        Some(rel(REVIEWER, Quant::One, None)),
                    )),
                ),
                cond(rel(
                    REVIEWER,
                    Quant::One,
                    Some(rel(USER_POSTS, Quant::Some, Some(IsNotNull(REVIEWER)))),
                )),
            ),
            // A folded inner condition can fold the outer quantifier.
            (
                rel(
                    REVIEWER,
                    Quant::One,
                    Some(rel(USER_POSTS, Quant::Every, None)),
                ),
                cond(IsNotNull(REVIEWER)),
            ),
        ]
    }

    /// A `Rel` on a field that is not a relation (rejected by validation) has
    /// no target schema, so it is kept as is.
    fn non_relations() -> Cases {
        [
            rel(TITLE, Quant::Some, Some(t())),
            rel(FieldIdx(99), Quant::One, None),
        ]
        .into_iter()
        .map(|c| (c.clone(), cond(c)))
        .collect()
    }

    fn datetimes() -> Cases {
        let nanos = dt("2020-01-15T10:30:00.123456789Z");
        let micros = Value::DateTime(dt("2020-01-15T10:30:00.123456Z"));
        let cmp = |op, value| Condition::Cmp {
            field: PUBLISHED_AT,
            op,
            value,
        };
        vec![
            (
                Post::PUBLISHED_AT.gt(nanos).into_condition(),
                cond(cmp(CmpOp::Gt, micros.clone())),
            ),
            (
                Post::PUBLISHED_AT.is_in([nanos]).into_condition(),
                cond(Condition::In {
                    field: PUBLISHED_AT,
                    values: vec![micros.clone()],
                }),
            ),
            (
                Post::PUBLISHED_AT.not_in([nanos]).into_condition(),
                cond(Condition::NotIn {
                    field: PUBLISHED_AT,
                    values: vec![micros.clone()],
                }),
            ),
            // Equal once truncated, so deduplicated.
            (
                Or(vec![
                    Post::PUBLISHED_AT.eq(nanos).into_condition(),
                    cmp(CmpOp::Eq, micros.clone()),
                ]),
                cond(cmp(CmpOp::Eq, micros)),
            ),
        ]
    }

    fn all_cases() -> Cases {
        [
            empty_groups(),
            constant_children(),
            negated_constants(),
            empty_lists(),
            true_relation_conditions(),
            present_nullable_to_one(),
            absent_nullable_to_one(),
            every_with_true(),
            one_or_some_with_false(),
            none_with_false(),
            every_with_false(),
            unfolded_presence(),
            canonical(),
            relation_scopes(),
            non_relations(),
            datetimes(),
        ]
        .concat()
    }

    #[test]
    fn empty_and_or_are_constants() {
        check(empty_groups());
    }

    #[test]
    fn and_false_or_true_absorb_and_other_constants_drop() {
        check(constant_children());
    }

    #[test]
    fn not_of_constant_is_constant() {
        check(negated_constants());
    }

    #[test]
    fn empty_in_is_false_and_empty_not_in_is_true() {
        check(empty_lists());
    }

    #[test]
    fn rel_with_true_is_stored_as_none() {
        check(true_relation_conditions());
    }

    #[test]
    fn nullable_to_one_rel_with_true_becomes_is_not_null() {
        check(present_nullable_to_one());
    }

    #[test]
    fn nullable_to_one_none_with_true_becomes_is_null() {
        check(absent_nullable_to_one());
    }

    #[test]
    fn every_with_true_is_true() {
        check(every_with_true());
    }

    #[test]
    fn one_or_some_with_false_is_false() {
        check(one_or_some_with_false());
    }

    #[test]
    fn none_with_false_is_true() {
        check(none_with_false());
    }

    #[test]
    fn every_with_false_is_none_of_true() {
        check(every_with_false());
    }

    #[test]
    fn non_nullable_to_one_not_folded() {
        check(unfolded_presence());
    }

    #[test]
    fn canonical_form() {
        check(canonical());
    }

    #[test]
    fn folds_inside_relation_scopes() {
        check(relation_scopes());
    }

    #[test]
    fn rel_on_non_relation_is_kept() {
        check(non_relations());
    }

    #[test]
    fn datetime_operands_truncated() {
        check(datetimes());
        // An instance value is truncated before comparison, so only the
        // truncated operand matches it.
        let mut p = post();
        p.published_at = Some(dt("2020-01-15T10:30:00.123456789Z"));
        let c = Post::PUBLISHED_AT
            .eq(dt("2020-01-15T10:30:00.123456789Z"))
            .into_condition();
        let Folded::Cond(folded) = fold_post(c) else {
            panic!("not a condition");
        };
        assert_eq!(eval(&folded, Post::schema(), p.as_dyn()), Ok(true));
    }

    #[test]
    fn fold_is_idempotent() {
        for (input, _) in all_cases() {
            let once = fold_post(input);
            let twice = match once.clone() {
                Folded::Cond(c) => fold_post(c),
                k => k,
            };
            assert_eq!(twice, once);
        }
    }

    /// A post with a reviewer and tags, differing from `post()` on `b()` and `c()`.
    fn reviewed_post() -> Post {
        let mut p = post();
        p.title = "Rust".into();
        p.locked = true;
        p.reviewer_id = Some(2);
        p.published_at = Some(dt("2020-01-15T10:30:00Z"));
        p.reviewer = Some(User {
            id: 2,
            name: "Rev".into(),
            email: "rev@example.com".into(),
            posts: vec![],
        });
        p.tags = vec![
            Tag {
                id: 1,
                name: Some("rust".into()),
            },
            Tag { id: 2, name: None },
        ];
        p
    }

    #[test]
    fn fold_preserves_meaning_in_memory() {
        let eval_folded = |f: Folded, p: &Post| -> Result<bool, EvalError> {
            match f {
                Folded::True => Ok(true),
                Folded::False => Ok(false),
                Folded::Cond(c) => eval(&c, Post::schema(), p.as_dyn()),
            }
        };
        for p in [post(), reviewed_post()] {
            for (input, _) in all_cases() {
                let before = eval(&input, Post::schema(), p.as_dyn());
                assert!(before.is_ok(), "{input:?}: {before:?}");
                assert_eq!(
                    eval_folded(fold_post(input.clone()), &p),
                    before,
                    "{input:?}"
                );
            }
        }
    }
}
