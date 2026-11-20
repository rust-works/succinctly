//! The collapsed view behind [`ObjectMapOf`](super::ObjectMapOf): an
//! insertion-ordered list of `(key, value)` fields, with a hash index only
//! once the object is larger than [`SMALL_FIELDS`] (#7260).
//!
//! # Why not `IndexMap`
//!
//! An `IndexMap` is the entry list plus a hash table plus its hasher, for
//! every object. A record of six fields paid three heap blocks: the
//! refcounted map (88 bytes requested, a 96-byte glibc chunk), the table
//! (88 bytes, 96), and six 64-byte entries (each carries its 8-byte hash), so
//! about 592 bytes against jq 1.7.1's one 392-byte block, and the table had to
//! be built (hashing every key) for an object that is looked up a handful of
//! times. A linear scan of a few keys is faster than hashing one (four lookups
//! on a six-field object took 12 ns against 42 ns in a micro-benchmark on a
//! Ryzen 9 7950X), so a small object here is only its entries: 56 bytes each,
//! no table, no hasher.
//!
//! # Representation
//!
//! Two shapes, three words either way (`Repr`):
//!
//! - **Small**, at most [`SMALL_FIELDS`] fields: the field list alone.
//!   Lookups scan it; an object of six fields is one 40-byte refcounted
//!   block (a 48-byte glibc chunk) and its six 56-byte fields.
//! - **Large**, past that: the same list plus a hash index, boxed, built the
//!   first time the object grows past the threshold and dropped once a removal
//!   brings it back. Each index element packs the key's 32-bit hash (high
//!   half) and its position (low half), so growing the table, cloning it and
//!   rejecting a probe never re-hash a key.
//!
//! Positions are the `IndexMap` positions they replace: `get_index`,
//! `swap_remove`'s permutation and `shift_remove`'s order are the same.
//!
//! The semantics are `IndexMap`'s: inserting an existing key replaces the
//! value and keeps the key's position (and its first key string);
//! `swap_remove` moves the last field into the hole; `==` ignores order.
//!
//! The index hashes with `IndexMap`'s own default (SipHash 1-3, seeded per
//! process), so a document's keys cannot be crafted against it any more than
//! against the map it replaces.

#[cfg(not(test))]
use alloc::boxed::Box;
#[cfg(not(test))]
use alloc::string::String;
#[cfg(not(test))]
use alloc::vec::Vec;

use core::hash::BuildHasher;

use hashbrown::HashTable;
use indexmap::IndexMap;

use super::SharableString;

/// One field: its key (inline, or shared with the same key of the records
/// around it, #7260) and its value.
type Field<V> = (SharableString, V);

/// The most fields an object holds without a hash index (#7260).
///
/// Up to this a lookup is a scan comparing key lengths and last bytes first,
/// which beats hashing the probe; past it the index pays for itself. 8 made a
/// 12-field record pay the index's box and table and measured +26% on building
/// 60,000 of them (Apple M4 Pro); 16 keeps such records plain.
pub const SMALL_FIELDS: usize = 16;

/// The hasher state `IndexMap` uses by default, named without `std` (the
/// library builds `no_std`; `indexmap` itself brings `std`'s `RandomState`).
trait DefaultStateOf {
    type State;
}

impl<K, V, S> DefaultStateOf for IndexMap<K, V, S> {
    type State = S;
}

type DefaultState = <IndexMap<(), ()> as DefaultStateOf>::State;

/// A fresh hasher state, seeded as a new `IndexMap`'s is.
#[inline]
fn new_state() -> DefaultState {
    IndexMap::<(), ()>::new().hasher().clone()
}

/// The hash index of a large [`FieldMap`]: one element per field.
#[derive(Clone)]
struct FieldIndex {
    table: HashTable<u64>,
    state: DefaultState,
}

/// Whether the stored key `stored` is the probe `probe`: lengths first, then
/// the last byte (numbered keys such as `field_1`/`field_2` share everything
/// before it), and only then the whole key, so a scan of a small object calls
/// `memcmp` about once per lookup.
#[inline(always)]
fn same_key(stored: &str, probe: &[u8]) -> bool {
    let stored = stored.as_bytes();
    stored.len() == probe.len()
        && match probe.last() {
            None => true,
            Some(last) => stored[stored.len() - 1] == *last && stored == probe,
        }
}

/// The 32-bit hash a field is indexed by.
#[inline]
fn short_hash(state: &DefaultState, key: &str) -> u32 {
    (state.hash_one(key) >> 32) as u32
}

/// The table hash of a 32-bit key hash: the same bits in both halves, so
/// hashbrown's bucket (low bits) and tag (top bits) both come from it and a
/// rehash can recompute it from the element alone.
#[inline]
const fn table_hash(hash: u32) -> u64 {
    ((hash as u64) << 32) | hash as u64
}

#[inline]
const fn element(hash: u32, pos: usize) -> u64 {
    ((hash as u64) << 32) | pos as u64
}

#[inline]
const fn element_hash(element: u64) -> u32 {
    (element >> 32) as u32
}

#[inline]
const fn element_pos(element: u64) -> usize {
    element as u32 as usize
}

#[inline]
fn rehash(element: &u64) -> u64 {
    table_hash(element_hash(*element))
}

impl FieldIndex {
    fn with_capacity(capacity: usize) -> Self {
        Self {
            table: HashTable::with_capacity(capacity),
            state: new_state(),
        }
    }

    /// Indexes the field at `pos` (whose key hashes to `hash`).
    #[inline]
    fn insert(&mut self, hash: u32, pos: usize) {
        assert!(
            u32::try_from(pos).is_ok(),
            "an object field index past u32::MAX"
        );
        self.table
            .insert_unique(table_hash(hash), element(hash, pos), rehash);
    }

    /// Un-indexes the field at `pos`.
    #[inline]
    fn erase(&mut self, hash: u32, pos: usize) {
        if let Ok(found) = self
            .table
            .find_entry(table_hash(hash), |&e| element_pos(e) == pos)
        {
            found.remove();
        } else {
            debug_assert!(false, "an indexed field is missing from the table");
        }
    }

    /// Re-points the element for the field at `from` (hash `hash`) to `to`.
    #[inline]
    fn relocate(&mut self, hash: u32, from: usize, to: usize) {
        if let Some(found) = self
            .table
            .find_mut(table_hash(hash), |&e| element_pos(e) == from)
        {
            *found = element(hash, to);
        } else {
            debug_assert!(false, "an indexed field is missing from the table");
        }
    }

