// Proptest strategies over the tracked fixture (`TPost`, `TOrg`, `TUser`,
// `TTag`), built only from the public typed API. Value domains are small so
// that operands and instance values collide often.

use std::fmt::Debug;

use chrono::NaiveDate;
use mandate::{
    Ability, AbilityBuilder, Cond, Field, FieldRef, GroupBuilder, Nullable, Ordered, Scalar,
    Textual,
};
use proptest::collection::vec;
use proptest::prelude::*;
use proptest::strategy::Union;
use uuid::Uuid;

use super::fixture::*;

/// Titles: empty, prefixes of each other, and two encodings of "é"
/// (precomposed, and `e` + combining acute) that compare unequal byte-wise.
pub static TITLES: [&str; 5] = ["", "a", "ab", "é", "e\u{301}"];

/// The other strings (org names, user emails, tag names, reasons): the
/// titles plus strings that the template encoding must escape (§6.3).
pub static TEXTS: [&str; 10] = [
    "", "a", "ab", "é", "e\u{301}", "$", "$a", "$$a", "${a}", "${ctx.v}",
];

/// Owners (and owner operands): the fixture's [`OWNER`], the nil UUID, and
/// the max UUID.
pub static UUIDS: [Uuid; 3] = [OWNER, Uuid::nil(), Uuid::max()];

/// Due dates (and date operands): a leap day, the day after it, and the
/// first day of the next year. Instances may also have no due date.
pub fn dates() -> [NaiveDate; 3] {
    [(2024, 2, 29), (2024, 3, 1), (2025, 1, 1)]
        .map(|(y, m, d)| NaiveDate::from_ymd_opt(y, m, d).expect("valid date"))
}

/// Every action, for properties that check each one.
pub const ACTIONS: [Action; 5] = [
    Action::Read,
    Action::Create,
    Action::Update,
    Action::Delete,
    Action::Manage,
];

pub fn int() -> impl Strategy<Value = i64> + Clone {
    0i64..4
}

pub fn title() -> impl Strategy<Value = String> + Clone {
    prop::sample::select(&TITLES[..]).prop_map(String::from)
}

pub fn text() -> impl Strategy<Value = String> + Clone {
    prop::sample::select(&TEXTS[..]).prop_map(String::from)
}

pub fn uuid() -> impl Strategy<Value = Uuid> + Clone {
    prop::sample::select(&UUIDS[..])
}

pub fn date() -> impl Strategy<Value = NaiveDate> + Clone {
    prop::sample::select(dates().to_vec())
}

pub fn status() -> impl Strategy<Value = Status> + Clone {
    prop::sample::select(vec![Status::Draft, Status::Published, Status::Archived])
}

/// Finite scores only: non-finite operands are rejected at build and
/// non-finite instance values are unspecified (§5.2). `-0.0 == 0.0`.
pub fn score() -> impl Strategy<Value = f64> + Clone {
    prop::sample::select(vec![-0.0, 0.0, 0.5, 1.0, 2.5])
}

type Leaves<R> = Vec<BoxedStrategy<Cond<R>>>;

/// `eq`, `ne`, `is_in`, `not_in` (lists may be empty).
fn eq_ops<R: 'static, T: Scalar + 'static>(
    f: Field<R, T>,
    v: impl Strategy<Value = T::Inner> + Clone + 'static,
) -> Leaves<R>
where
    T::Inner: Clone + Debug,
{
    vec![
        v.clone().prop_map(move |x| f.eq(x)).boxed(),
        v.clone().prop_map(move |x| f.ne(x)).boxed(),
        vec(v.clone(), 0..3).prop_map(move |xs| f.is_in(xs)).boxed(),
        vec(v, 0..3).prop_map(move |xs| f.not_in(xs)).boxed(),
    ]
}

/// `lt`, `lte`, `gt`, `gte`.
fn ord_ops<R: 'static, T: Scalar + 'static>(
    f: Field<R, T>,
    v: impl Strategy<Value = T::Inner> + Clone + 'static,
) -> Leaves<R>
where
    T::Inner: Ordered + Clone + Debug,
{
    vec![
        v.clone().prop_map(move |x| f.lt(x)).boxed(),
        v.clone().prop_map(move |x| f.lte(x)).boxed(),
        v.clone().prop_map(move |x| f.gt(x)).boxed(),
        v.prop_map(move |x| f.gte(x)).boxed(),
    ]
}

