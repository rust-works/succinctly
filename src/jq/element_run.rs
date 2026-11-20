//! The elements of a fold read from the front of their vector while its values are written into the
//! slots already read (#7412).
//!
//! `fold_pipe_stages`' `ManyCursor` arm folds the rest of a pipe over each element's cursor
//! (`[.strings[] | length]`), and once a long run of elements has each yielded one computed value or
//! nothing it moves those values into a vector of their own (#7267). The cursors stayed alive in
//! theirs until the fold ended, so its peak held both: 32 bytes a cursor beside 32 bytes a value, 2.4
//! MB of cursors for the 75,000 strings of a 10 MB file, which jq, iterating, never holds. A value
//! takes a slot only after the element it came from has been read out of it, and a JSON cursor and a
//! value have the same size and alignment, so [`ElementRun`] writes the values over the cursors' own
//! buffer and hands that buffer on as the vector of values. A cursor of another layout keeps its
//! values in a vector of their own, as before.
//!
//! The unsafe code of the move is confined to this module, behind a safe interface: every slot is
//! known to hold a value, an element, or nothing, and [`Drop`] drops exactly what is left.

#![allow(unsafe_code)] // a vector's buffer read from the front and written behind the reads (#7412)

#[cfg(not(test))]
use alloc::vec::Vec;
use core::marker::PhantomData;
use core::mem::{self, ManuallyDrop};
use core::ptr::{self, NonNull};

/// The elements of a fold, taken from the front, and the values it has computed: written over the
/// slots already read when an element and a value have the same layout ([`Self::IN_PLACE`]), and into a
/// vector of their own otherwise.
pub(crate) struct ElementRun<T, U> {
    /// The elements' buffer, as `Vec::into_raw_parts` would give it.
    ptr: NonNull<T>,
    len: usize,
    cap: usize,
    /// Slots `[read, len)` still hold elements.
    read: usize,
    /// In place, slots `[0, written)` hold values; `written <= read` always.
    written: usize,
    /// The values when they are not written in place.
    side: Vec<U>,
    _owns: PhantomData<(T, U)>,
}

impl<T, U> ElementRun<T, U> {
    /// Whether values are written over the elements' slots: only when a value fits a slot exactly
    /// and the buffer can be handed on as a `Vec<U>` (same size and alignment, so the same layout
    /// for the allocator), and neither is zero-sized.
    pub(crate) const IN_PLACE: bool = mem::size_of::<T>() == mem::size_of::<U>()
        && mem::align_of::<T>() == mem::align_of::<U>()
        && mem::size_of::<T>() != 0;

    pub(crate) fn new(elements: Vec<T>) -> Self {
        let mut elements = ManuallyDrop::new(elements);
        let (len, cap) = (elements.len(), elements.capacity());
        // A `Vec`'s pointer is never null, even for no allocation.
        let ptr = NonNull::new(elements.as_mut_ptr()).expect("a Vec's pointer is non-null");
        Self {
            ptr,
            len,
            cap,
            read: 0,
            written: 0,
            side: Vec::new(),
            _owns: PhantomData,
        }
    }

    /// The elements not read yet.
    #[inline(always)]
    pub(crate) fn left(&self) -> usize {
        self.len - self.read
    }

    /// The values held so far.
    #[cfg(test)]
    pub(crate) fn values_len(&self) -> usize {
        if Self::IN_PLACE {
            self.written
        } else {
            self.side.len()
        }
    }

    /// The next element.
    #[inline(always)]
    #[allow(clippy::should_implement_trait)]
    pub(crate) fn next(&mut self) -> Option<T> {
        if self.read == self.len {
            return None;
        }
        // SAFETY: `read < len`, so the slot is inside the buffer and still holds an element; it is
        // marked read before anything else can observe it, so it is moved out exactly once.
        let element = unsafe { ptr::read(self.ptr.as_ptr().add(self.read)) };
        self.read += 1;
        Some(element)
    }

    /// Makes room for `n` more values when they are kept on the side, as the fold's own vector was
    /// sized before #7412; in place the elements' slots are the room.
    pub(crate) fn reserve_values(&mut self, n: usize) {
        if !Self::IN_PLACE {
            self.side.reserve_exact(n);
        }
    }

    /// Adds a value. In place there must be a slot read and not yet written: the fold writes at most
    /// one value per element it has read.
    #[inline(always)]
    pub(crate) fn push_value(&mut self, value: U) {
        if Self::IN_PLACE {
            assert!(
                self.written < self.read,
                "a value is written only over a slot already read"
            );
            // SAFETY: `written < read <= len <= cap`, so the slot is inside the buffer; its element
            // was moved out by `next`, so writing does not leak or drop one. `IN_PLACE` makes the
            // slot's size and alignment those of a `U`.
            unsafe { ptr::write(self.ptr.as_ptr().cast::<U>().add(self.written), value) };
            self.written += 1;
        } else {
            self.side.push(value);
        }
    }

