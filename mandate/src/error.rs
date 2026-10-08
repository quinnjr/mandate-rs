//! Error types.

use std::borrow::Cow;

use crate::{Action, Kind, Subject};

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

/// Why a list of [`RuleTemplate`](crate::RuleTemplate)s could not be compiled.
///
/// `Display` renders ``rule {rule_index} at `{path}`: {kind}``.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("rule {rule_index} at `{path}`: {kind}")]
pub struct LoadError {
    /// Index of the offending template in the list passed to `compile`.
    pub rule_index: usize,
    /// Where in the template the problem is: `action`, `subject`, `fields`
    /// or `conditions`, followed by dotted keys and `[i]` array indices, e.g.
    /// `conditions.$or[1].author_id` or `fields[2]`.
    pub path: String,
    /// What is wrong.
    pub kind: LoadErrorKind,
}

/// What is wrong with a rule template (see [`LoadError`]).
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum LoadErrorKind {
    /// No action has this name.
    #[error("unknown action `{0}`")]
    UnknownAction(String),
    /// No subject has this name.
    #[error("unknown subject `{0}`")]
    UnknownSubject(String),
    /// The schema in scope has no field with this name. Also used for a
    /// `fields` entry on a rule without a single resource-bound subject.
    #[error("unknown field `{0}`")]
    UnknownField(String),
    /// A `$`-key that is not an operator valid in its position.
    #[error("unknown operator `{0}`")]
    UnknownOperator(String),
    /// A key a relation object does not accept, e.g. a to-many key other
    /// than `$some`, `$every` or `$none`.
    #[error("unexpected key `{0}`")]
    UnknownKey(String),
    /// A placeholder names a context root that was not declared.
    #[error("unknown context root `{0}`")]
    UnknownRoot(String),
    /// An enum literal that is not one of the field's variants.
    #[error("unknown variant `{0}`")]
    UnknownVariant(String),
    /// A literal of the wrong JSON type for the field's kind.
    #[error("expected {expected:?}, found {found}")]
    TypeMismatch {
        /// The kind the field (or operator) requires.
        expected: Kind,
        /// The offending JSON value (`array`/`object` for compound values).
        found: String,
    },
    /// The operator is not defined on the field's kind (e.g. ordering on a
    /// string, or text operators on an enum).
    #[error("operator `{op}` is not allowed on {kind:?} fields")]
    OperatorNotAllowed {
        /// The operator.
        op: String,
        /// The field's kind.
        kind: Kind,
    },
    /// A null test on a non-nullable field or relation, or `null` as the
    /// operand of an operator other than `$eq`/`$ne`.
    #[error("null is not allowed here")]
    NullNotAllowed,
    /// `$isNull`/`$none` mixed with other keys in a to-one relation object, or
    /// several quantifiers in a to-many relation object.
    #[error("a relation null test or quantifier must be the only key of its object")]
    MixedRelationObject,
    /// Conditions need exactly one subject bound to a resource.
    #[error("conditions need a single resource-bound subject")]
    ConditionsNotAllowed,
    /// An empty `action`, `subject` or `fields` list (names the key).
    #[error("`{0}` must not be empty")]
    Empty(&'static str),
    /// A string that does not parse as the field's kind (UUID, RFC 3339
    /// date-time, `YYYY-MM-DD` date), or a kind whose feature is disabled.
    #[error("invalid value: {0}")]
    InvalidValue(String),
    /// A structural error: a malformed placeholder, a wrong JSON type where
    /// the grammar expects an object or array, a condition on an opaque
    /// field, or nesting deeper than 32 levels.
    #[error("malformed template: {0}")]
    Malformed(String),
}

/// Why [`Templates::bind`](crate::Templates::bind) failed: a context value
/// does not fit its placeholder. This indicates a context or schema bug, not
/// missing data (missing data is [`Unresolved`]).
///
/// `Display` renders ``rule {rule_index} at `{path}`: {kind}``.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("rule {rule_index} at `{path}`: {kind}")]
pub struct BindError {
    /// Index of the template containing the placeholder.
    pub rule_index: usize,
    /// Where the placeholder is in the template, as in [`LoadError::path`]
    /// (e.g. `conditions.org.id`); for an element of a list placeholder,
    /// followed by its `[i]` index in the context's array.
    pub path: String,
    /// What is wrong with the value.
    pub kind: BindErrorKind,
}

/// What is wrong with a context value (see [`BindError`]).
///
/// Values are checked as template literals are at compile time.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum BindErrorKind {
    /// A value of the wrong JSON type for the field's kind, including an
    /// array for a scalar placeholder or a non-array for a list placeholder
    /// (whose `expected` kind is that of its elements).
    #[error("expected {expected:?}, found {found}")]
    TypeMismatch {
        /// The kind the field requires.
        expected: Kind,
        /// The offending JSON value (`array`/`object` for compound values).
        found: String,
    },
    /// An enum value that is not one of the field's variants.
    #[error("unknown variant `{0}`")]
    UnknownVariant(String),
    /// A string that does not parse as the field's kind (UUID, RFC 3339
    /// date-time, `YYYY-MM-DD` date), a kind whose feature is disabled, or
    /// `null` inside a list (`"null in list"`).
    #[error("invalid value: {0}")]
    InvalidValue(String),
}

/// A placeholder that was missing from the context or `null` when binding,
/// and what binding did instead (spec §6.4). Reported for diagnostics; it is
/// not an error.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unresolved {
    /// Index of the template containing the placeholder.
    pub rule_index: usize,
    /// The placeholder's dotted path, root first (e.g. `user.manager_id`).
    pub placeholder: String,
    /// What binding did instead.
    pub outcome: UnresolvedOutcome,
}

/// What binding did about an [`Unresolved`] placeholder.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnresolvedOutcome {
    /// The placeholder is optional (`${…?}`): its rule was dropped.
    RuleDropped,
    /// Its leaf became `false`, the value that grants the least access
    /// (a leaf of a `can` rule, unless negated by `$not` or `$none`).
    LeafFalse,
    /// Its leaf became `true`, the value that grants the least access
    /// (a leaf of a `cannot` rule, unless negated by `$not` or `$none`).
    LeafTrue,
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
