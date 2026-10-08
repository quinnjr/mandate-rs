//! `mandate-rs`: a CASL-style authorization library.
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
pub use error::{BuildError, CheckError, EvalError, Forbidden, LoadError, LoadErrorKind};
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
pub use template::{OneOrMany, RuleTemplate, TemplateValue, Templates};
pub use traits::{Action, DynResource, LoadState, Resource, Subject, SubjectResource};
#[cfg(feature = "chrono")]
pub use value::truncate_micros;
pub use value::{Value, ValueRef};

#[cfg(all(test, feature = "derive", feature = "chrono", feature = "uuid"))]
#[allow(dead_code, unused_imports)]
pub(crate) mod test_fixture {
    include!("../tests/common/fixture.rs");
}
