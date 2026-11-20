//! The storage behind [`ArrayOf`](super::ArrayOf): one heap block holding a
//! reference count, the element count, the capacity and the elements
//! (#7260).
//!
//! # Why not `Rc<Vec<V>>`
//!
//! `Rc<Vec<V>>` is two heap blocks per array: the `Rc`'s 40 bytes (two counts
//! and the `Vec`'s three words, a 48-byte glibc chunk) and the `Vec`'s
//! buffer. An array-heavy result pays the first block once per array: on
//! `[tostream]` over a 1 MB document every event is two arrays (the event and
//! its path), and the `Rc` blocks alone were 5.1 MiB of a 35.9 MiB peak heap,
//! about the whole gap to jq 1.7.1's 30.7 MiB. jq's own arrays are one block
//! (`jvp_array`: count, sizes, elements); this is the same shape.
//!
//! # Layout
//!
//! A [`Header`] of three words (`strong`, `len`, `bytes`), then as many
//! slots of `V` as fit in `bytes`, of which the first `len` are initialized.
//! Three words, not two packed ones: an element is 32 bytes, so a block
//! requests 24 + 32n bytes, which glibc and macOS's allocator round to the
//! same bin as a 16-byte header would; a narrower header would only cap the
//! length.
//!
//! The header records the allocation's size in bytes rather than a slot
//! count, so a `Vec` whose buffer has a spare slot becomes a block *in
//! place*: its elements move up past the header and the allocation is
//! untouched (its size is no longer a whole number of slots past the
//! header, which only the byte count can say). Shrinking it instead cost a
//! transient second copy of a large array on macOS, whose `realloc` moves a
//! large block to shrink it (`walk(.)` over 100 MB of records: +21 MB peak
//! RSS on an M4 Pro).
//!
//! A `Vec` of 16 KiB or more becomes an *external* block instead: a 40-byte
//! block with the header and the `Vec`'s buffer pointer and capacity, its
//! elements left where they are (`EXTERNAL_MIN_BYTES`). Moving them up past
//! a header would copy the whole array, and one more small block per large
//! array is free. `bytes` says which kind a block is; every method below
//! works on both, and the external buffer converts back to its `Vec` as it
//! stands.
//!
//! There is no weak count (nothing ever made a `Weak` of the `Rc`), and the
//! window (`start`/`len` of [`ArrayOf`](super::ArrayOf)) stays in the handle:
//! the block only knows its own elements, all of which the last handle drops.
//!
//! # Safety
//!
//! [`Block`] is a counted handle like `Rc`: cloning bumps `strong`, dropping
//! the last handle drops the elements and frees the block. Everything that
//! writes the elements or their count goes through [`UniqueBlock`], which
//! [`Block::get_mut`] hands out only while `strong == 1`, so a write can
//! never be seen through another handle -- `Rc::get_mut`'s contract. Every
//! operation that changes the element count sets `len` to a value whose
//! prefix is initialized *before* it can run a destructor or a caller's
//! closure, so a panic there leaks elements but never drops one twice or
//! reads one uninitialized.

#![allow(unsafe_code)] // a refcounted vector in one block (#7260); see the module docs

use alloc::alloc::{alloc, dealloc, handle_alloc_error, realloc, Layout};
use alloc::vec::Vec;
use core::cell::Cell;
use core::marker::PhantomData;
use core::mem::{self, ManuallyDrop};
use core::ops::{Deref, DerefMut, Range};
use core::panic::{RefUnwindSafe, UnwindSafe};
use core::ptr::{self, NonNull};

/// The three words in front of a block's elements.
#[repr(C)]
struct Header {
    /// Handles on this block. Shared handles bump it, hence the `Cell`.
    strong: Cell<usize>,
    /// Initialized elements, a prefix of the slots. Written only through a
    /// unique handle.
    len: usize,
    /// The allocation's size, header included. Written only through a
    /// unique handle.
    bytes: usize,
}

/// [`Header::bytes`] of an *external* block: one whose elements stay in the
/// buffer of the `Vec` it was made from ([`ExternalBlock`]).
const EXTERNAL: usize = usize::MAX;

/// The smallest `Vec` (in bytes of elements) that becomes an external block
/// rather than moving its elements up past a header. Below it the move is a
/// few cache lines; above it, it would be a copy of the whole array (a
/// 100 MB `[.numbers[] | floor]` moved 320 MB, +1.9% wall-clock on a Ryzen 9
/// 7950X), and one more 40-byte block per large array costs nothing.
const EXTERNAL_MIN_BYTES: usize = 16 * 1024;

/// A block whose elements live in a `Vec`'s buffer elsewhere: the header,
/// then that buffer and its capacity.
#[repr(C)]
struct ExternalBlock {
    header: Header,
    /// The buffer (`Vec::as_mut_ptr`).
    buffer: *mut u8,
    /// The buffer's capacity in elements (`Vec::capacity`).
    cap: usize,
}

/// The layout of an [`ExternalBlock`] for elements of `V`.
#[inline]
fn external_layout<V>() -> Layout {
    Layout::from_size_align(mem::size_of::<ExternalBlock>(), block_align::<V>())
        .expect("a header and two words fit")
}

/// Bytes from the start of a block to its first element.
#[inline]
const fn elements_offset<V>() -> usize {
    let align = mem::align_of::<V>();
    (mem::size_of::<Header>() + align - 1) & !(align - 1)
}

