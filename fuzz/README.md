# Fuzzing the JSON validator

[Home](../) > Fuzzing

`succinctly json validate` and `sjq --validate` run the validator over
attacker-controlled bytes, so a panic there is a denial of service — which is
not hypothetical: [#151](https://github.com/rust-works/succinctly/issues/151)
was a reachable stack-overflow abort via unbounded recursion. These targets
search for the next one.

## Why a separate crate

This is a standalone workspace with its own `Cargo.lock`, mirroring
[`bench-compare/`](../bench-compare/). `cargo-fuzz` needs nightly
(`-Zsanitizer=address`) while the main crate promises `rust-version = "1.73.0"`,
so isolating the harness keeps `libfuzzer-sys` and `arbitrary` out of the main
dependency graph entirely — neither the MSRV promise nor `cargo deny check all`
is affected.

## Targets

| Target | Checks | Catches |
|---|---|---|
| `validate_never_panics` | no panic or UB on any input | DoS; memory unsafety, under ASAN |
| `validate_vs_serde_json` | validity agrees with `serde_json`, modulo the two classified divergences | the validator being *self-consistently* wrong |
| `validate_position_invariant` | reported `Position` is reconstructible from its offset alone | drift in the CLI's rendered caret column |

The last two share their oracle with `tests/json_validate_properties.rs` via
`#[path]` include of [`tests/common/json_oracle.rs`](../tests/common/json_oracle.rs),
so the divergence classifier cannot go stale in one copy and silently excuse a
real bug in the other.

## Running

A single target, by hand:

```bash
cargo install cargo-fuzz              # once
./fuzz/seed-corpus.sh                 # materialise seeds from the vendored corpora

cargo +nightly fuzz run validate_never_panics \
    fuzz/corpus/validate_never_panics fuzz/seed-corpus
```

The seed corpus is generated, not committed — every seed duplicates something
already tracked (the 318 vendored JSONTestSuite cases, the real-workload bench
seed, the proptest regression seeds).

All three targets, for a full campaign (see "When to run it" below):

```bash
./fuzz/run-campaign.sh                # 4 CPU-hours/target, auto worker count
./fuzz/run-campaign.sh --cpu-hours 8
```

`run-campaign.sh` builds the targets, materialises the seed corpus, runs all
three concurrently via cargo-fuzz's fork mode (`-j`), and exits nonzero if any
target crashed or left an artifact behind.

## When to run it

**Not on the pull-request path.** A useful run is minutes to hours; a 60-second
CI run finds nothing and grants false confidence — the exact failure mode
[`note_simd_skip`](../src/util/simd/mod.rs) exists to prevent. Instead:

* **Before any change to the validator's control flow**, run ≥ 4 CPU-hours per
  target on both benchmark architectures (they are different code paths only if
  the validator grows arch-specific fast paths, but the wall-clock is cheap and
  the toolchains differ). Link the run in the pull request — `run-campaign.sh`'s
  summary table is written for pasting directly into a comment.
* Optionally weekly, as a regression tripwire.

## When it finds something

Turn the artifact into a **named unit test** in `src/json/validate.rs`'s test
module before fixing it, per
[`.claude/skills/testing/SKILL.md`](../.claude/skills/testing/SKILL.md) and the
naming precedent of `test_bit63_carry_regression` — the issue number belongs in
the test name so a reintroduction fails loudly and legibly.