    /// Indexes every field of `entries` afresh.
    fn rebuild<V>(&mut self, entries: &[Field<V>]) {
        self.table.clear();
        self.table.reserve(entries.len(), rehash);
        for (pos, (key, _)) in entries.iter().enumerate() {
            let hash = short_hash(&self.state, key.as_str());
            self.insert(hash, pos);
        }
    }
}

/// An insertion-ordered map from `String` keys to `V` with `IndexMap`'s
/// semantics, stored as a plain field list while small (#7260). See the
/// module documentation.
pub struct FieldMap<V>(Repr<V>);

/// [`FieldMap`]'s two shapes. Three words either way (the `Large` arm sits in
/// the field list's capacity niche), so the refcounted block an object holds
/// is 40 bytes, a 48-byte glibc chunk, where a field list beside an
/// `Option<Box<_>>` index would be 48 bytes and a 64-byte chunk.
enum Repr<V> {
    /// At most [`SMALL_FIELDS`] fields: looked up by scanning.
    Small(Vec<Field<V>>),
    /// More: the same list plus its hash index.
    Large(Box<Large<V>>),
}

/// A [`FieldMap`] past [`SMALL_FIELDS`] fields.
struct Large<V> {
    entries: Vec<Field<V>>,
    index: FieldIndex,
}

impl<V> Large<V> {
    /// [`FieldMap::insert_full`] in one probe of the table, as `IndexMap`'s
    /// own insert is: a miss leaves the probe at the slot the new field takes.
    #[inline]
    fn insert_full(&mut self, key: SharableString, value: V) -> (usize, Option<V>) {
        let hash = short_hash(&self.index.state, key.as_str());
        let entries = &self.entries;
        match self.index.table.entry(
            table_hash(hash),
            |&e| element_hash(e) == hash && entries[element_pos(e)].0.as_str() == key.as_str(),
            rehash,
        ) {
            hashbrown::hash_table::Entry::Occupied(found) => {
                let pos = element_pos(*found.get());
                (
                    pos,
                    Some(core::mem::replace(&mut self.entries[pos].1, value)),
                )
            }
            hashbrown::hash_table::Entry::Vacant(slot) => {
                let pos = self.entries.len();
                assert!(
                    u32::try_from(pos).is_ok(),
                    "an object field index past u32::MAX"
                );
                slot.insert(element(hash, pos));
                self.reserve_one();
                self.entries.push((key, value));
                (pos, None)
            }
        }
    }

    /// Appends a field whose key is known to be absent and hashes to `hash`.
    #[inline]
    fn push_absent(&mut self, key: SharableString, value: V, hash: u32) -> usize {
        let pos = self.entries.len();
        self.index.insert(hash, pos);
        self.reserve_one();
        self.entries.push((key, value));
        pos
    }

    /// Room for one more field, growing the list to what the table already
    /// holds rather than doubling it, as `IndexMap` sizes its entries: a list
    /// of 34 for a table of 28 would be six fields (336 bytes) never used.
    #[inline]
    fn reserve_one(&mut self) {
        let len = self.entries.len();
        if len == self.entries.capacity() {
            let target = self.index.table.capacity().max(len + 1);
            self.entries.reserve_exact(target - len);
        }
    }
}

impl<V: Clone> Clone for Repr<V> {
    #[inline]
    fn clone(&self) -> Self {
        match self {
            Self::Small(entries) => Self::Small(entries.clone()),
            Self::Large(large) => Self::Large(Box::new(Large {
                entries: large.entries.clone(),
                index: large.index.clone(),
            })),
        }
    }
}

/// Where a key is, or the hash a new one would be indexed by.
enum Probe {
    Found(usize),
    /// Absent; the key's short hash when the map is indexed.
    Absent(Option<u32>),
}

impl<V> FieldMap<V> {
    /// An empty map; does not allocate.
    #[inline]
    pub const fn new() -> Self {
        Self(Repr::Small(Vec::new()))
    }

    /// An empty map with room for `capacity` fields: indexed from the start
    /// when that is past [`SMALL_FIELDS`], so a large object built to a known
    /// size never scans or re-indexes on the way.
    #[inline]
    pub fn with_capacity(capacity: usize) -> Self {
        if capacity > SMALL_FIELDS {
            return Self(Repr::Large(Box::new(Large {
                entries: Vec::with_capacity(capacity),
                index: FieldIndex::with_capacity(capacity),
            })));
        }
        Self(Repr::Small(Vec::with_capacity(capacity)))
    }

    #[inline]
    fn entries(&self) -> &Vec<Field<V>> {
        match &self.0 {
            Repr::Small(entries) => entries,
            Repr::Large(large) => &large.entries,
        }
    }

    /// The field list, for a change that keeps every key at its position (a
    /// value write) -- or one whose caller fixes the index up itself.
    #[inline]
    fn entries_mut(&mut self) -> &mut Vec<Field<V>> {
        match &mut self.0 {
            Repr::Small(entries) => entries,
            Repr::Large(large) => &mut large.entries,
        }
    }

    /// The hash index, when the map has one (the tests' view of the shape).
    #[cfg(test)]
    fn index(&self) -> Option<&FieldIndex> {
        match &self.0 {
            Repr::Small(_) => None,
            Repr::Large(large) => Some(&large.index),
        }
    }

    /// How many fields the map holds.
    #[inline]
    pub fn len(&self) -> usize {
        self.entries().len()
    }

