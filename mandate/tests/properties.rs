#![cfg(all(feature = "derive", feature = "chrono", feature = "uuid"))]
//! Property tests (spec §10, items 1–6): in-memory checks, filter plans,
//! field plans, projections, and stored-rule round trips agree.
//!
//! Besides comparing the library's entry points with each other, the
//! properties compare them with `model`, an independent reading of the
//! §5.3 semantics and the §7.1–§7.4 rule walks over the generated
//! (unfolded) conditions.

mod common;

use common::fixture::*;
use common::shape::{canonical, plan_shaped};
use common::strategies::*;
use common::unresolved::as_tuples;
use mandate::{
    Ability, Access, CheckError, Condition, Context, EvalError, FieldDef, FieldIdx, FieldKind,
    FieldMask, Projection, Quant, RelationProjection, Resource as _, RuleTemplate, Schema,
    TemplateValue, Templates, Unresolved, UnresolvedOutcome,
};
use proptest::prelude::*;
use proptest::sample::Index;
use proptest::test_runner::TestRunner;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering::Relaxed;

type Ab = Ability<Action, TSubject>;

/// An independent model of the semantics, for fully loaded instances only.
mod model {
    use super::*;
    use mandate::{CmpOp, StrOp, Value};

    /// A scalar read from an instance.
    pub enum Mv<'a> {
        Null,
        Int(i64),
        Float(f64),
        Str(&'a str),
        Uuid(uuid::Uuid),
        Date(chrono::NaiveDate),
    }

    /// A fully loaded instance.
    pub trait Model {
        fn scalar(&self, f: FieldIdx) -> Mv<'_>;
        /// The related rows of relation `f` (none for an absent to-one), or
        /// `None` if `f` is not a relation.
        fn rows(&self, f: FieldIdx) -> Option<Vec<&dyn Model>>;
    }

    fn loaded<S>(slot: &Lazy<S>) -> &S {
        match slot {
            Lazy::Loaded(s) => s,
            Lazy::NotLoaded => panic!("the model needs loaded relations"),
        }
    }

    fn all_loaded(l: &Loaded) {
        assert!(l.0.is_empty(), "the model needs loaded scalars: {l:?}");
    }

