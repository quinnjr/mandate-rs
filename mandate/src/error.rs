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