    /// Whether the map holds no field.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.entries().is_empty()
    }

    /// How many fields fit before the field list reallocates.
    #[inline]
    pub fn capacity(&self) -> usize {
        self.entries().capacity()
    }

    #[inline]
    fn probe<Q>(&self, key: &Q) -> Probe
    where
        Q: ?Sized + AsRef<str>,
    {
        let key = key.as_ref();
        match &self.0 {
            Repr::Small(entries) => {
                let probe = key.as_bytes();
                match entries.iter().position(|(k, _)| same_key(k, probe)) {
                    Some(pos) => Probe::Found(pos),
                    None => Probe::Absent(None),
                }
            }
            Repr::Large(large) => {
                let hash = short_hash(&large.index.state, key);
                let entries = &large.entries;
                match large.index.table.find(table_hash(hash), |&e| {
                    element_hash(e) == hash && entries[element_pos(e)].0.as_str() == key
                }) {
                    Some(&e) => Probe::Found(element_pos(e)),
                    None => Probe::Absent(Some(hash)),
                }
            }
        }
    }

    /// The position of `key`, if present.
    #[inline]
    pub fn get_index_of<Q>(&self, key: &Q) -> Option<usize>
    where
        Q: ?Sized + AsRef<str>,
    {
        match self.probe(key) {
            Probe::Found(pos) => Some(pos),
            Probe::Absent(_) => None,
        }
    }

    /// Whether `key` is present.
    #[inline]
    pub fn contains_key<Q>(&self, key: &Q) -> bool
    where
        Q: ?Sized + AsRef<str>,
    {
        self.get_index_of(key).is_some()
    }

    /// The value for `key`.
    #[inline]
    pub fn get<Q>(&self, key: &Q) -> Option<&V>
    where
        Q: ?Sized + AsRef<str>,
    {
        let pos = self.get_index_of(key)?;
        Some(&self.entries()[pos].1)
    }

    /// The stored key and the value for `key`.
    #[inline]
    pub fn get_key_value<Q>(&self, key: &Q) -> Option<(&String, &V)>
    where
        Q: ?Sized + AsRef<str>,
    {
        let pos = self.get_index_of(key)?;
        let (k, v) = &self.entries()[pos];
        Some((&**k, v))
    }

    /// The position, stored key and value for `key`.
    #[inline]
    pub fn get_full<Q>(&self, key: &Q) -> Option<(usize, &String, &V)>
    where
        Q: ?Sized + AsRef<str>,
    {
        let pos = self.get_index_of(key)?;
        let (k, v) = &self.entries()[pos];
        Some((pos, &**k, v))
    }

    /// The value for `key`, mutably.
    #[inline]
    pub fn get_mut<Q>(&mut self, key: &Q) -> Option<&mut V>
    where
        Q: ?Sized + AsRef<str>,
    {
        let pos = self.get_index_of(key)?;
        Some(&mut self.entries_mut()[pos].1)
    }

    /// The stored key and the value for `key`, mutably.
    #[inline]
    pub fn get_key_value_mut<Q>(&mut self, key: &Q) -> Option<(&String, &mut V)>
    where
        Q: ?Sized + AsRef<str>,
    {
        let pos = self.get_index_of(key)?;
        let (k, v) = &mut self.entries_mut()[pos];
        Some((&**k, v))
    }

    /// The position, stored key and value for `key`, mutably.
    #[inline]
    pub fn get_full_mut<Q>(&mut self, key: &Q) -> Option<(usize, &String, &mut V)>
    where
        Q: ?Sized + AsRef<str>,
    {
        let pos = self.get_index_of(key)?;
        let (k, v) = &mut self.entries_mut()[pos];
        Some((pos, &**k, v))
    }

    /// The field at position `pos`.
    #[inline]
    pub fn get_index(&self, pos: usize) -> Option<(&String, &V)> {
        self.entries().get(pos).map(|(k, v)| (&**k, v))
    }

    /// The field at position `pos`, its value mutably.
    #[inline]
    pub fn get_index_mut(&mut self, pos: usize) -> Option<(&String, &mut V)> {
        self.entries_mut().get_mut(pos).map(|(k, v)| (&**k, v))
    }

    /// The first field.
    #[inline]
    pub fn first(&self) -> Option<(&String, &V)> {
        self.entries().first().map(|(k, v)| (&**k, v))
    }

    /// The first field, its value mutably.
    #[inline]
    pub fn first_mut(&mut self) -> Option<(&String, &mut V)> {
        self.entries_mut().first_mut().map(|(k, v)| (&**k, v))
    }

    /// The last field.
    #[inline]
    pub fn last(&self) -> Option<(&String, &V)> {
        self.entries().last().map(|(k, v)| (&**k, v))
    }

    /// The last field, its value mutably.
    #[inline]
    pub fn last_mut(&mut self) -> Option<(&String, &mut V)> {
        self.entries_mut().last_mut().map(|(k, v)| (&**k, v))
    }

    /// Appends a field whose key is known to be absent; `hash` is the key's
    /// short hash when the map is indexed. Returns its position.
    #[inline]
    fn push_absent(&mut self, key: SharableString, value: V, hash: Option<u32>) -> usize {
        if let Repr::Small(entries) = &mut self.0 {
            if entries.len() < SMALL_FIELDS {
                let pos = entries.len();
                entries.push((key, value));
                return pos;
            }
            // The field that takes the map past the threshold: index the
            // fields so far first, so the list grows once, to the table's size.
            self.grow_index();
        }
        match &mut self.0 {
            Repr::Large(large) => {
                let hash = hash.unwrap_or_else(|| short_hash(&large.index.state, key.as_str()));
                large.push_absent(key, value, hash)
            }
            Repr::Small(_) => unreachable!("grow_index leaves the map indexed"),
        }
    }

    /// Moves a field list about to outgrow [`SMALL_FIELDS`] (or one that
    /// did, from a bulk build) into the indexed shape.
    #[cold]
    #[inline(never)]
    fn grow_index(&mut self) {
        if let Repr::Small(entries) = &mut self.0 {
            let entries = core::mem::take(entries);
            // Sized for the field about to be added; the table then grows by
            // doubling and the list follows it (`Large::reserve_one`).
            let mut index = FieldIndex::with_capacity(entries.len() + 1);
            index.rebuild(&entries);
            self.0 = Repr::Large(Box::new(Large { entries, index }));
        }
    }

    /// Back to the plain shape, once a removal leaves at most
    /// [`SMALL_FIELDS`] fields.
    #[cold]
    #[inline(never)]
    fn drop_index(&mut self) {
        if let Repr::Large(large) = &mut self.0 {
            self.0 = Repr::Small(core::mem::take(&mut large.entries));
        }
    }

    /// The index after a bulk removal: dropped once the map is small again,
    /// rebuilt otherwise.
    fn reindex_after_bulk_change(&mut self) {
        if self.len() <= SMALL_FIELDS {
            self.drop_index();
        } else {
            self.reindex_after_reorder();
        }
    }

    /// Sets `key` to `value`, returning the value it replaced. An existing
    /// key keeps its position and its stored key; a new one is appended.
    #[inline]
    pub fn insert(&mut self, key: String, value: V) -> Option<V> {
        self.insert_full(key, value).1
    }

    /// [`insert`](Self::insert) with a key that may share its storage with
    /// the same key of other objects (#7260). An existing key keeps the
    /// handle it has.
    #[inline]
    pub fn insert_shared(&mut self, key: SharableString, value: V) -> Option<V> {
        self.insert_shared_full(key, value).1
    }

    /// [`insert`](Self::insert), also returning the field's position.
    #[inline]
    pub fn insert_full(&mut self, key: String, value: V) -> (usize, Option<V>) {
        self.insert_shared_full(SharableString::from(key), value)
    }

    /// [`insert_full`](Self::insert_full) with a key that may be shared.
    #[inline]
    pub fn insert_shared_full(&mut self, key: SharableString, value: V) -> (usize, Option<V>) {
        if let Repr::Large(large) = &mut self.0 {
            return large.insert_full(key, value);
        }
        match self.probe(key.as_str()) {
            Probe::Found(pos) => (
                pos,
                Some(core::mem::replace(&mut self.entries_mut()[pos].1, value)),
            ),
            Probe::Absent(hash) => (self.push_absent(key, value, hash), None),
        }
    }

    /// The entry for `key`, for in-place insertion or update.
    #[inline]
    pub fn entry(&mut self, key: String) -> Entry<'_, V> {
        match self.probe(key.as_str()) {
            Probe::Found(pos) => Entry::Occupied(OccupiedEntry { map: self, pos }),
            Probe::Absent(hash) => Entry::Vacant(VacantEntry {
                map: self,
                key,
                hash,
            }),
        }
    }

    /// The key at `pos`, with its storage.
    #[inline]
    pub fn key_at(&self, pos: usize) -> Option<&SharableString> {
        self.entries().get(pos).map(|(k, _)| k)
    }

    /// A handle on the key at `pos` that shares its storage: the key is
    /// moved behind a refcount first if it is inline (#7260). Changes how
    /// the key is held, not what it is, so a map another handle shares is a
    /// caller's to avoid: this is for a map a builder still owns alone, whose
    /// next sibling repeats the key.
    #[inline]
    pub fn share_key(&mut self, pos: usize) -> Option<SharableString> {
        let key = &mut self.entries_mut().get_mut(pos)?.0;
        key.promote();
        Some(key.clone())
    }

    /// Moves every inline key behind a refcount, leaving a key that already
    /// shares its storage as it is, so each key has an identity that a
    /// handle on it ([`key_at`](Self::key_at)) carries (#7401). Changes how
    /// the keys are held, not what they are.
    pub fn promote_keys(&mut self) {
        for (key, _) in self.entries_mut() {
            key.promote();
        }
    }

    /// Gives every key storage of its own, refcounted: a key that shares its
    /// storage with another object's (#7260) gets a copy first, so no two
    /// objects' keys share an identity (#7401). For a map materialized from
    /// a document, where jq's parser allocates every key on its own.
    pub fn own_keys(&mut self) {
        for (key, _) in self.entries_mut() {
            if key.is_shared() {
                *key = SharableString::from(key.as_str());
            }
            key.promote();
        }
    }

    /// Removes the field at `pos`, shifting every later one down.
    pub fn shift_remove_index(&mut self, pos: usize) -> Option<(String, V)> {
        self.shift_remove_field(pos)
            .map(|(k, v)| (k.into_string(), v))
    }

    /// [`shift_remove_index`](Self::shift_remove_index), the key as stored.
    fn shift_remove_field(&mut self, pos: usize) -> Option<Field<V>> {
        let len = self.len();
        if pos >= len {
            return None;
        }
        if len - 1 <= SMALL_FIELDS {
            self.drop_index();
        }
        match &mut self.0 {
            Repr::Small(entries) => Some(entries.remove(pos)),
            Repr::Large(large) => {
                let hash = short_hash(&large.index.state, large.entries[pos].0.as_str());
                large.index.erase(hash, pos);
                for e in &mut large.index.table {
                    let at = element_pos(*e);
                    if at > pos {
                        *e = element(element_hash(*e), at - 1);
                    }
                }
                Some(large.entries.remove(pos))
            }
        }
    }

    /// Removes the field at `pos`, moving the last field into its place.
    pub fn swap_remove_index(&mut self, pos: usize) -> Option<(String, V)> {
        self.swap_remove_field(pos)
            .map(|(k, v)| (k.into_string(), v))
    }

    /// [`swap_remove_index`](Self::swap_remove_index), the key as stored.
    fn swap_remove_field(&mut self, pos: usize) -> Option<Field<V>> {
        let len = self.len();
        if pos >= len {
            return None;
        }
        if len - 1 <= SMALL_FIELDS {
            self.drop_index();
        }
        match &mut self.0 {
            Repr::Small(entries) => Some(entries.swap_remove(pos)),
            Repr::Large(large) => {
                let state = &large.index.state;
                let hash = short_hash(state, large.entries[pos].0.as_str());
                large.index.erase(hash, pos);
                let last = len - 1;
                if pos != last {
                    let moved = short_hash(&large.index.state, large.entries[last].0.as_str());
                    large.index.relocate(moved, last, pos);
                }
                Some(large.entries.swap_remove(pos))
            }
        }
    }

    /// Removes `key`, shifting every later field down; returns its value.
    #[inline]
    pub fn shift_remove<Q>(&mut self, key: &Q) -> Option<V>
    where
        Q: ?Sized + AsRef<str>,
    {
        self.shift_remove_full(key).map(|(_, _, v)| v)
    }

    /// [`shift_remove`](Self::shift_remove), also returning the position and
    /// stored key.
    pub fn shift_remove_full<Q>(&mut self, key: &Q) -> Option<(usize, String, V)>
    where
        Q: ?Sized + AsRef<str>,
    {
        let pos = self.get_index_of(key)?;
        let (k, v) = self.shift_remove_index(pos)?;
        Some((pos, k, v))
    }

    /// [`shift_remove`](Self::shift_remove), also returning the stored key.
    pub fn shift_remove_entry<Q>(&mut self, key: &Q) -> Option<(String, V)>
    where
        Q: ?Sized + AsRef<str>,
    {
        self.shift_remove_full(key).map(|(_, k, v)| (k, v))
    }

    /// Removes `key`, moving the last field into its place; returns its value.
    #[inline]
    pub fn swap_remove<Q>(&mut self, key: &Q) -> Option<V>
    where
        Q: ?Sized + AsRef<str>,
    {
        self.swap_remove_full(key).map(|(_, _, v)| v)
    }

    /// [`swap_remove`](Self::swap_remove), also returning the position and
    /// stored key.
    pub fn swap_remove_full<Q>(&mut self, key: &Q) -> Option<(usize, String, V)>
    where
        Q: ?Sized + AsRef<str>,
    {
        let pos = self.get_index_of(key)?;
        let (k, v) = self.swap_remove_index(pos)?;
        Some((pos, k, v))
    }

    /// Removes the last field.
    pub fn pop(&mut self) -> Option<(String, V)> {
        let len = self.len();
        if len == 0 {
            return None;
        }
        self.shift_remove_index(len - 1)
    }

    /// Keeps the fields `keep` accepts, in order.
    pub fn retain<F>(&mut self, mut keep: F)
    where
        F: FnMut(&String, &mut V) -> bool,
    {
        let entries = self.entries_mut();
        let before = entries.len();
        entries.retain_mut(|(k, v)| keep(k, v));
        if entries.len() != before {
            self.reindex_after_bulk_change();
        }
    }

    /// Keeps the first `len` fields.
    pub fn truncate(&mut self, len: usize) {
        if len < self.len() {
            self.entries_mut().truncate(len);
            self.reindex_after_bulk_change();
        }
    }

    /// Removes every field (the field list keeps its allocation).
    #[inline]
    pub fn clear(&mut self) {
        self.drop_index();
        self.entries_mut().clear();
    }

    /// Makes room for `additional` more fields.
    pub fn reserve(&mut self, additional: usize) {
        match &mut self.0 {
            Repr::Small(entries) => entries.reserve(additional),
            Repr::Large(large) => {
                large.entries.reserve(additional);
                large.index.table.reserve(additional, rehash);
            }
        }
    }

    /// Releases spare capacity.
    pub fn shrink_to_fit(&mut self) {
        match &mut self.0 {
            Repr::Small(entries) => entries.shrink_to_fit(),
            Repr::Large(large) => {
                large.entries.shrink_to_fit();
                large.index.table.shrink_to_fit(rehash);
            }
        }
    }

    /// Sorts the fields by key.
    pub fn sort_keys(&mut self) {
        self.entries_mut()
            .sort_by(|a, b| a.0.as_str().cmp(b.0.as_str()));
        self.reindex_after_reorder();
    }

    /// Sorts the fields by key, not keeping equal keys' order (keys are
    /// unique, so the result is [`sort_keys`](Self::sort_keys)').
    pub fn sort_unstable_keys(&mut self) {
        self.entries_mut()
            .sort_unstable_by(|a, b| a.0.as_str().cmp(b.0.as_str()));
        self.reindex_after_reorder();
    }

    /// Sorts the fields with `cmp` over `(key, value, key, value)`, stably.
    pub fn sort_by<F>(&mut self, mut cmp: F)
    where
        F: FnMut(&String, &V, &String, &V) -> core::cmp::Ordering,
    {
        self.entries_mut()
            .sort_by(|a, b| cmp(&a.0, &a.1, &b.0, &b.1));
        self.reindex_after_reorder();
    }

    /// Sorts the fields with `cmp`, not keeping equal fields' order.
    pub fn sort_unstable_by<F>(&mut self, mut cmp: F)
    where
        F: FnMut(&String, &V, &String, &V) -> core::cmp::Ordering,
    {
        self.entries_mut()
            .sort_unstable_by(|a, b| cmp(&a.0, &a.1, &b.0, &b.1));
        self.reindex_after_reorder();
    }

    /// Sorts the fields by a key `f` computes once per field.
    pub fn sort_by_cached_key<T, F>(&mut self, mut f: F)
    where
        T: Ord,
        F: FnMut(&String, &V) -> T,
    {
        self.entries_mut().sort_by_cached_key(|(k, v)| f(k, v));
        self.reindex_after_reorder();
    }

    /// Reverses the field order.
    pub fn reverse(&mut self) {
        self.entries_mut().reverse();
        self.reindex_after_reorder();
    }

    /// Re-indexes every field of an indexed map after its fields moved.
    #[inline]
    fn reindex_after_reorder(&mut self) {
        if let Repr::Large(large) = &mut self.0 {
            large.index.rebuild(&large.entries);
        }
    }

    /// The fields in order.
    #[inline]
    pub fn iter(&self) -> Iter<'_, V> {
        Iter(self.entries().iter())
    }

    /// The fields in order, values mutably.
    #[inline]
    pub fn iter_mut(&mut self) -> IterMut<'_, V> {
        IterMut(self.entries_mut().iter_mut())
    }

    /// The fields in order, each key with its storage (#7401).
    #[inline]
    pub fn iter_stored(&self) -> core::slice::Iter<'_, (SharableString, V)> {
        self.entries().iter()
    }

    /// The keys in order.
    #[inline]
    pub fn keys(&self) -> Keys<'_, V> {
        Keys(self.entries().iter())
    }

    /// The values in order.
    #[inline]
    pub fn values(&self) -> Values<'_, V> {
        Values(self.entries().iter())
    }

    /// The values in order, mutably.
    #[inline]
    pub fn values_mut(&mut self) -> ValuesMut<'_, V> {
        ValuesMut(self.entries_mut().iter_mut())
    }

    /// The keys in order, by value.
    #[inline]
    pub fn into_keys(self) -> impl DoubleEndedIterator<Item = String> + ExactSizeIterator {
        self.into_fields().into_iter().map(|(k, _)| k.into_string())
    }

    /// The values in order, by value.
    #[inline]
    pub fn into_values(self) -> impl DoubleEndedIterator<Item = V> + ExactSizeIterator {
        self.into_fields().into_iter().map(|(_, v)| v)
    }

    /// Removes and yields every field, in order.
    #[inline]
    pub fn drain(
        &mut self,
    ) -> impl DoubleEndedIterator<Item = (String, V)> + ExactSizeIterator + '_ {
        self.drop_index();
        self.entries_mut()
            .drain(..)
            .map(|(k, v)| (k.into_string(), v))
    }

    /// The fields as a plain list, by move.
    #[inline]
    pub fn into_entries(self) -> Vec<(String, V)> {
        self.into_fields()
            .into_iter()
            .map(|(k, v)| (k.into_string(), v))
            .collect()
    }

    /// The fields, keys as stored.
    #[inline]
    fn into_fields(self) -> Vec<Field<V>> {
        match self.0 {
            Repr::Small(entries) => entries,
            Repr::Large(large) => large.entries,
        }
    }

    /// A map of `entries` taken as they are: the caller guarantees no key
    /// repeats (checked in debug builds).
    #[inline]
    pub fn from_unique_entries(entries: Vec<(String, V)>) -> Self {
        let entries: Vec<Field<V>> = entries
            .into_iter()
            .map(|(k, v)| (SharableString::from(k), v))
            .collect();
        debug_assert!(
            keys_are_unique(&entries),
            "from_unique_entries given a repeated key"
        );
        let mut map = Self(Repr::Small(entries));
        if map.len() > SMALL_FIELDS {
            map.grow_index();
        }
        map
    }
}