    impl Model for TPost {
        fn scalar(&self, f: FieldIdx) -> Mv<'_> {
            all_loaded(&self.loaded);
            if f == TPost::ID.idx() {
                Mv::Int(self.id)
            } else if f == TPost::AUTHOR_ID.idx() {
                Mv::Int(self.author_id)
            } else if f == TPost::REVIEWER_ID.idx() {
                self.reviewer_id.map_or(Mv::Null, Mv::Int)
            } else if f == TPost::TITLE.idx() {
                Mv::Str(&self.title)
            } else if f == TPost::STATUS.idx() {
                Mv::Str(match self.status {
                    Status::Draft => "draft",
                    Status::Published => "published",
                    Status::Archived => "archived",
                })
            } else if f == TPost::SCORE.idx() {
                Mv::Float(self.score)
            } else if f == TPost::OWNER.idx() {
                Mv::Uuid(self.owner)
            } else if f == TPost::DUE.idx() {
                self.due.map_or(Mv::Null, Mv::Date)
            } else {
                panic!("not a TPost scalar: {f:?}")
            }
        }
        fn rows(&self, f: FieldIdx) -> Option<Vec<&dyn Model>> {
            if f == TPost::ORG.idx() {
                Some(vec![loaded(&self.org) as &dyn Model])
            } else if f == TPost::REVIEWER.idx() {
                Some(
                    loaded(&self.reviewer)
                        .iter()
                        .map(|u| u as &dyn Model)
                        .collect(),
                )
            } else if f == TPost::TAGS.idx() {
                Some(loaded(&self.tags).iter().map(|t| t as &dyn Model).collect())
            } else {
                None
            }
        }
    }

    impl Model for TOrg {
        fn scalar(&self, f: FieldIdx) -> Mv<'_> {
            all_loaded(&self.loaded);
            if f == TOrg::ID.idx() {
                Mv::Int(self.id)
            } else if f == TOrg::NAME.idx() {
                Mv::Str(&self.name)
            } else {
                panic!("not a TOrg scalar: {f:?}")
            }
        }
        fn rows(&self, _: FieldIdx) -> Option<Vec<&dyn Model>> {
            None
        }
    }

    impl Model for TUser {
        fn scalar(&self, f: FieldIdx) -> Mv<'_> {
            all_loaded(&self.loaded);
            if f == TUser::ID.idx() {
                Mv::Int(self.id)
            } else if f == TUser::EMAIL.idx() {
                Mv::Str(&self.email)
            } else {
                panic!("not a TUser scalar: {f:?}")
            }
        }
        fn rows(&self, _: FieldIdx) -> Option<Vec<&dyn Model>> {
            None
        }
    }

    impl Model for TTag {
        fn scalar(&self, f: FieldIdx) -> Mv<'_> {
            all_loaded(&self.loaded);
            if f == TTag::ID.idx() {
                Mv::Int(self.id)
            } else if f == TTag::NAME.idx() {
                self.name.as_deref().map_or(Mv::Null, Mv::Str)
            } else {
                panic!("not a TTag scalar: {f:?}")
            }
        }
        fn rows(&self, f: FieldIdx) -> Option<Vec<&dyn Model>> {
            (f == TTag::AUTHOR.idx()).then(|| {
                loaded(&self.author)
                    .iter()
                    .map(|u| u as &dyn Model)
                    .collect()
            })
        }
    }

    /// `None` if `v` is null; operands always have the field's kind.
    fn equals(v: &Mv<'_>, x: &Value) -> Option<bool> {
        Some(match (v, x) {
            (Mv::Null, _) => return None,
            (Mv::Int(a), Value::Int(b)) => a == b,
            (Mv::Float(a), Value::Float(b)) => a == b,
            (Mv::Str(a), Value::String(b)) => *a == b.as_str(),
            (Mv::Uuid(a), Value::Uuid(b)) => a == b,
            (Mv::Date(a), Value::Date(b)) => a == b,
            (_, x) => panic!("operand {x:?} does not fit the field"),
        })
    }

    fn order(v: &Mv<'_>, x: &Value) -> Option<std::cmp::Ordering> {
        match (v, x) {
            (Mv::Null, _) => None,
            (Mv::Int(a), Value::Int(b)) => Some(a.cmp(b)),
            (Mv::Float(a), Value::Float(b)) => a.partial_cmp(b),
            (Mv::Date(a), Value::Date(b)) => Some(a.cmp(b)),
            (_, x) => panic!("operand {x:?} is not ordered against the field"),
        }
    }

    /// §5.3: two-valued logic; on a null value only `Ne`, `NotIn` and
    /// `IsNull` hold.
    pub fn eval(c: &Condition, r: &dyn Model) -> bool {
        use std::cmp::Ordering::{Equal, Greater, Less};
        match c {
            Condition::Cmp { field, op, value } => {
                let v = r.scalar(*field);
                match op {
                    CmpOp::Eq => equals(&v, value) == Some(true),
                    CmpOp::Ne => equals(&v, value) != Some(true),
                    CmpOp::Lt => order(&v, value) == Some(Less),
                    CmpOp::Lte => matches!(order(&v, value), Some(Less | Equal)),
                    CmpOp::Gt => order(&v, value) == Some(Greater),
                    CmpOp::Gte => matches!(order(&v, value), Some(Greater | Equal)),
                    op => unreachable!("unknown operator {op:?}"),
                }
            }
            Condition::In { field, values } => {
                let v = r.scalar(*field);
                values.iter().any(|x| equals(&v, x) == Some(true))
            }
            Condition::NotIn { field, values } => {
                let v = r.scalar(*field);
                !values.iter().any(|x| equals(&v, x) == Some(true))
            }
            Condition::Str { field, op, value } => match r.scalar(*field) {
                Mv::Str(s) => match op {
                    StrOp::Contains => s.contains(value.as_str()),
                    StrOp::StartsWith => s.starts_with(value.as_str()),
                    StrOp::EndsWith => s.ends_with(value.as_str()),
                    op => unreachable!("unknown operator {op:?}"),
                },
                Mv::Null => false,
                _ => panic!("text operator on a non-text field"),
            },
            Condition::IsNull(f) => is_null(r, *f),
            Condition::IsNotNull(f) => !is_null(r, *f),
            Condition::And(cs) => cs.iter().all(|c| eval(c, r)),
            Condition::Or(cs) => cs.iter().any(|c| eval(c, r)),
            Condition::Not(c) => !eval(c, r),
            Condition::Rel {
                relation,
                quant,
                cond,
            } => {
                let rows = r.rows(*relation).expect("a relation");
                let holds = |row: &&dyn Model| cond.as_deref().is_none_or(|c| eval(c, *row));
                match quant {
                    Quant::One | Quant::Some => rows.iter().any(holds),
                    Quant::Every => rows.iter().all(holds),
                    Quant::None => !rows.iter().any(holds),
                    q => unreachable!("unknown quantifier {q:?}"),
                }
            }
            c => unreachable!("unknown condition {c:?}"),
        }
    }

    fn is_null(r: &dyn Model, f: FieldIdx) -> bool {
        match r.rows(f) {
            Some(rows) => rows.is_empty(),
            None => matches!(r.scalar(f), Mv::Null),
        }
    }

    fn matches(s: &RuleSpec, p: &TPost) -> bool {
        s.cond.as_ref().is_none_or(|c| eval(c.condition(), p))
    }

    /// §7.1–§7.2: the last applicable matching group decides; none denies.
    pub fn can(specs: &[RuleSpec], a: Action, p: &TPost, field: Option<FieldIdx>) -> bool {
        specs
            .iter()
            .rev()
            .filter(|s| s.covers(a))
            .filter(|s| match (field, &s.fields) {
                (_, None) => true,
                (Some(f), Some(fs)) => fs.iter().any(|x| x.idx() == f),
                (None, Some(_)) => !s.inverted,
            })
            .find(|s| matches(s, p))
            .is_some_and(|s| !s.inverted)
    }

    /// §7.4: in definition order, matching `can`s add and `cannot`s remove
    /// their fields (all fields if none are listed).
    pub fn permitted(specs: &[RuleSpec], a: Action, p: &TPost) -> FieldMask {
        let mut set = FieldMask::default();
        for s in specs.iter().filter(|s| s.covers(a) && matches(s, p)) {
            let fields: Vec<FieldIdx> = match &s.fields {
                Some(fs) => fs.iter().map(|f| f.idx()).collect(),
                None => tpost_fields().iter().map(|f| f.idx()).collect(),
            };
            for f in fields {
                if s.inverted {
                    set.remove(f);
                } else {
                    set.insert(f);
                }
            }
        }
        set
    }
}

