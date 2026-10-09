// A resource that claims to be the fixture's `Post` subject but has its own
// schema: the guard against a hand-written `SubjectResource` whose resource
// is not its subject's (spec §9 `EvalError::SchemaMismatch`).

use mandate::{
    DynResource, FieldDef, FieldIdx, Kind, RelationRef, Resource, Schema, SubjectResource, ValueRef,
};

use super::fixture::Subject;

/// Bound to [`Subject::Post`], with the schema `Imposter { id: Int }`; every
/// value and relation is `NotLoaded`.
pub struct Imposter;

static IMPOSTER: Schema = Schema::new("Imposter", &[FieldDef::scalar("id", Kind::Int, false)]);

impl Resource for Imposter {
    fn schema() -> &'static Schema {
        &IMPOSTER
    }
    fn as_dyn(&self) -> &dyn DynResource {
        self
    }
}

impl DynResource for Imposter {
    fn resource_schema(&self) -> &'static Schema {
        &IMPOSTER
    }
    fn value(&self, _: FieldIdx) -> ValueRef<'_> {
        ValueRef::NotLoaded
    }
    fn relation(&self, _: FieldIdx) -> RelationRef<'_> {
        RelationRef::NotLoaded
    }
}

impl SubjectResource<Subject> for Imposter {
    const SUBJECT: Subject = Subject::Post;
}
