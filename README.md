# mandate-rs

`mandate-rs` (library name `mandate`) is a CASL-style authorization library for
Rust. One rule set, held as plain data, drives three things that always agree:
instance checks (`can`, `check`), database filters (`access`, `field_plan`), and
field masks (`permitted_fields`, `projection`).

- Rules are written in code with a typed builder: a typo in an action, subject,
  field or enum variant, or a value of the wrong type, is a compile error.
- Rules can also be stored as JSON templates, compiled once and bound per
  request with a context (`${user.id}`). Stored rules that do not match the
  schema are rejected with a precise error, never silently ignored.
- Actions and subjects are your own enums, using the `Action` and `Subject`
  derives; resources use the `Resource` derive.
- The core is independent of any framework or ORM. Evaluation is Rust-only for
  now, and rules stay serializable.

## Quickstart

```rust
use mandate::{Ability, Access, Action, IntoValue, Resource, Subject};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, IntoValue)]
enum Status { Draft, Published }

#[derive(Clone, Debug, Resource)]
struct Org { id: i64 }

#[derive(Clone, Debug, Resource)]
struct Post {
    id: i64,
    author_id: i64,
    title: String,
    body: String,
    status: Status,
    #[resource(relation)]
    org: Org,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Action)]
enum Act { Read, Update, Delete, #[action(manage)] Manage }

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Subject)]
enum Sub {
    #[subject(resource = Post)]
    Post,
    Dashboard,
    #[subject(all)]
    All,
}

struct User { id: i64, org_id: i64 }

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let user = User { id: 7, org_id: 3 };

    let ability = Ability::<Act, Sub>::builder()
        .can(Act::Read, Sub::Post)
            .when(Post::STATUS.eq(Status::Published))
        .can([Act::Read, Act::Update], Sub::Post)
            .when(Post::AUTHOR_ID.eq(user.id))
            .fields([Post::TITLE.into(), Post::BODY.into()])
        .can(Act::Delete, Sub::Post)
            .when(Post::AUTHOR_ID.eq(user.id))
        .can(Act::Read, Sub::Post)
            .when(Post::ORG.then(Org::ID.eq(user.org_id)))
        .cannot(Act::Delete, Sub::Post)
            .when(Post::STATUS.eq(Status::Published))
            .because("Published posts cannot be deleted")
        .can(Act::Read, Sub::Dashboard)
        .build()?;

    let draft = Post {
        id: 1,
        author_id: 7,
        title: "Hello".into(),
        body: "World".into(),
        status: Status::Draft,
        org: Org { id: 3 },
    };

    // Instance checks: the author may update, but only title and body.
    assert!(ability.can(Act::Update, &draft));
    assert!(ability.can_field(Act::Update, &draft, Post::TITLE));
    assert!(!ability.can_field(Act::Update, &draft, Post::ID));
    assert!(ability.can_type(Act::Read, Sub::Dashboard));

    // The author may delete the draft, but the later `cannot` rule denies
    // deleting it once it is published, and says why.
    assert!(ability.can(Act::Delete, &draft));
    let published = Post { status: Status::Published, ..draft.clone() };
    let denied = ability.check(Act::Delete, &published).unwrap_err();
    assert_eq!(
        denied.to_string(),
        "cannot delete Post: Published posts cannot be deleted"
    );

    // The same rules as a database filter: rows the user may read.
    let Access::Filter(plan) = ability.access::<Post>(Act::Read)? else {
        panic!("a conditional rule yields a filter");
    };
    assert!(plan.eval(&draft)?);
    Ok(())
}
```

## Stored templates

Rules loaded from storage go through a three-step flow:

```text
RuleTemplate (serde)  --compile once-->  Templates<A, S>  --bind(&Context) per request-->  Bound<A, S>  --extend-->  Ability
```

`Templates::compile` validates action, subject and field names, operators,
literal kinds and enum variants, placeholder slots and roots. `Templates::bind`
substitutes the placeholders of a per-request `Context` and checks their kinds.
Bound rules are mixed with code-defined rules through `AbilityBuilder::extend`,
and `build()` folds everything. Bound rules are never written back as templates.

```rust
use mandate::{Ability, Action, Context, Resource, RuleTemplate, Subject, Templates};

#[derive(Clone, Debug, Resource)]
struct Post { id: i64, author_id: i64 }

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Action)]
enum Act { Read, Update, #[action(manage)] Manage }

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Subject)]
enum Sub { #[subject(resource = Post)] Post, #[subject(all)] All }

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let stored = r#"[{
        "action": "update",
        "subject": "Post",
        "conditions": { "author_id": "${user.id}" },
        "reason": "Authors edit their own posts"
    }]"#;

    // startup, or whenever the stored rules change
    let raw: Vec<RuleTemplate> = serde_json::from_str(stored)?;
    let templates = Templates::<Act, Sub>::compile(&raw, &["user"])?;

    // per request
    let ctx = Context::new().with("user", &serde_json::json!({ "id": 7 }))?;
    let bound = templates.bind(&ctx)?;
    for unresolved in bound.unresolved() {
        eprintln!("unresolved placeholder: {unresolved:?}");
    }
    let ability = Ability::builder()
        .extend(bound)
        .can(Act::Read, Sub::Post)
        .build()?;

    let post = Post { id: 1, author_id: 7 };
    assert!(ability.can(Act::Update, &post));
    assert!(!ability.can(Act::Update, &Post { author_id: 8, ..post }));
    Ok(())
}
```

