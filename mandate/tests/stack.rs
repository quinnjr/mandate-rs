#![cfg(all(feature = "derive", feature = "chrono", feature = "uuid"))]
//! The largest rule sets `build()` accepts check, plan and evaluate on a
//! thread with a small stack (`common::stack::SMALL_STACK`, which documents
//! the measured margin): long `and`/`or` chains and one rule per record are
//! flat, and the access formula of the most `can`/`cannot` alternations and
//! the deepest conditions, alone and together, stay within it.
//!
//! These tests have their own binary: a stack overflow aborts the whole
//! process, not just the test that overflowed.

mod common;
use common::fixture::*;
use common::stack::{
    alternating, alternating_when, depth, grouped, negated, on_small_stack, related,
};
use mandate::Access;

type Ab = mandate::Ability<Action, Subject>;

fn with_id(id: i64) -> Post {
    Post { id, ..post() }
}

#[test]
fn long_or_chain_builds_and_evaluates() {
    on_small_stack(|| {
        let ids = 1..10_000i64;
        let c = ids.fold(Post::ID.eq(0), |c, id| c.or(Post::ID.eq(id)));
        let a = Ab::builder()
            .can(Action::Read, Subject::Post)
            .when(c)
            .build()
            .unwrap();
        assert!(a.can(Action::Read, &with_id(9_999)));
        assert!(!a.can(Action::Read, &with_id(10_000)));
        let Access::Filter(plan) = a.access::<Post>(Action::Read).unwrap() else {
            panic!("expected a filter");
        };
        assert_eq!(plan.eval(&with_id(5_000)), Ok(true));
        assert_eq!(plan.eval(&with_id(-1)), Ok(false));
    });
}

#[test]
fn long_and_chain_builds_and_evaluates() {
    on_small_stack(|| {
        let ids = 1..10_000i64;
        let c = ids.fold(Post::ID.ne(0), |c, id| c.and(Post::ID.ne(id)));
        let a = Ab::builder()
            .can(Action::Read, Subject::Post)
            .when(c)
            .build()
            .unwrap();
        assert!(!a.can(Action::Read, &with_id(9_999)));
        assert!(a.can(Action::Read, &with_id(10_000)));
    });
}

#[test]
fn the_maximum_alternations_plan_on_a_small_stack() {
    on_small_stack(|| {
        let a = alternating(257).unwrap();
        let Access::Filter(plan) = a.access::<Post>(Action::Read).unwrap() else {
            panic!("expected a filter");
        };
        for id in [0, 1, 2, 127, 254, 255, 256, 257, 1_000] {
            let p = with_id(id);
            let expected = (0..257).contains(&id) && id % 2 == 0;
            assert_eq!(a.can(Action::Read, &p), expected, "id {id}");
            assert_eq!(plan.eval(&p), Ok(expected), "id {id}");
        }
    });
}

/// A post whose reviewer has one post, whose reviewer has one post, and so
/// on, `hops` times, ending in `last`.
fn reviewed(hops: usize, last: Post) -> Post {
    (0..hops).fold(last, |p, _| Post {
        reviewer: Some(User {
            id: 2,
            name: "Rev".into(),
            email: "rev@example.com".into(),
            posts: vec![p],
        }),
        ..post()
    })
}

/// The 64-level shapes of `common::stack`, the deepest conditions `build()`
/// accepts (`limits.rs` rejects 65), on posts they hold on and posts they
/// fail on: as a `can` condition, and as a `cannot` condition after an
/// unconditional `can`, which the plan rewrites through every level in
/// negative polarity.
#[test]
fn the_deepest_conditions_on_a_small_stack() {
    on_small_stack(|| {
        for (name, c, holds, fails) in [
            (
                "negated",
                negated(64),
                vec![with_id(-1), with_id(31)],
                vec![with_id(30), with_id(1_000)],
            ),
            (
                "grouped",
                grouped(64, 0),
                vec![with_id(-1), with_id(63)],
                vec![with_id(62), with_id(1_000)],
            ),
            (
                "related",
                related(64),
                vec![reviewed(16, post())],
                vec![post(), reviewed(15, post()), reviewed(16, with_id(2))],
            ),
        ] {
            let can = Ab::builder()
                .can(Action::Read, Subject::Post)
                .when(c.clone())
                .build()
                .unwrap();
            let cannot = Ab::builder()
                .can(Action::Read, Subject::Post)
                .cannot(Action::Read, Subject::Post)
                .when(c)
                .build()
                .unwrap();
            for (a, inverted) in [(can, false), (cannot, true)] {
                // Folding at `build()` keeps every level.
                let rule = a.rules().last().unwrap();
                assert_eq!(rule.condition().map(depth), Some(64), "{name}");
                let Access::Filter(plan) = a.access::<Post>(Action::Read).unwrap() else {
                    panic!("expected a filter");
                };
                for (p, expected) in holds
                    .iter()
                    .map(|p| (p, !inverted))
                    .chain(fails.iter().map(|p| (p, inverted)))
                {
                    assert_eq!(a.can(Action::Read, p), expected, "{name}: {p:?}");
                    assert_eq!(plan.eval(p), Ok(expected), "{name}: {p:?}");
                }
            }
        }
    });
}

/// The most alternations `build()` accepts (257 rules, so 256 switches, the
/// `MAX_ALTERNATIONS`; `limits.rs` rejects 258), each rule with a different
/// 64-level condition: the access formula nests by alternation and by
/// condition at once.
#[test]
fn max_alternations_of_deepest_conditions_on_a_small_stack() {
    on_small_stack(|| {
        // Rule `r` holds on ids `100 r - 1` and `100 r + i` for odd `i < 64`,
        // and is a `can` rule for even `r`.
        let a = alternating_when(257, |r| grouped(64, 100 * r)).unwrap();
        assert!(
            a.rules()
                .iter()
                .all(|r| r.condition().map(depth) == Some(64))
        );
        let Access::Filter(plan) = a.access::<Post>(Action::Read).unwrap() else {
            panic!("expected a filter");
        };
        for (id, expected) in [
            (-1, true),
            (63, true),
            (62, false),
            (99, false),
            (163, false),
            (199, true),
            (25_499, false),
            (25_563, false),
            (25_599, true),
            (25_663, true),
            (25_664, false),
            (1_000_000, false),
        ] {
            let p = with_id(id);
            assert_eq!(a.can(Action::Read, &p), expected, "id {id}");
            assert_eq!(plan.eval(&p), Ok(expected), "id {id}");
        }
    });
}

#[test]
fn many_rules_on_a_small_stack() {
    // One `can` rule per shared record: the formula must not nest per rule.
    on_small_stack(|| {
        let mut g = Ab::builder()
            .can(Action::Read, Subject::Post)
            .when(Post::ID.eq(0));
        for i in 1..10_000 {
            g = g.can(Action::Read, Subject::Post).when(Post::ID.eq(i));
        }
        let a = g.build().unwrap();
        let Access::Filter(plan) = a.access::<Post>(Action::Read).unwrap() else {
            panic!("expected a filter");
        };
        for (id, expected) in [(0, true), (9_999, true), (10_000, false)] {
            let p = with_id(id);
            assert_eq!(a.can(Action::Read, &p), expected, "id {id}");
            assert_eq!(plan.eval(&p), Ok(expected), "id {id}");
        }
    });
}
