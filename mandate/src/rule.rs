//! A single authorization rule.

use std::borrow::Cow;

use crate::{Condition, FieldMask};

/// One bound rule: an (action, subject) pair with optional condition,
/// field restriction, and denial reason.
#[derive(Clone, Debug, PartialEq)]
pub struct Rule<A, S> {
    action: A,
    subject: S,
    inverted: bool,
    condition: Option<Condition>,
    fields: Option<FieldMask>,
    reason: Option<Cow<'static, str>>,
}

impl<A: Copy, S: Copy> Rule<A, S> {
    pub(crate) fn new(
        action: A,
        subject: S,
        inverted: bool,
        condition: Option<Condition>,
        fields: Option<FieldMask>,
        reason: Option<Cow<'static, str>>,
    ) -> Self {
        Self {
            action,
            subject,
            inverted,
            condition,
            fields,
            reason,
        }
    }

    /// The rule's action (possibly the manage wildcard).
    pub fn action(&self) -> A {
        self.action
    }

    /// The rule's subject (possibly the all wildcard).
    pub fn subject(&self) -> S {
        self.subject
    }

    /// Whether this is a `cannot` rule.
    pub fn inverted(&self) -> bool {
        self.inverted
    }

    /// The folded condition; `None` means unconditional.
    pub fn condition(&self) -> Option<&Condition> {
        self.condition.as_ref()
    }

    /// The fields the rule is restricted to; `None` means all fields.
    pub fn fields(&self) -> Option<FieldMask> {
        self.fields
    }

    /// The reason given for a `cannot` rule, if any.
    pub fn reason(&self) -> Option<&str> {
        self.reason.as_deref()
    }
}
