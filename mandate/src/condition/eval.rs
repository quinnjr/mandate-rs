//! In-memory evaluation of conditions (spec §5.3).
//!
//! Logic is two-valued: null is tested only by `IsNull`/`IsNotNull`, and every
//! other leaf on a null value has the fixed result of the §5.3 null table.
//! Unloaded data and schema mismatches fail closed with an [`EvalError`].

use core::cmp::Ordering;

use super::{CmpOp, Condition, Quant, StrOp};
#[cfg(feature = "chrono")]
use crate::truncate_micros;
use crate::{
    DynResource, EvalError, FieldDef, FieldIdx, FieldKind, RelationRef, Schema, Value, ValueRef,
};

/// Evaluates `cond` against `r`, whose schema must be `schema`.
///
/// `And`/`Or` evaluate their children, and quantifiers their related rows,
/// in order, stopping at the first one that decides the result; data that is
/// not loaded is an error only if it is reached.
// Only the unit tests below call `eval` until `Ability` does.
#[cfg_attr(
    not(all(test, feature = "derive", feature = "chrono", feature = "uuid")),
    allow(dead_code)
)]
pub(crate) fn eval(
    cond: &Condition,
    schema: &'static Schema,
    r: &dyn DynResource,
) -> Result<bool, EvalError> {
    let found = r.resource_schema();
    if !core::ptr::eq(found, schema) {
        return Err(EvalError::SchemaMismatch {
            expected: schema.name,
            found: found.name,
        });
    }
    node(cond, schema, r)
}

fn node(cond: &Condition, schema: &'static Schema, r: &dyn DynResource) -> Result<bool, EvalError> {
    Ok(match cond {
        Condition::Cmp { field, op, value } => compare(scalar(schema, r, *field)?, *op, value),
        Condition::In { field, values } => {
            let v = scalar(schema, r, *field)?;
            values.iter().any(|x| equals(v, x))
        }
        Condition::NotIn { field, values } => {
            let v = scalar(schema, r, *field)?;
            !values.iter().any(|x| equals(v, x))
        }
        Condition::Str { field, op, value } => match scalar(schema, r, *field)? {
            // Byte-exact: no case folding, normalization, or trimming.
            ValueRef::Str(s) => match op {
                StrOp::Contains => s.contains(value.as_str()),
                StrOp::StartsWith => s.starts_with(value.as_str()),
                StrOp::EndsWith => s.ends_with(value.as_str()),
            },
            // Null, or a kind mismatch read as null.
            _ => false,
        },
        Condition::IsNull(field) => is_null(schema, r, *field)?,
        Condition::IsNotNull(field) => !is_null(schema, r, *field)?,
        Condition::And(cs) => {
            for c in cs {
                if !node(c, schema, r)? {
                    return Ok(false);
                }
            }
            true
        }
        Condition::Or(cs) => {
            for c in cs {
                if node(c, schema, r)? {
                    return Ok(true);
                }
            }
            false
        }
        Condition::Not(c) => !node(c, schema, r)?,
        Condition::Rel {
            relation,
            quant,
            cond,
        } => related(schema, r, *relation, *quant, cond.as_deref())?,
    })
}

/// Reads a scalar field, failing closed if it is not loaded.
fn scalar<'r>(
    schema: &Schema,
    r: &'r dyn DynResource,
    field: FieldIdx,
) -> Result<ValueRef<'r>, EvalError> {
    match r.value(field) {
        ValueRef::NotLoaded => Err(not_loaded(schema, field)),
        v => Ok(v),
    }
}

/// Applies `op`. A null instance value, or one whose kind differs from the
/// operand's, satisfies only `Ne`.
fn compare(v: ValueRef<'_>, op: CmpOp, x: &Value) -> bool {
    use Ordering::{Equal, Greater, Less};
    match op {
        CmpOp::Eq => equals(v, x),
        CmpOp::Ne => !equals(v, x),
        CmpOp::Lt => order(v, x) == Some(Less),
        CmpOp::Lte => matches!(order(v, x), Some(Less | Equal)),
        CmpOp::Gt => order(v, x) == Some(Greater),
        CmpOp::Gte => matches!(order(v, x), Some(Greater | Equal)),
    }
}