/// `contains`, `starts_with`, `ends_with`.
fn text_ops<R: 'static, T: Scalar + 'static>(
    f: Field<R, T>,
    v: impl Strategy<Value = String> + Clone + 'static,
) -> Leaves<R>
where
    T::Inner: Textual,
{
    vec![
        v.clone().prop_map(move |x| f.contains(x)).boxed(),
        v.clone().prop_map(move |x| f.starts_with(x)).boxed(),
        v.prop_map(move |x| f.ends_with(x)).boxed(),
    ]
}

/// `is_null`, `is_not_null`.
fn null_ops<R: 'static, T: Scalar<Nullability = Nullable> + 'static>(f: Field<R, T>) -> Leaves<R> {
    vec![Just(f.is_null()).boxed(), Just(f.is_not_null()).boxed()]
}

/// Leaves combined with `and`, `or`, `!`, `Cond::all` and `Cond::any`
/// (including the empty `all`/`any` constants) up to `depth` levels.
fn combine<R: 'static>(leaf: BoxedStrategy<Cond<R>>, depth: u32) -> BoxedStrategy<Cond<R>> {
    leaf.prop_recursive(depth, 24, 3, |inner| {
        prop_oneof![
            (inner.clone(), inner.clone()).prop_map(|(a, b)| a.and(b)),
            (inner.clone(), inner.clone()).prop_map(|(a, b)| a.or(b)),
            inner.clone().prop_map(|a| !a),
            vec(inner.clone(), 0..4).prop_map(Cond::all),
            vec(inner, 0..4).prop_map(Cond::any),
        ]
    })
    .boxed()
}

/// Depth of conditions nested under a relation of a `depth`-level condition.
fn inner_depth(depth: u32) -> u32 {
    depth.saturating_sub(1).min(2)
}

pub fn cond_torg(depth: u32) -> impl Strategy<Value = Cond<TOrg>> {
    let mut leaves = eq_ops(TOrg::ID, int());
    leaves.extend(ord_ops(TOrg::ID, int()));
    leaves.extend(eq_ops(TOrg::NAME, text()));
    leaves.extend(text_ops(TOrg::NAME, text()));
    combine(Union::new(leaves).boxed(), depth)
}

pub fn cond_tuser(depth: u32) -> impl Strategy<Value = Cond<TUser>> {
    let mut leaves = eq_ops(TUser::ID, int());
    leaves.extend(ord_ops(TUser::ID, int()));
    leaves.extend(eq_ops(TUser::EMAIL, text()));
    leaves.extend(text_ops(TUser::EMAIL, text()));
    combine(Union::new(leaves).boxed(), depth)
}

/// Conditions on `TTag`, including the nested nullable to-one `author`
/// (`then`, `is_null`, `is_not_null`). `TUser` has no relations, so the
/// nesting stops there.
pub fn cond_ttag(depth: u32) -> impl Strategy<Value = Cond<TTag>> {
    let mut scalars = eq_ops(TTag::ID, int());
    scalars.extend(ord_ops(TTag::ID, int()));
    scalars.extend(eq_ops(TTag::NAME, text()));
    scalars.extend(text_ops(TTag::NAME, text()));
    scalars.extend(null_ops(TTag::NAME));
    let relations: Leaves<TTag> = vec![
        cond_tuser(inner_depth(depth))
            .prop_map(|c| TTag::AUTHOR.then(c))
            .boxed(),
        Just(TTag::AUTHOR.is_null()).boxed(),
        Just(TTag::AUTHOR.is_not_null()).boxed(),
    ];
    let leaf = Union::new_weighted(vec![
        (2, Union::new(scalars).boxed()),
        (1, Union::new(relations).boxed()),
    ]);
    combine(leaf.boxed(), depth)
}