/// The alignment of a block: the header's or the element's, whichever is
/// stricter.
#[inline]
const fn block_align<V>() -> usize {
    let (header, element) = (mem::align_of::<Header>(), mem::align_of::<V>());
    if element > header {
        element
    } else {
        header
    }
}

/// The layout of a block with `cap` slots, or `None` if it does not fit in
/// an `isize`.
#[inline]
fn block_layout<V>(cap: usize) -> Option<Layout> {
    let bytes = cap
        .checked_mul(mem::size_of::<V>())?
        .checked_add(elements_offset::<V>())?;
    Layout::from_size_align(bytes, block_align::<V>()).ok()
}

/// Whether a `Vec<V>`'s buffer can be grown into a block in place: its
/// alignment must already be the block's, so `realloc` (which keeps the
/// alignment) yields a block.
#[inline]
const fn vec_buffer_is_block_aligned<V>() -> bool {
    mem::size_of::<V>() != 0 && mem::align_of::<V>() >= mem::align_of::<Header>()
}

#[cold]
#[inline(never)]
#[track_caller]
fn capacity_overflow() -> ! {
    panic!("capacity overflow");
}

/// Why [`UniqueBlock::try_reserve`] could not reserve: the size does not fit,
/// or the allocator refused it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArrayReserveError;

impl core::fmt::Display for ArrayReserveError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("memory allocation failed")
    }
}

/// One counted handle on a block. Dropping the last one drops the elements
/// and frees the block.
pub(crate) struct Block<V> {
    ptr: NonNull<Header>,
    _owns: PhantomData<V>,
}

// `Rc<Vec<V>>` was `UnwindSafe` for `V: RefUnwindSafe` (std says so
// explicitly); the `Cell` in the header would otherwise take it away from
// every `OwnedValue`. Like `Rc`, no `RefUnwindSafe`, `Send` or `Sync`.
impl<V: RefUnwindSafe> UnwindSafe for Block<V> {}

impl<V> Block<V> {
    /// A new block with room for `cap` elements and none yet.
    #[inline]
    pub(crate) fn with_capacity(cap: usize) -> Self {
        let Some(layout) = block_layout::<V>(cap) else {
            capacity_overflow()
        };
        // SAFETY: the layout has a non-zero size (the header).
        let raw = unsafe { alloc(layout) };
        let Some(ptr) = NonNull::new(raw.cast::<Header>()) else {
            handle_alloc_error(layout)
        };
        // SAFETY: freshly allocated, aligned for `Header`.
        unsafe {
            ptr.as_ptr().write(Header {
                strong: Cell::new(1),
                len: 0,
                bytes: layout.size(),
            });
        }
        Self {
            ptr,
            _owns: PhantomData,
        }
    }

    /// A block holding `vec`'s elements. The `Vec`'s buffer becomes the block
    /// where its alignment allows, so a large array is never copied into a
    /// second buffer beside the first: with a spare slot's room for the header
    /// the elements just move up past it, and an exactly sized buffer grows by
    /// the header (`realloc`) first.
    pub(crate) fn from_vec(vec: Vec<V>) -> Self {
        let len = vec.len();
        if mem::size_of::<V>() != 0 && len.saturating_mul(mem::size_of::<V>()) >= EXTERNAL_MIN_BYTES
        {
            return Self::external(vec);
        }
        if !vec_buffer_is_block_aligned::<V>() || vec.capacity() == 0 {
            let mut block = Self::with_capacity(len);
            let mut vec = ManuallyDrop::new(vec);
            // SAFETY: the block has `len` free slots; the `Vec`'s elements
            // are moved (its length is zeroed before its buffer is freed).
            unsafe {
                ptr::copy_nonoverlapping(vec.as_ptr(), block.elements(), len);
                vec.set_len(0);
                ManuallyDrop::drop(&mut vec);
                block.header_mut().len = len;
            }
            return block;
        }
        let mut vec = ManuallyDrop::new(vec);
        let cap = vec.capacity();
        let buffer = vec.as_mut_ptr().cast::<u8>();
        // SAFETY: `buffer` was allocated by the global allocator with
        // `Layout::array::<V>(cap)`, whose size and alignment are these;
        // `cap > 0` and `V` is not zero-sized, so it is a real allocation.
        let old = unsafe {
            Layout::from_size_align_unchecked(cap * mem::size_of::<V>(), mem::align_of::<V>())
        };
        let Some(needed) = block_layout::<V>(len) else {
            capacity_overflow()
        };
        debug_assert_eq!(needed.align(), old.align());
        let (raw, bytes) = if needed.size() <= old.size() {
            // The header fits in the spare slots: the buffer is the block.
            (buffer, old.size())
        } else {
            // SAFETY: `old` is the buffer's layout; `needed.size()` is
            // non-zero and fits an `isize` (checked by `block_layout`), and
            // grows it, so every element survives.
            let raw = unsafe { realloc(buffer, old, needed.size()) };
            if raw.is_null() {
                handle_alloc_error(needed)
            }
            (raw, needed.size())
        };
        // SAFETY: non-null, from the allocator or the `Vec`.
        let ptr = unsafe { NonNull::new_unchecked(raw.cast::<Header>()) };
        // SAFETY: the block holds the `len` elements at its start; they move
        // up past the header (overlapping), then the header is written into
        // the bytes they left.
        unsafe {
            ptr::copy(
                raw.cast::<V>(),
                raw.add(elements_offset::<V>()).cast::<V>(),
                len,
            );
            ptr.as_ptr().write(Header {
                strong: Cell::new(1),
                len,
                bytes,
            });
        }
        Self {
            ptr,
            _owns: PhantomData,
        }
    }

