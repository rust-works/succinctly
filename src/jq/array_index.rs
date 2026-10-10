//! A per-array element index, so reading a document array more than once is
//! O(1) per read instead of O(length) (#4035).
//!
//! `.users[0]`, `.[5]` and `$root.nodes[.from]` resolve through
//! [`DocumentElements::len_checked`] before they fetch the element: the walk
//! normalizes a negative index and raises yq's own out-of-range error, and it
//! is also where the malformed-delimiter checks live (a missing or doubled
//! `,`, a trailing stray one, #1677, #2261, #2594). That is free for one read
//! and quadratic for a program that uses a document array as a lookup table,
//! because every read walks the whole array again, and the element itself is
//! then another sibling walk to the index.
//!
//! # Why it cannot change an answer
//!
//! - [`ElementIndex::build`] is the same walk `len_checked` makes, with the
//!   same checks, that also records each element's node id. **Any anomaly
//!   refuses the build** (the first element, any element, or the trailing
//!   delimiter fails its gap check, or the array is past
//!   [`MAX_INDEXED_ELEMENTS`]) and the caller runs the walk it always ran,
//!   which raises exactly what it always raised. So an index exists only for
//!   an array the walk would not have raised on for *any* index, and then the
//!   length is the number of ids and the element at `k` is the node `ids[k]`
//!   names -- what `get_cursor(k)` returns.
//! - No value is decoded: validation stays "where something reads it"
//!   (#2692), as it was.
//!
//! # When one is built
//!
//! Not on first sight, and not on second. A walk of [`WIDE_ELEMENTS`] or more
//! registers the array, the next *length* lookup walks again and counts, and
//! the third builds the index (an element lookup never does). An index costs
//! one walk and the ids it keeps, so building it on the second lookup made an
//! array read exactly twice slower than the two walks it replaced (+5% to +9%
//! on `.[] | [.[3], .[4]]` over 20,000 hundred-element arrays); on the third
//! it replaces a walk with its price and every later read is O(1). An array
//! that is small, or read twice, never costs more than the walks it always
//! cost; a refused build is remembered so it is not retried, and so is an
//! eviction, as in [`super::key_index`].
//!
//! # Element reads with no length lookup: a prefix (#4162)
//!
//! The path-context index step (`.[$i] | ... | key`, `getpath([$i]) | ... | key`)
//! reads a non-negative index with no length lookup, so it never reached the
//! rule above and each read walked to its index: a loop over a wide array was
//! quadratic in either order. Building the whole index on an element read
//! would walk the whole array where the reads walk only to their indexes
//! (three reads near the front of a 1M-element array would walk 1M elements),
//! so such an array gets an [`ElementPrefix`] instead: the ids of the elements
//! the reads have walked past, and where that walk stopped. A read inside it
//! is answered from the ids; a read beyond it resumes the walk there and
//! records as it goes. It never walks further than the plain reads would, and
//! no element twice, so a loop over `n` elements is O(n) ascending or
//! descending.
//!
//! It cannot change an answer: `get_cursor(k)` is `k` `uncons_cursor` steps
//! from the head with no checks, and a prefix is the same steps split across
//! reads, resumed from the list [`DocumentElements::at_head_id`] rebuilds (a
//! cursor is its document plus a node, so the rebuilt list is the list the
//! walk left). Because it makes no checks, a prefix never answers a length.
//!
//! A first element read at [`WIDE_ELEMENTS`] or more registers the array, the
//! second walks, and the third starts the prefix: recording costs a push per
//! element and the ids' allocation, so starting on the second read made
//! `path(.[90], .[91])` over 20,000 arrays 10-14% slower than its two walks.
//! It starts only while element reads run two ahead of length lookups. The
//! value route makes a length lookup before each element read, so it never
//! does, and stays on the rule above; a length lookup that finds a prefix hands
//! the array back to it. A prefix counts against the same limits as an index,
//! charged for the ids it holds; retired, its array walks element reads for
//! good but can still be indexed by the length route. A read too far to record
//! walks and leaves the prefix in place, and only a walk that found its
//! element registers an array, so a read past the end of a short one does not.
//!
//! # Scope
//!
//! The memo lives only inside an evaluation's scope ([`memo::enter`], opened
//! beside `key_index`'s and `slot_memo`'s) and is consulted only for that
//! document's own cursors; see `key_index` for why the document token is not
//! enough on its own. `no_std` has no `thread_local!`, so there the memo
//! never exists and every read is the walk.

#![cfg_attr(not(feature = "std"), allow(dead_code))]

#[cfg(not(test))]
use alloc::vec::Vec;

use super::document::{trailing_element_gap_ok, DocumentCursor, DocumentElements};
use super::error::EvalError;

/// A walk of at least this many elements registers an array as wide. Below it
/// the walk is cheaper than the thread-local probe plus a build.
pub(crate) const WIDE_ELEMENTS: usize = 64;

/// An array with more elements than this keeps the walk: the index costs 8
/// bytes per element.
pub(crate) const MAX_INDEXED_ELEMENTS: usize = 1 << 21;

/// What an element read found in an index.
pub(crate) enum Element<C> {
    /// The element's cursor.
    Found(C),
    /// An index past the end.
    Past,
}

/// The node id of every element of one array, in document order.
pub(crate) struct ElementIndex {
    ids: Vec<usize>,
}

impl ElementIndex {
    /// Index the elements `elements` still holds, or `None` when the array is
    /// anything but plainly well-formed (see the module doc: a refusal sends
    /// the caller to the walk, which says what is wrong). The checks are
    /// [`DocumentElements::len_checked`]'s, in the same order.
    pub(crate) fn build<E: DocumentElements>(elements: &E, len: usize) -> Option<Self> {
        Self::build_within(elements, len, MAX_INDEXED_ELEMENTS)
    }

    /// `len` is the length a walk of this list found, to size the ids: a wrong
    /// guess costs only a reallocation.
    fn build_within<E: DocumentElements>(elements: &E, len: usize, max: usize) -> Option<Self> {
        let mut ids = Vec::with_capacity(len.min(max));
        let mut elems = *elements;
        let mut is_first = true;
        let mut last = None;
        while let Some((cursor, rest)) = elems.uncons_cursor() {
            if ids.len() >= max || !cursor.element_gap_ok(is_first) {
                return None;
            }
            ids.push(cursor.node_id());
            last = Some(cursor);
            elems = rest;
            is_first = false;
        }
        if let Some(last) = last {
            if !trailing_element_gap_ok(&last, b']') {
                return None;
            }
        }
        Some(Self { ids })
    }