    /// The values so far, moved out into a vector with room for one value per element (what the
    /// fold's own vector had before #7412), for a fold that goes on without writing in place.
    ///
    /// By value, as the fold's other helpers take its vectors (#7267 review): a `&mut` to the run
    /// passed to a call that is not inlined lets its fields escape, and the fold's loop would reload
    /// them after every element's evaluation.
    #[inline(never)]
    pub(crate) fn take_values(mut self) -> (Self, Vec<U>) {
        if !Self::IN_PLACE {
            let side = mem::take(&mut self.side);
            return (self, side);
        }
        let mut values = Vec::with_capacity(self.len);
        // SAFETY: slots `[0, written)` hold values, and `values` has room for `written` of them; once
        // copied they are the vector's, and `written = 0` makes the slots dead so they are not
        // dropped again.
        unsafe {
            ptr::copy_nonoverlapping(
                self.ptr.as_ptr().cast::<U>(),
                values.as_mut_ptr(),
                self.written,
            );
            values.set_len(self.written);
        }
        self.written = 0;
        (self, values)
    }

    /// The values, as one vector; the elements not read are dropped. In place it is the elements'
    /// own buffer.
    pub(crate) fn into_values(self) -> Vec<U> {
        let mut this = ManuallyDrop::new(self);
        // SAFETY: slots `[read, len)` hold the elements not read, each dropped once here; `this` is
        // never dropped, so nothing else drops them.
        unsafe { this.drop_unread() };
        let side = mem::take(&mut this.side);
        if Self::IN_PLACE {
            // SAFETY: the buffer was a `Vec<T>` of capacity `cap`; `IN_PLACE` gives `U` the same
            // size and alignment, so it is a `Vec<U>` allocation of capacity `cap`, whose first
            // `written` slots hold values. `this` is never dropped, so the buffer is freed once.
            unsafe { Vec::from_raw_parts(this.ptr.as_ptr().cast::<U>(), this.written, this.cap) }
        } else {
            // SAFETY: the buffer was a `Vec<T>` of capacity `cap` whose elements are all moved out
            // or dropped; freed once, as `this` is never dropped.
            drop(unsafe { Vec::from_raw_parts(this.ptr.as_ptr(), 0, this.cap) });
            side
        }
    }

    /// Drops the elements not read.
    ///
    /// # Safety
    ///
    /// Called at most once, by whatever then frees the buffer without reading them again.
    unsafe fn drop_unread(&mut self) {
        if mem::needs_drop::<T>() && self.read < self.len {
            // SAFETY: by the caller, `[read, len)` still hold elements and are not dropped again.
            unsafe {
                ptr::drop_in_place(ptr::slice_from_raw_parts_mut(
                    self.ptr.as_ptr().add(self.read),
                    self.len - self.read,
                ));
            }
        }
        self.read = self.len;
    }
}

impl<T, U> Drop for ElementRun<T, U> {
    #[inline]
    fn drop(&mut self) {
        // SAFETY: the only drop of the elements not read; the buffer is freed just below.
        unsafe { self.drop_unread() };
        if Self::IN_PLACE && mem::needs_drop::<U>() {
            // SAFETY: slots `[0, written)` hold values, dropped only here.
            unsafe {
                ptr::drop_in_place(ptr::slice_from_raw_parts_mut(
                    self.ptr.as_ptr().cast::<U>(),
                    self.written,
                ));
            }
        }
        self.written = 0;
        // SAFETY: the buffer was a `Vec<T>` of capacity `cap`, every slot of which is now moved out
        // or dropped; this frees it once.
        drop(unsafe { Vec::from_raw_parts(self.ptr.as_ptr(), 0, self.cap) });
    }
}

#[cfg(test)]
mod tests {
    use super::ElementRun;
    use std::cell::Cell;
    use std::rc::Rc;

    /// A value that counts its drops, 16 bytes like the element it is paired with.
    #[derive(Debug)]
    struct Counted(Rc<Cell<usize>>, u64);
    impl Drop for Counted {
        fn drop(&mut self) {
            self.0.set(self.0.get() + 1);
        }
    }

    fn elements(n: usize, drops: &Rc<Cell<usize>>) -> Vec<Counted> {
        (0..n as u64).map(|i| Counted(drops.clone(), i)).collect()
    }

