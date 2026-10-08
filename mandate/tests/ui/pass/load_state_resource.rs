#[allow(dead_code, unused_imports)]
#[path = "../../common/fixture.rs"]
mod fixture;
use fixture::*;
use mandate::{Ability, Access, CheckError, EvalError};

// `TPost` reports its unloaded scalars through `#[resource(load_state)]` and
// keeps its relations in user-defined `Lazy` slots. Conditions through those
// slots type-check against the slots' targets, and unloaded data reads as
// `NotLoaded` at its path: `can` denies, and `check` and the plan fail.
fn main() {
    let a = Ability::<Action, TSubject>::builder()
        .can(Action::Read, TSubject::TPost)
        .when(
            TPost::ORG
                .then(TOrg::NAME.eq("Acme"))
                .and(TPost::TAGS.none(TTag::AUTHOR.then(TUser::EMAIL.ends_with("@spam")))),
        )
        .build()
        .unwrap();
    let Ok(Access::Filter(plan)) = a.access::<TPost>(Action::Read) else {
        panic!("expected a filter");
    };
    assert!(a.can(Action::Read, &loaded_tpost()));
    assert_eq!(plan.eval(&loaded_tpost()), Ok(true));

    let no_org = TPost {
        org: Lazy::NotLoaded,
        ..loaded_tpost()
    };
    let no_org_name = TPost {
        org: Lazy::Loaded(TOrg {
            loaded: Loaded(vec!["name"]),
            id: 3,
            name: "Acme".into(),
        }),
        ..loaded_tpost()
    };
    let no_tag_author = TPost {
        tags: Lazy::Loaded(vec![TTag {
            loaded: Loaded::default(),
            id: 1,
            name: None,
            author: Lazy::NotLoaded,
        }]),
        ..loaded_tpost()
    };
    for (p, path) in [
        (no_org, "org"),
        (no_org_name, "org.name"),
        (no_tag_author, "tags.author"),
    ] {
        let not_loaded = |e: &EvalError| matches!(e, EvalError::NotLoaded { path: x, .. } if x == path);
        assert!(!a.can(Action::Read, &p), "{path}");
        let checked = a.check(Action::Read, &p);
        assert!(
            matches!(&checked, Err(CheckError::Unresolvable(e)) if not_loaded(e)),
            "{path}: {checked:?}"
        );
        let planned = plan.eval(&p);
        assert!(matches!(&planned, Err(e) if not_loaded(e)), "{path}: {planned:?}");
    }
}
