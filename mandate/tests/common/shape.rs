// Shape predicates over the condition AST: the §5.1 canonical form that
// `build()` leaves rules in, and the §8 restricted NNF of filter plans.

use mandate::{Condition, Quant};

/// §5.1 canonical form: groups have two or more distinct children and no
/// child of their own kind, no constants, no double negation, and
/// `In`/`NotIn` have values.
pub fn canonical(c: &Condition) -> bool {
    fn group(cs: &[Condition], same: fn(&Condition) -> bool) -> bool {
        cs.len() >= 2
            && !cs.iter().any(same)
            && cs.iter().enumerate().all(|(i, c)| !cs[..i].contains(c))
            && cs.iter().all(canonical)
    }
    match c {
        Condition::And(cs) => group(cs, |c| matches!(c, Condition::And(_))),
        Condition::Or(cs) => group(cs, |c| matches!(c, Condition::Or(_))),
        Condition::Not(inner) => !matches!(**inner, Condition::Not(_)) && canonical(inner),
        Condition::In { values, .. } | Condition::NotIn { values, .. } => !values.is_empty(),
        Condition::Rel { cond, .. } => cond.as_deref().is_none_or(canonical),
        _ => true,
    }
}

/// §8 restricted NNF on top of the canonical form: no `Every`, and `Not`
/// only directly above a `Str` leaf.
pub fn plan_shaped(c: &Condition) -> bool {
    plan_shape_error(c).is_none()
}

/// Why `c` is not [`plan_shaped`]: "Every quantifier", "Not above a non-Str
/// node" or "not canonical"; `None` if it is.
pub fn plan_shape_error(c: &Condition) -> Option<&'static str> {
    fn nnf(c: &Condition) -> Option<&'static str> {
        match c {
            Condition::Not(inner) => {
                (!matches!(**inner, Condition::Str { .. })).then_some("Not above a non-Str node")
            }
            Condition::Rel {
                quant: Quant::Every,
                ..
            } => Some("Every quantifier"),
            Condition::Rel { cond, .. } => cond.as_deref().and_then(nnf),
            Condition::And(cs) | Condition::Or(cs) => cs.iter().find_map(nnf),
            _ => None,
        }
    }
    nnf(c).or_else(|| (!canonical(c)).then_some("not canonical"))
}
