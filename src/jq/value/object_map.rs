//! The object container behind `OwnedValue::Object`: a collapsed
//! `IndexMap` view plus, when a key repeats, the earlier entries of that key
//! ("shadows") in document order (#1344, ADR-0027).
//!
//! # Why
//!
//! `yq` keeps a mapping as an ordered list of entries, so `a: 1 / a: 2` is
//! two entries and a write through `.a` reaches only the last one. An
//! `IndexMap` holds one value per key, so the first entry is gone before any
//! builtin sees the tree. This module gives every `OwnedValue::Object` a place
//! to keep the earlier entries without changing what any existing reader sees.
//!
//! # Representation
//!
//! [`ObjectMapOf`] is two pointers:
//!
//! - `map` is the **collapsed view**: a refcounted `IndexMap` holding each key
//!   once, at its first entry's position, with its last entry's value (I1).
//!   Every reader that goes through [`Deref`] sees exactly what it saw when
//!   the handle was a bare pointer to that map. The heap block is the
//!   `IndexMap` and nothing else, so an object that repeats no key costs the
//!   allocator what it always cost (#3000 sized `OwnedValue` and `JqValue`
//!   against those very chunk sizes).
//! - `shadows` is `None` unless some key repeats (I2). When it is `Some`, it
//!   lists every entry in document order, so the full view
//!   ([`ObjectMapOf::occurrences`]) is recoverable.
//!
//! The second pointer lives in the handle, not in the map's heap block, on
//! purpose: it adds 8 bytes to a value that is already 32 wide (so no value
//! grows), where a field in the block would step the allocator one bin on
//! glibc for every object and showed as +1% to +3% peak RSS on object-heavy
//! inputs (#1344).
//!
//! Invariants, kept by every method here:
//!
//! - **I1.** `map` is the collapsed view described above.
//! - **I2.** `shadows.is_none()` exactly when no key repeats.
//! - **I3.** A shadow is created ([`ObjectMapOf::push_occurrence`]), removed
//!   with its key, or dropped wholesale ([`ObjectMapOf::collapse`]). The one
//!   method that writes through to a shadow is
//!   [`ObjectMapOf::first_occurrence_mut`], which exists for yq's `+`.
//! - **I4.** Nothing in `jq` mode creates a shadow: the producers that do are
//!   gated on the evaluation mode, so a consumer needs no mode check.
//!
//! # How the existing call sites stay correct
//!
//! An inherent method on [`ObjectMapOf`] is chosen before the `IndexMap`
//! method of the same name it would auto-dereference to, so the mutators in
//! the table below pick up the shadow-aware behaviour without being edited.
//! [`DerefMut`] hands out the raw `&mut IndexMap`, which cannot keep the
//! shadows in step, so it **collapses first**: a raw-map caller can only ever
//! produce today's answer, never a mix of today's answer and yq's. The
//! by-value conversions ([`ObjectMapOf::into_index_map`], the owned
//! `IntoIterator`) and [`Extend`] do the same.
//!
//! | Method                                                     | With shadows                                   |
//! |------------------------------------------------------------|------------------------------------------------|
//! | `get_mut`, `get_index_mut`, `iter_mut`, `values_mut`       | the live (last) entry only                      |
//! | `insert` on an existing key                                | the live entry only                             |
//! | `insert`/`entry` on a vacant key                           | appends                                         |
//! | `shift_remove`, `swap_remove`, `retain`, `clear`           | removes **every** entry for the key             |
//! | [`DerefMut`], `into_index_map`, owned `into_iter`, `extend` | collapses first                                 |
//! | `occurrences`, `occurrence_len`, `into_occurrences`        | the full view                                   |
//! | `push_occurrence`                                          | appends; an existing live entry becomes a shadow |
//! | `first_occurrence`, `first_occurrence_mut`                 | the earliest entry for the key                  |
//!
//! # Why the map keeps the default hasher
//!
//! The map hashes with `IndexMap`'s default, SipHash 1-3 seeded per process, which a document's
//! keys cannot be crafted against. A fixed hasher would give that up for very little, measured
//! (#5594) by building the same commit with `indexmap`'s default replaced by an unseeded Fx hash,
//! the cheapest a swap could get. On the `users` pattern (Apple M5 Max, instructions retired),
//! `del(.users[0]) | .users | length` went from 1,625M to 1,584M (-2.5%) at 7 MB and -0.8% at
//! 0.7 MB, an assignment -2.6% and -1.3%, and `tojson` -0.7%. The rest of the insert's cost is the
//! table itself (probing, growth and its allocations), which no hasher changes. Revisit this
//! with new numbers, not on the strength of a profile that attributes `insert_full` to hashing.

#[cfg(all(not(test), feature = "unshared-containers"))]
use alloc::boxed::Box;
#[cfg(not(test))]
use alloc::string::String;
#[cfg(not(test))]
use alloc::vec::Vec;

#[cfg(not(feature = "unshared-containers"))]
use alloc::rc::Rc;
use core::hash::Hash;
use core::ops::{Deref, DerefMut};

use indexmap::{Equivalent, IndexMap};

/// Records a forced copy for [`share_stats`](crate::jq::share_stats), in the
/// builds that read it. Only the shared (refcounted) shape ever copies
/// forcibly, so the holdout has no use for it.
#[cfg(not(feature = "unshared-containers"))]
macro_rules! note_forced_copy {
    ($kind:ident) => {
        #[cfg(any(test, feature = "share-stats"))]
        crate::jq::share_stats::forced(crate::jq::share_stats::Kind::$kind);
    };
}

/// The backing store for an object-valued enum arm: an
/// [`IndexMap<String, V>`](IndexMap) (the collapsed view) behind a single
/// pointer, plus the shadows of any repeated key (#1344).
///
/// The map is refcounted so that clones share it until one side writes
/// (#2999), and held behind that one pointer (#3000). The cost is one extra
/// allocation (plus, in the shipped shape, a 16-byte refcount header) and one
/// extra pointer chase per object -- charged per object regardless of size,
/// so an *empty* `{}`, which used to cost nothing on the heap
/// (`IndexMap::new()` does not allocate), now costs a `malloc`/`free` pair
/// with no offsetting inline saving of its own. The shadows are a second,
/// separately refcounted pointer that is `None` for an object with no
/// repeated key.
///
/// The indirection is invisible to callers. [`Deref`]/[`DerefMut`] expose
/// the whole `IndexMap` API, [`IntoIterator`] is implemented for the owned,
/// shared and mutable forms, and [`From`]/[`FromIterator`] build one from
/// anything an `IndexMap` can be built from -- so `Object(map)` patterns keep
/// reading and writing the map exactly as before. Only *construction* from a
/// bare `IndexMap` needs a `.into()`.
///
/// `DerefMut` and the by-value conversions (`into_index_map`, the owned
/// `IntoIterator`) are where a shared map is copied; both are
/// `#[track_caller]` in the builds that read it (`test`, `share-stats`) so
/// [`share_stats`](crate::jq::share_stats) can name the line that forced it.
///
/// Note for downstream users: the shape of the `OwnedValue::Object` variant
/// changed in #3000 (boxed) and again in #2999 (refcounted).
/// [`OwnedValue::as_object_mut`](super::OwnedValue::as_object_mut) returns an
/// `ObjectMapOf` since #1344, not a bare `IndexMap`. Construct with
/// [`OwnedValue::object_from`](super::OwnedValue::object_from) (or
/// `IndexMap::....into()`) and read through
/// [`OwnedValue::as_object`](super::OwnedValue::as_object).
pub struct ObjectMapOf<V> {
    /// The collapsed view (I1).
    map: ObjectMapInnerOf<V>,
    /// Every repeated key's earlier entries, or `None` when no key repeats
    /// (I2).
    shadows: Option<ShadowsInnerOf<V>>,
}

/// [`ObjectMapOf`]'s map: refcounted in the shipped build.
#[cfg(not(feature = "unshared-containers"))]
type ObjectMapInnerOf<V> = Rc<IndexMap<String, V>>;

/// See [`ObjectMapInnerOf`]'s shared twin -- the `unshared-containers`
/// measurement holdout, the #3000 layout: boxed, not shared.
#[cfg(feature = "unshared-containers")]
type ObjectMapInnerOf<V> = Box<IndexMap<String, V>>;

/// [`ObjectMapOf`]'s side list: refcounted in the shipped build.
#[cfg(not(feature = "unshared-containers"))]
type ShadowsInnerOf<V> = Rc<Shadows<V>>;

/// See [`ShadowsInnerOf`]'s shared twin.
#[cfg(feature = "unshared-containers")]
type ShadowsInnerOf<V> = Box<Shadows<V>>;

