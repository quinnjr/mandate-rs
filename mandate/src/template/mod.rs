//! Stored rule templates: the serde data model (database or config file
//! form), compiling them against the schemas, and binding them per request.

mod bind;
mod compile;
mod serialize;

pub use bind::{Bound, Context};
pub use compile::Templates;

use std::collections::HashSet;
use std::fmt;

use serde::de::{self, Deserializer, MapAccess, SeqAccess, Visitor};
use serde::ser::{SerializeMap, SerializeSeq, Serializer};
use serde::{Deserialize, Serialize};

/// A stored rule template, before compilation against a schema.
///
/// Deserialize templates with `serde_json::from_str` from `json` or `text`
/// storage: the condition document rejects duplicate keys and keeps key
/// order. Passing through `serde_json::Value` or a Postgres `jsonb` column
/// first loses both (see the crate docs, "Storing templates"). To build one
/// in code, use [`new`](Self::new) and the `with_*` methods.
///
/// As in CASL, `"conditions": null` means the same as no `conditions` (an
/// unconditional rule), and `"fields": null` the same as no `fields` (every
/// field). An empty `fields` list is an error.
///
/// # Optional placeholders in `cannot` rules
///
/// **An optional placeholder (`${…?}`) in an inverted rule fails open.**
/// When its value is missing, [`Templates::bind`] drops the whole rule, so
/// the deny no longer applies and the request gets whatever the other rules
/// grant. That is right only when the value may legitimately be absent, as
/// in "deny posts by the blocked author, if any": a user who blocked no one
/// has nothing to deny. The real risk is a value that is missing by mistake:
/// a mistyped placeholder path ([`Templates::compile`] checks only its
/// root) or a context that was not fully populated drops the deny just the
/// same. So keep a list of the optional denies you expect, and treat any
/// other
/// [`UnresolvedOutcome::RuleDropped`](crate::UnresolvedOutcome::RuleDropped)
/// report with [`inverted`](crate::Unresolved::inverted) set as
/// security-relevant: log or alert on it, or fail the request.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct RuleTemplate {
    /// Action name or names the rule applies to.
    pub action: OneOrMany,
    /// Subject name or names the rule applies to.
    pub subject: OneOrMany,
    /// Optional condition document; may contain placeholders. Absent or
    /// `null`: the rule is unconditional.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conditions: Option<TemplateValue>,
    /// Optional field restriction. Absent or `null`: every field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fields: Option<Vec<String>>,
    /// Whether the rule is a prohibition.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub inverted: bool,
    /// Optional human-readable reason.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

impl RuleTemplate {
    /// An unconditional `can` template for `action` on `subject`, covering
    /// every field and without a reason; the `with_*` methods set the rest.
    ///
    /// `action` and `subject` each take one name (`&str` or `String`) or
    /// several (an array or `Vec` of either).
    ///
    /// # Examples
    ///
    /// ```
    /// use mandate::{RuleTemplate, TemplateValue};
    ///
    /// let conditions: TemplateValue =
    ///     serde_json::from_str(r#"{"author_id": "${user.id}"}"#).unwrap();
    /// let built = RuleTemplate::new("update", "Post")
    ///     .with_conditions(conditions)
    ///     .with_fields(["title", "body"])
    ///     .with_reason("Authors edit their own posts");
    ///
    /// let stored: RuleTemplate = serde_json::from_str(
    ///     r#"{"action": "update", "subject": "Post",
    ///         "conditions": {"author_id": "${user.id}"},
    ///         "fields": ["title", "body"],
    ///         "reason": "Authors edit their own posts"}"#,
    /// )
    /// .unwrap();
    /// assert_eq!(built, stored);
    ///
    /// // A `cannot` rule for several actions.
    /// let deny = RuleTemplate::new(["update", "delete"], "Post").with_inverted(true);
    /// assert!(deny.inverted);
    /// ```
    pub fn new(action: impl Into<OneOrMany>, subject: impl Into<OneOrMany>) -> Self {
        Self {
            action: action.into(),
            subject: subject.into(),
            conditions: None,
            fields: None,
            inverted: false,
            reason: None,
        }
    }

    /// Sets the condition document, which may contain placeholders.
    pub fn with_conditions(mut self, conditions: TemplateValue) -> Self {
        self.conditions = Some(conditions);
        self
    }

    /// Restricts the rule to these fields, by schema name. An empty list is
    /// rejected by [`Templates::compile`].
    pub fn with_fields(mut self, fields: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.fields = Some(fields.into_iter().map(Into::into).collect());
        self
    }

