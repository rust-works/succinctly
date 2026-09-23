//! Allocation probe for #3022: how many allocator calls one evaluation makes.
//!
//! Parsing, indexing, cursor creation and one warm-up evaluation all happen
//! outside the counted window, so the number is the evaluation's own term and
//! not the fixture's. Counts `alloc`, `alloc_zeroed` and `realloc` calls; it
//! does not measure bytes or time.
//!
//!     cargo run --release --example alloc_probe_3022 -- <jq|yq> <file> <query>

// A counting global allocator is the whole point of this example, and
// `GlobalAlloc` is an unsafe trait. The crate's own `unsafe_code = "deny"`
// policy (Cargo.toml) asks for a localized, reasoned opt-in; nothing here
// ships in the library.
#![allow(unsafe_code)]

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use succinctly::jq::eval_generic::{eval_with_cursor_using, GenericResult};
use succinctly::jq::OwnedValue;
use succinctly::jq::{parse_with_mode_and_extensions, JqSemantics, ParserMode, YqSemantics};
use succinctly::json::JsonIndex;
use succinctly::yaml::{YamlIndex, YamlValue};

static COUNT: AtomicUsize = AtomicUsize::new(0);
static ARMED: AtomicBool = AtomicBool::new(false);

struct Counting;

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if ARMED.load(Ordering::Relaxed) {
            COUNT.fetch_add(1, Ordering::Relaxed);
        }
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        if ARMED.load(Ordering::Relaxed) {
            COUNT.fetch_add(1, Ordering::Relaxed);
        }
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        if ARMED.load(Ordering::Relaxed) {
            COUNT.fetch_add(1, Ordering::Relaxed);
        }
        unsafe { System.realloc(ptr, layout, new_size) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

/// Force every result to the same amount of work in both builds. A macro
/// rather than a function because `DocumentValue` is crate-private, so this
/// cannot be written generically from outside.
///
/// Every row this probe records produces a settled scalar (`length` or
/// `first`), so anything lazy means the query is not measuring what the row
/// claims.
macro_rules! settle {
    ($result:expr) => {
        match $result {
            GenericResult::One(_) | GenericResult::OneCursor(_) => (1usize, None),
            GenericResult::Many(v) => (v.len(), None),
            GenericResult::ManyCursor(v) => (v.len(), None),
            GenericResult::ManyOwned(v) => (v.len(), None),
            GenericResult::Owned(v) => (1usize, Some(alloc_probe_render(&v))),
            GenericResult::None => (0usize, None),
            // Rule 9 of the benchmarking guide: never record a row that is
            // really an error path, however plausible its number looks.
            GenericResult::Error(e) => panic!("query failed: {e:?}"),
            GenericResult::LazyKeys { .. }
            | GenericResult::LazyIndexRange(_)
            | GenericResult::LazySeq(_) => {
                panic!("query left a lazy result; this probe expects a settled one")
            }
            GenericResult::Break(l) => panic!("query broke out to label {l:?}"),
            GenericResult::Halt(c) => panic!("query halted with {c}"),
            GenericResult::Partial(..) => panic!("query returned a partial result"),
        }
    };
}

/// The answer itself, so a row that silently stops doing work is visible.
fn alloc_probe_render(value: &OwnedValue) -> String {
    format!("{value:?}")
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [mode, path, query] = <[String; 3]>::try_from(args)
        .unwrap_or_else(|_| panic!("usage: alloc_probe_3022 <jq|yq> <file> <query>"));

    let bytes = std::fs::read(&path).expect("read fixture");
    let parser_mode = match mode.as_str() {
        "jq" => ParserMode::Jq,
        "yq" => ParserMode::Yq,
        other => panic!("mode must be jq or yq, got {other}"),
    };
    // yq's lexer rejects the jq-only builtins these rows use; the CLI opts in
    // with --jq-extensions, so the probe does too.
    let expr = parse_with_mode_and_extensions(&query, parser_mode, true).expect("parse query");

    let mut answer = None;
    let mut reached = 0usize;
    let counted;
    if path.ends_with(".json") {
        let index = JsonIndex::build(&bytes);
        let root = index.root(&bytes);
        let (n, rendered) = match parser_mode {
            ParserMode::Jq => settle!(eval_with_cursor_using::<JqSemantics, _>(&expr, root)),
            _ => settle!(eval_with_cursor_using::<YqSemantics, _>(&expr, root)),
        };
        reached += n;
        answer = answer.or(rendered);
        ARMED.store(true, Ordering::SeqCst);
        COUNT.store(0, Ordering::SeqCst);
        let (n, rendered) = match parser_mode {
            ParserMode::Jq => settle!(eval_with_cursor_using::<JqSemantics, _>(&expr, root)),
            _ => settle!(eval_with_cursor_using::<YqSemantics, _>(&expr, root)),
        };
        reached += n;
        answer = answer.or(rendered);
        counted = COUNT.load(Ordering::SeqCst);
        ARMED.store(false, Ordering::SeqCst);
    } else {
        let index = YamlIndex::build(&bytes).expect("build yaml index");
        // A YAML index's root is the *document stream*, so `.[]` on it would
        // yield documents, not the first document's items -- the CLI uncons's
        // the first document cursor before evaluating and so does this.
        let stream = index.root(&bytes);
        let root = match stream.value() {
            YamlValue::Sequence(docs) => {
                docs.uncons_cursor()
                    .expect("the file holds at least one document")
                    .0
            }
            _ => panic!("a YAML index root is always a document sequence"),
        };
        let (n, rendered) = match parser_mode {
            ParserMode::Jq => settle!(eval_with_cursor_using::<JqSemantics, _>(&expr, root)),
            _ => settle!(eval_with_cursor_using::<YqSemantics, _>(&expr, root)),
        };
        reached += n;
        answer = answer.or(rendered);
        ARMED.store(true, Ordering::SeqCst);
        COUNT.store(0, Ordering::SeqCst);
        let (n, rendered) = match parser_mode {
            ParserMode::Jq => settle!(eval_with_cursor_using::<JqSemantics, _>(&expr, root)),
            _ => settle!(eval_with_cursor_using::<YqSemantics, _>(&expr, root)),
        };
        reached += n;
        answer = answer.or(rendered);
        counted = COUNT.load(Ordering::SeqCst);
        ARMED.store(false, Ordering::SeqCst);
    }

    println!(
        "allocs={counted} reached={reached} answer={}",
        answer.as_deref().unwrap_or("-")
    );
}
