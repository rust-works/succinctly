//! Allocation probe for #3692: allocator calls of one *streaming* evaluation.
//!
//! The CLI's jq path drives `eval_each_with_cursor`, not the collecting
//! `eval_with_cursor_using` that `alloc_probe_3022` uses, and the owned pipe
//! drivers this issue is about are only reached through the former. Parsing,
//! indexing and one warm-up evaluation happen outside the counted window.
//!
//!     cargo run --example alloc_probe_3692 -- <file.json> <query>

#![allow(unsafe_code)]

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use succinctly::jq::eval_generic::{eval_each_with_cursor_using, GenericResult};
use succinctly::jq::{parse_with_mode_and_extensions, JqSemantics, ParserMode};
use succinctly::json::JsonIndex;

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

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [path, query] = <[String; 2]>::try_from(args)
        .unwrap_or_else(|_| panic!("usage: alloc_probe_3692 <file.json> <query>"));
    let bytes = std::fs::read(&path).expect("read fixture");
    let expr = parse_with_mode_and_extensions(&query, ParserMode::Jq, true).expect("parse query");
    let index = JsonIndex::build(&bytes);
    let root = index.root(&bytes);

    let run = || {
        let mut outputs = 0usize;
        let control = eval_each_with_cursor_using::<JqSemantics, _>(&expr, root, &mut |result| {
            if let GenericResult::Error(e) = &result {
                panic!("query failed: {e:?}");
            }
            outputs += 1;
            true
        });
        assert!(control.is_none(), "query ended in a control");
        outputs
    };

    let warm = run();
    ARMED.store(true, Ordering::SeqCst);
    COUNT.store(0, Ordering::SeqCst);
    let outputs = run();
    let counted = COUNT.load(Ordering::SeqCst);
    ARMED.store(false, Ordering::SeqCst);
    assert_eq!(warm, outputs);
    println!("allocs={counted} outputs={outputs}");
}