/// Whether no key of `entries` repeats (a debug-build check).
fn keys_are_unique<V>(entries: &[Field<V>]) -> bool {
    let mut keys: Vec<&str> = entries.iter().map(|(k, _)| k.as_str()).collect();
    keys.sort_unstable();
    keys.windows(2).all(|w| w[0] != w[1])
}

impl<V> Default for FieldMap<V> {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

impl<V: Clone> Clone for FieldMap<V> {
    #[inline]
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

/// `IndexMap`'s equality: the same keys with equal values, in any order.
impl<V: PartialEq> PartialEq for FieldMap<V> {
    fn eq(&self, other: &Self) -> bool {
        self.len() == other.len()
            && self
                .iter()
                .all(|(key, value)| other.get(key.as_str()).is_some_and(|v| *value == *v))
    }
}

impl<V: Eq> Eq for FieldMap<V> {}

impl<V: core::fmt::Debug> core::fmt::Debug for FieldMap<V> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_map().entries(self.iter()).finish()
    }
}

impl<V> FromIterator<(String, V)> for FieldMap<V> {
    #[inline]
    fn from_iter<I: IntoIterator<Item = (String, V)>>(iter: I) -> Self {
        let iter = iter.into_iter();
        let mut map = Self::with_capacity(iter.size_hint().0);
        map.extend(iter);
        map
    }
}

