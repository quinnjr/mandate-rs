//! Typed field handles.

use crate::FieldIdx;
use core::fmt;
use core::marker::PhantomData;

macro_rules! handle {
    ($(#[$m:meta])* $name:ident<$($g:ident),+>) => {
        $(#[$m])*
        pub struct $name<$($g),+>(FieldIdx, PhantomData<fn() -> ($($g,)+)>);

        impl<$($g),+> $name<$($g),+> {
            /// Creates a handle for the field at `idx`.
            pub const fn new(idx: u16) -> Self {
                Self(FieldIdx(idx), PhantomData)
            }
            /// The field index.
            pub fn idx(self) -> FieldIdx {
                self.0
            }
        }
        impl<$($g),+> Clone for $name<$($g),+> {
            fn clone(&self) -> Self {
                *self
            }
        }
        impl<$($g),+> Copy for $name<$($g),+> {}
        impl<$($g),+> fmt::Debug for $name<$($g),+> {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.debug_tuple(stringify!($name)).field(&(self.0).0).finish()
            }
        }
    };
}

handle!(
    /// Handle to a scalar field of type `T` on resource `R`.
    Field<R, T>
);
handle!(
    /// Handle to a relation field targeting `T` on resource `R`.
    Rel<R, T>
);
handle!(
    /// Handle to an opaque field on resource `R`.
    Opaque<R>
);
handle!(
    /// Handle to any field on resource `R`.
    FieldRef<R>
);

impl<R, T> From<Field<R, T>> for FieldRef<R> {
    fn from(f: Field<R, T>) -> Self {
        FieldRef::new(f.idx().0)
    }
}
impl<R, T> From<Rel<R, T>> for FieldRef<R> {
    fn from(f: Rel<R, T>) -> Self {
        FieldRef::new(f.idx().0)
    }
}
impl<R> From<Opaque<R>> for FieldRef<R> {
    fn from(f: Opaque<R>) -> Self {
        FieldRef::new(f.idx().0)
    }
}
