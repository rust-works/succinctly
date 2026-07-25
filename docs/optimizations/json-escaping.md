# JSON String Escaping: one shared, SIMD-backed escaper

[Home](../../) > [Docs](../) > [Optimizations](./) > JSON Escaping

**Status: ACCEPTED — July 2026**

**Issue**: [#91](https://github.com/rust-works/succinctly/issues/91) ·
builds on [#87](https://github.com/rust-works/succinctly/issues/87) (O3: SIMD escape
scanning for the yq streaming path) ·
depends on [#125](https://github.com/rust-works/succinctly/issues/125) (O5: extract the
shared scanner module)

> **TL;DR.** Eleven copies of JSON string escaping, in three mutually incompatible sets of
> semantics, collapsed to one SIMD-backed implementation in
> [`src/json/escape.rs`](../../src/json/escape.rs). The escaper itself is **2–4× faster**
> at and above the corpus p90. End-to-end the picture splits sharply: default `jq` output
> is **neutral**, because the CLI's zero-copy passthrough bypasses escaping entirely on
> escape-free input; `jq --ascii-output` and `yq -o json`, which have no such bypass, are
> **6–38%** and **1–11%** faster. Three jq-conformance bugs fell out along the way, and
> those — not the throughput — are the change's most durable result.

## Problem

[#87](https://github.com/rust-works/succinctly/issues/87) gave the yq streaming path a
SIMD escape scanner. #91 set out to reuse it in the jq output path. Surveying the callers
first turned up a larger problem than the issue described: **eleven** escaping routines,
not the "~4 copies" the issue's own comment estimated, spread across three semantics that
disagreed with each other and, in one case, with jq itself.

| Family | Predicate | Sites |
|---|---|---|
| jq CLI | `is_control()` **with** `\b`/`\f` | `output.rs` ×2 |
| jq library | `is_control()` **without** `\b`/`\f` | `value.rs`, `eval.rs`, `lazy.rs` ×2 |
| yq / streaming | `< 0x20` | `output.rs` ×2, `stream.rs`, `light.rs` ×2 |

Only two of the eleven were SIMD-accelerated. Two more hand-rolled a scalar loop over
*exactly* the scanner's predicate. And the jq-library family matched neither its own CLI
nor jq.

## Where the win actually comes from

Not the SIMD — and this is the part worth remembering.
[`docs/benchmarks/corpus-shape.md`](../benchmarks/corpus-shape.md) measures real JSON
string lengths at **p50 = 7 bytes, p90 = 11, p99 = 24**, with an escape density of
**0.00 per KiB**. The scanners engage at 16 bytes (NEON/SSE2) and 32 (AVX2), so over 90%
of real strings never reach a SIMD kernel at all, and the ones that do find nothing.

What adopting a scanner *forces* is a different shape of scalar work:

| | before | after |
|---|---|---|
| safe runs | `result.push(c)` per `char` | one `write_str` per span |
| control chars | `format!("\\u{:04x}")` — a heap allocation each | four writes from a nibble table |
| decoding | UTF-8 decoded to `char`, re-encoded | bytes copied as bytes |

That is what the numbers below are measuring. The SIMD is a bonus that arrives at 16 bytes.

## Design: three scanners, four escapers, two layers

`crate::util::simd::escape` (crate-private) answers *where is the next byte I must look
at*; `succinctly::json::escape` (public) answers *what do I emit for it*. The escapers had
to live in the library rather than the CLI — `src/bin` is a separate crate, so without a
library home neither tests nor benches can reach them.

| Scanner | Predicate | Serves |
|---|---|---|
| `find_json_escape` | `"` `\` `< 0x20` | yq output, YAML→JSON transcoding |
| `find_jq_escape` | + DEL (`0x7F`) | jq output |
| `find_ascii_escape` | `"` `\` `< 0x20` `>= 0x7F` | both `--ascii-output` modes |

`EscapeStyle` × `ascii` selects one of four monomorphizations of a single loop. The const
generics stay private; a runtime enum on the public boundary gives identical codegen
because every call site passes literals.

### The C1 decision, and why it kept the predicate pure ASCII

succinctly escaped the C1 block (U+0080–U+009F) because `char::is_control()` covers
Unicode category Cc, which includes it. Checked against the oracle, **jq does not**:

```console
$ printf '{"s":"abc"}' | jq -c '.' | xxd
00000000: 7b22 7322 3a22 61c2 8562 5c75 3030 3766  {"s":"a..b
00000010: 6322 7d0a                                c"}.
```

C1 raw, DEL escaped. #91 chose to match jq, and that choice paid for itself twice.
Escaping C1 would have required flagging its UTF-8 lead byte `0xC2` — which also leads
U+00A0–U+00BF, so NBSP and the Latin-1 punctuation `¡ ¢ £ ° » ¿` would each become a
false-positive stop, turning accented-Latin text into a stream of interruptions. Dropping
it leaves `find_jq_escape` **pure ASCII**, which means no byte ≥ 0x80 can ever match and
its returned index is unconditionally a UTF-8 char boundary.

`benches/jq_escape_micro.rs` keeps a `latin1_heavy` cell as a standing guard: if a future
change reintroduces a `0xC2` lane, that is where the bill appears.

### The one conditional invariant

`find_ascii_escape` is the exception. Its `>= 0x7F` lane matches UTF-8 *continuation*
bytes as well as lead bytes, and is sound only because a left-to-right scan that starts on
a char boundary always meets the lead byte (≥ 0xC2) first. The obligation transfers to
callers: **advance past a stop by `len_utf8()`, never by one byte.** A one-byte advance
resumes mid-character, and the next scan returns a continuation-byte index that panics
anything slicing on it. `advancing_one_byte_desyncs_the_ascii_scanner` pins that as a
demonstrated fact rather than a comment.

## Results

Measured on the dedicated bench machines, never the laptop. Median of 7.

### Escaper micro-benchmark (`jq_escape_micro`)

Sizes are the corpus percentiles; delta is the shared escaper against a frozen verbatim
copy of the per-char escaper it replaced.

| bytes | | Apple M4 Pro | AMD Ryzen 9 7950X |
|---|---|---|---|
| 1 | | **+8% … +14%** | −3% … +7% |
| 4 | | −3% … −12% | **+5% … +12%** |
| 7 | p50 | −8% … −25% | −4% … +2% |
| 11 | p90 | −24% … −37% | −10% … −24% |
| 24 | p99 | −48% … −58% | −50% … −65% |
| 51 | max | −69% … −78% | −75% … −78% |
| object mix | | −6.5% … −16.9% | −13% … +6% |

The crossover sits between 4 and 11 bytes. Below it the loop's own bookkeeping costs more
than the per-`char` loop it replaced; above it the bulk copy wins outright.

**The 1-byte regression is real and is not chased further.** It is ~1.5 ns absolute, and
the obvious remedy — a length threshold in the escaper — is exactly the shape of
[P2.8](../parsing/yaml.md#p28-simd-threshold-tuning---rejected-), which this project
already rejected for regressing 8–15% end-to-end.

What *was* worth fixing: the first A/B put 1-byte at +16–24% and the realistic
object-shaped mix at **+5% slower**. Fast-pathing the escape-free case in the
`*_to_string` entry points — one scan, then a single bulk copy, no emit loop — moved the
object mix to −6.5…−16.9% and everything ≥ 4 bytes into the black. The writer forms
deliberately keep the loop: they cannot copy in bulk, so a pre-scan there is duplicated
work.

### End-to-end

| workload | M4 Pro | 7950X |
|---|---|---|
| `jq -c .` (default) | ±0.4% | — |
| `jq -a -c .` strings/1mb | **−14.0%** | **−23.1%** |
| `jq -a -c .` unicode/1mb | **−38.3%** | **−30.2%** |
| `jq -a -c .` users/1mb | −6.2% | −2.4% |
| `yq -o json .` strings/1mb | **−10.9%** | **−8.3%** |
| `yq -o json .` users/1mb | −5.8% | −5.6% |
| `yq -o json .` nested/1mb | −1.1% | — |

**Default `jq` output is neutral, and the reason is structural, not statistical.**
[`jq_runner.rs`](../../src/bin/succinctly/jq_runner.rs) echoes a string's source bytes
verbatim when they contain no backslash and `--ascii-output` is off, so on escape-free
input the escaper is never called. `-a` removes that bypass and every string goes through
it — which is where the 14–38% appears. `yq -o json` has no such bypass at all, which is
why its streaming path shows a win at default settings.

A first pass through `dev bench jq` across 12 pattern/size cells came back within ±2.6%
with the system-`jq` drift control swinging 3–41% between passes. That measurement was
discarded as uninformative rather than reported as a win: the noise floor was an order of
magnitude larger than the effect, and the identity query barely touches the escaper.

## Correctness, which outlasts the throughput

Three jq-conformance defects surfaced and were fixed:

1. **Object keys could produce invalid JSON.** `JqValue::write_json` escaped only `"` and
   `\` in keys, so a key containing a newline, tab, or NUL was written as a raw byte inside
   a quoted string. Latent — the path has no caller outside its own tests — which is
   precisely why it survived.
2. **`tojson` / `@json` disagreed with jq on backspace and form feed**, emitting `` /
   `` where jq emits `\b` / `\f`.
3. **C1 controls were escaped**, where jq emits them raw (above).

The jq golden corpus could not have caught any of these: all 41 seed cases from #300 were
escape-free ASCII, so the only external oracle in the suite was blind to the entire
escaping surface. #91 added five cases covering C0 controls, DEL, C1, object keys, and
`--ascii-output`; two landed on the known-failures manifest at capture time and both were
removed by the end of the issue.

## Reproducing

```bash
cargo bench --bench jq_escape_micro           # whole-escaper A/B (runs check_parity first)
cargo bench --bench json_escape_micro         # the three scanners
cargo test  --bench jq_escape_micro           # parity guard only

cargo test --lib json::escape                 # differential vs frozen escapers + oracles
cargo test --lib util::simd::escape           # per-scanner exhaustive kernel parity
cargo test --features cli --test jq_golden_tests
```

Benchmarks run **one machine at a time** — `johns-mac-mini` (M4 Pro, NEON) and `terminus`
(Ryzen 9 7950X, AVX2). Laptop numbers are thermally biased and do not belong in this file.
Note there is no AVX-512 kernel: on `terminus` you are measuring the 32-byte AVX2 loop.

## Lessons

- **Survey the callers before believing the issue.** #91 was written as "SIMD-ify one
  function"; it was eleven functions in three conventions, and the duplication was doing
  more damage than the missing SIMD.
- **Check the oracle before preserving a behaviour.** The `0xC2` lane, its false-positive
  contract, and a whole class of boundary test existed only to preserve a divergence from
  jq that nobody had verified was intentional. Deleting it made the code faster *and*
  simpler *and* more correct.
- **A corpus makes "is this worth optimizing" answerable.** p50 = 7 bytes said up front
  that SIMD could not be the win, and told the benchmark which sizes to gate on.
- **Know why a neutral result is neutral.** Default `jq` showing ±0.4% was not noise and
  not failure — it was a zero-copy passthrough doing its job. Finding that out changed the
  claim from "no effect" to "no effect *here*, 14–38% where the code actually runs".

## See also

- [O3: SIMD escape scanning](../parsing/yaml.md#o3-simd-escape-scanning-for-json-output--accepted-) — the #87 predecessor
- [SIMD strategy](simd-strategy.md) — dispatch and predicate conventions
- [Benchmark corpus shape](../benchmarks/corpus-shape.md) — the percentiles this gate uses
- [`src/json/escape.rs`](../../src/json/escape.rs) · [`src/util/simd/escape.rs`](../../src/util/simd/escape.rs)