/// Whether `v` equals `x`; `false` if `v` is null or of another kind.
fn equals(v: ValueRef<'_>, x: &Value) -> bool {
    match (v, x) {
        (ValueRef::Bool(a), Value::Bool(b)) => a == *b,
        (ValueRef::Int(a), Value::Int(b)) => a == *b,
        (ValueRef::Float(a), Value::Float(b)) => a == *b,
        // Byte-exact; also covers enums, whose values are variant names.
        (ValueRef::Str(a), Value::String(b)) => a == b.as_str(),
        #[cfg(feature = "uuid")]
        (ValueRef::Uuid(a), Value::Uuid(b)) => a == *b,
        #[cfg(feature = "chrono")]
        (ValueRef::DateTime(a), Value::DateTime(b)) => truncate_micros(a) == *b,
        #[cfg(feature = "chrono")]
        (ValueRef::Date(a), Value::Date(b)) => a == *b,
        _ => false,
    }
}

/// How `v` orders against `x`; `None` if `v` is null, of another kind, or of a
/// kind without an ordering.
fn order(v: ValueRef<'_>, x: &Value) -> Option<Ordering> {
    match (v, x) {
        (ValueRef::Int(a), Value::Int(b)) => Some(a.cmp(b)),
        (ValueRef::Float(a), Value::Float(b)) => a.partial_cmp(b),
        #[cfg(feature = "chrono")]
        (ValueRef::DateTime(a), Value::DateTime(b)) => Some(truncate_micros(a).cmp(b)),
        #[cfg(feature = "chrono")]
        (ValueRef::Date(a), Value::Date(b)) => Some(a.cmp(b)),
        _ => None,
    }
}

/// Whether a nullable scalar is null or a to-one relation is absent.
fn is_null(schema: &Schema, r: &dyn DynResource, field: FieldIdx) -> Result<bool, EvalError> {
    match schema.field(field) {
        Some(FieldDef {
            kind: FieldKind::Relation { .. },
            ..
        }) => match r.relation(field) {
            RelationRef::NotLoaded => Err(not_loaded(schema, field)),
            RelationRef::Absent => Ok(true),
            RelationRef::One(_) | RelationRef::Many(_) => Ok(false),
        },
        _ => Ok(matches!(scalar(schema, r, field)?, ValueRef::Null)),
    }
}

/// Evaluates a quantifier over a relation's rows: none for an absent to-one,
/// one for a present to-one, and each element of a to-many, in order.
fn related(
    schema: &Schema,
    r: &dyn DynResource,
    field: FieldIdx,
    quant: Quant,
    cond: Option<&Condition>,
) -> Result<bool, EvalError> {
    let Some(FieldDef {
        name,
        kind: FieldKind::Relation { target, .. },
    }) = schema.field(field)
    else {
        // Not a relation (unreachable after validation): read as absent.
        return Ok(matches!(quant, Quant::Every | Quant::None));
    };
    let target = target();
    // Whether a row satisfies `cond`; a row the relation fails to yield is
    // treated as not loaded.
    let test = |row: Option<&dyn DynResource>| match (row, cond) {
        (None, _) => Err(not_loaded(schema, field)),
        (Some(_), None) => Ok(true),
        (Some(row), Some(c)) => eval(c, target, row).map_err(|e| within(name, e)),
    };
    // The first row whose result is `witness` decides the quantifier: a
    // satisfying row for One/Some/None, a failing row for Every.
    let witness = quant != Quant::Every;
    let found = match r.relation(field) {
        RelationRef::NotLoaded => return Err(not_loaded(schema, field)),
        RelationRef::Absent => false,
        RelationRef::One(row) => test(Some(row))? == witness,
        RelationRef::Many(rows) => 'rows: {
            for i in 0..rows.len() {
                if test(rows.get(i))? == witness {
                    break 'rows true;
                }
            }
            false
        }
    };
    Ok(match quant {
        Quant::One | Quant::Some => found,
        Quant::Every | Quant::None => !found,
    })
}

/// The `NotLoaded` error for a field of `schema`.
fn not_loaded(schema: &Schema, field: FieldIdx) -> EvalError {
    let path = match schema.field(field) {
        Some(def) => def.name.to_owned(),
        // Unreachable after validation.
        None => format!("#{}", field.0),
    };
    EvalError::NotLoaded { path }
}