/// The document-order list of entries for a map in which some key repeats.
///
/// `order` holds one element per entry. A key's last entry is `Live(slot)`,
/// naming the `map` slot that holds its value; every earlier entry is a
/// `Shadow` carrying its own key and value. Each `map` slot therefore appears
/// exactly once as a `Live`, and `order.len() == map.len() + (shadow count)`.
///
/// `live_pos[slot]` is the position in `order` of `Live(slot)`, so turning a
/// live entry into a shadow ([`ObjectMapOf::push_occurrence`]) is O(1) rather
/// than a search -- a document with many repeats of one key must not go
/// quadratic.
struct Shadows<V> {
    order: Vec<Occurrence<V>>,
    live_pos: Vec<usize>,
}

/// One entry of [`Shadows::order`].
enum Occurrence<V> {
    /// The key's last entry; its value is the `map` slot's.
    Live(usize),
    /// An earlier entry for a key whose later entry is live.
    Shadow(String, V),
}

impl<V: Clone> Clone for Occurrence<V> {
    fn clone(&self) -> Self {
        match self {
            Self::Live(slot) => Self::Live(*slot),
            Self::Shadow(key, value) => Self::Shadow(key.clone(), value.clone()),
        }
    }
}

impl<V: Clone> Clone for Shadows<V> {
    fn clone(&self) -> Self {
        Self {
            order: self.order.clone(),
            live_pos: self.live_pos.clone(),
        }
    }
}

impl<V> Shadows<V> {
    /// A side list for a map with `len` slots and no shadow yet.
    fn with_live_slots(len: usize) -> Self {
        Self {
            order: (0..len).map(Occurrence::Live).collect(),
            live_pos: (0..len).collect(),
        }
    }

    /// Records a key that was just appended to the map at `slot`.
    #[inline]
    fn push_live(&mut self, slot: usize) {
        self.live_pos.push(self.order.len());
        self.order.push(Occurrence::Live(slot));
    }

    /// Recomputes `live_pos` from `order` for a map of `len` slots.
    fn rebuild_live_pos(&mut self, len: usize) {
        self.live_pos.clear();
        self.live_pos.resize(len, 0);
        for (pos, entry) in self.order.iter().enumerate() {
            if let Occurrence::Live(slot) = entry {
                self.live_pos[*slot] = pos;
            }
        }
    }

    /// Whether every entry is live, i.e. no key repeats (I2).
    #[inline]
    fn has_no_shadow(&self, map_len: usize) -> bool {
        self.order.len() == map_len
    }
}

/// The map, copied first if another handle shares it -- the copy-on-write
/// point (#2999). `V: Clone` is what `Rc::make_mut` needs and is the only
/// bound the sharing adds anywhere.
#[inline]
#[cfg_attr(any(test, feature = "share-stats"), track_caller)]
fn map_mut<V: Clone>(map: &mut ObjectMapInnerOf<V>) -> &mut IndexMap<String, V> {
    #[cfg(not(feature = "unshared-containers"))]
    {
        if Rc::strong_count(map) > 1 {
            note_forced_copy!(ObjectMakeMut);
        }
        Rc::make_mut(map)
    }
    #[cfg(feature = "unshared-containers")]
    {
        map
    }
}

/// The side list, copied first if another handle shares it.
#[inline]
fn shadows_mut<V: Clone>(shadows: &mut ShadowsInnerOf<V>) -> &mut Shadows<V> {
    #[cfg(not(feature = "unshared-containers"))]
    {
        Rc::make_mut(shadows)
    }
    #[cfg(feature = "unshared-containers")]
    {
        shadows
    }
}

/// `shadows` as a mutable reference to its list, if there is one.
#[inline]
fn live_shadows_mut<V: Clone>(shadows: &mut Option<ShadowsInnerOf<V>>) -> Option<&mut Shadows<V>> {
    shadows.as_mut().map(shadows_mut)
}

impl<V> ObjectMapOf<V> {
    /// An empty container.
    #[inline]
    pub fn new() -> Self {
        Self::from(IndexMap::new())
    }

    /// An empty container with room for `capacity` entries.
    #[inline]
    pub fn with_capacity(capacity: usize) -> Self {
        Self::from(IndexMap::with_capacity(capacity))
    }

    /// Whether `self` and `other` are two handles on the *same*
    /// storage -- the pointer identity real jq's `jv_identical`
    /// has for arrays and objects, which the embed table (#2889)
    /// reads to recognize a container that is still some document
    /// node's own unmodified value.
    ///
    /// Sound as an identity test precisely because a write through
    /// either handle copies first ([`DerefMut`], `Rc::make_mut`):
    /// while the answer is `true`, neither handle has been written
    /// through since they were split. Every method that changes the side
    /// list also changes the map, so equal map pointers with different side
    /// lists cannot happen.
    ///
    /// Always `false` under `unshared-containers`, where a clone
    /// already deep-copied and no two handles can share anything --
    /// the measurement holdout therefore refuses every identity the
    /// table would have granted, which costs acceptance, never
    /// correctness.
    #[inline]
    pub fn ptr_eq(&self, other: &Self) -> bool {
        #[cfg(not(feature = "unshared-containers"))]
        {
            Rc::ptr_eq(&self.map, &other.map)
                && match (&self.shadows, &other.shadows) {
                    (None, None) => true,
                    (Some(a), Some(b)) => Rc::ptr_eq(a, b),
                    _ => false,
                }
        }
        #[cfg(feature = "unshared-containers")]
        {
            let _ = other;
            false
        }
    }

    /// Whether another handle currently shares this storage, i.e.
    /// whether the next write through this handle will copy the map.
    ///
    /// The side list of a repeated key is a separate allocation, copied only
    /// by a write that changes it (a new key, a removal, a new repeat), so a
    /// shared side list does not show here.
    ///
    /// Always `false` under `unshared-containers`, where every clone
    /// already copied eagerly.
    #[inline]
    pub fn is_shared(&self) -> bool {
        #[cfg(not(feature = "unshared-containers"))]
        {
            Rc::strong_count(&self.map) > 1
        }
        #[cfg(feature = "unshared-containers")]
        {
            false
        }
    }

    /// Whether some key of this object has more than one entry, i.e. whether
    /// [`occurrences`](Self::occurrences) differs from the collapsed view.
    #[inline]
    pub fn has_shadows(&self) -> bool {
        self.shadows.is_some()
    }

    /// How many entries the full view has: the collapsed view's length plus
    /// one per shadow.
    #[inline]
    pub fn occurrence_len(&self) -> usize {
        match self.shadows.as_deref() {
            Some(shadows) => shadows.order.len(),
            None => self.map.len(),
        }
    }

    /// Every entry in document order, a repeated key once per entry. For an
    /// object with no shadow this is the collapsed view's own iteration.
    #[inline]
    pub fn occurrences(&self) -> Occurrences<'_, V> {
        Occurrences(match self.shadows.as_deref() {
            Some(shadows) => OccurrencesInner::Ordered {
                map: &self.map,
                entries: shadows.order.iter(),
            },
            None => OccurrencesInner::Plain(self.map.iter()),
        })
    }

    /// The earliest entry for `key`: the collapsed view's value when the key
    /// does not repeat, the first shadow's otherwise.
    ///
    /// A scan of the side list when the object has shadows, so a caller that
    /// asks for many keys of a large object with shadows pays O(entries) per
    /// key; the phase that needs it (yq's `+` and `pick`) measures before
    /// adding an index.
    pub fn first_occurrence<Q>(&self, key: &Q) -> Option<&V>
    where
        Q: ?Sized + Hash + Equivalent<String>,
    {
        let Some(shadows) = self.shadows.as_deref() else {
            return self.map.get(key);
        };
        for entry in &shadows.order {
            match entry {
                Occurrence::Shadow(shadow_key, value) if key.equivalent(shadow_key) => {
                    return Some(value);
                }
                Occurrence::Live(slot) => {
                    let (live_key, value) = self.map.get_index(*slot)?;
                    if key.equivalent(live_key) {
                        return Some(value);
                    }
                }
                Occurrence::Shadow(..) => {}
            }
        }
        None
    }

    /// Whether `self` and `other` hold the same entries in the same document
    /// order -- repeated keys included.
    ///
    /// `==` compares the collapsed views (jq-value equality: `{"a":1,"a":2}`
    /// equals `{"a":2}`). Anything that asks "would these print the same"
    /// -- presentation identity, an alias's anchor -- has to use this.
    pub fn eq_occurrences(&self, other: &Self) -> bool
    where
        V: PartialEq,
    {
        self.occurrence_len() == other.occurrence_len()
            && self
                .occurrences()
                .zip(other.occurrences())
                .all(|((ka, va), (kb, vb))| ka == kb && va == vb)
    }
}