    pub(crate) fn len(&self) -> usize {
        self.ids.len()
    }

    /// The cursor of element `index`, or [`Element::Past`] past the end;
    /// `None` when the id does not resolve (the caller then walks).
    fn get<C: DocumentCursor>(&self, head: &C, index: usize) -> Option<Element<C>> {
        match self.ids.get(index) {
            Some(id) => head.at_node_id(*id).map(Element::Found),
            None => Some(Element::Past),
        }
    }
}

/// The most ids one extension of an [`ElementPrefix`] reserves room for up
/// front; a longer walk grows the vector as it goes.
const RESERVE_ELEMENTS: usize = 1 << 12;

/// The node ids of an array's first elements, in document order, as element
/// reads walked them, and the list the walk stopped at (#4162; see the module
/// doc). Answers element reads only, never a length.
pub(crate) struct ElementPrefix {
    ids: Vec<usize>,
    /// The [`DocumentElements::head_id`] of the list after the last recorded
    /// element, or `None` once the walk has reached the end of the array.
    frontier: Option<usize>,
}

impl ElementPrefix {
    /// An empty prefix of the list whose head id is `head`.
    pub(crate) fn new(head: usize) -> Self {
        Self {
            ids: Vec::new(),
            frontier: Some(head),
        }
    }

    pub(crate) fn len(&self) -> usize {
        self.ids.len()
    }

    /// Whether the walk has reached the end of the array, so the prefix is
    /// the whole array and every read beyond it is past the end.
    fn is_complete(&self) -> bool {
        self.frontier.is_none()
    }

    /// The cursor of element `index` of the array `elements` heads, extending
    /// the walk to it when it lies past the prefix, or [`Element::Past`] past
    /// the end; `None` when an id does not resolve (the caller then walks).
    pub(crate) fn get<E: DocumentElements>(
        &mut self,
        elements: &E,
        index: usize,
    ) -> Option<Element<E::Cursor>> {
        if let Some(id) = self.ids.get(index) {
            let (head, _) = elements.uncons_cursor()?;
            return head.at_node_id(*id).map(Element::Found);
        }
        let Some(frontier) = self.frontier else {
            return Some(Element::Past);
        };
        let mut list = elements.at_head_id(frontier)?;
        // The ids move into a local for the walk, so the loop does not store
        // the vector's length back through `self` on every push.
        let mut ids = core::mem::take(&mut self.ids);
        let steps = index + 1 - ids.len();
        // One allocation for the walk to `index` rather than a doubling every
        // few pushes, capped: `index` may lie far past the end of the array.
        ids.reserve(steps.min(RESERVE_ELEMENTS));
        let mut last = None;
        for _ in 0..steps {
            let Some((cursor, rest)) = list.uncons_cursor() else {
                break;
            };
            ids.push(cursor.node_id());
            note_recorded();
            last = Some(cursor);
            list = rest;
        }
        self.ids = ids;
        if self.ids.len() > index {
            self.frontier = list.head_id().map(|(id, _)| id);
            last.map(Element::Found)
        } else {
            self.frontier = None;
            Some(Element::Past)
        }
    }
}

#[cfg(test)]
std::thread_local! {
    static RECORDED: core::cell::Cell<usize> = const { core::cell::Cell::new(0) };
}

/// Elements recorded by every prefix so far, for the tests that pin that a
/// loop over an array walks each element once.
#[cfg(test)]
pub(crate) fn recorded() -> usize {
    RECORDED.with(core::cell::Cell::get)
}

#[inline(always)]
fn note_recorded() {
    #[cfg(test)]
    RECORDED.with(|r| r.set(r.get() + 1));
}

/// [`DocumentElements::len_checked`] for a caller inside an evaluation: the
/// same answer, from an [`ElementIndex`] once the array has proved wide.
///
/// A drop-in for `len_checked` and nothing more: it does **not** make the
/// zero-element `[,]` check (`empty_elements_tail_gap_ok`), which `len_checked`
/// cannot make either and which every caller makes first. (An index is only
/// ever built for an array of [`WIDE_ELEMENTS`] or more, so the two never meet,
/// but a caller must not rely on that.)
#[inline]
pub(crate) fn len_checked_memoized<E: DocumentElements>(elements: &E) -> Result<usize, EvalError> {
    if let Some(len) = memo::len(elements) {
        return Ok(len);
    }
    let len = elements.len_checked()?;
    if len >= WIDE_ELEMENTS {
        memo::note_wide(elements, len);
    }
    Ok(len)
}

/// [`DocumentElements::get_cursor`] for a caller inside an evaluation, from an
/// [`ElementIndex`] when one exists (never building one), or from an
/// [`ElementPrefix`] when element reads have outnumbered length lookups.
#[inline]
pub(crate) fn get_cursor_memoized<E: DocumentElements>(
    elements: &E,
    index: usize,
) -> Option<E::Cursor> {
    match memo::get(elements, index) {
        Read::Answered(Element::Found(cursor)) => Some(cursor),
        Read::Answered(Element::Past) => None,
        Read::Walk => elements.get_cursor(index),
        Read::Unregistered => {
            let found = elements.get_cursor(index);
            // Only a walk that found its element proves the array wide: a read
            // past the end of a short array proves nothing, and registering
            // one would let the length route index a small array.
            if found.is_some() {
                memo::note_wide_read(elements);
            }
            found
        }
    }
}

/// What the memo says about one element read.
pub(crate) enum Read<C> {
    /// An index or a prefix answered it.
    Answered(Element<C>),
    /// Walk; the array is registered (or never will be from this read).
    Walk,
    /// Walk, and the read is wide but the array is not registered: register
    /// it if the walk finds the element.
    Unregistered,
}

#[cfg(feature = "std")]
pub(crate) mod memo {
    use std::cell::{Cell, RefCell};

    use super::{
        DocumentElements, ElementIndex, ElementPrefix, Read, MAX_INDEXED_ELEMENTS, WIDE_ELEMENTS,
    };

    /// Arrays remembered at once, indexed or not. Least recently used first.
    const ENTRIES: usize = 16;
    /// Indexes kept at once.
    const INDEXES: usize = 4;
    /// The length lookup that builds is the one after this many walks of the
    /// array (the registering one included): the third lookup.
    const BUILD_AT: u8 = 2;
    /// Element reads beyond the length lookups that walk before a prefix
    /// starts: the registering read and one more, so the third records.
    const PREFIX_AFTER: u8 = 2;
    /// `Seen::reads` of an array whose prefix was evicted: element reads walk
    /// and never start another (no second chance, as for an index), while
    /// length lookups still count toward building an index. Counting reads
    /// never gets near it: a prefix starts by `PREFIX_AFTER` past the lookups.
    const NO_PREFIX: u8 = u8::MAX;
    /// Elements indexed across all the indexes kept.
    const ELEMENT_BUDGET: usize = MAX_INDEXED_ELEMENTS;