impl<V> Extend<(String, V)> for FieldMap<V> {
    #[inline]
    fn extend<I: IntoIterator<Item = (String, V)>>(&mut self, iter: I) {
        let iter = iter.into_iter();
        self.reserve(iter.size_hint().0);
        for (key, value) in iter {
            self.insert(key, value);
        }
    }
}

impl<V, const N: usize> From<[(String, V); N]> for FieldMap<V> {
    #[inline]
    fn from(entries: [(String, V); N]) -> Self {
        entries.into_iter().collect()
    }
}

impl<V, S> From<IndexMap<String, V, S>> for FieldMap<V> {
    #[inline]
    fn from(map: IndexMap<String, V, S>) -> Self {
        Self::from_unique_entries(map.into_iter().collect())
    }
}

impl<V> From<FieldMap<V>> for IndexMap<String, V> {
    #[inline]
    fn from(map: FieldMap<V>) -> Self {
        map.into_iter().collect()
    }
}

impl<V, Q> core::ops::Index<&Q> for FieldMap<V>
where
    Q: ?Sized + AsRef<str>,
{
    type Output = V;

    #[inline]
    fn index(&self, key: &Q) -> &V {
        self.get(key).expect("FieldMap: key not found")
    }
}

impl<V> core::ops::Index<usize> for FieldMap<V> {
    type Output = V;

