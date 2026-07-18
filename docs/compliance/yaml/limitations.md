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
see [YAML 1.2 Compliance](1.2.md); its known divergences from the 1.2 core schema
(`Null`/`NULL`, hex and octal ints, bare `nan`/`inf`) are tracked in
[#226](https://github.com/rust-works/succinctly/issues/226).

## Summary

Measured against suite tag `data-2022-01-17` (402 cases), on the library's streaming
path:

| Dimension                              | Result              | Meaning                                        |
|----------------------------------------|---------------------|------------------------------------------------|
| **Load** (valid YAML, output compared) | **202/279 = 72.4%** | Parses and produces the JSON the suite expects |
| **Reject** (invalid YAML, must fail)   | **11/94 = 11.7%**   | Correctly refuses malformed input              |
| **Parse** (valid YAML, no JSON form)   | **27/29 = 93.1%**   | Parses without error                           |

The 162 non-passing cases are enumerated individually, with a category and reason, in
[`tests/data/yaml-test-suite-known-failures.txt`](../../../tests/data/yaml-test-suite-known-failures.txt).
That file is the machine-readable source of truth; the test asserts it matches reality
exactly, so it cannot silently drift from this page.

## Validation is out of scope by design

**succinctly is a non-validating YAML loader.** It rejects 11 of the suite's 94 invalid
documents; the other 83 are accepted and produce a value.

This follows from semi-indexing. The index records *structure* — where values start and
end, and how they nest — not grammar conformance. The parser is a structure recognizer:
it resolves enough context to emit a correct tree for well-formed input, and absorbs
anything it does not recognize as scalar text. Checking the ~1200 productions of the YAML
1.2 grammar is work it deliberately does not do, and that omission is a large part of why
it is 5-10x faster than `yq`.

If you need malformed YAML rejected, you need a validating parser. Passing untrusted or
unverified YAML through succinctly and relying on a parse error to catch problems will
not work.

The 83 accepted-but-invalid documents break down as:

| Category           | Cases | What is not checked                          |
|--------------------|-------|----------------------------------------------|
| `lax:mapping`      | 14    | Mapping and implicit-key rules               |
| `lax:documents`    | 12    | Directive and document-marker placement      |
| `lax:flow`         | 11    | Flow collection syntax                       |
| `lax:tabs`         | 9     | Tabs where indentation is expected           |
| `lax:indentation`  | 7     | Indentation consistency                      |
| `lax:other`        | 7     | Assorted                                     |
| `lax:quoting`      | 6     | Quoting and escape sequences                 |
| `lax:block-scalar` | 6     | Block scalar header validity (`\|--` parses) |
| `lax:anchors`      | 6     | Anchor and alias rules                       |
| `lax:comments`     | 5     | Comment placement                            |

An opt-in validation mode is planned, mirroring the JSON side's existing
`succinctly json validate` / `sjq --validate`. There, validation is a separate pass that
runs before indexing, so the default path pays nothing for it — see
[`src/json/validate.rs`](../../../src/json/validate.rs). The 83 cases above are its
acceptance criteria. Tracked in
[#223](https://github.com/rust-works/succinctly/issues/223).

## Unsupported features

These are absent rather than wrong, and account for 47 of the 77 load failures.

### Tags — 33 cases (31 load, 2 parse)

`!!str`, `!custom`, and verbatim `!<tag:...>` are not supported. In block context the
parser rejects them outright:

```
$ echo 'a: !!str 1' | succinctly yq '.'
Error: YAML parse error: tags (!) not supported at offset 3
```

In flow context they are silently absorbed into the scalar instead, so `[!!str a]` yields
the string `"!!str a"` — silently wrong data rather than an error. Tracked in
[#224](https://github.com/rust-works/succinctly/issues/224), which covers both tag support
and that inconsistency.

### Directives — 16 cases (all load)

`%YAML` and `%TAG` directives are not recognized. A directive line parses as an ordinary
plain scalar, which also swallows the `---` that follows it:

```
$ printf '%%YAML 1.2\n--- text\n' | succinctly yq '.'
"%YAML 1.2 --- text"      # expected: "text"
```

Tracked in [#225](https://github.com/rust-works/succinctly/issues/225).

## Two output paths that disagree — 8 cases

`succinctly yq` has two YAML-to-JSON implementations and picks between them based on
output formatting:

- **Compact** (`-I 0`) takes the P9 streaming path (`YamlCursor::to_json` / `stream_json`).
- **Pretty** (the default) builds an `OwnedValue` DOM instead.

They do not agree. Across the suite they produce different *values* — not merely different
whitespace — for 29 cases. Where the suite can adjudicate, the DOM path is right 15 times
and the streaming path twice (3 both wrong, 9 have no JSON form to compare against):

```
$ succinctly yq -o json '.'      26DV.yaml   # {"top3": {"scalar1": "scalar3"}}  correct
$ succinctly yq -o json -I 0 '.' 26DV.yaml   # {"top3": {"":        "scalar3"}}  wrong
```

That case uses an alias as a mapping key (`*alias1 : scalar3`); the DOM path resolves it,
the streaming path emits an empty string. Other divergences involve empty keys, block
scalar folding, and trailing tabs.

An indentation flag changing a document's *value* is a bug in its own right, independent
of YAML conformance; tracked in
[#222](https://github.com/rust-works/succinctly/issues/222). This page counts the 8 cases
where the streaming path fails and the DOM path would have passed; the harness tests the
streaming path because that is the library's public API and the one the performance work
targets.

## Full accounting of the 77 load failures

| Category     | Cases | Cause                                                             |
|--------------|-------|-------------------------------------------------------------------|
| `tags`       | 31    | Tags not supported (above)                                        |
| `directives` | 16    | `%YAML` / `%TAG` not recognized (above)                           |
| `scalars`    | 13    | Block scalar folding and chomping edge cases; trailing whitespace |
| `structure`  | 9     | Document end markers; anchors with colons in the name             |
| `streaming`  | 8     | Streaming path diverges from the DOM path (above)                 |

The two `parse` failures (`FH7J`, `UKK6/02`) are also tags.

## Why not wrap an existing parser?

Issue [#49](https://github.com/rust-works/succinctly/issues/49) raised the option of using
libyaml or yaml-rust2 for parsing and emitting our own index bits on top of their event
stream. Measuring first changed the picture:

- The load gap is **not** diffuse unsoundness. It is dominated by two absent features —
  tags (33) and directives (16) — that are additive work on the existing parser, plus a
  self-inflicted divergence between our own two output paths (8).
- The rejection gap is **deliberate**. A hybrid would close it, but only by doing the
  grammar checking that semi-indexing exists to avoid. The correct fix is an opt-in
  validation pass, which does not require a third-party parser.

Against that, a hybrid costs FFI complexity (libyaml), likely `no_std` support, and the
oracle's control over index construction — which is what the ARM NEON and P9 streaming work
is built on. The evidence does not support the trade. **Rejected.**

## Provenance

The corpus is vendored at a pinned upstream tag so `cargo test` needs no network and the
exact conformance input is reviewable in-tree:

| Artifact       | Path                                                                                                      |
|----------------|-----------------------------------------------------------------------------------------------------------|
| Corpus         | [`tests/data/yaml-test-suite-2022-01-17.json`](../../../tests/data/yaml-test-suite-2022-01-17.json)       |
| Known failures | [`tests/data/yaml-test-suite-known-failures.txt`](../../../tests/data/yaml-test-suite-known-failures.txt) |
| Harness        | [`tests/yaml_test_suite.rs`](../../../tests/yaml_test_suite.rs)                                           |
| Sync script    | [`scripts/sync-yaml-test-suite.sh`](../../../scripts/sync-yaml-test-suite.sh)                             |

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
