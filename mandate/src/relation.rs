//! Relation slots: how relation fields expose related resources.

use crate::{CardinalityKind, DynResource, NonNull, Nullability, Nullable, Resource};

/// Type-level cardinality of a relation.
pub trait Cardinality {
    /// The runtime cardinality.
    const KIND: CardinalityKind;
}

/// Marker for to-one relations.
pub enum ToOne {}
/// Marker for to-many relations.
pub enum ToMany {}

impl Cardinality for ToOne {
    const KIND: CardinalityKind = CardinalityKind::ToOne;
}
impl Cardinality for ToMany {
    const KIND: CardinalityKind = CardinalityKind::ToMany;
}

/// A pointer-like holder of a single resource.
pub trait ResourcePtr {
    /// The pointed-to resource type.
    type Target: Resource;
    /// Borrows the target.
    fn target(&self) -> &Self::Target;
}

/// A relation field's storage type.
pub trait RelationSlot {
    /// The related resource type.
    type Target: Resource;
    /// To-one or to-many.
    type Cardinality: Cardinality;
    /// Whether the relation may be absent.
    type Nullability: Nullability;
    /// The current state of the slot.
    fn get(&self) -> RelationRef<'_>;
}

/// The runtime state of a relation slot.
pub enum RelationRef<'a> {
    /// The relation is not loaded.
    NotLoaded,
    /// A loaded to-one relation with no target.
    Absent,
    /// A loaded to-one relation.
    One(&'a dyn DynResource),
    /// A loaded to-many relation.
    Many(&'a dyn DynMany),
}

/// Object-safe view of a to-many relation.
pub trait DynMany {
    /// Number of related resources.
    fn len(&self) -> usize;
    /// Whether there are no related resources.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
    /// The `i`-th related resource. Callers must pass `i < len()`; an
    /// out-of-range index panics (the signature admits no other outcome).
    fn get(&self, i: usize) -> &dyn DynResource;
}

impl<T: Resource> ResourcePtr for Box<T> {
    type Target = T;
    fn target(&self) -> &T {
        self
    }
}

impl<T: Resource> RelationSlot for Box<T> {
    type Target = T;
    type Cardinality = ToOne;
    type Nullability = NonNull;
    fn get(&self) -> RelationRef<'_> {
        RelationRef::One((**self).as_dyn())
    }
}

impl<P: ResourcePtr> RelationSlot for Option<P> {
    type Target = P::Target;
    type Cardinality = ToOne;
    type Nullability = Nullable;
    fn get(&self) -> RelationRef<'_> {
        match self {
            Some(p) => RelationRef::One(p.target().as_dyn()),
            None => RelationRef::Absent,
        }
    }
}

impl<P: ResourcePtr> RelationSlot for Vec<P> {
    type Target = P::Target;
    type Cardinality = ToMany;
    type Nullability = NonNull;
    fn get(&self) -> RelationRef<'_> {
        RelationRef::Many(self)
    }
}

impl<P: ResourcePtr> DynMany for Vec<P> {
    fn len(&self) -> usize {
        Vec::len(self)
    }
    fn get(&self, i: usize) -> &dyn DynResource {
        self[i].target().as_dyn()
    }
}
