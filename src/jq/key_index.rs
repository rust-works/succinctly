//! A per-object key index, so a keyed read of a wide document object is one
//! hash probe instead of a walk of every member (#3913).
//!
//! `DocumentFields::find_cursor` answers "last member whose key is `name`"
//! by visiting every member: the answer can always be superseded by a later
//! duplicate, and a malformed sibling has to raise whether or not it is the
//! one asked for. On an owned (decoded) object the same read is a hash
//! lookup, so the cost showed up only where a node of the *document* was
//! indexed -- `.big[$k]`, and `$b[.]` for a `$b` bound to a node of the
//! document the use site is reading (#2072) -- and grew linearly with the
//! key count (350x at 10,000 keys).
//!
//! # Why it cannot change an answer
//!
//! - The index is built in one pass over the member *keys* only
//!   ([`DocumentFields::uncons_key`]); no value is decoded, so validation
//!   stays "where something reads it" (#2692).
//! - **Every anomaly refuses the build** (a non-string key, an unpaired tail,
//!   a trailing stray comma, an object past [`MAX_INDEXED_MEMBERS`], or keys
//!   so repeated that their runs of slots outgrow the build's probe budget)
//!   and the
//!   caller runs the walk it always ran, which raises exactly what it always
//!   raised. So an index exists only for an object the walk would not have
//!   raised on for *any* name, and what is left to check per lookup is the
//!   one thing that depends on the name: the winning member's own `,`/`:`
//!   delimiters.
//! - A key with no reliable identity (it will not decode) is not entered,
//!   which is what the walk's `as_str().is_ok_and(..)` does too (#1247).
//! - Last duplicate wins: a probe keeps the highest-numbered entry whose
//!   decoded key equals the name, in both evaluation modes (the walk does not
//!   look at the mode).
//!
//! # When one is built
//!
//! Not on first sight. `find_cursor_counted` reports how many members a walk
//! visited; only a walk of [`WIDE_MEMBERS`] or more registers the object, and
//! the *next* lookup of it builds the index. An object that is small, or
//! looked up once, never costs more than the walk it always cost, and a
//! refused build is remembered so it is not retried. So is an eviction: only
//! four indexes are kept, and an object whose index was dropped goes back to
//! the walk for good rather than being rebuilt each time a pipeline cycles
//! past it.
//!
//! # Scope
//!
//! An entry names a node by its position in one document's tree, and a
//! [`DocumentCursor::document_token`] is not a security boundary (an index
//! dropped and rebuilt can reuse an address). So the memo lives only inside
//! an evaluation's scope ([`memo::enter`], opened beside `slot_memo`'s) and
//! is consulted only for that document's own cursors; nothing outlives the
//! document it describes. `no_std` has no `thread_local!`, so there the
//! memo never exists and every lookup is the walk.

// `no_std` never builds an index (no `thread_local!` to hold one), so there
// everything below the one-line `find_cursor_memoized` is unused.
#![cfg_attr(not(feature = "std"), allow(dead_code))]

#[cfg(not(test))]
use alloc::vec::Vec;

use super::document::{
    key_delimiter_ok, key_hash, key_hash_of, key_is_malformed, key_only_value_delimiter_ok,
    last_field_trailing_gap_ok, DocumentCursor, DocumentFields, DocumentValue,
};
use super::error::EvalError;

/// A walk of at least this many members registers an object as wide.
///
/// Below it the walk is already cheaper than the thread-local probe plus a
/// build; the figure is where the second lookup of an object stops being
/// slower with an index than without one (see the #3913 measurements in the
/// PR).
pub(crate) const WIDE_MEMBERS: usize = 64;

/// An object with more members than this keeps the walk: the index costs
/// about 24-40 bytes per member, and a document this wide is better served
/// by the caller restructuring the query than by a second copy of its keys.
pub(crate) const MAX_INDEXED_MEMBERS: usize = 1 << 21;

/// One slot of the open-addressed table: the high half of the key's hash,
/// and the entry it names plus one (`0` is an empty slot).
#[derive(Clone, Copy, Default)]
struct Slot {
    hash: u32,
    entry: u32,
}

/// The keys of one object, by hash.
pub(crate) struct KeyIndex {
    /// The key node id of every member that has an identity, in document order.
    keys: Vec<usize>,
    table: Vec<Slot>,
    mask: usize,
    /// Whether a `has` walk of this object would find nothing to raise on
    /// (#4002), decided by [`Self::walk_is_clean`] the first time a `has`
    /// asks, so a `find`-only object never pays for it.
    clean: core::cell::Cell<Option<bool>>,
}

