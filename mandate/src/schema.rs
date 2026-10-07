//! Static schema description of an entity type.

use serde::Serialize;

/// Maximum number of fields a [`Schema`] may declare.
pub const MAX_FIELDS: usize = 128;

/// Index of a field within its [`Schema`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub struct FieldIdx(pub u16);

/// The scalar kind of a field or value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Boolean.
    Bool,
    /// Signed 64-bit integer.
    Int,
    /// 64-bit float.
    Float,
    /// UTF-8 string.
    String,
    /// String restricted to the listed variant names.
    Enum(&'static [&'static str]),
    /// UUID.
    Uuid,
    /// UTC timestamp (microsecond precision).
    DateTime,
    /// Calendar date.
    Date,
}

/// Cardinality of a relation field.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CardinalityKind {
    /// At most one related entity.
    ToOne,
    /// Any number of related entities.
    ToMany,
}

/// What a field holds.
#[derive(Clone, Copy, Debug)]
pub enum FieldKind {
    /// A scalar value.
    Scalar {
        /// Scalar kind.
        kind: Kind,
        /// Whether the field may be null.
        nullable: bool,
    },
    /// A field that cannot be inspected by conditions.
    Opaque,
    /// A relation to another schema.
    Relation {
        /// Target schema accessor (a function so schemas may be recursive).
        target: fn() -> &'static Schema,
        /// Relation cardinality.
        cardinality: CardinalityKind,
        /// Whether the relation may be absent.
        nullable: bool,
    },
}

/// A named field of a [`Schema`].
#[derive(Clone, Copy, Debug)]
pub struct FieldDef {
    /// Field name.
    pub name: &'static str,
    /// Field kind.
    pub kind: FieldKind,
}

impl FieldDef {
    /// Declares a scalar field.
    pub const fn scalar(name: &'static str, kind: Kind, nullable: bool) -> FieldDef {
        FieldDef {
            name,
            kind: FieldKind::Scalar { kind, nullable },
        }
    }

    /// Declares an opaque field.
    pub const fn opaque(name: &'static str) -> FieldDef {
        FieldDef {
            name,
            kind: FieldKind::Opaque,
        }
    }

    /// Declares a relation field.
    pub const fn relation(
        name: &'static str,
        target: fn() -> &'static Schema,
        cardinality: CardinalityKind,
        nullable: bool,
    ) -> FieldDef {
        FieldDef {
            name,
            kind: FieldKind::Relation {
                target,
                cardinality,
                nullable,
            },
        }
    }
}

/// Static description of an entity type's fields.
#[derive(Debug)]
pub struct Schema {
    /// Entity type name.
    pub name: &'static str,
    /// Declared fields, indexed by [`FieldIdx`].
    pub fields: &'static [FieldDef],
}

impl Schema {
    /// Builds a schema.
    ///
    /// Panics at compile time (in `const` context) if there are more than
    /// [`MAX_FIELDS`] fields or any field name starts with `$`.
    pub const fn new(name: &'static str, fields: &'static [FieldDef]) -> Schema {
        assert!(
            fields.len() <= MAX_FIELDS,
            "schema has more than MAX_FIELDS fields"
        );
        let mut i = 0;
        while i < fields.len() {
            let bytes = fields[i].name.as_bytes();
            assert!(
                bytes.is_empty() || bytes[0] != b'$',
                "field names must not start with `$`"
            );
            i += 1;
        }
        Schema { name, fields }
    }

    /// Returns the field at `idx`, if any.
    pub fn field(&self, idx: FieldIdx) -> Option<&FieldDef> {
        self.fields.get(usize::from(idx.0))
    }

    /// Returns the index of the field called `name`, if any.
    pub fn index_of(&self, name: &str) -> Option<FieldIdx> {
        self.fields
            .iter()
            .position(|f| f.name == name)
            .and_then(|i| u16::try_from(i).ok())
            .map(FieldIdx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn index_of_finds_fields() {
        static S: Schema = Schema::new(
            "T",
            &[
                FieldDef::scalar("a", Kind::Int, false),
                FieldDef::opaque("b"),
            ],
        );
        assert_eq!(S.index_of("b"), Some(FieldIdx(1)));
        assert_eq!(S.index_of("zz"), None);
        assert!(S.field(FieldIdx(2)).is_none());
    }
}
