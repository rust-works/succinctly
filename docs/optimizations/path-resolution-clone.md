# Path-Resolution Per-Step Cloning (#2058)

[Home](../../) > [Docs](../) > [Optimizations](./) > Path-resolution per-step cloning

Investigation of [#2058](https://github.com/rust-works/succinctly/issues/2058), which
[del-path-trie.md](del-path-trie.md#2058) flagged as the term still binding
`jq_write_path_del_shared_prefix_depth`'s exponent to ~1.6 even after #1690 removed a
genuine per-*branch* O(d²) from the same code region: holding the branch count fixed at
two and scaling only depth `D` still read an exponent of ~1.8-1.95 for `path()`, `=` and
`del()` alike.

**Outcome: two independent O(d²) clone-per-step sites, in two different evaluators, both
triggered by the same two-branch static-chain shape (`.c…c[0], .c…c[1]`) — fixing both drops
every measured exponent from ~1.9 to ~1.0 on both `nodes.yaml` targets.**

## Site 1 — `value_after_components` (`src/jq/eval.rs`)

`resolve_seq`'s no-computed-key fast path (reached by `path()`/`=`/`del()` alike once a
top-level `Comma` routes the whole expression through `resolve_node`) threads a document
value through a run of static `Field`/`Index` components via `value_after_components`. Its
loop cloned the *entire remaining value* at every step:

```rust
let mut current = value.clone();
for component in components {
    let mut values = eval_owned_multi::<S>(component, &current)?;
    // ...
    current = values.pop().expect("len checked");
}
```

`eval_owned_multi` → `eval_owned_fast_path`'s `Field`/`Index` arms already avoid the
expensive JSON-serialize-and-reparse bridge for these two shapes, but still read
`map.get(name).cloned()` / `items.get(i).cloned()` — a **deep** clone of the matched
child's whole subtree. Step `i` of a `d`-deep chain holds a subtree of size O(d−i), so the
loop's per-step clones sum to O(d²).

### The fix

`value_after_components` now takes `value: OwnedValue` by move instead of `&OwnedValue`,
and a new `navigate_static_component` does the per-step navigation *destructively*:
`IndexMap::swap_remove`/`Vec::swap_remove` pull the one matched child out of its parent by
move, letting the untouched siblings drop instead of being cloned. `swap_remove` (not the
order-preserving `shift_remove`/`Vec::remove`) is what makes this O(1) rather than O(n): the
parent container is never read again once this function has navigated past it, so its
remaining order doesn't matter — it is dropped, in whatever order swap-removal leaves it, the
moment the caller moves on to the next component. `resolve_static_tail` (the sole caller)
pays exactly one `.clone()` at the boundary — unavoidable, since it never owns the document
it's handed — and everything after that is a move.

`Expr::Slice` and `Expr::Optional`-wrapped components fall back to the pre-existing
`eval_owned_multi` clone-and-reevaluate path unchanged: neither was ever on
`eval_owned_fast_path`'s fast path to begin with (both already pay a full
`to_json_for_reindex` + reparse round trip per step, a separate, pre-existing cost this fix
does not touch), so leaving them alone is a no-op, not a missed case.

## Site 2 — `path()`'s own value and path-array cloning (two evaluators)

Even with site 1 fixed, `path()` alone kept a growing exponent (~1.5 at D=30→60, ~1.7 at
D=120→240) that `=`/`del()` did not. Profiling and direct measurement traced this to a
*second* mechanism, present in **both** of this crate's two evaluators, each reached by a
different caller:

- `src/jq/eval.rs`'s `walk_path`/`walk_pipe`/`step_into` (reached by `builtin_path_on_owned`,
  itself reached when `eval_generic.rs`'s own cursor-native fast path below can't take a
  shape).