impl KeyIndex {
    /// Index the members `fields` still holds, or `None` when the object is
    /// anything but plainly well-formed (see the module doc: a refusal sends
    /// the caller to the walk, which says what is wrong).
    pub(crate) fn build<F: DocumentFields>(fields: &F) -> Option<Self> {
        Self::build_within(fields, MAX_INDEXED_MEMBERS)
    }

    /// [`build`](Self::build) for an object of at most `max_members` members.
    fn build_within<F: DocumentFields>(fields: &F, max_members: usize) -> Option<Self> {
        let mut keys = Vec::new();
        let mut hashes: Vec<u64> = Vec::new();
        let mut members = 0usize;
        let mut last_key_cursor = None;
        let mut rest = fields.clone();
        while let Some((key, key_cursor, next)) = rest.uncons_key() {
            members += 1;
            if members > max_members {
                return None;
            }
            // A hash exists only for a key that decoded, so asking for it first
            // decodes each key once; the malformed test is for a key without one.
            if let Some(hash) = key_hash_of(&key) {
                keys.push(key_cursor.node_id());
                hashes.push(hash);
            } else if key_is_malformed(&key) {
                return None;
            }
            last_key_cursor = Some(key_cursor);
            rest = next;
        }
        if rest.ends_unpaired() || !last_field_trailing_gap_ok(last_key_cursor, b'}') {
            return None;
        }
        let size = (keys.len() * 2).next_power_of_two().max(8);
        let mask = size - 1;
        let mut table = alloc_table(size);
        // Equal keys share a hash and so a run of slots, and inserting into a
        // run of m costs m probes: a million repeats of one key would be
        // quadratic to build, and every lookup of it a scan of the run. The
        // walk is linear, so an object that clusters this badly keeps it: the
        // budget lets a run grow to about 4 * sqrt(members).
        let mut budget = 8 * keys.len() + 64;
        for (entry, hash) in hashes.iter().enumerate() {
            let mut at = (*hash as usize) & mask;
            while table[at].entry != 0 {
                budget = budget.checked_sub(1)?;
                at = (at + 1) & mask;
            }
            table[at] = Slot {
                hash: (*hash >> 32) as u32,
                entry: entry as u32 + 1,
            };
        }
        Some(Self {
            keys,
            table,
            mask,
            clean: core::cell::Cell::new(None),
        })
    }

    /// What `contains_checked(name)` answers for the list `fields` is, or
    /// `None` when the index cannot say (the caller then walks) (#4002).
    ///
    /// The walk's early exit makes its errors depend on where the match sits
    /// (#1739, #2261, #2288): a malformed sibling raises only if visited
    /// before it. The build already refused every anomaly that does not depend
    /// on the name (a non-string key, an unpaired tail, a trailing comma), so
    /// what is left is a member whose key will not decode, or whose `,`/`:` is
    /// missing or doubled. An object holding either keeps the walk, which
    /// raises exactly where it always did; for every other object the answer
    /// is "some entry's decoded key equals the name".
    pub(crate) fn contains<F: DocumentFields>(
        &self,
        fields: &F,
        head: &F::Cursor,
        name: &str,
    ) -> Option<bool> {
        if !self.walk_is_clean(fields) {
            return None;
        }
        match self.lookup(head, name)? {
            Ok(found) => Some(found.is_some()),
            Err(_) => None,
        }
    }

    /// Whether `fields`, the list this index was built from, has no member a
    /// `contains_checked` walk could raise on: every key decodes and every
    /// key's `,` and `:` are in place. One key-only pass, run once.
    fn walk_is_clean<F: DocumentFields>(&self, fields: &F) -> bool {
        if let Some(clean) = self.clean.get() {
            return clean;
        }
        let mut is_first = true;
        let mut rest = fields.clone();
        let mut clean = true;
        while let Some((key, key_cursor, next)) = rest.uncons_key() {
            if !matches!(key.decoded_key_str(), Ok(Some(_)))
                || !key_delimiter_ok::<F>(&key, &key_cursor, is_first)
                || !key_only_value_delimiter_ok::<F>(&key, &key_cursor)
            {
                clean = false;
                break;
            }
            is_first = false;
            rest = next;
        }
        self.clean.set(Some(clean));
        clean
    }

