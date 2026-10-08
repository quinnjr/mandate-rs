//! Serializing bound rules back to the template JSON shape (spec §6.6).
//!
//! The inverse of [`Templates::compile`](crate::Templates::compile): compiling
//! the output (with no placeholders) and building reproduces the rules.

use std::collections::HashSet;

use serde::Serialize;
use serde::ser::{Error, SerializeMap, Serializer};

use super::TemplateValue;
use crate::{
    Action, CardinalityKind, CmpOp, Condition, FieldIdx, FieldKind, Quant, Rule, Schema, StrOp,
    Subject, Value,
};

type Entries = Vec<(String, TemplateValue)>;

impl<A: Action, S: Subject> Serialize for Rule<A, S> {
    fn serialize<Ser: Serializer>(&self, s: Ser) -> Result<Ser::Ok, Ser::Error> {
        let schema = self.subject().schema();
        let conditions = match self.condition() {
            None => None,
            Some(c) => {
                let schema = schema.ok_or_else(|| {
                    Ser::Error::custom("rule has a condition but its subject has no schema")
                })?;
                Some(TemplateValue::Object(
                    object(c, schema).map_err(Ser::Error::custom)?,
                ))
            }
        };
        let mut map = s.serialize_map(None)?;
        map.serialize_entry("action", self.action().name())?;
        map.serialize_entry("subject", self.subject().name())?;
        if let Some(c) = &conditions {
            map.serialize_entry("conditions", c)?;
        }
        if let Some(mask) = self.fields() {
            let schema =
                schema.ok_or_else(|| Ser::Error::custom("rule has fields but no schema"))?;
            let names = mask
                .iter()
                .map(|i| {
                    schema
                        .field(i)
                        .map(|d| d.name())
                        .ok_or_else(|| Ser::Error::custom("field index out of range"))
                })
                .collect::<Result<Vec<_>, _>>()?;
            map.serialize_entry("fields", &names)?;
        }
        if self.inverted() {
            map.serialize_entry("inverted", &true)?;
        }
        if let Some(reason) = self.reason() {
            map.serialize_entry("reason", reason)?;
        }
        map.end()
    }
}

fn key(s: &str) -> String {
    s.to_owned()
}

/// The entries of the condition object for `c` over `schema`.
fn object(c: &Condition, schema: &'static Schema) -> Result<Entries, String> {
    Ok(match c {
        Condition::And(children) => {
            let parts = children
                .iter()
                .map(|c| object(c, schema))
                .collect::<Result<Vec<_>, _>>()?;
            let mut seen = HashSet::new();
            if parts.iter().flatten().all(|(k, _)| seen.insert(k.as_str())) {
                parts.into_iter().flatten().collect()
            } else {
                let items = parts.into_iter().map(TemplateValue::Object).collect();
                vec![(key("$and"), TemplateValue::Array(items))]
            }
        }
        Condition::Or(children) => {
            let items = children
                .iter()
                .map(|c| object(c, schema).map(TemplateValue::Object))
                .collect::<Result<_, _>>()?;
            vec![(key("$or"), TemplateValue::Array(items))]
        }
        Condition::Not(inner) => vec![(key("$not"), TemplateValue::Object(object(inner, schema)?))],
        Condition::Cmp { field, op, value } => {
            let v = value_json(value)?;
            let name = name(schema, *field)?;
            match op {
                CmpOp::Eq => vec![(name, v)],
                _ => vec![(name, op_object(cmp_key(*op), v))],
            }
        }
        Condition::In { field, values } => {
            vec![(name(schema, *field)?, op_object("$in", list(values)?))]
        }
        Condition::NotIn { field, values } => {
            vec![(name(schema, *field)?, op_object("$nin", list(values)?))]
        }
        Condition::Str { field, op, value } => {
            let v = value_json(&Value::String(value.clone()))?;
            vec![(name(schema, *field)?, op_object(str_key(*op), v))]
        }
        Condition::IsNull(field) => vec![(name(schema, *field)?, TemplateValue::Null)],
        Condition::IsNotNull(field) => {
            let v = if is_relation(schema, *field) {
                op_object("$isNull", TemplateValue::Bool(false))
            } else {
                op_object("$ne", TemplateValue::Null)
            };
            vec![(name(schema, *field)?, v)]
        }
        Condition::Rel {
            relation,
            quant,
            cond,
        } => {
            let def = schema
                .field(*relation)
                .ok_or("relation index out of range")?;
            let FieldKind::Relation {
                target,
                cardinality,
                ..
            } = def.kind()
            else {
                return Err(format!("`{}` is not a relation", def.name()));
            };
            let inner = match cond {
                Some(c) => TemplateValue::Object(object(c, target())?),
                None => TemplateValue::Object(Vec::new()),
            };
            let value = match (cardinality, quant) {
                (CardinalityKind::ToOne, Quant::One) => inner,
                (CardinalityKind::ToOne, Quant::None) | (CardinalityKind::ToMany, Quant::None) => {
                    op_object("$none", inner)
                }
                (CardinalityKind::ToMany, Quant::Some) => op_object("$some", inner),
                (CardinalityKind::ToMany, Quant::Every) => op_object("$every", inner),
                _ => {
                    return Err(format!(
                        "quantifier {quant:?} does not fit `{}`",
                        def.name()
                    ));
                }
            };
            vec![(key(def.name()), value)]
        }
    })
}