    enum Kind {
        /// Wide walks have been seen, this many length lookups of them so far
        /// (counting the one that registered it), for an array of this many
        /// elements (0 when only element reads have walked it): the third
        /// lookup builds, sized from the length. `reads` counts element reads;
        /// the one that puts them [`PREFIX_AFTER`] past the lookups starts a
        /// prefix, unless they are [`NO_PREFIX`].
        Seen {
            lookups: u8,
            len: usize,
            reads: u8,
        },
        /// Element reads with no length lookup to build from (#4162).
        Prefix(ElementPrefix),
        /// Always walk: the build was refused (malformed, too wide), or the
        /// index was evicted (an evicted array is not given a second chance,
        /// for the reason `key_index` gives).
        Refused,
        Indexed(ElementIndex),
    }

    struct Entry {
        /// The node id of the first element of the list this entry describes.
        head: usize,
        kind: Kind,
    }

    struct State {
        document: usize,
        entries: Vec<Entry>,
    }

    thread_local! {
        static MEMO: RefCell<Option<State>> = const { RefCell::new(None) };
        /// A Bloom filter over the heads of the open scope's entries (one bit
        /// per head, from [`bit`]): zero while the scope holds none, which is
        /// the one load every lookup of an evaluation that has seen no wide
        /// array makes, and a cheap reject for the small arrays read after one
        /// has. A stale bit (an evicted entry) only costs a probe.
        static BLOOM: Cell<u64> = const { Cell::new(0) };
    }

    /// The filter bit of the list whose first element has node id `id`.
    #[inline(always)]
    fn bit(id: usize) -> u64 {
        1 << ((id as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) >> 58)
    }

    fn bloom_of(entries: &[Entry]) -> u64 {
        entries.iter().fold(0, |b, e| b | bit(e.head))
    }

    #[cfg(test)]
    thread_local! {
        static WORK: Cell<(usize, usize)> = const { Cell::new((0, 0)) };
    }

    /// Indexes built, and lookups an index answered, for the tests that pin
    /// the memo's cost.
    #[cfg(test)]
    pub(crate) fn work() -> (usize, usize) {
        WORK.with(Cell::get)
    }

    #[inline(always)]
    fn note_build() {
        #[cfg(test)]
        WORK.with(|w| w.set((w.get().0 + 1, w.get().1)));
    }

    #[inline(always)]
    fn note_hit() {
        #[cfg(test)]
        WORK.with(|w| w.set((w.get().0, w.get().1 + 1)));
    }

    enum Restore {
        Nothing,
        Previous(Option<State>),
    }

    pub(crate) struct Guard(Restore);

    impl Drop for Guard {
        fn drop(&mut self) {
            if let Restore::Previous(previous) = std::mem::replace(&mut self.0, Restore::Nothing) {
                BLOOM.with(|b| b.set(previous.as_ref().map_or(0, |s| bloom_of(&s.entries))));
                MEMO.with(|m| *m.borrow_mut() = previous);
            }
        }
    }

    /// Open the scope for evaluations over `document`: a no-op while one is
    /// already open for it, a nested scope (restored on drop) otherwise.
    pub(crate) fn enter(document: usize) -> Guard {
        MEMO.with(|m| {
            let mut m = m.borrow_mut();
            if m.as_ref().is_some_and(|s| s.document == document) {
                return Guard(Restore::Nothing);
            }
            BLOOM.with(|b| b.set(0));
            Guard(Restore::Previous(m.replace(State {
                document,
                entries: Vec::new(),
            })))
        })
    }

    /// The length of the list `elements`, if an index (built now, on the
    /// third length lookup of a wide array) can say; `None` means "walk".
    #[inline]
    pub(crate) fn len<E: DocumentElements>(elements: &E) -> Option<usize> {
        let bloom = BLOOM.with(Cell::get);
        if bloom == 0 {
            return None;
        }
        length(elements, bloom)
    }

    /// Element `index` of the list `elements`, when an index or a prefix can
    /// say, else whether to walk and register. Starts or extends a prefix
    /// (#4162); never builds an index.
    #[inline]
    pub(crate) fn get<E: DocumentElements>(elements: &E, index: usize) -> Read<E::Cursor> {
        let bloom = BLOOM.with(Cell::get);
        if bloom == 0 {
            return unregistered(index);
        }
        element(elements, bloom, index)
    }

    /// A read of an array the memo holds no entry for.
    #[inline(always)]
    fn unregistered<C>(index: usize) -> Read<C> {
        if index >= WIDE_ELEMENTS {
            Read::Unregistered
        } else {
            Read::Walk
        }
    }

    #[inline(never)]
    fn length<E: DocumentElements>(elements: &E, bloom: u64) -> Option<usize> {
        let (id, document) = elements.head_id()?;
        if bloom & bit(id) == 0 {
            return None;
        }
        MEMO.with(|m| {
            let mut m = m.borrow_mut();
            let state = m.as_mut().filter(|s| s.document == document)?;
            let at = state.entries.iter().position(|e| e.head == id)?;
            let mut entry = state.entries.remove(at);
            match entry.kind {
                Kind::Seen {
                    lookups,
                    len,
                    reads,
                } => {
                    if lookups < BUILD_AT {
                        entry.kind = Kind::Seen {
                            lookups: lookups + 1,
                            len,
                            reads,
                        };
                    } else {
                        make_room(&mut state.entries, 1, 0);
                        entry.kind = match ElementIndex::build(elements, len) {
                            Some(index) => {
                                note_build();
                                make_room(&mut state.entries, 1, index.len());
                                Kind::Indexed(index)
                            }
                            None => Kind::Refused, // patchcov: coverage tolerate-line reason="defensive: an array registers only after a walk that succeeded, and the build makes that walk's checks, so it refuses only past MAX_INDEXED_ELEMENTS, which a unit test pins on build_within directly"
                        };
                    }
                }
                // A length lookup of an array element reads were walking: the
                // prefix cannot answer it (it makes no checks), so the array
                // goes back to the length route, this lookup counting as the
                // one that registers it there.
                Kind::Prefix(_) => {
                    entry.kind = Kind::Seen {
                        lookups: 1,
                        len: 0,
                        reads: 0,
                    };
                }
                Kind::Refused | Kind::Indexed(_) => {}
            }
            let found = match &entry.kind {
                Kind::Indexed(index) => {
                    note_hit();
                    Some(index.len())
                }
                _ => None,
            };
            state.entries.push(entry);
            found
        })
    }