/// A check's outcome: allowed, denied, or unresolvable.
fn verdict(r: Result<(), CheckError<Action, TSubject>>) -> Result<bool, EvalError> {
    match r {
        Ok(()) => Ok(true),
        Err(CheckError::Forbidden(_)) => Ok(false),
        Err(CheckError::Unresolvable(e)) => Err(e),
        Err(e) => unreachable!("unknown check error {e}"),
    }
}

/// Whether the rows `access` allows include `p`.
fn admits(access: &Access<TPost>, p: &TPost) -> Result<bool, EvalError> {
    match access {
        Access::Denied => Ok(false),
        Access::All => Ok(true),
        Access::Filter(plan) => plan.eval(p),
    }
}

/// `access`, after checking that a filter plan has the §8 shape.
fn access(a: &Ab, act: Action) -> Result<Access<TPost>, TestCaseError> {
    let acc = a
        .access::<TPost>(act)
        .expect("TPost's schema is its subject's");
    if let Access::Filter(plan) = &acc {
        prop_assert!(
            plan_shaped(plan.condition()),
            "malformed plan {:?}",
            plan.condition()
        );
    }
    Ok(acc)
}

/// The fields of every relation of `TPost`.
fn relation_fields() -> FieldMask {
    let mut m = FieldMask::default();
    for f in [TPost::ORG.idx(), TPost::REVIEWER.idx(), TPost::TAGS.idx()] {
        m.insert(f);
    }
    m
}

