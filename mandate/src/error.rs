//! Error types.

use std::borrow::Cow;

use crate::{Action, Subject};

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

/// An action that the rules do not permit.
#[derive(Clone, Debug, PartialEq)]
pub struct Forbidden<A, S> {
    /// The denied action.
    pub action: A,
    /// The subject it was attempted on.
    pub subject: S,
    /// The field, for field-level checks.
    pub field: Option<&'static str>,
    /// The reason from the deciding `cannot` rule, if it has one.
    pub reason: Option<Cow<'static, str>>,
}

impl<A: Action, S: Subject> core::fmt::Display for Forbidden<A, S> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "cannot {} {}", self.action.name(), self.subject.name())?;
        if let Some(field) = self.field {
            write!(f, ".{field}")?;
        }
        if let Some(reason) = &self.reason {
            write!(f, ": {reason}")?;
        }
        Ok(())
    }
}

impl<A: Action, S: Subject> std::error::Error for Forbidden<A, S> {}

/// Why a `check*` call did not succeed.
#[derive(Clone, Debug, PartialEq)]
pub enum CheckError<A, S> {
    /// The rules deny the action.
    Forbidden(Forbidden<A, S>),
    /// The rules could not be evaluated (unloaded data or schema mismatch).
    Unresolvable(EvalError),
}

impl<A: Action, S: Subject> core::fmt::Display for CheckError<A, S> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            CheckError::Forbidden(e) => e.fmt(f),
            CheckError::Unresolvable(e) => e.fmt(f),
        }
    }
}

impl<A: Action, S: Subject> std::error::Error for CheckError<A, S> {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            CheckError::Forbidden(e) => Some(e),
            CheckError::Unresolvable(e) => Some(e),
        }
    }
}

impl<A, S> From<Forbidden<A, S>> for CheckError<A, S> {
    fn from(e: Forbidden<A, S>) -> Self {
        CheckError::Forbidden(e)
    }
}