A placeholder that cannot be resolved binds under the most restrictive
interpretation: a leaf of a `can` rule becomes `false` and a leaf of a `cannot`
rule becomes `true` (unless negated by `$not` or `$none`). A placeholder marked
optional (`${...?}`) drops its rule instead. `Bound::unresolved` reports what
happened to each one (`UnresolvedOutcome`).

As in CASL, `"conditions": null` means the same as leaving `conditions` out (an
unconditional rule), and `"fields": null` the same as leaving `fields` out
(every field).

### Storing templates

Store templates in a `json` or `text` column (or a file) and deserialize them
with `serde_json::from_str`, as above. The template deserializer rejects
duplicate keys and keeps the stored key order, so the children of an implicit
`And` keep their order. Going through `serde_json::Value` first, or storing
templates in a Postgres `jsonb` column, silently drops duplicate keys and
reorders keys. Because `And` and `Or` stop at the first child that decides
them, a changed order can change whether a partially loaded resource yields an
answer or `EvalError::NotLoaded` (never two different answers): `can*` stays
fail-closed either way, but `permitted_fields` may fail where it otherwise
would not. Validate templates with `Templates::compile` when they are written.

## Fail closed and load state

A condition that cannot be evaluated never evaluates to `false` and never
silently stops a `cannot` rule from applying. Reading a scalar that reports
`NotLoaded`, or a relation whose slot reports `NotLoaded`, yields
`EvalError::NotLoaded`; a resource whose schema is not its subject's yields
`EvalError::SchemaMismatch`. `can*` return `false` on any such error, and
`check*` return `CheckError::Unresolvable`.

To know which data is loaded, resources use load-tracking types: a
`#[resource(load_state = field)]` tracker implementing `LoadState` for scalars
(asked by schema name, after any `rename`; it should answer `false` for names
it does not know), and `RelationSlot` implementations that can report
`NotLoaded` for relations. `Ability::projection` tells you what to fetch so the
checks for an action can run without meeting unfetched data.

## Limits

These keep evaluation, folding and planning within a small, fixed stack, so a
rule set built from data cannot overflow it:

- A schema has at most 128 fields (`MAX_FIELDS`).
- `build()` rejects a rule condition nested more than 64 levels deep
  (`BuildError::TooDeep`). Every `And`, `Or`, `Not` and relation quantifier is
  a level; `Cond::and` and `Cond::or` extend one group, so chains of any
  length do not nest.
- `build()` rejects rules for one (action, subject) pair that switch between
  `can` and `cannot` more than 256 times (`BuildError::TooManyAlternations`);
  rules from `manage` or `all` count for every pair they cover.
- Stored templates nest condition objects at most 32 deep
  (`LoadErrorKind::Malformed`). An object with several keys is an `And` of
  them, so a template near that limit may still exceed the 64-level limit and
  fail at `build()`.

## Caveats

- **Relation integrity.** Rows that violate their declared relation shape (a
  non-nullable to-one relation with no related row, such as a dangling or
  soft-deleted target, or a to-one relation with several related rows) cannot
  be represented faithfully in memory. For them only the database semantics of
  a `Plan` (`EXISTS`-based) apply: `can` may disagree with the rows the
  database returns for `access`.
- **Non-finite floats.** Non-finite operands (`NaN`, `±∞`) are rejected at
  `build()`. Non-finite instance values are compared as Rust compares `f64`
  (`NaN` equals nothing and is not ordered against anything), which may
  differ from the database (Postgres orders `NaN` above every number), so
  `can` and a database filter may disagree on such rows.

## Differences from CASL

1. Exact query formula (`Ability::access`) instead of CASL's `rulesToQuery`: a
   row matching `can A; cannot B; can C` through B and C is allowed by both `can`
   and `access`.
2. No closures or functions in conditions; the condition AST is closed.
3. To-many relations use `$some`/`$every`/`$none` instead of `$elemMatch`/`$all`.
4. No `$regex`, `$size`, `$all`, and no ordering on strings.
5. Subject detection is static (`SubjectResource::SUBJECT`), not
   `__caslSubjectType__`.
6. Unloaded relations and unfetched fields fail closed instead of evaluating
   against `undefined`.
7. Null is never a comparison operand; unresolved placeholders bind most
   restrictively (or drop the rule if marked optional).
8. Stored rules are templates compiled once and bound per request, with named
   context roots.
9. Field permissions do not cascade through relations.

## Integrations

The core has no ORM or web-framework dependency. Adapters are planned as
separate crates, not yet published: `mandate-prax` (lowering `Plan` to Prax
queries and `Projection` to Prax `select`) and `mandate-armature` (route
guards for the Armature framework).

## Cargo features

- `derive` (default): the `Action`, `Subject`, `Resource` and `IntoValue` derives.
- `uuid`: `uuid::Uuid` as a field type.
- `chrono`: `chrono::DateTime<Utc>` and `NaiveDate` as field types.

The minimum supported Rust version is 1.94.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or <https://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or <https://opensource.org/licenses/MIT>)

at your option.

### Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in this crate by you, as defined in the Apache-2.0 license, shall
be dual licensed as above, without any additional terms or conditions.
