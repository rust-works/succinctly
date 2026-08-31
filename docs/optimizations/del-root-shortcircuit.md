# `del()` Root-Branch Short-Circuit (#1651)

[Home](../../) > [Docs](../) > [Optimizations](./) > `del()` root short-circuit

Investigation of [#1651](https://github.com/rust-works/succinctly/issues/1651): #701/PR #1631
made `push_recursive_branches`'s path-construction O(1) per node (an `Rc<PathPrefix>`
persistent cons-list replacing a per-node `Vec<Expr>` clone), but `del(..)` on a depth-`d`
document measured unchanged growth — k≈1.94 before *and* after that fix.

**Outcome: the O(d²) was never in `delete_at_path`'s apply walk, as the issue's own title
hypothesized — it was in two redundant flattening passes upstream that ran unconditionally
before delete's own short-circuit ever got a chance to fire.**

## The corrected root cause

For `del(..)` on a linear-nesting document, `delete_expr_paths_at` (`src/jq/eval.rs`) already
short-circuits to `Ok(Null)` on its very first call — `paths.iter().any(|path| path.len() ==
start)` is true immediately, because the root branch's path has length 0. `delete_at_path`'s
own `Expr::Identity` arm does the same thing for the single-path case. Neither function ever
actually walks the tree for this query shape.

The real O(d²) sat in two passes that ran *before* either of those checks:

1. `resolve_dynamic_indexes`'s `assemble()` closure — shared infrastructure for
   `path()`/`del()`/`=`/`|=` — calls `PathPrefix::to_vec()`, the persistent chain's one
   documented O(depth) operation, **once per resolved branch**. `del(..)` resolves `d+1`
   branches (one per visited node) with path lengths `0..=d`, summing to O(d²).
2. `builtin_del`'s multi-path branch then calls `flatten_delete_path` **again**, per branch,
   on the `Vec<Expr>` `assemble()` just built — a second, independent O(d²) pass.

Both ran to completion, and both outputs were discarded, before `delete_expr_paths_at` was ever
invoked.

## Why `..`/`recurse` always hit this

`push_recursive_branches` emits the branch for the current node *before* recursing into its
children — the root branch is always `out[0]`. `recurse`'s own definition emits `.` regardless
of what the recursion function does. So every unconditional recursive-descent construct
(`..`, bare `recurse`, `recurse(f)`, `recurse(f;cond)`) resolves the document root as one of
its branches, unconditionally — and `delete_expr_paths_at`'s existing rule (an exhausted path
subsumes every sibling, collapsing the whole subtree to `null`) already covers that case. The
fix is checking for it *before* paying either flatten, not a new semantic.

## The fix

`resolve_dynamic_indexes` gained a `short_circuit_del_root: bool` parameter, set to `true` only
by `del()`'s own call site (`path()`, `=`, `|=` pass `false` — they need every resolved branch
regardless of whether one is the root). When true, and any resolved branch's
`PathPrefix::depth() == 0` — an O(1) check, since `depth` is a cached field on the persistent
chain — it returns `[Expr::Identity]` immediately, skipping both flattens entirely:

```rust
Ok(branches) => {
    if short_circuit_del_root && branches.iter().any(|b| b.path.depth() == 0) {
        return Ok(vec![Expr::Identity]);
    }
    Ok(assemble(branches))
}
```

