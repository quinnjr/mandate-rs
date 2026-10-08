#![cfg(all(feature = "derive", feature = "chrono", feature = "uuid"))]
mod common;
use common::fixture::*;
use mandate::{
    Ability, Access, Condition, DynResource, EvalError, FieldDef, FieldIdx, Kind, Plan, Quant,
    RelationRef, Resource, Schema, SubjectResource, ValueRef,
};

type Ab = Ability<Action, Subject>;

/// Whether every `Not` in `c` sits directly above a `Str` leaf.
fn only_str_under_not(c: &Condition) -> bool {
    match c {
        Condition::Not(inner) => matches!(**inner, Condition::Str { .. }),
        Condition::And(cs) | Condition::Or(cs) => cs.iter().all(only_str_under_not),
        Condition::Rel { cond, .. } => cond.as_deref().is_none_or(only_str_under_not),
        _ => true,
    }
}

/// Whether `c` has no `Every` quantifier.
fn no_every(c: &Condition) -> bool {
    match c {
        Condition::Rel {
            quant: Quant::Every,
            ..
        } => false,
        Condition::Rel { cond, .. } => cond.as_deref().is_none_or(no_every),
        Condition::And(cs) | Condition::Or(cs) => cs.iter().all(no_every),
        Condition::Not(c) => no_every(c),
        _ => true,
    }
}

/// Whether `c`'s groups are flattened, deduplicated, and free of constants.
fn canonical(c: &Condition) -> bool {
    let group = |cs: &[Condition], same: fn(&Condition) -> bool| {
        cs.len() >= 2
            && !cs.iter().any(same)
            && cs.iter().enumerate().all(|(i, c)| !cs[..i].contains(c))
            && cs.iter().all(canonical)
    };
    match c {
        Condition::And(cs) => group(cs, |c| matches!(c, Condition::And(_))),
        Condition::Or(cs) => group(cs, |c| matches!(c, Condition::Or(_))),
        Condition::Not(c) => canonical(c),
        Condition::Rel { cond, .. } => cond.as_deref().is_none_or(canonical),
        _ => true,
    }
}

fn access(a: &Ab) -> Access<Post> {
    a.access::<Post>(Action::Read).unwrap()
}

fn plan(a: &Ab) -> Plan<Post> {
    match access(a) {
        Access::Filter(p) => p,
        other => panic!("expected a filter, got {other:?}"),
    }
}

/// Asserts that the plan agrees with `can` on `posts`, which must include
/// both an allowed and a denied post.
fn assert_agrees(a: &Ab, posts: &[Post]) {
    let allowed = |p: &Post| a.can(Action::Read, p);
    assert!(posts.iter().any(allowed) && !posts.iter().all(allowed));
    let plan = plan(a);
    for p in posts {
        assert_eq!(
            plan.eval(p),
            Ok(a.can(Action::Read, p)),
            "{:?} on {p:?}",
            plan.condition()
        );
    }
}

#[test]
fn denied_and_all() {
    let a = Ab::builder().build().unwrap();
    assert!(matches!(access(&a), Access::Denied));
    let a = Ab::builder()
        .can(Action::Read, Subject::Post)
        .build()
        .unwrap();
    assert!(matches!(access(&a), Access::All));
    assert!(matches!(
        a.access::<Post>(Action::Update),
        Ok(Access::Denied)
    ));
    let a = Ab::builder()
        .can(Action::Read, Subject::Post)
        .cannot(Action::Read, Subject::Post)
        .build()
        .unwrap();
    assert!(matches!(access(&a), Access::Denied));
    // An unconditional rule resets everything before it.
    let a = Ab::builder()
        .cannot(Action::Read, Subject::Post)
        .when(Post::LOCKED.eq(true))
        .can(Action::Manage, Subject::All)
        .build()
        .unwrap();
    assert!(matches!(access(&a), Access::All));
}

#[test]
fn field_restricted_rules_apply_as_without_a_field() {
    // §7.2: `can` rules apply regardless of `fields`, `cannot` rules only
    // without `fields`.
    let a = Ab::builder()
        .can(Action::Read, Subject::Post)
        .fields([Post::TITLE.into()])
        .build()
        .unwrap();
    assert!(matches!(access(&a), Access::All));
    let a = Ab::builder()
        .can(Action::Read, Subject::Post)
        .cannot(Action::Read, Subject::Post)
        .fields([Post::BODY.into()])
        .build()
        .unwrap();
    assert!(matches!(access(&a), Access::All));
}

