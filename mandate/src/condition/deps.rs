//! The fields and relation paths a condition reads.

use crate::{Condition, FieldIdx, FieldKind, FieldMask, RelationProjection, Schema};

/// Adds every field `c` reads on `schema` to `fields`, and every relation path it
/// follows to `rels` (merged per relation, sorted by relation index).
///
/// Recursion follows the condition's own paths only, so cyclic schemas terminate.
pub(crate) fn collect(
    c: &Condition,
    schema: &'static Schema,
    fields: &mut FieldMask,
    rels: &mut Vec<RelationProjection>,
) {
    match c {
        Condition::Cmp { field, .. }
        | Condition::In { field, .. }
        | Condition::NotIn { field, .. }
        | Condition::Str { field, .. }
        | Condition::IsNull(field)
        | Condition::IsNotNull(field) => fields.insert(*field),
        Condition::And(cs) | Condition::Or(cs) => {
            for c in cs {
                collect(c, schema, fields, rels);
            }
        }
        Condition::Not(c) => collect(c, schema, fields, rels),
        Condition::Rel { relation, cond, .. } => {
            let Some(FieldKind::Relation { target, .. }) = schema.field(*relation).map(|d| d.kind)
            else {
                return;
            };
            let entry = entry(rels, *relation, target());
            if let Some(c) = cond {
                collect(c, entry.target, &mut entry.fields, &mut entry.relations);
            }
        }
    }
}

/// The entry for `relation`, inserted in sorted position if missing.
fn entry<'a>(
    rels: &'a mut Vec<RelationProjection>,
    relation: FieldIdx,
    target: &'static Schema,
) -> &'a mut RelationProjection {
    let at = match rels.binary_search_by_key(&relation, |r| r.relation) {
        Ok(at) => at,
        Err(at) => {
            rels.insert(
                at,
                RelationProjection {
                    relation,
                    target,
                    fields: FieldMask::default(),
                    relations: Vec::new(),
                },
            );
            at
        }
    };
    &mut rels[at]
}
