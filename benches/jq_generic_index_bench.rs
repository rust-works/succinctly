//! Depth-scaling benchmark for `eval_generic::eval_index_expr`'s computed-key
//! materialization (#680), a precursor to #669.
//!
//! #669 suspects `eval_generic.rs::eval_index_expr` (`src/jq/eval_generic.rs`,
//! the evaluator shared by `jq_runner.rs` and `yq_runner.rs`) shares #626's
//! eager-materialization bug: it evaluates a computed key/index and
//! unconditionally `.collect_owned()`s the *entire* result before ever
//! checking whether the key is a usable string/number vs. an array/object.
//! `index_one_generic`'s Array/Object-key rejection arm, and
//! `EvalError::cannot_index`, only ever call `.type_name()` on such a key —
//! never inspect its contents (confirmed by reading both) — so, exactly like
//! `eval.rs` before PR #670's `to_owned_key_shape`, a full recursive
//! materialization of an Array/Object candidate key is pure waste whenever it
//! can only ever be rejected on type.
//!
//! **Why this does not reuse `jq_recurse_depth_bench`'s `.. | .[.k]?`
//! verbatim.** `eval_generic.rs` has no native arm for `Expr::Recurse` (`..`):
//! it falls to the catch-all at the bottom of `eval_single`'s match, which
//! `to_owned()`s the *entire current value* up front and hands the whole
//! query off to `eval::eval` (`eval.rs`'s full evaluator — already fixed by
//! #670) via a JSON round-trip. Once that first pipe stage returns
//! `ManyOwned`, every following stage — including `.[.k]?` — routes through
//! `eval_on_many_owned`/`eval_on_owned`, which *also* reindexes each value
//! and re-enters `eval::eval` individually. A `.. | .[.k]?` benchmark here
//! would therefore measure `eval.rs`'s already-fixed `eval_index_expr`
//! wrapped in per-node JSON-reindex overhead, not
//! `eval_generic::eval_index_expr` at all — the same "signal swamped by
//! irrelevant overhead" failure #680 was filed to get past, just one layer
//! deeper than the CLI differential that motivated it.
//!
//! **Design used instead.** `Expr::Field` (`.k`), `Expr::Optional` (`?`), and
//! `Expr::IndexExpr` (`.[.k]`) *are* all handled natively — cursor-threaded
//! through `Expr::Pipe`'s `OneCursor` arm with no reindex — so a probe of the
//! form `.k.k...k | .[.k]?` (`i` copies of `.k`) stays entirely on
//! `eval_generic`'s own code path and reaches its `eval_index_expr` directly.
//! Since a single such probe can't be chained into the next one (the `?`
//! collapses the pipe to `GenericResult::None`, and every later stage of a
//! `None`-valued pipe short-circuits without evaluating — see the `Pipe`
//! match arm), each level's probe is instead its own independent expression,
//! evaluated from a fresh root cursor. Running all `depth` of them inside one
//! `b.iter` sample reproduces the same O(depth) count of probes `..`'s
//! traversal of the `k`-chain would have made, each paying O(remaining
//! subtree) under the suspected bug — the same O(depth^2) total signature
//! `#626`/`#670` found, if this turns out to have it too.
//!
//! The document — `{"k": {"k": ... "pad": {"a":{},"b":{},"c":{}}}}` — is
//! #626's own fixture, verbatim (`jq_recurse_depth_bench.rs`), for visual
//! comparability, even though these probes only ever walk the `k`-chain
//! (never descending into `pad`): that chain is the O(depth^2) contributor,
//! not the `pad` leaf. Every probe target is a `{"k": ...}` object indexed by
//! its own `.k` field (an Object), so `.[.k]?` always rejects on type and
//! never produces output — the `?` suppresses the resulting error.
//!
//! This file makes **no timing assertion** — it is a Criterion benchmark for
//! manual before/after comparison, not a CI gate. Run it interleaved
//! before/after a relevant fix lands, per the A/B method in
//! `docs/guides/benchmarking.md#ab-benchmarking-method`, and record the
//! resulting table in #669 rather than here.
//!
//! Run with:
//! ```bash
//! cargo bench --bench jq_generic_index_bench
//! ```

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use std::hint::black_box;
use succinctly::jq::eval_generic::{eval_with_cursor, eval_with_cursor_using};
use succinctly::jq::{parse, Expr, YqSemantics};
use succinctly::json::JsonIndex;
use succinctly::yaml::YamlIndex;

/// `{"k": {"k": ... "pad": {"a":{},"b":{},"c":{}}}}`, `depth` levels of `"k"`
/// nesting — #626's synthetic linear-nesting document, verbatim.
fn linear_nest_with_pad_json(depth: usize) -> Vec<u8> {
    let open = "{\"k\":".repeat(depth);
    let close = "}".repeat(depth);
    format!("{open}{{\"pad\":{{\"a\":{{}},\"b\":{{}},\"c\":{{}}}}}}{close}").into_bytes()
}

