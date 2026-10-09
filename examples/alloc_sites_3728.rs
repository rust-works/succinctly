//! Allocation-site sampler (#3728, #4159): *where* one jq evaluation allocates.
//!
//! `alloc_probe_3022` says how many allocator calls an evaluation makes; this
//! says which call sites they come from. It records a backtrace for every
//! `PERIOD`th allocator call inside the counted window and groups the samples
//! by their first few `succinctly::` frames.
//!
//! Parsing, indexing and one warm-up evaluation happen outside the counted
//! window, as in `alloc_probe_3022`. JSON and jq mode only.
//!
//! ```text
//! CARGO_PROFILE_RELEASE_DEBUG=1 CARGO_PROFILE_RELEASE_LTO=off \
//! CARGO_PROFILE_RELEASE_CODEGEN_UNITS=16 CARGO_TARGET_DIR=target/sites \
//!     cargo run --release --example alloc_sites_3728 -- <file.json> <query> [period] [frames]
//! ```
//!
//! The build settings matter: the shipped profile (fat LTO, one codegen unit,
//! no debug info) gives no `file:line`, and a callee inlined into its caller
//! is not a frame at all, so its allocations are attributed to the caller.
//! Counts from the attribution build can differ slightly from the shipped
//! one, so re-check the headline number with `alloc_probe_3022`.
//!
//! Two traps (see `docs/guides/benchmarking.md`, rule 11):
//!
//! * The re-entrancy guard is tested *before* the sampling counter advances.
//!   Capturing a backtrace allocates; if those calls advance the counter too,
//!   `allocs` is inflated (255,740 against 14,013 on the `path(.[])?` row of
//!   #4157) and the samples are biased towards whatever the counter lands on
//!   after each capture, so the mix changes with the period.
//! * A sample every `period`th call aliases with an allocation pattern that
//!   repeats every `k` calls when `period` and `k` share a factor: a loop that
//!   allocates seven times per element, sampled with `period=7` or `14`, sees
//!   one of its seven sites. Use a prime period and read two of them.
//! * It is a single-threaded probe: the sample table is behind a mutex taken
//!   inside the allocator, so another thread allocating in the window would
//!   serialise behind symbol resolution.
//! * `Backtrace::force_capture` is used, not `capture`: the latter reads
//!   `RUST_BACKTRACE` and returns nothing when it is unset.

// A sampling global allocator is the whole point of this example, and
// `GlobalAlloc` is an unsafe trait. The crate's own `unsafe_code = "deny"`
// policy (Cargo.toml) asks for a localized, reasoned opt-in; nothing here
// ships in the library.
#![allow(unsafe_code)]

use std::alloc::{GlobalAlloc, Layout, System};
use std::backtrace::Backtrace;
use std::cell::Cell;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Mutex;

use succinctly::jq::eval_generic::{eval_with_cursor_using, GenericResult};
use succinctly::jq::{parse_with_mode_and_extensions, JqSemantics, ParserMode};
use succinctly::json::JsonIndex;

static ARMED: AtomicBool = AtomicBool::new(false);
/// Allocator calls seen inside the window, outside the sampler itself.
static CALLS: AtomicUsize = AtomicUsize::new(0);
static PERIOD: AtomicUsize = AtomicUsize::new(37);
static DEPTH: AtomicUsize = AtomicUsize::new(3);
/// Samples per call site, interned as they are taken so that memory follows
/// the number of distinct sites rather than the number of samples. Updated
/// under the guard below.
static SITES: Mutex<BTreeMap<String, usize>> = Mutex::new(BTreeMap::new());

thread_local! {
    // Const-initialised and without a destructor, so touching it from inside
    // the allocator never allocates.
    static IN_SAMPLER: Cell<bool> = const { Cell::new(false) };
}

struct Sampling;

impl Sampling {
    fn note() {
        if !ARMED.load(Ordering::Relaxed) {
            return;
        }
        // The guard comes first: the backtrace below allocates, and those
        // calls must neither be sampled nor advance the counter.
        if IN_SAMPLER.with(|g| g.replace(true)) {
            return;
        }
        let n = CALLS.fetch_add(1, Ordering::Relaxed) + 1;
        if n % PERIOD.load(Ordering::Relaxed) == 0 {
            let rendered = Backtrace::force_capture().to_string();
            let site = site(&rendered, DEPTH.load(Ordering::Relaxed));
            *SITES.lock().unwrap().entry(site).or_default() += 1;
        }
        IN_SAMPLER.with(|g| g.set(false));
    }
}

