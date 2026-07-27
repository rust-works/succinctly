# YAML Test Suite Conformance and Known Limitations

[Home](../../../) > [Docs](../../) > [Compliance](../) > YAML Limitations

This page records what succinctly's YAML parser does and does not do, measured against
the official [YAML Test Suite](https://github.com/yaml/yaml-test-suite) rather than
asserted. Every number here is produced by `tests/yaml_test_suite.rs`; regenerate them
with:

```bash
cargo test --test yaml_test_suite -- --nocapture
```

For scalar type handling specifically (the Norway problem, booleans, quoted numbers),
see [YAML 1.2 Compliance](1.2.md). Plain scalars resolve per the 1.2 core schema via a
single shared resolver (`src/yaml/scalar.rs`, added in
[#226](https://github.com/rust-works/succinctly/issues/226)); the deliberate rejections
of YAML 1.1 legacy numeric forms that `yq` still accepts (`1_000`, `0X2A`, `0b101`,
signed hex/octal) are documented in that page's
[Differences from System yq](1.2.md#differences-from-system-yq) table.

## Summary

Measured against suite tag `data-2022-01-17` (402 cases), on the library's streaming
path:

| Dimension                              | Result              | Meaning                                        |
|----------------------------------------|---------------------|------------------------------------------------|
| **Load** (valid YAML, output compared) | **215/279 = 77.1%** | Parses and produces the JSON the suite expects |
| **Reject** (invalid YAML, must fail)   | **70/94 = 74.5%**   | Refused by the loader or the opt-in validator  |
| **Parse** (valid YAML, no JSON form)   | **27/29 = 93.1%**   | Parses without error                           |

The **Reject** figure is what the conformance harness rejects with the opt-in
validator enabled (loader OR validator, see below). The *default non-validating
loader alone* still rejects only 12/94 (12.8%) by design — the opt-in validator
([#223](https://github.com/rust-works/succinctly/issues/223)) closes 58 more.

The 90 non-passing cases are enumerated individually, with a category and reason, in
[`tests/data/yaml-test-suite-known-failures.txt`](../../../tests/data/yaml-test-suite-known-failures.txt).
That file is the machine-readable source of truth; the test asserts it matches reality
exactly, so it cannot silently drift from this page.

## Validation: default loader vs. the opt-in validator

**The default loader is non-validating by design.** `YamlIndex::build` rejects 12 of the
suite's 94 invalid documents; the other 82 are accepted and produce a value.

This follows from semi-indexing. The index records *structure* — where values start and
end, and how they nest — not grammar conformance. The parser is a structure recognizer:
it resolves enough context to emit a correct tree for well-formed input, and absorbs
anything it does not recognize as scalar text. Checking the ~1200 productions of the YAML
1.2 grammar is work it deliberately does not do, and that omission is a large part of why
it is 5-10x faster than `yq`.

**When you need malformed YAML rejected, use the opt-in validator.**
[`succinctly yaml validate`](../../guides/cli.md) / `syq --validate` runs a separate strict
pass ([`src/yaml/validate.rs`](../../../src/yaml/validate.rs)) before indexing — mirroring
`json validate` / `sjq --validate`, so the default path pays nothing. It rejects **58 of
the 82** documents below with zero false positives on the valid corpus (a guardrail test,
`validator_accepts_all_valid_cases`, enforces that). The conformance harness treats a
must-fail case as handled if the loader *or* the validator rejects it, which is why the
Reject figure above is 70/94 rather than 12/94.

The 24 documents the validator does not yet reject (marked `lax:*` in the manifest) need
deeper structural analysis — cross-line anchor binding, block-scalar content indentation,
flow multi-line implicit keys — and are tracked in
[#223](https://github.com/rust-works/succinctly/issues/223).

The 82 accepted-but-invalid documents (of which the validator now rejects 58) break down as:

| Category           | Cases | What is not checked                          |
|--------------------|-------|----------------------------------------------|
| `lax:mapping`      | 14    | Mapping and implicit-key rules               |
| `lax:documents`    | 12    | Directive and document-marker placement      |
| `lax:flow`         | 11    | Flow collection syntax                       |
| `lax:tabs`         | 8     | Tabs where indentation is expected           |
| `lax:indentation`  | 7     | Indentation consistency                      |
| `lax:other`        | 7     | Assorted                                     |
| `lax:quoting`      | 6     | Quoting and escape sequences                 |
| `lax:block-scalar` | 6     | Block scalar header validity (`\|--` parses) |
| `lax:anchors`      | 6     | Anchor and alias rules                       |
| `lax:comments`     | 5     | Comment placement                            |

The opt-in validation mode now exists ([#223](https://github.com/rust-works/succinctly/issues/223)),
mirroring the JSON side's `succinctly json validate` / `sjq --validate`. Validation is a
separate pass run before indexing, so the default path pays nothing for it — see
[`src/yaml/validate.rs`](../../../src/yaml/validate.rs). The 82 cases above were its
acceptance criteria; it rejects 58 of them today (each `lax:*` line removed from the
manifest is a case now rejected), with the 24 harder cases still tracked in #223.

### What "accepted" means: the text is kept, not dropped

For an accepted-but-invalid document the loader's job is to produce *some* value without
losing input. Where a construct has no valid reading, the parser absorbs it as scalar text
rather than discarding it — the same principle as the tag handling below.

`- ` followed by content is one such case. It is always the sequence-entry indicator, and
no block sequence can begin inside a flow collection or after a `:` on the same line, so
`[- x]` and `key: - a` are invalid (`yq` rejects both, and so does the validator). The
loader reads them as the plain scalar `"- x"` / `"- a"`:

```
$ printf '[- x]\n' | succinctly yq -o json -I0 '.'
["- x"]                   # yq: did not find expected node content
```

Until [#332](https://github.com/rust-works/succinctly/issues/332) these yielded `[null]`
and `{"key":null}` — the content was silently discarded, which is a worse failure mode than
either erroring or keeping the text. A `-` *not* followed by whitespace is an ordinary flow
plain scalar and always was: `[-]` is `["-"]`, `[-1]` is `[-1]`.

### Two exceptions: an alias with no usable target is rejected

Two alias forms are refused at index build. They are the same failure underneath — an
alias with nothing to resolve to:

- **Cyclic.** An alias that would make an anchored value contain itself
  (`a: &x {self: *x}`) fails with `YamlError::AliasCycle` ("anchor value contains itself").
  A cyclic document cannot be materialized at all; before
  [#153](https://github.com/rust-works/succinctly/issues/153) it aborted the process with a
  stack overflow. YAML 1.2's representation graph technically permits cycles, but `yq`
  rejects the same input at decode time, so rejecting is also the compatible behavior.
- **Unknown.** An alias naming an anchor that is not in scope — never defined, or defined
  *later* — fails with `YamlError::UnknownAnchor` since
  [#372](https://github.com/rust-works/succinctly/issues/372). YAML 1.2 §7.1 requires an
  alias to name a *previous* anchor, so a miss is invalid input rather than a value, and
  `yq` reports `unknown anchor 'x' referenced` for the same input.

```
$ printf 'a: &x {self: *x}\n' | succinctly yq '.'
Error: YAML parse error: cyclic alias 'x' at offset 13 (anchor value contains itself)
$ printf 'a: *x\nb: &x 5\n' | succinctly yq '.'
Error: YAML parse error: unknown anchor 'x' referenced at offset 3
$ printf 'b: &x 5\na: *x\n' | succinctly yq -o json -I0 '.'
{"b":5,"a":5}             # a resolvable alias is unaffected
```

Until #372 the second silently yielded `null` — or `""` where the alias was a key — the
same silent-wrong-data mode as the `- ` case above.

These two are the loader's only checks on *document structure*, and neither is grammar
conformance: both are cases where there is no value to produce at all, not cases where the
input breaks a rule the index could otherwise ignore. Rejecting the rest of what the suite
calls invalid is the validator's job.

Neither costs this page's conformance numbers anything: no case in the corpus contains an
alias to an out-of-scope anchor, so the 12/94 loader figure, the 82 accepted-but-invalid
documents and the `lax:anchors` row are unmoved by #372.

### The other ways `YamlIndex::build` fails

Structure aside, `build` still refuses input it has no reading for at all. None of these is
a conformance check either — they are an absent feature, bytes it cannot tokenize, and the
index's own ceilings:

| Rejected                                        | Example      | `YamlError`           |
|-------------------------------------------------|--------------|-----------------------|
| A tag in node position (absent feature, below)  | `a: !!str 1` | `TagNotSupported`     |
| A tab where block structure expects indentation | `\ta: 1`     | `TabIndentation`      |
| A quote with no close before end of input       | `a: "x`      | `UnclosedQuote`       |
| Input ending inside a flow collection or escape | `a: {k: 1`   | `UnexpectedEof`       |
| A flow item not followed by `,` or its closer   | `a: [[1] 2]` | `UnexpectedCharacter` |
| A key run that ends before its `:`              | `b #c: d`    | `KeyWithoutValue`     |
| `&` or `*` with an empty name                   | `a: &`       | `InvalidAnchorName`   |
| Non-UTF-8 bytes in an anchor name               | —            | `InvalidUtf8`         |
| Nesting past 128 levels                         | 129 × `[`    | `NestingTooDeep`      |
| Empty input                                     | —            | `EmptyInput`          |
| Input of `u32::MAX` bytes or more               | ≥ 4 GiB      | `InputTooLarge`       |

The last is a property of the index rather than of YAML — text positions are stored as
`u32`, see [Input Size Limits](../../reference/limits.md).

The flow row is the closest the loader comes to grammar checking, and it marks where the
line actually falls. `[[1] 2]` fails because a `]` cannot be followed by a bare `2`; the
same missing comma in `[1 2]` does not, because a plain scalar absorbs it:

```
$ printf 'a: [[1] 2]\n' | succinctly yq '.'
Error: YAML parse error: unexpected character '2' at offset 8: expected ',' or ']' in flow sequence
$ printf 'a: [1 2]\n' | succinctly yq -o json -I0 '.'
{"a":["1 2"]}             # same missing comma, absorbed
```

The loader stops where it cannot continue, not where a rule is broken — which is why the
11 `lax:flow` cases above survive it.

## Unsupported features

These are absent rather than wrong, and account for 47 of the 77 load failures.

### Tags — 33 cases (31 load, 2 parse)

`!!str`, `!custom`, and verbatim `!<tag:...>` are not supported. In block context the
parser rejects them outright:

```
$ echo 'a: !!str 1' | succinctly yq '.'
Error: YAML parse error: tags (!) not supported at offset 3
```

Flow context rejects them the same way, in every position — sequence item, mapping value,
mapping key, and the explicit `? k : v` form
([#369](https://github.com/rust-works/succinctly/issues/369); before that fix flow context
absorbed the tag into the scalar, so `[!!str a]` yielded the string `"!!str a"`):

```
$ echo 'a: [!!str x]' | succinctly yq '.'
Error: YAML parse error: tags (!) not supported at offset 4
```

A `!` *inside* plain scalar content is not an indicator and remains ordinary text in both
contexts — `[x!y]` is `["x!y"]`, `a: hello!world` is `"hello!world"`. Only a `!` starting a
node is a tag.

Tag *support* is tracked in [#224](https://github.com/rust-works/succinctly/issues/224);
these 33 cases stay failures until it lands.

### Directives — 16 cases (all load)

`%YAML` and `%TAG` directives are not recognized. A directive line parses as an ordinary
plain scalar, which also swallows the `---` that follows it:

```
$ printf '%%YAML 1.2\n--- text\n' | succinctly yq '.'
"%YAML 1.2 --- text"      # expected: "text"
```

Tracked in [#225](https://github.com/rust-works/succinctly/issues/225).

### An explicit key and its `: ` on one line

`? k: v` — the explicit-key indicator and a value indicator on the *same* line —
diverges from `yq`, which reads the whole `k: v` as a mapping used as the key and
so renders the key as `""` (complex keys stringify to `""`, as above):

```
$ printf '? k\n: v\n' | succinctly yq -o=json -I=0 '.'
{"k":"v"}                            # multi-line: agrees with yq
$ printf '? k: v\n' | succinctly yq -o=json -I=0 '.'
{"k":"v"}                            # yq: {"":null}
$ printf 'm:\n  ? k: v\n' | succinctly yq -o=json -I=0 '.'
{"m":{"k":null}}                     # yq: {"m":{"":null}}
$ printf -- '- ? k: v\n' | succinctly yq -o=json -I=0 '.'
[{"k":null}]                         # yq: [{"":null}]
```

`parse_explicit_key` ends the key at the `: ` and returns with the parser
*mid-line*. The main loop then re-derives that line's indentation from the
mid-line position and reads it as 0, so `parse_explicit_value` closes the mapping
it should have been filling — which is why the nested spellings lose the value,
while at top level the mapping is at indent 0 and survives.

The multi-line spelling — the one every real document uses, and the only one the
YAML Test Suite corpus exercises — is unaffected in all three positions.

Since [#339](https://github.com/rust-works/succinctly/issues/339) the sequence-item
position shares this wart instead of having one of its own: routing `- ? k`
through the shared `parse_explicit_key` replaced a *fourth*, different corruption
(the `? ` folded into a plain scalar) with the behaviour the nested mapping case
already had. Fixing the mid-line return would fix all of them at once.

## Line breaks — resolved

All three YAML 1.2 §5.4 line-break forms — LF (`\n`), CRLF (`\r\n`) and a lone CR
(`\r`) — are normalized to `\n` on input, so the same document loads identically
whichever one it uses.

This was not always so. Until [#324](https://github.com/rust-works/succinctly/issues/324)
the `\r` was folded into every *plain* scalar as a trailing space, which also
destroyed type resolution: a Windows-authored `a: 1` loaded as the string `"1 "`
rather than the number `1`, and `a: true` as `"true "`. It was silent — no error,
no diagnostic, and well-formed JSON out — and it went unnoticed because every
fixture and benchmark input in this repo uses LF.

`tests/yaml_crlf_tests.rs` now holds the line against it: every case in the YAML
Test Suite corpus is parsed under all three break forms and the outputs must be
identical, and `tests/yq_golden_tests.rs` runs the pinned-`yq` goldens the same
way. Both assertions are on invariance, so a document's line-break form can never
again change what it loads as.

## Two output paths that used to disagree — resolved

`succinctly yq` has two YAML-to-JSON implementations and picks between them based on
output formatting:

- **Compact** (`-I 0`) takes the P9 streaming path (`YamlCursor::to_json` / `stream_json`).
- **Pretty** (the default) builds an `OwnedValue` DOM instead.

They used to produce different *values* — not merely different whitespace — for 29 cases:
an alias used as a mapping key resolved on the DOM path but became `""` on the streaming
path; complex (sequence/mapping) keys were dropped entirely by the DOM path but kept by
the streaming path; quoted-string line folding trimmed literal versus escaped whitespace
inconsistently across three separate decoders; and empty or numeric block scalars were
type-inferred on the streaming path.

```
$ succinctly yq -o json '.'      26DV.yaml   # {"top3": {"scalar1": "scalar3"}}
$ succinctly yq -o json -I 0 '.' 26DV.yaml   # {"top3": {"scalar1": "scalar3"}}  — now agrees
```

[#222](https://github.com/rust-works/succinctly/issues/222) fixed all of these: mapping
keys share one stringification rule (`YamlValue::key_string`) that resolves alias keys and
keeps complex keys as `""`; the three quoted-string decoders share one fold rule (drop only
*literal* trailing whitespace before a line break, preserve escaped whitespace as content);
and block scalars are always emitted as strings. `tests/yq_path_consistency_tests.rs` now
runs every corpus case through both paths and asserts they agree, so the paths cannot drift
apart again.

Three corpus cases still differ, each from a *separately-tracked* bug rather than the
key/fold/block-scalar drift #222 fixed, and are listed in
[`tests/data/yq-path-consistency-known-failures.txt`](../../../tests/data/yq-path-consistency-known-failures.txt):
`55WF` and `HRE5` (invalid `\`-escapes the non-validating loader accepts, where the
streaming transcoder cannot roll back and emits malformed JSON —
[#223](https://github.com/rust-works/succinctly/issues/223)), and `UGM3` (integer-valued
floats render as integers on the streaming path, `1.0` → `1` —
[#168](https://github.com/rust-works/succinctly/issues/168) /
[#170](https://github.com/rust-works/succinctly/issues/170)).

## Full accounting of the 64 load failures

| Category     | Cases | Cause                                                             |
|--------------|-------|-------------------------------------------------------------------|
| `tags`       | 31    | Tags not supported (above)                                        |
| `directives` | 16    | `%YAML` / `%TAG` not recognized (above)                           |
| `structure`  | 9     | Document end markers; anchors with colons in the name             |
| `scalars`    | 8     | Zero-indented block scalars; tabs; trailing whitespace            |

`scalars` was 13 until [#329](https://github.com/rust-works/succinctly/issues/329). Folded
(`>`) block scalars mis-counted the newlines a blank line is worth — a blank line yielded
N+1 where YAML 1.2 §8.1.3 and `yq` give N — and folded blocks with keep chomping (`>+`)
dropped `b-chomped-last` entirely, so `>+` over `a` produced `"a"` rather than `"a\n"`.
Fixing the folding rule in `decode_block_folded` cleared `4Q9F`, `7T8X`, `93WF`, `K527`
and `TS54`. What remains under `scalars` is unrelated to folding: zero-indented block
scalars (`DK3J`, `FP8R`), tab handling (`DK95/00`, `K54U`), trailing whitespace
(`L24T/00`, `JEF9/02`), explicit indentation indicators (`M5C3`), and the empty stream
(`AVM7`, filed under `scalars` although it is really a document-level case).

### `JEF9/02`: the one case where we follow `yq` over the suite

[#344](https://github.com/rust-works/succinctly/issues/344) fixed content-less keep-chomped
block scalars, which used to keep the break that ended the *indicator* line — a break that
belongs to the header's `s-b-comment`, not to the scalar. That cleared `2G84/03` (`--- |1+`)
from `scalars` and `JEF9/01` (`- |+` over a spaces-only line) from `structure`, whose blank
line's indentation had been emitted as content.

It also flipped `JEF9/02` — `- |+` over a spaces-only line with **no terminating break** —
into a known failure, because the two oracles contradict each other there. `l-empty` ends in
`b-as-line-feed`, and the suite treats end-of-stream as supplying that break (expecting
`"\n"`); `yq` v4.53.3 does not (giving `""`). We follow `yq`, which is the compatibility
target the golden fixtures and the `yq-drift` CI job pin. The net effect on the ledger is one
case either way.

Three `|+` / `>+` families still diverge from `yq`, none of them specific to content-less
blocks and all of them predating #344 — an A/B of the fix over 432 block-scalar shapes moved
45 toward `yq` and none away, leaving these untouched. A block on a document-start line keeps
a break it should chomp (`--- |+` over a blank line gives `"\n"`, `yq` gives `""`). An
explicit indentation indicator on the *folded* path does not strip that indent from a
spaces-only line (`>1+` over `   ` gives `""`, `yq` gives `"  "`). And a content-less block
with an explicit indicator swallows the line after it, dropping that key outright — `a: |2+`
over `b: 2` loses `b`, which is why the `empty_block_scalars_keep_contentless` fixture orders
its `explicit_indent` case last.

The two `parse` failures (`FH7J`, `UKK6/02`) are also tags.

## Why not wrap an existing parser?

Issue [#49](https://github.com/rust-works/succinctly/issues/49) raised the option of using
libyaml or yaml-rust2 for parsing and emitting our own index bits on top of their event
stream. Measuring first changed the picture:

- The load gap is **not** diffuse unsoundness. It is dominated by two absent features —
  tags (31, plus 2 `parse` cases) and directives (16) — that are additive work on the
  existing parser. The self-inflicted divergence between our own two output paths that
  used to sit here has since been fixed ([#222](https://github.com/rust-works/succinctly/issues/222)).
- The rejection gap is **deliberate**. A hybrid would close it, but only by doing the
  grammar checking that semi-indexing exists to avoid. The correct fix is an opt-in
  validation pass, which does not require a third-party parser.

Against that, a hybrid costs FFI complexity (libyaml), likely `no_std` support, and the
oracle's control over index construction — which is what the ARM NEON and P9 streaming work
is built on. The evidence does not support the trade. **Rejected.**

## Provenance

The corpus is vendored at a pinned upstream tag so `cargo test` needs no network and the
exact conformance input is reviewable in-tree:

| Artifact         | Path                                                                                                             |
|------------------|------------------------------------------------------------------------------------------------------------------|
| Corpus           | [`tests/data/yaml-test-suite-2022-01-17.json`](../../../tests/data/yaml-test-suite-2022-01-17.json)               |
| Known failures   | [`tests/data/yaml-test-suite-known-failures.txt`](../../../tests/data/yaml-test-suite-known-failures.txt)         |
| Harness          | [`tests/yaml_test_suite.rs`](../../../tests/yaml_test_suite.rs)                                                   |
| Sync script      | [`scripts/sync-yaml-test-suite.sh`](../../../scripts/sync-yaml-test-suite.sh)                                     |
| Path consistency | [`tests/yq_path_consistency_tests.rs`](../../../tests/yq_path_consistency_tests.rs)                               |
| Path divergences | [`tests/data/yq-path-consistency-known-failures.txt`](../../../tests/data/yq-path-consistency-known-failures.txt) |

The path-consistency harness asserts `-I 0` and pretty output agree on every corpus case
([#222](https://github.com/rust-works/succinctly/issues/222)).

To move to a newer upstream release, bump `SUITE_TAG` in the sync script and re-run it;
changes surface as churn in the known-failures manifest.

### A note on the previous harness

Before this page existed, `tests/yaml_test_suite.rs` was 5040 lines of generated tests
that appeared to run the suite but did not. It covered a hand-picked 253 of 402 cases;
all 64 error cases were `#[ignore]`d, so rejection behavior was never checked; 54 of the
then-failing cases were simply absent; and its expectations had been transcribed by hand,
at least one of them wrongly (`4Q9F` expected `"ab cd\n\nef gh\n"` where upstream says
`"ab cd\nef\n\ngh\n"`, letting a real folding bug pass). It also compared against its own
private YAML-to-JSON converter rather than the shipped one.

That folding bug was real and is now fixed
([#329](https://github.com/rust-works/succinctly/issues/329)); `4Q9F` passes against the
upstream expectation. The hand-transcribed expectation is the failure mode this harness
exists to prevent — an oracle you edit is not an oracle.

The current harness runs every case on every invocation and asserts the failure set
matches the manifest exactly, in both directions — a new failure and a newly passing case
both break the build. Cherry-picking is not available.

## Depends On

- [YamlIndex](../../parsing/yaml-index.md) - the structure being tested
- [YAML 1.2 Compliance](1.2.md) - scalar type resolution rules

## Used By

- [yq benchmarks](../../benchmarks/yq.md) - feature comparison against `yq`

## Source & Docs

- [`tests/yaml_test_suite.rs`](../../../tests/yaml_test_suite.rs) - the harness
- [`src/yaml/`](../../../src/yaml/) - parser and index
- [YAML Test Suite](https://github.com/yaml/yaml-test-suite) - upstream corpus
- [YAML 1.2.2 specification](https://yaml.org/spec/1.2.2/)
