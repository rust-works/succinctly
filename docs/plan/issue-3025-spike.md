# Issue #3025: lossless reindex spike

Date: 2026-09-23. Baseline: `717833c17724a2db6281518af8cb9a3a175cbeb3`.

## Decision

Removing the cap fixes the observed correctness failures, but is not sufficient for a production fix: current loop machinery still repeatedly indexes and parses unchanged long literals. Implement lossless serialization together with a measured reduction in owned-state bridge work. Do not substitute a rounded value, and do not treat `Rc<str>` alone as the performance solution.

## Experiment

A temporary candidate replaced both capped `NumberLiteral` serializer branches with verbatim text serialization (retaining the earlier NaN case), and changed the identity predicate to admit non-NaN literals at any length. The source files were restored after building; the experimental patch is in `.ai/scratch/3025-spike/candidate.patch`. This patch deliberately does not update the old tests/documentation and is not a merge-ready implementation.

Both binaries used `cargo build --release --features cli`, the repository's release profile, and no additional target CPU flags. Local exploratory and confirmation runs used Apple M5 Max, ARM64, macOS 26.5.2, Rust 1.98.0, on AC power. Following user approval, fleet runs used Apple M4 Pro / Darwin 25.5.0 / Rust 1.96.0 and AMD Ryzen 9 7950X / WSL2 Linux 6.18.33.2 / Rust 1.97.0. The compiler is identical within each host's baseline/candidate pair; cross-host absolute timing differences are not attributed solely to CPU architecture. The source archive and candidate patch hashes agree across hosts. Host activity checks found no competing builds or benchmark processes before these runs.

The confirmation runs interleaved baseline/candidate order across three repetitions and report median process wall time. Fixtures were generated before timing; only small scalar/count results were printed. 20,000-, 200,000-, and 1,000,000-digit zero-mantissa inputs establish scaling. Millisecond-scale controls are startup-dominated and do not establish small percentage changes. Local results are sufficient to establish the large algorithmic regression, not a substitute for the fleet measurements. An exploratory run overlapped profiling briefly; use the separate confirmation data for numerical claims.

Exact #1402 commands were not present in the issue/PR comments read. These are explicitly reconstructed workloads, not a claim to reproduce identical historical commands.

Input: `{"i":0,"n":0.<L zeros>e-400}`. Queries replace `N` with 200, 800, or 3200:

```jq
reduce range(0;N) as $i (. ; .) | .n | tostring | length
[foreach range(0;N) as $i (. ; .; .i)] | length
[while(.i < N; .i += 1) | .i] | length
until(.i >= N; .i += 1) | .n | tostring | length
reduce range(0;N) as $i (. ; to_entries | from_entries) | .n | tostring | length
```

The simple reduce/foreach bodies stay effectively flat in the exploratory sweep. While/until update an unrelated counter, leaving the literal unchanged. The entries round trip exposes a bridged reduce body. Additional exploratory inputs were `0.<L zeros>1` and `<L nines>e400`; both also exhibit slow bridged workloads.

## Local confirmation results

Apple M5 Max, median wall milliseconds; three interleaved repetitions.

- **while**, N=200 / 800 / 3200: baseline 6.0 / 7.6 / 11.8 ms; lossless 224.7 / 864.6 / 3727.3 ms.
- **until**, N=200 / 800 / 3200: baseline 5.8 / 6.4 / 9.4 ms; lossless 137.7 / 557.7 / 2111.2 ms.
- **reduce_entries**, N=200 / 800 / 3200: baseline 6.3 / 7.7 / 15.8 ms; lossless 273.9 / 1088.9 / 4415.4 ms.

At N=800, increasing the zero run from 20,000 to 200,000 to 1,000,000 digits gives lossless while times of 90.5 / 864.6 / 4424.6 ms, until 58.1 / 557.7 / 2625.3 ms, and entries-reduce 116.9 / 1088.9 / 5498.2 ms. This is consistent with iteration-times-literal-length work. All 15 confirmation candidate outputs matched pinned jq.

## Fleet confirmation: ARM64 and x86_64

Both fleet runs completed the same 30-case zero-mantissa matrix (20 / 20,000 / 200,000 / 1,000,000 zeros; five loop shapes; 200 / 800 / 3200 iterations for the 200,000-zero input). Three interleaved repetitions per row. All 30 candidate outputs matched pinned jq on each host. The 54-case boundary/filter sweep also passed on each candidate; each baseline had the same 29 mismatches.