    /// Sets whether the rule is a prohibition (a `cannot` rule).
    ///
    /// An unresolved optional placeholder (`${…?}`) drops an inverted rule,
    /// and with it the deny; see [`RuleTemplate`].
    pub fn with_inverted(mut self, inverted: bool) -> Self {
        self.inverted = inverted;
        self
    }

    /// Sets the human-readable reason.
    pub fn with_reason(mut self, reason: impl Into<String>) -> Self {
        self.reason = Some(reason.into());
        self
    }
}

/// A single string or a list of strings.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum OneOrMany {
    /// A single name.
    One(String),
    /// Several names.
    Many(Vec<String>),
}

impl From<&str> for OneOrMany {
    fn from(name: &str) -> Self {
        Self::One(name.to_owned())
    }
}

impl From<String> for OneOrMany {
    fn from(name: String) -> Self {
        Self::One(name)
    }
}

impl<T: Into<String>> From<Vec<T>> for OneOrMany {
    fn from(names: Vec<T>) -> Self {
        Self::Many(names.into_iter().map(Into::into).collect())
    }
}

impl<T: Into<String>, const N: usize> From<[T; N]> for OneOrMany {
    fn from(names: [T; N]) -> Self {
        Self::Many(names.into_iter().map(Into::into).collect())
    }
}

/// A JSON-like value that preserves object key order and rejects duplicate keys.
#[derive(Clone, Debug, PartialEq)]
pub enum TemplateValue {
    /// JSON `null`.
    Null,
    /// A boolean.
    Bool(bool),
    /// A finite number.
    Number(serde_json::Number),
    /// A string.
    String(String),
    /// An array.
    Array(Vec<TemplateValue>),
    /// An object, as key/value pairs in stored order.
    Object(Vec<(String, TemplateValue)>),
}

impl Serialize for TemplateValue {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Null => s.serialize_unit(),
            Self::Bool(b) => s.serialize_bool(*b),
            Self::Number(n) => n.serialize(s),
            Self::String(v) => s.serialize_str(v),
            Self::Array(items) => {
                let mut seq = s.serialize_seq(Some(items.len()))?;
                for item in items {
                    seq.serialize_element(item)?;
                }
                seq.end()
            }
            Self::Object(entries) => {
                let mut map = s.serialize_map(Some(entries.len()))?;
                for (k, v) in entries {
                    map.serialize_entry(k, v)?;
                }
                map.end()
            }
        }
    }
}

struct ValueVisitor;

impl<'de> Visitor<'de> for ValueVisitor {
    type Value = TemplateValue;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a JSON value")
    }

    fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
        Ok(TemplateValue::Null)
    }

    fn visit_none<E: de::Error>(self) -> Result<Self::Value, E> {
        Ok(TemplateValue::Null)
    }

    fn visit_some<D: Deserializer<'de>>(self, d: D) -> Result<Self::Value, D::Error> {
        d.deserialize_any(self)
    }

    fn visit_bool<E: de::Error>(self, v: bool) -> Result<Self::Value, E> {
        Ok(TemplateValue::Bool(v))
    }

    fn visit_i64<E: de::Error>(self, v: i64) -> Result<Self::Value, E> {
        Ok(TemplateValue::Number(v.into()))
    }

    fn visit_u64<E: de::Error>(self, v: u64) -> Result<Self::Value, E> {
        Ok(TemplateValue::Number(v.into()))
    }

    fn visit_f64<E: de::Error>(self, v: f64) -> Result<Self::Value, E> {
        serde_json::Number::from_f64(v)
            .map(TemplateValue::Number)
            .ok_or_else(|| E::custom("non-finite number is not representable in JSON"))
    }

    fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
        Ok(TemplateValue::String(v.to_owned()))
    }

    fn visit_string<E: de::Error>(self, v: String) -> Result<Self::Value, E> {
        Ok(TemplateValue::String(v))
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
        let mut items = Vec::with_capacity(seq.size_hint().unwrap_or(0).min(4096));
        while let Some(item) = seq.next_element()? {
            items.push(item);
        }
        Ok(TemplateValue::Array(items))
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
        let mut seen = HashSet::new();
        let mut entries = Vec::new();
        while let Some(key) = map.next_key::<String>()? {
            if !seen.insert(key.clone()) {
                return Err(de::Error::custom(format!("duplicate key `{key}`")));
            }
            entries.push((key, map.next_value()?));
        }
        Ok(TemplateValue::Object(entries))
    }
}

impl<'de> Deserialize<'de> for TemplateValue {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        d.deserialize_any(ValueVisitor)
    }
}
