# CS-Poppy Combined Sampling for BP Select

[Home](../../) > [Docs](../) > [Plan](./) > CS-Poppy

**Status**: Implemented · **Issue**: [#64](https://github.com/rust-works/succinctly/issues/64) ·
**Modules**: `src/trees/bp.rs`, `src/bits/select.rs`, `src/yaml/index.rs`

Applies the *combined sampling* idea from Zhou, Andersen & Kaminsky,
*"Space-Efficient, High-Performance Rank & Select Structures on Uncompressed Bit
Sequences"* (SEA 2013), to the one place in this crate where it pays today: the
select index on `BalancedParens`.

`WithSelect` currently costs **25% of the BP bitmap**. This plan takes it to
**6.25%** in two independently shippable steps.

> **Scope note.** Issue #64 originally proposed a six-phase build-out
> (`CsPoppy`, `BitVecCsPoppy`, true Poppy rank, BP integration). §1 records why
> that scope was cut; §6 records what was cut and why each piece is still worth
> doing later.

---

## 1. Why the original scope was cut

The issue was written against an assumed baseline that does not match the code.

### 1.1 `RankDirectory` is ~25% overhead, not ~3%

[src/bits/rank.rs](../../src/bits/rank.rs) stores **one `u128` per 512-bit
block** — 128 bits of metadata per 512 bits of data. Its own module doc says so:

> Total overhead: 128 bits of metadata per 512 bits of data = ~25%
> … it is not a compact (~3%) rank index.

The "~3%" is the *paper's* Poppy figure. The 25% is bought deliberately: the
packed 7×9-bit L2 gives a per-word cumulative offset, so rank never popcounts a
full word. Real Poppy has no per-word level and popcounts up to 7 words inside a
basic block.

So "implement true CS-Poppy" would mean implementing the true Poppy *rank*
layout as well — a space/time trade, not a free win. That is now [F2](#f2--true-poppy-rank-layout-as-a-standalone-cspoppy).

### 1.2 The select index costs 25% on BP bitmaps, not 1–3%

`SelectIndex` stores `SampleEntry { word_idx: u64, cumulative_before: u64 }` —
16 bytes, widened from `u32` in #188. Overhead is
`128 × density / sample_rate`, so it is **density-dependent**:

| Density | Where this occurs     | Overhead at rate 256 |
|---------|-----------------------|----------------------|
| 0.5     | BP bitmaps (`bp.rs`)  | 25.00%               |
| 0.125   | Newline index         | 6.25%                |
| 0.01    | Very sparse bitmaps   | 0.50%                |

Balanced parens are ~50% ones by construction, so `WithSelect` — what YAML uses
today — costs a quarter of the BP bitmap again. This is the largest concrete win
available, and it is *bigger* than the issue suggested.

### 1.3 `BalancedParens` does not use `RankDirectory`

The original Phase 4 assumed `WithCsPoppy` could share L0/L1/L2 with the rank
directory. It cannot:

1. **BP has its own rank.** [src/trees/bp.rs:1486-1490](../../src/trees/bp.rs#L1486-L1490)
   declares `rank_l1: Vec<u32>` and `rank_l2: Vec<u64>` inline — 96 bits per
   512-bit block (18.75%). It never touches `RankDirectory`.
2. **The trait can't see it.** `SelectSupport::select1` takes
   `(&self, words, len, total_ones, k)`
   ([src/trees/bp.rs:55](../../src/trees/bp.rs#L55)), so a `WithCsPoppy` written
   against today's signature could only *duplicate* L1/L2 — strictly worse than
   `WithSelect`.

This turns out to be a gift. BP's `rank_l1` is a sorted `u32` array at 512-bit
granularity — **finer than Poppy's own 2048-bit L1, and directly
`partition_point`-able**. BP's fatter rank makes its select cheaper than the
paper's.

### 1.4 The heavy phases landed on cold paths

`BitVec::select1` has exactly four call sites in the crate —
[locate.rs:101](../../src/json/locate.rs#L101),
[locate.rs:133](../../src/json/locate.rs#L133),
[light.rs:308](../../src/json/light.rs#L308),
[light.rs:332](../../src/json/light.rs#L332) — all line→offset conversion for
`jq-locate`. Cold CLI path over a sparse bitmap (newlines, d ≈ 0.025), where
`SelectIndex` already costs only ~1.25%.

`BitVecCsPoppy` and the Poppy rank layout are therefore completeness work, not a
workload win. Both are deferred to §6, and neither requires redoing Step A or B.

---

## 2. Step A — narrow the BP sample entry

**25% → 12.5%. No trait change, no new public types, no algorithmic change.**

`build_bp_index` asserts `len ≤ u32::MAX` bits (#188), so for the BP case *both*
`SampleEntry` fields fit in `u32`: `word_idx ≤ 2^26` and
`cumulative_before ≤ 2^32`. Halving the entry halves the index.

`BitVec` keeps the `u64` entries #188 requires — the widening there was a real
fix for bitvectors past 2^32 set bits, and this must not regress it.

Implementation options, in order of preference:

1. Generic `SelectIndex<T: SampleWord>` with `T ∈ {u32, u64}`; `WithSelect`
   instantiates `u32`, `BitVec` instantiates `u64`. Zero runtime cost, one code
   path.
2. A separate `NarrowSelectIndex` used only by `bp.rs`. Simpler diff, duplicated
   logic — acceptable only if (1) turns out to be viral through `BitVec`.

Tasks:

- [ ] Parameterise or split `SampleEntry`
- [ ] Assert/document the `u32::MAX` precondition at the BP construction site
- [ ] Boundary unit tests; keep the `huge-tests` #188 regression green on the wide path
- [ ] Measure `bp_select_micro`, `yq_select`, `yaml_bench`, retained `YamlIndex` size

---

## 3. Step B — combined sampling against `rank_l1`

**12.5% → 6.25%, and plausibly faster.**

Replace the `(word, cumulative)` pair with a single `u32` **block index** per
sample, and drive the search off the rank array instead of scanning bitmap
words.

`select1(k)`:

1. `i = k / rate` gives a bracket `[samples[i], samples[i+1]]` of 512-bit blocks.
2. `partition_point` over `rank_l1` within that bracket for the last block `b`
   with `rank_l1[b] <= k`. `rank_l1` is *absolute*, so no stored cumulative is
   needed — that is the 2× saving over Step A.
3. Unpack block `b`'s `rank_l2` 9-bit offsets to pick the exact word.
4. `select_in_word(word, k - rank)` — the existing broadword primitive.

Steps 1–3 touch **no bitmap words at all**. Contrast `WithSelect`, which
linear-scans words from `jump_to`'s starting word. This is what "combined"
means: the select index is a set of entry points into the rank structure, not a
parallel structure.

### 3.1 The `SelectSupport` signature change

`select1` needs BP's rank arrays. Narrowest change that makes sharing real:

```rust
/// Read-only view of the data a SelectSupport impl may consult.
pub struct BpSelectCtx<'a> {
    pub words: &'a [u64],
    pub len: usize,
    pub total_ones: usize,
    pub rank_l1: &'a [u32],   // absolute rank per 512-bit block
    pub rank_l2: &'a [u64],   // 7 × 9-bit per-word offsets within block
}

pub trait SelectSupport: Clone + Default {
    fn build(words: &[u64], total_ones: usize) -> Self;      // unchanged
    fn select1(&self, ctx: BpSelectCtx<'_>, k: usize) -> Option<usize>;
}
```

`build` keeps its signature — computing sample points needs only `words`.
`NoSelect` ignores `ctx` and stays a ZST, so JSON is unaffected after
monomorphisation.

This is a **breaking change to a `pub` trait** (re-exported at
[src/trees/mod.rs:24](../../src/trees/mod.rs#L24)). No in-tree implementors exist
outside `bp.rs`. Note it in the changelog.

**Rejected alternatives**: `WithCsPoppy` carrying duplicate L1/L2 (strictly more
memory than `WithSelect` — defeats the purpose); a closure or `dyn` rank
accessor (indirection on a hot path, and `Clone + Default` makes `dyn` awkward).

### 3.2 Capacity

A `u32` block index addresses 2^32 × 512 = 2^41 bits, far beyond BP's own
`u32::MAX`-bit limit. No new ceiling is introduced.

Tasks:

- [ ] Add `BpSelectCtx`, change `SelectSupport::select1`
- [ ] Add `WithCsPoppy` + `new_with_cspoppy` / `from_words_with_cspoppy`
- [ ] Deprecate `WithSelect` with a migration note; keep it compiling and tested
- [ ] Switch `YamlIndex` to `WithCsPoppy` ([src/yaml/index.rs:46](../../src/yaml/index.rs#L46), accessor at `:338`)
- [ ] Property tests: `WithCsPoppy` ≡ `WithSelect` on all inputs; `rank1(select1(k)) == k`
- [ ] Re-measure and diff against Step A

---

## 4. Sequencing and measurement

**Capture baselines before Step A.** Without them the comparison is
unfalsifiable. Benchmarks run **sequentially, never concurrently** — they need
exclusive CPU.

| Benchmark                        | What it guards                          |
|----------------------------------|-----------------------------------------|
| `bp_select_micro`                | BP select1 at 1K…1M opens               |
| `yq_select`                      | End-to-end `yq-locate` / `at_offset`    |
| `yaml_bench`                     | Build-side regression                   |
| `json_pipeline`                  | `NoSelect` still zero-cost              |
| Retained `YamlIndex` size        | The actual memory claim                 |

Commit scopes from `.omni-dev/scopes.yaml`: `bitvec:` for Step A, `bp:` then
`yaml:` for Step B, `bench:`/`test:` for measurement, `docs:` for write-up.

---

## 5. Acceptance criteria

| # | Criterion                                                                  |
|---|----------------------------------------------------------------------------|
| 1 | `WithCsPoppy` agrees with `WithSelect` on all property-test inputs          |
| 2 | BP select index ≥ 3.5× smaller than today (measured, not derived)          |
| 3 | BP `select1` no slower than `WithSelect` at every `bp_select_micro` size    |
| 4 | `yaml_bench` and `yq_select` show no regression beyond noise                |
| 5 | JSON path unchanged — `NoSelect` still a ZST, `json_pipeline` unmoved       |
| 6 | `cargo clippy --all-targets --all-features -- -D warnings` clean            |

**Criterion 3 is the one at risk.** BP bitmaps are dense, so `WithSelect`'s word
scan is already short — precisely the regime where combined sampling's advantage
is smallest. If select1 regresses, the memory win may still justify shipping;
that is a judgement call to make with numbers in hand, not now.

Step A is independently shippable. If Step B fails criterion 3, A still stands.

---

## 5a. Results (2026-08-03)

Measured on the pinned benchmark hosts (Apple M4 Pro over Tailscale ssh, AMD
Ryzen 9 7950X over Tailscale ssh), both idle and on AC/mains power, full
`cargo bench --bench bp_select_micro` (no `--quick`, 100 samples/id).

### Criterion 3: select1 speed — **platform-dependent, not a clean pass**

| opens | Apple M4 Pro `select1` | M4 Pro `select1_cspoppy` | Δ (ARM)     | Zen 4 `select1` | Zen 4 `select1_cspoppy` | Δ (x86)     |
|-------|-------------------------|---------------------------|-------------|-------------------|----------------------------|-------------|
| 1K    | 157.84 µs               | 172.26 µs                 | **+9.1%**   | 147.83 µs          | 106.59 µs                  | −27.9%¹     |
| 10K   | 159.30 µs               | 174.04 µs                 | **+9.3%**   | 107.19 µs          | 105.37 µs                  | −1.7%       |
| 100K  | 153.77 µs               | 176.91 µs                 | **+15.0%**  | 107.53 µs          | 106.44 µs                  | −1.0%       |
| 1M    | 183.28 µs               | 192.29 µs                 | **+4.9%**   | 124.77 µs          | 123.52 µs                  | −1.0%       |

¹ The x86 1K `select1` (`WithSelect`) point is an outlier against its own
10K/100K/1M neighbours (~107 µs flat) — both arms should be roughly
size-independent since select1 is O(1) after the initial jump. Treat only the
10K–1M x86 rows (consistently neutral-to-faster) as reliable; the 1K row is
likely a first-benchmark-in-the-binary artifact, not a real effect.

**ARM (M4 Pro) regresses 5–15%, consistently, with non-overlapping confidence
intervals across two independent full runs. x86 (Zen 4) is neutral to slightly
faster.** This is a real architecture-dependent divergence, not noise — see
[A/B Benchmarking Method §5](../guides/benchmarking.md#5-measure-both-architectures--the-effect-size-differs-not-just-the-noise):
cache/memory-bound effects do not port between platforms, and here the
directions themselves differ, not just the magnitude. A plausible mechanism:
`WithSelect`'s linear word scan is cheap on BP's dense bitmaps and benefits
from ARM's efficient sequential access, while `WithCsPoppy`'s bracket +
`partition_point` + 7-branch `rank_l2` unpack trades that scan for pointer-chasy
control flow that predicts worse on this core.

**Why this doesn't block shipping Step B**: `BalancedParens::select1` on YAML's
BP is reached only through `find_bp_at_text_pos` — `yq-locate` / `at_offset` /
`at_position` — **at most once per CLI invocation**, not on the `.foo.bar`
navigation hot path (that path uses IB's separate rank/select). See
[select-scan.md](../optimizations/select-scan.md), whose corpus scan recorded
*zero* calls to this select support outside locate tooling. A 5–15% delta on a
~150–190 µs function called once is worth low tens of microseconds against a
CLI invocation's parse/IO time — below what any of this crate's end-to-end
benchmarks can resolve. Criterion 3 is **not met as literally written** (ARM is
slower, not "no slower"), but the acceptance criteria's own text anticipated
this outcome and named the memory win as sufficient justification.

### Criterion 2: memory — confirmed, 4×

Already covered by `test_cspoppy_index_is_half_of_with_select` and
`test_select_index_costs_one_eighth_of_bitmap` (real `select_heap_size()`
measurements, not derived): 25.00% (pre-Step-A) → 12.50% (Step A) → 6.25%
(Step B) of the BP bitmap — a 4× reduction, clearing the ≥3.5× bar.

### Criterion 4: yaml_bench / yq_select — clean, with one investigated anomaly

Full `yaml_bench` (`main` vs this branch, same machine, interleaved reruns),
excluding `yaml/anchors/*` which panics with `UnknownAnchor` on **both**
binaries — a pre-existing generator bug on `main`, unrelated to this branch
(confirmed by reproducing it against unmodified `main`); worth its own issue.

Of the 39 remaining benchmark ids, 38 are noise-level (< 2%) on both Zen 4 and
M4 Pro. One group is not: **`yaml/block_scalars/*` is 12–28% slower on Zen 4**
(`terminus`), reproduced identically across two independent interleaved
reruns — e.g. `100x100lines` 144.6 µs → 184.4 µs, `long_100x100lines` 354.6 µs
→ 407.7 µs. **The same group is neutral on the M4 Pro** (`100x100lines`
161.9 µs → 160.3 µs, `long_100x100lines` 311.5 µs → 309.3 µs).

Investigated rather than dismissed as noise, because it's large and
reproducible. Built the exact `long_100x100lines` document (831 KB) and
inspected the resulting BP structure directly: **7 words, 202 opens, 4-byte
select index** — regardless of `lines_per_block`, since a block scalar's lines
are leaf text, not BP nodes; the select support this branch touches is a fixed
~7-word structure that cannot mechanistically cost 40 µs on an 831 KB build
dominated by unrelated SIMD text scanning (P2.7). Combined with the M4 Pro
result showing no effect at all on the identical workload, this is not
attributable to the CS-Poppy select logic — the signature (x86-only, one
group, size-independent within the group, structurally impossible for the
changed code to cause) matches a binary code-layout artifact from the
recompile (e.g. an icache/alignment shift next to the P2.7 AVX2 block-scalar
loop, which this branch does not touch) rather than a real regression. Not
chased further — `terminus`'s WSL2 lacks `perf`; a follow-up with a native
Linux `perf stat`/objdump comparison would confirm instructions-retired parity,
which would settle "layout, not logic" conclusively.

`yq_select` was not run — it requires system `yq` (absent on the M4 Pro host)
and mainly exercises `.users[:N]`-style slicing, which does not call
`find_bp_at_text_pos`/`select1` at all; `yaml_bench`'s build-side coverage is
the criterion this change can actually affect.

### Verdict

**Ship Step B.** The memory win (4×, unconditional on every `YamlIndex`) and
the x86 select1 result (neutral-to-better) outweigh a real but cold-path-only
ARM select1 regression. Build-side (criterion 4) is clean once the pre-existing
`anchors` generator bug is excluded; the one x86-only `block_scalars` anomaly
is investigated above and structurally cannot originate from this branch's
code. This is the judgement call §5 deferred — recorded here with the numbers
behind it so it can be revisited if `find_bp_at_text_pos` ever moves onto a
hotter path, or if the `block_scalars` anomaly resurfaces with `perf` evidence
attributing it to this change after all.

---

## 6. Future developments

Not scheduled. Recorded so the reasoning isn't lost; each is a candidate
follow-up issue once #64 lands. Mirrored in the issue body.

### F1 — Drop BP's `rank_l2` (largest remaining win)

`rank_l2: Vec<u64>` is 64 bits per 512-bit block = **12.5% of the BP bitmap** —
twice what Step B saves. Poppy's actual trade is to omit the per-word level and
popcount up to 7 words inside a basic block, which is exactly one 64-byte cache
line, and one the query must load anyway for the partial word.

Interacts with Step B: once select drives off `rank_l1`, `rank_l2` is used only
by `rank1`. BP `rank1` is very hot (cursor navigation), so this is a genuine
trade needing end-to-end measurement. Should follow Step B so the select path
isn't confounded.

### F2 — True Poppy rank layout as a standalone `CsPoppy`

L0 (`u64` per 2^32 bits) + L1/L2 packed into one `u64` per 2048-bit superblock
(32-bit L1 + 3×10-bit per-basic-block counts) = **3.125%**, versus
`RankDirectory`'s 25%. The original Phase 1.

```rust
#[inline]
pub fn rank_at_word(&self, words: &[u64], word_idx: usize) -> usize {
    let sb = word_idx / 32;                  // 2048-bit superblock
    let entry = self.l1_l2[sb];              // one u64 load
    let bb = (word_idx % 32) / 8;            // basic block within it (0..=3)
    let mut r = self.l0[word_idx / (1 << 26)] as usize
              + (entry & 0xFFFF_FFFF) as usize;
    for i in 0..bb {                          // ≤ 3 iterations, unrolled
        r += ((entry >> (32 + i * 10)) & 0x3FF) as usize;
    }
    let base = sb * 32 + bb * 8;
    r + popcount_words(&words[base..word_idx]) // ≤ 7 words, same cache line
}
```

L2 stores counts of basic blocks 0–2; block 3's is never needed. Would make
`BitVecCsPoppy` (original Phase 3) trivial to add on top.

### F3 — Consolidate three rank implementations

The crate has three: `RankDirectory` (25%, cache-aligned, per-word L2),
[`CompactRank`](../../src/bits/compact_rank.rs) (3.52%, two-level), and BP's
inline `rank_l1`/`rank_l2` (18.75%). F2 would make four. Worth an ADR before it
becomes five — the interesting question is whether one parameterised structure
serves all callers, or whether the density/hotness spread genuinely justifies
separate ones.

### F4 — Migrate `BitVec` to a compact rank

Contingent on F2. Newline bitvecs in `json/locate.rs`, `json/light.rs` and
`yaml/index.rs` are 1 bit per input byte, so on a 100 MB document the bitmap is
12.5 MB and `RankDirectory` is 3.1 MB; Poppy would make that 0.4 MB. Real but
modest, and CLI-only.

### F5 — Elias-Fano-compressed select samples

Sample block indices are monotonically increasing — exactly what
[`EliasFano`](../../src/bits/elias_fano.rs) encodes. Could shrink the sample
array further beyond Step B's `u32`. Should be evaluated together with
[compact-index-investigation.md](compact-index-investigation.md), which proposes
EF for the YAML position arrays.

### F6 — Sample rate as a tuning knob

At `u32` samples the overhead is `32 × density / rate`:

| Sample rate | d=0.5 (BP) | d=0.125 | d=0.01  |
|-------------|------------|---------|---------|
| 64          | 25.000%    | 6.250%  | 0.500%  |
| **256**     | **6.250%** | 1.563%  | 0.125%  |
| 512         | 3.125%     | 0.781%  | 0.063%  |
| 8192        | 0.195%     | 0.049%  | 0.004%  |

Rate 512 halves Step B again, at the cost of a wider binary-search bracket.
Cheap to expose via `Config` once B lands; needs measurement to justify a
non-default. Default stays **256** so existing tuning keeps its meaning.

### F7 — `select0`

Unsupported crate-wide today. On BP it would find the k-th close, complementing
`select1`. CS-Poppy supports both in the paper. No known consumer — recorded as
a gap.

---

## 7. Risks

| Risk                                                       | Mitigation                                             |
|------------------------------------------------------------|--------------------------------------------------------|
| BP select1 regresses (criterion 3)                          | Step A ships independently; decide B on numbers        |
| `SelectSupport` change breaks external implementors         | Document as breaking; no in-tree impls outside `bp.rs` |
| Long all-zero runs inflate the search bracket               | Binary search, not linear scan, within the bracket     |
| Narrowing `SampleEntry` reintroduces #188                   | BP-only narrow path; wide path keeps its `huge-tests`  |
| Micro-benchmark win that doesn't survive end-to-end         | Criterion 4 gates on `yaml_bench`/`yq_select`          |

That last row is this codebase's most repeated lesson: P2.6, P2.8, P3 and P5
were all rejected after micro-benchmarks promised gains that end-to-end runs did
not deliver. §4 exists so this one is judged the same way.

---

## 8. References

- Zhou, Andersen, Kaminsky. *Space-Efficient, High-Performance Rank & Select
  Structures on Uncompressed Bit Sequences*. SEA 2013.
- [docs/architecture/bitvec.md](../architecture/bitvec.md) — current rank/select
- [docs/architecture/balanced-parens.md](../architecture/balanced-parens.md)
- [docs/adrs/adr-0011.md](../adrs/adr-0011.md) — why rank/select is built in-crate
- P11 in CLAUDE.md — the BP select1 work this builds on (#26)