/// Prefixes a `NotLoaded` path with the relation it was reached through.
fn within(relation: &str, e: EvalError) -> EvalError {
    match e {
        EvalError::NotLoaded { path } => EvalError::NotLoaded {
            path: format!("{relation}.{path}"),
        },
        e => e,
    }
}

#[cfg(all(test, feature = "derive", feature = "chrono", feature = "uuid"))]
mod tests {
    use chrono::{DateTime, Utc};

    use super::eval;
    use crate::test_fixture::*;
    use crate::{
        CmpOp, Cond, Condition, DynMany, DynResource, EvalError, FieldIdx, Quant, RelationRef,
        Resource, Schema, StrOp, Value, ValueRef,
    };

    fn ev<R: Resource>(c: Cond<R>, r: &R) -> Result<bool, EvalError> {
        eval(&c.into_condition(), R::schema(), r.as_dyn())
    }

    fn ev_raw<R: Resource>(c: Condition, r: &R) -> Result<bool, EvalError> {
        eval(&c, R::schema(), r.as_dyn())
    }

    fn rel(relation: FieldIdx, quant: Quant, cond: Option<Condition>) -> Condition {
        Condition::Rel {
            relation,
            quant,
            cond: cond.map(Box::new),
        }
    }

    fn not_loaded(path: &str) -> Result<bool, EvalError> {
        Err(EvalError::NotLoaded { path: path.into() })
    }

    fn tag(id: i64, name: &str) -> Tag {
        Tag {
            id,
            name: Some(name.into()),
        }
    }

    /// A fully loaded tracked post mirroring `post()`.
    fn tpost() -> TPost {
        TPost {
            loaded: Loaded::default(),
            id: 1,
            author_id: 7,
            reviewer_id: None,
            title: "Hello".into(),
            status: Status::Published,
            score: 1.0,
            org: Lazy::Loaded(TOrg {
                loaded: Loaded::default(),
                id: 3,
                name: "Acme".into(),
            }),
            reviewer: Lazy::Loaded(None),
            tags: Lazy::Loaded(vec![]),
        }
    }

    #[test]
    fn null_semantics_table() {
        let mut p = post(); // reviewer_id: None
        assert_eq!(ev(Post::REVIEWER_ID.eq(1), &p), Ok(false));
        assert_eq!(ev(Post::REVIEWER_ID.ne(1), &p), Ok(true));
        assert_eq!(ev(Post::REVIEWER_ID.lt(1), &p), Ok(false));
        assert_eq!(ev(Post::REVIEWER_ID.lte(1), &p), Ok(false));
        assert_eq!(ev(Post::REVIEWER_ID.gt(1), &p), Ok(false));
        assert_eq!(ev(Post::REVIEWER_ID.gte(1), &p), Ok(false));
        assert_eq!(ev(Post::REVIEWER_ID.is_in([1]), &p), Ok(false));
        assert_eq!(ev(Post::REVIEWER_ID.not_in([1]), &p), Ok(true));
        assert_eq!(ev(Post::REVIEWER_ID.is_null(), &p), Ok(true));
        assert_eq!(ev(Post::REVIEWER_ID.is_not_null(), &p), Ok(false));
        // `Not` is plain boolean negation.
        assert_eq!(ev(!Post::REVIEWER_ID.eq(1), &p), Ok(true));
        assert_eq!(ev(!Post::REVIEWER_ID.ne(1), &p), Ok(false));

        // The same leaves on a non-null value.
        p.reviewer_id = Some(1);
        assert_eq!(ev(Post::REVIEWER_ID.eq(1), &p), Ok(true));
        assert_eq!(ev(Post::REVIEWER_ID.ne(1), &p), Ok(false));
        assert_eq!(ev(Post::REVIEWER_ID.is_in([1]), &p), Ok(true));
        assert_eq!(ev(Post::REVIEWER_ID.not_in([1]), &p), Ok(false));
        assert_eq!(ev(Post::REVIEWER_ID.is_null(), &p), Ok(false));
        assert_eq!(ev(Post::REVIEWER_ID.is_not_null(), &p), Ok(true));

        let t = Tag { id: 1, name: None };
        assert_eq!(ev(Tag::NAME.contains("a"), &t), Ok(false));
        assert_eq!(ev(Tag::NAME.starts_with(""), &t), Ok(false));
        assert_eq!(ev(Tag::NAME.ends_with(""), &t), Ok(false));
        assert_eq!(ev(!Tag::NAME.contains("a"), &t), Ok(true));
    }

