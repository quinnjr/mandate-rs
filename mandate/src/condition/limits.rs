//! Nesting and alternation limits, and the build-time checks on condition
//! operands.
//!
//! Conditions are compiled, evaluated, folded and planned recursively, so
//! their nesting is capped to keep every pass within a small, fixed stack.
//! There are three limits. Two cap nesting, on two representations:
//!
//! - [`MAX_BUILD_DEPTH`] caps the [`Condition`] AST that `build()` accepts,
//!   whether built in code or bound from templates: every `And`, `Or`, `Not`
//!   and `Rel` is one level, and leaves are at depth 0.
//! - [`MAX_TEMPLATE_DEPTH`] caps the JSON condition objects that
//!   `Templates::compile` accepts, as it walks them: `conditions` is level 0,
//!   and each `$and`/`$or` element, `$not` operand and relation operand is
//!   one level deeper than its parent.
//!
//! One template level compiles to at most two AST levels (the `And` of an
//! object's keys, then the `$and`/`$or`, `$not` or relation node of one of
//! them), so `MAX_BUILD_DEPTH` is twice `MAX_TEMPLATE_DEPTH`. The deepest
//! templates within the template limit still come out a few levels deeper
//! than that (multi-key objects and operator objects at the innermost level,
//! or an unresolved placeholder's constant); `build()` rejects those with
//! [`BuildError::TooDeep`] like any other condition. The template limit
//! exists to bound the compiler's own recursion.
//!
//! The third, [`MAX_ALTERNATIONS`], caps the rules rather than a condition:
//! how often the rules of one (action, subject) pair switch between `can`
//! and `cannot`. Each switch nests the `access` formula one level deeper,
//! so `build()` rejects more with [`BuildError::TooManyAlternations`].

use crate::{BuildError, Condition, Value};

/// Deepest allowed nesting of `And`/`Or`/`Not`/`Rel` in a rule condition
/// at `build()` (see [`BuildError::TooDeep`]).
pub(crate) const MAX_BUILD_DEPTH: usize = 64;

/// Deepest allowed nesting of condition objects below `conditions` (which
/// is level 0) in a template; deeper is `LoadErrorKind::Malformed`.
pub(crate) const MAX_TEMPLATE_DEPTH: usize = 32;

// One template level compiles to at most two AST levels (see the module
// docs), so the two nesting limits move together.
const _: () = assert!(MAX_BUILD_DEPTH == 2 * MAX_TEMPLATE_DEPTH);

/// Most `can`/`cannot` switches allowed among the rules of one cell (see
/// [`BuildError::TooManyAlternations`]).
pub(crate) const MAX_ALTERNATIONS: usize = 256;

/// The nesting depth of `c`: every `And`, `Or`, `Not` and `Rel` is one level,
/// leaves are at depth 0. Iterative, so any depth is measured safely.
fn depth(c: &Condition) -> usize {
    let mut max = 0;
    let mut stack = vec![(c, 0)];
    while let Some((c, d)) = stack.pop() {
        let d = match c {
            Condition::And(cs) | Condition::Or(cs) => {
                stack.extend(cs.iter().map(|c| (c, d + 1)));
                d + 1
            }
            Condition::Not(c) => {
                stack.push((c, d + 1));
                d + 1
            }
            Condition::Rel { cond, .. } => {
                stack.extend(cond.as_deref().map(|c| (c, d + 1)));
                d + 1
            }
            Condition::Cmp { .. }
            | Condition::In { .. }
            | Condition::NotIn { .. }
            | Condition::Str { .. }
            | Condition::IsNull(_)
            | Condition::IsNotNull(_) => d,
        };
        max = max.max(d);
    }
    max
}

/// Rejects a condition nested deeper than [`MAX_BUILD_DEPTH`]. Runs before
/// any recursive pass over the condition.
pub(crate) fn check_depth(c: &Condition, subject: &'static str) -> Result<(), BuildError> {
    match depth(c) {
        d if d > MAX_BUILD_DEPTH => Err(BuildError::TooDeep { subject, depth: d }),
        _ => Ok(()),
    }
}

/// Rejects non-finite float operands anywhere in `c`, which must be of
/// bounded depth ([`check_depth`]).
pub(crate) fn check_finite(c: &Condition) -> Result<(), BuildError> {
    let bad = |v: &Value| {
        if v.is_finite() {
            Ok(())
        } else {
            Err(BuildError::InvalidValue {
                reason: format!("non-finite float operand {v:?}"),
            })
        }
    };
    match c {
        Condition::Cmp { value, .. } => bad(value),
        Condition::In { values, .. } | Condition::NotIn { values, .. } => {
            values.iter().try_for_each(bad)
        }
        Condition::And(cs) | Condition::Or(cs) => cs.iter().try_for_each(check_finite),
        Condition::Not(c) => check_finite(c),
        Condition::Rel { cond, .. } => cond.as_deref().map_or(Ok(()), check_finite),
        Condition::Str { .. } | Condition::IsNull(_) | Condition::IsNotNull(_) => Ok(()),
    }
}
