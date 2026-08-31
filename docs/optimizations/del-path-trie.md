# `del()` Path Trie (#1690)

[Home](../../) > [Docs](../) > [Optimizations](./) > `del()` path trie

Investigation of [#1690](https://github.com/rust-works/succinctly/issues/1690), the follow-up
to [#1651](https://github.com/rust-works/succinctly/issues/1651)
([del-root-shortcircuit.md](del-root-shortcircuit.md)). #1651 removed two O(depth)-per-branch
flattening passes for the one case where a resolved branch *is* the document root. A
**filtered** recursive descent — `del(.. | select(cond))` where `cond` rejects `.` — never
produces that branch, so it kept paying both in full.

**Outcome: the flatten really was O(d²) and merging the resolved paths into a hash-consed
trie removes it — but it is not the term that dominates the shape the issue names. That shape
is bounded by a *second*, independent O(d²): `select` re-serializing each resolved branch's
value through `to_json_for_reindex_at_depth`, tracked separately.**

## What was quadratic

For a multi-path `del()`, three passes each cost O(depth) **per resolved branch**:

1. `resolve_dynamic_indexes`'s `assemble` — `PathPrefix::to_vec()`, the persistent chain's one
   documented O(depth) operation, once per branch.
2. `builtin_del`'s `flatten_delete_path`, again per branch, over the `Vec<Expr>` `assemble`
   just built.
3. `delete_expr_paths_at`'s own sibling grouping, which compared the flat paths against each
   other one position at a time — a path of length `L` participates in `L` levels, so the walk
   itself summed to O(total path length) too.

With `b` branches at depth `d` that is O(b·d), and a match set that grows with depth makes
`b ≈ d`.

## The fix

Sibling branches under a shared ancestor already share the same `Rc<PathPrefix>` allocation
for that ancestor — `push_recursive_branches` clones the *same* parent `Rc` into every child
(#701), so a `d+1`-branch traversal creates exactly `d` distinct `PathPrefix::Node`s, not
O(d²). `DeleteTrie` walks each branch's chain leaf-to-root, memoizing already-interned nodes
by pointer identity, so a branch only does new work for the part of its chain no earlier
branch already walked. The apply walk then visits each *distinct prefix* once instead of once
per path running through it.

All three passes above become O(d) amortized, and the four functions implementing pass 3
(`delete_expr_paths_at`, `delete_expr_paths_through_absent`, `delete_expr_object_paths`,
`delete_expr_array_paths`) are replaced by three structural mirrors reading from the trie
rather than left standing as a second implementation of the same rules.

Two details are load-bearing and easy to get wrong:

- **Group order is not key order.** The old walkers kept recursion groups (`groups`) separate
  from terminal keys (`terminal`), so a key first seen as a terminal path and only later as a
  prefix is ordered by that *later* appearance. A trie node has one child map, so this needs
  an explicit `field_groups`/`index_groups` list. Ordering the recursion by the child map
  instead silently rewrites which error jq reports: `del(.a, .b.x, .a.y)` on `{"a":5,"b":"s"}`
  is `Cannot index string with string "x"` in jq 1.7.1, and would become `Cannot index number
  with string "y"`.
- **A terminal node is not consulted on the way in.** `del(.a, .a[0])` on `{"a":"s"}` reaches
  `.a` as both a doomed key and a prefix, and jq still walks *into* it and raises. Only a
  node's *parent* reads its `terminal` flag, when batching terminal children into one
  `delete_keys` call.

## Measurement

Depth is capped by `MAX_NESTING_DEPTH = 256`. Two shapes, A/B interleaved within each
repetition, process-spawn floor subtracted, output identity gated first (Apple M5 Pro Max, on
AC power).

**Depth-scaled** — a `D`-deep chain ending in a `D`-element array, `del(.c…c[(0,…,D-1)])`, so
both the shared prefix and the fan-out scale with `D`:

| D   | trie   | pre-#1690 | ratio | trie exp | pre exp |
|-----|--------|-----------|-------|----------|---------|
| 30  | 0.67ms | 0.64ms    | 0.96x |          |         |
| 60  | 1.23ms | 1.59ms    | 1.30x | 0.88     | 1.32    |
| 120 | 2.13ms | 3.26ms    | 1.53x | 0.80     | 1.04    |
| 240 | 6.36ms | 12.01ms   | 1.89x | 1.58     | 1.88    |

**Width-scaled** — the isolating fixture. Depth pinned at 240 and only the branch count `K`
varies, so anything growing with `K` is charged *per branch*:

| K    | trie    | pre-#1690 | ratio |
|------|---------|-----------|-------|
| 250  | 11.1ms  | 17.4ms    | 1.57x |
| 500  | 11.7ms  | 24.7ms    | 2.11x |
| 1000 | 14.7ms  | 41.4ms    | 2.81x |
| 2000 | 21.1ms  | 82.0ms    | 3.88x |
| 4000 | 33.1ms  | 158.1ms   | 4.77x |

Per-branch marginal cost falls from ~37.5µs to ~5.9µs — a 6.4x reduction — and the ratio is
still climbing at K=4000.

Both are checked in as `jq_write_path_del_filtered_descent_depth` and
`jq_write_path_del_shared_prefix_width` in
[`benches/jq_write_path_bench.rs`](../../benches/jq_write_path_bench.rs).

## The term this does *not* fix

The issue's own headline shape, `del(.. | select(type == "number"))`, measured **no
improvement at all** from this change — exponent ~1.8 before and after. That is not the fix
failing; it is a second, larger O(d²) sitting on top of it.

`resolve_node` evaluates `select(...)` against each resolved branch's value via
`eval_owned_multi_keep_partial`/`eval_owned_input`, which serializes that value back to JSON
text and reparses it (`OwnedValue::to_json_for_reindex_at_depth`). Branch `i` of a depth-`d`
chain holds a subtree of size O(d−i), so the serialization alone sums to O(d²). A `sample`
profile of `del(.. | select(type == "number"))` at depth 240 put `builtin_del` at 73% of
process time with `to_json_for_reindex_at_depth` the dominant leaf; `builtin_del`'s own trie
work (`insert_branch` + `delete_trie_apply`) was ~16% of that.

This is why the width-scaled table above is the one that demonstrates #1690: it reaches the
same shared-deep-prefix delete through a computed key instead of a filter, so no `select` runs
per branch and the path-flatten term is left on its own.

## Lessons

- **Two O(d²) terms on one path both fit "hold the count fixed, vary the depth."** #1690's
  premise was verified that way (resolved-path count held within 1.2% while time doubled per
  depth doubling) and the conclusion — "the cost is O(depth) per resolved branch" — was
  correct. It just does not distinguish *which* per-branch O(depth) cost, and there were two.
  The discriminating experiment is the one that holds the *suspected mechanism* out: a shape
  with the same branch count and depth but no `select`.
- **A fix landing with no measurable win is not automatically wrong.** Reverting on the
  headline benchmark alone would have removed a real O(d²) that becomes the binding constraint
  the moment the reindex term is fixed. Build the shape that isolates your own term before
  believing either verdict.
- **Same lesson as #1651, one level down.** That write-up's own closing note — "the
  benchmark's exponent staying flat meant a *second*, independent O(d²) was hiding behind it"
  — applied again, to its own successor.

## See also

- [del-root-shortcircuit.md](del-root-shortcircuit.md) — #1651, the predecessor: same code
  region, same "the issue named the wrong culprit" shape
- [select-scan.md](select-scan.md) — the original precedent for both
- [`src/jq/eval.rs`](../../src/jq/eval.rs) — `DeleteTrie`, `DeleteTrieBuilder`,
  `resolve_del_path_branches`, `builtin_del`, `PathPrefix`
- [docs/guides/benchmarking.md](../guides/benchmarking.md#ab-benchmarking-method) — the A/B
  method used above