At 200,000 zeros and 3,200 iterations, median milliseconds (baseline → lossless):

- **Apple M4 Pro**: reduce 3.97 → 4.93; foreach 4.34 → 4.46; while 10.68 → 3366.98; until 7.93 → 2049.17; reduce_entries 13.17 → 4096.26.
- **AMD Ryzen 9 7950X**: reduce 3.19 → 4.37; foreach 3.39 → 3.29; while 10.98 → 4031.20; until 7.07 → 2679.97; reduce_entries 12.66 → 5271.50.

Lossless size scaling at 800 iterations (20,000 / 200,000 / 1,000,000 zeros), milliseconds:

- **Apple M4 Pro**: while 90.1 / 851.4 / 4191.0; until 55.6 / 520.3 / 2538.3; reduce_entries 109.3 / 1045.7 / 5087.0.
- **AMD Ryzen 9 7950X**: while 98.2 / 1056.5 / 4714.4; until 58.6 / 681.5 / 3502.3; reduce_entries 118.4 / 1323.8 / 6702.6.

Peak RSS for the collecting while query was 12.44 → 697.75 MB on M4 Pro and 13.12 → 653.26 MB on Ryzen (decimal MB; Linux KiB converted to bytes). The streaming control stayed at 10.63 → 11.35 MB and 10.92 → 12.35 MB respectively. Until also stayed near 11–12 MB. Both architectures therefore reproduce repeated CPU work and the separate collecting-memory amplification.

These are within-host A/B findings, not a CPU ranking. The lossless candidate is substantially slower on the affected paths on both platforms; startup noise cannot explain the seconds-versus-milliseconds gap. Small differences in simple reduce/foreach timings are not interpreted as meaningful performance changes.

Reproduction: build the archived baseline with `cargo build --release --features cli --target-dir <scratch>/target`, copy the CLI to `<scratch>/base`, apply `candidate.patch`, rebuild and copy to `<scratch>/candidate`. Run `python3 fleet.py <scratch> <pinned-jq>`, `python3 variants.py <scratch> <pinned-jq>`, and `python3 differential.py <scratch> <pinned-jq>`. ARM used `/usr/bin/jq` 1.7.1-apple; Ryzen used the official jq 1.7.1 `jq-linux-amd64` release binary in `/tmp/3025-jq-1.7.1`, rather than its installed 1.6/1.8.1 versions.

Source archive SHA-256: `0a61b85e6c4acc470fddda881f9ea256eea685fffb6f0d3ace82bb83f5212158`. Candidate patch SHA-256: `0b6d7f80440c261de2fa1b8e4e918c931d08f7e00c2c485285c42e2a11ae1e9f`. Linux oracle SHA-256: `5942c9b0934e510ee61eb3e30273f1b3fe2590df93933a93d7c58b81d19c8ff5`.

## Correctness

A 54-case differential sweep compared 256/257/300-byte integer and decimal literals across the nine filters from #3025 against `/usr/bin/jq` 1.7.1-apple. The baseline mismatched 29 cases; the candidate mismatched zero. This includes exact comparison against `1e300`, path predicates, entry transformations, deletion, and update assignment.

The loop confirmation harness also captures pinned jq output for each query. The zero-mantissa query produces tostring length 9 in jq/candidate versus 1 in the capped baseline for reduce-to-value/until cases. Those baseline and candidate timings compare different semantics; the baseline is a historical cost reference, not a correct alternative. The while count agrees, but baseline agreement on that projection does not mean its carried literal was preserved.

Six additional yq v4.53.3 checks (native, entries, and with_entries at 257 and 200,000 zeros) all matched with the candidate; the baseline failed the four bridged cases. These are focused checks, not full yq conformance.

## Extreme-value variants: separate native formatting mismatches

Both hosts completed another 30 cases using `0.<200,000 zeros>1` (named `finite` by the harness, but its parsed f64 underflows) and `<200,000 nines>e400`. All processes succeeded, but 18 of these 30 candidate outputs differ from jq on each host. Do not count this variant sweep as correctness-passing.

Native `.n | tostring | length` proves these are pre-existing and independent of reindexing, identically on both hosts:

- Underflow literal: pinned jq prints 9; both baseline and candidate print 200003.
- Overflow literal: pinned jq prints 200009; both baseline and candidate print 23.

The nine affected rows per literal are reduce-to-value, until, and entries-reduce at three iteration counts. While/foreach project only counts, so their matching outputs do not establish literal formatting parity. The bridge patch preserves the source internally but does not repair these final-formatting discrepancies. Record these repros for separate triage; they must not be hidden by the performance result.