    /// A block of exactly `items`, moved in at once.
    #[inline]
    pub(crate) fn from_array<const N: usize>(items: [V; N]) -> Self {
        let mut block = Self::with_capacity(N);
        // SAFETY: a new inline block with `N` free slots; the array is moved
        // in whole and then counted.
        unsafe {
            block.elements().cast::<[V; N]>().write(items);
            block.header_mut().len = N;
        }
        block
    }

    /// A block of clones of `items`, exactly sized.
    #[inline]
    pub(crate) fn from_slice(items: &[V]) -> Self
    where
        V: Clone,
    {
        let block = Self::with_capacity(items.len());
        let base = block.elements();
        let mut counted = CountOnDrop {
            header: block.ptr,
            len: 0,
        };
        for (at, item) in items.iter().enumerate() {
            // SAFETY: slot `at < items.len()` is allocated and free; the
            // count covers it once it is written, also if a later clone
            // panics.
            unsafe { base.add(at).write(item.clone()) };
            counted.len = at + 1;
        }
        drop(counted);
        block
    }

    /// An external block over `vec`'s buffer, which it takes as it stands.
    #[cold]
    #[inline(never)]
    fn external(vec: Vec<V>) -> Self {
        let layout = external_layout::<V>();
        // SAFETY: a non-zero size.
        let raw = unsafe { alloc(layout) };
        let Some(ptr) = NonNull::new(raw.cast::<Header>()) else {
            handle_alloc_error(layout)
        };
        let mut vec = ManuallyDrop::new(vec);
        // SAFETY: freshly allocated and aligned for `ExternalBlock`; the
        // buffer is owned by the block from here on.
        unsafe {
            ptr.as_ptr().cast::<ExternalBlock>().write(ExternalBlock {
                header: Header {
                    strong: Cell::new(1),
                    len: vec.len(),
                    bytes: EXTERNAL,
                },
                buffer: vec.as_mut_ptr().cast::<u8>(),
                cap: vec.capacity(),
            });
        }
        Self {
            ptr,
            _owns: PhantomData,
        }
    }

    /// Whether the elements live in a separate buffer ([`ExternalBlock`]).
    #[inline]
    fn is_external(&self) -> bool {
        self.header().bytes == EXTERNAL
    }

    /// An external block's buffer and capacity. Only for an external block.
    #[inline]
    fn external_parts(&self) -> (*mut V, usize) {
        debug_assert!(self.is_external());
        // SAFETY: an external block is an `ExternalBlock`.
        let block = unsafe { &*self.ptr.as_ptr().cast::<ExternalBlock>() };
        (block.buffer.cast::<V>(), block.cap)
    }

    #[inline]
    fn header(&self) -> &Header {
        // SAFETY: the block is live while a handle is.
        unsafe { self.ptr.as_ref() }
    }

    /// The header, for a write. Only a unique handle may call it.
    #[inline]
    unsafe fn header_mut(&mut self) -> &mut Header {
        self.ptr.as_mut()
    }

    /// The first element slot.
    #[inline]
    fn elements(&self) -> *mut V {
        if self.is_external() {
            return self.external_parts().0;
        }
        // SAFETY: the offset is within the block (its header's size,
        // rounded to the element's alignment, which the layout includes).
        unsafe {
            self.ptr
                .as_ptr()
                .cast::<u8>()
                .add(elements_offset::<V>())
                .cast::<V>()
        }
    }

    /// The block's address, for identity.
    #[cfg(test)]
    pub(crate) fn as_ptr(&self) -> *const () {
        self.ptr.as_ptr().cast_const().cast::<()>()
    }

    /// Whether two handles are on one block (`Rc::ptr_eq`).
    #[inline]
    pub(crate) fn ptr_eq(&self, other: &Self) -> bool {
        self.ptr == other.ptr
    }

    /// Handles on this block (`Rc::strong_count`).
    #[inline]
    pub(crate) fn strong_count(&self) -> usize {
        self.header().strong.get()
    }

    /// Initialized elements in the block (not the window).
    #[inline]
    pub(crate) fn len(&self) -> usize {
        self.header().len
    }

    /// Whether the block holds no element.
    #[inline]
    pub(crate) fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Slots the block has.
    #[inline]
    pub(crate) fn capacity(&self) -> usize {
        if self.is_external() {
            return self.external_parts().1;
        }
        match mem::size_of::<V>() {
            0 => usize::MAX,
            size => (self.header().bytes - elements_offset::<V>()) / size,
        }
    }

    /// The layout the block was allocated (or last reallocated) with.
    #[inline]
    fn layout(&self) -> Layout {
        if self.is_external() {
            return external_layout::<V>();
        }
        // SAFETY: the size and alignment of a live allocation.
        unsafe { Layout::from_size_align_unchecked(self.header().bytes, block_align::<V>()) }
    }

    /// The block's initialized elements.
    #[inline]
    pub(crate) fn as_slice(&self) -> &[V] {
        // SAFETY: the first `len` slots are initialized, and nothing writes
        // them while this shared borrow lives (writes need a unique handle,
        // which `&self` borrows).
        unsafe { core::slice::from_raw_parts(self.elements(), self.len()) }
    }