impl<V: Clone> ObjectMapOf<V> {
    /// Consume the wrapper, yielding the collapsed `IndexMap` -- by move when
    /// this handle was the last one, by copy otherwise. Shadows are dropped.
    #[inline]
    #[cfg_attr(any(test, feature = "share-stats"), track_caller)]
    pub fn into_index_map(self) -> IndexMap<String, V> {
        #[cfg(not(feature = "unshared-containers"))]
        {
            match Rc::try_unwrap(self.map) {
                Ok(map) => map,
                Err(shared) => {
                    note_forced_copy!(ObjectUnwrap);
                    (*shared).clone()
                }
            }
        }
        #[cfg(feature = "unshared-containers")]
        {
            *self.map
        }
    }

    /// Consume the wrapper, yielding every entry in document order -- by
    /// move when this handle was the last one, by copy otherwise.
    #[cfg_attr(any(test, feature = "share-stats"), track_caller)]
    pub fn into_occurrences(self) -> Vec<(String, V)> {
        let Self { map, shadows } = self;
        #[cfg(not(feature = "unshared-containers"))]
        let (map, shadows) = {
            let shared = Rc::strong_count(&map) > 1;
            if shared {
                note_forced_copy!(ObjectUnwrap);
            }
            (
                Rc::try_unwrap(map).unwrap_or_else(|shared| (*shared).clone()),
                shadows.map(|s| Rc::try_unwrap(s).unwrap_or_else(|shared| (*shared).clone())),
            )
        };
        #[cfg(feature = "unshared-containers")]
        let (map, shadows) = (*map, shadows.map(|s| *s));
        let Some(shadows) = shadows else {
            return map.into_iter().collect();
        };
        let mut slots: Vec<Option<(String, V)>> = map.into_iter().map(Some).collect();
        shadows
            .order
            .into_iter()
            .map(|entry| match entry {
                Occurrence::Shadow(key, value) => (key, value),
                Occurrence::Live(slot) => slots[slot]
                    .take()
                    .expect("each map slot is live exactly once"),
            })
            .collect()
    }

    /// Drops every shadow, leaving the collapsed view -- the answer a reader
    /// of the bare `IndexMap` always gave.
    #[inline]
    pub fn collapse(&mut self) {
        self.shadows = None;
    }

    /// Appends an entry for `key`, keeping an earlier entry for the same key.
    ///
    /// For a key the object does not have this is [`insert`](Self::insert).
    /// For one it has, the existing live entry becomes a shadow at its
    /// position and the new value goes live at the end of the document order,
    /// while the collapsed view keeps the key at its first entry's slot with
    /// the new value -- the `IndexMap` you get by inserting the entries in
    /// order.
    #[cfg_attr(any(test, feature = "share-stats"), track_caller)]
    pub fn push_occurrence(&mut self, key: String, value: V) {
        let map = map_mut(&mut self.map);
        let (slot, old) = map.insert_full(key, value);
        let Some(old_value) = old else {
            if let Some(shadows) = live_shadows_mut(&mut self.shadows) {
                shadows.push_live(slot);
            }
            return;
        };
        self.shadow_replaced(slot, old_value);
    }

    /// The value at `slot` of the collapsed view was just replaced by a later entry of the same key
    /// (a caller that filled a plain `IndexMap` with `insert_full`, then moved it in with
    /// [`From`]): keep the replaced `old` value as the key's earlier entry. The same step
    /// [`push_occurrence`](Self::push_occurrence) takes on a repeat, so a builder can use the
    /// plain map's single-lookup `insert_full` for every member and pay for the side list only
    /// at the first repeat.
    pub fn shadow_replaced(&mut self, slot: usize, old: V) {
        // `insert_full` dropped the incoming key for an existing one, so the
        // shadow takes a copy of the map's. Only a repeat pays this; a key
        // seen for the first time never reaches here.
        let key = self
            .map
            .get_index(slot)
            .map(|(k, _)| k.clone())
            .expect("the replaced slot is live");
        let len = self.map.len();
        let shadows = shadows_mut(
            self.shadows
                .get_or_insert_with(|| Shadows::with_live_slots(len).into()),
        );
        let pos = shadows.live_pos[slot];
        shadows.order[pos] = Occurrence::Shadow(key, old);
        shadows.live_pos[slot] = shadows.order.len();
        shadows.order.push(Occurrence::Live(slot));
    }

    /// A mutable reference to the earliest entry for `key`.
    ///
    /// The one method that writes through to a shadow rather than the live
    /// entry: yq's `+` overwrites the *first* entry of a repeated key. The
    /// map is copied first even when the entry is a shadow, so two handles
    /// that differ only in a shadow never share a map pointer.
    #[cfg_attr(any(test, feature = "share-stats"), track_caller)]
    pub fn first_occurrence_mut<Q>(&mut self, key: &Q) -> Option<&mut V>
    where
        Q: ?Sized + Hash + Equivalent<String>,
    {
        let map = map_mut(&mut self.map);
        let Some(shadows) = live_shadows_mut(&mut self.shadows) else {
            return map.get_mut(key);
        };
        let mut found = None;
        for (pos, entry) in shadows.order.iter().enumerate() {
            let hit = match entry {
                Occurrence::Shadow(shadow_key, _) => key.equivalent(shadow_key),
                Occurrence::Live(slot) => map
                    .get_index(*slot)
                    .is_some_and(|(live_key, _)| key.equivalent(live_key)),
            };
            if hit {
                found = Some(pos);
                break;
            }
        }
        match &mut shadows.order[found?] {
            Occurrence::Shadow(_, value) => Some(value),
            Occurrence::Live(slot) => map.get_index_mut(*slot).map(|(_, value)| value),
        }
    }

    /// A mutable reference to the value for `key` -- the live (last) entry
    /// when the key repeats.
    #[inline]
    #[cfg_attr(any(test, feature = "share-stats"), track_caller)]
    pub fn get_mut<Q>(&mut self, key: &Q) -> Option<&mut V>
    where
        Q: ?Sized + Hash + Equivalent<String>,
    {
        map_mut(&mut self.map).get_mut(key)
    }

    /// A mutable reference to the entry at `index` of the collapsed view.
    #[inline]
    #[cfg_attr(any(test, feature = "share-stats"), track_caller)]
    pub fn get_index_mut(&mut self, index: usize) -> Option<(&String, &mut V)> {
        map_mut(&mut self.map).get_index_mut(index)
    }