    /// How many members the index holds.
    pub(crate) fn len(&self) -> usize {
        self.keys.len()
    }

    /// What `find_cursor(name)` answers for the list `head` starts, or `None`
    /// when the index cannot say (the caller then walks).
    ///
    /// `head` is the list's first key node. The winner is the last entry
    /// whose key decodes to `name`; its own delimiters are then checked the
    /// way the walk checks the winner's (and only the winner's).
    pub(crate) fn lookup<C: DocumentCursor>(
        &self,
        head: &C,
        name: &str,
    ) -> Option<Result<Option<C>, EvalError>> {
        let hash = key_hash(name.as_bytes());
        let high = (hash >> 32) as u32;
        let mut at = (hash as usize) & self.mask;
        let mut winner: Option<(usize, C)> = None;
        loop {
            let slot = self.table[at];
            if slot.entry == 0 {
                break;
            }
            let entry = slot.entry as usize - 1;
            if slot.hash == high && winner.as_ref().map_or(true, |(w, _)| entry > *w) {
                let key_cursor = head.at_node_id(self.keys[entry])?;
                let value = key_cursor.value();
                if matches!(value.decoded_key_str(), Ok(Some(k)) if k == name) {
                    winner = Some((entry, key_cursor));
                }
            }
            at = (at + 1) & self.mask;
        }
        let Some((entry, key_cursor)) = winner else {
            return Some(Ok(None));
        };
        let value_cursor = key_cursor.next_sibling()?;
        let is_first = self.keys[entry] == head.node_id();
        let expected = if is_first { None } else { Some(b',') };
        let key_ok = key_cursor
            .text_position()
            .map_or(true, |at| key_cursor.preceding_delimiter_ok(at, expected));
        if !key_ok {
            return Some(Err(key_cursor.malformed_delimiter_error()));
        }
        let value_ok = value_cursor.text_position().map_or(true, |at| {
            value_cursor.preceding_delimiter_ok(at, Some(b':'))
        });
        if !value_ok {
            return Some(Err(value_cursor.malformed_delimiter_error()));
        }
        Some(Ok(Some(value_cursor)))
    }
}

fn alloc_table(size: usize) -> Vec<Slot> {
    let mut table = Vec::with_capacity(size);
    table.resize(size, Slot::default());
    table
}

/// [`DocumentFields::find_cursor`] for a caller inside an evaluation: the
/// same answer, answered from a [`KeyIndex`] once the object has proved wide.
#[inline]
pub(crate) fn find_cursor_memoized<F: DocumentFields>(
    fields: &F,
    name: &str,
) -> Result<Option<F::Cursor>, EvalError> {
    if let Some(answer) = memo::answer(fields, name) {
        return answer;
    }
    let (found, walked) = fields.find_cursor_counted(name);
    if walked >= WIDE_MEMBERS && found.is_ok() {
        memo::note_wide(fields);
    }
    found
}

/// [`DocumentFields::contains_checked`] for a caller inside an evaluation: the
/// same answer, from a [`KeyIndex`] once the object has proved wide (#4002).
#[inline]
pub(crate) fn contains_memoized<F: DocumentFields>(
    fields: &F,
    name: &str,
) -> Result<bool, EvalError> {
    if let Some(answer) = memo::answer_has(fields, name) {
        return Ok(answer);
    }
    let (found, walked) = fields.contains_checked_counted(name);
    if walked >= WIDE_MEMBERS && found.is_ok() {
        memo::note_wide(fields);
    }
    found
}

#[cfg(feature = "std")]
pub(crate) mod memo {
    use std::cell::{Cell, RefCell};

    use super::{DocumentCursor, DocumentFields, EvalError, KeyIndex, MAX_INDEXED_MEMBERS};

    /// Objects remembered at once, indexed or not. Least recently used first.
    const ENTRIES: usize = 16;
    /// Indexes kept at once. A build in progress holds the fourth's place, and
    /// its transient hash vector comes on top of the retained size.
    const INDEXES: usize = 4;
    /// Members indexed across all the indexes kept.
    const MEMBER_BUDGET: usize = MAX_INDEXED_MEMBERS;

