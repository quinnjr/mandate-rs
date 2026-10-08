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
///
/// Built with the `const` constructors [`scalar`](Self::scalar),
/// [`opaque`](Self::opaque) and [`relation`](Self::relation), and read
/// through [`name`](Self::name) and [`kind`](Self::kind).
#[derive(Clone, Copy, Debug)]
pub struct FieldDef {
    name: &'static str,
    kind: FieldKind,
}

impl FieldDef {
    /// The field's name, as used in rules.
    pub const fn name(&self) -> &'static str {
        self.name
    }

    /// What the field holds.
    pub const fn kind(&self) -> FieldKind {
        self.kind
    }

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
///
/// Built with the `const` constructor [`new`](Self::new), normally into a
/// `static` (its address is its identity), and read through
/// [`name`](Self::name) and [`fields`](Self::fields).
#[derive(Debug)]
pub struct Schema {
    name: &'static str,
    fields: &'static [FieldDef],
}

impl Schema {
    /// Builds a schema.
    ///
    /// Panics at compile time (in `const` context) if there are more than
    /// [`MAX_FIELDS`] fields, any field name starts with `$`, or two fields
    /// have the same name.
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
            let mut j = 0;
            while j < i {
                assert!(
                    !same_bytes(bytes, fields[j].name.as_bytes()),
                    "duplicate field name in schema"
                );
                j += 1;
            }
            i += 1;
        }
        Schema { name, fields }
    }

    /// The entity type's name.
    pub const fn name(&self) -> &'static str {
        self.name
    }

    /// The declared fields, indexed by [`FieldIdx`].
    pub const fn fields(&self) -> &'static [FieldDef] {
        self.fields
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

/// Byte-wise equality, usable in `const` context.
const fn same_bytes(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut i = 0;
    while i < a.len() {
        if a[i] != b[i] {
            return false;
        }
        i += 1;
    }
    true
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
        assert_eq!(S.name(), "T");
        assert_eq!(S.fields().len(), 2);
        assert_eq!(S.fields()[1].name(), "b");
        assert!(matches!(S.fields()[1].kind(), FieldKind::Opaque));
    }

    // `Schema::new` is meant for `const` context, where these panics are
    // compile errors (see `tests/ui/fail/schema_duplicate_name.rs`); called
    // at run time, they are ordinary panics.

    #[test]
    #[should_panic(expected = "duplicate field name in schema")]
    fn duplicate_names_are_rejected() {
        static FIELDS: [FieldDef; 3] = [
            FieldDef::scalar("ab", Kind::Int, false),
            FieldDef::opaque("a"),
            FieldDef::opaque("ab"),
        ];
        Schema::new("T", &FIELDS);
    }

    #[test]
    fn names_differing_in_length_or_bytes_are_distinct() {
        static FIELDS: [FieldDef; 4] = [
            FieldDef::opaque("a"),
            FieldDef::opaque("ab"),
            FieldDef::opaque("b"),
            FieldDef::opaque(""),
        ];
        assert_eq!(Schema::new("T", &FIELDS).fields().len(), 4);
    }
}