    /// Write access, if this is the only handle (`Rc::get_mut`).
    #[inline]
    pub(crate) fn get_mut(&mut self) -> Option<UniqueBlock<'_, V>> {
        if self.strong_count() == 1 {
            Some(UniqueBlock { block: self })
        } else {
            None
        }
    }

    /// The elements as a `Vec`, moved, if this is the only handle; the
    /// handle back otherwise. Reuses the block's buffer where
    /// [`from_vec`](Self::from_vec) would have.
    pub(crate) fn try_into_vec(self) -> Result<Vec<V>, Self> {
        if self.strong_count() != 1 {
            return Err(self);
        }
        let this = ManuallyDrop::new(self);
        let len = this.len();
        let raw = this.ptr.as_ptr().cast::<u8>();
        let layout = this.layout();
        if this.is_external() {
            let (buffer, cap) = this.external_parts();
            // SAFETY: the buffer and capacity of the `Vec` the block was made
            // from (or grown since), with `len` initialized elements; the
            // block itself is freed without touching them.
            unsafe {
                dealloc(raw, layout);
                return Ok(Vec::from_raw_parts(buffer, len, cap));
            }
        }
        // The most slots a `Vec` can say the allocation holds (it has no
        // header), at least `len`.
        let cap = match mem::size_of::<V>() {
            0 => 0,
            size => layout.size() / size,
        };
        if !vec_buffer_is_block_aligned::<V>() || cap == 0 {
            let mut vec = Vec::with_capacity(len);
            // SAFETY: the elements move into the new buffer; the block is
            // then freed without dropping them.
            unsafe {
                ptr::copy_nonoverlapping(this.elements(), vec.as_mut_ptr(), len);
                vec.set_len(len);
                dealloc(raw, layout);
            }
            return Ok(vec);
        }
        let bytes = cap * mem::size_of::<V>();
        // SAFETY: the elements move down to the start of the block, over the
        // header (overlapping); the block then has, or shrinks to, exactly
        // the `Layout::array::<V>(cap)` a `Vec` of capacity `cap` frees with
        // (same alignment, `bytes > 0`).
        unsafe {
            ptr::copy(this.elements(), raw.cast::<V>(), len);
            let buffer = if bytes == layout.size() {
                raw
            } else {
                let shrunk = realloc(raw, layout, bytes);
                if shrunk.is_null() {
                    handle_alloc_error(Layout::from_size_align_unchecked(bytes, layout.align()));
                }
                shrunk
            };
            Ok(Vec::from_raw_parts(buffer.cast::<V>(), len, cap))
        }
    }

    /// Drop the elements and free the block: the last handle's release.
    #[inline(never)]
    fn drop_slow(&mut self) {
        /// Frees the block even if an element's destructor panics.
        struct Free(*mut u8, Layout);
        impl Drop for Free {
            fn drop(&mut self) {
                // SAFETY: the block's own pointer and layout, freed once.
                unsafe { dealloc(self.0, self.1) };
            }
        }
        let _free = Free(self.ptr.as_ptr().cast::<u8>(), self.layout());
        let len = self.len();
        if self.is_external() {
            let (buffer, cap) = self.external_parts();
            // SAFETY: as `try_into_vec`'s; the `Vec` drops the elements and
            // frees its buffer (even if a destructor panics).
            unsafe {
                self.header_mut().len = 0;
                drop(Vec::from_raw_parts(buffer, len, cap));
            }
            return;
        }
        // SAFETY: this is the last handle; the count is zeroed first so a
        // destructor that reaches this block again sees no element.
        unsafe {
            self.header_mut().len = 0;
            ptr::drop_in_place(ptr::slice_from_raw_parts_mut(self.elements(), len));
        }
    }
}

impl<V> Clone for Block<V> {
    #[inline]
    fn clone(&self) -> Self {
        let strong = &self.header().strong;
        // A count can only reach `usize::MAX` through `mem::forget`; refuse
        // to wrap it, as `Rc` does.
        let Some(next) = strong.get().checked_add(1) else {
            capacity_overflow()
        };
        strong.set(next);
        Self {
            ptr: self.ptr,
            _owns: PhantomData,
        }
    }
}

impl<V> Drop for Block<V> {
    #[inline]
    fn drop(&mut self) {
        let strong = &self.header().strong;
        let left = strong.get() - 1;
        strong.set(left);
        if left == 0 {
            self.drop_slow();
        }
    }
}

/// Writes a fresh block's element count when it drops: the count of slots
/// written so far, so a panic part-way through filling the block drops
/// exactly those.
struct CountOnDrop {
    header: NonNull<Header>,
    len: usize,
}

impl Drop for CountOnDrop {
    #[inline]
    fn drop(&mut self) {
        // SAFETY: the header of a block only its filler holds.
        unsafe { (*self.header.as_ptr()).len = self.len };
    }
}

/// Write access to a block that has no other handle: the `Vec` methods an
/// array write needs, and a `[V]` of all its elements.
pub(crate) struct UniqueBlock<'a, V> {
    block: &'a mut Block<V>,
}

impl<'a, V> UniqueBlock<'a, V> {
    #[inline]
    fn elements(&self) -> *mut V {
        self.block.elements()
    }

    /// Set the element count. The first `len` slots must be initialized.
    #[inline]
    unsafe fn set_len(&mut self, len: usize) {
        self.block.header_mut().len = len;
    }

