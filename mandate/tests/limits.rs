#![cfg(all(feature = "derive", feature = "chrono", feature = "uuid"))]
//! Data-driven rule sets stay within bounded recursion: `and`/`or` chains
//! are flat, and `build()` rejects rule sets whose access formula or
//! conditions would nest too deeply, instead of overflowing the stack later.
//! `stack.rs` runs the largest accepted rule sets on a small stack.

mod common;
use common::fixture::*;
use common::stack::{alternating, grouped, negated, related};
use mandate::{BuildError, Cond, Context, RuleTemplate, Templates};

type Ab = mandate::Ability<Action, Subject>;

#[test]
fn and_or_flatten_into_an_existing_group() {
    let (a, b, c) = (Post::ID.eq(1), Post::ID.eq(2), Post::ID.eq(3));
    let leaves = || {
        vec![
            a.clone().into_condition(),
            b.clone().into_condition(),
            c.clone().into_condition(),
        ]
    };
    assert_eq!(
        a.clone().or(b.clone()).or(c.clone()).into_condition(),
        mandate::Condition::Or(leaves())
    );
    assert_eq!(
        a.clone().and(b.clone().and(c.clone())).into_condition(),
        mandate::Condition::And(leaves())
    );
    // A group of the other kind stays one child.
    assert_eq!(
        a.clone().and(b.clone()).or(c.clone()).into_condition(),
        mandate::Condition::Or(vec![
            mandate::Condition::And(vec![a.into_condition(), b.into_condition()]),
            c.into_condition(),
        ])
    );
}

#[test]
fn too_many_alternations_are_rejected() {
    let e = alternating(258).unwrap_err();
    assert!(
        matches!(
            e,
            BuildError::TooManyAlternations {
                subject: "Post",
                action: "read",
                count: 257,
                ..
            }
        ),
        "{e:?}"
    );
    // A `manage`/`all` rule counts in every cell it covers.
    let mut b = Ab::builder().can(Action::Manage, Subject::All);
    for i in 0..257 {
        b = if i % 2 == 0 {
            b.cannot(Action::Update, Subject::Org)
        } else {
            b.can(Action::Update, Subject::Org)
        }
        .when(Org::ID.eq(i));
    }
    let e = b.build().unwrap_err();
    assert!(
        matches!(
            e,
            BuildError::TooManyAlternations {
                subject: "Org",
                action: "update",
                count: 257,
                ..
            }
        ),
        "{e:?}"
    );
}

/// Asserts that `e` is `TooDeep` on `Post` at depth 65.
#[track_caller]
fn assert_too_deep(e: BuildError) {
    assert!(
        matches!(
            e,
            BuildError::TooDeep {
                subject: "Post",
                depth: 65,
                ..
            }
        ),
        "{e:?}"
    );
}

fn build_with(c: Cond<Post>) -> Result<Ab, BuildError> {
    Ab::builder()
        .can(Action::Read, Subject::Post)
        .when(c)
        .build()
}

#[test]
fn conditions_deeper_than_64_are_rejected() {
    // `stack.rs` runs the 64-level shapes.
    assert!(build_with(negated(64)).is_ok());
    assert!(build_with(grouped(64, 0)).is_ok());
    assert!(build_with(related(64)).is_ok());
    assert_too_deep(build_with(negated(65)).unwrap_err());
    assert_too_deep(build_with(grouped(65, 0)).unwrap_err());
    // Relations count as a level.
    assert_too_deep(build_with(related(65)).unwrap_err());
    // Deep conditions fail even when folding would remove them.
    assert!(matches!(
        build_with(negated(1_000).or(Cond::all([]))),
        Err(BuildError::TooDeep { depth: 1_001, .. })
    ));
}

#[test]
fn deep_bound_rules_are_rejected() {
    // 32 levels of `{"id": i, "$not": …}` (an `And` and a `Not` each), then
    // a two-key object: 65 levels, within the template nesting limit.
    let mut cond = r#"{"id": 1, "author_id": 2}"#.to_owned();
    for i in 0..32 {
        cond = format!(r#"{{"id": {i}, "$not": {cond}}}"#);
    }
    let json = format!(r#"[{{"action": "read", "subject": "Post", "conditions": {cond}}}]"#);
    let raw: Vec<RuleTemplate> = serde_json::from_str(&json).unwrap();
    let bound = Templates::<Action, Subject>::compile(&raw, &[])
        .unwrap()
        .bind(&Context::empty())
        .unwrap();
    assert_too_deep(Ab::builder().extend(bound).build().unwrap_err());
}