    #[test]
    fn comparisons_by_kind() {
        let mut p = post(); // author_id 7, score 1.0, locked false
        assert_eq!(ev(Post::SCORE.lt(1.5), &p), Ok(true));
        assert_eq!(ev(Post::SCORE.lt(1.0), &p), Ok(false));
        assert_eq!(ev(Post::SCORE.gte(1.0), &p), Ok(true));
        assert_eq!(ev(Post::SCORE.gte(1.5), &p), Ok(false));
        assert_eq!(ev(Post::AUTHOR_ID.eq(7), &p), Ok(true));
        assert_eq!(ev(Post::AUTHOR_ID.ne(7), &p), Ok(false));
        assert_eq!(ev(Post::AUTHOR_ID.gt(6), &p), Ok(true));
        assert_eq!(ev(Post::AUTHOR_ID.lte(6), &p), Ok(false));
        assert_eq!(ev(Post::AUTHOR_ID.is_in([1, 7]), &p), Ok(true));
        assert_eq!(ev(Post::AUTHOR_ID.not_in([1, 7]), &p), Ok(false));
        assert_eq!(ev(Post::LOCKED.eq(false), &p), Ok(true));

        // Instance timestamps are truncated to microseconds before comparison.
        let instant: DateTime<Utc> = "2020-01-15T10:30:00.000000400Z".parse().unwrap();
        let operand: DateTime<Utc> = "2020-01-15T10:30:00Z".parse().unwrap();
        p.published_at = Some(instant);
        assert_eq!(ev(Post::PUBLISHED_AT.eq(operand), &p), Ok(true));
        assert_eq!(ev(Post::PUBLISHED_AT.gt(operand), &p), Ok(false));
        assert_eq!(ev(Post::PUBLISHED_AT.lte(operand), &p), Ok(true));
    }

    #[test]
    fn kind_mismatch_reads_as_null() {
        let p = post(); // author_id 7, title "Hello"
        let author = Post::AUTHOR_ID.idx();
        let cmp = |field, op, value| Condition::Cmp { field, op, value };
        let seven = || Value::String("7".into());
        assert_eq!(ev_raw(cmp(author, CmpOp::Eq, seven()), &p), Ok(false));
        assert_eq!(ev_raw(cmp(author, CmpOp::Ne, seven()), &p), Ok(true));
        assert_eq!(ev_raw(cmp(author, CmpOp::Gte, seven()), &p), Ok(false));
        // Compared by kind, never converted.
        assert_eq!(
            ev_raw(cmp(author, CmpOp::Eq, Value::Float(7.0)), &p),
            Ok(false)
        );
        let not_in = Condition::NotIn {
            field: author,
            values: vec![seven()],
        };
        assert_eq!(ev_raw(not_in, &p), Ok(true));
        let contains = Condition::Str {
            field: author,
            op: StrOp::Contains,
            value: "7".into(),
        };
        assert_eq!(ev_raw(contains, &p), Ok(false));
        // Ordering is undefined on strings.
        let title_lt = cmp(Post::TITLE.idx(), CmpOp::Lt, Value::String("Z".into()));
        assert_eq!(ev_raw(title_lt, &p), Ok(false));
    }

    #[test]
    fn string_ops_are_byte_exact() {
        let titled = |t: &str| Post {
            title: t.into(),
            ..post()
        };
        assert_eq!(ev(Post::TITLE.eq("\u{e9}"), &titled("\u{e9}")), Ok(true));
        assert_eq!(ev(Post::TITLE.eq("e\u{301}"), &titled("\u{e9}")), Ok(false));
        assert_eq!(
            ev(Post::TITLE.contains("\u{c9}"), &titled("\u{e9}")),
            Ok(false)
        );
        assert_eq!(ev(Post::TITLE.eq("a"), &titled("A")), Ok(false));
        assert_eq!(ev(Post::TITLE.eq("a"), &titled("a ")), Ok(false));

        let p = post(); // title "Hello"
        assert_eq!(ev(Post::TITLE.contains("ell"), &p), Ok(true));
        assert_eq!(ev(Post::TITLE.starts_with("He"), &p), Ok(true));
        assert_eq!(ev(Post::TITLE.starts_with("he"), &p), Ok(false));
        assert_eq!(ev(Post::TITLE.ends_with("llo"), &p), Ok(true));
        assert_eq!(ev(Post::TITLE.ends_with("llo "), &p), Ok(false));
    }

