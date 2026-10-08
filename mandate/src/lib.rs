//! `mandate-rs` (library name `mandate`) is a CASL-style authorization library for
//! Rust. One rule set, held as plain data, drives three things that always agree:
//! instance checks (`can`, `check`), database filters (`access`, `field_plan`), and
//! field masks (`permitted_fields`, `projection`).
//!
//! - Rules are written in code with a typed builder: a typo in an action, subject,
//!   field or enum variant, or a value of the wrong type, is a compile error.
//! - Rules can also be stored as JSON templates, compiled once and bound per
//!   request with a context (`${user.id}`). Stored rules that do not match the
//!   schema are rejected with a precise error, never silently ignored.
//! - Actions and subjects are your own enums, using the `Action` and `Subject`
//!   derives; resources use the `Resource` derive.
//! - The core is independent of any framework or ORM. Evaluation is Rust-only for
//!   now, and rules stay serializable.
//!
//! ## Quickstart
//!
//! ```rust
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! use mandate::{Ability, Access, Action, IntoValue, Resource, Subject};
//!
//! #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, IntoValue)]
//! enum Status { Draft, Published }
//!
//! #[derive(Clone, Debug, Resource)]
//! struct Org { id: i64 }
//!
//! #[derive(Clone, Debug, Resource)]
//! struct Post {
//!     id: i64,
//!     author_id: i64,
//!     title: String,
//!     body: String,
//!     status: Status,
//!     #[resource(relation)]
//!     org: Org,
//! }
//!
//! #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Action)]
//! enum Act { Read, Update, Delete, #[action(manage)] Manage }
//!
//! #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Subject)]
//! enum Sub {
//!     #[subject(resource = Post)]
//!     Post,
//!     Dashboard,
//!     #[subject(all)]
//!     All,
//! }
//!
//! struct User { id: i64, org_id: i64 }
//! let user = User { id: 7, org_id: 3 };
//!
//! let ability = Ability::<Act, Sub>::builder()
//!     .can(Act::Read, Sub::Post)
//!         .when(Post::STATUS.eq(Status::Published))
//!     .can([Act::Read, Act::Update], Sub::Post)
//!         .when(Post::AUTHOR_ID.eq(user.id))
//!         .fields([Post::TITLE.into(), Post::BODY.into()])
//!     .can(Act::Read, Sub::Post)
//!         .when(Post::ORG.then(Org::ID.eq(user.org_id)))
//!     .cannot(Act::Delete, Sub::Post)
//!         .when(Post::STATUS.eq(Status::Published))
//!         .because("Published posts cannot be deleted")
//!     .can(Act::Read, Sub::Dashboard)
//!     .build()?;
//!
//! let post = Post {
//!     id: 1,
//!     author_id: 7,
//!     title: "Hello".into(),
//!     body: "World".into(),
//!     status: Status::Draft,
//!     org: Org { id: 3 },
//! };
//!
//! // Instance checks: the author may update, but only title and body.
//! assert!(ability.can(Act::Update, &post));
//! assert!(ability.can_field(Act::Update, &post, Post::TITLE));
//! assert!(!ability.can_field(Act::Update, &post, Post::ID));
//! assert!(ability.check(Act::Delete, &post).is_err());
//! assert!(ability.can_type(Act::Read, Sub::Dashboard));
//!
//! // The same rules as a database filter: rows the user may read.
//! let Access::Filter(plan) = ability.access::<Post>(Act::Read)? else {
//!     panic!("a conditional rule yields a filter");
//! };
//! assert!(plan.eval(&post)?);
//! # Ok(())
//! # }
//! ```
//!
//! ## Stored templates
//!
//! Rules loaded from storage go through a three-step flow:
//!
//! ```text
//! RuleTemplate (serde)  --compile once-->  Templates<A, S>  --bind(&Context) per request-->  Bound<A, S>  --extend-->  Ability
//! ```
//!
//! `Templates::compile` validates action, subject and field names, operators,
//! literal kinds and enum variants, placeholder slots and roots. `Templates::bind`
//! substitutes the placeholders of a per-request `Context` and checks their kinds.
//! Bound rules are mixed with code-defined rules through `AbilityBuilder::extend`,
//! and `build()` folds everything. Bound rules are never written back as templates.
//!
//! ```rust
//! # use mandate::{Ability, Action, Context, Resource, RuleTemplate, Subject, Templates};
//! # #[derive(Clone, Debug, Resource)]
//! # struct Post { id: i64, author_id: i64 }
//! # #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Action)]
//! # enum Act { Read, Update, #[action(manage)] Manage }
//! # #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Subject)]
//! # enum Sub { #[subject(resource = Post)] Post, #[subject(all)] All }
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let stored = r#"[{
//!     "action": "update",
//!     "subject": "Post",
//!     "conditions": { "author_id": "${user.id}" },
//!     "reason": "Authors edit their own posts"
//! }]"#;
//!
//! // startup, or whenever the stored rules change
//! let raw: Vec<RuleTemplate> = serde_json::from_str(stored)?;
//! let templates = Templates::<Act, Sub>::compile(&raw, &["user"])?;
//!
//! // per request
//! let ctx = Context::new().with("user", &serde_json::json!({ "id": 7 }))?;
//! let bound = templates.bind(&ctx)?;
//! for unresolved in bound.unresolved() {
//!     eprintln!("unresolved placeholder: {unresolved:?}");
//! }
//! let ability = Ability::builder()
//!     .extend(bound)
//!     .can(Act::Read, Sub::Post)
//!     .build()?;
//!
//! let post = Post { id: 1, author_id: 7 };
//! assert!(ability.can(Act::Update, &post));
//! assert!(!ability.can(Act::Update, &Post { author_id: 8, ..post }));
//! # Ok(())
//! # }
//! ```
//!
//! A placeholder that cannot be resolved binds under the most restrictive
//! interpretation: a leaf of a `can` rule becomes `false` and a leaf of a `cannot`
//! rule becomes `true` (unless negated by `$not` or `$none`). A placeholder marked
//! optional (`${...?}`) drops its rule instead. `Bound::unresolved` reports what
//! happened to each one (`UnresolvedOutcome`).
//!
//! ## Fail closed and load state
//!
//! A condition that cannot be evaluated never evaluates to `false` and never
//! silently stops a `cannot` rule from applying. Reading a scalar that reports
//! `NotLoaded`, or a relation whose slot reports `NotLoaded`, yields
//! `EvalError::NotLoaded`; a resource whose schema is not its subject's yields
//! `EvalError::SchemaMismatch`. `can*` return `false` on any such error, and
//! `check*` return `CheckError::Unresolvable`.
//!
//! To know which data is loaded, resources use load-tracking types: a
//! `#[resource(load_state = field)]` tracker implementing `LoadState` for scalars,
//! and `RelationSlot` implementations that can report `NotLoaded` for relations.
//! `Ability::projection` tells you what to fetch so the checks for an action can
//! run without meeting unfetched data.
//!
//! ## Differences from CASL
//!
//! 1. Exact query formula (`Ability::access`) instead of CASL's `rulesToQuery`: a
//!    row matching `can A; cannot B; can C` through B and C is allowed by both `can`
//!    and `access`.
//! 2. No closures or functions in conditions; the condition AST is closed.
//! 3. To-many relations use `$some`/`$every`/`$none` instead of `$elemMatch`/`$all`.
//! 4. No `$regex`, `$size`, `$all`, and no ordering on strings.
//! 5. Subject detection is static (`SubjectResource::SUBJECT`), not
//!    `__caslSubjectType__`.
//! 6. Unloaded relations and unfetched fields fail closed instead of evaluating
//!    against `undefined`.
//! 7. Null is never a comparison operand; unresolved placeholders bind most
//!    restrictively (or drop the rule if marked optional).
//! 8. Stored rules are templates compiled once and bound per request, with named
//!    context roots.
//! 9. Field permissions do not cascade through relations.
//!
//! ## Integrations
//!
//! The core has no ORM or web-framework dependency. Adapters are planned as
//! separate crates, not yet published: `mandate-prax` (lowering `Plan` to Prax
//! queries and `Projection` to Prax `select`) and `mandate-armature` (route
//! guards for the Armature framework).
//!
//! ## Cargo features
//!
//! - `derive` (default): the `Action`, `Subject`, `Resource` and `IntoValue` derives.
//! - `uuid`: `uuid::Uuid` as a field type.
//! - `chrono`: `chrono::DateTime<Utc>` and `NaiveDate` as field types.
//!
//! The minimum supported Rust version is 1.85.
//!
#![deny(missing_docs)]
#![forbid(unsafe_code)]