    // In place only for the same layout.
    const _: () = assert!(ElementRun::<Counted, Counted>::IN_PLACE);
    const _: () = assert!(ElementRun::<u64, i64>::IN_PLACE);
    const _: () = assert!(!ElementRun::<u32, u64>::IN_PLACE);
    const _: () = assert!(!ElementRun::<[u8; 8], u64>::IN_PLACE); // same size, smaller alignment
    const _: () = assert!(!ElementRun::<(), ()>::IN_PLACE);

    #[test]
    fn values_reuse_the_elements_buffer_7412() {
        let mut v: Vec<u64> = (0..1000).collect();
        v.reserve_exact(24);
        let (at, cap) = (v.as_ptr() as usize, v.capacity());
        let mut run = ElementRun::<u64, i64>::new(v);
        while let Some(x) = run.next() {
            if x % 3 != 0 {
                run.push_value(-(x as i64));
            }
        }
        assert_eq!(run.left(), 0);
        assert_eq!(run.values_len(), 666);
        let values = run.into_values();
        assert_eq!(values.as_ptr() as usize, at);
        assert_eq!(values.capacity(), cap);
        let want: Vec<i64> = (0..1000).filter(|x| x % 3 != 0).map(|x| -x).collect();
        assert_eq!(values, want);
    }

    #[test]
    fn a_different_layout_keeps_its_values_on_the_side_7412() {
        let v: Vec<u32> = (0..100).collect();
        let mut run = ElementRun::<u32, u64>::new(v);
        run.reserve_values(100);
        while let Some(x) = run.next() {
            run.push_value(u64::from(x) * 2);
        }
        let values = run.into_values();
        assert_eq!(values, (0..100).map(|x| x * 2).collect::<Vec<u64>>());
        assert!(values.capacity() >= 100);
    }

    #[test]
    #[should_panic(expected = "a value is written only over a slot already read")]
    fn a_value_needs_a_slot_already_read_7412() {
        let mut run = ElementRun::<u64, i64>::new(vec![1, 2]);
        let _ = run.next();
        run.push_value(1);
        run.push_value(2);
    }

    #[test]
    fn every_element_and_value_is_dropped_once_7412() {
        for stop in [0, 1, 5, 10] {
            for into_values in [false, true] {
                let drops = Rc::new(Cell::new(0));
                let values_drops = Rc::new(Cell::new(0));
                let mut run = ElementRun::<Counted, Counted>::new(elements(10, &drops));
                for _ in 0..stop {
                    let e = run.next().expect("an element");
                    if e.1 % 2 == 0 {
                        run.push_value(Counted(values_drops.clone(), e.1));
                    }
                }
                let kept = run.values_len();
                if into_values {
                    let values = run.into_values();
                    assert_eq!(values.len(), kept);
                    assert_eq!(drops.get(), 10, "stop {stop}");
                    drop(values);
                } else {
                    drop(run);
                    assert_eq!(drops.get(), 10, "stop {stop}");
                }
                assert_eq!(values_drops.get(), kept, "stop {stop}");
            }
        }
    }

    #[test]
    fn taken_values_leave_the_rest_to_read_7412() {
        let drops = Rc::new(Cell::new(0));
        let values_drops = Rc::new(Cell::new(0));
        let mut run = ElementRun::<Counted, Counted>::new(elements(8, &drops));
        for _ in 0..3 {
            let e = run.next().expect("an element");
            run.push_value(Counted(values_drops.clone(), e.1 + 100));
        }
        let (mut run, taken) = run.take_values();
        assert_eq!(
            taken.iter().map(|c| c.1).collect::<Vec<_>>(),
            [100, 101, 102]
        );
        assert_eq!(taken.capacity(), 8);
        assert_eq!(run.values_len(), 0);
        assert_eq!(run.next().map(|c| c.1), Some(3));
        assert_eq!(run.left(), 4);
        drop(run);
        assert_eq!(drops.get(), 8);
        assert_eq!(values_drops.get(), 0);
        drop(taken);
        assert_eq!(values_drops.get(), 3);
    }

    #[test]
    fn a_panic_mid_fold_drops_what_is_held_7412() {
        let drops = Rc::new(Cell::new(0));
        let values_drops = Rc::new(Cell::new(0));
        let (d, vd) = (drops.clone(), values_drops.clone());
        let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
            let mut run = ElementRun::<Counted, Counted>::new(elements(6, &d));
            while let Some(e) = run.next() {
                assert!(e.1 != 3, "stop");
                run.push_value(Counted(vd.clone(), e.1));
            }
        }));
        assert!(caught.is_err());
        assert_eq!(drops.get(), 6);
        assert_eq!(values_drops.get(), 3);
    }
}