At 3,200 iterations, the candidate times in seconds (underflow / overflow) were:

- **Apple M4 Pro**: while 3.431 / 3.585; until 2.083 / 2.247; reduce_entries 4.164 / 4.512.
- **AMD Ryzen 9 7950X**: while 4.051 / 4.207; until 2.678 / 2.865; reduce_entries 5.271 / 5.654.

These variants corroborate the expensive paths, but their differing semantics preclude describing their timings as a correctness-preserving optimization comparison.

## Memory: a distinct second cost

At 3,200 iterations and 200,000 zeros, macOS `time -l` measured peak RSS of 12,533,760 bytes baseline versus 700,153,856 bytes candidate for the collecting while query. Until was 10,829,824 versus 13,598,720 bytes.

The streaming control `while(.i < 3200; .i += 1) | .i | select(. == 3199)` produced 3199 on both binaries and used only 10,928,128 / 13,582,336 bytes, but the candidate still took 3.39 seconds. Thus retained output copies explain the collecting memory blowup; repeated bridge work remains even without that retention.

This makes cheap sharing of unchanged literal payloads worth evaluating for the collecting route, alongside owned-state execution for CPU. It is evidence against treating either change alone as the complete solution. Also inspect why this collecting pipeline retains full states before projecting `.i`, and whether projection can stream without changing evaluation/control ordering.

## Profile and implementation direction

A two-second macOS `sample` of the lossless while workload recorded 1669 worker-thread samples. Within `loop_step_generic -> LoopState::fork -> eval_each_owned`, approximately 745 samples (45%) were in `JsonIndex::build`, 815 (49%) in evaluation dominated by `eval_compound_assign` materializing the document, and 107 (6%) in serialization. Materialization includes number validation, decimal-to-f64 parsing, and document gap scanning. These are approximate sample shares from one run, not benchmark deltas.

This identifies a repeated full-document bridge on the `.i += 1` update, not just a costly clone of `Box<str>`. Changing it to shared text would leave most sampled work in place. Caching the whole document is also insufficient on this workload: `.i` changes every iteration, so a whole-state cache invalidates each step.

Recommended next implementation step: factor owned-state assignment/update execution so the existing path resolution, RHS/fanout/control semantics, and mode-specific behavior can operate on the already-owned tree. Preserve unchanged literal nodes and their parsed representation. Exercise it through the shared owned-evaluation entry points used by loops, not special cases in each loop. Add owned entry conversion where the measured `to_entries | from_entries` workload still bridges. Evaluate shared literal payloads or earlier streaming projection for the independently measured collecting-memory cost. Avoid a wholesale evaluator rewrite or public `NumberLiteral` representation change until measured necessity is established.

An implementation must retain jq's original-input RHS semantics and yq's different assignment behavior, multiple outputs, empty results, errors/halt/break, path identity, and invalidation after actual writes. A narrow native implementation needs fallback parity tests and end-to-end reference tests before expanding coverage.

## Route coverage audit

`test_reindex_bridge_identity_predicate_agrees_1909` explicitly asserts that an over-cap literal is not identity; change that assertion and retain actual round-trip checks in both modes. `test_path_non_navigable_falls_back_when_reindex_is_not_identity_3122` uses a 257-byte literal specifically to force fallback; after this fix it no longer tests that route. Preserve it as a fidelity case and separately force the bridge via the existing `eval_owned_input_bridge` test seam or an appropriate constructed NaN-literal non-identity fixture. CLI tests around #2925 and parity tests also mention the removed cap and require route intent review. Do not mechanically replace all expectations and assume equivalent coverage.

## Remaining work

- Implement and measure the owned-state bridge reduction; the spike does not implement that optimization.
- Run the broader yq/shared-bridge and regression suites when producing the real fix. #3040 remains separate.
- Use the benchmark guide's existing investigation thresholds (>5% end-to-end, >10% memory) for normal workloads; these are investigation triggers, not permission to lose correctness. For the long-literal loop, also require removal of avoidable iteration-times-literal-length reindexing. An exact final budget should follow the lossless optimized measurements.

Reproduction scripts and raw evidence are in `.ai/scratch/3025-spike/`, with fleet results in its `m4-pro/` and `ryzen/` directories. The two-architecture measurement spike is complete; implementation and its final performance budget remain work for the fix. No full tests were run for the disposable candidate; correctness evidence is the focused CLI differential sweep. Production source is unchanged.
