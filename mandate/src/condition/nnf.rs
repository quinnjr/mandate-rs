//! Restricted negation normal form for filter plans (spec §8).
//!
//! Negations are pushed to the leaves and rewritten away, except directly
//! above a `Str` leaf, and `Every` is rewritten to `None`. Every rewrite is
//! exact under the two-valued null semantics of §5.3, so an adapter never
//! needs a SQL `NOT` over a nullable expression.

use super::eval::{path, within};
use super::{CmpOp, Condition, Quant};
use crate::{CardinalityKind, EvalError, FieldIdx, FieldKind, Schema};

/// Rewrites `c` into restricted negation normal form (§8); `schema` is the
/// schema `c`'s field indices refer to.
///
/// `c` should be folded. The result has no `Every` and no `Not` except
/// directly above a `Str` leaf; folding it again restores canonical form
/// without reintroducing either.
///
/// Fails with [`EvalError::InvalidCondition`] on a quantifier over a field
/// that is not a relation (rejected by validation), at the path evaluating
/// it reports: no rewrite of it is right for every rule.
pub(crate) fn nnf(c: Condition, schema: &'static Schema) -> Result<Condition, EvalError> {
    // Checked in a pass of its own, with small frames, so that the rewrite,
    // which recurses as deep as `c` nests, stays infallible: a `Result`
    // threaded through it about doubles its frames in a debug build, and
    // with them the stack the deepest plans need.
    check_quantifiers(&c, schema)?;
    Ok(positive(c, schema))
}

/// Fails with the error evaluating `c` reports at its first quantifier over
/// a field that is not a relation, if any: `InvalidCondition` at the
/// field's path, prefixed with the relations it is reached through.
fn check_quantifiers(c: &Condition, schema: &'static Schema) -> Result<(), EvalError> {
    match c {
        Condition::And(cs) | Condition::Or(cs) => {
            for c in cs {
                check_quantifiers(c, schema)?;
            }
            Ok(())
        }
        Condition::Not(c) => check_quantifiers(c, schema),
        Condition::Rel { relation, cond, .. } => {
            let Some((name, FieldKind::Relation { target, .. })) =
                schema.field(*relation).map(|def| (def.name(), def.kind()))
            else {
                // There is no target schema to rewrite `cond` against, and
                // no constant is right for every rule (true grants in a
                // `can`, false in a `cannot`), so the plan fails closed.
                return Err(EvalError::InvalidCondition {
                    path: path(schema, *relation),
                });
            };
            match cond {
                Some(c) => check_quantifiers(c, target()).map_err(|e| within(name, e)),
                None => Ok(()),
            }
        }
        _ => Ok(()),
    }
}

/// `c` in restricted NNF.
fn positive(c: Condition, schema: &'static Schema) -> Condition {
    match c {
        Condition::And(cs) => Condition::And(cs.into_iter().map(|c| positive(c, schema)).collect()),
        Condition::Or(cs) => Condition::Or(cs.into_iter().map(|c| positive(c, schema)).collect()),
        Condition::Not(c) => negative(*c, schema),
        Condition::Rel {
            relation,
            quant,
            cond,
        } => rel(relation, quant, cond, false, schema),
        leaf => leaf,
    }
}

/// `¬c` in restricted NNF.
fn negative(c: Condition, schema: &'static Schema) -> Condition {
    match c {
        Condition::Cmp { field, op, value } => {
            let (op, ordering) = match op {
                CmpOp::Eq => (CmpOp::Ne, false),
                CmpOp::Ne => (CmpOp::Eq, false),
                CmpOp::Lt => (CmpOp::Gte, true),
                CmpOp::Lte => (CmpOp::Gt, true),
                CmpOp::Gt => (CmpOp::Lte, true),
                CmpOp::Gte => (CmpOp::Lt, true),
            };
            let cmp = Condition::Cmp { field, op, value };
            // An ordering and its dual are both false on null, where the
            // negation holds. (`Eq`/`Ne` are already complements on null.)
            if ordering && may_be_null(schema, field) {
                Condition::Or(vec![cmp, Condition::IsNull(field)])
            } else {
                cmp
            }
        }
        Condition::In { field, values } => Condition::NotIn { field, values },
        Condition::NotIn { field, values } => Condition::In { field, values },
        c @ Condition::Str { .. } => Condition::Not(Box::new(c)),
        Condition::IsNull(field) => Condition::IsNotNull(field),
        Condition::IsNotNull(field) => Condition::IsNull(field),
        Condition::And(cs) => Condition::Or(cs.into_iter().map(|c| negative(c, schema)).collect()),
        Condition::Or(cs) => Condition::And(cs.into_iter().map(|c| negative(c, schema)).collect()),
        Condition::Not(c) => positive(*c, schema),
        Condition::Rel {
            relation,
            quant,
            cond,
        } => rel(relation, quant, cond, true, schema),
    }
}