    #[test]
    fn relation_semantics() {
        let mut p = post(); // org Org{id 3}, reviewer None, tags []
        assert_eq!(ev(Post::ORG.then(Org::ID.eq(3)), &p), Ok(true));
        assert_eq!(ev(Post::ORG.then(Org::ID.eq(4)), &p), Ok(false));

        // Absent to-one reviewer.
        let reviewer = Post::REVIEWER.idx();
        let user_1 = || Some(User::ID.eq(1).into_condition());
        assert_eq!(ev(Post::REVIEWER.then(User::ID.eq(1)), &p), Ok(false));
        assert_eq!(ev_raw(rel(reviewer, Quant::One, None), &p), Ok(false));
        assert_eq!(ev(Post::REVIEWER.is_null(), &p), Ok(true));
        assert_eq!(ev(Post::REVIEWER.is_not_null(), &p), Ok(false));
        assert_eq!(ev_raw(rel(reviewer, Quant::None, user_1()), &p), Ok(true));
        assert_eq!(ev_raw(rel(reviewer, Quant::None, None), &p), Ok(true));

        // Present to-one reviewer.
        p.reviewer = Some(User {
            id: 1,
            name: "Ann".into(),
            email: "ann@example.com".into(),
            posts: vec![],
        });
        assert_eq!(ev(Post::REVIEWER.then(User::ID.eq(1)), &p), Ok(true));
        assert_eq!(ev_raw(rel(reviewer, Quant::One, None), &p), Ok(true));
        assert_eq!(ev(Post::REVIEWER.is_null(), &p), Ok(false));
        assert_eq!(ev(Post::REVIEWER.is_not_null(), &p), Ok(true));
        assert_eq!(ev_raw(rel(reviewer, Quant::None, user_1()), &p), Ok(false));
        assert_eq!(ev_raw(rel(reviewer, Quant::None, None), &p), Ok(false));

        // No tags.
        assert_eq!(ev(Post::TAGS.some(Tag::ID.eq(1)), &p), Ok(false));
        assert_eq!(ev(Post::TAGS.every(Tag::ID.eq(1)), &p), Ok(true));
        assert_eq!(ev(Post::TAGS.none(Tag::ID.eq(1)), &p), Ok(true));
        assert_eq!(
            ev_raw(rel(Post::TAGS.idx(), Quant::Some, None), &p),
            Ok(false)
        );
        assert_eq!(
            ev_raw(rel(Post::TAGS.idx(), Quant::None, None), &p),
            Ok(true)
        );

        // A tag with a null name matches neither `name == "rust"` nor every-rust.
        p.tags = vec![Tag { id: 1, name: None }];
        assert_eq!(ev(Post::TAGS.every(Tag::NAME.eq("rust")), &p), Ok(false));
        assert_eq!(ev(Post::TAGS.some(Tag::NAME.eq("rust")), &p), Ok(false));
        assert_eq!(ev(Post::TAGS.none(Tag::NAME.eq("rust")), &p), Ok(true));

        p.tags = vec![tag(1, "rust"), tag(2, "go")];
        assert_eq!(ev(Post::TAGS.some(Tag::NAME.eq("go")), &p), Ok(true));
        assert_eq!(ev(Post::TAGS.every(Tag::NAME.eq("rust")), &p), Ok(false));
        assert_eq!(ev(Post::TAGS.every(Tag::ID.gte(1)), &p), Ok(true));
        assert_eq!(ev(Post::TAGS.none(Tag::NAME.eq("go")), &p), Ok(false));
        assert_eq!(ev(Post::TAGS.none(Tag::NAME.eq("c")), &p), Ok(true));
        assert_eq!(
            ev_raw(rel(Post::TAGS.idx(), Quant::Some, None), &p),
            Ok(true)
        );
        assert_eq!(
            ev_raw(rel(Post::TAGS.idx(), Quant::None, None), &p),
            Ok(false)
        );
    }