    /// Initialized elements.
    #[inline]
    pub(crate) fn len(&self) -> usize {
        self.block.len()
    }

    /// Whether there is no element.
    #[inline]
    pub(crate) fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Slots allocated.
    #[inline]
    pub(crate) fn capacity(&self) -> usize {
        self.block.capacity()
    }

    /// Run `write` on an external block's buffer as the `Vec` it is, putting
    /// its buffer, capacity and length back afterwards (also if `write`
    /// panics, which a `Vec` method leaves consistent).
    fn with_vec<R>(&mut self, write: impl FnOnce(&mut Vec<V>) -> R) -> R {
        struct PutBack<'b, V> {
            vec: ManuallyDrop<Vec<V>>,
            block: &'b mut Block<V>,
        }
        impl<V> Drop for PutBack<'_, V> {
            fn drop(&mut self) {
                // SAFETY: the block is external and this is its only handle.
                unsafe {
                    let block = &mut *self.block.ptr.as_ptr().cast::<ExternalBlock>();
                    block.buffer = self.vec.as_mut_ptr().cast::<u8>();
                    block.cap = self.vec.capacity();
                    block.header.len = self.vec.len();
                }
            }
        }
        let (buffer, cap) = self.block.external_parts();
        // SAFETY: the buffer, length and capacity of a `Vec`, lent out once.
        let vec = unsafe { Vec::from_raw_parts(buffer, self.len(), cap) };
        let mut lent = PutBack {
            vec: ManuallyDrop::new(vec),
            block: &mut *self.block,
        };
        write(&mut lent.vec)
    }

    /// Resize the allocation to exactly `cap` slots (`cap >= len`).
    fn grow_to(&mut self, cap: usize) -> Result<(), ArrayReserveError> {
        debug_assert!(cap >= self.len());
        if self.block.is_external() {
            return self.with_vec(|vec| {
                if cap < vec.capacity() {
                    vec.shrink_to(cap);
                    Ok(())
                } else {
                    vec.try_reserve_exact(cap - vec.len())
                        .map_err(|_| ArrayReserveError)
                }
            });
        }
        let old = self.block.layout();
        let new = block_layout::<V>(cap).ok_or(ArrayReserveError)?;
        // SAFETY: the block's own layout; `new.size()` is non-zero and fits
        // an `isize`; the same alignment.
        let raw = unsafe { realloc(self.block.ptr.as_ptr().cast::<u8>(), old, new.size()) };
        let ptr = NonNull::new(raw.cast::<Header>()).ok_or(ArrayReserveError)?;
        self.block.ptr = ptr;
        // SAFETY: unique, and the header moved with the block.
        unsafe { self.block.header_mut().bytes = new.size() };
        Ok(())
    }

    /// Give back the slots past the last element.
    pub(crate) fn shrink_to_fit(&mut self) {
        if self.capacity() > self.len() {
            // A refused shrink keeps the larger block, which is still valid.
            let _ = self.grow_to(self.len());
        }
    }

    /// The elements, for as long as the handle is borrowed.
    #[inline]
    pub(crate) fn into_mut_slice(self) -> &'a mut [V] {
        // SAFETY: as `DerefMut`'s, for the borrow this value held.
        unsafe { core::slice::from_raw_parts_mut(self.block.elements(), self.block.len()) }
    }

    /// Room for `additional` more elements, growing geometrically, as
    /// `Vec::try_reserve` does.
    pub(crate) fn try_reserve(&mut self, additional: usize) -> Result<(), ArrayReserveError> {
        let (len, cap) = (self.len(), self.capacity());
        if cap - len >= additional {
            return Ok(());
        }
        let needed = len.checked_add(additional).ok_or(ArrayReserveError)?;
        let doubled = cap.saturating_mul(2);
        let floor = if mem::size_of::<V>() > 1024 { 1 } else { 4 };
        self.grow_to(needed.max(doubled).max(floor))
    }

    /// [`try_reserve`](Self::try_reserve), panicking on overflow and
    /// aborting on allocation failure, as `Vec::reserve` does.
    #[inline]
    pub(crate) fn reserve(&mut self, additional: usize) {
        if self.capacity() - self.len() < additional {
            self.reserve_cold(additional);
        }
    }

    #[cold]
    #[inline(never)]
    fn reserve_cold(&mut self, additional: usize) {
        if self.try_reserve(additional).is_err() {
            match self
                .len()
                .checked_add(additional)
                .and_then(block_layout::<V>)
            {
                Some(layout) => handle_alloc_error(layout),
                None => capacity_overflow(),
            }
        }
    }

    /// Append one element.
    #[inline]
    pub(crate) fn push(&mut self, value: V) {
        let len = self.len();
        if len == self.capacity() {
            self.reserve_cold(1);
        }
        // SAFETY: slot `len` is allocated and free.
        unsafe {
            self.elements().add(len).write(value);
            self.set_len(len + 1);
        }
    }

    /// Remove and return the last element.
    #[inline]
    pub(crate) fn pop(&mut self) -> Option<V> {
        let len = self.len().checked_sub(1)?;
        // SAFETY: slot `len` was initialized and is no longer counted.
        unsafe {
            self.set_len(len);
            Some(self.elements().add(len).read())
        }
    }

    /// Drop every element from `len` on.
    pub(crate) fn truncate(&mut self, len: usize) {
        let old = self.len();
        if len >= old {
            return;
        }
        // SAFETY: the count shrinks before the tail is dropped, as
        // `Vec::truncate` does, so a panicking destructor leaves a valid
        // array.
        unsafe {
            self.set_len(len);
            ptr::drop_in_place(ptr::slice_from_raw_parts_mut(
                self.elements().add(len),
                old - len,
            ));
        }
    }

    /// Drop every element.
    #[inline]
    pub(crate) fn clear(&mut self) {
        self.truncate(0);
    }

    /// Insert `value` at `index`, shifting the rest right.
    ///
    /// # Panics
    ///
    /// If `index > len`.
    #[track_caller]
    pub(crate) fn insert(&mut self, index: usize, value: V) {
        let len = self.len();
        assert!(
            index <= len,
            "insertion index (is {index}) should be <= len (is {len})"
        );
        self.reserve(1);
        // SAFETY: room for one more; the tail moves up one slot.
        unsafe {
            let at = self.elements().add(index);
            ptr::copy(at, at.add(1), len - index);
            at.write(value);
            self.set_len(len + 1);
        }
    }

    /// Remove and return the element at `index`, shifting the rest left.
    ///
    /// # Panics
    ///
    /// If `index >= len`.
    #[track_caller]
    pub(crate) fn remove(&mut self, index: usize) -> V {
        let len = self.len();
        assert!(
            index < len,
            "removal index (is {index}) should be < len (is {len})"
        );
        // SAFETY: the element is read out, the tail moves down over it.
        unsafe {
            let at = self.elements().add(index);
            let value = at.read();
            ptr::copy(at.add(1), at, len - index - 1);
            self.set_len(len - 1);
            value
        }
    }

    /// Append every item of `iter`.
    pub(crate) fn extend<I: IntoIterator<Item = V>>(&mut self, iter: I) {
        let mut iter = iter.into_iter();
        self.reserve(iter.size_hint().0);
        // The reserved slots are filled without a capacity check or a
        // header write per element; anything past them is pushed.
        let (start, room) = (self.len(), self.capacity());
        let base = self.elements();
        let mut counted = CountOnDrop {
            header: self.block.ptr,
            len: start,
        };
        while counted.len < room {
            let Some(value) = iter.next() else { break };
            // SAFETY: slot `counted.len < room` is allocated and free.
            unsafe { base.add(counted.len).write(value) };
            counted.len += 1;
        }
        drop(counted);
        for value in iter {
            self.push(value);
        }
    }

    /// Keep only the elements `keep` answers `true` for, in order.
    pub(crate) fn retain<F: FnMut(&V) -> bool>(&mut self, mut keep: F) {
        let len = self.len();
        // Zeroed while `keep` and the destructors run: a panic leaks the
        // elements rather than leaving holes in the counted prefix.
        // SAFETY: zero is always a valid count.
        unsafe { self.set_len(0) };
        let base = self.elements();
        let mut kept = 0;
        for at in 0..len {
            // SAFETY: slot `at` is initialized and not yet moved or dropped;
            // slot `kept <= at` is free (moved from, or `at` itself).
            unsafe {
                let slot = base.add(at);
                if keep(&*slot) {
                    if kept != at {
                        ptr::copy_nonoverlapping(slot, base.add(kept), 1);
                    }
                    kept += 1;
                } else {
                    ptr::drop_in_place(slot);
                }
            }
        }
        // SAFETY: the first `kept` slots hold the kept elements.
        unsafe { self.set_len(kept) };
    }

    /// Replace the elements in `range` with `replacement`'s, in order
    /// (`Vec::splice` with its returned iterator dropped).
    ///
    /// # Panics
    ///
    /// If `range` is not within the elements.
    #[track_caller]
    pub(crate) fn splice<I: IntoIterator<Item = V>>(
        &mut self,
        range: Range<usize>,
        replacement: I,
    ) {
        let len = self.len();
        assert!(
            range.start <= range.end && range.end <= len,
            "splice range {range:?} out of bounds for length {len}"
        );
        let mut items = ManuallyDrop::new(replacement.into_iter().collect::<Vec<V>>());
        let (inserted, removed) = (items.len(), range.end - range.start);
        self.reserve(inserted.saturating_sub(removed));
        let base = self.elements();
        // SAFETY: the count drops to the range's start before the range's
        // elements are dropped (a panic leaks the tail and the replacement);
        // the tail then moves to just past the replacement, which is moved
        // in, and the count covers all of it again. The reservation above
        // made room for the longer of the two.
        unsafe {
            self.set_len(range.start);
            ptr::drop_in_place(ptr::slice_from_raw_parts_mut(
                base.add(range.start),
                removed,
            ));
            ptr::copy(
                base.add(range.end),
                base.add(range.start + inserted),
                len - range.end,
            );
            ptr::copy_nonoverlapping(items.as_ptr(), base.add(range.start), inserted);
            items.set_len(0);
            ManuallyDrop::drop(&mut items);
            self.set_len(len - removed + inserted);
        }
    }

    /// Drop the elements in `range`, shifting the rest left (`Vec::drain`
    /// with its returned iterator dropped).
    #[track_caller]
    pub(crate) fn drain(&mut self, range: Range<usize>) {
        self.splice(range, core::iter::empty());
    }

    /// Grow to `new_len` with clones of `value` (the last slot takes `value`
    /// itself), or truncate to it.
    pub(crate) fn resize(&mut self, new_len: usize, value: V)
    where
        V: Clone,
    {
        let len = self.len();
        if new_len <= len {
            self.truncate(new_len);
            return;
        }
        self.reserve(new_len - len);
        for _ in len + 1..new_len {
            self.push(value.clone());
        }
        self.push(value);
    }
}