/// Whether `field` may be null: anything but a non-nullable scalar. An
/// `IsNull` guard is exact on any field, as it never holds on a value; it is
/// left out only where the schema rules null out.
fn may_be_null(schema: &Schema, field: FieldIdx) -> bool {
    !matches!(
        schema.field(field).map(|def| def.kind()),
        Some(FieldKind::Scalar {
            nullable: false,
            ..
        })
    )
}

/// The quantifier `quant` over `relation`, negated if `negated`, in
/// restricted NNF. Only `One`/`Some` ("a related row satisfies `cond`") and
/// `None` remain: `Every c` is `None ¬c`, and negation swaps the two.
fn rel(
    relation: FieldIdx,
    quant: Quant,
    cond: Option<Box<Condition>>,
    negated: bool,
    schema: &'static Schema,
) -> Condition {
    let Some(FieldKind::Relation {
        target,
        cardinality,
        ..
    }) = schema.field(relation).map(|def| def.kind())
    else {
        // Not a relation: unreachable, as `nnf` rejects every such
        // quantifier before rewriting. Kept as written, negation included,
        // rather than panicking.
        let rel = Condition::Rel {
            relation,
            quant,
            cond,
        };
        return if negated {
            Condition::Not(Box::new(rel))
        } else {
            rel
        };
    };
    let target = target();
    let (exists, cond) = match (quant, cond) {
        (Quant::One | Quant::Some, cond) => (true, cond.map(|c| positive(*c, target))),
        (Quant::None, cond) => (false, cond.map(|c| positive(*c, target))),
        (Quant::Every, Some(c)) => (false, Some(negative(*c, target))),
        // Every row satisfies true (folded away before this point).
        (Quant::Every, None) => return Condition::constant(!negated),
    };
    let quant = match (exists != negated, cardinality) {
        (true, CardinalityKind::ToOne) => Quant::One,
        (true, CardinalityKind::ToMany) => Quant::Some,
        (false, _) => Quant::None,
    };
    Condition::Rel {
        relation,
        quant,
        cond: cond.map(Box::new),
    }
}

#[cfg(all(test, feature = "derive", feature = "chrono", feature = "uuid"))]
mod tests {
    use super::nnf;
    use crate::condition::eval::eval;
    use crate::condition::fold::{Folded, fold};
    use crate::test_fixture::*;
    use crate::{Cond, Condition, EvalError, FieldIdx, Quant, Resource};

    use Condition::{And, Not, Or};

    /// Inputs (folded before rewriting) paired with their expected rewrite.
    type Cases = Vec<(Condition, Condition)>;

    // Post fields (see the fixture).
    const TITLE: FieldIdx = FieldIdx(3);
    /// To-one, non-nullable (→ Org).
    const ORG: FieldIdx = FieldIdx(9);
    /// To-one, nullable (→ User).
    const REVIEWER: FieldIdx = FieldIdx(10);
    /// To-many (→ Tag).
    const TAGS: FieldIdx = FieldIdx(11);
    /// `User.posts`: to-many (→ Post).
    const USER_POSTS: FieldIdx = FieldIdx(3);