/// `can A; cannot B; can C` with A = locked, B = draft, C = author 7.
fn formula_ability() -> Ab {
    Ab::builder()
        .can(Action::Read, Subject::Post)
        .when(Post::LOCKED.eq(true))
        .cannot(Action::Read, Subject::Post)
        .when(Post::STATUS.eq(Status::Draft))
        .can(Action::Read, Subject::Post)
        .when(Post::AUTHOR_ID.eq(7))
        .build()
        .unwrap()
}

#[test]
fn exact_formula_matches_can() {
    let a = formula_ability();
    let mut posts = Vec::new();
    for locked in [false, true] {
        for status in [Status::Published, Status::Draft] {
            for author_id in [7, 8] {
                posts.push(Post {
                    locked,
                    status,
                    author_id,
                    ..post()
                });
            }
        }
    }
    assert_agrees(&a, &posts);
    // `C ∨ (¬B ∧ (A ∨ false))`, folded and in NNF.
    assert_eq!(
        plan(&a).condition(),
        &Condition::Or(vec![
            Post::AUTHOR_ID.eq(7).into_condition(),
            Condition::And(vec![
                Post::STATUS.ne(Status::Draft).into_condition(),
                Post::LOCKED.eq(true).into_condition(),
            ]),
        ])
    );
    // A row matching B and C is allowed (CASL's `rulesToQuery` denies it).
    let bc = Post {
        status: Status::Draft,
        author_id: 7,
        ..post()
    };
    assert!(a.can(Action::Read, &bc));
    assert_eq!(plan(&a).eval(&bc), Ok(true));
}

fn every_ability() -> Ab {
    Ab::builder()
        .can(Action::Read, Subject::Post)
        .when(Post::TAGS.every(Tag::NAME.eq("rust")))
        .build()
        .unwrap()
}

fn tag(id: i64, name: Option<&str>) -> Tag {
    Tag {
        id,
        name: name.map(Into::into),
    }
}

#[test]
fn every_over_nullable_names() {
    let a = every_ability();
    let plan = plan(&a);
    assert!(no_every(plan.condition()));
    let p = Post {
        tags: vec![tag(1, None)],
        ..post()
    };
    assert!(!a.can(Action::Read, &p));
    assert_eq!(plan.eval(&p), Ok(false));
    let posts = [
        vec![],
        vec![tag(1, Some("rust"))],
        vec![tag(1, Some("rust")), tag(2, None)],
        vec![tag(1, Some("rust")), tag(2, Some("go"))],
    ]
    .map(|tags| Post { tags, ..post() });
    assert_agrees(&a, &posts);
}

fn strings_ability() -> Ab {
    Ab::builder()
        .can(Action::Read, Subject::Post)
        .when(!Post::TITLE.contains("é"))
        .build()
        .unwrap()
}

#[test]
fn plan_agrees_on_byte_exact_strings() {
    let a = strings_ability();
    let posts = ["é", "e\u{301}", "É"].map(|t| Post {
        title: t.into(),
        ..post()
    });
    assert_agrees(&a, &posts);
    let plan = plan(&a);
    assert_eq!(plan.eval(&posts[0]), Ok(false));
    assert_eq!(plan.eval(&posts[1]), Ok(true));
    assert_eq!(plan.eval(&posts[2]), Ok(true));
}

/// Negations over nullable fields, both relation kinds, and nested `Every`.
fn mixed_ability() -> Ab {
    Ab::builder()
        .can(Action::Read, Subject::Post)
        .when(
            Post::REVIEWER_ID
                .gt(3)
                .or(Post::TAGS.some(Tag::NAME.starts_with("r"))),
        )
        .cannot(Action::Read, Subject::Post)
        .when(
            Post::TAGS
                .every(Tag::NAME.contains("u"))
                .and(Post::REVIEWER.then(User::POSTS.every(Post::SCORE.lt(2.0)))),
        )
        .cannot(Action::Read, Subject::Post)
        .when(!Post::ORG.then(Org::NAME.ends_with("e")))
        // Negated, this needs flattening, and deduplicating the `IsNull`
        // guard each nullable ordering adds.
        .cannot(Action::Read, Subject::Post)
        .when(Post::REVIEWER_ID.lt(2).and(Post::REVIEWER_ID.gte(0)))
        .can(Action::Read, Subject::Post)
        .when(!(Post::REVIEWER_ID.lte(9).or(Post::LOCKED.eq(true))))
        .build()
        .unwrap()
}

