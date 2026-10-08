//! Field masks and typed field sets.

use crate::{FieldIdx, FieldRef, Resource};
use core::fmt;
use core::marker::PhantomData;

/// A bitset of up to 128 field indices.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct FieldMask([u64; 2]);

impl FieldMask {
    /// Adds a field (indices of 128 or more are ignored).
    pub fn insert(&mut self, f: FieldIdx) {
        if let Some(w) = self.0.get_mut(usize::from(f.0) / 64) {
            *w |= 1 << (f.0 % 64);
        }
    }
    /// Removes a field.
    pub fn remove(&mut self, f: FieldIdx) {
        if let Some(w) = self.0.get_mut(usize::from(f.0) / 64) {
            *w &= !(1 << (f.0 % 64));
        }
    }
    /// Whether the field is present.
    pub fn contains(&self, f: FieldIdx) -> bool {
        self.0
            .get(usize::from(f.0) / 64)
            .is_some_and(|w| w >> (f.0 % 64) & 1 == 1)
    }
    /// The union of two masks.
    pub fn union(self, other: FieldMask) -> FieldMask {
        FieldMask([self.0[0] | other.0[0], self.0[1] | other.0[1]])
    }
    /// Whether every field in `self` is in `other`.
    pub fn is_subset(&self, other: &FieldMask) -> bool {
        self.0[0] & !other.0[0] == 0 && self.0[1] & !other.0[1] == 0
    }
    /// Whether no field is present.
    pub fn is_empty(&self) -> bool {
        self.0 == [0, 0]
    }
    /// Number of fields present.
    pub fn len(&self) -> usize {
        (self.0[0].count_ones() + self.0[1].count_ones()) as usize
    }
    /// Present fields in ascending order.
    pub fn iter(&self) -> impl Iterator<Item = FieldIdx> {
        let m = *self;
        (0..128u16).map(FieldIdx).filter(move |f| m.contains(*f))
    }
    /// A mask with the first `n` fields set (`n` is capped at 128).
    pub fn all(n: usize) -> FieldMask {
        let n = n.min(128);
        let word = |lo: usize| match n.saturating_sub(lo) {
            0 => 0,
            k if k >= 64 => u64::MAX,
            k => (1u64 << k) - 1,
        };
        FieldMask([word(0), word(64)])
    }
}

/// A set of fields of resource `R`.
pub struct FieldSet<R> {
    mask: FieldMask,
    _r: PhantomData<fn() -> R>,
}

impl<R> FieldSet<R> {
    pub(crate) fn from_mask(mask: FieldMask) -> Self {
        FieldSet {
            mask,
            _r: PhantomData,
        }
    }
    /// The underlying mask.
    pub fn mask(&self) -> FieldMask {
        self.mask
    }
    /// Whether the set contains the field.
    pub fn contains(&self, f: impl Into<FieldRef<R>>) -> bool {
        self.mask.contains(f.into().idx())
    }
}

impl<R: Resource> FieldSet<R> {
    /// Fields in the set with their names, in ascending index order.
    pub fn iter(&self) -> impl Iterator<Item = (FieldIdx, &'static str)> {
        let schema = R::schema();
        self.mask
            .iter()
            .filter_map(move |i| schema.field(i).map(|d| (i, d.name())))
    }
}

impl<R> Clone for FieldSet<R> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<R> Copy for FieldSet<R> {}
impl<R> PartialEq for FieldSet<R> {
    fn eq(&self, other: &Self) -> bool {
        self.mask == other.mask
    }
}
impl<R> fmt::Debug for FieldSet<R> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("FieldSet").field(&self.mask).finish()
    }
}