/// The names of `schema`'s fields outside `fetched`, reported as not loaded.
fn unfetched(schema: &Schema, fetched: FieldMask) -> Loaded {
    Loaded(
        (0..schema.fields().len())
            .filter(|&i| !fetched.contains(FieldIdx(i as u16)))
            .map(|i| schema.fields()[i].name())
            .collect(),
    )
}

/// Checks a projection's relation tree against the schemas: each entry is
/// a relation of its owner with that relation's target, entries are sorted
/// by relation without duplicates, and the fields read exist on the target.
fn check_relations(rels: &[RelationProjection], owner: &'static Schema) {
    assert!(
        rels.windows(2).all(|w| w[0].relation < w[1].relation),
        "unsorted or duplicate relations: {rels:?}"
    );
    for r in rels {
        let Some(FieldKind::Relation { target, .. }) = owner.field(r.relation).map(FieldDef::kind)
        else {
            panic!("{:?} is not a relation of {}", r.relation, owner.name())
        };
        assert!(std::ptr::eq(target(), r.target), "wrong target in {r:?}");
        let all = FieldMask::all(r.target.fields().len());
        assert!(r.fields.is_subset(&all), "unknown fields in {r:?}");
        check_relations(&r.relations, r.target);
    }
}

/// The projection entry for relation `r`, if it is a dependency.
fn dependency(rels: &[RelationProjection], r: FieldIdx) -> Option<&RelationProjection> {
    rels.iter().find(|x| x.relation == r)
}

/// `p` reporting `NotLoaded` for everything outside `proj`: unfetched
/// scalars, relations that are not condition dependencies, and, within each
/// dependency, the target fields and nested relations (`tags.author`) it
/// does not read.
fn fetch(p: &TPost, proj: &Projection<TPost>) -> TPost {
    check_relations(&proj.relations, TPost::schema());
    let user = |d: &RelationProjection, u: &TUser| TUser {
        loaded: unfetched(TUser::schema(), d.fields),
        ..u.clone()
    };
    let org = match (dependency(&proj.relations, TPost::ORG.idx()), &p.org) {
        (Some(d), Lazy::Loaded(o)) => Lazy::Loaded(TOrg {
            loaded: unfetched(TOrg::schema(), d.fields),
            ..o.clone()
        }),
        _ => Lazy::NotLoaded,
    };
    let reviewer = match (
        dependency(&proj.relations, TPost::REVIEWER.idx()),
        &p.reviewer,
    ) {
        (Some(d), Lazy::Loaded(u)) => Lazy::Loaded(u.as_ref().map(|u| user(d, u))),
        _ => Lazy::NotLoaded,
    };
    let tag = |d: &RelationProjection, t: &TTag| TTag {
        loaded: unfetched(TTag::schema(), d.fields),
        author: match (dependency(&d.relations, TTag::AUTHOR.idx()), &t.author) {
            (Some(a), Lazy::Loaded(u)) => Lazy::Loaded(u.as_ref().map(|u| user(a, u))),
            _ => Lazy::NotLoaded,
        },
        ..t.clone()
    };
    let tags = match (dependency(&proj.relations, TPost::TAGS.idx()), &p.tags) {
        (Some(d), Lazy::Loaded(ts)) => Lazy::Loaded(ts.iter().map(|t| tag(d, t)).collect()),
        _ => Lazy::NotLoaded,
    };
    TPost {
        loaded: unfetched(TPost::schema(), proj.fields.mask()),
        org,
        reviewer,
        tags,
        ..p.clone()
    }
}

