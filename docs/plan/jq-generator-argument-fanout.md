# Generator-argument fan-out for jq builtins (#1279, #1277)

[Home](../../) > [Docs](../) > [Plan](./) > Generator-argument fan-out

**Status: all stages merged.** This document is the
deliverable for [#1279](https://github.com/rust-works/succinctly/issues/1279), whose tier
review (2026-08-20) classified it Tier 3 — "changes evaluation *shape* rather than fixing
a local mistake … wants a design doc first". It also scopes
[#1277](https://github.com/rust-works/succinctly/issues/1277), which the same review asked
to be read alongside it because "there is a real chance one design pass covering the
generator-argument model resolves more than one of them". It does: see
[One design, both issues](#one-design-both-issues).

| Stage | What                                                              | Status |
|-------|-------------------------------------------------------------------|--------|
| 0     | Oracle capture: 48 pinned goldens + 4 must-not-regress guards      | ✅ merged |
| 1     | `stream_outputs` + `ArgFanout` + `fanout_arg`; `contains` + `has` | ✅ merged |
| 2     | Group A, `finish_owned` family (13 remaining); `finish_owned` deleted | ✅ merged |
| 3     | Group A, bespoke-body family (`indices`/`index`/`rindex`/`test`/`bsearch`) | ✅ merged |
| 3b    | ~~`pow`/`atan2`~~ — moved into Stage 8; both arguments are generators (exponent outer) so they need `fanout_two_args` | moved |
| 4     | `resolve_node`'s `GetPath` arm (path context)                     | ✅ merged |
| 5     | `limit`/`nth`, value **and** path contexts                        | ✅ merged |
| 6     | `range` (1-, 2- and 3-arg)                                        | ✅ merged |
| 7     | `setpath`, `delpaths`, `combinations`; `fanout_two_args`          | ✅ merged |
| 8     | Regex pattern × flags; `pow`/`atan2`                              | ✅ merged |
| 9     | `sub`/`gsub` multi-output replacement (closes #840)               | ✅ merged |

This is the **opposite** bug to [#820](https://github.com/rust-works/succinctly/issues/820)
and its design doc, [jq-lazy-generator-consumers.md](jq-lazy-generator-consumers.md): that
one is about consumers being too *eager* (pulling outputs a real jq would never ask for);
this one is about builtins being too *lazy* (never asking for outputs a real jq does use).
Read that document's ["The `ltrimstr` trap"](jq-lazy-generator-consumers.md) section before
this one — it is the reason an ordinary builtin's argument may never be given a stopping
sink, and it names this issue directly.

## Problem

Real jq desugars a generator-argument builtin call `f(x)` as roughly `x as $b | body`, and
jq's `as` runs its body **once per output** of `x`. succinctly evaluates the argument once
with `eval_single` and then collapses the stream to its first value — through
`result_to_owned` / `result_to_owned_full`, or through a bespoke `match` whose catch-all
swallows `Many`/`ManyOwned`.

```
$ echo 1 | jq -c '[contains((1,2))]'
[true,false]
$ echo 1 | succinctly jq -c '[contains((1,2))]'
[true]
```

### Three failure modes, not one

Truncation is the mildest of them. The Stage 0 capture surfaced two worse ones.

**Silent data corruption.** `builtin_setpath`'s value argument falls through a
`_ => OwnedValue::Null` catch-all, so a multi-output value is *written as null*:

```
$ echo '{}' | jq -c '[setpath(["a"]; (1,2))]'
[{"a":1},{"a":2}]
$ echo '{}' | succinctly jq -c '[setpath(["a"]; (1,2))]'
[{"a":null}]
```

**Silent data loss.** `builtin_bsearch`'s catch-all is `_ => QueryResult::None`, so a
multi-output target produces nothing at all:

```
$ echo '[1,2,3]' | jq -c '[bsearch((2,3))]'
[1,2]
$ echo '[1,2,3]' | succinctly jq -c '[bsearch((2,3))]'
[]
```

Neither is reachable by grepping for `result_to_owned`. `pow`/`atan2` are a third such
site, reached through `get_number_from_result` instead.

### One design, both issues

#1277 is the same wound from the other side: ~21 `result_to_owned` callers that drop a
`Partial`'s trailing `Control`. Its own text says the remaining sites "need real, separate
design work per-cluster, not a mechanical extension" of #1164's fix.

That is true of the *single-value* framing and false of the fan-out framing. **A builtin
that loops over every argument value and fires the trailing control after the loop cannot
structurally drop it** — there is no "first value" to keep and no remainder to discard. So
#1277's cluster 1 (`indices`/`index`/`rindex`, whose obstacle was "several scattered
internal `return` points") is closed by the same body-extraction that fan-out needs anyway,
and its cluster 3 (`limit`/`nth`) stops needing a merge design because `n`'s own trailing
control and the `expr` argument's bespoke `Flow` reconciliation end up at different nesting
levels, where they cannot double-count.

## The rules (oracle-established, not assumed)

Verified against the pinned `jq-1.7.1`. Every claim here has a golden case behind it.

1. **Fan out** — loop over every argument output, emitting one result each.
2. **Then** fire the argument's trailing `Control`, after the whole prefix, never before.

   ```
   $ echo '"abcabc"' | jq -c 'label $out | [ltrimstr(("a","b", break $out))]'
   ["bcabc","abcabc"]
   ```

3. **A failure inside the loop aborts it, keeping the prefix already produced** — the same
   shape, not an early return.

   ```
   $ echo '"abc"' | jq -c 'contains(("a", 1))'
   true                                                   # stdout
   jq: error (…): string ("abc") and number (1) cannot …  # stderr, exit 5
   ```

4. **Therefore the builtin's own error can outrank a trailing `halt`** — and for
   `setpath`/`delpaths` it does. This is the correction that matters most: "emit the
   prefix, then halt" is not a rule of its own, it is what rule 3 produces when the loop
   gets that far.

   ```
   $ jq -cn 'setpath((1, halt_error(6)); 1)'            exit 5, "Path must be specified as an array"
   $ jq -cn '{"a":1} | delpaths((1, halt_error(9)))'    exit 5, "Paths must be specified as an array"
   $ jq -cn 'setpath(["a"]; (1, halt_error(6)))'        exit 6, stdout {"a":1}
   $ jq -cn 'nth((1, halt_error(3)); 1,2,3)'            exit 3, stdout 2
   $ jq -cn 'range((1, halt_error(4)); 10)'             exit 4, stdout 1..9
   ```

   The first two abort on the value `1` — not an array — *before* the halt is reached. A
   fix that uniformly emits the prefix and then halts regresses both.

5. **`empty` mid-stream skips; it does not terminate.**
   `"abcabc" | [ltrimstr(("a", empty, "b"))]` is `["bcabc","abcabc"]`. Zero outputs overall
   still means zero results (#1045) — including `combinations(empty)`, which is `[[]]`.

Rules 2 and 3 are already stated and oracle-verified in `src/jq/eval.rs`, on
`build_string_parts` (#1403, string interpolation) and `build_object_entries` (#354, object
construction). **Copy those; do not re-derive them.**

## Nesting order: four mechanisms, and they disagree

For a builtin with two generator arguments, the order is decided by *how jq defines that
builtin*, not by its signature. Inferring it from the signature produces the right number
of outputs in the wrong sequence — which no count-based test catches. This is CLAUDE.md's
#1120 lesson exactly.

| jq mechanism | order | witness |
|---|---|---|
| C builtin, 2 generator args | **rightmost is OUTER** | `{} \| [setpath((["x"],["y"]); (1,2))]` → `[{"x":1},{"y":1},{"x":2},{"y":2}]` |
| jq-level `def f($a; $b)` (`$`-bound) | **leftmost is OUTER** | `[range((0,1);(4,5);(1,2))]` → from outer, to middle, by inner |
| plain (non-`$`) filter param inside an array/reduce | **COLLECTED — no fan-out** | `"XaYbZ" \| [split(("a","b"); ("","i"))]` → **2** outputs, not 4 |
| plain filter param evaluated per match | **innermost, transposed** | `"ab" \| [sub(("a","b"); ("X","Y"))]` → `["Xb","Yb","aX","aY"]` |

Per builtin:

- `test/2`, `match/2`, `capture/2` — **flags OUTER**. `def test(re; mode): _match_impl(re; mode; true)`
  is a C call, so rightmost-outermost. `"aaa" | [match(("a","aa"); ("","g")).string]` →
  `["a","aa","a","a","a","aa"]`.
- `scan/2` — **regex OUTER** (`def scan($re; $flags)`, both `$`-bound).
- `sub/3`, `gsub/3` — **`$re` outer, `$flags` middle, replacement innermost**.
  `"aB" | [sub(("a","b"); ("X","Y"); ("","i"))]` → 7 outputs.
- `split/2`, `splits/2` — **`$re` fans out, `flags` does not.**
- `setpath/2` — **value outer, path inner.** Note this is the opposite of `sub` at the same
  arity, and it is a *change of evaluation order* from today's code, which evaluates the
  path first. That is observable through `debug`/`stderr`, so it belongs in its own commit.
- `range/1,2,3` — leftmost outermost.
- `nth`, `limit`, `indices`, `index`, `rindex`, `join`, `flatten`, `getpath`, `inside` —
  plain single-argument fan-out.

**Happily, the regex family's order is already correct in-tree.** `RegexArgFamily`
(#938/#942) forces flags-before-pattern for `TestMatchCapture` and pattern-before-flags for
`Sub` — and jq's nesting order *is* that evaluation order, first-evaluated outermost. Only
the collapse-to-one-value has to change, which makes this family far more tractable than
#1279's own audit comment feared.

## Two builtins that are not fan-out at all

**`combinations(n)` collects.** jq defines it as
`def combinations(n): . as $dot | [range(n)] | map($dot) | combinations;`. `n` is *not*
`$`-bound, so `[range(n)]` absorbs every output into one arity:

```
$ echo '[1,2]' | jq -c '[combinations((1,2))] | length'
8                       # == combinations(3), not 2^1 + 2^2 = 6
$ jq -cn '[range((1,2))]'
[0,0,1]                 # why: three elements
$ echo '[1,2]' | jq -c '[combinations(empty)]'
[[]]                    # one output, not zero
```

succinctly implements `combinations(n)` natively, so nothing falls out of the `range` work:
the effective arity is Σ over `n`'s outputs of `|range(n_i)|`. Array construction's own
escape rule applies too, so a trailing break yields no output at all.

**`sub`/`gsub`'s multi-output replacement transposes.** The replacement generator is drained
fully, in match order, before any output; the per-match lists are then transposed, padded by
*absence* rather than `null` — a match with no k-th cell contributes nothing to row k,
dropping its own preceding gap along with its text:

```
"ab"  | [gsub("(?<c>[ab])"; if .c=="a" then ("1","2")     else ("8","9") end)]  => ["18","29"]
"ab"  | [gsub("(?<c>[ab])"; if .c=="a" then ("1","2","3") else ("8","9") end)]  => ["18","29","3"]
"a-b" | [gsub("(?<c>[ab])"; if .c=="a" then ("1","2")     else "9"       end)]  => ["1-9","2"]
"abc" | [sub("a"; empty)]                                                       => ["abc"]
```

Row 1 of the third example is `"2"`, not `"2-"` — that is the absence padding. The last line
is #840's all-empty rule, which still sits on top. Implementing this closes the #840
divergence recorded in [docs/compliance/jq/limitations.md](../compliance/jq/limitations.md).

## yq mode

These builtins are `<S: EvalSemantics>`-generic and shared with yq mode, and per
[ADR-0018](../adrs/adr-0018.md) the **mode** decides. Real yq v4.53.3, probed live, is not
uniform either:

| real yq | fans out? | succinctly yq |
|---|---|---|
| `contains` (scalars, arrays, objects) | **yes** — `[true,false]` | fan out; closes a pre-existing divergence |
| `has`, `test/1`, `test/2`, `sub/2`, `sub/3`, `split`, `join`, `match`, `capture`, `tz` | no | **gate off** |
| `setpath` | errors — `SETPATH: expected single path but found 2 results instead` | keep erroring |
| `delpaths` | errors — `DELPATHS: expected single value but found 2` | keep erroring |
| `flatten(n)` | literal only — `bad expression, please check expression syntax` | gate off |
| `getpath`, `range`, `nth`, `limit`, `combinations`, `paths`, `ltrimstr`, `rtrimstr`, `startswith`, `endswith`, `index`, `rindex`, `indices`, `inside`, `splits`, `scan`, `gsub`, `strftime`, `strptime`, `bsearch`, `pow` | **lexer-rejected — the builtin does not exist** | fan out; unopposed |

`succinctly yq` accepts the lexer-rejected ones as extensions, so letting them fan out in
both modes keeps one code path at no fidelity cost. The gate is a named predicate
(`ArgFanout::yq_native`), not a scattered `S::TAG == EvalTag::Yq` check, so that
`grep ArgFanout::yq_native` returns exactly the gated set — CLAUDE.md's #106 lesson that
duplicated predicates diverge silently.

Real yq has no `halt_error` and never emits a prefix before an error, so **the halt-prefix
change has no yq oracle at all**. That belongs in
[docs/compliance/yq/limitations.md](../compliance/yq/limitations.md) rather than being left
an unremarked mode difference.

## The mechanism

### The unpacker already exists

`stream_outputs` in `src/jq/eval.rs` (named `object_outputs` before Stage 1) was already
the all-values unpacker: `QueryResult -> (Vec<OwnedValue>, Option<Control>)`, with bare
escapes becoming an empty prefix and `Partial` splitting into its prefix and control. Its
own doc comment already said it was "fully general", and `string_part_outputs` already
reused it for #1403 — so Stage 1 renamed it and generalised the doc rather than adding
anything.

Two consequences:

- **Do not add a `Result`-returning `result_to_owned_all`.** Once every value is taken,
  every case is `(values, Some(control))`; a bare escape *is* an empty prefix, which is
  `partial()`'s existing contract.
- It has **no `Partial(_, Halt)` special case**, unlike `result_to_owned_full`. So it
  already returns `(prefix, Some(Halt))` — the #1277 halt fix, for free. That arm exists in
  `result_to_owned_full` only because *that* function keeps one value and so has no honest
  prefix to emit.

Everything new is the driver, not the unpacker.

### The drivers

`fanout_arg(arg, fanout, body)` — unpack, optionally `truncate(1)` for the yq gate, then:

- **Fast path**: when there is no trailing control and exactly one value, return `body(v)`
  *verbatim*. This preserves a borrowed `One`/`OneCursor` result, which is load-bearing for
  `nth` → `index_one` and `flatten` → `builtin_flatten`, and reproduces exactly what
  `finish_result(result, None)` does today.
- Otherwise loop with `push_owned_values`, returning `partial(out, control)` on the first
  body escape (rule 3), and finishing with `partial(out, trailing)` or
  `owned_vec_to_result(out)`.

`fanout_two_args(outer, inner, …)` — same shape, with the inner expression **re-evaluated
once per outer value** (jq's `b as $b | a as $a | body`; confirmed by `debug` probes). The
inner trailing control unwinds the outer loop immediately; the outer's fires last.
`binary_fanout_core` (#768) is the existing precedent, kept separate because its
`finish_fork(optional)` policy for a combine failure differs.

`range/3` gets three nested `stream_outputs` loops written directly in `eval_range` — the
three arities differ enough that a generic 3-argument helper would be all glue.

`finish_owned` and `finish_result` become dead once the last Group A site migrates; they are
deleted in that same commit, or clippy's `dead_code` fails the stage.

### Per-builtin shapes

**Shape A** — the `finish_owned`/`finish_result` builtins already migrated by #1164. Drop
the `Ok(None)`/`Err` arms (the driver produces them), turn `finish_owned(v, trailing)` into
`QueryResult::Owned(v)` and `finish_result(r, trailing)` into `r`, and **hoist loop
invariants above the driver call**. That last point is the one non-mechanical judgement per
site: `builtin_contains`/`builtin_inside` do a whole-document `to_owned`, and `builtin_join`
collects elements — left inside the closure, an N-output argument does N deep copies.

**Shape B** — bespoke bodies with scattered internal returns (`indices`, `index`, `rindex`,
the non-regex `test`, `bsearch`). Extract the body **verbatim** into a named
`fn …_with_pattern(&StandardJson<'a, W>, &OwnedValue, bool) -> QueryResult<'a, W>` so the
diff is a pure move; `unsearchable_input` and `string_slice_pattern` already prove that
signature works here. This is the same "compute into a single local binding first" pattern
#1164 used for `has`/`load`, and it is what #1277's cluster 1 was asking for.

**Shape C** — bespoke `match` with a swallowing catch-all (`setpath`, `delpaths`, `range`,
`nth/2`, `limit`). Replace the catch-all with real `Many`/`ManyOwned`/`Partial` handling
first, then fan out.

## Two evaluators

`src/jq/eval_generic.rs` needs no direct change. Its `eval_builtin` has native arms only for
builtins outside this set and routes the rest through `eval_on_owned`, whose result
conversion maps `QueryResult::Many`/`ManyOwned` to `GenericResult::ManyOwned` and `Partial`
through `partial_generic`. **Multi-output and `Partial` both survive the bridge intact**, and
`S` is threaded through, so the yq gates apply on the YAML path with no second copy.

Add a `tests/jq_evaluator_parity_tests.rs` row per family anyway, so a future native arm
cannot bypass the fix silently.

## Risks for an implementer to sanity-check

- **Allocation.** `stream_outputs` always builds a `Vec`, where `result_to_owned_full` built
  none for `One`/`Owned`. `fanout_arg`'s fast path preserves the zero-copy *result*, not the
  zero *allocation* — that is one `Vec` per builtin call on paths as hot as `has`/`contains`
  inside `map`/`walk`. Measure before optimising; the fallback is a `One|Owned` pre-check
  that only materialises for `Many`/`ManyOwned`/`Partial`.
- **Evaluation-order change.** `setpath` flips from path-first to value-first, which is
  observable. Own commit, own probe cited, or a bisect blames the wrong change.
- **Path context** returns `PathResolveResult`, not `QueryResult`, so it cannot use
  `fanout_arg`. Two doc-commented rules point opposite ways and must both be re-read first:
  the "prefix is never longer than jq's" invariant (the exact assumption
  #972/#985 got wrong) and `resolve_leaf`'s halt-*keeping* rule.
- **`#[cfg]` matrix.** `builtin_test` has a `#[cfg(not(feature = "regex"))]` twin and a regex
  twin, touched in different stages — do not let them drift. `cargo check --no-default-features`
  is the only `no_std` gate.

## Verification

The Stage 0 corpus is the contract. `tests/data/jq-golden-known-failures.txt` holds 48 rows
across four categories (`fan-out`, `fanout-order`, `trailing-escape`, `zero-output`); the
manifest check is two-sided, so each stage must delete exactly its own rows and a row left
behind fails the build. Four further goldens
(`slice_bounds_multi_output_fanout`, `slice_bounds_multi_output_one_sided`,
`first_multi_output_arg`, `paths_filter_multi_output_cond`) pass today and guard the shapes
the fix must not touch.

New behaviour tests go in `tests/jq_cli_tests.rs` using **`run_jq_full`** — `run_jq_stdin`
shells out to `cargo run`, building a second uninstrumented binary that `cargo llvm-cov`
cannot see.

The borrowed `QueryResult::Many` arm needs a direct `#[cfg(test)]` unit test built from
`JsonIndex::build`, because most CLI-reachable generators produce `ManyOwned`; the existing
`result_to_owned_ctrl_many_arm_takes_first_borrowed_value_1164` shows the construction.

Never hand-write a golden's `expected.out` — author `filter`, `input.json` and `args`, then
run `./scripts/sync-jq-golden.sh` with the pinned jq on `PATH`.

## Related

- [#1279](https://github.com/rust-works/succinctly/issues/1279) — this document's subject.
- [#1277](https://github.com/rust-works/succinctly/issues/1277) — closed by the same design; see [One design, both issues](#one-design-both-issues).
- [#1280](https://github.com/rust-works/succinctly/issues/1280) — `eval_owned_expr_ctrl_full` collapsing an empty result to `null`. The third #1045/#1164 sibling; **not** closed here. Re-check after Stage 5.
- [#820](https://github.com/rust-works/succinctly/issues/820) and [jq-lazy-generator-consumers.md](jq-lazy-generator-consumers.md) — the opposite bug, and the source of the rule that an ordinary builtin's argument may never get a stopping sink.
- [#840](https://github.com/rust-works/succinctly/issues/840) — `sub`/`gsub` multi-output replacement; closed by Stage 9.
- [#1045](https://github.com/rust-works/succinctly/issues/1045), [#1164](https://github.com/rust-works/succinctly/issues/1164) — the zero-output and trailing-escape fixes this work completes.
- [#1556](https://github.com/rust-works/succinctly/issues/1556) and
  [jq-range-lazy-bounds.md](jq-range-lazy-bounds.md) — the sixth review finding (below),
  closed by its own design doc rather than branch-local like the other five.
- [ADR-0018](../adrs/adr-0018.md) — the reference-fidelity rule the yq gates answer to.

## Follow-ups

Explicit non-goals of the fan-out work, so they do not read as oversights. Filed where a
dedicated issue was warranted; otherwise the existing issue that already covers them.

- **#1277 cluster 4** — `eval_owned_expr_opt` inside `eval_pipe_with_path_context_internal`
  drops its trailing control *and* array-collapses 2+ outputs.
  Filed as [#1559](https://github.com/rust-works/succinctly/issues/1559); no user-visible
  divergence found for it yet, and the issue records the probes that came back clean.
- **`pick`/`omit`** — they collapse to the first output too, but implement yq's value-array
  `pick`, a different builtin from jq's path-expression one. Not filed: it needs the oracle
  question settled first (which `pick` is succinctly's?), not a fan-out change.
- **`eval_index_expr`/`eval_slice_expr`** — the conservative `Partial(_, Error) => Error`
  prefix drop, documented in place as outside #400/#494's verified semantics.
- **yq `setpath`/`delpaths` count-message wording** — succinctly's `expected a single result
  but found 2` versus real yq's per-slot `SETPATH:`/`DELPATHS:` text. Recorded in
  [docs/compliance/yq/limitations.md](../compliance/yq/limitations.md).
- **`combinations` error wording** on a non-array input (`expected array, got number` vs jq's
  `Cannot iterate over number (1)`) — pre-existing and unrelated to fan-out; already covered by
  [#991](https://github.com/rust-works/succinctly/issues/991)'s `type_error` wording audit.

## Review findings on this work

A high-effort review of the implementation branch found six further defects. Five were
branch-local and fixed there: [#1531](https://github.com/rust-works/succinctly/issues/1531)
(arguments pulled eagerly, reaching `input`/side effects jq's laziness skips),
[#1532](https://github.com/rust-works/succinctly/issues/1532) (fanned-out `getpath` discarding
its resolved prefix), [#1534](https://github.com/rust-works/succinctly/issues/1534) (yq's
`FirstOnly` gate emitting a value it then raises over),
[#1537](https://github.com/rust-works/succinctly/issues/1537) (the `ArgFanout` dispatch
duplicated five times), and the single-argument half of
[#1533](https://github.com/rust-works/succinctly/issues/1533). The sixth,
[#1556](https://github.com/rust-works/succinctly/issues/1556), was `range`'s own bound
resolution — left on the eager `stream_outputs` pattern from the start (Stage 6's own
bespoke-triple-loop decision, above), not one of #1531's shared `fanout_arg`/`fanout_two_args`
call sites — and needed its own design pass rather than a branch-local fix; see
[jq-range-lazy-bounds.md](jq-range-lazy-bounds.md).

#1531's lazy pull is the one that changed a documented conclusion: it closed the last golden
known-failure, the #820 eager-argument residue this design had recorded as out of scope.

#1533's two-argument half is now closed too, but not by the general "an escape in the argument
clears its values" rule — applying that to `fanout_two_args` regressed two shapes, because
emptying one slot's values skips the body, and the body is where the *other* slot gets
validated (`test` wants the flags, `setpath` wants the path — no shared outer/inner order).
That general rule stays reverted. The fix instead is narrower: `fanout_two_args` only defers to
a slot's own trailing escape when that slot's `RejectMany` *count* check (`args.len() > 1`) is
what's about to fire — a count violation there is itself a symptom of the escape (`(1, 2,
error("x"))` has two values only because the generator didn't collapse to one before raising),
so it doesn't touch the single-value-then-escape shapes the reverted rule broke. Confirmed by
`test_yq_setpath_two_argument_reject_many_propagates_an_embedded_error_1533` (the former pin,
now flipped) alongside the still-passing regression guards
(`test_yq_two_argument_body_validation_outranks_a_slot_escape_1533`,
`test_yq_fanout_two_args_argument_escape_reports_bare_not_prefix_then_raise`).

Review of that fix found one more gap before it landed: when *both* slots independently trip
`RejectMany` at once, it reported whichever slot (outer) it happened to check first, rather than
real yq's own consistent rule — inner (for `setpath`, the path) always wins, whether either
slot's violation escapes or is a plain count mismatch. `fanout_two_args` now always evaluates
inner before reporting outer's own violation of either kind, matching real yq's evaluation order
rather than just guessing at its final answer. Live-verified across all four
escaping/clean combinations; see `test_yq_setpath_reject_many_prefers_inner_violation_over_outer_1533`.