    #[inline(never)]
    fn element<E: DocumentElements>(elements: &E, bloom: u64, index: usize) -> Read<E::Cursor> {
        let Some((id, document)) = elements.head_id() else {
            return Read::Walk;
        };
        if bloom & bit(id) == 0 {
            return unregistered(index);
        }
        MEMO.with(|m| {
            let mut m = m.borrow_mut();
            let Some(state) = m.as_mut().filter(|s| s.document == document) else {
                return Read::Walk;
            };
            let Some(at) = state.entries.iter().position(|e| e.head == id) else {
                return unregistered(index);
            };
            let mut entry = state.entries.remove(at);
            if let Kind::Seen {
                lookups,
                len,
                reads,
            } = entry.kind
            {
                let reads = reads.saturating_add(1);
                if reads == NO_PREFIX || reads <= lookups.saturating_add(PREFIX_AFTER) {
                    // Read once or twice, the length route is serving this
                    // array (a length lookup came before every element read),
                    // or its prefix was evicted: walk, as #4035 does.
                    entry.kind = Kind::Seen {
                        lookups,
                        len,
                        reads,
                    };
                    state.entries.push(entry);
                    return Read::Walk;
                }
                // Room is made below, for what the extension records.
                entry.kind = Kind::Prefix(ElementPrefix::new(id));
            }
            let found = match &mut entry.kind {
                Kind::Indexed(ix) => match elements.uncons_cursor() {
                    Some((head, _)) => ix.get(&head, index),
                    None => None, // patchcov: coverage tolerate-line reason="defensive: a list with a head id has a first element"
                },
                Kind::Prefix(prefix) if index < prefix.len() || prefix.is_complete() => {
                    prefix.get(elements, index)
                }
                // Too far to record: walk this read, and keep the prefix for
                // the reads it does serve.
                Kind::Prefix(_) if index >= MAX_INDEXED_ELEMENTS => None,
                Kind::Prefix(prefix) => {
                    let found = prefix.get(elements, index);
                    // Charged for what it holds now, not for `index`, which a
                    // read past the end of a short array overstates.
                    make_room(&mut state.entries, 1, prefix.len());
                    found
                }
                Kind::Seen { .. } | Kind::Refused => None,
            };
            state.entries.push(entry);
            match found {
                Some(element) => {
                    note_hit();
                    Read::Answered(element)
                }
                None => Read::Walk,
            }
        })
    }

    /// Retire the least recently used indexes (to [`Kind::Refused`]) and
    /// prefixes (to `Seen` with [`NO_PREFIX`], so the length route can still
    /// index the array) until `incoming` more of `elements` elements fit both
    /// the count and the element budget.
    fn make_room(entries: &mut [Entry], incoming: usize, elements: usize) {
        let (mut count, mut held) = entries.iter().fold((0, 0), |(c, n), e| match &e.kind {
            Kind::Indexed(ix) => (c + 1, n + ix.len()),
            Kind::Prefix(p) => (c + 1, n + p.len()),
            _ => (c, n),
        });
        // Oldest first; the loop ends without room only when a single index
        // is larger than the whole element budget, which MAX_INDEXED_ELEMENTS
        // (equal to the budget) rules out.
        for entry in entries.iter_mut() {
            if count + incoming <= INDEXES && held + elements <= ELEMENT_BUDGET {
                return;
            }
            let (size, retired) = match &entry.kind {
                Kind::Indexed(ix) => (ix.len(), Kind::Refused),
                Kind::Prefix(p) => (
                    p.len(),
                    Kind::Seen {
                        lookups: 0,
                        len: 0,
                        reads: NO_PREFIX,
                    },
                ),
                Kind::Seen { .. } | Kind::Refused => continue,
            };
            entry.kind = retired;
            count -= 1;
            held -= size;
        }
    }

    /// Add an entry for the list whose head id is `id`, dropping the oldest
    /// when the memo is full.
    fn register(state: &mut State, id: usize, kind: Kind) {
        if state.entries.len() >= ENTRIES {
            state.entries.remove(0);
        }
        state.entries.push(Entry { head: id, kind });
        BLOOM.with(|b| b.set(b.get() | bit(id)));
    }

    /// Remember that an element read walked `elements` past
    /// [`WIDE_ELEMENTS`] and found its element, so later element reads of the
    /// same list count toward a prefix.
    pub(crate) fn note_wide_read<E: DocumentElements>(elements: &E) {
        let Some((id, document)) = elements.head_id() else {
            return; // patchcov: coverage tolerate-line reason="unreachable: a list whose walk just found an element has a head"
        };
        MEMO.with(|m| {
            let mut m = m.borrow_mut();
            let Some(state) = m.as_mut().filter(|s| s.document == document) else {
                return;
            };
            if state.entries.iter().any(|e| e.head == id) {
                return; // patchcov: coverage tolerate-line reason="defensive: `element` answers Unregistered only when no entry exists, and nothing registers between that probe and the walk"
            }
            register(
                state,
                id,
                Kind::Seen {
                    lookups: 0,
                    len: 0,
                    reads: 1,
                },
            );
        });
    }

    /// Remember that a walk of `elements` was wide, so the next length lookup
    /// of the same list counts toward building an index.
    pub(crate) fn note_wide<E: DocumentElements>(elements: &E, len: usize) {
        let Some((id, document)) = elements.head_id() else {
            return; // patchcov: coverage tolerate-line reason="unreachable: a list that just walked to a length of WIDE_ELEMENTS or more has a head"
        };
        MEMO.with(|m| {
            let mut m = m.borrow_mut();
            let Some(state) = m.as_mut().filter(|s| s.document == document) else {
                return;
            };
            if let Some(entry) = state.entries.iter_mut().find(|e| e.head == id) {
                // Registered by element reads, or handed back from a prefix,
                // before any length was known: keep it to size the build.
                if let Kind::Seen { len: known, .. } = &mut entry.kind {
                    *known = len;
                }
                return;
            }
            register(
                state,
                id,
                Kind::Seen {
                    lookups: 1,
                    len,
                    reads: 0,
                },
            );
        });
    }
}

/// `no_std` has no `thread_local!`: the memo never exists, every read walks.
#[cfg(not(feature = "std"))]
pub(crate) mod memo {
    use super::DocumentElements;

    pub(crate) struct Guard;

    pub(crate) fn enter(_document: usize) -> Guard {
        Guard
    }