/// Calls `f` on every literal leaf of a template condition object (a value
/// under a field key or under a comparison or text operator, or a whole
/// `$in`/`$nin` array) with the leaf's polarity: negative under an odd
/// number of `$not`s and `$none`s (§6.4).
fn literals(
    entries: &mut [(String, TemplateValue)],
    negative: bool,
    f: &mut dyn FnMut(&mut TemplateValue, bool),
) {
    fn object(v: &mut TemplateValue, negative: bool, f: &mut dyn FnMut(&mut TemplateValue, bool)) {
        let TemplateValue::Object(entries) = v else {
            panic!("expected an object, found {v:?}")
        };
        literals(entries, negative, f);
    }
    for (key, value) in entries.iter_mut() {
        match key.as_str() {
            "$and" | "$or" => {
                let TemplateValue::Array(items) = value else {
                    panic!("expected an array, found {value:?}")
                };
                for item in items {
                    object(item, negative, f);
                }
            }
            "$some" | "$every" => object(value, negative, f),
            "$not" | "$none" => object(value, !negative, f),
            "$in" | "$nin" => f(value, negative),
            "$eq" | "$ne" | "$lt" | "$lte" | "$gt" | "$gte" | "$contains" | "$startsWith"
            | "$endsWith" => {
                if !matches!(value, TemplateValue::Null) {
                    f(value, negative);
                }
            }
            "$isNull" => {}
            op if op.starts_with('$') => panic!("unexpected operator `{op}`"),
            _field => match value {
                TemplateValue::Object(_) => object(value, negative, f),
                TemplateValue::Null => {}
                _ => f(value, negative),
            },
        }
    }
}

/// The context value a serialized literal stands for: the serializer adds a
/// `$` to every literal string starting with `$` (§6.3).
fn context_value(v: &TemplateValue) -> serde_json::Value {
    match v {
        TemplateValue::String(s) if s.starts_with('$') => {
            assert!(s.starts_with("$$"), "unescaped literal {s:?}");
            serde_json::Value::String(s[1..].to_owned())
        }
        TemplateValue::Array(items) => items.iter().map(context_value).collect(),
        other => serde_json::to_value(other).expect("literals serialize"),
    }
}

/// Compiles serialized rules, binds them to `ctx`, and builds the result;
/// also returns the unresolved placeholders.
fn round_trip(rules: &str, roots: &[&str], ctx: &Context) -> (Ab, Vec<Unresolved>) {
    let raw: Vec<RuleTemplate> = serde_json::from_str(rules).expect("serialized rules parse");
    let bound = Templates::<Action, TSubject>::compile(&raw, roots)
        .expect("serialized rules compile")
        .bind(ctx)
        .expect("serialized rules bind");
    let unresolved = bound.unresolved().to_vec();
    let ability = Ab::builder()
        .extend(bound)
        .build()
        .expect("bound rules build");
    (ability, unresolved)
}

/// §10.1: on partially loaded instances, `can` and the plan never give two
/// different answers; errors are `NotLoaded` and `can` fails closed.
///
/// A property like the ones in `proptest!` below, run by hand so that it can
/// also count the cases where `check` or the plan was unresolvable: the run
/// must reach that branch. `p1_partial_errors_are_not_loaded` pins it
/// deterministically.
#[test]
fn p1_partial_never_contradicts() {
    let (cases, checks_failed, plans_failed) = (
        AtomicUsize::new(0),
        AtomicUsize::new(0),
        AtomicUsize::new(0),
    );
    let mut runner = TestRunner::new(ProptestConfig {
        test_name: Some(concat!(module_path!(), "::p1_partial_never_contradicts")),
        source_file: Some(file!()),
        ..ProptestConfig::default()
    });
    let result = runner.run(&(rules(), tpost(false)), |(rs, p)| {
        let a = build(&rs);
        let (mut check_failed, mut plan_failed) = (false, false);
        for act in ACTIONS {
            let checked = verdict(a.check(act, &p));
            let planned = admits(&access(&a, act)?, &p);
            if let (Ok(x), Ok(y)) = (&checked, &planned) {
                prop_assert_eq!(x, y, "{:?}", act);
            }
            for r in [&checked, &planned] {
                if let Err(e) = r {
                    prop_assert!(matches!(e, EvalError::NotLoaded { .. }), "{:?}", e);
                }
            }
            prop_assert_eq!(a.can(act, &p), checked == Ok(true));
            check_failed |= checked.is_err();
            plan_failed |= planned.is_err();
        }
        cases.fetch_add(1, Relaxed);
        checks_failed.fetch_add(usize::from(check_failed), Relaxed);
        plans_failed.fetch_add(usize::from(plan_failed), Relaxed);
        Ok(())
    });
    if let Err(e) = result {
        panic!("{e}\n{runner}");
    }
    let (cases, checks_failed, plans_failed) = (
        cases.into_inner(),
        checks_failed.into_inner(),
        plans_failed.into_inner(),
    );
    // About a quarter of the cases have an unresolvable check and a sixth
    // an unresolvable plan, so 128 cases miss either with probability
    // below 1e-10. Smaller runs (`PROPTEST_CASES`) skip this.
    if cases >= 128 {
        assert!(
            checks_failed > 0 && plans_failed > 0,
            "{cases} cases: {checks_failed} unresolvable checks, {plans_failed} unresolvable plans"
        );
    }
}

