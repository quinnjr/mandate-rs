//! Error types.

/// Why a condition could not be evaluated against a resource.
///
/// Evaluation fails closed: neither case is ever coerced to `false`, since
/// that could make a `cannot` rule silently stop applying.
#[derive(Clone, Debug, PartialEq, thiserror::Error)]
pub enum EvalError {
    /// A field or relation the condition reads is not loaded.
    #[error("`{path}` is not loaded")]
    NotLoaded {
        /// Dotted path from the checked resource, e.g. `org.name`.
        path: String,
    },
    /// The resource's schema is not the schema the condition was built for.
    #[error("resource schema `{found}` does not match subject schema `{expected}`")]
    SchemaMismatch {
        /// Name of the schema the condition was built for.
        expected: &'static str,
        /// Name of the resource's schema.
        found: &'static str,
    },
}

/// Why a rule set could not be built.
#[derive(Clone, Debug, PartialEq, thiserror::Error)]
pub enum BuildError {
    /// A condition was built for a different resource than the rule's subject.
    #[error("condition on `{condition_schema}` does not match subject `{subject}`")]
    SubjectMismatch {
        /// Name of the rule's subject.
        subject: &'static str,
        /// Name of the schema the condition was built for.
        condition_schema: &'static str,
    },
    /// Conditions need exactly one subject bound to a resource.
    #[error(
        "conditions are not allowed on subject `{subject}` (needs a single resource-bound subject)"
    )]
    ConditionsNotAllowed {
        /// Name of the offending subject.
        subject: &'static str,
    },
    /// A field belongs to a different resource than the rule's subjects.
    #[error("field of `{field_schema}` is not a field of subject `{subject}`")]
    ForeignField {
        /// Name of the offending subject.
        subject: &'static str,
        /// Name of the schema the field belongs to.
        field_schema: &'static str,
    },
    /// An actions, subjects, or fields list was empty.
    #[error("empty list of {what}")]
    Empty {
        /// Which list: `"actions"`, `"subjects"`, or `"fields"`.
        what: &'static str,
    },
    /// A condition holds a value that can never be compared (non-finite float).
    #[error("invalid value: {reason}")]
    InvalidValue {
        /// What is wrong with the value.
        reason: String,
    },
}
