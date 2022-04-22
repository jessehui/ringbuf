use alloc::{sync::Arc, vec::Vec};
use cache_padded::CachePadded;
use core::{
    cell::UnsafeCell,
    convert::{AsMut, AsRef},
    marker::PhantomData,
    mem::MaybeUninit,
    sync::atomic::{AtomicUsize, Ordering},
};

use crate::{
    consumer::{ArcConsumer, RefConsumer},
    producer::{ArcProducer, RefProducer},
};

pub trait Storage<U> {
    fn len(&self) -> usize;
    unsafe fn as_slice(&self) -> &[U];
    unsafe fn as_mut_slice(&mut self) -> &mut [U];
}

pub trait Container<U>: AsRef<[U]> + AsMut<[U]> {}
impl<U, C> Container<U> for C where C: AsRef<[U]> + AsMut<[U]> {}

struct ContainerStorage<U, C: Container<U>> {
    len: usize,
    container: UnsafeCell<C>,
    phantom: PhantomData<U>,
}

unsafe impl<U, C: Container<U>> Sync for ContainerStorage<U, C> {}

impl<U, C> ContainerStorage<U, C>
where
    C: AsRef<[U]> + AsMut<[U]>,
{
    pub fn new(mut container: C) -> Self {
        Self {
            len: container.as_mut().len(),
            container: UnsafeCell::new(container),
            phantom: PhantomData,
        }
    }

    pub fn into_inner(self) -> C {
        self.container.into_inner()
    }
}

impl<U, C: Container> Storage<T> for ContainerStorage<U, C> {
    fn len(&self) -> usize {
        self.len
    }

    unsafe fn as_slice(&self) -> &[U] {
        (&*self.container.get()).as_ref()
    }

    unsafe fn as_mut_slice(&mut self) -> &mut [U] {
        (&mut *self.container.get()).as_mut()
    }
}

pub struct RingBuffer<T, C: Container<MaybeUninit<T>>> {
    pub(crate) data: ContainerStorage<MaybeUninit<T>, C>,
    pub(crate) head: CachePadded<AtomicUsize>,
    pub(crate) tail: CachePadded<AtomicUsize>,
}

//pub type StaticRingBuffer<T, const N: usize> = RingBuffer<T, [MaybeUninit<T>; N]>;
//pub type HeapRingBuffer<T> = RingBuffer<T, Vec<MaybeUninit<T>>>;

impl<T> RingBuffer<T, Vec<MaybeUninit<T>>> {
    pub fn new(capacity: usize) -> Self {
        let mut data = Vec::new();
        data.resize_with(capacity + 1, MaybeUninit::uninit);
        unsafe { Self::from_raw_parts(data, 0, 0) }
    }
}

impl<T, const N: usize> Default for RingBuffer<T, [MaybeUninit<T>; N]> {
    fn default() -> Self {
        let uninit = MaybeUninit::<[T; N]>::uninit();
        let array = unsafe { (&uninit as *const _ as *const [MaybeUninit<T>; N]).read() };
        unsafe { Self::from_raw_parts(array, 0, 0) }
    }
}

impl<T, C: Container<MaybeUninit<T>>> RingBuffer<T, C> {
    pub unsafe fn from_raw_parts(container: C, head: usize, tail: usize) -> Self {
        Self {
            data: ContainerStorage::new(container),
            head: CachePadded::new(AtomicUsize::new(head)),
            tail: CachePadded::new(AtomicUsize::new(tail)),
        }
    }

    /// Splits ring buffer into producer and consumer.
    pub fn split(self) -> (ArcProducer<T, C>, ArcConsumer<T, C>) {
        let arc = Arc::new(self);
        (ArcProducer { rb: arc.clone() }, ArcConsumer { rb: arc })
    }

    pub fn split_ref(&mut self) -> (RefProducer<'_, T, C>, RefConsumer<'_, T, C>) {
        (RefProducer { rb: self }, RefConsumer { rb: self })
    }

    /// Returns capacity of the ring buffer.
    pub fn capacity(&self) -> usize {
        self.data.len() - 1
    }

    /// Checks if the ring buffer is empty.
    pub fn is_empty(&self) -> bool {
        let head = self.head.load(Ordering::Acquire);
        let tail = self.tail.load(Ordering::Acquire);
        head == tail
    }

    /// Checks if the ring buffer is full.
    pub fn is_full(&self) -> bool {
        let head = self.head.load(Ordering::Acquire);
        let tail = self.tail.load(Ordering::Acquire);
        (tail + 1) % self.data.len() == head
    }

    /// The length of the data in the buffer.
    pub fn len(&self) -> usize {
        let head = self.head.load(Ordering::Acquire);
        let tail = self.tail.load(Ordering::Acquire);
        (tail + self.data.len() - head) % self.data.len()
    }

    /// The remaining space in the buffer.
    pub fn remaining(&self) -> usize {
        self.capacity() - self.len()
    }
}

impl<T, C: Container<MaybeUninit<T>>> Drop for RingBuffer<T, C> {
    fn drop(&mut self) {
        let data = unsafe { self.data.as_mut_slice() };

        let head = self.head.load(Ordering::Acquire);
        let tail = self.tail.load(Ordering::Acquire);
        let len = data.len();

        let slices = if head <= tail {
            (head..tail, 0..0)
        } else {
            (head..len, 0..tail)
        };

        let drop = |elem_ref: &mut MaybeUninit<T>| unsafe {
            elem_ref.as_ptr().read();
        };
        for elem in data[slices.0].iter_mut() {
            drop(elem);
        }
        for elem in data[slices.1].iter_mut() {
            drop(elem);
        }
    }
}

/*
/// Moves at most `count` items from the `src` consumer to the `dst` producer.
/// Consumer and producer may be of different buffers as well as of the same one.
///
/// `count` is the number of items being moved, if `None` - as much as possible items will be moved.
///
/// Returns number of items been moved.
pub fn move_items<T>(src: &mut Consumer<T>, dst: &mut Producer<T>, count: Option<usize>) -> usize {
    unsafe {
        src.pop_access(|src_left, src_right| -> usize {
            dst.push_access(|dst_left, dst_right| -> usize {
                let n = count.unwrap_or_else(|| {
                    min(
                        src_left.len() + src_right.len(),
                        dst_left.len() + dst_right.len(),
                    )
                });
                let mut m = 0;
                let mut src = (SlicePtr::new(src_left), SlicePtr::new(src_right));
                let mut dst = (SlicePtr::new(dst_left), SlicePtr::new(dst_right));

                loop {
                    let k = min(n - m, min(src.0.len, dst.0.len));
                    if k == 0 {
                        break;
                    }
                    copy(src.0.ptr, dst.0.ptr, k);
                    if src.0.len == k {
                        src.0 = src.1;
                        src.1 = SlicePtr::null();
                    } else {
                        src.0.shift(k);
                    }
                    if dst.0.len == k {
                        dst.0 = dst.1;
                        dst.1 = SlicePtr::null();
                    } else {
                        dst.0.shift(k);
                    }
                    m += k
                }

                m
            })
        })
    }
}
*/
