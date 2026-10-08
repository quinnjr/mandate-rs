// Shared helpers for the template tests: compiling `can read Post when <c>`,
// asserting on the `LoadError` it gives, and the `InvalidValue` reasons for
// strings that are not of a field's kind.

use mandate::{Kind, LoadError, LoadErrorKind, RuleTemplate, Templates};

use super::fixture::{Action, Subject};

/// Compiles `can read Post when <c>` with the `user` root.
pub fn cond(c: &str) -> Result<Templates<Action, Subject>, LoadError> {
    let rule: RuleTemplate = serde_json::from_str(&format!(
        r#"{{"action":"read","subject":"Post","conditions":{c}}}"#
    ))
    .expect("valid JSON");
    Templates::compile(&[rule], &["user"])
}

/// The error compiling `can read Post when <c>`.
#[track_caller]
pub fn cond_err(c: &str) -> LoadError {
    match cond(c) {
        Ok(_) => panic!("{c} compiled"),
        Err(e) => e,
    }
}

/// Asserts that `can read Post when <c>` fails to compile, in rule 0 at
/// `path`, with a kind `kind` accepts.
#[track_caller]
pub fn assert_load_err(c: &str, path: &str, kind: impl Fn(&LoadErrorKind) -> bool) {
    let e = cond_err(c);
    assert!(
        e.rule_index == 0 && e.path == path && kind(&e.kind),
        "{c}: {e:?}"
    );
}

/// Asserts that `can read Post when <c>` fails to compile at `path` with
/// `InvalidValue(r)` for an `r` that `reason` accepts.
#[track_caller]
pub fn assert_invalid(c: &str, path: &str, reason: impl Fn(&str) -> bool) {
    assert_load_err(
        c,
        path,
        |k| matches!(k, LoadErrorKind::InvalidValue(r) if reason(r)),
    );
}

/// Asserts that `can read Post when <c>` fails to compile at `path` with
/// `TypeMismatch { expected, found }`.
#[track_caller]
pub fn assert_mismatch(c: &str, path: &str, expected: Kind, found: &str) {
    assert_load_err(c, path, |k| {
        matches!(
            k,
            LoadErrorKind::TypeMismatch { expected: e, found: f, .. } if *e == expected && f == found
        )
    });
}

/// Asserts that `can read Post when <c>` fails to compile at `path` with
/// `OperatorNotAllowed { op, kind }`.
#[track_caller]
pub fn assert_not_allowed(c: &str, path: &str, op: &str, kind: Kind) {
    assert_load_err(c, path, |k| {
        matches!(
            k,
            LoadErrorKind::OperatorNotAllowed { op: o, kind: x, .. } if o == op && *x == kind
        )
    });
}

/// The `InvalidValue` reason for a string that is not a hyphenated UUID
/// (the whole reason).
pub fn not_uuid(text: &str) -> String {
    format!("{text:?} is not a hyphenated UUID")
}

/// The start of the `InvalidValue` reason for a string that is not a
/// `YYYY-MM-DD` date; chrono's own message follows.
pub fn not_date(text: &str) -> String {
    format!("{text:?} is not a YYYY-MM-DD date: ")
}

/// The start of the `InvalidValue` reason for a string that is not an
/// RFC 3339 date-time; chrono's own message follows.
pub fn not_date_time(text: &str) -> String {
    format!("{text:?} is not an RFC 3339 date-time: ")
}