    #[inline]
    fn index(&self, pos: usize) -> &V {
        &self.entries()[pos].1
    }
}

impl<V> core::ops::IndexMut<usize> for FieldMap<V> {
    #[inline]
    fn index_mut(&mut self, pos: usize) -> &mut V {
        &mut self.entries_mut()[pos].1
    }
}

impl<V> IntoIterator for FieldMap<V> {
    type Item = (String, V);
    type IntoIter = IntoIter<V>;

    #[inline]
    fn into_iter(self) -> IntoIter<V> {
        IntoIter(self.into_fields().into_iter())
    }
}

impl<'a, V> IntoIterator for &'a FieldMap<V> {
    type Item = (&'a String, &'a V);
    type IntoIter = Iter<'a, V>;

    #[inline]
    fn into_iter(self) -> Iter<'a, V> {
        self.iter()
    }
}

impl<'a, V> IntoIterator for &'a mut FieldMap<V> {
    type Item = (&'a String, &'a mut V);
    type IntoIter = IterMut<'a, V>;

    #[inline]
    fn into_iter(self) -> IterMut<'a, V> {
        self.iter_mut()
    }
}

/// By-value iteration over a [`FieldMap`]; a shared key comes out as a copy.
pub struct IntoIter<V>(alloc::vec::IntoIter<Field<V>>);

impl<V> Iterator for IntoIter<V> {
    type Item = (String, V);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        self.0.next().map(|(k, v)| (k.into_string(), v))
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.0.size_hint()
    }
}

impl<V> DoubleEndedIterator for IntoIter<V> {
    #[inline]
    fn next_back(&mut self) -> Option<Self::Item> {
        self.0.next_back().map(|(k, v)| (k.into_string(), v))
    }
}

impl<V> ExactSizeIterator for IntoIter<V> {}

impl<V> core::iter::FusedIterator for IntoIter<V> {}

macro_rules! slice_iter {
    ($(#[$doc:meta])* $name:ident, $slice:ident, $item:ty, |$e:ident| $map:expr) => {
        $(#[$doc])*
        pub struct $name<'a, V>(core::slice::$slice<'a, Field<V>>);

        impl<'a, V> Iterator for $name<'a, V> {
            type Item = $item;

            #[inline]
            fn next(&mut self) -> Option<Self::Item> {
                self.0.next().map(|$e| $map)
            }

            #[inline]
            fn size_hint(&self) -> (usize, Option<usize>) {
                self.0.size_hint()
            }

            #[inline]
            fn nth(&mut self, n: usize) -> Option<Self::Item> {
                self.0.nth(n).map(|$e| $map)
            }

            #[inline]
            fn count(self) -> usize {
                self.0.len()
            }

            #[inline]
            fn last(mut self) -> Option<Self::Item> {
                self.next_back()
            }
        }

        impl<V> DoubleEndedIterator for $name<'_, V> {
            #[inline]
            fn next_back(&mut self) -> Option<Self::Item> {
                self.0.next_back().map(|$e| $map)
            }

            #[inline]
            fn nth_back(&mut self, n: usize) -> Option<Self::Item> {
                self.0.nth_back(n).map(|$e| $map)
            }
        }

        impl<V> ExactSizeIterator for $name<'_, V> {
            #[inline]
            fn len(&self) -> usize {
                self.0.len()
            }
        }

        impl<V> core::iter::FusedIterator for $name<'_, V> {}
    };
}

