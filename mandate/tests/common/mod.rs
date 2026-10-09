#![allow(dead_code, unused_imports, unused_macros)]
pub mod fixture;
pub mod imposter;
pub mod shape;
pub mod spec;
pub mod stack;
pub mod strategies;
pub mod templates;
pub mod unresolved;

/// Asserts that an expression matches a pattern (`A | B` and an `if` guard
/// are allowed), evaluating the expression once. On a mismatch it panics
/// with the value's `Debug`.
macro_rules! assert_matches {
    ($e:expr, $p:pat $(if $guard:expr)? $(,)?) => {
        match $e {
            $p $(if $guard)? => {}
            ref v => panic!(
                "assertion failed: `{v:?}` does not match `{}`",
                stringify!($p $(if $guard)?)
            ),
        }
    };
}
pub(crate) use assert_matches;
