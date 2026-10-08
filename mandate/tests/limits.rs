#![cfg(all(feature = "derive", feature = "chrono", feature = "uuid"))]
//! Data-driven rule sets stay within bounded recursion: long `and`/`or`
//! chains are flat, and `build()` rejects rule sets whose access formula or
//! conditions would nest too deeply, instead of overflowing the stack later.

mod common;
use common::fixture::*;
use mandate::{Access, BuildError, Cond, Context, RuleTemplate, Templates};

type Ab = mandate::Ability<Action, Subject>;

/// Runs `f` on a thread with a 2 MiB stack (the default for spawned threads).
fn on_small_stack<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::Builder::new()
        .stack_size(2 * 1024 * 1024)
        .spawn(f)
        .unwrap()
        .join()
        .unwrap()
}

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

/// `n` rules on (read, Post) alternating `can`/`cannot`, starting with
/// `can`: rule `i` matches the post with id `i`, so `n - 1` alternations.
fn alternating(n: i64) -> Result<Ab, BuildError> {
    let mut b = Ab::builder()
        .can(Action::Read, Subject::Post)
        .when(Post::ID.eq(0));
    for i in 1..n {
        b = if i % 2 == 0 {
            b.can(Action::Read, Subject::Post)
        } else {
            b.cannot(Action::Read, Subject::Post)
        }
        .when(Post::ID.eq(i));
    }
    b.build()
}

#[test]
fn too_many_alternations_are_rejected() {
    assert_eq!(
        alternating(258).unwrap_err(),
        BuildError::TooManyAlternations {
            subject: "Post",
            action: "read",
            count: 257,
        }
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
    assert_eq!(
        b.build().unwrap_err(),
        BuildError::TooManyAlternations {
            subject: "Org",
            action: "update",
            count: 257,
        }
    );
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

/// `n` negations of a leaf: nesting depth `n`.
fn negated(n: usize) -> Cond<Post> {
    (0..n).fold(Post::LOCKED.eq(true), |c, _| !c)
}

/// `n` alternating `And`/`Or` groups, each with a leaf: nesting depth `n`.
fn grouped(n: i64) -> Cond<Post> {
    (0..n).fold(Post::ID.eq(-1), |c, i| {
        if i % 2 == 0 {
            Cond::all([c, Post::ID.ne(i)])
        } else {
            Cond::any([c, Post::ID.eq(i)])
        }
    })
}

fn build_with(c: Cond<Post>) -> Result<Ab, BuildError> {
    Ab::builder()
        .can(Action::Read, Subject::Post)
        .when(c)
        .build()
}

#[test]
fn conditions_deeper_than_64_are_rejected() {
    assert!(build_with(negated(64)).is_ok());
    assert!(build_with(grouped(64)).is_ok());
    assert_eq!(
        build_with(negated(65)).unwrap_err(),
        BuildError::TooDeep {
            subject: "Post",
            depth: 65
        }
    );
    assert_eq!(
        build_with(grouped(65)).unwrap_err(),
        BuildError::TooDeep {
            subject: "Post",
            depth: 65
        }
    );
    // Relations count as a level.
    let rel = Post::TAGS.some(Tag::ID.eq(1));
    assert_eq!(
        build_with((0..64).fold(rel, |c, _| !c)).unwrap_err(),
        BuildError::TooDeep {
            subject: "Post",
            depth: 65
        }
    );
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
    assert_eq!(
        Ab::builder().extend(bound).build().unwrap_err(),
        BuildError::TooDeep {
            subject: "Post",
            depth: 65
        }
    );
}