    /// Mutable iteration over the collapsed view's values.
    #[inline]
    #[cfg_attr(any(test, feature = "share-stats"), track_caller)]
    pub fn iter_mut(&mut self) -> indexmap::map::IterMut<'_, String, V> {
        map_mut(&mut self.map).iter_mut()
    }

    /// Mutable iteration over the collapsed view's values.
    #[inline]
    #[cfg_attr(any(test, feature = "share-stats"), track_caller)]
    pub fn values_mut(&mut self) -> indexmap::map::ValuesMut<'_, String, V> {
        map_mut(&mut self.map).values_mut()
    }

    /// The slot, key and a mutable reference to the value for `key` -- the
    /// live (last) entry when the key repeats.
    #[inline]
    #[cfg_attr(any(test, feature = "share-stats"), track_caller)]
    pub fn get_full_mut<Q>(&mut self, key: &Q) -> Option<(usize, &String, &mut V)>
    where
        Q: ?Sized + Hash + Equivalent<String>,
    {
        map_mut(&mut self.map).get_full_mut(key)
    }

    /// The key and a mutable reference to the value for `key`.
    #[inline]
    #[cfg_attr(any(test, feature = "share-stats"), track_caller)]
    pub fn get_key_value_mut<Q>(&mut self, key: &Q) -> Option<(&String, &mut V)>
    where
        Q: ?Sized + Hash + Equivalent<String>,
    {
        map_mut(&mut self.map).get_key_value_mut(key)
    }

    /// The collapsed view's first entry, mutably.
    #[inline]
    #[cfg_attr(any(test, feature = "share-stats"), track_caller)]
    pub fn first_mut(&mut self) -> Option<(&String, &mut V)> {
        map_mut(&mut self.map).first_mut()
    }

    /// The collapsed view's last entry, mutably.
    #[inline]
    #[cfg_attr(any(test, feature = "share-stats"), track_caller)]
    pub fn last_mut(&mut self) -> Option<(&String, &mut V)> {
        map_mut(&mut self.map).last_mut()
    }

    /// Sets `key` to `value`, returning the value it replaced.
    ///
    /// An existing key's live (last) entry is replaced and earlier entries
    /// stay (`.a = 5` over `a: 1 / a: 2` is `a: 1 / a: 5`); a new key is
    /// appended.
    #[inline]
    #[cfg_attr(any(test, feature = "share-stats"), track_caller)]
    pub fn insert(&mut self, key: String, value: V) -> Option<V> {
        let map = map_mut(&mut self.map);
        if self.shadows.is_none() {
            return map.insert(key, value);
        }
        let (slot, old) = map.insert_full(key, value);
        if old.is_none() {
            // Only a new key changes the side list; replacing a live value
            // must not copy a side list another handle shares.
            if let Some(shadows) = live_shadows_mut(&mut self.shadows) {
                shadows.push_live(slot);
            }
        }
        old
    }

    /// The entry for `key`, for in-place insertion.
    ///
    /// Narrower than [`IndexMap::entry`] on purpose: it offers no removal, so
    /// a site that wants one (`OccupiedEntry::remove`) fails to compile and
    /// has to choose between [`shift_remove`](Self::shift_remove) and
    /// [`collapse`](Self::collapse).
    #[cfg_attr(any(test, feature = "share-stats"), track_caller)]
    pub fn entry(&mut self, key: String) -> ObjectMapEntry<'_, V> {
        let map = map_mut(&mut self.map);
        match map.entry(key) {
            indexmap::map::Entry::Occupied(entry) => {
                ObjectMapEntry::Occupied(OccupiedMapEntry(entry))
            }
            indexmap::map::Entry::Vacant(entry) => ObjectMapEntry::Vacant(VacantMapEntry {
                entry,
                shadows: &mut self.shadows,
            }),
        }
    }

    /// Removes every entry for `key` and returns the live (last) one's value,
    /// keeping the remaining entries' order.
    #[cfg_attr(any(test, feature = "share-stats"), track_caller)]
    pub fn shift_remove<Q>(&mut self, key: &Q) -> Option<V>
    where
        Q: ?Sized + Hash + Equivalent<String>,
    {
        let map = map_mut(&mut self.map);
        let (slot, _, value) = map.shift_remove_full(key)?;
        if let Some(shadows) = live_shadows_mut(&mut self.shadows) {
            shadows.order.retain_mut(|entry| match entry {
                Occurrence::Live(live) if *live == slot => false,
                Occurrence::Live(live) => {
                    if *live > slot {
                        *live -= 1;
                    }
                    true
                }
                Occurrence::Shadow(shadow_key, _) => !key.equivalent(shadow_key),
            });
            shadows.rebuild_live_pos(map.len());
            if shadows.has_no_shadow(map.len()) {
                self.shadows = None;
            }
        }
        Some(value)
    }

    /// Removes every entry for `key` and returns the live (last) one's value,
    /// moving the collapsed view's last key into the hole as
    /// [`IndexMap::swap_remove`] does.
    ///
    /// While another key still repeats, the full view keeps its document
    /// order; once no shadow is left it is the collapsed view's permuted
    /// slot order, as for any object.
    #[cfg_attr(any(test, feature = "share-stats"), track_caller)]
    pub fn swap_remove<Q>(&mut self, key: &Q) -> Option<V>
    where
        Q: ?Sized + Hash + Equivalent<String>,
    {
        let map = map_mut(&mut self.map);
        let (slot, _, value) = map.swap_remove_full(key)?;
        if let Some(shadows) = live_shadows_mut(&mut self.shadows) {
            // The key that sat in the last slot now sits in `slot`.
            let moved_from = map.len();
            shadows.order.retain_mut(|entry| match entry {
                Occurrence::Live(live) if *live == slot => false,
                Occurrence::Live(live) => {
                    if *live == moved_from {
                        *live = slot;
                    }
                    true
                }
                Occurrence::Shadow(shadow_key, _) => !key.equivalent(shadow_key),
            });
            shadows.rebuild_live_pos(map.len());
            if shadows.has_no_shadow(map.len()) {
                self.shadows = None;
            }
        }
        Some(value)
    }

    /// Keeps the keys `keep` accepts, judging each by its live entry; a key
    /// that goes takes every one of its entries with it.
    #[cfg_attr(any(test, feature = "share-stats"), track_caller)]
    pub fn retain<F>(&mut self, mut keep: F)
    where
        F: FnMut(&String, &mut V) -> bool,
    {
        let map = map_mut(&mut self.map);
        let Some(shadows) = live_shadows_mut(&mut self.shadows) else {
            map.retain(keep);
            return;
        };
        let mut new_slot: Vec<Option<usize>> = Vec::with_capacity(map.len());
        let mut kept = 0;
        map.retain(|key, value| {
            let k = keep(key, value);
            new_slot.push(if k {
                kept += 1;
                Some(kept - 1)
            } else {
                None
            });
            k
        });
        let map = &*map;
        shadows.order.retain_mut(|entry| match entry {
            Occurrence::Live(live) => match new_slot[*live] {
                Some(slot) => {
                    *live = slot;
                    true
                }
                None => false,
            },
            Occurrence::Shadow(shadow_key, _) => map.contains_key(shadow_key),
        });
        shadows.rebuild_live_pos(map.len());
        if shadows.has_no_shadow(map.len()) {
            self.shadows = None;
        }
    }

    /// Removes every entry.
    #[cfg_attr(any(test, feature = "share-stats"), track_caller)]
    pub fn clear(&mut self) {
        map_mut(&mut self.map).clear();
        self.shadows = None;
    }
}

/// The entry for one key of an [`ObjectMapOf`], from
/// [`ObjectMapOf::entry`]. Insertion only; see there for why.
pub enum ObjectMapEntry<'a, V> {
    /// The object already has the key.
    Occupied(OccupiedMapEntry<'a, V>),
    /// The object does not have the key.
    Vacant(VacantMapEntry<'a, V>),
}

/// An [`ObjectMapEntry`] for a key the object has: its live (last) entry.
pub struct OccupiedMapEntry<'a, V>(indexmap::map::OccupiedEntry<'a, String, V>);

/// An [`ObjectMapEntry`] for a key the object lacks.
pub struct VacantMapEntry<'a, V> {
    entry: indexmap::map::VacantEntry<'a, String, V>,
    shadows: &'a mut Option<ShadowsInnerOf<V>>,
}

impl<'a, V> OccupiedMapEntry<'a, V> {
    /// The key.
    #[inline]
    pub fn key(&self) -> &String {
        self.0.key()
    }

    /// The live value.
    #[inline]
    pub fn get(&self) -> &V {
        self.0.get()
    }

    /// The live value, mutably.
    #[inline]
    pub fn get_mut(&mut self) -> &mut V {
        self.0.get_mut()
    }

    /// The live value, mutably, for as long as the object is borrowed.
    #[inline]
    pub fn into_mut(self) -> &'a mut V {
        self.0.into_mut()
    }

    /// Replaces the live value, returning the old one.
    #[inline]
    pub fn insert(&mut self, value: V) -> V {
        self.0.insert(value)
    }
}

impl<V> VacantMapEntry<'_, V> {
    /// The key that would be inserted.
    #[inline]
    pub fn key(&self) -> &String {
        self.entry.key()
    }
}

impl<'a, V: Clone> VacantMapEntry<'a, V> {
    /// Appends the key with `value`.
    #[inline]
    pub fn insert(self, value: V) -> &'a mut V {
        let slot = self.entry.index();
        if let Some(shadows) = live_shadows_mut(self.shadows) {
            shadows.push_live(slot);
        }
        self.entry.insert(value)
    }
}

impl<V> ObjectMapEntry<'_, V> {
    /// The key this entry is for.
    #[inline]
    pub fn key(&self) -> &String {
        match self {
            Self::Occupied(entry) => entry.key(),
            Self::Vacant(entry) => entry.key(),
        }
    }
}

impl<'a, V: Clone> ObjectMapEntry<'a, V> {
    /// Runs `f` on the key's live value if the object has the key, then
    /// returns the entry for chaining.
    #[inline]
    pub fn and_modify<F: FnOnce(&mut V)>(mut self, f: F) -> Self {
        if let Self::Occupied(entry) = &mut self {
            f(entry.get_mut());
        }
        self
    }