    enum Kind {
        /// A wide walk has been seen: the next lookup builds.
        Seen,
        /// Always walk: the build was refused (malformed, clustered, too
        /// wide), or the index was evicted. An evicted object is not given a
        /// second chance: a pipeline cycling over more wide objects than
        /// there are indexes would otherwise rebuild one on nearly every
        /// lookup, and a build costs more than the walk it replaces.
        Refused,
        Indexed(KeyIndex),
    }

    struct Entry {
        /// The node id of the first key of the list this entry describes.
        head: usize,
        kind: Kind,
    }

    struct State {
        document: usize,
        entries: Vec<Entry>,
    }

    thread_local! {
        static MEMO: RefCell<Option<State>> = const { RefCell::new(None) };
        /// Whether the open scope holds any entry. A plain `Cell` with a
        /// constant initializer needs no destructor registration, so the
        /// test every lookup makes costs a load, where reaching into `MEMO`
        /// (a `Vec` inside, so a lazily registered destructor) cost about
        /// 60 instructions per lookup of a small object.
        static ARMED: Cell<bool> = const { Cell::new(false) };
    }

    // What the memo did, for the tests that pin its cost: indexes built, and
    // lookups an index answered.
    #[cfg(test)]
    thread_local! {
        static WORK: std::cell::Cell<(usize, usize)> = const { std::cell::Cell::new((0, 0)) };
    }

    #[cfg(test)]
    pub(crate) fn work() -> (usize, usize) {
        WORK.with(std::cell::Cell::get)
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

    /// What a [`Guard`] puts back when it drops.
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

    /// Answer a lookup from the memo: `None` means "walk" (nothing remembered
    /// for this list, or the index could not say).
    #[inline]
    pub(crate) fn answer<F: DocumentFields>(
        fields: &F,
        name: &str,
    ) -> Option<Result<Option<F::Cursor>, EvalError>> {
        // The common case -- an evaluation that has seen no wide object --
        // costs this one test.
        if !ARMED.with(Cell::get) {
            return None;
        }
        answer_armed(fields, |index, head| index.lookup(head, name))
    }

    /// [`answer`] for `has(name)` (#4002): whether the object holds the key,
    /// or `None` to walk.
    #[inline]
    pub(crate) fn answer_has<F: DocumentFields>(fields: &F, name: &str) -> Option<bool> {
        if !ARMED.with(Cell::get) {
            return None;
        }
        answer_armed(fields, |index, head| index.contains(fields, head, name))
    }

    #[inline(never)]
    fn answer_armed<F: DocumentFields, R>(
        fields: &F,
        probe: impl FnOnce(&KeyIndex, &F::Cursor) -> Option<R>,
    ) -> Option<R> {
        let head = fields.head_key_cursor()?;
        let id = head.node_id();
        let document = head.document_token();
        MEMO.with(|m| {
            let mut m = m.borrow_mut();
            let state = m.as_mut().filter(|s| s.document == document)?;
            let at = state.entries.iter().position(|e| e.head == id)?;
            let mut entry = state.entries.remove(at);
            if matches!(entry.kind, Kind::Seen) {
                // The second lookup of a wide object: build. Room by count
                // first, so the indexes held while this one is built are
                // INDEXES - 1 at most; by members once its size is known.
                make_room(&mut state.entries, 1, 0);
                entry.kind = match KeyIndex::build(fields) {
                    Some(index) => {
                        note_build();
                        make_room(&mut state.entries, 1, index.len());
                        Kind::Indexed(index)
                    }
                    None => Kind::Refused,
                };
            }
            let found = match &entry.kind {
                Kind::Indexed(index) => {
                    let found = probe(index, &head);
                    if found.is_some() {
                        note_hit();
                    }
                    found
                }
                Kind::Seen | Kind::Refused => None,
            };
            state.entries.push(entry);
            found
        })
    }

    /// Retire the least recently used indexes (to [`Kind::Refused`]) until
    /// `incoming` more indexes of `members` members fit both the count and the
    /// member budget.
    fn make_room(entries: &mut [Entry], incoming: usize, members: usize) {
        loop {
            let (count, held) = entries.iter().fold((0, 0), |(c, n), e| match &e.kind {
                Kind::Indexed(ix) => (c + 1, n + ix.len()),
                _ => (c, n),
            });
            if count + incoming <= INDEXES && held + members <= MEMBER_BUDGET {
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

    /// Remember that a walk of `fields` was wide, so the next lookup of the
    /// same list builds an index.
    pub(crate) fn note_wide<F: DocumentFields>(fields: &F) {
        let Some(head) = fields.head_key_cursor() else {
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
                kind: Kind::Seen,
            });
            ARMED.with(|a| a.set(true));
        });
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn make_room_gives_up_when_no_index_is_left_to_retire_3913() {
            // An object wider than the whole budget cannot be made room for by
            // retiring anything: there is nothing held, and the loop must end.
            let mut entries: Vec<Entry> = Vec::new();
            make_room(&mut entries, 1, MEMBER_BUDGET + 1);
            assert!(entries.is_empty());
        }
    }
}

/// `no_std` has no `thread_local!`: the memo never exists, every lookup walks.
#[cfg(not(feature = "std"))]
pub(crate) mod memo {
    use super::{DocumentFields, EvalError};

