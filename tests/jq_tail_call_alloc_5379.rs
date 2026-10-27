//! A tail-recursive `def` holds no memory per round (#5379).
//!
//! jq runs `def f: if . < 300000 then . + 1 | f else . end; 0 | f` in constant
//! space (it compiles the call to a jump), and so must the evaluators here once
//! they loop in place: a loop that kept what a round bound -- the body it
//! installed, the retention scope of the call, a frame -- would hold it for as
//! many rounds as the recursion is long.
//!
//! The assertion is on that proportion: the live-heap high-water mark of the
//! same program at ten times the depth, in the same process, against a bound
//! that no per-round allocation of even a few words could meet (a round retained
//! 16 bytes would add 7 MB at the deep end). Counting is gated to the calling
//! thread, so the harness's other threads and the other tests in this file
//! cannot add to the window.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use succinctly::jq::eval_generic::{eval_each_with_cursor_using, GenericResult};
use succinctly::jq::{eval_full, parse, JqSemantics, OwnedValue, QueryResult};
use succinctly::json::JsonIndex;

thread_local! {
    // `(live, peak)` in bytes while this thread is counting, `None` otherwise.
    // Const-initialized with no destructor, so reading it from inside the
    // allocator never allocates.
    static LIVE: Cell<Option<(i64, i64)>> = const { Cell::new(None) };
}

struct Peak;

fn note(delta: i64) {
    // `try_with` so the allocator itself can never panic.
    let _ = LIVE.try_with(|cell| {
        if let Some((live, peak)) = cell.get() {
            let live = live + delta;
            cell.set(Some((live, peak.max(live))));
        }
    });
}

// A counting global allocator is the whole point of this test, and
// `GlobalAlloc` is an unsafe trait, so the unsafe code is opted back in here
// and only here.
#[allow(unsafe_code)] // STYLE-0004: a test-only counting allocator, forwarding to `System` unchanged.
                      // SAFETY: every method forwards its arguments unchanged to `System`, which upholds
                      // `GlobalAlloc`'s contract; the only addition is a counter update that neither
                      // allocates nor touches the pointer.
unsafe impl GlobalAlloc for Peak {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        note(layout.size() as i64);
        // SAFETY: `layout` is the caller's, passed straight to `System`.
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        note(layout.size() as i64);
        // SAFETY: `layout` is the caller's, passed straight to `System`.
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        note(new_size as i64 - layout.size() as i64);
        // SAFETY: `ptr`, `layout` and `new_size` are the caller's, passed straight to
        // `System`, and `ptr` came from `System` through this allocator's `alloc`.
        unsafe { System.realloc(ptr, layout, new_size) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        note(-(layout.size() as i64));
        // SAFETY: `ptr` and `layout` are the caller's, passed straight to `System`, and
        // `ptr` came from `System` through this allocator's `alloc`.
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static ALLOCATOR: Peak = Peak;

/// Closes the window when dropped, so a `run` that panics cannot leave the
/// thread counting through the unwind.
struct Window;

impl Drop for Window {
    fn drop(&mut self) {
        let _ = LIVE.try_with(|cell| cell.set(None));
    }
}

/// The most bytes `run` held live at once on this thread, and what it returned.
/// Memory the window frees that it did not allocate only lowers the figure.
fn peak_live<R>(run: impl FnOnce() -> R) -> (i64, R) {
    let window = Window;
    LIVE.with(|cell| cell.set(Some((0, 0))));
    let result = run();
    let (_, peak) = LIVE
        .with(|cell| cell.replace(None))
        .expect("the window was opened above");
    drop(window);
    (peak, result)
}

/// How far past the shallow run's peak the deep run's may go: what a handful of
/// allocator size classes and a grown `Vec` of the sink's can account for, and
/// a small fraction of one retained word per round.
const SLACK: i64 = 256 * 1024;

/// A program of a given depth.
type Program = fn(u64) -> String;

const SHALLOW: u64 = 30_000;
const DEEP: u64 = 600_000;

/// `query` through the streaming entry (what `succinctly jq` drives): its one
/// integer output, and the peak live heap of one run after a warm-up run.
fn streaming(query: &str) -> (i64, i64) {
    let json = b"null";
    let index = JsonIndex::build(json);
    let expr = parse(query).expect("parse");
    let run = || {
        let mut answered = None;
        let escaped =
            eval_each_with_cursor_using::<JqSemantics, _>(&expr, index.root(json), &mut |result| {
                match result {
                    GenericResult::Owned(OwnedValue::Int(n)) => answered = Some(n),
                    GenericResult::Error(e) => panic!("`{query}` failed: {e:?}"),
                    _ => panic!("`{query}` did not settle to one integer"),
                }
                true
            });
        assert!(escaped.is_none(), "`{query}` ended early: {escaped:?}");
        answered.expect("one output")
    };
    run();
    let (peak, answered) = peak_live(run);
    (answered, peak)
}

/// `query` through `eval_full`, the value evaluator.
fn value_route(query: &str) -> (i64, i64) {
    let json = b"null";
    let index = JsonIndex::build(json);
    let expr = parse(query).expect("parse");
    let run = || match eval_full::<Vec<u64>, JqSemantics>(&expr, index.root(json)) {
        QueryResult::Owned(OwnedValue::Int(n)) => n,
        _ => panic!("`{query}` did not settle to one integer"),
    };
    run();
    let (peak, answered) = peak_live(run);
    (answered, peak)
}

fn assert_flat(route: &str, program: Program, measure: fn(&str) -> (i64, i64)) {
    let (shallow_answer, shallow) = measure(&program(SHALLOW));
    let (deep_answer, deep) = measure(&program(DEEP));
    assert_eq!(shallow_answer as u64, SHALLOW, "{route}: shallow answer");
    assert_eq!(deep_answer as u64, DEEP, "{route}: deep answer");
    assert!(
        shallow > 0,
        "{route}: the shallow run counted no memory, so the counter is not counting"
    );
    assert!(
        deep <= shallow + SLACK,
        "{route}: peak live heap grew with depth: {shallow} bytes at {SHALLOW} levels, \
         {deep} at {DEEP}"
    );
}

#[test]
fn a_tail_loop_holds_no_memory_per_round_5379() {
    let shapes: [(&str, Program); 4] = [
        ("if/pipe", |n| {
            format!("def f: if . < {n} then . + 1 | f else . end; 0 | f")
        }),
        ("through a nested def", |n| {
            format!("def f: def g: f; if . < {n} then . + 1 | g else . end; 0 | f")
        }),
        ("with an as binding", |n| {
            format!("def f: if . < {n} then . as $x | $x + 1 | f else . end; 0 | f")
        }),
        ("through a wrapper", |n| {
            format!("def f: if . < {n} then . + 1 | f else . end; def g: f; 0 | g")
        }),
    ];
    for (shape, program) in shapes {
        assert_flat(&format!("streaming, {shape}"), program, streaming);
        assert_flat(&format!("value route, {shape}"), program, value_route);
    }
}
