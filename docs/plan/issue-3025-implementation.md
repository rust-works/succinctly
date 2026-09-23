# #3025 implementation plan after the spike

This supersedes the initial implementation plan. The [completed spike](https://github.com/rust-works/succinctly/issues/3025#issuecomment-5789511740) supplies the performance evidence; its disposable patch is not the production implementation.

## Chosen design

1. Preserve every non-NaN `NumberLiteral` through the internal bridge, without a length cap. This is the correctness fix for all routes, including routes that remain bridged.
2. Add bounded, consuming owned execution for the operations demonstrated to cause repeated bridge work. Keep the existing lossless bridge for everything outside that explicitly supported set.
3. Fix the collecting pipeline to collect projected results rather than retaining all intermediate loop states. Keep array construction externally atomic.
4. Keep `OwnedValue::NumberLiteral(NumberRepr, Box<str>)` and the existing public API. Use existing copy-on-write array/object storage. Do not introduce a shared-literal public representation, a cache, an internal numeric side table, or a general owned evaluator in this issue.

These choices separate the two measured costs: repeated parsing/indexing and retention of full intermediate states. Some literal-byte copying may remain in supported transformations; the acceptance condition is bounded peak memory and acceptable measured cost, not a claim that every operation becomes independent of literal length.

#3040 is not a dependency. It concerns admission of YAML source text before an owned literal exists. The two extreme native formatting mismatches recorded in the spike also remain separate: neither is fixed by preserving a literal across the bridge. Do not broaden this issue to fix their final formatting.

## Work package 1: preserve the literal and pin the invariant

In `src/jq/value.rs`, retain the existing NaN handling before a single verbatim `NumberLiteral` arm in `to_json_for_reindex_at_depth`. Remove both finite and infinite cap fallbacks. Preserve computed-float tokens, bare infinity handling, and depth limits.

In `src/jq/eval_generic.rs`, remove `REINDEX_LITERAL_LEN_CAP` and admit non-NaN literals at all lengths in `reindex_bridge_is_identity`. Retain the NaN-literal exception and recursive container checks.

Replace the two serializer tests requiring short lossy output with exact round-trip text checks in both modes. Update `test_reindex_bridge_identity_predicate_agrees_1909` using real round trips at 256, 257, 300, and 200,000-character scales. Check text/provenance as well as numerical observables. Keep coverage for bare floats, signed zero, infinities, NaNs, and depth guards.

This package is necessary but must not land alone: the spike demonstrated its seconds-scale CPU and hundreds-of-MB memory regression.

## Work package 2: bounded owned operations

Add an internal consuming dispatcher in `eval.rs`, conceptually:

```rust
// Proposed internal shape; use existing project types for errors/control.
enum OwnedStep {
    Declined(OwnedValue),
    Handled(Flow),
}

fn try_eval_owned_step<S: EvalSemantics>(
    expr: &Expr,
    state: OwnedValue,
    optional: bool,
    reentry: Reentry,
    sink: &mut dyn FnMut(OwnedValue) -> Demand,
) -> OwnedStep;
```

The defining contract is more important than the final spelling:

- Decide eligibility before emitting, evaluating side effects, or mutating state. `Declined` returns the original state and has not touched the sink.
- Once execution starts, return its actual success/error/control result. Never replay the expression through the bridge after partial execution.
- A successful step consumes state so unique copy-on-write containers can be mutated or moved without an artificial extra owner. Existing borrowed entry points can clone the container handle when they genuinely must preserve an input.
- Carry the existing `Reentry` proof rules; do not manufacture a document identity for rebuilt state. Initially decline operations with active tracked-node/embedding context where the existing path/identity doors must handle them.

Initial supported set:

- Identity and already-supported constant field/index navigation.
- `to_entries` on owned arrays/objects and `from_entries` on valid owned input containers. Factor/reuse `entries_to_object::<S, _>` for the latter, including key aliases, duplicates, ordering, mode-specific input acceptance, and errors. Do not special-case the exact composition as identity: array keys and malformed entries make that unsound.
- Pipes whose complete stage sequence belongs to this supported set. Preflight the complete sequence; a general pipe with unsupported stages keeps the existing evaluation route. This makes `to_entries | from_entries` work without a serialization boundary between those stages.
- Compound arithmetic assignment to a single, existing, top-level object field, with an already evaluated numeric literal RHS and numeric field value. This covers the measured `.i += 1` without pretending to support general assignment. Support the compound operators through the existing mode-aware arithmetic implementation where their semantics are defined; no handwritten arithmetic or numeric formatting.

For the compound-assignment arm, preflight the AST and target before mutation. Evaluate/convert the literal RHS once, obtain the target using existing lookup semantics, use the existing arithmetic helper, then write through the existing copy-on-write object API. Generated numeric results remain computed numbers; untouched siblings retain their source text. Apply the existing optional-error convention.

Missing fields, nested/dynamic/multiple targets, slices, nonnumeric operands, general RHS expressions, tracked-node contexts, and unsupported assignment forms decline to the lossless bridge. They retain correctness but do not acquire a new performance guarantee in #3025. This restriction avoids copying the complex jq/yq assignment/RHS/fanout machinery into a second implementation.

Wire the consuming entry at `fold_step_each` and the computed-state update in `LoopState`/`loop_step_generic`. Factor common dispatch so borrowed `eval_each_owned`, collecting owned evaluation, and generic `eval_on_owned` can use the same operation definitions rather than disagreeing about semantics. Preserve the existing path/identity doors and their precedence for contexts the new dispatcher declines. Do not route a document cursor through an owned materialization solely to access these optimizations.

## Work package 3: collect final outputs, not intermediate states

The inspected `eval_generic::eval_single` array arm starts by eagerly evaluating its entire inner expression. Change the applicable owned-result collection path to drive the inner expression through the existing sink evaluator into a private array buffer. In `[while(...) | .i]`, each loop state must reach `.i` before the next state is accumulated; the buffer receives integers, not copies of the long-literal object.

Preserve cursor/LazySeq behavior where the current arm intentionally retains document cursors; do not indiscriminately convert that path to owned values. Factor the existing array result/control conversion so both paths share the same outcome policy.

Atomicity is mandatory: the private buffer is not published to the outer sink until array construction completes under its existing policy. Preserve error, optional, break, halt, and partial-result handling rather than treating every non-success alike. No new short-circuiting may skip side effects that current array evaluation performs. Final arrays that actually request all full states, such as `[while(...)]`, may still require proportional output storage; the memory guarantee applies to collecting a small projection, not intrinsically large output.

## Work package 4: one bounded integration prototype

Before expanding tests or refactoring more callers, combine the three packages in a disposable/provisional implementation and rerun the existing fixture harness. This is an implementation gate, not another open-ended design investigation.

The prototype must demonstrate:

- The 54-case boundary/filter matrix still passes against pinned jq on both hosts.
- `.i += 1` and `to_entries | from_entries` loop bodies no longer enter the serialization/index-building bridge once their state is owned. Verify with narrowly scoped test instrumentation or a profiler; do not infer this merely from elapsed time.
- Collecting the `.i` projection no longer retains every long-literal state.
- Existing public literal storage remains unchanged.

If this fails, identify the failing contract and report the required design amendment before expanding to shared literal storage or a general evaluator. Do not quietly broaden the issue or claim the benchmark fixes are complete. A public representation migration would require a separately explicit API compatibility decision.

## Work package 5: correctness, fallbacks, and route coverage

Apply the repository testing skill: reproduce failures first and assert outputs, not just successful execution.

- Preserve the existing issue/filter matrix, integer and decimal boundaries, exact comparison against `1e300`, nested values, update/delete selection, and large literal round trips. Run the shared serializer checks in both jq and yq modes; capture oracle expectations from the pinned binaries.
- For each newly supported operation, compare direct owned execution with the explicit lossless bridge test seam and with reference CLI output where supported. Exercise repeated execution, not only one step.
- For entries: array/object ordering, duplicate keys, accepted key/value aliases, malformed entries, invalid input types, optional errors, and unchanged literal payloads.
- For narrow assignment: every admitted operator/mode, computed-number provenance, signed zero/overflow/error cases, existing numeric target, and unchanged long siblings. Verify missing/nested/dynamic/multiple targets and general RHS expressions decline without mutation or sink calls. Capture the reference semantics rather than assuming jq and yq agree.
- For array collection: normal output, `empty`, errors after earlier values, optional errors, nested arrays, labels/break/halt, sink cancellation where applicable, and tracked path identity. Include checks that no partial array escapes on an error path that should be atomic.
- Update the identity agreement test. `test_path_non_navigable_falls_back_when_reindex_is_not_identity_3122` can no longer claim its 257-byte literal forces fallback. Keep it as a fidelity test and add deliberate fallback coverage through the existing bridge seam or an appropriate constructed NaN-literal fixture. Audit #2925 CLI and evaluator-parity cases for the same change in routing.
- Keep the two extreme native-formatting repros explicitly characterized as pre-existing failures, separately from correctness-passing matrices. Link separate tracking before closing #3025 if no existing issue owns them.

## Performance acceptance

Use the same M4 Pro and Ryzen hosts, baseline commit `717833c17724a2db6281518af8cb9a3a175cbeb3`, pinned oracles, release profile, and interleaved harness from the spike. Also retain the unoptimized lossless candidate as a third comparison. Record compiler/platform metadata and output checks. Generate fixtures outside timing.

Chosen engineering budgets for this fix (targets, not results already achieved):

- At 200,000 zero digits and 3,200 iterations, each measured while/until/entries-reduce workload must be below 500 ms median on each host and at least 8x faster than that host's unoptimized lossless candidate. Report both conditions rather than choosing whichever passes.
- The collecting `.i` projection must remain below 64 MiB peak RSS; streaming and until below 32 MiB on these fixtures. The purpose is to reject the measured 600+ MB retained-state behavior, not to promise these limits for arbitrary outputs.
- Repeat the 20,000 / 200,000 / 1,000,000-digit and 200 / 800 / 3,200-iteration curves. Remaining copying/output work must be distinguished from repeated indexing/parsing; there must be no per-update full-index rebuild for the supported bodies.
- Simple reduce/foreach and representative ordinary-input workloads must not show a repeatable >5% end-to-end regression outside the A/A noise envelope. Use enough work per sample to overcome startup overhead; do not enforce percentage gates on 3–5 ms process runs. Investigate >10% memory regressions on ordinary workloads.

These budgets deliberately allow some lossless copying overhead compared with the incorrect capped baseline. Correctness is not negotiable if a timing target is missed.

## Delivery and stopping point

Keep the work reviewable as separate commits for invariant/tests, owned operations, projected collection, and evidence/docs, but merge only the combined, verified result. Do not release the known-regressing cap removal on its own.

Run focused tests during implementation, then relevant full jq/yq/parity suites, default/SIMD builds, CLI release build, formatting, clippy, rustdoc, the documented no_std configuration, the prescribed build script, and CI-feature coverage. Update the serializer/identity/route comments and jq evaluator/compliance documentation. No new ADR is required for applying ADR-0018 without changing public storage or compatibility policy.

Close #3025 when the bridge is lossless on all supported literals, the bounded optimized paths and projected collection satisfy the correctness/performance gates, fallback routes remain correct and covered, and both-platform evidence is posted. General assignments and other unsupported bodies may still incur lossless bridge cost; state that scope explicitly. #3040 and extreme final-formatting problems remain separately tracked.

The design choices above are now explicit. The remaining uncertainty is whether the bounded implementation meets the chosen budgets; work package 4 resolves that before the fix grows. No claim is made that a routing tool will assign a lower model tier: the control-flow and identity review remains demanding even with a concrete plan.