    /// The key's live value, inserting `default` first if the key is absent.
    #[inline]
    pub fn or_insert(self, default: V) -> &'a mut V {
        self.or_insert_with(|| default)
    }

    /// The key's live value, inserting `make()` first if the key is absent.
    #[inline]
    pub fn or_insert_with<F: FnOnce() -> V>(self, make: F) -> &'a mut V {
        match self {
            Self::Occupied(entry) => entry.into_mut(),
            Self::Vacant(entry) => entry.insert(make()),
        }
    }

    /// The key's live value, inserting `V::default()` first if the key is
    /// absent.
    #[inline]
    pub fn or_default(self) -> &'a mut V
    where
        V: Default,
    {
        self.or_insert_with(V::default)
    }
}

/// Iteration over every entry of an [`ObjectMapOf`] in document order; see
/// [`ObjectMapOf::occurrences`].
pub struct Occurrences<'a, V>(OccurrencesInner<'a, V>);

enum OccurrencesInner<'a, V> {
    /// No key repeats: the collapsed view's own iterator.
    Plain(indexmap::map::Iter<'a, String, V>),
    /// Some key repeats: the side list, with live entries read from `map`.
    Ordered {
        map: &'a IndexMap<String, V>,
        entries: core::slice::Iter<'a, Occurrence<V>>,
    },
}

impl<'a, V> Occurrences<'a, V> {
    /// The same iteration, each entry flagged `true` when it is its key's live (last) entry and
    /// `false` when it is a shadow -- for a writer that keys per-entry presentation by key and must
    /// not hand a shadow its live entry's.
    #[inline]
    pub fn flagged(self) -> FlaggedOccurrences<'a, V> {
        FlaggedOccurrences(self)
    }

    #[inline]
    fn next_flagged(&mut self) -> Option<(&'a String, &'a V, bool)> {
        match &mut self.0 {
            OccurrencesInner::Plain(iter) => iter.next().map(|(k, v)| (k, v, true)),
            OccurrencesInner::Ordered { map, entries } => {
                let entry = entries.next()?;
                Self::resolve(map, entry).map(|(k, v)| (k, v, matches!(entry, Occurrence::Live(_))))
            }
        }
    }

    #[inline]
    fn resolve(
        map: &'a IndexMap<String, V>,
        entry: &'a Occurrence<V>,
    ) -> Option<(&'a String, &'a V)> {
        match entry {
            Occurrence::Shadow(key, value) => Some((key, value)),
            Occurrence::Live(slot) => map.get_index(*slot),
        }
    }
}

/// [`Occurrences::flagged`]'s iterator.
pub struct FlaggedOccurrences<'a, V>(Occurrences<'a, V>);

impl<'a, V> Iterator for FlaggedOccurrences<'a, V> {
    type Item = (&'a String, &'a V, bool);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        self.0.next_flagged()
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.0.size_hint()
    }
}

impl<'a, V> Iterator for Occurrences<'a, V> {
    type Item = (&'a String, &'a V);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        match &mut self.0 {
            OccurrencesInner::Plain(iter) => iter.next(),
            OccurrencesInner::Ordered { map, entries } => Self::resolve(map, entries.next()?),
        }
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        match &self.0 {
            OccurrencesInner::Plain(iter) => iter.size_hint(),
            OccurrencesInner::Ordered { entries, .. } => entries.size_hint(),
        }
    }
}

impl<V> DoubleEndedIterator for Occurrences<'_, V> {
    #[inline]
    fn next_back(&mut self) -> Option<Self::Item> {
        match &mut self.0 {
            OccurrencesInner::Plain(iter) => iter.next_back(),
            OccurrencesInner::Ordered { map, entries } => Self::resolve(map, entries.next_back()?),
        }
    }
}

impl<V> ExactSizeIterator for Occurrences<'_, V> {}

// Hand-written rather than derived: a derive would put a `V: Default`
// / `V: Clone` bound on the *container*, but an empty or cloned one
// needs nothing of its value type beyond what the clone itself needs.
// `OwnedValue` has no `Default` at all, so `#[derive(Default)]` would
// not even compile here.
impl<V> Default for ObjectMapOf<V> {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

/// A refcount bump per pointer in the shipped shape (the copy is deferred to
/// the first write through either handle); a deep copy under
/// `unshared-containers`.
impl<V: Clone> Clone for ObjectMapOf<V> {
    #[inline]
    fn clone(&self) -> Self {
        Self {
            map: self.map.clone(),
            shadows: self.shadows.clone(),
        }
    }
}

/// jq-value equality: the collapsed views. [`ObjectMapOf::eq_occurrences`] is
/// the comparison that sees repeated keys.
impl<V: PartialEq> PartialEq for ObjectMapOf<V> {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        // Through `Deref`, not the field: the field is an `Rc` in the
        // shipped shape and something else under `unshared-containers`.
        **self == **other
    }
}

/// Transparent, so `{:?}` on an `OwnedValue` prints the bare
/// container exactly as it did before the wrapper existed. A derived
/// `Debug` would print the wrapper's name -- a cosmetic but real
/// change to what a downstream `println!("{:?}", value)` shows, for a
/// type whose whole purpose is to be invisible.
impl<V: core::fmt::Debug> core::fmt::Debug for ObjectMapOf<V> {
    #[inline]
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        core::fmt::Debug::fmt(&**self, f)
    }
}

impl<V> Deref for ObjectMapOf<V> {
    type Target = IndexMap<String, V>;

    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.map
    }
}

/// The copy-on-write point, and the point where shadows are dropped: a raw
/// `&mut IndexMap` cannot keep them in step, so the caller gets today's
/// answer rather than a mix of today's answer and yq's.
impl<V: Clone> DerefMut for ObjectMapOf<V> {
    #[inline]
    #[cfg_attr(any(test, feature = "share-stats"), track_caller)]
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.shadows = None;
        map_mut(&mut self.map)
    }
}

impl<V> From<IndexMap<String, V>> for ObjectMapOf<V> {
    #[inline]
    fn from(map: IndexMap<String, V>) -> Self {
        #[cfg(not(feature = "unshared-containers"))]
        {
            Self {
                map: Rc::new(map),
                shadows: None,
            }
        }
        #[cfg(feature = "unshared-containers")]
        {
            Self {
                map: Box::new(map),
                shadows: None,
            }
        }
    }
}

impl<V: Clone> From<ObjectMapOf<V>> for IndexMap<String, V> {
    #[inline]
    #[cfg_attr(any(test, feature = "share-stats"), track_caller)]
    fn from(wrapper: ObjectMapOf<V>) -> Self {
        wrapper.into_index_map()
    }
}

impl<V> FromIterator<(String, V)> for ObjectMapOf<V> {
    #[inline]
    fn from_iter<I: IntoIterator<Item = (String, V)>>(iter: I) -> Self {
        Self::from(IndexMap::from_iter(iter))
    }
}

impl<V: Clone> Extend<(String, V)> for ObjectMapOf<V> {
    #[inline]
    #[cfg_attr(any(test, feature = "share-stats"), track_caller)]
    fn extend<I: IntoIterator<Item = (String, V)>>(&mut self, iter: I) {
        (**self).extend(iter);
    }
}

/// By-value iteration over an [`ObjectMapOf`]'s collapsed view.
///
/// The map's own iterator when this handle was the last one, otherwise one
/// pass over the shared map cloning each entry as it goes -- never a copy of
/// the whole map first and a second pass to consume it (#2999 review).
pub enum ObjectMapIntoIter<V> {
    /// This handle owned the map outright.
    Owned(indexmap::map::IntoIter<String, V>),
    /// Another handle still shares the map; the entries in `next..end` are
    /// cloned out, from either end (`DoubleEndedIterator`, like the owned
    /// form).
    #[cfg(not(feature = "unshared-containers"))]
    Shared {
        /// The map every remaining entry is cloned from.
        map: Rc<IndexMap<String, V>>,
        /// Index of the next entry from the front.
        next: usize,
        /// One past the next entry from the back.
        end: usize,
    },
}

impl<V: Clone> Iterator for ObjectMapIntoIter<V> {
    type Item = (String, V);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::Owned(iter) => iter.next(),
            #[cfg(not(feature = "unshared-containers"))]
            Self::Shared { map, next, end } => {
                if next >= end {
                    return None;
                }
                let (k, v) = map.get_index(*next)?;
                *next += 1;
                Some((k.clone(), v.clone()))
            }
        }
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        match self {
            Self::Owned(iter) => iter.size_hint(),
            #[cfg(not(feature = "unshared-containers"))]
            Self::Shared { next, end, .. } => {
                let n = end - next;
                (n, Some(n))
            }
        }
    }
}

impl<V: Clone> DoubleEndedIterator for ObjectMapIntoIter<V> {
    #[inline]
    fn next_back(&mut self) -> Option<Self::Item> {
        match self {
            Self::Owned(iter) => iter.next_back(),
            #[cfg(not(feature = "unshared-containers"))]
            Self::Shared { map, next, end } => {
                if next >= end {
                    return None;
                }
                *end -= 1;
                let (k, v) = map.get_index(*end)?;
                Some((k.clone(), v.clone()))
            }
        }
    }
}

