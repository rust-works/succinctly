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

/// [`DocumentElements::len_checked`] for a caller inside an evaluation: the
/// same answer, from an [`ElementIndex`] once the array has proved wide.
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
/// [`ElementIndex`] when one exists. Never builds one.
#[inline]
pub(crate) fn get_cursor_memoized<E: DocumentElements>(
    elements: &E,
    index: usize,
) -> Option<E::Cursor> {
    match memo::get(elements, index) {
        Some(Element::Found(cursor)) => Some(cursor),
        Some(Element::Past) => None,
        None => elements.get_cursor(index),
    }
}

#[cfg(feature = "std")]
pub(crate) mod memo {
    use std::cell::{Cell, RefCell};

    use super::{DocumentCursor, DocumentElements, ElementIndex, MAX_INDEXED_ELEMENTS};

    /// Arrays remembered at once, indexed or not. Least recently used first.
    const ENTRIES: usize = 16;
    /// Indexes kept at once.
    const INDEXES: usize = 4;
    /// The length lookup that builds is the one after this many walks of the
    /// array (the registering one included): the third lookup.
    const BUILD_AT: u8 = 2;
    /// Elements indexed across all the indexes kept.
    const ELEMENT_BUDGET: usize = MAX_INDEXED_ELEMENTS;

    enum Kind {
        /// Wide walks have been seen, this many length lookups of them so far
        /// (counting the one that registered it), for an array of this many
        /// elements: the third lookup builds, sized from the length.
        Seen {
            lookups: u8,
            len: usize,
        },
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
        /// Whether the open scope holds any entry: the one load every lookup
        /// of an evaluation that has seen no wide array makes.
        static ARMED: Cell<bool> = const { Cell::new(false) };
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
                ARMED.with(|a| a.set(previous.as_ref().is_some_and(|s| !s.entries.is_empty())));
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
            ARMED.with(|a| a.set(false));
            Guard(Restore::Previous(m.replace(State {
                document,
                entries: Vec::new(),
            })))
        })
    }

    /// The length of the list `elements`, if an index (built now, on the
    /// second length lookup of a wide array) can say; `None` means "walk".
    #[inline]
    pub(crate) fn len<E: DocumentElements>(elements: &E) -> Option<usize> {
        if !ARMED.with(Cell::get) {
            return None;
        }
        answer(elements, true, |index, _head| Some(index.len()))
    }

    /// Element `index` of the list `elements`, when an existing index can say,
    /// else `None` to walk. Never builds.
    #[inline]
    pub(crate) fn get<E: DocumentElements>(
        elements: &E,
        index: usize,
    ) -> Option<super::Element<E::Cursor>> {
        if !ARMED.with(Cell::get) {
            return None;
        }
        answer(elements, false, |ix, head| ix.get(head, index))
    }

    #[inline(never)]
    fn answer<E: DocumentElements, R>(
        elements: &E,
        may_build: bool,
        probe: impl FnOnce(&ElementIndex, &E::Cursor) -> Option<R>,
    ) -> Option<R> {
        let (head, _) = elements.uncons_cursor()?;
        let id = head.node_id();
        let document = head.document_token();
        MEMO.with(|m| {
            let mut m = m.borrow_mut();
            let state = m.as_mut().filter(|s| s.document == document)?;
            let at = state.entries.iter().position(|e| e.head == id)?;
            let mut entry = state.entries.remove(at);
            if may_build {
                if let Kind::Seen { lookups, len } = entry.kind {
                    if lookups < BUILD_AT {
                        entry.kind = Kind::Seen {
                            lookups: lookups + 1,
                            len,
                        };
                        state.entries.push(entry);
                        return None;
                    }
                    make_room(&mut state.entries, 1, 0);
                    entry.kind = match ElementIndex::build(elements, len) {
                        Some(index) => {
                            note_build();
                            make_room(&mut state.entries, 1, index.len());
                            Kind::Indexed(index)
                        }
                        None => Kind::Refused,
                    };
                }
            }
            let found = match &entry.kind {
                Kind::Indexed(index) => {
                    let found = probe(index, &head);
                    if found.is_some() {
                        note_hit();
                    }
                    found
                }
                Kind::Seen { .. } | Kind::Refused => None,
            };
            state.entries.push(entry);
            found
        })
    }

    /// Retire the least recently used indexes (to [`Kind::Refused`]) until
    /// `incoming` more indexes of `elements` elements fit both the count and
    /// the element budget.
    fn make_room(entries: &mut [Entry], incoming: usize, elements: usize) {
        loop {
            let (count, held) = entries.iter().fold((0, 0), |(c, n), e| match &e.kind {
                Kind::Indexed(ix) => (c + 1, n + ix.len()),
                _ => (c, n),
            });
            if count + incoming <= INDEXES && held + elements <= ELEMENT_BUDGET {
                return;
            }
            match entries
                .iter()
                .position(|e| matches!(e.kind, Kind::Indexed(_)))
            {
                Some(at) => entries[at].kind = Kind::Refused,
                None => return,
            }
        }
    }

    /// Remember that a walk of `elements` was wide, so the next length lookup
    /// of the same list builds an index.
    pub(crate) fn note_wide<E: DocumentElements>(elements: &E, len: usize) {
        let Some((head, _)) = elements.uncons_cursor() else {
            return;
        };
        let id = head.node_id();
        let document = head.document_token();
        MEMO.with(|m| {
            let mut m = m.borrow_mut();
            let Some(state) = m.as_mut().filter(|s| s.document == document) else {
                return;
            };
            if state.entries.iter().any(|e| e.head == id) {
                return;
            }
            if state.entries.len() >= ENTRIES {
                state.entries.remove(0);
            }
            state.entries.push(Entry {
                head: id,
                kind: Kind::Seen { lookups: 1, len },
            });
            ARMED.with(|a| a.set(true));
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

    pub(crate) fn get<E: DocumentElements>(
        _elements: &E,
        _index: usize,
    ) -> Option<super::Element<E::Cursor>> {
        None
    }

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
    }
}