extern crate self as mandate;

pub mod ability;
pub mod builder;
pub mod condition;
pub mod error;
pub mod field;
pub mod field_plan;
pub mod fieldset;
pub mod plan;
pub mod projection;
pub mod relation;
pub mod rule;
pub mod scalar;
pub mod schema;
pub mod template;
pub mod traits;
pub mod value;

pub use ability::Ability;
pub use builder::{AbilityBuilder, GroupBuilder, IntoActions, IntoSubjects};
pub use condition::{CmpOp, Cond, Condition, Quant, StrOp};
pub use error::{
    BindError, BindErrorKind, BuildError, CheckError, EvalError, Forbidden, LoadError,
    LoadErrorKind, Unresolved, UnresolvedOutcome,
};
pub use field::{Field, FieldRef, Opaque, Rel};
pub use field_plan::{FieldPlan, FieldRule};
pub use fieldset::{FieldMask, FieldSet};
#[cfg(feature = "derive")]
pub use mandate_derive::{Action, IntoValue, Resource, Subject};
pub use plan::{Access, Plan};
pub use projection::{Projection, RelationProjection};
pub use relation::{Cardinality, DynMany, RelationRef, RelationSlot, ResourcePtr, ToMany, ToOne};
pub use rule::Rule;
pub use scalar::{
    IntoValue, NonNull, Nullability, Nullable, Ordered, Scalar, ScalarValue, Textual,
};
pub use schema::{CardinalityKind, FieldDef, FieldIdx, FieldKind, Kind, MAX_FIELDS, Schema};
pub use template::{Bound, Context, OneOrMany, RuleTemplate, TemplateValue, Templates};
pub use traits::{Action, DynResource, LoadState, Resource, Subject, SubjectResource};
#[cfg(feature = "chrono")]
pub use value::truncate_micros;
pub use value::{Value, ValueRef};

#[cfg(all(test, feature = "derive", feature = "chrono", feature = "uuid"))]
#[allow(dead_code, unused_imports)]
pub(crate) mod test_fixture {
    include!("../tests/common/fixture.rs");
}