    #[test]
    fn not_loaded_fails_closed() {
        let p = TPost {
            loaded: Loaded(vec!["author_id"]),
            ..tpost()
        };
        assert_eq!(ev(TPost::AUTHOR_ID.eq(1), &p), not_loaded("author_id"));
        assert_eq!(ev(TPost::AUTHOR_ID.ne(1), &p), not_loaded("author_id"));
        assert_eq!(ev(!TPost::AUTHOR_ID.eq(1), &p), not_loaded("author_id"));
        assert_eq!(ev(TPost::ID.eq(1), &p), Ok(true));

        let p = TPost {
            loaded: Loaded(vec!["reviewer_id", "title"]),
            ..tpost()
        };
        assert_eq!(
            ev(TPost::REVIEWER_ID.is_null(), &p),
            not_loaded("reviewer_id")
        );
        assert_eq!(
            ev(TPost::REVIEWER_ID.is_not_null(), &p),
            not_loaded("reviewer_id")
        );
        assert_eq!(
            ev(TPost::REVIEWER_ID.not_in([1]), &p),
            not_loaded("reviewer_id")
        );
        assert_eq!(ev(TPost::TITLE.contains("H"), &p), not_loaded("title"));

        let p = TPost {
            tags: Lazy::NotLoaded,
            ..tpost()
        };
        assert_eq!(ev(TPost::TAGS.some(TTag::ID.eq(1)), &p), not_loaded("tags"));
        assert_eq!(
            ev(TPost::TAGS.every(TTag::ID.eq(1)), &p),
            not_loaded("tags")
        );
        assert_eq!(
            ev_raw(rel(TPost::TAGS.idx(), Quant::None, None), &p),
            not_loaded("tags")
        );

        let p = TPost {
            org: Lazy::Loaded(TOrg {
                loaded: Loaded(vec!["name"]),
                id: 3,
                name: "Acme".into(),
            }),
            ..tpost()
        };
        assert_eq!(
            ev(TPost::ORG.then(TOrg::NAME.eq("x")), &p),
            not_loaded("org.name")
        );
        assert_eq!(ev(TPost::ORG.then(TOrg::ID.eq(3)), &p), Ok(true));

        let p = TPost {
            reviewer: Lazy::NotLoaded,
            ..tpost()
        };
        assert_eq!(ev(TPost::REVIEWER.is_null(), &p), not_loaded("reviewer"));
        assert_eq!(
            ev(TPost::REVIEWER.is_not_null(), &p),
            not_loaded("reviewer")
        );
        assert_eq!(
            ev(TPost::REVIEWER.then(TUser::ID.eq(1)), &p),
            not_loaded("reviewer")
        );

        let p = TPost {
            tags: Lazy::Loaded(vec![TTag {
                loaded: Loaded(vec!["name"]),
                id: 1,
                name: None,
            }]),
            ..tpost()
        };
        assert_eq!(
            ev(TPost::TAGS.some(TTag::NAME.eq("rust")), &p),
            not_loaded("tags.name")
        );
        assert_eq!(ev(TPost::TAGS.some(TTag::ID.eq(1)), &p), Ok(true));
    }

    #[test]
    fn and_or_short_circuit() {
        let p = TPost {
            loaded: Loaded(vec!["author_id"]),
            ..tpost()
        };
        let unloaded = || TPost::AUTHOR_ID.eq(7);
        let false_leaf = || TPost::ID.eq(2);
        let true_leaf = || TPost::ID.eq(1);
        assert_eq!(ev(false_leaf().and(unloaded()), &p), Ok(false));
        assert_eq!(
            ev(unloaded().and(false_leaf()), &p),
            not_loaded("author_id")
        );
        assert_eq!(ev(true_leaf().and(unloaded()), &p), not_loaded("author_id"));
        assert_eq!(ev(true_leaf().or(unloaded()), &p), Ok(true));
        assert_eq!(ev(unloaded().or(true_leaf()), &p), not_loaded("author_id"));
        assert_eq!(ev(false_leaf().or(unloaded()), &p), not_loaded("author_id"));
        assert_eq!(ev(Cond::all([]), &p), Ok(true));
        assert_eq!(ev(Cond::any([]), &p), Ok(false));
    }