/// Conditions on `TPost`: every operator each field type allows (`Uuid`:
/// equality and lists; `Date`: those, ordering and null tests), the to-one
/// relations (`then`, and `is_null`/`is_not_null` on the nullable one), and
/// the to-many quantifiers, combined up to `depth` levels.
pub fn cond_tpost(depth: u32) -> impl Strategy<Value = Cond<TPost>> {
    let mut scalars = Vec::new();
    for f in [TPost::ID, TPost::AUTHOR_ID] {
        scalars.extend(eq_ops(f, int()));
        scalars.extend(ord_ops(f, int()));
    }
    scalars.extend(eq_ops(TPost::REVIEWER_ID, int()));
    scalars.extend(ord_ops(TPost::REVIEWER_ID, int()));
    scalars.extend(null_ops(TPost::REVIEWER_ID));
    scalars.extend(eq_ops(TPost::TITLE, title()));
    scalars.extend(text_ops(TPost::TITLE, title()));
    scalars.extend(eq_ops(TPost::STATUS, status()));
    scalars.extend(eq_ops(TPost::SCORE, score()));
    scalars.extend(ord_ops(TPost::SCORE, score()));
    scalars.extend(eq_ops(TPost::OWNER, uuid()));
    scalars.extend(eq_ops(TPost::DUE, date()));
    scalars.extend(ord_ops(TPost::DUE, date()));
    scalars.extend(null_ops(TPost::DUE));

    let d = inner_depth(depth);
    let relations: Leaves<TPost> = vec![
        cond_torg(d).prop_map(|c| TPost::ORG.then(c)).boxed(),
        cond_tuser(d).prop_map(|c| TPost::REVIEWER.then(c)).boxed(),
        Just(TPost::REVIEWER.is_null()).boxed(),
        Just(TPost::REVIEWER.is_not_null()).boxed(),
        cond_ttag(d).prop_map(|c| TPost::TAGS.some(c)).boxed(),
        cond_ttag(d).prop_map(|c| TPost::TAGS.every(c)).boxed(),
        cond_ttag(d).prop_map(|c| TPost::TAGS.none(c)).boxed(),
    ];
    let leaf = Union::new_weighted(vec![
        (3, Union::new(scalars).boxed()),
        (2, Union::new(relations).boxed()),
    ]);
    combine(leaf.boxed(), depth)
}

/// One `can`/`cannot` group on `TPost`.
#[derive(Clone, Debug)]
pub struct RuleSpec {
    pub inverted: bool,
    pub actions: Vec<Action>,
    pub cond: Option<Cond<TPost>>,
    pub fields: Option<Vec<FieldRef<TPost>>>,
    pub reason: Option<String>,
}

impl RuleSpec {
    /// Whether the group's rules cover checks of `a` (directly or via `Manage`).
    pub fn covers(&self, a: Action) -> bool {
        self.actions.contains(&a) || self.actions.contains(&Action::Manage)
    }
}

/// Every `TPost` field, relations included.
pub fn tpost_fields() -> [FieldRef<TPost>; 11] {
    [
        TPost::ID.into(),
        TPost::AUTHOR_ID.into(),
        TPost::REVIEWER_ID.into(),
        TPost::TITLE.into(),
        TPost::STATUS.into(),
        TPost::SCORE.into(),
        TPost::ORG.into(),
        TPost::REVIEWER.into(),
        TPost::TAGS.into(),
        TPost::OWNER.into(),
        TPost::DUE.into(),
    ]
}

fn action() -> impl Strategy<Value = Action> {
    prop_oneof![
        4 => Just(Action::Read),
        1 => Just(Action::Create),
        1 => Just(Action::Update),
        1 => Just(Action::Delete),
        2 => Just(Action::Manage),
    ]
}

pub fn rule_spec() -> impl Strategy<Value = RuleSpec> {
    (
        any::<bool>(),
        vec(action(), 1..3),
        prop::option::weighted(0.8, cond_tpost(3)),
        prop::option::weighted(
            0.35,
            vec(prop::sample::select(tpost_fields().to_vec()), 1..4),
        ),
        prop::option::weighted(0.3, text()),
    )
        .prop_map(|(inverted, actions, cond, fields, reason)| RuleSpec {
            inverted,
            actions,
            cond,
            fields,
            reason,
        })
}

/// One to six rule groups.
pub fn rules() -> impl Strategy<Value = Vec<RuleSpec>> {
    vec(rule_spec(), 1..=6)
}

fn open(b: AbilityBuilder<Action, TSubject>, s: &RuleSpec) -> GroupBuilder<Action, TSubject> {
    if s.inverted {
        b.cannot(s.actions.clone(), TSubject::TPost)
    } else {
        b.can(s.actions.clone(), TSubject::TPost)
    }
}

fn next(g: GroupBuilder<Action, TSubject>, s: &RuleSpec) -> GroupBuilder<Action, TSubject> {
    if s.inverted {
        g.cannot(s.actions.clone(), TSubject::TPost)
    } else {
        g.can(s.actions.clone(), TSubject::TPost)
    }
}