impl<V: Clone> ExactSizeIterator for ObjectMapIntoIter<V> {}

impl<V: Clone> IntoIterator for ObjectMapOf<V> {
    type Item = (String, V);
    type IntoIter = ObjectMapIntoIter<V>;

    #[inline]
    #[cfg_attr(any(test, feature = "share-stats"), track_caller)]
    fn into_iter(self) -> Self::IntoIter {
        #[cfg(not(feature = "unshared-containers"))]
        {
            match Rc::try_unwrap(self.map) {
                Ok(map) => ObjectMapIntoIter::Owned(map.into_iter()),
                Err(shared) => {
                    note_forced_copy!(ObjectUnwrap);
                    let end = shared.len();
                    ObjectMapIntoIter::Shared {
                        map: shared,
                        next: 0,
                        end,
                    }
                }
            }
        }
        #[cfg(feature = "unshared-containers")]
        {
            ObjectMapIntoIter::Owned((*self.map).into_iter())
        }
    }
}

impl<'a, V> IntoIterator for &'a ObjectMapOf<V> {
    type Item = (&'a String, &'a V);
    type IntoIter = indexmap::map::Iter<'a, String, V>;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        self.map.iter()
    }
}

impl<'a, V: Clone> IntoIterator for &'a mut ObjectMapOf<V> {
    type Item = (&'a String, &'a mut V);
    type IntoIter = indexmap::map::IterMut<'a, String, V>;

    #[inline]
    #[cfg_attr(any(test, feature = "share-stats"), track_caller)]
    fn into_iter(self) -> Self::IntoIter {
        self.iter_mut()
    }
}