impl<V> Deref for UniqueBlock<'_, V> {
    type Target = [V];

    #[inline]
    fn deref(&self) -> &[V] {
        self.block.as_slice()
    }
}

impl<V> DerefMut for UniqueBlock<'_, V> {
    #[inline]
    fn deref_mut(&mut self) -> &mut [V] {
        // SAFETY: the first `len` slots are initialized, and this is the only
        // handle on the block, borrowed mutably.
        unsafe { core::slice::from_raw_parts_mut(self.elements(), self.len()) }
    }
}

/// By-value iteration over a block this iterator was the only handle on:
/// the elements in `next..end` are moved out one at a time, and the rest are
/// already gone (the block's own count is zero, so freeing it drops none).
pub(crate) struct BlockIntoIter<V> {
    block: Block<V>,
    next: usize,
    end: usize,
}

impl<V> BlockIntoIter<V> {
    /// Iterate `window` of `block`'s elements, dropping the others first.
    /// `None` (with the handle back) if another handle shares the block.
    pub(crate) fn new(mut block: Block<V>, window: Range<usize>) -> Result<Self, Block<V>> {
        let Some(mut unique) = block.get_mut() else {
            return Err(block);
        };
        assert!(window.start <= window.end && window.end <= unique.len());
        unique.truncate(window.end);
        let base = unique.elements();
        // SAFETY: the count is zeroed first, so the elements before the
        // window are dropped once (here) and the window's are owned by the
        // iterator alone from now on.
        unsafe {
            unique.set_len(0);
            ptr::drop_in_place(ptr::slice_from_raw_parts_mut(base, window.start));
        }
        Ok(Self {
            block,
            next: window.start,
            end: window.end,
        })
    }
}