- `src/jq/eval_generic.rs`'s `path_walk_generic`/`path_step_generic` (#2061's cursor-walking
  fast path — the code the CLI's `succinctly jq`/`succinctly yq` actually dispatch `path()`
  to for the overwhelmingly common cursor-navigable shapes, including this issue's own
  benchmark).

Both built up the *reported* path (`["c","c","c",...,0]`) with the same anti-pattern:

```rust
let mut p = path.to_vec();  // clone the whole trail so far
p.push(component);
out.push((p, next));
```

Step `i` of a `d`-deep chain clones a trail of length `i`, so this sums to O(d²) on its own —
independent of, and in `eval_generic.rs`'s case *compounding with*, a second clone:
`path_walk_generic`'s `Expr::Pipe` arm split one component off the flat chain at a time and
rewrapped the *remaining* components in a fresh, owned `Expr::Pipe(rest.to_vec())` just to
recurse, an O(remaining length) AST clone at every one of the same `d` stages — the identical
shape #1510 already fixed in `eval.rs`'s own path-context evaluator, but not (until now) in
`eval_generic.rs`'s independent copy.

### The fix

A new `PathTrail` (`src/jq/eval.rs`, `pub(crate)`) is `PathPrefix`'s twin for `path()`'s own
*value*-typed components: an `Rc`-linked cons-list with O(1) `extend` and one O(depth)
`to_vec`, paid exactly once per branch when it becomes an actual `path()` output — not once
per step. Both evaluators' `current_path`/`path` parameters now carry `Rc<PathTrail>` instead
of `Vec<OwnedValue>`/`&[OwnedValue]`, and `eval_generic.rs`'s `Expr::Pipe` handling
(`path_walk_generic`'s top-level arm, and `path_step_generic`'s own copy for a pipe reached
as a single nested step) was split into slice-based helpers (`path_walk_pipe_generic`,
`path_step_pipe_generic`) that recurse on a borrowed `&[Expr]` directly, removing the
`rest.to_vec()` AST clone entirely rather than merely shrinking it.

One unrelated caller, `eval_generic.rs`'s `path_context_step_generic` (the `key`/`parent`/
`file_index` path-context machinery, a different feature from `path()`/`=`/`del()`), still
carries its own path as a plain `Vec<OwnedValue>` — out of scope here. `PathTrail::from_slice`
bridges it into the shared step helpers at the same O(depth) cost it already paid per call,
so that feature is unchanged, not regressed.

## Results

