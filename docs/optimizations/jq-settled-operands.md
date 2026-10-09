# jq Operand Strategy: Settled vs Sink-Fed Chains

[Home](../../) > [Docs](../) > [Optimizations](./) > jq Settled Operands

**Status: NO CHANGE (investigated, measured, kept as is) — October 2026**

**Issue**: [#4132](https://github.com/rust-works/succinctly/issues/4132) (this measurement) ·
observed while attributing [#3997](https://github.com/rust-works/succinctly/issues/3997) ·
the settled path itself is [#3296](https://github.com/rust-works/succinctly/issues/3296)
([ADR-0025](../adrs/adr-0025.md))

> **TL;DR.** On a chain of `def` calls (`def f: .+1; map(f+f+...+f)`) the settled operand path
> is faster than the sink-fed one from about 8 terms, and up to 1.29x faster at 200, on both
> x86_64 and ARM. It is *not* faster because it does less work: it executes **more**
> instructions (+9% to +18%). The sink-fed path nests its sinks N frames deep, and the deep
> stack misses the data cache. A chain of non-`def` operands does not nest that way and shows
> none of it, and settling those operands anyway is 3-24% *slower*. Nothing was changed.

## Background

`binary_fanout_each` (`src/jq/eval.rs`) and `binary_fanout_each_generic`
(`src/jq/eval_generic.rs`) enumerate an operator's operands through one of two strategies:

- **sink-fed** (the original): each operand delivers its output to the operator's closure from
  inside its own evaluation, so the right-hand call of `fib(n - 1) + fib(n - 2)` runs inside
  its left sibling's output sink.
- **settled** (`settled_operand_strategy` / `settle_then_replay`, #3296): the operand is
  evaluated to completion into a buffer and the buffer is replayed to the sink, so each call
  has returned before its sibling starts. Chosen by `settles_both` only when *both* operands
  are single-valued, effect-free and call a `def`.

The settled path exists for native stack depth (ADR-0025): `fib(24)` answers with it and is
refused without it. Its speed was never examined. #3997 found, while memoising the analysis,
that a build with `settles_both` forced to `false` was *slower* on a chain of `def` calls.

## Method

Release builds (`cgu=1`, fat LTO, `--features cli`) of `ce09e00da` (which contains #3997), in
three variants:

- **settled**: as shipped.
- **sink-fed**: `settles_both` returns `false` unconditionally (the analysis disabled).
- **widened**: an operand settles whenever the purity walk accepts it, `def` call or not.

`succinctly jq -c <query> arr20k.json` over a 20,000-element numeric array; user + system time;
the binaries alternate within each repetition (order rotating), minimum of 7; output hashes
identical across variants before any timing is read; a copy of the settled binary is the noise
control. Measured on `terminus` (Ryzen 9 7950X, one core pinned, load average 1.3) and
`johns-mac-mini` (Apple M4 Pro, AC power). The mini was not strictly idle -- load average about
2.2 from background daemons and two other Claude sessions -- and its control copy read -2.3%
to +1.4% against the settled binary, which is the noise floor for its rows.

## Results

### 1. A chain of `def` calls: sink-fed time over settled time

| Terms | 7950X | M4 Pro |
|-------|-------|--------|
| 2     | 0.94x | 0.91x  |
| 4     | 0.95x | 1.05x  |
| 8     | 1.03x | 1.05x  |
| 16    | 1.09x | 1.03x  |
| 25    | 1.10x | 1.19x  |
| 50    | 1.12x | 1.24x  |
| 100   | 1.20x | 1.29x  |
| 200   | 1.29x | 1.26x  |

Sink-fed wins on 2-term chains (`f+f`: 6% on the 7950X, 9% on the M4 Pro; `def f: .+1; def g:
.*2; f+g` measured 9% on the M4 Pro only) and, on the x86 box, up to 4 terms; on ARM it is
already 5% slower at 4. From 8 terms it loses on both, by a margin that grows with length. A
short chain cannot be moved to the sink-fed path without breaking #3296: `fib(n - 1) + fib(n -
2)` is two terms and must settle.

### 2. It is cache misses, not instructions

The sink-fed path executes fewer instructions at every length, on both architectures.

x86_64, `valgrind --tool=cachegrind --cache-sim=yes`, 2,000 elements (D1 is 32 KB, 8-way):

| Terms | Ir settled | Ir sink-fed | D1 misses settled | D1 misses sink-fed | D1 ratio |
|-------|------------|-------------|-------------------|--------------------|----------|
| 4     | 39.4 M     | 34.8 M      | 0.11 M            | 0.31 M             | 2.7x     |
| 25    | 207.6 M    | 177.4 M     | 4.47 M            | 7.43 M             | 1.7x     |
| 100   | 808.0 M    | 687.0 M     | 20.35 M           | 31.85 M            | 1.6x     |
| 200   | 1,608 M    | 1,367 M     | 41.4 M            | 64.2 M             | 1.6x     |

ARM, `/usr/bin/time -l`, 20,000 elements, minimum of 5 (instructions are deterministic to about
0.1%; cycles are not, so only instructions are read):

| Terms | Retired settled | Retired sink-fed | Change |
|-------|-----------------|------------------|--------|
| 2     | 302.8 M         | 278.3 M          | -8.1%  |
| 4     | 472.3 M         | 425.4 M          | -9.9%  |
| 25    | 2,297 M         | 1,969 M          | -14.3% |
| 100   | 8,810 M         | 7,472 M          | -15.2% |
| 200   | 17,494 M        | 14,815 M         | -15.3% |

The extra D1 misses sit in `eval_each_generic` and in the closures of
`binary_fanout_each_generic_with` -- the frames the nested sinks stack -- and almost none reach
the last-level cache (LL misses are about 0.06 M in both), so they are L2 hits. Growing the
simulated D1 on the 100-term chain (2,000 elements) separates the two working sets:

| Simulated D1        | Settled | Sink-fed |
|---------------------|---------|----------|
| 32 KB (the 7950X's) | 20.35 M | 31.85 M  |
| 128 KB              | 15.90 M | 28.57 M  |
| 512 KB              | 0.17 M  | 13.16 M  |

At 512 KB the settled path fits entirely; the sink-fed path still misses 13 M times. Its
working set is not just larger than 32 KB, it is larger than 512 KB at 100 terms.

The mechanism, as far as these measurements establish it: in a left-nested chain
`((f+f)+f)+...` the leftmost operand delivers its output through every enclosing operator's
sink in turn, each nested inside the evaluation of the next, so a value is combined at a stack
depth of up to twice the chain length, in frames large enough that the chain's working set
exceeds L1. The settled path buffers each operand and replays it after its evaluation has
returned, so the combines run at a shallow depth. What was not measured is the frame size per
level; the claim rests on the miss counts and the D1-size sweep above.

### 3. A chain of non-`def` operands

The issue asked whether `map(.+1+.+1+...)`, which never takes the settled path, pays the same
cost. It does not: the sink-fed path *is* what it takes, its cost per term stays flat from 2 to
100 terms, and the "analysis disabled" build matches the shipped one (0.97x-1.00x). Forcing
those operands through the settled path (the widened variant) is slower on every row:

| Query (20,000 elements) | 7950X sink-fed | 7950X widened | M4 Pro sink-fed | M4 Pro widened |
|-------------------------|----------------|---------------|-----------------|----------------|
| `map(.+1+...)` x 2      | 0.995x         | 1.216x        | 0.983x          | 1.179x         |
| `map(.+1+...)` x 4      | 0.983x         | 1.239x        | 0.983x          | 1.032x         |
| `map(.+1+...)` x 8      | 0.983x         | 1.201x        | 0.992x          | 1.129x         |
| `map(.+1+...)` x 25     | 0.978x         | 1.189x        | 1.000x          | 1.063x         |
| `map(.+1+...)` x 50     | 0.989x         | 1.204x        | 0.988x          | 1.083x         |
| `map(.+1+...)` x 100    | 0.998x         | 1.184x        | 0.986x          | 1.057x         |
| `map(.+1)`              | 0.997x         | 1.119x        | 0.988x          | 1.093x         |
| `map(.*2+1)`            | 0.999x         | 1.151x        | 0.980x          | 1.147x         |
| `map(. < 5 or . > 3)`   | 0.968x         | 1.213x        | 0.991x          | 1.172x         |

The widened variant also slows the two-different-`def`s chain `map(f+g)` (1.11x on the
7950X, 1.09x on the M4 Pro) and `fib(24)` (1.06x on the 7950X; `sum_to(2000)` is unchanged): it
adds a buffer and a replay to operators that were never deep.

### 4. Disabling the analysis is not an option

The sink-fed build answers `fib(24)` with `exceeded maximum recursion depth`, the refusal
#3296 exists to avoid.

## Decision

No change to `settles_both`, the settle analysis, or either evaluator.

- Narrowing the rule to long chains breaks the stack guarantee for two-term recursion.
- Widening it regresses every chain and plain operator that is not nested.
- The residual cost -- 5-9% on a 2-term `def` chain -- is the price of #3296's stack bound.

What would address the cause is a smaller frame for `binary_fanout_each_generic_with` and
`eval_each_generic`, so the sink-fed path stacks cheaper. That is a native-stack-floor change
under ADR-0025 with its own measurement burden (the guard's per-level cost model is calibrated
on today's frame sizes), for a gain that only chains of 8 or more `def` calls would see, and
those already take the faster path. It was not attempted.

## Lessons

- **A cheaper-per-operator path can lose on a cache it does not show in an instruction
  count.** The sink-fed path is 15% fewer instructions and 29% slower at 200 terms. Read
  `Ir` alongside D1 misses before attributing a difference to either.
- **Sweep the cache size before naming a stack-depth cause.** The 32 KB row alone shows only
  that the sink-fed path misses more; the 512 KB row shows that its working set is an order of
  magnitude larger than the settled path's.
- **The crossover is real.** The same switch is 6-9% faster at 2 terms and 26-29% slower at
  200, so a one-chain-length measurement would have reported either sign.

## See also

- [ADR-0025](../adrs/adr-0025.md) -- the native-stack floor and the #3296 amendment that
  introduced the settled path
- [`src/jq/eval.rs`](../../src/jq/eval.rs) -- `binary_fanout_each`, `settles_both`,
  `settled_operand_strategy`, `settle_then_replay`
- [`src/jq/eval_generic.rs`](../../src/jq/eval_generic.rs) -- `binary_fanout_each_generic`
- [docs/guides/benchmarking.md](../guides/benchmarking.md#ab-benchmarking-method) -- the A/B
  method used above