    #[test]
    fn quantifiers_short_circuit_in_order() {
        let unloaded_tag = TTag {
            loaded: Loaded(vec!["name"]),
            id: 1,
            name: None,
        };
        let rust_tag = TTag {
            loaded: Loaded::default(),
            id: 2,
            name: Some("rust".into()),
        };
        let with_tags = |tags| TPost {
            tags: Lazy::Loaded(tags),
            ..tpost()
        };
        let rust = || TTag::NAME.eq("rust");

        let p = with_tags(vec![rust_tag.clone(), unloaded_tag.clone()]);
        assert_eq!(ev(TPost::TAGS.some(rust()), &p), Ok(true));
        assert_eq!(ev(TPost::TAGS.none(rust()), &p), Ok(false));
        assert_eq!(ev(TPost::TAGS.every(!rust()), &p), Ok(false));

        let p = with_tags(vec![unloaded_tag, rust_tag]);
        assert_eq!(ev(TPost::TAGS.some(rust()), &p), not_loaded("tags.name"));
        assert_eq!(ev(TPost::TAGS.none(rust()), &p), not_loaded("tags.name"));
        assert_eq!(ev(TPost::TAGS.every(!rust()), &p), not_loaded("tags.name"));
    }

    #[test]
    fn enum_equality() {
        let p = post(); // status Published
        let published = Condition::Cmp {
            field: Post::STATUS.idx(),
            op: CmpOp::Eq,
            value: Value::String("published".into()),
        };
        assert_eq!(ev_raw(published, &p), Ok(true));
        assert_eq!(ev(Post::STATUS.eq(Status::Published), &p), Ok(true));
        assert_eq!(ev(Post::STATUS.eq(Status::Draft), &p), Ok(false));
        assert_eq!(ev(Post::STATUS.ne(Status::Draft), &p), Ok(true));
        assert_eq!(
            ev(Post::STATUS.is_in([Status::Draft, Status::Published]), &p),
            Ok(true)
        );
        assert_eq!(ev(Post::STATUS.not_in([Status::Draft]), &p), Ok(true));
    }

    /// A hand-written `Post` whose relations misbehave: `org` holds a `Tag`,
    /// and every other relation is a [`Short`] list.
    struct Imposter {
        org: Tag,
    }

    /// A to-many relation that reports one element but yields none.
    struct Short;

    impl DynMany for Short {
        fn len(&self) -> usize {
            1
        }
        fn get(&self, _: usize) -> Option<&dyn DynResource> {
            None
        }
    }

    impl DynResource for Imposter {
        fn resource_schema(&self) -> &'static Schema {
            Post::schema()
        }
        fn value(&self, _: FieldIdx) -> ValueRef<'_> {
            ValueRef::NotLoaded
        }
        fn relation(&self, field: FieldIdx) -> RelationRef<'_> {
            if field == Post::ORG.idx() {
                RelationRef::One(self.org.as_dyn())
            } else {
                RelationRef::Many(&Short)
            }
        }
    }

    #[test]
    fn schema_mismatch_fails_closed() {
        let org = post().org;
        let cond = Post::ID.eq(1).into_condition();
        assert_eq!(
            eval(&cond, Post::schema(), org.as_dyn()),
            Err(EvalError::SchemaMismatch {
                expected: "Post",
                found: "Org"
            })
        );

        let imposter = Imposter {
            org: tag(3, "Acme"),
        };
        let cond = Post::ORG.then(Org::ID.eq(3)).into_condition();
        assert_eq!(
            eval(&cond, Post::schema(), &imposter),
            Err(EvalError::SchemaMismatch {
                expected: "Org",
                found: "Tag"
            })
        );
    }

    #[test]
    fn missing_to_many_element_is_not_loaded() {
        let imposter = Imposter {
            org: tag(3, "Acme"),
        };
        let cond = Post::TAGS.some(Tag::ID.eq(1)).into_condition();
        assert_eq!(eval(&cond, Post::schema(), &imposter), not_loaded("tags"));
        let cond = Post::TAGS.every(Tag::ID.eq(1)).into_condition();
        assert_eq!(eval(&cond, Post::schema(), &imposter), not_loaded("tags"));
    }
}