/// YAML equivalent of [`linear_nest_with_pad_json`]: `depth` levels of block
/// mapping `k:` nesting, then a `pad:` sibling with three empty flow-mapping
/// children — same shape, same depths, so results are comparable across the
/// JSON/YAML halves of this benchmark.
fn linear_nest_with_pad_yaml(depth: usize) -> Vec<u8> {
    let mut out = String::new();
    for i in 0..depth {
        out.push_str(&"  ".repeat(i));
        out.push_str("k:\n");
    }
    out.push_str(&"  ".repeat(depth));
    out.push_str("pad:\n");
    for key in ["a", "b", "c"] {
        out.push_str(&"  ".repeat(depth + 1));
        out.push_str(key);
        out.push_str(": {}\n");
    }
    out.into_bytes()
}

/// One independent probe per `k`-chain level: `i` copies of `.k` (navigating
/// to the node `depth`-`i` levels from the leaf) piped into `.[.k]?`, which
/// re-evaluates `.k` *at that node* as the computed key. See the module
/// doc for why these can't be chained into a single expression.
fn probe_exprs(depth: usize) -> Vec<Expr> {
    (0..depth)
        .map(|i| {
            let query = if i == 0 {
                ".[.k]?".to_string()
            } else {
                format!("{} | .[.k]?", ".k".repeat(i))
            };
            parse(&query).unwrap_or_else(|e| panic!("level {i} filter must parse: {e}"))
        })
        .collect()
}

fn bench_generic_index_json(c: &mut Criterion) {
    let mut group = c.benchmark_group("jq_generic_index_json");
    // Matches #626/#661's depths so results stay comparable across this
    // benchmark family.
    for &depth in &[100usize, 200, 300, 400] {
        let json = linear_nest_with_pad_json(depth);
        let index = JsonIndex::build(&json);
        let exprs = probe_exprs(depth);

        // Guard the premise: every level's probe must be suppressed to no
        // output. A fixture that errored or produced output would no longer
        // isolate the key-materialization cost this benchmark targets.
        for (i, expr) in exprs.iter().enumerate() {
            let cursor = index.root(&json);
            let probe = eval_with_cursor(expr, cursor);
            assert!(
                !probe.is_error(),
                "depth {depth} level {i} fixture must not error"
            );
            assert!(
                probe.collect_owned().expect("materializes").is_empty(),
                "depth {depth} level {i} fixture must produce no output"
            );
        }

        group.throughput(Throughput::Elements(depth as u64));
        group.bench_with_input(BenchmarkId::from_parameter(depth), &json, |b, json| {
            b.iter(|| {
                let mut total = 0usize;
                for expr in &exprs {
                    let cursor = index.root(black_box(json));
                    let result = eval_with_cursor(expr, cursor);
                    total += result.collect_owned().expect("materializes").len();
                }
                black_box(total)
            });
        });
    }
    group.finish();
}

fn bench_generic_index_yaml(c: &mut Criterion) {
    let mut group = c.benchmark_group("jq_generic_index_yaml");
    for &depth in &[100usize, 200, 300, 400] {
        let yaml = linear_nest_with_pad_yaml(depth);
        let index = YamlIndex::build(&yaml).expect("benchmark fixture must parse");
        let exprs = probe_exprs(depth);

        // `YamlIndex::root` always reports a virtual sequence of documents
        // (multi-doc-stream support), not the document mapping itself —
        // every real call site (`yq_runner.rs`) unwraps one level via
        // `first_child()` before evaluating. Skipping that here would
        // evaluate `.k` against the virtual array wrapper instead of the
        // `{"k": ...}` document, erroring immediately at level 0 regardless
        // of depth.
        for (i, expr) in exprs.iter().enumerate() {
            let cursor = index
                .root(&yaml)
                .first_child()
                .expect("fixture has exactly one document");
            // yq semantics, matching `yq_runner.rs`'s own call — the bug
            // under test is in `eval_index_expr` itself, not in arithmetic
            // fallback, but this keeps the probe faithful to the real
            // production call site.
            let probe = eval_with_cursor_using::<YqSemantics, _>(expr, cursor);
            assert!(
                !probe.is_error(),
                "depth {depth} level {i} fixture must not error"
            );
            assert!(
                probe.collect_owned().expect("materializes").is_empty(),
                "depth {depth} level {i} fixture must produce no output"
            );
        }

        group.throughput(Throughput::Elements(depth as u64));
        group.bench_with_input(BenchmarkId::from_parameter(depth), &yaml, |b, yaml| {
            b.iter(|| {
                let mut total = 0usize;
                for expr in &exprs {
                    let cursor = index
                        .root(black_box(yaml))
                        .first_child()
                        .expect("fixture has exactly one document");
                    let result = eval_with_cursor_using::<YqSemantics, _>(expr, cursor);
                    total += result.collect_owned().expect("materializes").len();
                }
                black_box(total)
            });
        });
    }
    group.finish();
}

criterion_group!(benches, bench_generic_index_json, bench_generic_index_yaml);
criterion_main!(benches);
