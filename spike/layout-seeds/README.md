# Spike: layout-randomized A/B (2026-11-27)

Not for merge. The question: can an A/B that varies only the code layout tell a real change from a placement effect? #7933 found a +10-12% wall-clock slowdown at equal instruction counts, and `docs/guides/benchmarking.md` says layout alone moves rows by ±5%.

## Method

- **The binaries.**
  - A = `e43c3f9c4` and B = `6362c4bc4` (#7933's pair). B differs from A mainly by #7927's regenerated `scripts/link-order/symbols.txt`.
  - C = A plus one planted extra pass over `ascii_upcase`'s result. That is a known real cost: +2.80% instructions on `strings_upcase` at 1 MB (A 112,398,939 Ir, B 112,403,959, C 115,544,319).
  - All three are built with `--features cli,simd,regex,serde`, fat LTO, one codegen unit. Each is linked through this branch's `scripts/link-order/link.py` with its own tree's `symbols.txt`, and the link is saved.
- **The layouts.** `scripts/link-order/relink.py` links each saved link again, in seconds:
  - unseeded, which is the control;
  - under 8 seeds of lld's `--randomize-section-padding` (`pad`, Linux only);
  - under 8 seeded permutations of the ordered block (`shuf`, both platforms).
- **Re-link control.** On Linux the unseeded re-link is byte-identical to cargo's binary. On macOS `__text`, `__const` and `__eh_frame` are byte-identical, but `__LINKEDIT` is 1 MB larger, because the saved objects' paths are longer.
- **Timing.** `scripts/layout-ab.py` runs all 28 binaries once per repetition, in a shuffled order. Before timing, it checks that every binary prints the same bytes and exits 0.
  - Each comparison uses the median per binary and is paired by seed: `B.sK / A.sK - 1`, then the mean with a 95% t interval over the 8 seeds.
  - `Ap` is cargo's own A binary, so `Ap/A` is the A/A' control.
- **Machines.** terminus (Ryzen 9 7950X, Linux) and johns-mac-mini (Apple M4), each in one exclusive `benchlock` hold scheduled by the machine coordinator. Scripts: `build.sh`, `build-mac.sh`, `time.sh`, `cap.py`. Logs: `time-*.log`, `STATUS-*.txt`. Raw times: `res/*.csv`.

## Results

**terminus, `strings_upcase`.** B vs A is #7933's slowdown. C vs A is the planted +2.8% Ir.

| Size   | Seeds | Ap/A (A/A') | Plain B/A (one link each) | Paired B/A over seeds | Plain C/A | Paired C/A over seeds | A's spread over seeds (range) |
|--------|-------|-------------|---------------------------|-----------------------|-----------|-----------------------|-------------------------------|
| 1 MB   | pad   | +1.29%      | **+4.70%**                | +0.20% ± 2.19%        | +2.23%    | **+2.94% ± 0.85%**    | 4.66%                         |
| 10 MB  | pad   | +1.02%      | **+5.83%**                | +0.88% ± 2.41%        | +2.52%    | **+3.94% ± 0.88%**    | 5.00%                         |
| 100 MB | pad   | -0.47%      | **+5.14%**                | +1.24% ± 2.27%        | +1.66%    | **+4.08% ± 0.85%**    | 4.51%                         |
| 1 MB   | shuf  | +0.03%      | **+3.76%**                | -0.31% ± 1.26%        | +1.35%    | **+2.60% ± 1.23%**    | 4.90%                         |
| 10 MB  | shuf  | +0.46%      | **+5.21%**                | +0.39% ± 1.74%        | +1.96%    | **+3.38% ± 1.30%**    | 6.35%                         |
| 100 MB | shuf  | -0.03%      | **+5.26%**                | +0.14% ± 1.28%        | +1.37%    | **+3.25% ± 1.18%**    | 5.95%                         |

- A plain A/B (one link each, interleaved, A/A' within 1.5%) reports B as a 4-6% slowdown on every row, as #7933 did on this machine.
- Paired over layouts, B's interval includes 0 on all six rows, and its per-seed values change sign (-3.9% to +4.4%). That points to placement, not work, which is #7933's own reading.
- The planted cost's interval excludes 0 on all six rows. The plain A/B under-reports it (+1.4% to +2.5% for a +2.8% Ir change), because one unlucky layout hides part of it.
- Placement alone moves one fixed binary by 4.5-6.4% (range over 8 seeds) on this row. That is the size of #7933's reported slowdowns.

**johns-mac-mini, shuffle seeds only.** ld64 has no padding option.

| Row                    | Ap/A (A/A') | Plain B/A | Paired B/A     | Plain C/A | Paired C/A         |
|------------------------|-------------|-----------|----------------|-----------|--------------------|
| `ndjson_slurp` 1 MB    | +1.58%      | -1.07%    | -0.30% ± 0.69% | +0.06%    | -0.08% ± 0.62%     |
| `ndjson_slurp` 10 MB   | -0.13%      | -0.08%    | -0.09% ± 0.28% | -0.65%    | +0.13% ± 0.42%     |
| `seq_slurp` 1 MB       | +1.54%      | -0.66%    | -0.46% ± 0.73% | -0.38%    | -0.10% ± 0.47%     |
| `seq_slurp` 10 MB      | +0.49%      | -0.13%    | -0.35% ± 0.91% | +0.04%    | +0.02% ± 0.17%     |
| `strings_upcase` 1 MB  | +2.25%      | -0.13%    | -0.74% ± 0.91% | +3.48%    | **+3.05% ± 1.22%** |
| `strings_upcase` 10 MB | -0.59%      | -3.39%    | -0.73% ± 1.78% | +4.31%    | **+4.60% ± 1.38%** |

- **The A/A' control fails on three 1 MB rows** (+1.58%, +1.54%, +2.25%, against the coordinator's 1.5% bar). So the plain B/A on those rows cannot be read. On macOS, A' also differs from A by the 1 MB larger `__LINKEDIT`, so this is not a pure A/A' on a startup-bound row.
- **#7933's M4 Pro slowdown (+3-6% on the `-s` rows) does not reproduce here, plain or paired.**
  - These builds lack `bench-runner`, which `succinctly dev compare`'s build adds. That alone is a different layout, and could be the whole effect.
  - This machine is an M4, and #7933 measured an M4 Pro.
- The planted cost reads 0 on the `-s` rows, where `ascii_upcase` never runs, and is detected on both `strings_upcase` rows.

**How many seeds.** Every subset of the 8 seeds was re-analysed. The table gives the share of subsets whose interval excludes 0.

| Seeds | terminus pad: C detected / B flagged | terminus shuf: C detected / B flagged | mini shuf `strings_upcase`: C detected / B flagged |
|-------|--------------------------------------|---------------------------------------|----------------------------------------------------|
| 3     | 79-95% / 2-4%                        | 41-62% / 2-9%                         | 61-70% / 5-12%                                     |
| 4     | 100% / 3-16%                         | 50-79% / 1-7%                         | 90-100% / 7-23%                                    |
| 6     | 100% / 0-4%                          | 100% / 0%                             | 100% / 0-25%                                       |
| 8     | 100% / 0%                            | 100% / 0%                             | 100% / 0%                                          |

At 8 seeds the interval is ±0.85-1.4% for C on the strings rows, and the cost is about 28 binaries × 15 reps: under 2 minutes for a 10 MB row.

## Conclusions

1. **The method works.** Pairing A and B over the same seeded layouts separates a +2.8% real cost from a 5% placement effect, on both machines, where a plain interleaved A/B (with a passing A/A') reports the placement effect as a regression and under-reports the real one.
2. **Use 8 seeds.** Padding seeds (Linux) converge faster than shuffle seeds: 4 are already enough with padding, while shuffle needs 6.
3. **It is cheap.** Re-links take seconds from one saved link, so a build costs no more than before.
4. **#7933.** On the 7950X, `strings_upcase`'s +10-12% (+4-6% in this run) is placement. Under layout variation it is indistinguishable from zero (±1.3-2.4%). On the M4, the `-s` rows show nothing in this build configuration.
5. **Limits.**
   - Padding and shuffling do not reproduce every way a real code change moves placement. A change that moves the hot block itself, or its alignment against large pages, may not be covered by either perturbation.
   - macOS has only the shuffle.
   - The method measures the change's effect averaged over layouts, not the effect in the one layout that ships.