slice_iter! {
    /// Iteration over a [`FieldMap`]'s fields.
    Iter, Iter, (&'a String, &'a V), |e| (&*e.0, &e.1)
}
slice_iter! {
    /// Iteration over a [`FieldMap`]'s fields, values mutably.
    IterMut, IterMut, (&'a String, &'a mut V), |e| (&*e.0, &mut e.1)
}
slice_iter! {
    /// Iteration over a [`FieldMap`]'s keys.
    Keys, Iter, &'a String, |e| &*e.0
}
slice_iter! {
    /// Iteration over a [`FieldMap`]'s values.
    Values, Iter, &'a V, |e| &e.1
}
slice_iter! {
    /// Iteration over a [`FieldMap`]'s values, mutably.
    ValuesMut, IterMut, &'a mut V, |e| &mut e.1
}

impl<V> Clone for Iter<'_, V> {
    #[inline]
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<V> Clone for Keys<'_, V> {
    #[inline]
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<V> Clone for Values<'_, V> {
    #[inline]
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

/// The entry for one key of a [`FieldMap`].
pub enum Entry<'a, V> {
    /// The map has the key.
    Occupied(OccupiedEntry<'a, V>),
    /// The map does not have the key.
    Vacant(VacantEntry<'a, V>),
}

/// An [`Entry`] for a key the map has.
pub struct OccupiedEntry<'a, V> {
    map: &'a mut FieldMap<V>,
    pos: usize,
}

/// An [`Entry`] for a key the map lacks.
pub struct VacantEntry<'a, V> {
    map: &'a mut FieldMap<V>,
    key: String,
    hash: Option<u32>,
}

impl<'a, V> OccupiedEntry<'a, V> {
    /// The stored key.
    #[inline]
    pub fn key(&self) -> &String {
        &self.map.entries()[self.pos].0
    }

    /// The field's position.
    #[inline]
    pub fn index(&self) -> usize {
        self.pos
    }

    /// The value.
    #[inline]
    pub fn get(&self) -> &V {
        &self.map.entries()[self.pos].1
    }

    /// The value, mutably.
    #[inline]
    pub fn get_mut(&mut self) -> &mut V {
        &mut self.map.entries_mut()[self.pos].1
    }

    /// The value, mutably, for as long as the map is borrowed.
    #[inline]
    pub fn into_mut(self) -> &'a mut V {
        &mut self.map.entries_mut()[self.pos].1
    }

    /// Replaces the value, returning the old one.
    #[inline]
    pub fn insert(&mut self, value: V) -> V {
        core::mem::replace(self.get_mut(), value)
    }

    /// Removes the field, shifting every later one down.
    #[inline]
    pub fn shift_remove(self) -> V {
        self.map
            .shift_remove_index(self.pos)
            .map(|(_, v)| v)
            .expect("an occupied entry's field exists")
    }

    /// Removes the field, moving the last one into its place.
    #[inline]
    pub fn swap_remove(self) -> V {
        self.map
            .swap_remove_index(self.pos)
            .map(|(_, v)| v)
            .expect("an occupied entry's field exists")
    }
}

impl<'a, V> VacantEntry<'a, V> {
    /// The key that would be inserted.
    #[inline]
    pub fn key(&self) -> &String {
        &self.key
    }

    /// The position the key would be inserted at.
    #[inline]
    pub fn index(&self) -> usize {
        self.map.len()
    }

    /// The key, by value.
    #[inline]
    pub fn into_key(self) -> String {
        self.key
    }

    /// Appends the key with `value`.
    #[inline]
    pub fn insert(self, value: V) -> &'a mut V {
        let pos = self
            .map
            .push_absent(SharableString::from(self.key), value, self.hash);
        &mut self.map.entries_mut()[pos].1
    }
}

impl<'a, V> Entry<'a, V> {
    /// The key this entry is for.
    #[inline]
    pub fn key(&self) -> &String {
        match self {
            Self::Occupied(entry) => entry.key(),
            Self::Vacant(entry) => entry.key(),
        }
    }

    /// The position of the key, present or to be inserted.
    #[inline]
    pub fn index(&self) -> usize {
        match self {
            Self::Occupied(entry) => entry.index(),
            Self::Vacant(entry) => entry.index(),
        }
    }

    /// Runs `f` on the value if the map has the key.
    #[inline]
    pub fn and_modify<F: FnOnce(&mut V)>(mut self, f: F) -> Self {
        if let Self::Occupied(entry) = &mut self {
            f(entry.get_mut());
        }
        self
    }