fn mixed_posts() -> Vec<Post> {
    let reviewer = |posts: Vec<Post>| User {
        id: 2,
        name: "Rev".into(),
        email: "rev@example.com".into(),
        posts,
    };
    let acme = post();
    let high = Post {
        score: 3.0,
        ..post()
    };
    let initech = Post {
        org: Org {
            id: 4,
            name: "Initech".into(),
        },
        ..post()
    };
    vec![
        acme.clone(),
        Post {
            reviewer_id: Some(5),
            ..acme.clone()
        },
        Post {
            reviewer_id: Some(5),
            reviewer: Some(reviewer(vec![])),
            ..acme.clone()
        },
        Post {
            reviewer_id: Some(5),
            reviewer: Some(reviewer(vec![high.clone()])),
            tags: vec![tag(1, Some("rust"))],
            ..acme.clone()
        },
        Post {
            reviewer_id: Some(1),
            reviewer: Some(reviewer(vec![acme.clone()])),
            tags: vec![tag(1, Some("rust")), tag(2, None)],
            ..acme.clone()
        },
        Post {
            reviewer_id: Some(10),
            ..initech.clone()
        },
        Post {
            reviewer_id: Some(10),
            locked: true,
            ..acme.clone()
        },
        Post {
            tags: vec![tag(1, Some("ruby"))],
            ..initech
        },
    ]
}

#[test]
fn mixed_rules_agree_with_can() {
    assert_agrees(&mixed_ability(), &mixed_posts());
}

#[test]
fn many_rules_on_a_small_stack() {
    // One `can` rule per shared record: the formula must not nest per rule.
    // 2 MiB is a Tokio worker's default stack.
    std::thread::Builder::new()
        .stack_size(2 * 1024 * 1024)
        .spawn(|| {
            let mut g = Ab::builder()
                .can(Action::Read, Subject::Post)
                .when(Post::ID.eq(0));
            for i in 1..10_000 {
                g = g.can(Action::Read, Subject::Post).when(Post::ID.eq(i));
            }
            let a = g.build().unwrap();
            let posts = [0, 9_999, 10_000].map(|id| Post { id, ..post() });
            assert_agrees(&a, &posts);
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn plan_shape_invariants() {
    for a in [
        formula_ability(),
        every_ability(),
        strings_ability(),
        mixed_ability(),
    ] {
        let plan = plan(&a);
        let c = plan.condition();
        assert!(only_str_under_not(c), "{c:?}");
        assert!(no_every(c), "{c:?}");
        assert!(canonical(c), "{c:?}");
    }
}

#[test]
fn plan_exposes_schema_and_serializes_its_condition() {
    let plan = plan(&formula_ability());
    assert!(core::ptr::eq(plan.schema(), Post::schema()));
    assert_eq!(
        serde_json::to_value(&plan).unwrap(),
        serde_json::to_value(plan.condition()).unwrap()
    );
}

struct Imposter;
static IMPOSTER: Schema = Schema::new("Imposter", &[FieldDef::scalar("id", Kind::Int, false)]);
impl Resource for Imposter {
    fn schema() -> &'static Schema {
        &IMPOSTER
    }
    fn as_dyn(&self) -> &dyn DynResource {
        self
    }
}
impl DynResource for Imposter {
    fn resource_schema(&self) -> &'static Schema {
        &IMPOSTER
    }
    fn value(&self, _: FieldIdx) -> ValueRef<'_> {
        ValueRef::NotLoaded
    }
    fn relation(&self, _: FieldIdx) -> RelationRef<'_> {
        RelationRef::NotLoaded
    }
}
impl SubjectResource<Subject> for Imposter {
    const SUBJECT: Subject = Subject::Post;
}

#[test]
fn access_guard() {
    let a = Ab::builder()
        .can(Action::Manage, Subject::All)
        .build()
        .unwrap();
    assert_eq!(
        a.access::<Imposter>(Action::Read).unwrap_err(),
        EvalError::SchemaMismatch {
            expected: "Post",
            found: "Imposter",
        }
    );
}