    pub(crate) struct Guard;

    pub(crate) fn enter(_document: usize) -> Guard {
        Guard
    }

    pub(crate) fn answer<F: DocumentFields>(
        _fields: &F,
        _name: &str,
    ) -> Option<Result<Option<F::Cursor>, EvalError>> {
        None
    }

    pub(crate) fn answer_has<F: DocumentFields>(_fields: &F, _name: &str) -> Option<bool> {
        None
    }

    pub(crate) fn note_wide<F: DocumentFields>(_fields: &F) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::json::JsonIndex;

    /// What a lookup answered, comparable across the walk and the index: the
    /// value node's id, or the error's text.
    fn shown<C: DocumentCursor>(r: Result<Option<C>, EvalError>) -> Result<Option<usize>, String> {
        r.map(|c| c.map(|c| c.node_id()))
            .map_err(|e| format!("{e:?}"))
    }

    fn wide_object(members: usize) -> String {
        let body: Vec<String> = (0..members).map(|i| format!("\"k{i}\":{i}")).collect();
        format!("{{{}}}", body.join(","))
    }

    /// Run `check` over the root object of `doc`.
    fn with_root_object(doc: &str, check: impl FnOnce(&crate::json::light::JsonFields<'_>)) {
        let index = JsonIndex::build(doc.as_bytes());
        let root = index.root(doc.as_bytes());
        let fields = root.value().as_object().expect("an object document");
        check(&fields);
    }

    /// The index answers what the walk answers, for every name asked.
    fn assert_agrees(doc: &str, names: &[&str]) {
        with_root_object(doc, |fields| {
            let index = KeyIndex::build(fields).expect("a well-formed object indexes");
            let head = fields.head_key_cursor().expect("a non-empty object");
            for name in names {
                let walked = shown(fields.find_cursor(name));
                let probed = index
                    .lookup(&head, name)
                    .map(shown)
                    .expect("the index answers");
                assert_eq!(probed, walked, "{name:?} in {doc}");
            }
        });
    }

    #[test]
    fn index_agrees_with_the_walk_on_present_absent_and_repeated_keys_3913() {
        let mut doc = wide_object(200);
        doc.truncate(doc.len() - 1);
        doc.push_str(r#","k5":"again","k5":"and again","":"empty"}"#);
        assert_agrees(
            &doc,
            &["k0", "k5", "k199", "k200", "missing", "", "K5", "k"],
        );
    }

    #[test]
    fn index_agrees_on_escaped_and_non_ascii_keys_3913() {
        // `"ab"` spells `ab`; `é` and a literal `é` are one key; a
        // lone surrogate will not decode and is skipped, as the walk skips it
        // (#1247) without hiding the keys after it.
        let doc = r#"{"ab":1,"é":2,"é":3,"\ud800":4,"after":5,"tab\there":6,"plain":7}"#;
        assert_agrees(
            doc,
            &[
                "ab",
                "a\\u0062",
                "é",
                "after",
                "plain",
                "tab\there",
                "\u{fffd}",
            ],
        );
    }

    #[test]
    fn index_agrees_when_only_a_non_winning_member_has_a_bad_delimiter_3913() {
        // `"b"` follows `1` with no `,`: the walk raises only for a lookup
        // whose winner is `"b"`, never for `"a"` or an absent name, and the
        // index must draw the same line.
        let doc = r#"{"a":1 "b":2,"c":3}"#;
        assert_agrees(doc, &["a", "b", "c", "absent"]);
        let doc = r#"{"a":1,"b" 2,"c":3}"#;
        assert_agrees(doc, &["a", "b", "c", "absent"]);
        let doc = r#"{"a":1,,"b":2,"c":3}"#;
        assert_agrees(doc, &["a", "b", "c", "absent"]);
    }

    /// Two different keys that share the high half of their hash and the
    /// first table slot, found by search (a 35-bit match is ~2^17.5 keys).
    fn colliding_keys() -> (String, String) {
        let mut seen = std::collections::HashMap::new();
        for i in 0u32.. {
            let key = format!("c{i}");
            let hash = key_hash(key.as_bytes());
            let signature = (hash >> 32, hash & 7);
            if let Some(other) = seen.insert(signature, key.clone()) {
                return (other, key);
            }
        }
        unreachable!("a collision exists well before u32::MAX keys") // patchcov: coverage tolerate-line reason="unreachable: the search above ends at the first collision, which the birthday bound puts near 2^17 keys"
    }

    #[test]
    fn a_hash_match_is_confirmed_against_the_key_itself_3913() {
        let (a, b) = colliding_keys();
        assert_ne!(a, b);
        // Both land in the 8-slot table's same first slot with the same
        // stored hash, so only comparing the keys tells them apart.
        assert_agrees(&format!("{{\"{a}\":1,\"{b}\":2}}"), &[&a, &b, "absent"]);
        assert_agrees(&format!("{{\"{b}\":1,\"{a}\":2}}"), &[&a, &b, "absent"]);
    }

    #[test]
    fn build_refuses_every_object_the_walk_would_raise_on_3913() {
        for doc in [
            r#"{"a":1,123:2}"#,
            r#"{"a":1,}"#,
            r#"{"a":1,"b"}"#,
            "{invalid}",
            r#"{"a":1,"b":2,}"#,
        ] {
            with_root_object(doc, |fields| {
                assert!(KeyIndex::build(fields).is_none(), "{doc}");
            });
        }
    }

    #[test]
    fn build_refuses_an_object_whose_keys_all_cluster_3913() {
        // Equal keys share a hash, so a run of them is a run of slots: the
        // build is quadratic in the run and a lookup scans all of it.
        let repeated = format!("{{{}}}", vec!["\"a\":1"; 3000].join(","));
        with_root_object(&repeated, |fields| {
            assert!(KeyIndex::build(fields).is_none());
        });
        // A few repeats of many keys is ordinary, and indexes.
        let few: Vec<String> = (0..200)
            .flat_map(|i| (0..3).map(move |j| format!("\"k{i}\":{j}")))
            .collect();
        let few = format!("{{{}}}", few.join(","));
        with_root_object(&few, |fields| {
            assert!(KeyIndex::build(fields).is_some());
        });
        assert_agrees(&few, &["k0", "k100", "k199", "absent"]);
    }

    #[test]
    fn build_refuses_nothing_else_3913() {
        for doc in [r#"{"a":1}"#, r#"{"a":{"b":[1,2,{"c":3}]},"d":null}"#] {
            with_root_object(doc, |fields| {
                assert!(KeyIndex::build(fields).is_some(), "{doc}");
            });
        }
    }

    #[test]
    fn build_refuses_an_object_wider_than_the_cap_3913() {
        let doc = wide_object(5);
        with_root_object(&doc, |fields| {
            assert!(KeyIndex::build_within(fields, 4).is_none());
            assert_eq!(
                KeyIndex::build_within(fields, 5).map(|ix| ix.len()),
                Some(5)
            );
        });
    }

    #[cfg(feature = "std")]
    mod memo_flow {
        use super::*;

        fn lookups(doc: &str, name: &str, times: usize) -> Vec<Result<Option<usize>, String>> {
            let index = JsonIndex::build(doc.as_bytes());
            let root = index.root(doc.as_bytes());
            let _scope = memo::enter(root.document_token());
            let fields = root.value().as_object().expect("an object document");
            (0..times)
                .map(|_| shown(find_cursor_memoized(&fields, name)))
                .collect()
        }

        #[test]
        fn a_wide_object_builds_on_its_second_lookup_3913() {
            let doc = wide_object(WIDE_MEMBERS * 2);
            let before = memo::work();
            let answers = lookups(&doc, "k100", 6);
            let (builds, hits) = memo::work();
            assert_eq!(builds - before.0, 1, "one build, not one per lookup");
            assert_eq!(hits - before.1, 5, "every lookup after the first");
            assert!(answers.iter().all(|a| *a == answers[0]), "{answers:?}");
            assert!(matches!(answers[0], Ok(Some(_))));
        }

        #[test]
        fn a_small_object_is_never_registered_3913() {
            let doc = wide_object(WIDE_MEMBERS - 1);
            let before = memo::work();
            let answers = lookups(&doc, "k3", 6);
            assert_eq!(memo::work(), before);
            assert!(answers.iter().all(|a| *a == answers[0]));
        }

        #[test]
        fn an_object_looked_up_once_is_never_indexed_3913() {
            let doc = wide_object(WIDE_MEMBERS * 4);
            let before = memo::work();
            lookups(&doc, "k1", 1);
            assert_eq!(memo::work(), before);
        }

        #[test]
        fn a_refused_object_is_walked_every_time_and_built_once_3913() {
            // Wide, and malformed only after the part a lookup of `k1` reads:
            // the build refuses, and the walk keeps raising what it raised.
            let mut doc = wide_object(WIDE_MEMBERS * 2);
            doc.truncate(doc.len() - 1);
            doc.push_str(",}");
            let before = memo::work();
            let answers = lookups(&doc, "k1", 5);
            assert_eq!(
                memo::work(),
                before,
                "a refusal is neither a build nor a hit"
            );
            assert!(answers.iter().all(Result::is_err), "{answers:?}");
        }

        #[test]
        fn a_partly_consumed_list_is_not_answered_from_the_whole_objects_index_3913() {
            let doc = wide_object(WIDE_MEMBERS * 2);
            let index = JsonIndex::build(doc.as_bytes());
            let root = index.root(doc.as_bytes());
            let _scope = memo::enter(root.document_token());
            let fields = root.value().as_object().expect("an object document");
            for _ in 0..3 {
                assert!(matches!(find_cursor_memoized(&fields, "k0"), Ok(Some(_))));
            }
            let (_, rest) = fields.uncons().expect("a first member");
            // `k0` is the consumed member: the tail does not hold it.
            // Whatever the walk says of a tail -- `k0` is not in it, and `k1`
            // is the tail's first member, which the walk's own first-member
            // rule then finds preceded by a `,` -- the memo says too, round
            // after round, through its build.
            for name in ["k0", "k1", "k7"] {
                let walked = shown(rest.find_cursor(name));
                for round in 0..4 {
                    assert_eq!(
                        shown(find_cursor_memoized(&rest, name)),
                        walked,
                        "{name} round {round}"
                    );
                }
            }
        }

        #[test]
        fn a_refused_clustered_object_is_walked_not_built_3913() {
            let doc = format!("{{{}}}", vec!["\"a\":1"; 3000].join(","));
            let before = memo::work();
            let answers = lookups(&doc, "a", 5);
            assert_eq!(memo::work(), before, "refused: neither a build nor a hit");
            assert!(answers.iter().all(|a| *a == answers[0]));
            assert!(matches!(answers[0], Ok(Some(_))));
        }

        #[test]
        fn more_wide_objects_than_indexes_do_not_rebuild_on_every_round_3913() {
            // Six wide objects read in a cycle, with room for four indexes:
            // an evicted object goes back to the walk and is not rebuilt each
            // time round (a build costs more than the walk), so the builds
            // are bounded by the objects, not by the lookups.
            let objects = 6;
            let members = WIDE_MEMBERS * 2;
            let doc = format!(
                "{{{}}}",
                (0..objects)
                    .map(|o| {
                        let inner: Vec<String> =
                            (0..members).map(|i| format!("\"k{i}\":{o}")).collect();
                        format!("\"o{o}\":{{{}}}", inner.join(","))
                    })
                    .collect::<Vec<_>>()
                    .join(",")
            );
            let index = JsonIndex::build(doc.as_bytes());
            let root = index.root(doc.as_bytes());
            let _scope = memo::enter(root.document_token());
            let top = root.value().as_object().expect("an object document");
            let inner: Vec<_> = (0..objects)
                .map(|o| {
                    let c = top.find_cursor(&format!("o{o}")).unwrap().expect("member");
                    c.value().as_object().expect("an object")
                })
                .collect();
            let before = memo::work();
            for round in 0..8 {
                for (o, fields) in inner.iter().enumerate() {
                    let want = shown(fields.find_cursor("k7"));
                    assert_eq!(
                        shown(find_cursor_memoized(fields, "k7")),
                        want,
                        "object {o} round {round}"
                    );
                }
            }
            let builds = memo::work().0 - before.0;
            // Each object is built once at most, however many rounds.
            assert!(
                builds <= objects,
                "{builds} builds over {objects} objects x 8 rounds"
            );
        }

        #[test]
        fn more_wide_objects_than_the_memo_remembers_still_answer_alike_3913() {
            // One more wide object than the memo has entries for: the oldest
            // entry is forgotten to make room, and every object still answers
            // as its walk does.
            let objects = 17;
            let members = WIDE_MEMBERS * 2;
            let doc = format!(
                "{{{}}}",
                (0..objects)
                    .map(|o| {
                        let inner: Vec<String> =
                            (0..members).map(|i| format!("\"k{i}\":{o}")).collect();
                        format!("\"o{o}\":{{{}}}", inner.join(","))
                    })
                    .collect::<Vec<_>>()
                    .join(",")
            );
            let index = JsonIndex::build(doc.as_bytes());
            let root = index.root(doc.as_bytes());
            let _scope = memo::enter(root.document_token());
            let top = root.value().as_object().expect("an object document");
            for o in 0..objects {
                let c = top.find_cursor(&format!("o{o}")).unwrap().expect("member");
                let fields = c.value().as_object().expect("an object");
                let want = shown(fields.find_cursor("k7"));
                for _ in 0..3 {
                    assert_eq!(shown(find_cursor_memoized(&fields, "k7")), want, "{o}");
                }
            }
        }

        #[test]
        fn a_yaml_mapping_is_never_registered_3913() {
            use crate::yaml::{YamlIndex, YamlValue};

            let yaml: String = (0..WIDE_MEMBERS * 2)
                .map(|i| format!("k{i}: {i}\n"))
                .collect();
            let index = YamlIndex::build(yaml.as_bytes()).unwrap();
            let root = index.root(yaml.as_bytes());
            let YamlValue::Sequence(docs) = root.value() else {
                panic!("a YAML root is a sequence of documents"); // patchcov: coverage tolerate-line reason="unreachable: YamlIndex::build always wraps parsed documents in a virtual root Sequence (#798)"
            };
            let Some(YamlValue::Mapping(fields)) = docs.into_iter().next() else {
                panic!("expected a mapping document"); // patchcov: coverage tolerate-line reason="unreachable: the document built above is a block mapping (#3913)"
            };
            assert!(fields.head_key_cursor().is_none());
            let _scope = memo::enter(root.document_token());
            let before = memo::work();
            // A format without the key index is told "not wide" by its own
            // walk; asking the memo to remember it anyway is a no-op.
            memo::note_wide(&fields);
            for _ in 0..4 {
                assert!(matches!(find_cursor_memoized(&fields, "k9"), Ok(Some(_))));
            }
            assert_eq!(memo::work(), before);
        }

        #[test]
        fn a_lookup_outside_any_scope_walks_3913() {
            let doc = wide_object(WIDE_MEMBERS * 2);
            let index = JsonIndex::build(doc.as_bytes());
            let root = index.root(doc.as_bytes());
            let fields = root.value().as_object().expect("an object document");
            let before = memo::work();
            for _ in 0..4 {
                assert!(matches!(find_cursor_memoized(&fields, "k9"), Ok(Some(_))));
            }
            assert_eq!(memo::work(), before);
        }

        #[test]
        fn another_documents_cursors_are_never_answered_from_this_ones_index_3913() {
            let doc = wide_object(WIDE_MEMBERS * 2);
            let other_doc = wide_object(WIDE_MEMBERS * 2).replace("\"k9\"", "\"zz\"");
            let index = JsonIndex::build(doc.as_bytes());
            let other_index = JsonIndex::build(other_doc.as_bytes());
            let root = index.root(doc.as_bytes());
            let other_root = other_index.root(other_doc.as_bytes());
            let _scope = memo::enter(root.document_token());
            let fields = root.value().as_object().expect("an object document");
            let other = other_root.value().as_object().expect("an object document");
            for _ in 0..3 {
                assert!(matches!(find_cursor_memoized(&fields, "k9"), Ok(Some(_))));
            }
            // Same shape, same node ids, different document and different keys.
            for _ in 0..3 {
                assert!(matches!(find_cursor_memoized(&other, "k9"), Ok(None)));
                assert!(matches!(find_cursor_memoized(&other, "zz"), Ok(Some(_))));
            }
        }
    }
}