    /// The value, inserting `default` first if the key is absent.
    #[inline]
    pub fn or_insert(self, default: V) -> &'a mut V {
        self.or_insert_with(|| default)
    }

    /// The value, inserting `make()` first if the key is absent.
    #[inline]
    pub fn or_insert_with<F: FnOnce() -> V>(self, make: F) -> &'a mut V {
        match self {
            Self::Occupied(entry) => entry.into_mut(),
            Self::Vacant(entry) => entry.insert(make()),
        }
    }

    /// The value, inserting `V::default()` first if the key is absent.
    #[inline]
    pub fn or_default(self) -> &'a mut V
    where
        V: Default,
    {
        self.or_insert_with(V::default)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[derive(Debug, Clone)]
    enum Op {
        Insert(u8, u32),
        EntryOrInsert(u8, u32),
        ShiftRemove(u8),
        SwapRemove(u8),
        ShiftRemoveIndex(usize),
        SwapRemoveIndex(usize),
        Retain(u8),
        SortKeys,
        Reverse,
        Pop,
        Truncate(usize),
        Clear,
    }

    fn op() -> impl Strategy<Value = Op> {
        // Keys drawn from 24 names so maps cross SMALL_FIELDS both ways.
        prop_oneof![
            6 => (0u8..24, any::<u32>()).prop_map(|(k, v)| Op::Insert(k, v)),
            2 => (0u8..24, any::<u32>()).prop_map(|(k, v)| Op::EntryOrInsert(k, v)),
            2 => (0u8..24).prop_map(Op::ShiftRemove),
            2 => (0u8..24).prop_map(Op::SwapRemove),
            1 => (0usize..26).prop_map(Op::ShiftRemoveIndex),
            1 => (0usize..26).prop_map(Op::SwapRemoveIndex),
            1 => (1u8..5).prop_map(Op::Retain),
            1 => Just(Op::SortKeys),
            1 => Just(Op::Reverse),
            1 => Just(Op::Pop),
            1 => (0usize..20).prop_map(Op::Truncate),
            1 => Just(Op::Clear),
        ]
    }

    fn name(k: u8) -> String {
        // Lengths vary so the scan's length check is exercised.
        format!("k{}{}", k, "x".repeat(usize::from(k % 3)))
    }

    fn assert_same(map: &FieldMap<u32>, model: &IndexMap<String, u32>) {
        assert_eq!(map.len(), model.len());
        assert!(map
            .iter()
            .map(|(k, v)| (k.clone(), *v))
            .eq(model.iter().map(|(k, v)| (k.clone(), *v))));
        for k in 0..26 {
            let key = name(k);
            assert_eq!(map.get(key.as_str()), model.get(key.as_str()));
            assert_eq!(
                map.get_index_of(key.as_str()),
                model.get_index_of(key.as_str())
            );
        }
        assert_eq!(
            map.index().is_some(),
            map.len() > SMALL_FIELDS,
            "indexed exactly when past the threshold"
        );
        if let Some(index) = map.index() {
            assert_eq!(index.table.len(), map.len(), "the index maps every field");
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(512))]

        /// Every operation leaves the map exactly as the same operation leaves
        /// an `IndexMap`: order, values, positions, and lookups.
        #[test]
        fn field_map_matches_index_map_7260(ops in prop::collection::vec(op(), 0..120)) {
            let mut map: FieldMap<u32> = FieldMap::new();
            let mut model: IndexMap<String, u32> = IndexMap::new();
            for op in ops {
                match op {
                    Op::Insert(k, v) => {
                        prop_assert_eq!(map.insert_full(name(k), v), model.insert_full(name(k), v));
                    }
                    Op::EntryOrInsert(k, v) => {
                        let a = map.entry(name(k));
                        let b = model.entry(name(k));
                        prop_assert_eq!(a.index(), b.index());
                        *a.or_insert(v) += 1;
                        *b.or_insert(v) += 1;
                    }
                    Op::ShiftRemove(k) => {
                        prop_assert_eq!(map.shift_remove_full(name(k).as_str()), model.shift_remove_full(name(k).as_str()));
                    }
                    Op::SwapRemove(k) => {
                        prop_assert_eq!(map.swap_remove_full(name(k).as_str()), model.swap_remove_full(name(k).as_str()));
                    }
                    Op::ShiftRemoveIndex(i) => {
                        prop_assert_eq!(map.shift_remove_index(i), model.shift_remove_index(i));
                    }
                    Op::SwapRemoveIndex(i) => {
                        prop_assert_eq!(map.swap_remove_index(i), model.swap_remove_index(i));
                    }
                    Op::Retain(m) => {
                        map.retain(|_, v| *v % u32::from(m) != 0);
                        model.retain(|_, v| *v % u32::from(m) != 0);
                    }
                    Op::SortKeys => {
                        map.sort_keys();
                        model.sort_keys();
                    }
                    Op::Reverse => {
                        map.reverse();
                        model.reverse();
                    }
                    Op::Pop => {
                        prop_assert_eq!(map.pop(), model.pop());
                    }
                    Op::Truncate(n) => {
                        map.truncate(n);
                        model.truncate(n);
                    }
                    Op::Clear => {
                        map.clear();
                        model.clear();
                    }
                }
                assert_same(&map, &model);
                let cloned = map.clone();
                assert_same(&cloned, &model);
                prop_assert!(cloned == map);
            }
        }
    }

    #[test]
    fn insert_of_an_existing_key_keeps_position_and_first_key_7260() {
        let mut map = FieldMap::new();
        map.insert("a".to_string(), 1);
        map.insert("b".to_string(), 2);
        assert_eq!(map.insert("a".to_string(), 3), Some(1));
        assert_eq!(
            map.iter()
                .map(|(k, v)| (k.as_str(), *v))
                .collect::<Vec<_>>(),
            [("a", 3), ("b", 2)]
        );
    }

    #[test]
    fn equality_ignores_order_7260() {
        let a: FieldMap<u32> = [("x".to_string(), 1), ("y".to_string(), 2)].into();
        let b: FieldMap<u32> = [("y".to_string(), 2), ("x".to_string(), 1)].into();
        let c: FieldMap<u32> = [("y".to_string(), 2), ("x".to_string(), 3)].into();
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn index_is_built_past_the_threshold_and_dropped_below_it_7260() {
        let mut map = FieldMap::new();
        for i in 0..=SMALL_FIELDS {
            assert!(map.index().is_none());
            map.insert(i.to_string(), i);
        }
        assert!(map.index().is_some());
        assert_eq!(map.get("3"), Some(&3));
        map.shift_remove("0");
        assert!(map.index().is_none());
        assert_eq!(map.get("3"), Some(&3));
        assert_eq!(
            map.get_index_of(SMALL_FIELDS.to_string().as_str()),
            Some(SMALL_FIELDS - 1)
        );
    }

    #[test]
    fn a_large_capacity_starts_indexed_and_reads_the_same_7260() {
        let mut map = FieldMap::with_capacity(SMALL_FIELDS + 4);
        assert!(map.index().is_some());
        for i in 0..3 {
            map.insert(i.to_string(), i);
        }
        assert_eq!(map.insert("1".to_string(), 10), Some(1));
        assert_eq!(map.get("1"), Some(&10));
        assert_eq!(map.get_index_of("2"), Some(2));
        assert_eq!(map.shift_remove("0"), Some(0));
        assert!(map.index().is_none(), "a removal at a small size drops it");
        assert_eq!(
            map.iter()
                .map(|(k, v)| (k.as_str(), *v))
                .collect::<Vec<_>>(),
            [("1", 10), ("2", 2)]
        );
        let small = FieldMap::<u32>::with_capacity(SMALL_FIELDS);
        assert!(small.index().is_none());
    }

    /// Past the threshold the field list grows to the table's capacity, as
    /// an `IndexMap`'s entries do, not by doubling: 24 fields fit a table of
    /// 28, so the list holds 28 (doubling from 16 would have made it 32).
    #[test]
    fn a_large_field_list_grows_with_its_table_7260() {
        let mut map = FieldMap::with_capacity(SMALL_FIELDS);
        for i in 0..24 {
            map.insert(i.to_string(), i);
        }
        let table = map.index().map(|index| index.table.capacity());
        assert_eq!(Some(map.capacity()), table);
        assert!(map.capacity() < 32, "{}", map.capacity());
    }

    #[test]
    fn a_field_map_is_three_words_7260() {
        assert_eq!(
            core::mem::size_of::<FieldMap<u64>>(),
            3 * core::mem::size_of::<usize>()
        );
    }
}
