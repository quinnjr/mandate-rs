#![cfg(all(feature = "derive", feature = "chrono", feature = "uuid"))]
//! Hand-written `Action`/`Subject` implementations that break the dense
//! index contract are rejected by `build()`, and never make a check panic.

use mandate::{Ability, Access, BuildError, CheckError, Forbidden, Resource, SubjectResource};

#[derive(Clone, Debug, Resource)]
struct Doc {
    id: i64,
}

/// An action enum with a hand-written `Action` impl: `COUNT`, the values
/// `all()` lists, and the `index()` of each variant are given.
macro_rules! action {
    ($name:ident, count = $count:expr, all = [$($all:ident),*], index = [$read:expr, $write:expr, $hidden:expr]) => {
        // Not every invocation uses every variant.
        #[allow(dead_code)]
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        enum $name {
            Read,
            Write,
            Hidden,
        }

        impl mandate::Action for $name {
            const COUNT: usize = $count;
            const MANAGE: Option<Self> = None;
            fn index(self) -> usize {
                match self {
                    Self::Read => $read,
                    Self::Write => $write,
                    Self::Hidden => $hidden,
                }
            }
            fn name(self) -> &'static str {
                match self {
                    Self::Read => "read",
                    Self::Write => "write",
                    Self::Hidden => "hidden",
                }
            }
            fn from_name(name: &str) -> Option<Self> {
                <Self as mandate::Action>::all()
                    .iter()
                    .copied()
                    .find(|a| mandate::Action::name(*a) == name)
            }
            fn all() -> &'static [Self] {
                &[$(Self::$all),*]
            }
        }
    };
}

/// A subject enum with a hand-written `Subject` impl binding `Doc`.
macro_rules! subject {
    ($name:ident, count = $count:expr, index = $index:expr) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        enum $name {
            Doc,
        }

        impl mandate::Subject for $name {
            const COUNT: usize = $count;
            const ALL: Option<Self> = None;
            fn index(self) -> usize {
                $index
            }
            fn name(self) -> &'static str {
                "Doc"
            }
            fn from_name(name: &str) -> Option<Self> {
                (name == "Doc").then_some(Self::Doc)
            }
            fn all() -> &'static [Self] {
                &[Self::Doc]
            }
            fn schema(self) -> Option<&'static mandate::Schema> {
                Some(Doc::schema())
            }
        }

        impl SubjectResource<$name> for Doc {
            const SUBJECT: $name = $name::Doc;
        }
    };
}

// `Hidden` is not listed by `all()`, and its index is out of range.
action!(Act, count = 2, all = [Read, Write], index = [0, 1, 9]);
// `all()` lists fewer values than `COUNT`.
action!(ShortAll, count = 3, all = [Read, Write], index = [0, 1, 2]);
// An index outside `0..COUNT`.
action!(Sparse, count = 2, all = [Read, Write], index = [0, 2, 1]);
// Two values with the same index.
action!(Shared, count = 2, all = [Read, Write], index = [0, 0, 1]);

subject!(Sub, count = 1, index = 0);
subject!(BadSub, count = 1, index = 1);

/// Asserts that `e` is `InvalidEnum` for `which`.
#[track_caller]
fn assert_invalid(e: BuildError, which: &str) {
    assert!(
        matches!(&e, BuildError::InvalidEnum { which: w, .. } if *w == which),
        "{e:?}"
    );
}

#[test]
fn inconsistent_actions_are_rejected() {
    assert!(
        Ability::<Act, Sub>::builder()
            .can(Act::Read, Sub::Doc)
            .build()
            .is_ok()
    );
    assert_invalid(
        Ability::<ShortAll, Sub>::builder()
            .can(ShortAll::Read, Sub::Doc)
            .build()
            .unwrap_err(),
        "action",
    );
    assert_invalid(
        Ability::<Sparse, Sub>::builder()
            .can(Sparse::Read, Sub::Doc)
            .build()
            .unwrap_err(),
        "action",
    );
    assert_invalid(
        Ability::<Shared, Sub>::builder()
            .can(Shared::Read, Sub::Doc)
            .build()
            .unwrap_err(),
        "action",
    );
    // Even without rules.
    assert_invalid(
        Ability::<Shared, Sub>::builder().build().unwrap_err(),
        "action",
    );
}

#[test]
fn rules_on_unlisted_values_are_rejected() {
    assert_invalid(
        Ability::<Act, Sub>::builder()
            .can(Act::Read, Sub::Doc)
            .cannot(Act::Hidden, Sub::Doc)
            .build()
            .unwrap_err(),
        "action",
    );
}

#[test]
fn inconsistent_subjects_are_rejected() {
    assert_invalid(
        Ability::<Act, BadSub>::builder()
            .can(Act::Read, BadSub::Doc)
            .build()
            .unwrap_err(),
        "subject",
    );
}

#[test]
fn unlisted_values_are_denied_without_panicking() {
    let a = Ability::<Act, Sub>::builder()
        .can(Act::Read, Sub::Doc)
        .can(Act::Write, Sub::Doc)
        .build()
        .unwrap();
    let doc = Doc { id: 1 };
    assert!(a.can(Act::Read, &doc));
    assert!(!a.can(Act::Hidden, &doc));
    assert!(!a.can_field(Act::Hidden, &doc, Doc::ID));
    assert!(!a.can_type(Act::Hidden, Sub::Doc));
    // Denied like any action without rules: no field, no reason.
    let denied = |f: &Forbidden<Act, Sub>| {
        f.action == Act::Hidden && f.subject == Sub::Doc && f.field.is_none() && f.reason.is_none()
    };
    let e = a.check(Act::Hidden, &doc).unwrap_err();
    assert!(matches!(&e, CheckError::Forbidden(f) if denied(f)), "{e:?}");
    let e = a.check_type(Act::Hidden, Sub::Doc).unwrap_err();
    assert!(denied(&e), "{e:?}");
    assert_eq!(a.access::<Doc>(Act::Hidden), Ok(Access::Denied));
    assert!(
        a.permitted_fields(Act::Hidden, &doc)
            .unwrap()
            .mask()
            .is_empty()
    );
    assert!(a.field_plan::<Doc>(Act::Hidden).unwrap().rules.is_empty());
    assert!(
        a.projection::<Doc>(Act::Hidden)
            .unwrap()
            .fields
            .mask()
            .is_empty()
    );
}
