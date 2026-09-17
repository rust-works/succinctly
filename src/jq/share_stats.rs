//! Copy-on-write instrumentation for `OwnedValue`'s shared containers (#2999).
//!
//! Since #2999, `OwnedValue::Array` and `OwnedValue::Object` hold their
//! storage behind a refcount ([`ArrayOf`](super::value::ArrayOf) /
//! [`ObjectMapOf`](super::value::ObjectMapOf)), so cloning a value is a
//! pointer bump and the deep copy happens only when a *shared* container is
//! written through (`Rc::make_mut`) or consumed by value (`Rc::try_unwrap`
//! failing). That is the mechanism the whole change exists for, and it is
//! also its one risk class: a by-value destructure or an `&mut` deref that
//! *used* to be free because the value was uniquely owned silently becomes a
//! whole-subtree copy if something else still holds the container.
//!
//! An A/B cannot tell those two apart. This module can: every forced copy is
//! recorded with the file and line that forced it (`#[track_caller]` on the
//! wrappers' `DerefMut`/unwrap paths -- which does see through implicit
//! auto-deref), so a run over the test suite or a benchmark query yields the
//! exact list of sites that copied, for ADR-0024's fourth acceptance
//! criterion ("the clone list is empty or every entry is explained").
//!
//! # Zero cost when disabled
//!
//! The wrappers call [`record`] only under
//! `#[cfg(any(test, feature = "share-stats"))]`. The shipped build carries
//! the refcount check `Rc::make_mut` needs anyway and nothing else. This
//! module itself is compiled under `std`/`test` (it needs thread-local
//! storage), the same shape as `util::select_stats` (#40), so its own logic
//! is covered by the ordinary test run.
//!
//! # Usage
//!
//! ```text
//! SUCCINCTLY_SHARE_STATS=1 target/release/succinctly jq '.[(0,1)] = 0' big.json
//! ```
//!
//! with a binary built `--features cli,share-stats` prints, on stderr at exit,
//! one line per (kind, file, line) that forced a copy, with its count.

use core::cell::RefCell;

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;

/// What kind of forced copy happened.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    /// A shared array was written through (`DerefMut` -> `Rc::make_mut` cloned).
    ArrayMakeMut,
    /// A shared object map was written through.
    ObjectMakeMut,
    /// A shared array was consumed by value (`into_vec`/`into_iter` cloned).
    ArrayUnwrap,
    /// A shared object map was consumed by value (`into_index_map`/`into_iter` cloned).
    ObjectUnwrap,
}

impl Kind {
    /// Short label for reports.
    pub fn label(self) -> &'static str {
        match self {
            Self::ArrayMakeMut => "array make_mut",
            Self::ObjectMakeMut => "object make_mut",
            Self::ArrayUnwrap => "array unwrap",
            Self::ObjectUnwrap => "object unwrap",
        }
    }
}

/// One forced copy: where, what kind, and how many handles shared the storage.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Site {
    /// Source file of the call that forced the copy.
    pub file: &'static str,
    /// Line of that call.
    pub line: u32,
    /// Which wrapper path copied.
    pub kind: Kind,
}

thread_local! {
    static EVENTS: RefCell<BTreeMap<Site, u64>> = const { RefCell::new(BTreeMap::new()) };
}

/// Record one forced copy at `site`.
///
/// Called by the container wrappers, never directly.
#[inline]
pub fn record(site: Site) {
    EVENTS.with(|events| {
        *events.borrow_mut().entry(site).or_insert(0) += 1;
    });
}

/// Build the [`Site`] for the caller of the wrapper method that observed a
/// shared container. `#[track_caller]` here means the location reported is
/// the wrapper's *own* caller, i.e. the `&mut`/by-value site in `eval.rs`
/// or wherever, provided the wrapper method is itself `#[track_caller]`.
#[inline]
#[track_caller]
pub fn site(kind: Kind) -> Site {
    let location = core::panic::Location::caller();
    Site {
        file: location.file(),
        line: location.line(),
        kind,
    }
}

/// Total forced copies of `kind` recorded on this thread since the last
/// [`reset`].
pub fn count(kind: Kind) -> u64 {
    EVENTS.with(|events| {
        events
            .borrow()
            .iter()
            .filter(|(site, _)| site.kind == kind)
            .map(|(_, n)| *n)
            .sum()
    })
}

/// Total forced copies of every kind on this thread since the last [`reset`].
pub fn total() -> u64 {
    EVENTS.with(|events| events.borrow().values().sum())
}

/// Snapshot of every recorded site with its count, sorted by site.
pub fn sites() -> Vec<(Site, u64)> {
    EVENTS.with(|events| events.borrow().iter().map(|(s, n)| (*s, *n)).collect())
}

/// Forget everything recorded on this thread.
pub fn reset() {
    EVENTS.with(|events| events.borrow_mut().clear());
}

/// Render the recorded sites as one line each: `count  kind  file:line`.
///
/// Empty when nothing was forced, which is the answer ADR-0024's criterion 4
/// wants to see.
pub fn report() -> String {
    use core::fmt::Write as _;
    let mut out = String::new();
    for (site, n) in sites() {
        let _ = writeln!(
            out,
            "{n:>8}  {:<16} {}:{}",
            site.kind.label(),
            site.file,
            site.line
        );
    }
    out
}

/// Run `f` with a clean counter and return what it recorded, restoring
/// nothing else -- a test helper so unrelated recordings from earlier in the
/// same test thread cannot leak into an assertion.
pub fn measure<T>(f: impl FnOnce() -> T) -> (T, Vec<(Site, u64)>) {
    reset();
    let out = f();
    let recorded = sites();
    reset();
    (out, recorded)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_count_and_report_per_site() {
        let (_, recorded) = measure(|| {
            record(site(Kind::ArrayMakeMut));
            record(site(Kind::ArrayMakeMut));
            record(site(Kind::ObjectUnwrap));
        });
        assert_eq!(recorded.len(), 3, "three distinct call lines");
        assert!(recorded.iter().all(|(_, n)| *n == 1));
        assert!(recorded
            .iter()
            .all(|(s, _)| s.file.ends_with("share_stats.rs")));
        assert_eq!(total(), 0, "measure() resets after it reads");

        // Same line twice collapses into one entry with count 2.
        reset();
        let s = site(Kind::ObjectMakeMut);
        record(s);
        record(s);
        assert_eq!(count(Kind::ObjectMakeMut), 2);
        assert_eq!(count(Kind::ArrayMakeMut), 0);
        let text = report();
        assert!(text.contains("object make_mut"), "{text}");
        assert!(text.contains("share_stats.rs:"), "{text}");
        assert!(text.trim_start().starts_with('2'), "{text}");
        reset();
        assert!(report().is_empty());
    }
}
