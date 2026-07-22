# Environment Variables

[Home](../../) > [Docs](../) > [Reference](./) > Environment Variables

Every environment variable succinctly reads, what it accepts, and what it changes.

## Overview

| Variable                                                  | Applies to                       | Accepted values                | When unset                    |
|-----------------------------------------------------------|----------------------------------|--------------------------------|-------------------------------|
| [`SUCCINCTLY_SIMD`](#succinctly_simd)                     | Library (YAML parsing, x86_64)   | `scalar`/`sse2`/`sse4.2`/`avx2`| Best detected level           |
| [`SUCCINCTLY_SVE2`](#succinctly_sve2)                     | Library (JSON indexing, ARM)     | Exactly `1`                    | NEON kernels                  |
| [`SUCCINCTLY_PRESERVE_INPUT`](#succinctly_preserve_input) | `succinctly jq`                  | `1` or `true`                  | jq-compatible formatting      |
| [`NO_COLOR`](#no_color)                                   | `succinctly jq`, `succinctly yq` | Any non-empty value            | Color if stdout is a terminal |
| [`JQ_COLORS`](#jq_colors)                                 | `succinctly jq`                  | Eight `:`-separated SGR fields | Built-in color scheme         |
| [`JQ_LIBRARY_PATH`](#jq_library_path)                     | `succinctly jq`                  | `:`-separated directories      | Only `-L` paths and `~/.jq`   |
| [`HOME`](#home)                                           | `succinctly jq`                  | A directory path               | No `~/.jq` auto-loading       |
| [`TZ`](#tz)                                               | Library (jq date builtins)       | POSIX `STDoffset[DST]`         | UTC                           |
| [`SUCCINCTLY_EXPECT_SIMD`](#succinctly_expect_simd)       | Test suite (`cargo test`)        | Comma-separated CPU features   | Expectation check skipped     |

Queries can also read **any** variable through the [`env` builtins](#reading-the-environment-from-a-query).

Precedence rule of thumb: an explicit command-line flag always beats an environment variable.

## Library Variables

These affect the `succinctly` library itself, so they apply to anything built on it, not just the CLI.

### `SUCCINCTLY_SIMD`

Clamps the x86_64 **YAML** SIMD dispatch to a lower instruction-set level than the CPU supports.
`SUCCINCTLY_SIMD=sse2` makes the YAML parser use the 16-byte SSE2 kernels even on an AVX2 machine.

This is a **clamp, not a selector**: it can only lower the dispatch level, never raise it.
Requesting a level the CPU does not support would mean executing undetected instructions, which is
undefined behaviour, so values at or above the detected level are simply no-ops.

Reasons to set it:

- **CI regression coverage** ([#247](https://github.com/rust-works/succinctly/issues/247)): the
  x86 CI leg re-runs the test suite with `SUCCINCTLY_SIMD=sse2`, so the SSE2 classify/skip-width
  path (the [#231](https://github.com/rust-works/succinctly/issues/231) bug class) executes for
  real on AVX2 runners. An in-lib contract test fails loudly if the variable is set to an
  unrecognized value or the clamp stops applying.
- A/B benchmarking the SSE2 path against AVX2 on the same machine.

Accepted values (case-insensitive, surrounding whitespace ignored):

| Value                             | Effect on x86_64 YAML dispatch                                 |
|-----------------------------------|----------------------------------------------------------------|
| `scalar`, `sse2`, `sse42`/`sse4.2`| 16-byte SSE2 kernels (SSE2 is the x86_64 baseline)             |
| `avx2`, empty                     | No clamp — best detected level, same as unset                  |
| Anything else                     | Ignored at runtime; the test-suite contract test fails on it   |

Note that `scalar` still runs the SSE2 kernels: scalar YAML parsing is a compile-time choice
(`--features scalar-yaml`), not a runtime dispatch level, and SSE2 is unconditionally available on
x86_64.

Scope worth knowing: it clamps **YAML parsing on x86_64 only**
([`src/yaml/simd/x86.rs`](../../src/yaml/simd/x86.rs)). JSON, DSV, popcount, and balanced-parens
dispatch are unaffected (extending the clamp to those sites is tracked as follow-up to #247), and
`aarch64` never reads it. Requires the `std` feature; `no_std` builds compile straight to SSE2
dispatch with no AVX2 path at all.

The value is read **once per process**, on first use, and cached. Changing it mid-run has no effect.

### `SUCCINCTLY_SVE2`

Opts JSON semi-indexing into the experimental ARM SVE2 kernels instead of the default NEON ones.

**You almost certainly do not want this.** SVE2 measured **36% slower than NEON** on Neoverse-V2
(AWS Graviton 4), because converting an SVE2 predicate to a bitmask is expensive and today's shipping
SVE2 implementations are only 128 bits wide — the same width as NEON, so there are no extra lanes to
pay for that conversion. It is retained because the trade-off is a property of the *current* hardware,
not of the code: on a true 256-bit or 512-bit SVE2 implementation each iteration would cover two or
four times the bytes, and the balance could invert. Reasons to set it:

- A/B benchmarking NEON against SVE2 on new hardware, to find out whether that inversion has happened.
- Exercising the SVE2 kernels for correctness testing or profiling.

```bash
# Compare the two kernels on the same input.
succinctly bench run jq_bench
SUCCINCTLY_SVE2=1 succinctly bench run jq_bench
```

It takes effect only when **all** of the following hold; otherwise it is silently ignored:

| Requirement                    | Otherwise                                                                    |
|--------------------------------|------------------------------------------------------------------------------|
| Value is exactly `1`           | `true`, `yes`, `TRUE`, `0` and empty all mean "unset" — no error, no warning |
| Target is `aarch64`            | Ignored on x86_64 (whose YAML dispatch has its own [`SUCCINCTLY_SIMD`](#succinctly_simd) clamp) |
| CPU reports the `sve2` feature | Falls back to NEON                                                           |
| Built with the `std` feature   | `no_std` builds compile straight to NEON, with no dispatch                   |

Scope worth knowing: it switches **JSON index building only** ([`src/json/simd/mod.rs`](../../src/json/simd/mod.rs)).
YAML's own dispatch override is the x86-only [`SUCCINCTLY_SIMD`](#succinctly_simd) clamp, and DSV
and broadword select use the *different* `sve2-bitperm` feature unconditionally — so "SVE2" is
opt-in for JSON but always-on elsewhere when the CPU supports it.

The value is read **once per process**, on first use, and cached. Changing it mid-run has no effect.

How this path is validated (#194): the ARM64 CI job runs the full `--features simd` suite once more
with `SUCCINCTLY_SVE2=1` on its Neoverse-N2 runner (the only routine coverage of the JSON SVE2
dispatch — the plain runs cover the always-on `sve2-bitperm` kernels). Apple Silicon has no
non-streaming SVE2, so locally the equivalent check is
[`scripts/test-sve2-qemu.sh`](../../scripts/test-sve2-qemu.sh), which runs the suite under
`qemu-aarch64 -cpu max` emulation. See CONTRIBUTING.md ("SIMD CI coverage").

### `TZ`

Sets the timezone used by the jq `localtime` and `mktime` family of date builtins
([`src/jq/eval.rs`](../../src/jq/eval.rs)).

Only the POSIX `STDoffset[DST[offset][,rule]]` form is understood, such as `EST5EDT`, `PST8PDT`, or
`UTC-5:30`. The offset is read as `hours[:minutes]` and follows the POSIX sign convention, which is
the opposite of the one most people expect: it is the amount **added to local time to reach UTC**, so
the positive `5` in `EST5EDT` means five hours *behind* UTC.

Two limitations are worth knowing, because neither produces an error:

- **IANA zone names such as `America/New_York` are not supported.** They fail to parse and silently
  mean UTC.
- **DST transition rules are not applied.** The DST portion is parsed but its transition dates are
  ignored, so `EST5EDT` is always UTC-5 — even in July, when New York is really UTC-4.

Anything unparseable, and an unset `TZ`, both mean UTC.

```bash
# 1700000000 is 2023-11-14T22:13:20Z
succinctly jq -nc '1700000000 | gmtime'               # [2023,10,14,22,13,20,2,317]
TZ=EST5EDT succinctly jq -nc '1700000000 | localtime' # [2023,10,14,17,13,20,2,317] (UTC-5)
```

For anything requiring true local time, set an explicit numeric offset rather than a zone name, and
change it yourself across DST boundaries.

## Test-Only Variables

### `SUCCINCTLY_EXPECT_SIMD`

Read only by the test suite ([`tests/simd_expectation_tests.rs`](../../tests/simd_expectation_tests.rs));
the library and CLI never look at it.

Pins the set of CPU features a `cargo test` run is expected to detect. The SIMD test suites self-skip
when the running CPU lacks a feature (printing a `SKIPPED` line), which keeps local runs green on any
hardware — but on CI it would let a runner-fleet change silently skip entire suites. Setting this
variable turns those soft skips into a hard failure: every listed feature must be runtime-detected or
`test_expected_simd_features_are_detected` fails the run (#193).

The value is a comma-separated list of runtime feature names:

| Target    | Recognized names                          |
|-----------|-------------------------------------------|
| `x86_64`  | `sse2`, `sse4.2`, `avx2`, `bmi2`, `popcnt` |
| `aarch64` | `neon`, `sve2`, `sve2-bitperm`             |

Unknown names — including names for the *other* architecture — fail the test, so a typo cannot
silently satisfy the expectation. Unset, the check is skipped entirely.

```bash
# What CI sets per test leg (.github/workflows/ci.yml)
SUCCINCTLY_EXPECT_SIMD=sse2,sse4.2,avx2,bmi2,popcnt cargo test --test simd_expectation_tests  # x86_64
SUCCINCTLY_EXPECT_SIMD=neon,sve2,sve2-bitperm cargo test --test simd_expectation_tests       # ARM64 Linux
SUCCINCTLY_EXPECT_SIMD=neon cargo test --test simd_expectation_tests                         # macOS ARM64
```

## CLI Variables

### `SUCCINCTLY_PRESERVE_INPUT`

Keeps the original formatting of numbers and escape sequences from the input, instead of normalizing
them the way jq does (`4e4` → `4E+4`, `\b` → an escape sequence).

Accepts `1` or `true`, matched case-insensitively — so `TRUE` and `True` also work. Any other value,
including `yes` and `0`, leaves the default alone. Equivalent to the `--preserve-input` flag; the flag
wins when both are given. jq only, with no `yq` equivalent.

```bash
SUCCINCTLY_PRESERVE_INPUT=1 succinctly jq . input.json
```

See [CLI Guide → Output Formatting](../guides/cli.md#output-formatting) for what changes.

### `NO_COLOR`

Disables colored output, per the [no-color.org](https://no-color.org/) convention. Honored by both
`succinctly jq` and `succinctly yq`.

Any **non-empty** value disables color, whatever it says: `NO_COLOR=0` and `NO_COLOR=false` disable
color just as `NO_COLOR=1` does, because the convention gives the value no meaning. Setting it to the
empty string is the same as not setting it at all, and leaves color enabled.

Color is resolved in this order:

| Priority | Condition                    | Result                               |
|----------|------------------------------|--------------------------------------|
| 1        | `-M` / `--monochrome-output` | Never color                          |
| 2        | `-C` / `--color-output`      | Always color, overriding `NO_COLOR`  |
| 3        | `NO_COLOR` set and non-empty | No color                             |
| 4        | Otherwise                    | Color only when stdout is a terminal |

```bash
NO_COLOR=1 succinctly jq . input.json     # no color
NO_COLOR=1 succinctly jq -C . input.json  # color: -C wins
```

Because color is off by default when stdout is not a terminal, setting `NO_COLOR` changes nothing
when piping or redirecting.

### `JQ_COLORS`

Customizes the colors of `succinctly jq` output. Ignored by `succinctly yq`, which has its own fixed
scheme, and ignored by both when color is off.

The format is eight `:`-separated [SGR](https://en.wikipedia.org/wiki/ANSI_escape_code#SGR) parameters:

```
null:false:true:numbers:strings:arrays:objects:objectkeys
```

| Field | Meaning     | Default | Renders as        |
|-------|-------------|---------|-------------------|
| 1     | `null`      | `1;30`  | Bold black (gray) |
| 2     | `false`     | `0;39`  | Terminal default  |
| 3     | `true`      | `0;39`  | Terminal default  |
| 4     | Numbers     | `0;39`  | Terminal default  |
| 5     | Strings     | `0;32`  | Green             |
| 6     | Arrays      | `1;39`  | Bold default      |
| 7     | Objects     | `1;39`  | Bold default      |
| 8     | Object keys | `1;34`  | Bold blue         |

The full default is `1;30:0;39:0;39:0;39:0;32:1;39:1;39:1;34`. The reset sequence is not configurable.

```bash
# Red null, everything else left at its default. Spell the defaults out: an empty
# field is not "keep the default" (see below).
JQ_COLORS='0;31:0;39:0;39:0;39:0;32:1;39:1;39:1;34' succinctly jq -C . input.json

# Only the fields you omit entirely keep their default, so this is red null too.
JQ_COLORS='0;31' succinctly jq -C . input.json
```

Parsing rules, which match jq 1.7:

- A field may contain **only digits and `;`**. Anything else makes the whole variable invalid.
- An **invalid** variable is rejected as a whole: `Failed to set $JQ_COLORS` goes to stderr, every
  color falls back to its default — including the fields that were well-formed — and the exit status
  is unaffected.
- An **empty** field means the empty SGR sequence, *not* "keep the default".
- Fewer than eight fields leaves the remaining colors at their defaults.
- Fields after the eighth are ignored, and are not validated.

### `JQ_LIBRARY_PATH`

A `:`-separated list of directories to search for jq modules, used by `import` and `include`.

Entries that are not existing directories — including files and typo'd paths — are **dropped without
a warning**, so a module that fails to resolve may be a bad path rather than a bad import. Succinctly
does not expand `~` itself, so a tilde only works when the shell expands it first; prefer `$HOME`.
`;` is not accepted as a separator.

Search order, highest first:

1. `-L` / `--library-path` command-line paths
2. `JQ_LIBRARY_PATH` entries, in the order listed
3. `~/.jq`, when it is a directory

```bash
JQ_LIBRARY_PATH="$HOME/lib/jq:/usr/share/jq" succinctly jq 'import "utils" as u; u::helper' input.json
```

See [jq Language → Module System](jq-language.md#module-system).

### `HOME`

Locates `~/.jq`, which succinctly loads automatically. `HOME` is not read for any other purpose.

`~/.jq` is treated two different ways depending on what it is:

| `~/.jq` is      | Effect                                                                         |
|-----------------|--------------------------------------------------------------------------------|
| A **file**      | Parsed, and its function definitions are added to **every** query              |
| A **directory** | Appended to the module search path (see [`JQ_LIBRARY_PATH`](#jq_library_path)) |

Worth being aware of: when `~/.jq` is a file, its definitions are in scope for every invocation
without being imported, so a definition there can shadow one you expect from elsewhere. Failures are
**silent** — an unreadable file, or one that does not parse, is skipped with no diagnostic, which
makes a broken `~/.jq` look like a query that mysteriously lost its helpers. If `HOME` is unset,
neither behavior applies.

The spelling is Unix-only; there is no `USERPROFILE` fallback on Windows.

## Reading the Environment from a Query

The jq language exposes the whole process environment to queries. These builtins read arbitrary
variables, so the set of variables that matter is ultimately whatever your queries name.

| Syntax                    | Returns                  | If the variable is unset |
|---------------------------|--------------------------|--------------------------|
| `$ENV`                    | Object of every variable | n/a                      |
| `env`                     | Object of every variable | n/a                      |
| `env.VAR`, `$ENV.VAR`     | The value                | `null`                   |
| `env(VAR)` (yq syntax)    | The value                | Error                    |
| `strenv(VAR)` (yq syntax) | The value                | Error                    |

Values are always strings; nothing is coerced to a number or boolean.

```bash
API_KEY=secret succinctly jq -n 'env.API_KEY'      # "secret"
succinctly jq -n '$ENV | keys | length'            # count of variables
succinctly yq '.image = strenv(TAG)' deployment.yaml
```

These are gated on the `std` feature rather than `cli`, so they are available to any program
embedding the `succinctly::jq` library — meaning an embedder exposes its own process environment to
whoever writes the queries. Under `no_std` they return an empty object, `null`, or an error.

## See Also

- [CLI Guide](../guides/cli.md) - Command-line tool reference
- [jq Language](jq-language.md) - jq query language features
- [yq Language](yq-language.md) - yq query language features
- [SIMD Strategy](../optimizations/simd-strategy.md) - How SIMD kernels are selected