impl<V> Iterator for BlockIntoIter<V> {
    type Item = V;

    #[inline]
    fn next(&mut self) -> Option<V> {
        if self.next == self.end {
            return None;
        }
        // SAFETY: slot `next` is initialized and owned by the iterator.
        let value = unsafe { self.block.elements().add(self.next).read() };
        self.next += 1;
        Some(value)
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        let n = self.end - self.next;
        (n, Some(n))
    }
}

impl<V> DoubleEndedIterator for BlockIntoIter<V> {
    #[inline]
    fn next_back(&mut self) -> Option<V> {
        if self.next == self.end {
            return None;
        }
        self.end -= 1;
        // SAFETY: slot `end` is initialized and owned by the iterator.
        Some(unsafe { self.block.elements().add(self.end).read() })
    }
}

impl<V> Drop for BlockIntoIter<V> {
    fn drop(&mut self) {
        let (next, end) = (self.next, self.end);
        self.next = end;
        // SAFETY: the elements not yet yielded, dropped once; the block (its
        // count zero) is freed by `block`'s own drop after this.
        unsafe {
            ptr::drop_in_place(ptr::slice_from_raw_parts_mut(
                self.block.elements().add(next),
                end - next,
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::rc::Rc;
    use alloc::vec;

    #[derive(Clone)]
    struct Counted(Rc<Cell<usize>>);
    impl Drop for Counted {
        fn drop(&mut self) {
            self.0.set(self.0.get() + 1);
        }
    }

    fn ints(block: &Block<i64>) -> Vec<i64> {
        block.as_slice().to_vec()
    }

    #[test]
    fn a_vec_round_trips_through_a_block_in_place() {
        let block = Block::from_vec(vec![1_i64, 2, 3]);
        assert_eq!(ints(&block), [1, 2, 3]);
        assert_eq!(block.capacity(), 3, "exactly sized");
        let vec = block.try_into_vec().ok().unwrap();
        assert_eq!(vec, [1, 2, 3]);
        // A buffer with room for the header is the block as it stands, and
        // goes back to a `Vec` of the same capacity.
        let mut with_room = Vec::with_capacity(10);
        with_room.push(7_i64);
        let buffer = with_room.as_ptr().cast::<u8>();
        let block = Block::from_vec(with_room);
        assert_eq!((ints(&block), block.capacity()), (vec![7], 7));
        assert_eq!(block.ptr.as_ptr().cast::<u8>().cast_const(), buffer);
        let vec = block.try_into_vec().ok().unwrap();
        assert_eq!((vec.capacity(), vec.as_ptr().cast::<u8>()), (10, buffer));
        assert!(Block::from_vec(Vec::<i64>::new()).is_empty());
        // An element less aligned than the header takes the copying route.
        let small = Block::from_vec(vec![1_u16, 2, 3]);
        assert_eq!(small.as_slice(), [1, 2, 3]);
        assert_eq!(small.try_into_vec().ok().unwrap(), [1, 2, 3]);
    }

    #[test]
    fn a_shared_block_refuses_writes_and_moves() {
        let mut block = Block::from_vec(vec![1_i64]);
        let other = block.clone();
        assert_eq!(block.strong_count(), 2);
        assert!(block.get_mut().is_none());
        assert!(block.ptr_eq(&other));
        let block = block.try_into_vec().err().unwrap();
        drop(other);
        let mut block = block;
        assert!(block.get_mut().is_some());
    }

    #[test]
    fn the_vec_methods_agree_with_vec() {
        let mut block = Block::with_capacity(0);
        let mut model: Vec<i64> = Vec::new();
        let mut unique = block.get_mut().unwrap();
        for i in 0..50 {
            unique.push(i);
            model.push(i);
        }
        unique.insert(3, 100);
        model.insert(3, 100);
        assert_eq!(unique.remove(7), model.remove(7));
        unique.retain(|x| x % 3 != 0);
        model.retain(|x| x % 3 != 0);
        unique.splice(2..5, [9, 9]);
        model.splice(2..5, [9, 9]);
        unique.splice(1..2, [5, 6, 7, 8]);
        model.splice(1..2, [5, 6, 7, 8]);
        unique.drain(4..6);
        model.drain(4..6);
        unique.resize(40, -1);
        model.resize(40, -1);
        unique.truncate(30);
        model.truncate(30);
        assert_eq!(unique.pop(), model.pop());
        unique.extend([1, 2, 3]);
        model.extend([1, 2, 3]);
        unique[0] = 42;
        model[0] = 42;
        assert_eq!(&*unique, &model[..]);
        unique.clear();
        assert!(unique.is_empty());
    }

    #[test]
    fn a_large_vec_keeps_its_buffer_and_every_write_still_works() {
        let n = EXTERNAL_MIN_BYTES / mem::size_of::<i64>() + 500;
        let mut model: Vec<i64> = (0..n as i64).collect();
        model.reserve(100);
        let buffer = model.as_ptr();
        let mut block = Block::from_vec(model.clone());
        assert!(block.is_external());
        assert_eq!(block.as_slice(), &model[..]);
        let mut unique = block.get_mut().unwrap();
        for i in 0..300 {
            unique.push(i);
            model.push(i);
        }
        unique.insert(3, -1);
        model.insert(3, -1);
        assert_eq!(unique.remove(9), model.remove(9));
        unique.retain(|x| x % 5 != 0);
        model.retain(|x| x % 5 != 0);
        unique.splice(10..20, [7, 7, 7]);
        model.splice(10..20, [7, 7, 7]);
        unique.truncate(n - 300);
        model.truncate(n - 300);
        unique.shrink_to_fit();
        assert_eq!(unique.capacity(), n - 300);
        assert_eq!(&*unique, &model[..]);
        let vec = block.try_into_vec().ok().unwrap();
        assert_eq!(vec, model);
        // Taken back as it stands: no copy into or out of a block.
        let block = Block::from_vec(vec);
        let unchanged = block.as_slice().as_ptr();
        let back = block.try_into_vec().ok().unwrap();
        assert_eq!(back.as_ptr(), unchanged);
        let fresh: Vec<i64> = (0..n as i64).collect();
        let at = fresh.as_ptr();
        let fresh = Block::from_vec(fresh);
        assert_eq!(fresh.as_slice().as_ptr(), at);
        assert!(!buffer.is_null());
    }

    #[test]
    fn every_element_drops_once() {
        let drops = Rc::new(Cell::new(0));
        let make = |n: usize| -> Block<Counted> {
            Block::from_vec((0..n).map(|_| Counted(drops.clone())).collect())
        };
        drop(make(5));
        assert_eq!(drops.get(), 5);
        drops.set(0);
        let mut iter = BlockIntoIter::new(make(6), 1..4).ok().unwrap();
        assert_eq!(drops.get(), 3, "outside the window, dropped up front");
        drop(iter.next());
        drop(iter.next_back());
        assert_eq!(drops.get(), 5);
        drop(iter);
        assert_eq!(drops.get(), 6);
        drops.set(0);
        let mut block = make(6);
        {
            let mut unique = block.get_mut().unwrap();
            unique.retain(|_| false);
            assert!(unique.is_empty());
        }
        assert_eq!(drops.get(), 6);
        drops.set(0);
        let mut block = make(6);
        block
            .get_mut()
            .unwrap()
            .splice(1..3, [Counted(drops.clone())]);
        assert_eq!(drops.get(), 2);
        drop(block);
        assert_eq!(drops.get(), 7);
        let shared = make(2);
        let handle = shared.clone();
        drops.set(0);
        drop(shared);
        assert_eq!(drops.get(), 0, "another handle keeps the elements");
        drop(handle);
        assert_eq!(drops.get(), 2);
        // An external block drops each element once too, iterated or not.
        let big = EXTERNAL_MIN_BYTES / mem::size_of::<Counted>() + 1;
        drops.set(0);
        drop(make(big));
        assert_eq!(drops.get(), big);
        drops.set(0);
        let mut iter = BlockIntoIter::new(make(big), 2..big - 2).ok().unwrap();
        drop(iter.next());
        drop(iter);
        assert_eq!(drops.get(), big);
        let _ = Rc::try_unwrap(drops).ok();
    }

    #[test]
    fn a_block_is_one_allocation_of_header_and_elements() {
        assert_eq!(mem::size_of::<Block<u64>>(), mem::size_of::<usize>());
        assert_eq!(elements_offset::<[u64; 4]>(), 24);
        assert_eq!(block_layout::<[u64; 4]>(2).unwrap().size(), 24 + 64);
        assert!(block_layout::<[u64; 4]>(usize::MAX).is_none());
    }
}