fn name(schema: &Schema, field: FieldIdx) -> Result<String, String> {
    schema
        .field(field)
        .map(|d| key(d.name()))
        .ok_or_else(|| "field index out of range".to_owned())
}

fn is_relation(schema: &Schema, field: FieldIdx) -> bool {
    schema
        .field(field)
        .is_some_and(|d| matches!(d.kind(), FieldKind::Relation { .. }))
}

fn op_object(op: &str, v: TemplateValue) -> TemplateValue {
    TemplateValue::Object(vec![(key(op), v)])
}

fn cmp_key(op: CmpOp) -> &'static str {
    match op {
        CmpOp::Eq => "$eq",
        CmpOp::Ne => "$ne",
        CmpOp::Lt => "$lt",
        CmpOp::Lte => "$lte",
        CmpOp::Gt => "$gt",
        CmpOp::Gte => "$gte",
    }
}

fn str_key(op: StrOp) -> &'static str {
    match op {
        StrOp::Contains => "$contains",
        StrOp::StartsWith => "$startsWith",
        StrOp::EndsWith => "$endsWith",
    }
}

fn list(values: &[Value]) -> Result<TemplateValue, String> {
    values
        .iter()
        .map(value_json)
        .collect::<Result<_, _>>()
        .map(TemplateValue::Array)
}

/// A literal as the template compiler reads it back (spec §6.3).
fn value_json(v: &Value) -> Result<TemplateValue, String> {
    Ok(match v {
        Value::Bool(b) => TemplateValue::Bool(*b),
        Value::Int(i) => TemplateValue::Number((*i).into()),
        Value::Float(f) => TemplateValue::Number(
            serde_json::Number::from_f64(*f).ok_or("non-finite float is not representable")?,
        ),
        Value::String(s) if s.starts_with('$') => TemplateValue::String(format!("${s}")),
        Value::String(s) => TemplateValue::String(s.clone()),
        #[cfg(feature = "uuid")]
        Value::Uuid(u) => TemplateValue::String(u.hyphenated().to_string()),
        #[cfg(feature = "chrono")]
        Value::DateTime(dt) => {
            TemplateValue::String(dt.to_rfc3339_opts(chrono::SecondsFormat::Micros, true))
        }
        #[cfg(feature = "chrono")]
        Value::Date(d) => TemplateValue::String(d.format("%Y-%m-%d").to_string()),
    })
}