/// The error branch of `p1_partial_never_contradicts`, deterministically:
/// a condition on an unloaded scalar, an unloaded to-one or an unloaded
/// to-many relation makes `check` and the plan fail with `NotLoaded` at that
/// field, in a `can` rule and in a `cannot` rule, and `can` denies.
#[test]
fn p1_partial_errors_are_not_loaded() {
    let full = loaded_tpost();
    let cases = [
        (
            TPost::TITLE.eq("Hello"),
            TPost {
                loaded: Loaded(vec!["title"]),
                ..full.clone()
            },
            "title",
        ),
        (
            TPost::ORG.then(TOrg::NAME.eq("Acme")),
            TPost {
                org: Lazy::NotLoaded,
                ..full.clone()
            },
            "org",
        ),
        (
            TPost::TAGS.none(TTag::ID.eq(1)),
            TPost {
                tags: Lazy::NotLoaded,
                ..full.clone()
            },
            "tags",
        ),
    ];
    for (cond, p, path) in cases {
        let not_loaded = |r: &Result<bool, EvalError>| matches!(r, Err(EvalError::NotLoaded { path: x, .. }) if x == path);
        let spec = |inverted, cond| RuleSpec {
            inverted,
            actions: vec![Action::Read],
            cond,
            fields: None,
            reason: None,
        };
        // `can read when c`, and `can read; cannot read when c`: on the full
        // instance `c` holds, so the first allows and the second denies.
        for (rs, on_full) in [
            (vec![spec(false, Some(cond.clone()))], true),
            (
                vec![spec(false, None), spec(true, Some(cond.clone()))],
                false,
            ),
        ] {
            let a = build(&rs);
            assert_eq!(verdict(a.check(Action::Read, &full)), Ok(on_full), "{rs:?}");
            let checked = verdict(a.check(Action::Read, &p));
            assert!(not_loaded(&checked), "{rs:?}: {checked:?}");
            assert!(!a.can(Action::Read, &p), "{rs:?}");
            let access = a
                .access::<TPost>(Action::Read)
                .expect("TPost's schema is its subject's");
            let planned = admits(&access, &p);
            assert!(not_loaded(&planned), "{rs:?}: {planned:?}");
        }
    }
}

