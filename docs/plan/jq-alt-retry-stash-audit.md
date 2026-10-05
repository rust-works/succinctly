# `?//` stash-slot audit: verdicts for every out-of-band slot (#3517)

[Home](../../) > [Docs](../) > [Plan](./) > `?//` stash-slot audit

The closing audit of #3293, the stale-escape-across-a-`?//`-retry audit
(slices 1-9: #3411, #3421, #3434, #3451, #3472, #3485, #3492, #3495, #3508, #3515). Every
production slot the issue's triage grep finds gets a verdict, taken at `main` @ `1fa800b92`.

The class: a sink closure stashes an escape in a bare local and answers `Demand::Stop`; a
`?//` inside the generator it drives retries past that stop and calls the closure again; the
slot now describes an alternative jq has abandoned, and the exit path lets it outrank the
retry's own verdict. A reset at the top of the closure fixes the "retry answers cleanly" ending
only: a retry that produces nothing or raises never re-enters the closure, so a slot also needs
a generation stamp (`StashedEscape` + `retry_superseded`). See `stop_with_escape`'s
re-invocation contract in `src/jq/eval.rs`.

## Method

The triage grep, re-run on `main`:

```bash
git grep -nE "let (mut )?[a-z_]+: (core::cell::(Ref)?Cell<)?Option<(Control|Flow|EvalEscape|ResolveFlow|EvalError)>>? = " src/jq/eval.rs src/jq/eval_generic.rs
```

It finds 64 matches: 60 production slots and 4 in a unit test. Slots already moved onto
`StashedVerdict`/`StashedEscape` are not typed `Option<..>`, so they are out of the grep by
construction. For each slot the verdict rests on the code plus a probe grid: four retry
endings (answers, produces nothing, raises, fails to destructure) x consumers (none,
`first`, `limit`, a trailing comma) x both CLI routes (document on stdin, and `-n` with the
literal, the owned route), diffed against `/usr/bin/jq` 1.7.1, with a stderr marker
(`("A"|stderr)`) showing the `?//` really retried before a clean row was believed. Auditors
ran read-only against a debug build; the highest-severity rows in every group were reproduced
a second time by the audit's owner.

Verdicts: **FIXED** (the bare slot is safe: reset per invocation and read only where a stale
value is dead), **NOT REACHABLE** (the closure cannot be re-invoked after its stop, or can
only hold a non-retryable decode failure), **ACCUMULATING** (kept across invocations on
purpose; none are left in the bare list), **REACHABLE** (a live divergence from jq tied to
the slot; filed).

## Result

35 of 60 slots are REACHABLE, 6 FIXED, 19 NOT REACHABLE. The
reachable ones fall into six root causes, each filed:

| Issue | Family                                                                                        |
|-------|-----------------------------------------------------------------------------------------------|
| #3805 | the owned-identity walk (`eval_owned_identity_*`), nine slots, owned route                    |
| #3806 | a late-forced lazy item (`G \| map(f)`) on the document route, thirteen sinks                 |
| #3807 | the target of a computed index or slice                                                       |
| #3808 | five path-mode and owned sinks that ignore the retry generation at exit                       |
| #3809 | slice-bound decode failures bypass `stop_with_escape`, so a `?//` retries past them           |
| #3810 | `any`/`all`'s sticky `decided` swallows a retried alternative's raise or halt, on both routes |

`scripts/jq-alt-retry-oracle-sweep.sh` reports **2689 cases, 0 unexpected, 0 known, 2689
matched** on this tree, and carries no `#3293 slice N` tags. A clean sweep is not evidence the
table above is clean: the sweep feeds every case through stdin and never uses the owned `-n`
route, so it cannot reach `eval_owned_identity_*`, and it has no row whose `?//` precedes a
late-forced lazy `map` on a cursor-backed array, which is why none of the 35 reachable slots
tripped it. #3805 and #3806 each ask for the matching sweep family.

## Per-slot verdicts

| #  | File            | Function                                       | Slot                | Verdict       | Why                                                                                                                                       | Pin / issue                                                             |
|----|-----------------|------------------------------------------------|---------------------|---------------|-------------------------------------------------------------------------------------------------------------------------------------------|-------------------------------------------------------------------------|
| 1  | eval.rs         | `fanout_arg`                                   | `body_control`      | FIXED         | read only in the `Stopped` arm and every stop writes it afresh                                                                            | #2952 `fanout_arg` rows; `nth(G)` grid                                  |
| 2  | eval.rs         | `take_at_index`                                | `escape`            | NOT REACHABLE | written only for a decode failure, which `stop_with_escape` marks non-retryable                                                           | `test_as_pattern_alternatives_retry_under_a_wrapping_consumer_1519`     |
| 3  | eval.rs         | `each_any_all_gen_cond`                        | `probe_escape`      | FIXED         | stop flag reset per invocation; slot cleared on a decisive element                                                                        | `test_counted_bool_consumers_reset_their_stop_across_retry_3293`        |
| 4  | eval.rs         | `each_index_expr`                              | `own_escape`        | REACHABLE     | target-drive pair resets only on re-invocation; nothing/raises endings never re-enter                                                     | #3807                                                                   |
| 5  | eval.rs         | `eval_each_pipe`                               | `downstream`        | FIXED         | stamped with `stopped_at`; exit goes through `pipe_terminal_after_retry`                                                                  | `test_pipe_first_stage_retry_supersedes_stashed_sink_verdict_3293`      |
| 6  | eval.rs         | `owned_path_door`                              | `downstream`        | REACHABLE     | bare slot, no stamp; `downstream.unwrap_or(upstream)` always wins (owned route)                                                           | #3808                                                                   |
| 7  | eval.rs         | `builtin_in`                                   | `check_escape`      | NOT REACHABLE | plain local in a `for` over eagerly collected candidates; no sink closure                                                                 | `in(G)` is the eager-argument family (#2180)                            |
| 8  | eval.rs         | `any_all_gen_cond`                             | `probe_escape`      | FIXED         | cleared on `Ok(true)`, folded only on `Stopped`                                                                                           | `test_as_pattern_alternatives_retry_under_a_wrapping_consumer_1519`     |
| 9  | eval.rs         | `slice_pair_streaming`                         | `escape`            | REACHABLE     | same shape as `each_index_expr`: reset only on re-invocation                                                                              | #3807                                                                   |
| 10 | eval.rs         | `pull_slice_bound`                             | `decode_failure`    | REACHABLE     | bare `Demand::Stop` bypasses `stop_with_escape`, so a `?//` retries past a decode failure                                                 | #3809                                                                   |
| 11 | eval.rs         | `stream_path_writes`                           | `parked`            | REACHABLE     | `=` only: a retry that resolves no path never re-enters the sink; `\|=`/`op=`/`//=` accumulate by design (documented)                     | #3808                                                                   |
| 12 | eval.rs         | `resolve_node_sink`                            | `idx_escape`        | REACHABLE     | `INDEX` arm: never reset, no generation check at exit                                                                                     | #3808                                                                   |
| 13 | eval.rs         | `resolve_node_eager`                           | `arg_escape`        | NOT REACHABLE | straight-line local over an eagerly collected list; read once                                                                             | grid: 15 rows identical to jq                                           |
| 14 | eval.rs         | `resolve_leaf_sink`                            | `construct_refusal` | NOT REACHABLE | refusal-table builtins evaluate their `?//` operands eagerly, so the closure never runs after its stop                                    | grid (moderate confidence: inferred from behaviour)                     |
| 15 | eval.rs         | `resolve_leaf`                                 | `construct_refusal` | NOT REACHABLE | same as `resolve_leaf_sink`, with a `Vec` collector                                                                                       | same grid                                                               |
| 16 | eval.rs         | `expand_queued (value walk)`                   | `pending_error`     | NOT REACHABLE | `queue_children` collects `f` in full before the sink runs; per-activation local                                                          | `test_recurse_retry_supersedes_stashed_verdict_3293` (native walk only) |
| 17 | eval.rs         | `expand_queued (path walk)`                    | `pending_error`     | NOT REACHABLE | same: `queue_children` resolves `f`/`cond` in full                                                                                        | same pin (native walk only)                                             |
| 18 | eval.rs         | `resolve_terminal_sink`                        | `violation`         | REACHABLE     | reset on re-entry only; exit `match (violation, flow)` has no `retry_superseded`                                                          | #3808                                                                   |
| 19 | eval.rs         | `drive_lazy`                                   | `escape`            | NOT REACHABLE | written only when `into_owned` fails: a decode failure, non-retryable                                                                     | -                                                                       |
| 20 | eval.rs         | `walk_object_entries`                          | `ended`             | REACHABLE     | computed-key `{(K): $v}` entry: never reset, no generation check                                                                          | #3808                                                                   |
| 21 | eval_generic.rs | `owned_identity_bind_door`                     | `escaped`           | NOT REACHABLE | `forward` only receives `GenericItem::Owned`, so the slot is never written (code reading; no live probe reaches it)                       | -                                                                       |
| 22 | eval_generic.rs | `drive_foreach_expr_generic`                   | `escape`            | REACHABLE     | document route: a lazy `map` item fails with a retryable error; slot never reset (a `halt` ending matches jq)                             | #3806                                                                   |
| 23 | eval_generic.rs | `take_at_index_generic`                        | `skipped_err`       | REACHABLE     | document route: a skipped lazy item's retryable error is stashed and outranks the retry; its doc comment still describes #2199's ordering | #3806                                                                   |
| 24 | eval_generic.rs | `each_isempty_generic`                         | `escape`            | REACHABLE     | only `outer_stopped` is reset; exit prefers `escape` unconditionally                                                                      | #3806                                                                   |
| 25 | eval_generic.rs | `any_all_probe_item_generic`                   | `escape`            | REACHABLE     | stale `escape`, plus a sticky `decided` (both routes) that outranks a later raise or halt                                                 | #3806, #3810                                                            |
| 26 | eval_generic.rs | `each_any_all_gen_cond_generic`                | `probe_escape`      | FIXED         | cleared on each decisive match; folded only on `Stopped`                                                                                  | `test_as_pattern_alternatives_retry_under_a_wrapping_consumer_1519`     |
| 27 | eval_generic.rs | `each_upper_in_generic`                        | `escape`            | REACHABLE     | same shape as `each_isempty_generic`                                                                                                      | #3806                                                                   |
| 28 | eval_generic.rs | `each_skip_generic`                            | `escape`            | REACHABLE     | per-count slot never reset; `resume_from_escape` runs unconditionally                                                                     | #3806                                                                   |
| 29 | eval_generic.rs | `isvalid_generic`                              | `escape`            | REACHABLE     | `(Some(control), _)` matched ahead of `flow`; also swallows a later halt                                                                  | #3806                                                                   |
| 30 | eval_generic.rs | `LoopState::fork`                              | `escape`            | REACHABLE     | only once `loop_enter_stream` has declined (about 5000 frames of `def` recursion); low severity                                           | #3806                                                                   |
| 31 | eval_generic.rs | `LoopState::cond_bits`                         | `escape`            | REACHABLE     | same precondition as `fork`                                                                                                               | #3806                                                                   |
| 32 | eval_generic.rs | `each_select_generic`                          | `escape`            | REACHABLE     | `resume_from_escape` unconditional; value-mode, document route                                                                            | #3806                                                                   |
| 33 | eval_generic.rs | `process_index_key`                            | `own_escape`        | REACHABLE     | twin of `each_index_expr`: closure reset only on re-invocation                                                                            | #3807                                                                   |
| 34 | eval_generic.rs | `eval_each_pipe_generic`                       | `downstream`        | FIXED         | stamped; exit via `pipe_terminal_after_retry`                                                                                             | `test_pipe_first_stage_retry_supersedes_stashed_sink_verdict_3293`      |
| 35 | eval_generic.rs | `collect_yq_context`                           | `stray`             | NOT REACHABLE | yq-only route; yq has no `?//` (real yq rejects it at the lexer)                                                                          | -                                                                       |
| 36 | eval_generic.rs | `collect_together`                             | `stray`             | NOT REACHABLE | yq-only route; no `?//`                                                                                                                   | -                                                                       |
| 37 | eval_generic.rs | `cross_together`                               | `stray`             | NOT REACHABLE | yq-only; sink is only handed `Owned` items, so the stash is dead                                                                          | -                                                                       |
| 38 | eval_generic.rs | `eval_compare_generic`                         | `stray`             | NOT REACHABLE | sink is only handed `Owned` items; the `Err` arm is dead                                                                                  | -                                                                       |
| 39 | eval_generic.rs | `each_alternative_generic`                     | `escape`            | REACHABLE     | `escape` half: never reset, no generation check; the sticky `outer_stopped` half is not observable                                        | #3806                                                                   |
| 40 | eval_generic.rs | `boolean_operand_bits_generic`                 | `escape`            | REACHABLE     | stale lazy error masks a later `halt_error` (exit 5 for 3)                                                                                | #3806                                                                   |
| 41 | eval_generic.rs | `negate_driving_operand_sink`                  | `stray`             | NOT REACHABLE | sink only receives `Owned` values from `arith_negate`                                                                                     | `test_negate_retry_supersedes_stashed_sink_verdict_3293`                |
| 42 | eval_generic.rs | `stream_owned_outputs_generic`                 | `decode_err`        | REACHABLE     | `parent(n)` caller forces a lazy item; `decode_err.or(flow)` unconditional                                                                | #3806                                                                   |
| 43 | eval_generic.rs | `fanout_arg_generic`                           | `body_control`      | NOT REACHABLE | read only in the `Stopped` arm; every stop writes it afresh                                                                               | 60-row `limit`/`nth` grid                                               |
| 44 | eval_generic.rs | `fanout_arg_generic`                           | `decode_err`        | NOT REACHABLE | `MaterializesLazyItems` forces inside the `?//` body; other failures are decode failures (latent: no `StashedEscape`)                     | `getpath(X)` grid                                                       |
| 45 | eval_generic.rs | `nth_with_n_generic`                           | `skipped_err`       | REACHABLE     | skipped lazy item's retryable error outranks the retry                                                                                    | #3806                                                                   |
| 46 | eval_generic.rs | `slice_pair_streaming`                         | `escape`            | REACHABLE     | twin of the `eval.rs` function                                                                                                            | #3807                                                                   |
| 47 | eval_generic.rs | `pull_slice_bound_generic`                     | `decode_failure`    | REACHABLE     | twin of `pull_slice_bound`                                                                                                                | #3809                                                                   |
| 48 | eval_generic.rs | `path_context_step_getpath`                    | `walk_error`        | NOT REACHABLE | a walk failure inside a `?//` getpath argument is never retried (the `getpath(K)` family)                                                 | #3487                                                                   |
| 49 | eval_generic.rs | `path_context_component_flow`                  | `decode_err`        | REACHABLE     | path-context `if`/`select` condition forces a lazy item; returned unconditionally                                                         | #3806                                                                   |
| 50 | eval_generic.rs | `try_path_context_walk_sink`                   | `downstream`        | NOT REACHABLE | every walk step is eager; an upstream `?//` re-calls the function with a fresh slot                                                       | -                                                                       |
| 51 | eval_generic.rs | `eval_owned_identity_alternative`              | `rest_escape`       | REACHABLE     | owned route: no reset, no check; returned ahead of `left_flow`                                                                            | #3805                                                                   |
| 52 | eval_generic.rs | `eval_owned_identity_scoped`                   | `rest_escape`       | REACHABLE     | owned route: `label`/`try`/`?` raise the abandoned alternative's error                                                                    | #3805                                                                   |
| 53 | eval_generic.rs | `eval_owned_identity_try`                      | `rest_escape`       | REACHABLE     | owned route: catch-handler tail; `resume_from_escape` with no generation                                                                  | #3805                                                                   |
| 54 | eval_generic.rs | `eval_owned_identity_any_all`                  | `probe_escape`      | REACHABLE     | owned route: never reset, consulted before `flow`                                                                                         | #3805                                                                   |
| 55 | eval_generic.rs | `eval_owned_identity_any_all`                  | `rest_flow`         | REACHABLE     | owned route: with sticky `decided`, drops the default `any`/`all` output                                                                  | #3805                                                                   |
| 56 | eval_generic.rs | `eval_owned_identity_bounded`                  | `rest_escape`       | REACHABLE     | owned route: returned before `flow`                                                                                                       | #3805                                                                   |
| 57 | eval_generic.rs | `eval_owned_identity_bounded`                  | `rest_stopped`      | REACHABLE     | owned route: swallows a retry's raise (document route reproduces)                                                                         | #3805                                                                   |
| 58 | eval_generic.rs | `eval_owned_identity_stages (metadata getter)` | `downstream`        | NOT REACHABLE | one item from an argument-less getter: no sub-expression, so no `?//` and at most one closure call                                        | -                                                                       |
| 59 | eval_generic.rs | `eval_owned_identity_stages (AsPattern)`       | `ended`             | REACHABLE     | object-pattern key `{(K): $v}` re-invokes the closure; stale `Some(flow)` returned                                                        | #3805                                                                   |
| 60 | eval_generic.rs | `eval_owned_identity_stages (ruled arm)`       | `downstream`        | REACHABLE     | `Some(flow) => flow` at exit with no generation check                                                                                     | #3805                                                                   |

Counts: 35 REACHABLE, 6 FIXED, 19 NOT REACHABLE, 0 ACCUMULATING among the bare
slots (`stream_path_writes` accumulates deliberately for `|=`/`op=`/`//=`, whose divergence is
message-only and recorded in `docs/compliance/jq/limitations.md`).

## The issue's Tier C sites

| Site (issue's Tier C list)                      | Slot                                              | Verdict       | Where                                                          |
|-------------------------------------------------|---------------------------------------------------|---------------|----------------------------------------------------------------|
| `owned_identity_bind_door`                      | `escaped`                                         | NOT REACHABLE | row 21                                                         |
| `owned_path_door`                               | `downstream`                                      | REACHABLE     | row 6, #3808                                                   |
| `walk_object_entries`                           | `ended`                                           | REACHABLE     | row 20, #3808                                                  |
| `each_path_on_owned`                            | `StashedEscape` + `stopped_at` (no bare slot)     | FIXED         | `retry_superseded` at exit; the grep cannot see it             |
| `stream_path_writes`                            | `parked`                                          | REACHABLE     | row 11, #3808 (`=` only)                                       |
| `path_context_step_getpath`                     | `walk_error`                                      | NOT REACHABLE | row 48, #3487                                                  |
| `path_context_component_each`                   | none (adapter over `path_context_component_flow`) | NOT REACHABLE | inherits row 49 (#3806)                                        |
| `try_path_context_walk_sink`                    | `downstream`                                      | NOT REACHABLE | row 50                                                         |
| `each_alternative` / `each_alternative_generic` | `escape`, `outer_stopped`                         | REACHABLE     | `outer_stopped` not observable; `escape` half is row 39, #3806 |
| `queue_children`                                | `pending_error`                                   | NOT REACHABLE | rows 16-17: collected in full before the sink runs             |

## What the table does not say

- Two verdicts rest on code reading alone: `owned_identity_bind_door` (row 21) and the
  `resolve_leaf*` pair (rows 14-15, behavioural inference rather than instrumentation).
- `path_context_step_getpath`, `fanout_arg_generic` and the yq-only rows were argued from the
  call graph plus a probe grid that proved the retry happened; no breakpoint-level proof of
  reach was possible in the audit sandbox.
- Separate divergences met on the way, not stale slots: bare `G | map(f)` on the document
  route makes one attempt where jq retries (noted under #3806); value-mode `INDEX(G; f)`
  and `path(fromstream(G))` evaluate their `?//` eagerly (the #2180 / #3487 family).
- Reproduction status of each filed issue's rows is stated in that issue; rows the owner did
  not re-run are labelled as reported.
