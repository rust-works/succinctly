//! A note from the streamers to a colorizer about the part of a result they are writing (#5416).
//!
//! The streamers write plain text through a `core::fmt::Write`, and `succinctly yq -C` colors that
//! text as it passes. Most of it is the document, which yq colors by tokenizing it. A few parts yq
//! handles outside that: the comments and `---` lines it keeps from before a document, a document's
//! trailing comments, and a scalar result it prints without any YAML around it. The text alone does
//! not say which part it is (the string `a: 1` is a valid scalar result, and a valid mapping), so a
//! streamer says so here before writing the part, and the colorizer reads it with each write.
//!
//! The note is per thread and is read while the writer is still inside the call that set it, so a
//! [`RegionGuard`] held across the writes is all a streamer has to do. It costs one relaxed load
//! unless a colorizer has called [`enable`]: the streamers run it once per result whether anything
//! is listening or not. Without `std` there is no colorizer, and the note is dropped.

/// The part of a result being written.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum StreamRegion {
    /// The document, or the part of it a result is made of: tokenized and colored by its tokens.
    #[default]
    Body,
    /// Comments, blank lines and `---` lines yq keeps from before the first document: printed as
    /// they are, in grey, apart from the document.
    LeadingContent,
    /// A document's trailing comments, which yq adds to the text it then tokenizes.
    TrailingContent,
    /// A scalar result printed without quotes or any other YAML around it: not colored at all.
    RawScalar,
}

#[cfg(feature = "std")]
mod imp {
    use super::StreamRegion;
    use std::cell::Cell;
    use std::sync::atomic::{AtomicBool, Ordering};

    /// Whether anything will read the note.
    static ENABLED: AtomicBool = AtomicBool::new(false);

    pub(super) fn enable() {
        ENABLED.store(true, Ordering::Relaxed);
    }

    pub(super) fn enabled() -> bool {
        ENABLED.load(Ordering::Relaxed)
    }

    std::thread_local! {
        static REGION: Cell<StreamRegion> = const { Cell::new(StreamRegion::Body) };
    }

    pub(super) fn set(region: StreamRegion) -> StreamRegion {
        REGION.with(|r| r.replace(region))
    }

    pub(super) fn get() -> StreamRegion {
        REGION.with(Cell::get)
    }
}

#[cfg(not(feature = "std"))]
mod imp {
    use super::StreamRegion;

    pub(super) fn enable() {}

    pub(super) fn enabled() -> bool {
        false
    }

    pub(super) fn set(_region: StreamRegion) -> StreamRegion {
        StreamRegion::Body
    }

    pub(super) fn get() -> StreamRegion {
        StreamRegion::Body
    }
}

/// Asks the streamers to leave the note: a colorizer calls this before it reads one. Once asked,
/// every thread's streamers leave it.
pub fn enable() {
    imp::enable();
}

/// The region the streamer on this thread is writing now.
pub fn current_region() -> StreamRegion {
    imp::get()
}

/// Holds a region for as long as it lives, then puts back the one it replaced.
#[must_use = "the region ends when the guard is dropped"]
pub struct RegionGuard {
    /// The region replaced, or `None` when nothing is listening and none was set.
    previous: Option<StreamRegion>,
}

impl RegionGuard {
    /// Enters `region`.
    pub fn enter(region: StreamRegion) -> Self {
        Self {
            previous: imp::enabled().then(|| imp::set(region)),
        }
    }
}

impl Drop for RegionGuard {
    fn drop(&mut self) {
        if let Some(previous) = self.previous {
            imp::set(previous);
        }
    }
}

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;

    #[test]
    fn a_guard_restores_the_region_it_replaced() {
        enable();
        assert_eq!(current_region(), StreamRegion::Body);
        {
            let _outer = RegionGuard::enter(StreamRegion::LeadingContent);
            assert_eq!(current_region(), StreamRegion::LeadingContent);
            {
                let _inner = RegionGuard::enter(StreamRegion::RawScalar);
                assert_eq!(current_region(), StreamRegion::RawScalar);
            }
            assert_eq!(current_region(), StreamRegion::LeadingContent);
        }
        assert_eq!(current_region(), StreamRegion::Body);
    }
}
