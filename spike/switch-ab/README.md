# Spike: runtime switch vs two binaries (2026-11-27)

Not for merge. Builds on `spike-layout-seeds` (its `link.py` seeds, `relink.py` and `scripts/layout-ab.py`).

The question: if a behaviour change ships behind a runtime switch, does an A/B of one binary with the switch on and off measure the change without the code-placement term that a two-binary A/B carries?

## Method

- **M** = `f884a5148`, the reader prototype's base, on main.
- **P** = M plus the reader prototype (`design-input-reader` at `5884ba609`, applied as one diff).
- **P0** = P's own binaries run with `SUCCINCTLY_READER_MODEL=0`, which restores main's reading.
- Both are built and saved as in the layout spike, then re-linked unseeded and under 8 seeds of lld's `--randomize-section-padding`. Both re-link controls are byte-identical to cargo's binaries.
- That gives 27 arms (M, P, P0, each unseeded and under 8 seeds), interleaved per repetition, with outputs gated identical. Machine: terminus (Ryzen 9 7950X), in one exclusive hold.
- **The reader model never runs on these rows.**
  - It enters on a NUL, a broken BOM mark or a fault.
  - The probe's three trigger variants (a NUL at EOF, a NUL mid-file, a leading `0xEF`) all failed identically, exit 5, in M, P and P0, so none reached the model.
  - Instruction counts on 1 MB inputs: P = M + 0.013%, and P0 = P + 0.003%.
  - So this is the no-op case: the true P/M, P0/M and P/P0 are all about 0.

## Results

| Row (10 MB unless noted)           | Plain P/M | Plain P0/M | Plain P/P0 | Paired P/M     | Paired P/P0    | Per-seed sd P/M | Per-seed sd P/P0 |
|------------------------------------|-----------|------------|------------|----------------|----------------|-----------------|------------------|
| `identity_compact` (users)         | -0.37%    | +1.66%     | -2.00%     | -0.69% ± 2.10% | -0.51% ± 1.59% | 2.52%           | 1.91%            |
| `ndjson_identity`                  | -3.07%    | -1.37%     | -1.72%     | -0.41% ± 1.19% | -0.05% ± 0.30% | 1.42%           | 0.36%            |
| `ndjson_slurp`                     | -0.14%    | +0.07%     | -0.22%     | -0.58% ± 0.68% | -0.01% ± 0.29% | 0.81%           | 0.35%            |
| `ndjson_inputs_reduce`             | +2.12%    | +2.34%     | -0.21%     | -0.03% ± 1.46% | -0.01% ± 0.20% | 1.75%           | 0.24%            |
| `identity_compact` (users, 1 MB)   | +1.68%    | +2.33%     | -0.64%     | +0.54% ± 1.65% | -1.08% ± 1.23% | 1.98%           | 1.47%            |

- **The ndjson rows.**
  - Within one layout, the same-binary comparison (P/P0) varies 2-7x less from seed to seed than the two-binary one (P/M): a per-seed sd of 0.24-0.36% against 0.81-1.75%.
  - Paired over 8 seeds, it pins the no-op change to ±0.2-0.3%. The two-binary pairing needs ±0.7-1.5% for the same conclusion.
  - That is the claim the spike set out to check: a switch removes the layout term.
- **The users rows** do not show it. P/P0 varies 1.5-1.9% per seed, close to P/M's 2.0-2.5%, and the single unseeded P/P0 reads -2.00%, for one binary with one environment variable more.
  - That fits Mytkowicz et al. (ASPLOS '09): the environment's size moves the initial stack, and its alignment can move a row by a few percent.
  - The ndjson rows, which spend their time in the reader loop, are not sensitive to it.
- **A single unseeded comparison is unreliable either way.** Plain P/M reads -3.07% and +2.12% on rows where nothing changed, and plain P/P0 reads up to -2.00%.

### Follow-up: the switch in both arms, at equal length

This is the same 27 arms and rows in a second exclusive hold. P now runs with `SUCCINCTLY_READER_MODEL=1` and P0 with `=0`, so the two environments differ in one byte, not one variable (`time-eq.sh`, `time-eq.log`, `res/eq-*.csv`).

| Row (10 MB unless noted)           | Plain P/P0, one arm set | Plain P/P0, both set | Paired P/P0, both set | Per-seed sd P/P0, one / both | Per-seed sd P/M, both run |
|------------------------------------|-------------------------|----------------------|-----------------------|------------------------------|---------------------------|
| `identity_compact` (users)         | -2.00%                  | +0.61%               | -0.55% ± 0.95%        | 1.91% / 1.14%                | 1.29%                     |
| `ndjson_identity`                  | -1.72%                  | -0.71%               | +0.06% ± 0.50%        | 0.36% / 0.60%                | 1.83%                     |
| `ndjson_slurp`                     | -0.22%                  | +0.09%               | +0.06% ± 0.36%        | 0.35% / 0.44%                | 1.04%                     |
| `ndjson_inputs_reduce`             | -0.21%                  | -0.30%               | -0.14% ± 0.18%        | 0.24% / 0.22%                | 1.99%                     |
| `identity_compact` (users, 1 MB)   | -0.64%                  | -2.14%               | +0.40% ± 1.89%        | 1.47% / 2.27%                | 1.72%                     |

- With the switch set in both arms, a single unseeded same-binary A/B reads within 1.5% on every 10 MB row, including the two that failed the A/A' bar before.
- The users 10 MB row's per-seed spread drops from 1.91% to 1.14%. That is better, but still close to the two-binary 1.29% on that row, so the users rows carry run-to-run noise beyond the environment's size.
- The 1 MB users row (3.5 ms, mostly start-up) stays noisy either way. In this run the two-binary pairing even reads +1.68% ± 1.44%, an interval that excludes 0, for a no-op change. That is the false positive a startup-bound row gives at 8 seeds.

## Conclusions

1. **A runtime switch is a better A/B than two binaries, but not a free one.** It removes the layout term, which is the larger term on rows where placement matters. On ndjson rows it is 4-7x tighter per layout.
2. **Put the switch in both arms' environments with values of equal length** (for example `SUCCINCTLY_READER_MODEL=1` against `=0`), never as present against absent. The follow-up confirms this matters: on the users 10 MB row, the single comparison moved from -2.00% to +0.61% and the per-seed spread from 1.91% to 1.14%. It does not make the users rows as tight as the ndjson rows; something else, run-to-run noise on that shape, remains.
3. **Still pair it over a few layouts.** A switch A/B under one layout measures the change in that one layout; the layout spike's seeds measure it across layouts. The two combine: run both arms of the switch under each seed.
4. **What this did not test:** a row where the model runs. None of the probed trigger inputs reached it; the reader draft's corpus (`.ai/scratch/bughunt/lanes/tf19/`) has inputs that do.

5. **Rows under about 5 ms stay unreadable** with either method at 8 seeds: the users 1 MB row gave a false positive in the follow-up.

Scripts: `build.sh` (box build phase), `time.sh`, `time-eq.sh` and `rows.txt` (timing), `cap.py` (wall cap). Logs: `STATUS.txt`, `time.log`. Raw times: `res/*.csv`.