    pub(crate) fn len<E: DocumentElements>(_elements: &E) -> Option<usize> {
        None
    }

    pub(crate) fn get<E: DocumentElements>(_elements: &E, _index: usize) -> super::Read<E::Cursor> {
        super::Read::Walk
    }

    pub(crate) fn note_wide_read<E: DocumentElements>(_elements: &E) {}

    pub(crate) fn note_wide<E: DocumentElements>(_elements: &E, _len: usize) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jq::document::DocumentValue;
    use crate::json::JsonIndex;

    fn wide_array(elements: usize) -> String {
        let body: Vec<String> = (0..elements).map(|i| format!("{{\"i\":{i}}}")).collect();
        format!("[{}]", body.join(","))
    }

    /// Run `check` over the root array of `doc`.
    fn with_root_array(doc: &str, check: impl FnOnce(&crate::json::light::JsonElements<'_>)) {
        let index = JsonIndex::build(doc.as_bytes());
        let root = index.root(doc.as_bytes());
        let elements = root.value().as_array().expect("an array document");
        check(&elements);
    }

    /// What an element read answered, comparable across the walk and the
    /// index: the node's id, or `None` past the end.
    fn shown<C: DocumentCursor>(c: Option<C>) -> Option<usize> {
        c.map(|c| c.node_id())
    }

    #[test]
    fn index_agrees_with_the_walk_on_every_index_and_past_the_end_4035() {
        let doc = wide_array(130);
        with_root_array(&doc, |elements| {
            let index = ElementIndex::build(elements, 0).expect("a well-formed array indexes");
            let head = elements.uncons_cursor().expect("a non-empty array").0;
            assert_eq!(Ok(index.len()), elements.len_checked());
            for k in 0..135 {
                assert_eq!(
                    index.get(&head, k).map(|e| match e {
                        Element::Found(c) => Some(c.node_id()),
                        Element::Past => None,
                    }),
                    Some(shown(elements.get_cursor(k))),
                    "element {k}"
                );
            }
        });
    }

    #[test]
    fn build_refuses_what_the_walk_raises_on_4035() {
        for doc in ["[1,2,]", "[1,,2]", "[,1,2]", "[1,2,3,tru]", "[,]"] {
            with_root_array(doc, |elements| {
                if elements.len_checked().is_err() {
                    assert!(ElementIndex::build(elements, 0).is_none(), "{doc}");
                }
            });
        }
        // At least the trailing comma is one the walk raises on, so the pin
        // above is not vacuous.
        with_root_array("[1,2,]", |elements| {
            assert!(elements.len_checked().is_err());
            assert!(ElementIndex::build(elements, 0).is_none());
        });
    }

    /// The build is a copy of `len_checked`'s loop, so the module's "cannot
    /// change an answer" argument rests on the two agreeing. Pin it over every
    /// arrangement of up to four elements (scalar, malformed scalar, empty and
    /// string members) with every separator, leading and trailing delimiter
    /// the semi-index will accept: an index exists exactly when the walk
    /// succeeds, with the walk's length.
    #[test]
    fn build_succeeds_exactly_when_len_checked_does_4035() {
        let members = ["1", "tru", "[]", "\"a\"", "{\"k\":1}"];
        let mut checked = 0;
        for count in 0..=4usize {
            let mut picks = vec![0usize; count];
            loop {
                let els: Vec<&str> = picks.iter().map(|i| members[*i]).collect();
                for joiner in [",", ",,", " ", ""] {
                    for lead in ["", ","] {
                        for trail in ["", ",", ",,", " ,"] {
                            let doc = format!("[{lead}{}{trail}]", els.join(joiner));
                            let index = JsonIndex::build(doc.as_bytes());
                            let root = index.root(doc.as_bytes());
                            let elements = root.value().as_array().expect("an array document");
                            let walked = elements.len_checked();
                            let built = ElementIndex::build(&elements, 0);
                            assert_eq!(
                                built.as_ref().map(ElementIndex::len),
                                walked.as_ref().ok().copied(),
                                "{doc}"
                            );
                            checked += 1;
                        }
                    }
                }
                // Advance the odometer.
                let mut at = count;
                loop {
                    if at == 0 {
                        break;
                    }
                    at -= 1;
                    picks[at] += 1;
                    if picks[at] < members.len() {
                        break;
                    }
                    picks[at] = 0;
                    if at == 0 {
                        at = usize::MAX;
                        break;
                    }
                }
                if count == 0 || at == usize::MAX {
                    break;
                }
            }
        }
        assert!(checked > 1000, "the sweep covered {checked} documents");
    }

    #[test]
    fn build_refuses_an_array_longer_than_the_cap_4035() {
        let doc = wide_array(5);
        with_root_array(&doc, |elements| {
            assert!(ElementIndex::build_within(elements, 5, 4).is_none());
            assert_eq!(
                ElementIndex::build_within(elements, 5, 5).map(|ix| ix.len()),
                Some(5)
            );
        });
    }

    #[cfg(feature = "std")]
    mod memo_flow {
        use super::*;

        fn lens(doc: &str, times: usize) -> Vec<Result<usize, String>> {
            let index = JsonIndex::build(doc.as_bytes());
            let root = index.root(doc.as_bytes());
            let _scope = memo::enter(root.document_token());
            let elements = root.value().as_array().expect("an array document");
            (0..times)
                .map(|_| len_checked_memoized(&elements).map_err(|e| format!("{e:?}")))
                .collect()
        }

        #[test]
        fn a_wide_array_builds_on_its_third_length_lookup_4035() {
            let doc = wide_array(WIDE_ELEMENTS * 2);
            let before = memo::work();
            let answers = lens(&doc, 6);
            let (builds, hits) = memo::work();
            assert_eq!(builds - before.0, 1, "one build, not one per lookup");
            assert_eq!(hits - before.1, 4, "every lookup after the building one");
            assert!(answers.iter().all(|a| *a == Ok(WIDE_ELEMENTS * 2)));
        }

        #[test]
        fn element_reads_use_the_index_but_never_build_one_4035() {
            let doc = wide_array(WIDE_ELEMENTS * 2);
            let index = JsonIndex::build(doc.as_bytes());
            let root = index.root(doc.as_bytes());
            let _scope = memo::enter(root.document_token());
            let elements = root.value().as_array().expect("an array document");
            let before = memo::work();
            // One read: length (registers) then the element (walks).
            assert!(len_checked_memoized(&elements).is_ok());
            let walked = shown(elements.get_cursor(7));
            assert_eq!(shown(get_cursor_memoized(&elements, 7)), walked);
            assert_eq!(memo::work(), before, "a single read builds nothing");
            // The second read still walks: an array read twice must not pay
            // for an index it would use once.
            assert!(len_checked_memoized(&elements).is_ok());
            assert_eq!(memo::work(), before, "two reads build nothing");
            // The third builds (length), and the element read after it hits.
            assert!(len_checked_memoized(&elements).is_ok());
            let (builds, hits) = memo::work();
            assert_eq!(builds - before.0, 1);
            for k in [0usize, 7, 255, 256, 1000] {
                assert_eq!(
                    shown(get_cursor_memoized(&elements, k)),
                    shown(elements.get_cursor(k)),
                    "element {k}"
                );
            }
            assert!(memo::work().1 - hits >= 5, "the index answered the reads");
        }

        #[test]
        fn a_small_array_is_never_registered_4035() {
            let doc = wide_array(WIDE_ELEMENTS - 1);
            let before = memo::work();
            let answers = lens(&doc, 6);
            assert_eq!(memo::work(), before);
            assert!(answers.iter().all(|a| *a == Ok(WIDE_ELEMENTS - 1)));
        }

        #[test]
        fn an_array_read_once_is_never_indexed_4035() {
            let doc = wide_array(WIDE_ELEMENTS * 4);
            let before = memo::work();
            lens(&doc, 1);
            assert_eq!(memo::work(), before);
        }

        #[test]
        fn a_malformed_array_is_walked_every_time_and_raises_every_time_4035() {
            // Wide, and malformed in each place the walk checks: the walk
            // raises, so the array is never registered (only a walk that
            // succeeded registers one), and it keeps raising what it raised.
            // `build_refuses_what_the_walk_raises_on_4035` pins the build's own
            // checks, which this path never reaches. Scalar elements: the walk
            // checks the delimiters around those.
            let body: Vec<String> = (0..WIDE_ELEMENTS * 2).map(|i| i.to_string()).collect();
            let mid = body.len() / 2;
            let docs = [
                format!("[{},]", body.join(",")),
                format!("[{},,{}]", body[..mid].join(","), body[mid..].join(",")),
                format!("[,{}]", body.join(",")),
            ];
            for doc in docs {
                let before = memo::work();
                let answers = lens(&doc, 5);
                assert_eq!(
                    memo::work(),
                    before,
                    "a malformed array is neither built nor answered"
                );
                assert!(answers.iter().all(Result::is_err), "{answers:?}");
            }
        }

        #[test]
        fn more_wide_arrays_than_slots_stay_correct_and_bounded_4035() {
            // Twenty wide arrays in one scope: more than the four indexes and the
            // sixteen entries the memo keeps, so retirements and the oldest
            // entries falling out both happen. Every answer is the walk's,
            // whichever arrays happen to hold an index.
            let inner = wide_array(WIDE_ELEMENTS + 3);
            let doc = format!("[{}]", vec![inner; 20].join(","));
            let index = JsonIndex::build(doc.as_bytes());
            let root = index.root(doc.as_bytes());
            let _scope = memo::enter(root.document_token());
            let outer = root.value().as_array().expect("an array document");
            let arrays: Vec<_> = outer
                .collect_values()
                .iter()
                .map(|v| v.as_array().expect("an inner array"))
                .collect();
            assert_eq!(arrays.len(), 20);
            let check = |n: usize, round: usize| {
                let elements = &arrays[n];
                assert_eq!(
                    len_checked_memoized(elements).map_err(|e| format!("{e:?}")),
                    Ok(WIDE_ELEMENTS + 3),
                    "array {n} round {round}"
                );
                assert_eq!(
                    shown(get_cursor_memoized(elements, 5)),
                    shown(elements.get_cursor(5)),
                    "array {n} round {round}"
                );
            };
            let before = memo::work();
            // Six arrays read in turn, five times each: six builds against four
            // slots, so the oldest indexes are retired as the newer are built.
            for n in 0..6 {
                for round in 0..5 {
                    check(n, round);
                }
            }
            let (builds, _) = memo::work();
            assert_eq!(builds - before.0, 6, "each array built once");
            // The retired ones are walked for good, and still right.
            for round in 5..8 {
                check(0, round);
                check(1, round);
            }
            assert_eq!(memo::work().0, builds, "a retired array is not rebuilt");
            // Twenty registrations overflow the sixteen entries the memo keeps.
            for n in 6..20 {
                check(n, 0);
            }
            check(0, 9);
            check(19, 9);
        }

        #[test]
        fn a_partly_consumed_list_is_not_answered_from_the_whole_arrays_index_4035() {
            let doc = wide_array(WIDE_ELEMENTS * 2);
            let index = JsonIndex::build(doc.as_bytes());
            let root = index.root(doc.as_bytes());
            let _scope = memo::enter(root.document_token());
            let elements = root.value().as_array().expect("an array document");
            for _ in 0..3 {
                assert!(len_checked_memoized(&elements).is_ok());
            }
            let (_, rest) = elements.uncons().expect("a first element");
            // The tail has one element fewer, whatever the whole array's
            // index says, round after round.
            let walked = rest.len_checked().map_err(|e| format!("{e:?}"));
            for round in 0..4 {
                assert_eq!(
                    len_checked_memoized(&rest).map_err(|e| format!("{e:?}")),
                    walked,
                    "round {round}"
                );
            }
        }

        /// Every element read of `order` over `elements` through the memo, in
        /// one scope, against the walk; returns the elements the prefixes
        /// recorded.
        fn element_loop<E: DocumentElements>(
            elements: &E,
            token: usize,
            order: impl Iterator<Item = usize>,
        ) -> usize {
            let _scope = memo::enter(token);
            let before = recorded();
            for k in order {
                assert_eq!(
                    shown(get_cursor_memoized(elements, k)),
                    shown(elements.get_cursor(k)),
                    "element {k}"
                );
            }
            recorded() - before
        }

        #[test]
        fn an_element_read_loop_walks_each_element_once_in_either_order_4162() {
            let n = 300;
            let doc = wide_array(n);
            let index = JsonIndex::build(doc.as_bytes());
            let root = index.root(doc.as_bytes());
            let elements = root.value().as_array().expect("an array document");
            let token = root.document_token();
            // Past the end on both sides of the loop, too.
            let ascending = element_loop(&elements, token, (0..n).chain([n, n + 7]));
            let descending = element_loop(&elements, token, (0..n).rev().chain([n, n + 7]));
            // Ascending: below WIDE_ELEMENTS nothing is registered, the first
            // wide read registers, the second walks, the third records up to
            // itself, and each later read extends by one: every element at
            // most once.
            assert!(ascending > 0 && ascending <= n, "ascending {ascending}");
            // Descending: the first read registers, the second walks, the
            // third records the array up to itself, every later read is a hit,
            // and the reads past the end record the rest on their way there.
            assert_eq!(descending, n, "descending");
        }

        #[test]
        fn a_yaml_sequence_resumes_its_walk_where_it_stopped_4162() {
            // Block items whose value starts on the next line (the list stands
            // at the `-` wrapper, the element is its child), nested and flow
            // sequences, nulls, block scalars and comments: the frontier must
            // rebuild the list the walk left, not the element it resolved to.
            let items = [
                "- a\n",
                "-\n  k: 1\n  j: 2\n",
                "- [1, 2]\n",
                "- - x\n  - y\n",
                "-\n",
                "- |\n  text\n",
                "- # note\n  m: 3\n",
                "- {f: 1}\n",
            ];
            let yaml: String = (0..150).map(|i| items[i % items.len()]).collect();
            let index = crate::yaml::YamlIndex::build(yaml.as_bytes()).expect("valid YAML");
            let root = index.root(yaml.as_bytes());
            let elements = root
                .first_child()
                .expect("document content")
                .value()
                .as_array()
                .expect("a sequence");
            assert_eq!(elements.len(), 150);
            let token = root.document_token();
            let ascending = element_loop(&elements, token, (0..152).chain([3, 99]));
            let descending = element_loop(&elements, token, (0..152).rev());
            let strided = element_loop(&elements, token, (0..150).step_by(7).rev());
            assert!(ascending > 0 && ascending <= 150, "ascending {ascending}");
            assert!(
                descending > 0 && descending <= 150,
                "descending {descending}"
            );
            assert!(strided > 0 && strided <= 150, "strided {strided}");
        }

        #[test]
        fn an_array_read_twice_records_nothing_and_thrice_records_once_4162() {
            let doc = wide_array(WIDE_ELEMENTS * 4);
            let index = JsonIndex::build(doc.as_bytes());
            let root = index.root(doc.as_bytes());
            let _scope = memo::enter(root.document_token());
            let elements = root.value().as_array().expect("an array document");
            let k = WIDE_ELEMENTS + 36;
            let (before, work) = (recorded(), memo::work());
            assert_eq!(
                shown(get_cursor_memoized(&elements, k)),
                shown(elements.get_cursor(k))
            );
            assert_eq!((recorded(), memo::work()), (before, work), "read once");
            assert_eq!(
                shown(get_cursor_memoized(&elements, k)),
                shown(elements.get_cursor(k))
            );
            assert_eq!((recorded(), memo::work()), (before, work), "read twice");
            assert_eq!(
                shown(get_cursor_memoized(&elements, k)),
                shown(elements.get_cursor(k))
            );
            assert_eq!(recorded() - before, k + 1, "the third read records");
            assert_eq!(memo::work().0, work.0, "and builds no index");
            // A fourth read inside the prefix walks nothing and is a hit.
            let hits = memo::work().1;
            assert_eq!(
                shown(get_cursor_memoized(&elements, 5)),
                shown(elements.get_cursor(5))
            );
            assert_eq!(recorded() - before, k + 1);
            assert_eq!(memo::work().1 - hits, 1);
        }

        #[test]
        fn a_short_read_of_an_unregistered_array_never_registers_it_4162() {
            let doc = wide_array(WIDE_ELEMENTS * 4);
            let index = JsonIndex::build(doc.as_bytes());
            let root = index.root(doc.as_bytes());
            let _scope = memo::enter(root.document_token());
            let elements = root.value().as_array().expect("an array document");
            let (before, work) = (recorded(), memo::work());
            for round in 0..3 {
                for k in 0..WIDE_ELEMENTS {
                    assert_eq!(
                        shown(get_cursor_memoized(&elements, k)),
                        shown(elements.get_cursor(k)),
                        "element {k} round {round}"
                    );
                }
            }
            assert_eq!((recorded(), memo::work()), (before, work));
        }

        /// The value route reads the length before every element: it must stay
        /// on #4035's rule (build on the third length lookup) and never record.
        #[test]
        fn a_length_lookup_before_each_element_read_never_starts_a_prefix_4162() {
            let doc = wide_array(WIDE_ELEMENTS * 4);
            let index = JsonIndex::build(doc.as_bytes());
            let root = index.root(doc.as_bytes());
            let _scope = memo::enter(root.document_token());
            let elements = root.value().as_array().expect("an array document");
            let (before, work) = (recorded(), memo::work());
            for round in 0..6 {
                assert_eq!(
                    len_checked_memoized(&elements).map_err(|e| format!("{e:?}")),
                    Ok(WIDE_ELEMENTS * 4)
                );
                let k = 200 - round;
                assert_eq!(
                    shown(get_cursor_memoized(&elements, k)),
                    shown(elements.get_cursor(k)),
                    "round {round}"
                );
            }
            assert_eq!(recorded(), before, "nothing recorded");
            assert_eq!(
                memo::work().0 - work.0,
                1,
                "built once, on the third lookup"
            );
        }

        #[test]
        fn a_length_lookup_hands_a_prefix_back_to_the_index_4162() {
            let doc = wide_array(WIDE_ELEMENTS * 4);
            let index = JsonIndex::build(doc.as_bytes());
            let root = index.root(doc.as_bytes());
            let _scope = memo::enter(root.document_token());
            let elements = root.value().as_array().expect("an array document");
            let work = memo::work();
            for k in [200, 150, 100] {
                assert_eq!(
                    shown(get_cursor_memoized(&elements, k)),
                    shown(elements.get_cursor(k))
                );
            }
            // Three length lookups: the first hands the prefix back, the third
            // builds; every one answers the walk's length.
            for _ in 0..3 {
                assert_eq!(
                    len_checked_memoized(&elements).map_err(|e| format!("{e:?}")),
                    Ok(WIDE_ELEMENTS * 4)
                );
            }
            assert_eq!(memo::work().0 - work.0, 1);
            let recorded_before = recorded();
            for k in [250, 3, WIDE_ELEMENTS * 4] {
                assert_eq!(
                    shown(get_cursor_memoized(&elements, k)),
                    shown(elements.get_cursor(k))
                );
            }
            assert_eq!(recorded(), recorded_before, "the index answers");
        }

        /// One length lookup ahead of a loop of element reads (`.[-1]`, then
        /// `.[$i]` in a path context) must not leave the loop walking.
        #[test]
        fn an_element_loop_after_one_length_lookup_still_records_4162() {
            let n = WIDE_ELEMENTS * 4;
            let doc = wide_array(n);
            let index = JsonIndex::build(doc.as_bytes());
            let root = index.root(doc.as_bytes());
            let _scope = memo::enter(root.document_token());
            let elements = root.value().as_array().expect("an array document");
            assert!(len_checked_memoized(&elements).is_ok());
            let before = recorded();
            for k in (0..n).rev() {
                assert_eq!(
                    shown(get_cursor_memoized(&elements, k)),
                    shown(elements.get_cursor(k)),
                    "element {k}"
                );
            }
            // Three walks (two ahead of the one lookup), then the fourth
            // records up to itself and every later read is a hit.
            assert_eq!(recorded() - before, n - 3);
        }

        #[test]
        fn a_read_past_the_cap_walks_and_keeps_the_prefix_4162() {
            let doc = wide_array(WIDE_ELEMENTS * 4);
            let index = JsonIndex::build(doc.as_bytes());
            let root = index.root(doc.as_bytes());
            let _scope = memo::enter(root.document_token());
            let elements = root.value().as_array().expect("an array document");
            for _ in 0..3 {
                assert!(get_cursor_memoized(&elements, 100).is_some());
            }
            // Too far to record: walked (and past the end), the prefix kept.
            let before = recorded();
            assert!(get_cursor_memoized(&elements, MAX_INDEXED_ELEMENTS).is_none());
            assert_eq!(recorded(), before);
            let hits = memo::work().1;
            for k in [100, 50, 0] {
                assert_eq!(
                    shown(get_cursor_memoized(&elements, k)),
                    shown(elements.get_cursor(k))
                );
            }
            assert_eq!(memo::work().1 - hits, 3, "the prefix still answers");
            // Once the walk has reached the end, the prefix is the whole
            // array: a read at any distance is past the end, answered without
            // a walk.
            assert!(get_cursor_memoized(&elements, WIDE_ELEMENTS * 4 + 9).is_none());
            let (before, hits) = (recorded(), memo::work().1);
            assert!(get_cursor_memoized(&elements, MAX_INDEXED_ELEMENTS + 5).is_none());
            assert_eq!(recorded(), before);
            assert_eq!(memo::work().1 - hits, 1);
        }

        /// Review of #4162: a wide-index read past the end of a short array
        /// proves nothing about its width, so it must not register the array
        /// (which would let the length route index a small array and take a
        /// slot).
        #[test]
        fn a_read_past_the_end_of_a_short_array_never_registers_it_4162() {
            let doc = wide_array(10);
            let index = JsonIndex::build(doc.as_bytes());
            let root = index.root(doc.as_bytes());
            let _scope = memo::enter(root.document_token());
            let elements = root.value().as_array().expect("an array document");
            let (before, work) = (recorded(), memo::work());
            for _ in 0..4 {
                assert!(get_cursor_memoized(&elements, WIDE_ELEMENTS + 36).is_none());
                assert_eq!(
                    len_checked_memoized(&elements).map_err(|e| format!("{e:?}")),
                    Ok(10)
                );
            }
            assert_eq!((recorded(), memo::work()), (before, work));
        }

        /// Review of #4162: a prefix is charged for what it recorded, not for
        /// the index read, so a read far past the end of a short array does
        /// not retire the other arrays' indexes.
        #[test]
        fn a_far_read_past_the_end_retires_no_other_index_4162() {
            let a = wide_array(WIDE_ELEMENTS * 2);
            let b = wide_array(WIDE_ELEMENTS + 3);
            let doc = format!("[{a},{b}]");
            let index = JsonIndex::build(doc.as_bytes());
            let root = index.root(doc.as_bytes());
            let _scope = memo::enter(root.document_token());
            let outer = root.value().as_array().expect("an array document");
            let arrays: Vec<_> = outer
                .collect_values()
                .iter()
                .map(|v| v.as_array().expect("an inner array"))
                .collect();
            let builds = memo::work().0;
            for _ in 0..3 {
                assert!(len_checked_memoized(&arrays[0]).is_ok());
            }
            assert_eq!(memo::work().0 - builds, 1, "array a is indexed");
            for _ in 0..3 {
                assert!(get_cursor_memoized(&arrays[1], WIDE_ELEMENTS + 1).is_some());
            }
            assert!(get_cursor_memoized(&arrays[1], MAX_INDEXED_ELEMENTS - 1).is_none());
            // a's index still answers its length: a retired one would walk.
            let hits = memo::work().1;
            assert_eq!(
                len_checked_memoized(&arrays[0]).map_err(|e| format!("{e:?}")),
                Ok(WIDE_ELEMENTS * 2)
            );
            assert_eq!(memo::work().1 - hits, 1);
        }

        #[test]
        fn more_prefixes_than_slots_retire_the_oldest_and_stay_right_4162() {
            let inner = wide_array(WIDE_ELEMENTS + 3);
            let doc = format!("[{}]", vec![inner; 8].join(","));
            let index = JsonIndex::build(doc.as_bytes());
            let root = index.root(doc.as_bytes());
            let _scope = memo::enter(root.document_token());
            let outer = root.value().as_array().expect("an array document");
            let arrays: Vec<_> = outer
                .collect_values()
                .iter()
                .map(|v| v.as_array().expect("an inner array"))
                .collect();
            let read = |n: usize, k: usize| {
                assert_eq!(
                    shown(get_cursor_memoized(&arrays[n], k)),
                    shown(arrays[n].get_cursor(k)),
                    "array {n} element {k}"
                );
            };
            // Eight prefixes against four slots.
            for n in 0..8 {
                for k in [WIDE_ELEMENTS + 2, WIDE_ELEMENTS, 1] {
                    read(n, k);
                }
            }
            // The retired walk for good and record nothing; the newest are
            // still answered from their prefixes.
            let before = recorded();
            for k in [WIDE_ELEMENTS + 1, 0] {
                read(0, k);
                read(1, k);
            }
            assert_eq!(recorded(), before, "a retired prefix is not restarted");
            let hits = memo::work().1;
            read(7, WIDE_ELEMENTS + 1);
            assert_eq!(memo::work().1 - hits, 1);
            // A retired prefix leaves the length route open (review of #4162):
            // three length lookups index the array, as they would have before
            // element reads ever touched it.
            let builds = memo::work().0;
            for _ in 0..3 {
                assert_eq!(
                    len_checked_memoized(&arrays[0]).map_err(|e| format!("{e:?}")),
                    Ok(WIDE_ELEMENTS + 3)
                );
            }
            assert_eq!(memo::work().0 - builds, 1);
            read(0, WIDE_ELEMENTS + 2);
        }
    }
}