#[cfg(test)]
impl<V> ObjectMapOf<V> {
    /// Checks every structural invariant of the side list (I2 and the
    /// bookkeeping the shadow-aware methods rely on).
    fn assert_invariants(&self) {
        let map = &self.map;
        let Some(shadows) = self.shadows.as_deref() else {
            return;
        };
        let shadow_count = shadows
            .order
            .iter()
            .filter(|entry| matches!(entry, Occurrence::Shadow(..)))
            .count();
        assert!(shadow_count > 0, "I2: a side list with no shadow is kept");
        assert_eq!(shadows.order.len(), map.len() + shadow_count);
        assert_eq!(shadows.live_pos.len(), map.len());
        let mut seen = vec![false; map.len()];
        for (pos, entry) in shadows.order.iter().enumerate() {
            match entry {
                Occurrence::Live(slot) => {
                    assert!(!seen[*slot], "slot {slot} is live twice");
                    seen[*slot] = true;
                    assert_eq!(shadows.live_pos[*slot], pos, "live_pos[{slot}]");
                }
                Occurrence::Shadow(key, _) => {
                    assert!(map.contains_key(key), "shadow of a missing key {key}");
                }
            }
        }
        assert!(seen.iter().all(|s| *s), "a map slot is never live");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use indexmap::IndexMap;
    use proptest::prelude::*;

    type Map = ObjectMapOf<u32>;

    fn owned(map: &Map) -> Vec<(String, u32)> {
        map.occurrences().map(|(k, v)| (k.clone(), *v)).collect()
    }

    fn pairs(entries: &[(&str, u32)]) -> Vec<(String, u32)> {
        entries
            .iter()
            .map(|(k, v)| ((*k).to_string(), *v))
            .collect()
    }

    fn with_repeats(entries: &[(&str, u32)]) -> Map {
        let mut map = Map::new();
        for (k, v) in entries {
            map.push_occurrence((*k).to_string(), *v);
        }
        map.assert_invariants();
        map
    }

    #[test]
    fn a_map_without_a_repeat_has_no_shadows() {
        let map = with_repeats(&[("a", 1), ("b", 2)]);
        assert!(!map.has_shadows());
        assert_eq!(map.occurrence_len(), 2);
        assert_eq!(owned(&map), pairs(&[("a", 1), ("b", 2)]));
    }

    #[test]
    fn a_repeat_keeps_the_earlier_entry_and_collapses_to_the_last() {
        let map = with_repeats(&[("a", 1), ("a", 2), ("b", 3)]);
        assert!(map.has_shadows());
        assert_eq!(map.occurrence_len(), 3);
        assert_eq!(owned(&map), pairs(&[("a", 1), ("a", 2), ("b", 3)]));
        let collapsed: Vec<_> = map.iter().map(|(k, v)| (k.as_str(), *v)).collect();
        assert_eq!(collapsed, [("a", 2), ("b", 3)]);
        assert_eq!(map.len(), 2);
        assert_eq!(map.get("a"), Some(&2));
    }

    #[test]
    fn an_interleaved_repeat_sits_at_its_first_entry_in_the_collapsed_view() {
        // yq: `[.[]]` on `b: 1 / a: 2 / b: 3` is `[3, 2]`.
        let map = with_repeats(&[("b", 1), ("a", 2), ("b", 3)]);
        let collapsed: Vec<_> = map.values().copied().collect();
        assert_eq!(collapsed, [3, 2]);
        assert_eq!(owned(&map), pairs(&[("b", 1), ("a", 2), ("b", 3)]));
    }

    #[test]
    fn a_write_through_a_key_reaches_only_the_last_entry() {
        let mut map = with_repeats(&[("a", 1), ("a", 2), ("b", 3)]);
        // `.a = 5`
        assert_eq!(map.insert("a".into(), 5), Some(2));
        assert_eq!(owned(&map), pairs(&[("a", 1), ("a", 5), ("b", 3)]));
        // `.a |= . + 10`
        *map.get_mut("a").unwrap() += 10;
        assert_eq!(owned(&map), pairs(&[("a", 1), ("a", 15), ("b", 3)]));
        // `.[] |= . + 1` and `map_values`
        for (_, v) in &mut map {
            *v += 1;
        }
        for v in map.values_mut() {
            *v += 1;
        }
        assert_eq!(owned(&map), pairs(&[("a", 1), ("a", 17), ("b", 5)]));
        // `.c = 1`
        map.insert("c".into(), 1);
        assert_eq!(
            owned(&map),
            pairs(&[("a", 1), ("a", 17), ("b", 5), ("c", 1)])
        );
        map.assert_invariants();
    }

    #[test]
    fn a_removal_takes_every_entry_for_the_key() {
        let mut map = with_repeats(&[("a", 1), ("b", 2), ("a", 3), ("c", 4)]);
        assert_eq!(map.shift_remove("a"), Some(3));
        assert_eq!(owned(&map), pairs(&[("b", 2), ("c", 4)]));
        assert!(!map.has_shadows(), "the last shadow went with its key");
        map.assert_invariants();

        let mut map = with_repeats(&[("a", 1), ("b", 2), ("a", 3), ("c", 4)]);
        assert_eq!(map.shift_remove("b"), Some(2));
        assert_eq!(owned(&map), pairs(&[("a", 1), ("a", 3), ("c", 4)]));
        assert!(map.has_shadows());
        map.assert_invariants();
    }

    #[test]
    fn swap_remove_moves_the_last_key_into_the_hole_and_keeps_document_order() {
        let mut map = with_repeats(&[("a", 1), ("b", 2), ("c", 3), ("a", 4)]);
        assert_eq!(map.swap_remove("a"), Some(4));
        let slots: Vec<_> = map.keys().map(String::as_str).collect();
        assert_eq!(slots, ["c", "b"]);
        // No shadow is left, so the full view is the collapsed view's own
        // (permuted) order.
        assert_eq!(owned(&map), pairs(&[("c", 3), ("b", 2)]));
        assert!(!map.has_shadows());
        map.assert_invariants();

        let mut map = with_repeats(&[("a", 1), ("b", 2), ("c", 3), ("c", 4), ("a", 5)]);
        assert_eq!(map.swap_remove("a"), Some(5));
        assert_eq!(owned(&map), pairs(&[("b", 2), ("c", 3), ("c", 4)]));
        map.assert_invariants();
    }

    #[test]
    fn retain_judges_the_live_entry_and_drops_the_whole_key() {
        let mut map = with_repeats(&[("a", 1), ("b", 2), ("a", 3), ("b", 4), ("c", 5)]);
        map.retain(|_, v| *v != 3);
        assert_eq!(owned(&map), pairs(&[("b", 2), ("b", 4), ("c", 5)]));
        map.assert_invariants();
    }

    #[test]
    fn a_raw_map_borrow_collapses_first() {
        let mut map = with_repeats(&[("a", 1), ("a", 2), ("b", 3)]);
        (*map).insert("c".into(), 9);
        assert!(!map.has_shadows());
        assert_eq!(owned(&map), pairs(&[("a", 2), ("b", 3), ("c", 9)]));
        map.assert_invariants();
    }

    #[test]
    fn the_by_value_conversions_collapse() {
        let map = with_repeats(&[("a", 1), ("a", 2), ("b", 3)]);
        let owned_map: IndexMap<String, u32> = map.clone().into_index_map();
        assert_eq!(owned_map.get("a"), Some(&2));
        assert_eq!(owned_map.len(), 2);
        assert_eq!(
            map.clone().into_iter().collect::<Vec<_>>(),
            pairs(&[("a", 2), ("b", 3)])
        );
        assert_eq!(
            map.into_occurrences(),
            pairs(&[("a", 1), ("a", 2), ("b", 3)])
        );
    }

    #[test]
    fn equality_is_the_collapsed_view_and_eq_occurrences_sees_every_entry() {
        let twice = with_repeats(&[("a", 1), ("a", 2)]);
        let once = with_repeats(&[("a", 2)]);
        assert_eq!(twice, once);
        assert!(!twice.eq_occurrences(&once));
        assert!(twice.eq_occurrences(&twice.clone()));
        let other = with_repeats(&[("a", 9), ("a", 2)]);
        assert_eq!(twice, other);
        assert!(!twice.eq_occurrences(&other));
    }

    #[test]
    fn first_occurrence_finds_the_earliest_entry_and_can_write_it() {
        let mut map = with_repeats(&[("b", 0), ("a", 1), ("a", 2), ("a", 3)]);
        assert_eq!(map.first_occurrence("a"), Some(&1));
        assert_eq!(map.first_occurrence("b"), Some(&0));
        assert_eq!(map.first_occurrence("z"), None);
        // yq's `. + {"a": 9}` overwrites the first entry.
        *map.first_occurrence_mut("a").unwrap() = 9;
        assert_eq!(
            owned(&map),
            pairs(&[("b", 0), ("a", 9), ("a", 2), ("a", 3)])
        );
        assert_eq!(map.get("a"), Some(&3));
        map.assert_invariants();
    }

    #[test]
    fn a_clone_is_unaffected_by_a_write_through_the_original() {
        let mut map = with_repeats(&[("a", 1), ("a", 2)]);
        let before = map.clone();
        map.push_occurrence("a".into(), 3);
        map.insert("b".into(), 4);
        assert_eq!(owned(&before), pairs(&[("a", 1), ("a", 2)]));
        assert_eq!(
            owned(&map),
            pairs(&[("a", 1), ("a", 2), ("a", 3), ("b", 4)])
        );
    }

    #[test]
    #[cfg(not(feature = "unshared-containers"))]
    fn identity_follows_both_pointers_so_a_shadow_only_difference_is_not_the_same_node() {
        let map = with_repeats(&[("a", 1), ("a", 2)]);
        let mut copy = map.clone();
        assert!(map.ptr_eq(&copy));
        assert!(map.is_shared());
        // Writing a shadow copies the map as well as the side list.
        *copy.first_occurrence_mut("a").unwrap() = 9;
        assert!(!map.ptr_eq(&copy));
        assert_eq!(owned(&map), pairs(&[("a", 1), ("a", 2)]));
        assert_eq!(owned(&copy), pairs(&[("a", 9), ("a", 2)]));
        // Dropping the shadows leaves the map shared but is not the same node.
        let mut collapsed = map.clone();
        collapsed.collapse();
        assert!(!map.ptr_eq(&collapsed));
        assert!(!collapsed.has_shadows());
        assert_eq!(owned(&map), pairs(&[("a", 1), ("a", 2)]));
    }

    #[test]
    fn value_only_accessors_keep_the_shadows() {
        let mut map = with_repeats(&[("a", 1), ("b", 2), ("a", 3)]);
        *map.get_full_mut("a").unwrap().2 += 10;
        *map.get_key_value_mut("b").unwrap().1 += 10;
        *map.first_mut().unwrap().1 += 100;
        *map.last_mut().unwrap().1 += 1000;
        assert_eq!(owned(&map), pairs(&[("a", 1), ("b", 1012), ("a", 113)]));
        map.assert_invariants();
    }

    #[test]
    #[cfg(not(feature = "unshared-containers"))]
    fn replacing_a_live_value_does_not_copy_a_shared_side_list() {
        let map = with_repeats(&[("a", 1), ("a", 2)]);
        let mut copy = map.clone();
        copy.insert("a".into(), 9);
        copy.entry("a".into()).and_modify(|v| *v += 1);
        assert!(
            Rc::ptr_eq(
                map.shadows.as_ref().unwrap(),
                copy.shadows.as_ref().unwrap()
            ),
            "the side list was copied by a write that did not change it"
        );
        assert_eq!(owned(&copy), pairs(&[("a", 1), ("a", 10)]));
        // A new key does change it.
        copy.insert("b".into(), 5);
        assert!(!Rc::ptr_eq(
            map.shadows.as_ref().unwrap(),
            copy.shadows.as_ref().unwrap()
        ));
        assert_eq!(owned(&map), pairs(&[("a", 1), ("a", 2)]));
    }

    #[test]
    fn the_entry_api_appends_a_vacant_key_in_document_order() {
        let mut map = with_repeats(&[("a", 1), ("a", 2)]);
        *map.entry("b".into()).or_insert(5) += 1;
        *map.entry("a".into()).or_insert(0) += 10;
        map.entry("a".into()).and_modify(|v| *v += 100).or_default();
        assert_eq!(owned(&map), pairs(&[("a", 1), ("a", 112), ("b", 6)]));
        map.assert_invariants();
    }

    #[test]
    fn clear_forgets_the_shadows() {
        let mut map = with_repeats(&[("a", 1), ("a", 2)]);
        map.clear();
        assert!(map.is_empty());
        assert!(!map.has_shadows());
        assert_eq!(map.occurrence_len(), 0);
    }

    #[test]
    fn a_map_without_a_repeat_behaves_as_its_index_map() {
        let mut map: Map = [("a".to_string(), 1), ("b".to_string(), 2)]
            .into_iter()
            .collect();
        map.insert("c".into(), 3);
        map.shift_remove("a");
        map.swap_remove("c");
        assert!(!map.has_shadows());
        assert_eq!(owned(&map), pairs(&[("b", 2)]));
        assert_eq!(map.occurrences().len(), 1);
        assert_eq!(map.occurrences().next_back(), Some((&"b".to_string(), &2)));
    }

    /// One step of the model-based test.
    #[derive(Debug, Clone)]
    enum Op {
        Insert(u8, u32),
        PushOccurrence(u8, u32),
        GetMut(u8, u32),
        IterMutAdd(u32),
        EntryOrInsert(u8, u32),
        EntryAndModify(u8, u32),
        FirstOccurrenceMut(u8, u32),
        ShiftRemove(u8),
        SwapRemove(u8),
        Retain(u32),
        Clear,
        RawDerefMut(u8, u32),
        CloneThenWriteTheOriginal(u8, u32),
        IntoOccurrences,
    }

    fn key(k: u8) -> String {
        // A small alphabet, so repeats are common.
        char::from(b'a' + k % 5).to_string()
    }

    fn op() -> impl Strategy<Value = Op> {
        let k = 0u8..8;
        let v = 0u32..1000;
        prop_oneof![
            4 => (k.clone(), v.clone()).prop_map(|(k, v)| Op::Insert(k, v)),
            6 => (k.clone(), v.clone()).prop_map(|(k, v)| Op::PushOccurrence(k, v)),
            2 => (k.clone(), v.clone()).prop_map(|(k, v)| Op::GetMut(k, v)),
            1 => v.clone().prop_map(Op::IterMutAdd),
            2 => (k.clone(), v.clone()).prop_map(|(k, v)| Op::EntryOrInsert(k, v)),
            2 => (k.clone(), v.clone()).prop_map(|(k, v)| Op::EntryAndModify(k, v)),
            2 => (k.clone(), v.clone()).prop_map(|(k, v)| Op::FirstOccurrenceMut(k, v)),
            3 => k.clone().prop_map(Op::ShiftRemove),
            2 => k.clone().prop_map(Op::SwapRemove),
            1 => (2u32..5).prop_map(Op::Retain),
            1 => Just(Op::Clear),
            1 => (k.clone(), v.clone()).prop_map(|(k, v)| Op::RawDerefMut(k, v)),
            1 => (k, v).prop_map(|(k, v)| Op::CloneThenWriteTheOriginal(k, v)),
            1 => Just(Op::IntoOccurrences),
        ]
    }

    /// A reference implementation: every entry in document order, plus the
    /// `IndexMap` the collapsed view must equal -- kept in step by applying
    /// each operation to a real `IndexMap` directly, so the slot order after
    /// a `swap_remove` is `IndexMap`'s, not a re-derivation of it.
    struct Reference {
        entries: Vec<(String, u32)>,
        collapsed: IndexMap<String, u32>,
    }

    impl Reference {
        fn last_index(&self, key: &str) -> Option<usize> {
            self.entries.iter().rposition(|(k, _)| k == key)
        }

        fn remove_key(&mut self, key: &str) {
            self.entries.retain(|(k, _)| k != key);
        }
    }

    fn check(map: &Map, reference: &mut Reference) {
        if reference.entries.len() == reference.collapsed.len() {
            // With no repeated key the full view *is* the collapsed view, in
            // its slot order -- which a `swap_remove` earlier in the run may
            // have permuted away from the document order.
            reference.entries = reference
                .collapsed
                .iter()
                .map(|(k, v)| (k.clone(), *v))
                .collect();
        }
        map.assert_invariants();
        // I1: the collapsed view is the reference IndexMap, slot for slot.
        assert!(
            map.iter().eq(reference.collapsed.iter()),
            "collapsed view {:?} != {:?}",
            map.iter().collect::<Vec<_>>(),
            reference.collapsed
        );
        // The full view is the reference's document order.
        assert_eq!(owned(map), reference.entries);
        assert_eq!(map.occurrence_len(), reference.entries.len());
        // I2.
        let repeats = reference.entries.len() != reference.collapsed.len();
        assert_eq!(map.has_shadows(), repeats);
        for k in 0..5 {
            let k = key(k);
            let first = reference.entries.iter().find(|(e, _)| *e == k);
            assert_eq!(map.first_occurrence(&k), first.map(|(_, v)| v));
        }
    }

    fn apply(map: &mut Map, reference: &mut Reference, op: &Op) {
        match op {
            Op::Insert(k, v) => {
                let k = key(*k);
                let old = map.insert(k.clone(), *v);
                assert_eq!(old, reference.collapsed.insert(k.clone(), *v));
                match reference.last_index(&k) {
                    Some(i) => reference.entries[i].1 = *v,
                    None => reference.entries.push((k, *v)),
                }
            }
            Op::PushOccurrence(k, v) => {
                let k = key(*k);
                map.push_occurrence(k.clone(), *v);
                reference.collapsed.insert(k.clone(), *v);
                reference.entries.push((k, *v));
            }
            Op::GetMut(k, v) => {
                let k = key(*k);
                if let Some(slot) = map.get_mut(&k) {
                    *slot = *v;
                    *reference.collapsed.get_mut(&k).unwrap() = *v;
                    let i = reference.last_index(&k).unwrap();
                    reference.entries[i].1 = *v;
                } else {
                    assert!(!reference.collapsed.contains_key(&k));
                }
            }
            Op::IterMutAdd(d) => {
                for (_, v) in map.iter_mut() {
                    *v += d;
                }
                for v in reference.collapsed.values_mut() {
                    *v += d;
                }
                let keys: Vec<String> = reference.collapsed.keys().cloned().collect();
                for k in keys {
                    let i = reference.last_index(&k).unwrap();
                    reference.entries[i].1 += d;
                }
            }
            Op::EntryOrInsert(k, v) => {
                let k = key(*k);
                let got = *map.entry(k.clone()).or_insert(*v);
                assert_eq!(got, *reference.collapsed.entry(k.clone()).or_insert(*v));
                if reference.last_index(&k).is_none() {
                    reference.entries.push((k, *v));
                }
            }
            Op::EntryAndModify(k, v) => {
                let k = key(*k);
                map.entry(k.clone())
                    .and_modify(|slot| *slot += v)
                    .or_default();
                reference
                    .collapsed
                    .entry(k.clone())
                    .and_modify(|slot| *slot += v)
                    .or_default();
                match reference.last_index(&k) {
                    Some(i) => reference.entries[i].1 += v,
                    None => reference.entries.push((k, 0)),
                }
            }
            Op::FirstOccurrenceMut(k, v) => {
                let k = key(*k);
                if let Some(slot) = map.first_occurrence_mut(&k) {
                    *slot = *v;
                    let first = reference.entries.iter().position(|(e, _)| *e == k).unwrap();
                    reference.entries[first].1 = *v;
                    // The collapsed view holds the last entry, which is the
                    // first one only when the key does not repeat.
                    if reference.last_index(&k) == Some(first) {
                        *reference.collapsed.get_mut(&k).unwrap() = *v;
                    }
                } else {
                    assert!(!reference.collapsed.contains_key(&k));
                }
            }
            Op::ShiftRemove(k) => {
                let k = key(*k);
                assert_eq!(map.shift_remove(&k), reference.collapsed.shift_remove(&k));
                reference.remove_key(&k);
            }
            Op::SwapRemove(k) => {
                let k = key(*k);
                assert_eq!(map.swap_remove(&k), reference.collapsed.swap_remove(&k));
                reference.remove_key(&k);
            }
            Op::Retain(m) => {
                let dropped: Vec<String> = reference
                    .collapsed
                    .iter()
                    .filter(|(_, v)| **v % m == 0)
                    .map(|(k, _)| k.clone())
                    .collect();
                map.retain(|_, v| *v % m != 0);
                reference.collapsed.retain(|_, v| *v % m != 0);
                for k in dropped {
                    reference.remove_key(&k);
                }
            }
            Op::Clear => {
                map.clear();
                reference.collapsed.clear();
                reference.entries.clear();
            }
            Op::RawDerefMut(k, v) => {
                // A raw `&mut IndexMap` collapses: the document order becomes
                // the collapsed view's slot order.
                let k = key(*k);
                (**map).insert(k.clone(), *v);
                reference.collapsed.insert(k, *v);
                reference.entries = reference
                    .collapsed
                    .iter()
                    .map(|(k, v)| (k.clone(), *v))
                    .collect();
                assert!(!map.has_shadows());
            }
            Op::CloneThenWriteTheOriginal(k, v) => {
                let before = map.clone();
                let was = owned(&before);
                apply(map, reference, &Op::PushOccurrence(*k, *v));
                assert_eq!(
                    owned(&before),
                    was,
                    "a clone saw a write through the original"
                );
                before.assert_invariants();
            }
            Op::IntoOccurrences => {
                assert_eq!(map.clone().into_occurrences(), reference.entries);
            }
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(512))]

        #[test]
        fn matches_a_multimap_reference_after_every_operation(
            ops in proptest::collection::vec(op(), 0..48)
        ) {
            let mut map = Map::new();
            let mut reference = Reference {
                entries: Vec::new(),
                collapsed: IndexMap::new(),
            };
            check(&map, &mut reference);
            for op in &ops {
                apply(&mut map, &mut reference, op);
                check(&map, &mut reference);
            }
        }

        #[test]
        fn a_sequence_that_never_repeats_a_key_equals_its_index_map(
            ops in proptest::collection::vec(op(), 0..48)
        ) {
            let mut map = Map::new();
            let mut plain: IndexMap<String, u32> = IndexMap::new();
            for op in &ops {
                match op {
                    Op::Insert(k, v) | Op::PushOccurrence(k, v) => {
                        let k = key(*k);
                        if plain.contains_key(&k) {
                            continue;
                        }
                        map.push_occurrence(k.clone(), *v);
                        plain.insert(k, *v);
                    }
                    Op::ShiftRemove(k) => {
                        let k = key(*k);
                        prop_assert_eq!(map.shift_remove(&k), plain.shift_remove(&k));
                    }
                    Op::SwapRemove(k) => {
                        let k = key(*k);
                        prop_assert_eq!(map.swap_remove(&k), plain.swap_remove(&k));
                    }
                    Op::GetMut(k, v) => {
                        let k = key(*k);
                        prop_assert_eq!(map.get_mut(&k).map(|s| { *s = *v; *v }),
                                        plain.get_mut(&k).map(|s| { *s = *v; *v }));
                    }
                    Op::Retain(m) => {
                        map.retain(|_, v| *v % m != 0);
                        plain.retain(|_, v| *v % m != 0);
                    }
                    _ => {}
                }
                prop_assert!(!map.has_shadows());
                prop_assert!(map.iter().eq(plain.iter()));
                prop_assert!(map.occurrences().eq(plain.iter()));
            }
        }
    }
}