unsafe impl GlobalAlloc for Sampling {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        Self::note();
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        Self::note();
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        Self::note();
        unsafe { System.realloc(ptr, layout, new_size) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static ALLOCATOR: Sampling = Sampling;

/// `name` without its generic arguments (`f::<A, B<C>>::g` -> `f::g`), and a
/// trait-impl frame `<T as Trait>::m` as `T::m`.
fn strip_generics(name: &str) -> String {
    let mut name = name.to_string();
    if name.starts_with('<') {
        // `<T as Trait>::m`: find the `>` closing the leading `<`.
        let mut nesting = 0usize;
        let close = name.char_indices().find_map(|(i, c)| {
            nesting = match c {
                '<' => nesting + 1,
                '>' => nesting - 1,
                _ => nesting,
            };
            (nesting == 0).then_some(i)
        });
        if let Some(close) = close {
            let inner = &name[1..close];
            let ty = inner.split_once(" as ").map_or(inner, |(ty, _)| ty);
            name = format!("{ty}{}", &name[close + 1..]);
        }
    }
    let mut out = String::with_capacity(name.len());
    let mut nesting = 0usize;
    for c in name.chars() {
        match c {
            '<' => nesting += 1,
            '>' => nesting = nesting.saturating_sub(1),
            _ if nesting == 0 => out.push(c),
            _ => {}
        }
    }
    // `f::<A>::g` leaves `f::::g`, and `f::<A>` leaves `f::`.
    out.replace("::::", "::").trim_end_matches("::").to_string()
}

/// The first `depth` `succinctly::` frames of a rendered backtrace, innermost
/// first, as `name (file:line)`.
fn site(rendered: &str, depth: usize) -> String {
    let mut frames = Vec::new();
    let mut lines = rendered.lines().peekable();
    while let Some(line) = lines.next() {
        let Some((_, name)) = line.trim_start().split_once(": ") else {
            continue;
        };
        // An inherent frame, or a trait-impl frame `<succinctly::T as Trait>::m`.
        if !(name.starts_with("succinctly::") || name.starts_with("<succinctly::")) {
            continue;
        }
        let at = lines
            .peek()
            .and_then(|next| next.trim_start().strip_prefix("at "))
            .map(|location| {
                // `/abs/path/src/jq/eval.rs:123:45` -> `src/jq/eval.rs:123`
                let location = location.rsplit_once(':').map_or(location, |(head, _)| head);
                location.find("src/").map_or(location, |i| &location[i..])
            });
        let name = strip_generics(name);
        frames.push(match at {
            Some(at) => format!("{name} ({at})"),
            None => name,
        });
        if frames.len() == depth {
            break;
        }
    }
    if frames.is_empty() {
        "<no succinctly:: frame>".to_string()
    } else {
        frames.join("\n      <- ")
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [path, query, rest @ ..] = args.as_slice() else {
        panic!("usage: alloc_sites_3728 <file.json> <query> [period] [frames]");
    };
    let period: usize = rest.first().map_or(37, |p| p.parse().expect("period"));
    let depth: usize = rest.get(1).map_or(3, |d| d.parse().expect("frames"));
    assert!(period > 0 && depth > 0);
    PERIOD.store(period, Ordering::SeqCst);
    DEPTH.store(depth, Ordering::SeqCst);

    let bytes = std::fs::read(path).expect("read fixture");
    let expr = parse_with_mode_and_extensions(query, ParserMode::Jq, true).expect("parse query");
    let index = JsonIndex::build(&bytes);
    let root = index.root(&bytes);

    let run = || {
        match eval_with_cursor_using::<JqSemantics, _>(&expr, root) {
            GenericResult::One(_)
            | GenericResult::OneCursor(_)
            | GenericResult::Many(_)
            | GenericResult::ManyCursor(_)
            | GenericResult::ManyOwned(_)
            | GenericResult::Owned(_)
            | GenericResult::None => {}
            // Rule 9 of the benchmarking guide: never record a row that is
            // really an error path, however plausible its number looks. A lazy
            // result is the same trap from the other side: it would leave the
            // work the caller is owed outside the counted window.
            GenericResult::Error(e) => panic!("query failed: {e:?}"),
            GenericResult::LazyKeys { .. }
            | GenericResult::LazyIndexRange(_)
            | GenericResult::LazySeq(_)
            | GenericResult::LazyObject(_) => {
                panic!("query left a lazy result; this probe expects a settled one")
            }
            GenericResult::Break(l) => panic!("query broke out to label {l:?}"),
            GenericResult::Halt(c) => panic!("query halted with {c}"),
            GenericResult::Partial(..) => panic!("query returned a partial result"),
        }
    };
    run();
    ARMED.store(true, Ordering::SeqCst);
    run();
    ARMED.store(false, Ordering::SeqCst);

    let calls = CALLS.load(Ordering::SeqCst);
    let sites = std::mem::take(&mut *SITES.lock().unwrap());
    let samples: usize = sites.values().sum();
    let mut ranked: Vec<_> = sites.into_iter().collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));

    println!("allocs={calls} samples={samples} period={period}");
    for (site, n) in ranked.iter().take(12) {
        let share = 100.0 * *n as f64 / samples.max(1) as f64;
        println!("{n:6} {share:5.1}%  {site}");
    }
}