> **Since [#1690](https://github.com/rust-works/succinctly/issues/1690):** the flag is gone.
> `del()` no longer calls `resolve_dynamic_indexes` at all — it resolves through
> `resolve_del_path_branches`, which returns the branches themselves and reports this same
> root case as `DelPaths::Root`. The check and its reasoning are unchanged; only the spelling
> is. See [del-path-trie.md](del-path-trie.md) for what #1690 did to the *rest* of this
> write-up's "two redundant flattening passes."

Collapsing to `[Expr::Identity]` reuses a path `del(.)` already exercised before this change
(plain `del(.)` hits `resolve_dynamic_indexes`'s `!needs_path_prepass` early return and produces
exactly that), so no new code path was created.

## Measured result

A/B benchmark (`benches/jq_recurse_clone_bench.rs`'s `del(..)` on a linear-nesting document,
Apple M4 Pro, dedicated idle bench box, interleaved per
[the project's A/B method](../guides/benchmarking.md#ab-benchmarking-method), 3 rounds,
`before` = `2467ca6c8` — the pre-#701 merge-base the issue's own original measurement used —
`after` = this fix):

| depth | before (avg of 3) | after (avg of 3) | speedup |
|------:|-------------------:|-------------------:|--------:|
| 100   | 318.9 µs            | 15.9 µs             | **20.0x**  |
| 200   | 1225.4 µs           | 34.3 µs             | **35.7x**  |
| 300   | 2751.8 µs           | 55.8 µs             | **49.3x**  |

Fitting time ∝ depth^k across the three points: **k ≈ 1.96 before, k ≈ 1.14 after** — quadratic
to near-linear. The speedup itself grows with depth (20x → 36x → 49x) rather than staying flat,
which is the signature of an algorithmic term being removed rather than a constant-factor win.
`before` here predates both #701 and this fix, but the issue's own prior measurement already
established #701 alone left k≈1.94 unchanged — so this exponent drop is attributable to this
fix specifically, not to #701's earlier, separate change.

Output correctness: `del(..)` still collapses to `null` at every depth for both binaries (the
benchmark's own built-in assertion), and the full test suite — including the golden fixtures —
passes unchanged.

## Deliberate scope boundary

This fix covers exactly the shape where a resolved branch *is* the document root. It does
**not** cover a filtered recursive descent whose match set excludes the root —
`del(.. | select(cond))` where `cond` rejects `.` but matches scattered descendants (a "broom"
shape: a shared deep prefix ending in a wide fan-out of matching leaf siblings). That shape
never finds a `depth() == 0` branch, so both flattens still run in full. It's arguably the more
common real-world case — most practical uses of `del(.. | select(...))` want to delete a
*subset* of nodes, not the whole document — and is tracked as a separate, larger follow-up (a
hash-consed deletion trie built from `Rc<PathPrefix>` pointer identity, giving true amortized
O(d) for that shape too), matching this file's own established pattern of landing one verified
O(d²) fix at a time (`push_recursive_branches`'s value clone → #668, its path clone → #701,
`del()`'s flatten-before-apply cost → this fix).

## Lessons

- **A benchmark's own short-circuit can hide where the cost actually lives.** `del(..)`'s result
  was always `null`, computed via a pre-existing O(1) collapse — but the code paths *leading up
  to* that collapse still paid full freight. Profiling (or, here, reading the call graph against
  what the benchmark's own correctness assertion guaranteed about its output) is what separated
  "this function looks like it should be the bottleneck" from where the cost actually was.
- **A prior fix's own unchanged exponent is a strong signal to keep looking, not to distrust the
  fix.** #701's O(1)-per-node path construction was real and correct; the benchmark's exponent
  staying flat meant a *second*, independent O(d²) term was hiding behind it, not that #701
  didn't work.
- **Growth-exponent, not one ratio.** A single before/after ratio at one depth would have shown
  "faster" either way; only the scaling curve across three depths (and the speedup's own growth
  with depth) distinguishes an algorithmic fix from a constant-factor one.

## See also

- [select-scan.md](select-scan.md) — the precedent this write-up is modeled on: a proposed
  optimization that turned out to be measuring a different, quadratic defect entirely
- [del-path-trie.md](del-path-trie.md) — #1690, the follow-up that removed the two flattening
  passes this write-up only *skipped* for one shape
- [`src/jq/eval.rs`](../../src/jq/eval.rs) — `resolve_del_path_branches`, `builtin_del`,
  `PathPrefix`, `DeleteTrie`
- [`benches/jq_recurse_clone_bench.rs`](../../benches/jq_recurse_clone_bench.rs) — the
  benchmark this fix targets, and its own doc comment for the full `path(..)`-is-a-poor-isolator
  reasoning
- [docs/guides/benchmarking.md](../guides/benchmarking.md#ab-benchmarking-method) — the A/B
  method used for the measurement above