fn modify(mut g: GroupBuilder<Action, TSubject>, s: &RuleSpec) -> GroupBuilder<Action, TSubject> {
    if let Some(c) = &s.cond {
        g = g.when(c.clone());
    }
    if let Some(fs) = &s.fields {
        g = g.fields(fs.iter().copied());
    }
    if let Some(r) = &s.reason {
        g = g.because(r.clone());
    }
    g
}

/// Builds the groups in order. Generated groups are always valid.
pub fn build(specs: &[RuleSpec]) -> Ability<Action, TSubject> {
    let Some((first, rest)) = specs.split_first() else {
        return Ability::builder().build().expect("empty ability builds");
    };
    let mut g = modify(open(Ability::builder(), first), first);
    for s in rest {
        g = modify(next(g, s), s);
    }
    g.build().expect("generated rules build")
}

/// Field names of `names` that are not loaded: none when `fully_loaded`,
/// otherwise usually none and sometimes one or two.
fn unloaded(names: &'static [&'static str], fully_loaded: bool) -> BoxedStrategy<Loaded> {
    if fully_loaded {
        Just(Loaded::default()).boxed()
    } else {
        prop_oneof![
            2 => Just(Loaded::default()),
            1 => prop::sample::subsequence(names, 1..=2).prop_map(Loaded),
        ]
        .boxed()
    }
}

/// A relation slot: always loaded when `fully_loaded`, otherwise sometimes not.
fn lazy<S: Clone + Debug + 'static>(
    s: impl Strategy<Value = S> + 'static,
    fully_loaded: bool,
) -> BoxedStrategy<Lazy<S>> {
    if fully_loaded {
        s.prop_map(Lazy::Loaded).boxed()
    } else {
        prop_oneof![
            3 => s.prop_map(Lazy::Loaded),
            1 => Just(Lazy::NotLoaded),
        ]
        .boxed()
    }
}

pub fn torg(fully_loaded: bool) -> impl Strategy<Value = TOrg> {
    (unloaded(&["id", "name"], fully_loaded), int(), text()).prop_map(|(loaded, id, name)| TOrg {
        loaded,
        id,
        name,
    })
}

pub fn tuser(fully_loaded: bool) -> impl Strategy<Value = TUser> {
    (unloaded(&["id", "email"], fully_loaded), int(), text())
        .prop_map(|(loaded, id, email)| TUser { loaded, id, email })
}

/// A tag whose author is absent, present, or (unless `fully_loaded`)
/// sometimes `NotLoaded`.
pub fn ttag(fully_loaded: bool) -> impl Strategy<Value = TTag> {
    (
        unloaded(&["id", "name"], fully_loaded),
        int(),
        prop::option::of(text()),
        lazy(prop::option::of(tuser(fully_loaded)), fully_loaded),
    )
        .prop_map(|(loaded, id, name, author)| TTag {
            loaded,
            id,
            name,
            author,
        })
}

/// A post with null scalars, absent reviewers, empty and non-empty tag lists
/// (tags with null names, and with absent or present authors), and absent or
/// present due dates. Unless `fully_loaded`, scalars of the post and of its
/// related rows may be unloaded and relation slots `NotLoaded`.
pub fn tpost(fully_loaded: bool) -> impl Strategy<Value = TPost> {
    (
        unloaded(
            &[
                "id",
                "author_id",
                "reviewer_id",
                "title",
                "status",
                "score",
                "owner",
                "due",
            ],
            fully_loaded,
        ),
        (int(), int(), prop::option::of(int())),
        (title(), status(), score()),
        lazy(torg(fully_loaded), fully_loaded),
        lazy(prop::option::of(tuser(fully_loaded)), fully_loaded),
        lazy(vec(ttag(fully_loaded), 0..3), fully_loaded),
        (uuid(), prop::option::of(date())),
    )
        .prop_map(
            |(
                loaded,
                (id, author_id, reviewer_id),
                (title, status, score),
                org,
                reviewer,
                tags,
                (owner, due),
            )| {
                TPost {
                    loaded,
                    id,
                    author_id,
                    reviewer_id,
                    title,
                    status,
                    score,
                    org,
                    reviewer,
                    tags,
                    owner,
                    due,
                }
            },
        )
}