    fn c<R>(c: Cond<R>) -> Condition {
        c.into_condition()
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

    /// Folds `input` (the precondition of `nnf`), then rewrites it.
    fn try_nnf_post(input: Condition) -> Result<Condition, EvalError> {
        match fold(input.clone(), Post::schema()) {
            Folded::Cond(c) => nnf(c, Post::schema()),
            k => panic!("{input:?} folded to {k:?}"),
        }
    }

    /// [`try_nnf_post`] for an input that rewrites without error.
    fn nnf_post(input: Condition) -> Condition {
        try_nnf_post(input.clone()).unwrap_or_else(|e| panic!("rewriting {input:?}: {e:?}"))
    }

    fn check(cases: Cases) {
        for (input, expected) in cases {
            assert_eq!(nnf_post(input.clone()), expected, "rewriting {input:?}");
        }
    }

    /// `¬(a∧b)` / `¬(a∨b)` → `¬a∨¬b` / `¬a∧¬b`.
    fn de_morgan() -> Cases {
        let (a, b) = (Post::LOCKED.eq(true), Post::AUTHOR_ID.eq(7));
        vec![
            (
                c(!a.clone().and(b.clone())),
                Or(vec![c(Post::LOCKED.ne(true)), c(Post::AUTHOR_ID.ne(7))]),
            ),
            (
                c(!a.or(b)),
                And(vec![c(Post::LOCKED.ne(true)), c(Post::AUTHOR_ID.ne(7))]),
            ),
        ]
    }

    /// `¬Eq v` / `¬Ne v` → `Ne v` / `Eq v`, nullable or not.
    fn eq_ne() -> Cases {
        vec![
            (c(!Post::AUTHOR_ID.eq(7)), c(Post::AUTHOR_ID.ne(7))),
            (c(!Post::AUTHOR_ID.ne(7)), c(Post::AUTHOR_ID.eq(7))),
            (c(!Post::REVIEWER_ID.eq(7)), c(Post::REVIEWER_ID.ne(7))),
            (c(!Post::REVIEWER_ID.ne(7)), c(Post::REVIEWER_ID.eq(7))),
        ]
    }

    /// `¬In vs` / `¬NotIn vs` → `NotIn vs` / `In vs`, nullable or not.
    fn in_not_in() -> Cases {
        vec![
            (
                c(!Post::AUTHOR_ID.is_in([1, 2])),
                c(Post::AUTHOR_ID.not_in([1, 2])),
            ),
            (
                c(!Post::AUTHOR_ID.not_in([1, 2])),
                c(Post::AUTHOR_ID.is_in([1, 2])),
            ),
            (
                c(!Post::REVIEWER_ID.is_in([1])),
                c(Post::REVIEWER_ID.not_in([1])),
            ),
            (
                c(!Post::REVIEWER_ID.not_in([1])),
                c(Post::REVIEWER_ID.is_in([1])),
            ),
        ]
    }

    /// `¬Lt v`, `¬Lte v`, `¬Gt v`, `¬Gte v` → `Gte v`, `Gt v`, `Lte v`, `Lt v`,
    /// each `∨ IsNull` when the field is nullable.
    fn orderings() -> Cases {
        let null = || c(Post::REVIEWER_ID.is_null());
        vec![
            (c(!Post::AUTHOR_ID.lt(5)), c(Post::AUTHOR_ID.gte(5))),
            (c(!Post::AUTHOR_ID.lte(5)), c(Post::AUTHOR_ID.gt(5))),
            (c(!Post::AUTHOR_ID.gt(5)), c(Post::AUTHOR_ID.lte(5))),
            (c(!Post::AUTHOR_ID.gte(5)), c(Post::AUTHOR_ID.lt(5))),
            (c(!Post::SCORE.lt(0.5)), c(Post::SCORE.gte(0.5))),
            (
                c(!Post::REVIEWER_ID.lt(5)),
                Or(vec![c(Post::REVIEWER_ID.gte(5)), null()]),
            ),
            (
                c(!Post::REVIEWER_ID.lte(5)),
                Or(vec![c(Post::REVIEWER_ID.gt(5)), null()]),
            ),
            (
                c(!Post::REVIEWER_ID.gt(5)),
                Or(vec![c(Post::REVIEWER_ID.lte(5)), null()]),
            ),
            (
                c(!Post::REVIEWER_ID.gte(5)),
                Or(vec![c(Post::REVIEWER_ID.lt(5)), null()]),
            ),
            (
                c(!Post::PUBLISHED_AT.gt(dt("2020-01-01T00:00:00Z"))),
                Or(vec![
                    c(Post::PUBLISHED_AT.lte(dt("2020-01-01T00:00:00Z"))),
                    c(Post::PUBLISHED_AT.is_null()),
                ]),
            ),
        ]
    }

    /// `¬IsNull` / `¬IsNotNull` → `IsNotNull` / `IsNull`, on scalars and
    /// to-ones.
    fn null_tests() -> Cases {
        vec![
            (
                c(!Post::REVIEWER_ID.is_null()),
                c(Post::REVIEWER_ID.is_not_null()),
            ),
            (
                c(!Post::REVIEWER_ID.is_not_null()),
                c(Post::REVIEWER_ID.is_null()),
            ),
            (
                c(!Post::REVIEWER.is_null()),
                c(Post::REVIEWER.is_not_null()),
            ),
            (
                c(!Post::REVIEWER.is_not_null()),
                c(Post::REVIEWER.is_null()),
            ),
        ]
    }

    /// `¬Rel{One,c}` → `Rel{None,c}` (to-one).
    fn negated_one() -> Cases {
        vec![
            (
                c(!Post::ORG.then(Org::NAME.eq("Acme"))),
                rel(ORG, Quant::None, Some(c(Org::NAME.eq("Acme")))),
            ),
            (
                c(!Post::REVIEWER.then(User::ID.eq(2))),
                rel(REVIEWER, Quant::None, Some(c(User::ID.eq(2)))),
            ),
            // A non-nullable to-one is never assumed present (§5.1).
            (
                c(!Post::ORG.then(Cond::all([]))),
                rel(ORG, Quant::None, None),
            ),
        ]
    }

    /// `¬Rel{Some,c}` / `¬Rel{None,c}` → `Rel{None,c}` / `Rel{Some,c}`
    /// (to-many); `¬Rel{None,c}` on a to-one is `Rel{One,c}`.
    fn negated_some_none() -> Cases {
        vec![
            (
                c(!Post::TAGS.some(Tag::ID.eq(1))),
                rel(TAGS, Quant::None, Some(c(Tag::ID.eq(1)))),
            ),
            (
                c(!Post::TAGS.none(Tag::ID.eq(1))),
                rel(TAGS, Quant::Some, Some(c(Tag::ID.eq(1)))),
            ),
            // The related condition is rewritten too.
            (
                c(!Post::TAGS.some(!Tag::NAME.eq("x"))),
                rel(TAGS, Quant::None, Some(c(Tag::NAME.ne("x")))),
            ),
            (
                not(rel(ORG, Quant::None, Some(c(Org::NAME.eq("Acme"))))),
                rel(ORG, Quant::One, Some(c(Org::NAME.eq("Acme")))),
            ),
            (
                not(rel(REVIEWER, Quant::None, Some(c(User::ID.eq(2))))),
                rel(REVIEWER, Quant::One, Some(c(User::ID.eq(2)))),
            ),
        ]
    }

    /// `Rel{Every,c}` → `Rel{None, nnf(¬c)}`, and its negation is
    /// `Rel{Some, nnf(¬c)}`.
    fn every() -> Cases {
        vec![
            (
                c(Post::TAGS.every(Tag::NAME.eq("rust"))),
                rel(TAGS, Quant::None, Some(c(Tag::NAME.ne("rust")))),
            ),
            (
                c(!Post::TAGS.every(Tag::NAME.eq("rust"))),
                rel(TAGS, Quant::Some, Some(c(Tag::NAME.ne("rust")))),
            ),
            (
                c(Post::TAGS.every(Tag::ID.gt(3))),
                rel(TAGS, Quant::None, Some(c(Tag::ID.lte(3)))),
            ),
            (
                c(Post::TAGS.every(Tag::NAME.contains("r"))),
                rel(TAGS, Quant::None, Some(c(!Tag::NAME.contains("r")))),
            ),
            (
                c(Post::TAGS.every(!Tag::NAME.contains("r"))),
                rel(TAGS, Quant::None, Some(c(Tag::NAME.contains("r")))),
            ),
        ]
    }

    /// `Not` stays only directly above `Str`.
    fn text() -> Cases {
        vec![
            (c(!Post::TITLE.contains("x")), c(!Post::TITLE.contains("x"))),
            (
                c(!Post::TITLE.contains("x").or(Post::TITLE.ends_with("y"))),
                And(vec![
                    c(!Post::TITLE.contains("x")),
                    c(!Post::TITLE.ends_with("y")),
                ]),
            ),
            (
                c(!Post::TITLE.contains("x").and(!Post::TITLE.starts_with("y"))),
                Or(vec![
                    c(!Post::TITLE.contains("x")),
                    c(Post::TITLE.starts_with("y")),
                ]),
            ),
        ]
    }

    /// Conditions under a relation are rewritten against its target schema:
    /// `User` index 3 is a to-many (`Post` index 3 is `title`), and `Post`
    /// index 2 is nullable (`User` index 2 is `email`).
    fn target_schemas() -> Cases {
        vec![
            (
                c(Post::REVIEWER.then(User::POSTS.every(Post::REVIEWER_ID.gt(1)))),
                rel(
                    REVIEWER,
                    Quant::One,
                    Some(rel(
                        USER_POSTS,
                        Quant::None,
                        Some(Or(vec![
                            c(Post::REVIEWER_ID.lte(1)),
                            c(Post::REVIEWER_ID.is_null()),
                        ])),
                    )),
                ),
            ),
            (
                c(Post::REVIEWER.then(!User::POSTS.none(Post::LOCKED.eq(true)))),
                rel(
                    REVIEWER,
                    Quant::One,
                    Some(rel(USER_POSTS, Quant::Some, Some(c(Post::LOCKED.eq(true))))),
                ),
            ),
        ]
    }

    /// Conditions with a `Rel` on a field that is not a relation (rejected
    /// by validation), paired with the path of the error: there is no target
    /// schema to rewrite against, so the rewrite fails.
    fn non_relations() -> Vec<(Condition, &'static str)> {
        let org_name = Org::NAME.idx();
        vec![
            (rel(TITLE, Quant::Some, None), "title"),
            (not(rel(TITLE, Quant::Some, None)), "title"),
            (rel(TITLE, Quant::Every, Some(c(Tag::ID.eq(1)))), "title"),
            (
                not(rel(TITLE, Quant::None, Some(c(Tag::ID.eq(1))))),
                "title",
            ),
            (rel(FieldIdx(500), Quant::One, None), "#500"),
            (
                And(vec![c(Post::ID.eq(1)), rel(TITLE, Quant::None, None)]),
                "title",
            ),
            // Reached through a relation, the path names it, on either polarity.
            (
                rel(ORG, Quant::One, Some(rel(org_name, Quant::Every, None))),
                "org.name",
            ),
            (
                not(rel(ORG, Quant::One, Some(rel(org_name, Quant::Some, None)))),
                "org.name",
            ),
            (
                rel(
                    TAGS,
                    Quant::Every,
                    Some(rel(Tag::NAME.idx(), Quant::Some, None)),
                ),
                "tags.name",
            ),
        ]
    }

    /// Every case of a valid condition (`non_relations` is not one).
    fn all_cases() -> Cases {
        [
            de_morgan(),
            eq_ne(),
            in_not_in(),
            orderings(),
            null_tests(),
            negated_one(),
            negated_some_none(),
            every(),
            text(),
            target_schemas(),
        ]
        .concat()
    }

    #[test]
    fn negated_groups_follow_de_morgan() {
        check(de_morgan());
    }

    #[test]
    fn negated_eq_and_ne_swap() {
        check(eq_ne());
    }

    #[test]
    fn negated_in_and_not_in_swap() {
        check(in_not_in());
    }

    #[test]
    fn negated_orderings_guard_nullable_fields() {
        check(orderings());
    }

    #[test]
    fn negated_null_tests_swap() {
        check(null_tests());
    }

    #[test]
    fn negated_one_is_none() {
        check(negated_one());
    }

    #[test]
    fn negated_some_and_none_swap() {
        check(negated_some_none());
    }

    #[test]
    fn every_always_rewritten() {
        check(every());
    }

    #[test]
    fn not_kept_only_above_str() {
        check(text());
    }

    #[test]
    fn rewrites_under_relations_use_target_schema() {
        check(target_schemas());
    }

    #[test]
    fn rel_on_non_relation_is_invalid() {
        // The rewrite fails with the error evaluating the input reports (on a
        // post whose relations have rows, so evaluation reaches the node).
        let p = reviewed_post();
        for (input, path) in non_relations() {
            let invalid = EvalError::InvalidCondition { path: path.into() };
            assert_eq!(
                eval(&input, Post::schema(), p.as_dyn()),
                Err(invalid.clone()),
                "{input:?}"
            );
            assert_eq!(try_nnf_post(input.clone()), Err(invalid), "{input:?}");
        }
    }

    /// Whether `c` has no `Every` and no `Not` except directly above `Str`.
    fn restricted(c: &Condition) -> bool {
        match c {
            Not(inner) => matches!(**inner, Condition::Str { .. }),
            And(cs) | Or(cs) => cs.iter().all(restricted),
            Condition::Rel {
                quant: Quant::Every,
                ..
            } => false,
            Condition::Rel { cond, .. } => cond.as_deref().is_none_or(restricted),
            _ => true,
        }
    }

    #[test]
    fn output_is_restricted_before_and_after_folding() {
        for (input, _) in all_cases() {
            let out = nnf_post(input.clone());
            assert!(restricted(&out), "{input:?} → {out:?}");
            if let Folded::Cond(folded) = fold(out, Post::schema()) {
                assert!(restricted(&folded), "{input:?} → {folded:?}");
            }
        }
    }

    fn dt(s: &str) -> chrono::DateTime<chrono::Utc> {
        s.parse().unwrap()
    }

    /// A post with a reviewer (who has posts of their own) and tags, one of
    /// them with a null name.
    fn reviewed_post() -> Post {
        let mut own = post();
        own.reviewer_id = Some(1);
        own.locked = true;
        let mut p = post();
        p.title = "xy".into();
        p.locked = true;
        p.author_id = 5;
        p.reviewer_id = Some(2);
        p.published_at = Some(dt("2020-01-15T10:30:00Z"));
        p.reviewer = Some(User {
            id: 2,
            name: "Rev".into(),
            email: "rev@example.com".into(),
            posts: vec![own, post()],
        });
        p.tags = vec![
            Tag {
                id: 1,
                name: Some("rust".into()),
            },
            Tag { id: 4, name: None },
        ];
        p
    }

    /// A post whose reviewer has no posts, with a single tag.
    fn tagged_post() -> Post {
        let mut p = post();
        p.title = "Rust".into();
        p.reviewer_id = Some(9);
        p.score = 0.25;
        p.org.name = "Initech".into();
        p.reviewer = Some(User {
            id: 9,
            name: "Nine".into(),
            email: "nine@example.com".into(),
            posts: vec![],
        });
        p.tags = vec![Tag {
            id: 7,
            name: Some("rust".into()),
        }];
        p
    }

    #[test]
    fn nnf_preserves_meaning_in_memory() {
        for p in [post(), reviewed_post(), tagged_post()] {
            for (input, _) in all_cases() {
                let before = eval(&input, Post::schema(), p.as_dyn());
                assert!(before.is_ok(), "{input:?}: {before:?}");
                let after = nnf_post(input.clone());
                assert_eq!(
                    eval(&after, Post::schema(), p.as_dyn()),
                    before,
                    "{input:?} → {after:?}"
                );
            }
        }
    }
}