proptest! {
    /// §10.1: for fully loaded instances, the filter plan admits exactly
    /// the instances `can` allows, and both match the model (also per field).
    #[test]
    fn p1_plan_equals_can(rs in rules(), p in tpost(true)) {
        let a = build(&rs);
        for act in ACTIONS {
            let can = verdict(a.check(act, &p));
            prop_assert_eq!(&can, &Ok(model::can(&rs, act, &p, None)), "{:?}", act);
            prop_assert_eq!(a.can(act, &p), can == Ok(true));
            prop_assert_eq!(admits(&access(&a, act)?, &p), can, "{:?}", act);
            for f in tpost_fields() {
                prop_assert_eq!(
                    verdict(a.check_field(act, &p, f)),
                    Ok(model::can(&rs, act, &p, Some(f.idx()))),
                    "{:?} {:?}", act, f
                );
            }
        }
    }

    /// §10.2: folding and NNF preserve meaning: the model's reading of the
    /// raw condition, `can`, and the plan agree; the plan has the §8 shape.
    #[test]
    fn p2_nnf_preserves_meaning(c in cond_tpost(4), p in tpost(true)) {
        let expected = model::eval(c.condition(), &p);
        let a = Ab::builder()
            .can(Action::Read, TSubject::TPost)
            .when(c)
            .build()
            .expect("generated conditions build");
        prop_assert_eq!(verdict(a.check(Action::Read, &p)), Ok(expected));
        prop_assert_eq!(admits(&access(&a, Action::Read)?, &p), Ok(expected));
        for r in a.rules() {
            prop_assert!(r.condition().is_none_or(canonical), "{:?}", r.condition());
        }
    }

    /// §10.3: bound rules survive a JSON text round trip exactly.
    #[test]
    fn p3_round_trip(rs in rules()) {
        let original = build(&rs);
        for r in original.rules() {
            prop_assert!(r.condition().is_none_or(canonical), "{:?}", r.condition());
        }
        let json = serde_json::to_string(original.rules()).expect("rules serialize");
        let (rebuilt, unresolved) = round_trip(&json, &[], &Context::empty());
        prop_assert_eq!(unresolved, vec![]);
        prop_assert_eq!(rebuilt.rules(), original.rules(), "json: {}", json);
    }

    /// §10.4: the projection covers the permitted fields, and fetching only
    /// the projection lets `can`, `can_field` and `permitted_fields` run
    /// without `NotLoaded`, with the same answers as on the full instance.
    #[test]
    fn p4_projection_suffices(rs in rules(), p in tpost(true)) {
        let a = build(&rs);
        for act in ACTIONS {
            let proj = a.projection::<TPost>(act).expect("TPost's schema is its subject's");
            let permitted = a.permitted_fields(act, &p).expect("fully loaded");
            prop_assert!(
                permitted.mask().is_subset(&proj.fields.mask().union(relation_fields())),
                "{:?}: permitted {:?} outside {:?}", act, permitted, proj
            );
            let fetched = fetch(&p, &proj);
            prop_assert_eq!(
                verdict(a.check(act, &fetched)),
                verdict(a.check(act, &p)),
                "{:?} {:?}", act, proj
            );
            for f in tpost_fields() {
                prop_assert_eq!(
                    verdict(a.check_field(act, &fetched, f)),
                    verdict(a.check_field(act, &p, f)),
                    "{:?} {:?} {:?}", act, f, proj
                );
            }
            prop_assert_eq!(
                a.permitted_fields(act, &fetched),
                Ok(permitted),
                "{:?} {:?}", act, proj
            );
        }
    }

    /// §10.5: the field plan, combined with each rule's match, gives
    /// `permitted_fields`; each rule's plan matches when the model says the
    /// rule's condition holds, and both agree with the model's walk.
    #[test]
    fn p5_field_plan_equals_permitted(rs in rules(), p in tpost(true)) {
        let a = build(&rs);
        for act in ACTIONS {
            let fp = a.field_plan::<TPost>(act).expect("TPost's schema is its subject's");
            let covering: Vec<_> = a
                .rules()
                .iter()
                .filter(|r| r.action() == act || r.action() == Action::Manage)
                .collect();
            prop_assert_eq!(fp.rules.len(), covering.len());
            let mut matched = Vec::new();
            for (fr, r) in fp.rules.iter().zip(&covering) {
                prop_assert_eq!(fr.inverted, r.inverted());
                prop_assert_eq!(fr.fields.map(|f| f.mask()), r.fields());
                let m = match &fr.cond {
                    None => true,
                    Some(plan) => {
                        prop_assert!(plan_shaped(plan.condition()), "{:?}", plan);
                        plan.eval(&p).expect("fully loaded")
                    }
                };
                prop_assert_eq!(m, r.condition().is_none_or(|c| model::eval(c, &p)), "{:?}", r);
                matched.push(m);
            }
            let permitted = a.permitted_fields(act, &p).expect("fully loaded");
            prop_assert_eq!(fp.permitted(|i| matched[i]), permitted, "{:?}", act);
            prop_assert_eq!(permitted.mask(), model::permitted(&rs, act, &p), "{:?}", act);
        }
    }

    /// §10.6: replacing one literal of the serialized rules with a required
    /// placeholder, the ability bound with the original value permits at
    /// least as much as the one bound with the placeholder unresolved.
    #[test]
    fn p6_most_restrictive(rs in rules(), p in tpost(true), pick in any::<Index>()) {
        let original = build(&rs);
        let json = serde_json::to_string(original.rules()).expect("rules serialize");
        let mut raw: Vec<RuleTemplate> = serde_json::from_str(&json).expect("rules parse");
        let mut count = 0;
        for r in raw.iter_mut() {
            if let Some(TemplateValue::Object(c)) = &mut r.conditions {
                literals(c, false, &mut |_, _| count += 1);
            }
        }
        prop_assume!(count > 0);
        let target = pick.index(count);
        let (mut i, mut replaced) = (0, None);
        for (rule_index, r) in raw.iter_mut().enumerate() {
            let inverted = r.inverted;
            if let Some(TemplateValue::Object(c)) = &mut r.conditions {
                literals(c, false, &mut |v, negative| {
                    if i == target {
                        let placeholder = TemplateValue::String("${ctx.v}".into());
                        let old = std::mem::replace(v, placeholder);
                        replaced = Some((old, rule_index, inverted, negative));
                    }
                    i += 1;
                });
            }
        }
        let (old, rule_index, inverted, negative) = replaced.expect("the picked literal");
        let value = context_value(&old);
        // The least-access constant: false in a `can` rule, true in a
        // `cannot` rule, swapped under negative polarity (§6.4).
        let expected = (
            rule_index,
            "ctx.v",
            if inverted == negative {
                UnresolvedOutcome::LeafFalse
            } else {
                UnresolvedOutcome::LeafTrue
            },
        );
        let template = serde_json::to_string(&raw).expect("templates serialize");

        let ctx = Context::new().with("ctx", &serde_json::json!({ "v": value })).expect("json");
        let (resolved, leftovers) = round_trip(&template, &["ctx"], &ctx);
        prop_assert_eq!(leftovers, vec![]);
        prop_assert_eq!(resolved.rules(), original.rules(), "template: {}", template);
        let (restricted, reported) = round_trip(&template, &["ctx"], &Context::empty());
        prop_assert_eq!(as_tuples(&reported), vec![expected], "template: {}", template);

        for act in ACTIONS {
            let can = |a: &Ab| verdict(a.check(act, &p)).expect("fully loaded");
            prop_assert!(!can(&restricted) || can(&resolved), "{:?} can; {}", act, template);

            let admitted = |a: &Ab| {
                let acc = a.access::<TPost>(act).expect("TPost's schema is its subject's");
                admits(&acc, &p).expect("fully loaded")
            };
            prop_assert!(
                !admitted(&restricted) || admitted(&resolved),
                "{:?} access; {}", act, template
            );

            let fields = |a: &Ab| a.permitted_fields(act, &p).expect("fully loaded").mask();
            prop_assert!(
                fields(&restricted).is_subset(&fields(&resolved)),
                "{:?} permitted_fields; {}", act, template
            );

            let planned = |a: &Ab| {
                let fp = a.field_plan::<TPost>(act).expect("TPost's schema is its subject's");
                let matched: Vec<bool> = fp
                    .rules
                    .iter()
                    .map(|r| r.cond.as_ref().is_none_or(|c| c.eval(&p).expect("fully loaded")))
                    .collect();
                fp.permitted(|i| matched[i]).mask()
            };
            prop_assert!(
                planned(&restricted).is_subset(&planned(&resolved)),
                "{:?} field_plan; {}", act, template
            );
        }
    }
}
