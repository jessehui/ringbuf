use super::{macros::rb_impl_init, utils::ranges};
#[cfg(feature = "alloc")]
use crate::traits::Split;
use crate::{
    storage::{Shared, Static, Storage},
    traits::{Consumer, Observer, Producer, RingBuffer, SplitRef},
    wrap::{Cons, Prod},
};
#[cfg(feature = "alloc")]
use alloc::rc::Rc;
use core::{
    cell::Cell,
    fmt,
    mem::{ManuallyDrop, MaybeUninit},
    num::NonZeroUsize,
    ptr,
};
#[cfg(feature = "std")]
use std::io;

struct End {
    index: Cell<usize>,
    held: Cell<bool>,
}

impl End {
    fn new(index: usize) -> Self {
        Self {
            index: Cell::new(index),
            held: Cell::new(false),
        }
    }
}

/// Ring buffer for single-threaded use only.
pub struct LocalRb<S: Storage> {
    storage: Shared<S>,
    read: End,
    write: End,
}

impl<S: Storage> LocalRb<S> {
    /// Constructs ring buffer from storage and indices.
    ///
    /// # Safety
    ///
    /// The items in storage inside `read..write` range must be initialized, items outside this range must be uninitialized.
    /// `read` and `write` positions must be valid (see [`RbBase`](`crate::ring_buffer::RbBase`)).
    pub unsafe fn from_raw_parts(storage: S, read: usize, write: usize) -> Self {
        Self {
            storage: Shared::new(storage),
            read: End::new(read),
            write: End::new(write),
        }
    }
    /// Destructures ring buffer into underlying storage and `read` and `write` indices.
    ///
    /// # Safety
    ///
    /// Initialized contents of the storage must be properly dropped.
    pub unsafe fn into_raw_parts(self) -> (S, usize, usize) {
        let this = ManuallyDrop::new(self);
        (ptr::read(&this.storage).into_inner(), this.read_index(), this.write_index())
    }
}

impl<S: Storage> Observer for LocalRb<S> {
    type Item = S::Item;

    #[inline]
    fn capacity(&self) -> NonZeroUsize {
        self.storage.len()
    }

    #[inline]
    fn read_index(&self) -> usize {
        self.read.index.get()
    }
    #[inline]
    fn write_index(&self) -> usize {
        self.write.index.get()
    }

    unsafe fn unsafe_slices(&self, start: usize, end: usize) -> (&mut [MaybeUninit<S::Item>], &mut [MaybeUninit<S::Item>]) {
        let (first, second) = ranges(self.capacity(), start, end);
        (self.storage.slice(first), self.storage.slice(second))
    }

    #[inline]
    fn read_is_held(&self) -> bool {
        self.read.held.get()
    }
    #[inline]
    fn write_is_held(&self) -> bool {
        self.write.held.get()
    }
}

impl<S: Storage> Producer for LocalRb<S> {
    #[inline]
    unsafe fn set_write_index(&self, value: usize) {
        self.write.index.set(value);
    }
}

impl<S: Storage> Consumer for LocalRb<S> {
    #[inline]
    unsafe fn set_read_index(&self, value: usize) {
        self.read.index.set(value);
    }
}

impl<S: Storage> RingBuffer for LocalRb<S> {
    #[inline]
    unsafe fn hold_read(&self, flag: bool) {
        self.read.held.set(flag)
    }
    #[inline]
    unsafe fn hold_write(&self, flag: bool) {
        self.write.held.set(flag)
    }
}

impl<S: Storage> Drop for LocalRb<S> {
    fn drop(&mut self) {
        self.clear();
    }
}

#[cfg(feature = "alloc")]
impl<S: Storage> Split for LocalRb<S> {
    type Prod = Prod<Rc<Self>>;
    type Cons = Cons<Rc<Self>>;

    fn split(self) -> (Self::Prod, Self::Cons) {
        let rc = Rc::new(self);
        (Prod::new(rc.clone()), Cons::new(rc))
    }
}
impl<S: Storage> SplitRef for LocalRb<S> {
    type RefProd<'a> = Prod<&'a Self> where Self: 'a;
    type RefCons<'a> = Cons<&'a Self> where Self: 'a;

    fn split_ref(&mut self) -> (Self::RefProd<'_>, Self::RefCons<'_>) {
        (Prod::new(self), Cons::new(self))
    }
}

rb_impl_init!(LocalRb);

#[cfg(feature = "std")]
impl<S: Storage<Item = u8>> io::Write for LocalRb<S> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        <Self as Producer>::write(self, buf)
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
impl<S: Storage<Item = u8>> fmt::Write for LocalRb<S> {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        <Self as Producer>::write_str(self, s)
    }
}

#[cfg(feature = "std")]
impl<S: Storage<Item = u8>> io::Read for LocalRb<S> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        <Self as Consumer>::read(self, buf)
    }
}