Interleaved CLI A/B (`succinctly jq`, 200-document stream per invocation, 5 repetitions,
alternating before/after order per rep to cancel thermal drift — see
[the benchmarking guide](../guides/benchmarking.md#ab-benchmarking-method)), both idle
`nodes.yaml` targets, fixed two-branch shape (`.c…c[0], .c…c[1]`) over a document with an
8-element leaf array, scaling only `D`:

**Apple M4 Pro (ARM):**

| query    | D=30            | D=60            | D=120           | D=240            | exp (120→240) |
|----------|-----------------|-----------------|-----------------|------------------|---------------|
| `path()` | 0.083→0.038ms   | 0.170→0.045ms   | 0.562→0.078ms   | 2.090→0.150ms    | 1.89 → 0.95   |
| `=`      | 0.123→0.051ms   | 0.355→0.084ms   | 1.274→0.157ms   | 5.023→0.303ms    | 1.98 → 0.95   |
| `del()`  | 0.129→0.056ms   | 0.366→0.094ms   | 1.285→0.177ms   | 5.052→0.347ms    | 1.98 → 0.97   |

**AMD Ryzen 9 7950X (x86_64):**

| query    | D=30            | D=60            | D=120           | D=240            | exp (120→240) |
|----------|-----------------|-----------------|-----------------|------------------|---------------|
| `path()` | 0.089→0.026ms   | 0.281→0.051ms   | 0.996→0.094ms   | 3.716→0.187ms    | 1.90 → 1.00   |
| `=`      | 0.119→0.051ms   | 0.364→0.095ms   | 1.257→0.192ms   | 4.650→0.388ms    | 1.89 → 1.01   |
| `del()`  | 0.129→0.062ms   | 0.383→0.114ms   | 1.292→0.222ms   | 4.752→0.448ms    | 1.88 → 1.02   |

Speedups at D=240 range 5.8x (`del()`, x86) to 19.9x (`path()`, x86). Every exponent over the
last depth-doubling drops from ~1.9-2.0 to ~1.0 on both architectures, for all three query
shapes — the acceptance criteria's "drops toward ~1", read literally.

`jq_write_path_two_branch_{path,assign,del}_depth` (`benches/jq_write_path_bench.rs`) is the
checked-in regression fixture for this shape: a fixed two-branch comma over a
`D`-scaled `{"c": ...}` chain terminating in a *fixed*-size 8-element leaf array (unlike
`jq_write_path_del_shared_prefix_depth`, whose leaf size scales *with* `D` and so cannot
isolate a per-step term from a per-branch one).

## Lessons

- **A profile's own function names are the ground truth for *which* evaluator to look in,
  not an assumption from where the fix "should" live.** The issue's own profile named
  `walk_pipe`/`step_into` — `eval.rs`'s functions — but the CLI's actual `path()` dispatch for
  this shape goes through `eval_generic.rs`'s independent `path_walk_generic` instead (#2061).
  A Criterion benchmark that calls `eval.rs::eval()` directly (as this crate's own
  `jq_write_path_bench.rs` does, for good reason — it isolates the evaluator from I/O) is a
  real, valid measurement of *that* evaluator, but is not automatically a proxy for what the
  CLI itself does; the two can and did diverge for this exact query shape.
- **Two evaluators, two independent copies of the same bug.** This crate maintains
  `eval.rs` and `eval_generic.rs` as separate jq/yq evaluators for unrelated reasons (see
  `docs/adrs/`), and a fix in one does not reach the other — #2048's own CHANGELOG entry
  already flagged this once. `path()`'s clone-per-step bug existed in both, independently,
  and `eval_generic.rs`'s copy additionally compounded it with an AST-clone-per-step the
  `eval.rs` side never had (because `eval.rs`'s `walk_pipe` already threads a borrowed
  `&[Expr]` slice — the #1510 fix — where `eval_generic.rs`'s `path_walk_generic` still
  rebuilt an owned `Expr::Pipe` at each stage).
- **A CLI-level, multi-document-stream measurement is what actually settles a CLI-reachable
  claim.** A single-process-per-query timing (spawn overhead ~4-6ms) cannot see a
  per-document cost an order of magnitude smaller; the issue's own "200 documents per
  process" method, reproduced here, is what separates the two.
- **Re-verify the discriminating experiment after every partial fix, not just once at the
  end.** Fixing site 1 alone flattened `=`/`del()`'s exponents immediately, but left `path()`'s
  still growing with depth — the same "one exponent came down, a second, independent term is
  still under it" shape #1651 and #1690 (`del-root-shortcircuit.md`, `del-path-trie.md`) had
  already each hit once in this exact code region.

## See also

- [del-path-trie.md](del-path-trie.md) — #1690, the predecessor this issue's own acceptance
  benchmark depended on
- [del-root-shortcircuit.md](del-root-shortcircuit.md) — #1651, an earlier link in the same
  chain
- [`src/jq/eval.rs`](../../src/jq/eval.rs) — `value_after_components`,
  `navigate_static_component`, `walk_path`, `walk_pipe`, `step_into`, `PathTrail`,
  `PathPrefix`
- [`src/jq/eval_generic.rs`](../../src/jq/eval_generic.rs) — `path_walk_generic`,
  `path_walk_pipe_generic`, `path_step_generic`, `path_step_pipe_generic`
- [docs/guides/benchmarking.md](../guides/benchmarking.md#ab-benchmarking-method) — the A/B
  method used above
