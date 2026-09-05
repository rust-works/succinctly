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
| **Load** (valid YAML, output compared) | **267/279 = 95.7%** | Parses and produces the JSON the suite expects |
| **Reject** (invalid YAML, must fail)   | **67/94 = 71.3%**   | Refused by the loader or the opt-in validator  |
| **Parse** (valid YAML, no JSON form)   | **29/29 = 100.0%**  | Parses without error                           |

The **Reject** figure is what the conformance harness rejects with the opt-in
validator enabled (loader OR validator, see below). The *default non-validating
loader alone* rejects 16/94 (17.0%) by design — the opt-in validator
([#223](https://github.com/rust-works/succinctly/issues/223)) closes 51 more.

[#224](https://github.com/rust-works/succinctly/issues/224) (tag support) moved the Load
figure from 235/279 (84.2%) to 267/279, the largest single jump on this page, and closed
both `parse` failures. It also *cost* 4 cases on the Reject side: `check_unsupported`'s
blanket `!` rejection used to incidentally reject 4 documents whose real defect is
something else entirely (a malformed tag, an unindented anchor target, a directive with no
document footer before it) — implementing real tag resolution means those 4 are structurally
absorbed like any other invalid input the non-validating loader doesn't check for, dropping
the loader-alone figure from 12/94 to 8/94. See
[Tags — resolved](#tags--resolved-224) below.

By the time [#1374](https://github.com/rust-works/succinctly/issues/1374) (anchor-on-alias
rejection, see
[Three exceptions](#three-exceptions-an-alias-with-no-usable-target-or-an-illegal-decoration-is-rejected)
below) was implemented, re-measuring found the loader-alone figure already at 14/94 rather
than the 8/94 recorded just above — independent drift from other changes since #224 that this
page had not been regenerated against; not investigated further here, since it predates and is
unrelated to #1374's own change. #1374 itself moves the figure from 14/94 to 16/94: 2 corpus
cases (SR86, SU74) that were previously caught only by the opt-in validator are now caught by
the default loader too. This does not move the overall **Reject** figure (still 67/94, itself
also a re-measurement correction from the 66/94 recorded above): both cases were already
counted as rejected via the validator before this change.

The 39 non-passing cases are enumerated individually, with a category and reason, in
[`tests/data/yaml-test-suite-known-failures.txt`](../../../tests/data/yaml-test-suite-known-failures.txt).
That file is the machine-readable source of truth; the test asserts it matches reality
exactly, so it cannot silently drift from this page.

## Validation: default loader vs. the opt-in validator

**The default loader is non-validating by design.** `YamlIndex::build` rejects 16 of the
suite's 94 invalid documents; the other 78 are accepted and produce a value.

This follows from semi-indexing. The index records *structure* — where values start and
end, and how they nest — not grammar conformance. The parser is a structure recognizer:
it resolves enough context to emit a correct tree for well-formed input, and absorbs
anything it does not recognize as scalar text. Checking the ~1200 productions of the YAML
1.2 grammar is work it deliberately does not do, and that omission is a large part of why
it is 5-10x faster than `yq`.

**When you need malformed YAML rejected, use the opt-in validator.**
[`succinctly yaml validate`](../../guides/cli.md) / `syq --validate` runs a separate strict
pass ([`src/yaml/validate.rs`](../../../src/yaml/validate.rs)) before indexing — mirroring
`json validate` / `sjq --validate`, so the default path pays nothing. It rejects **51 of
the 78** documents below with zero false positives on the valid corpus (a guardrail test,
`validator_accepts_all_valid_cases`, enforces that). The conformance harness treats a
must-fail case as handled if the loader *or* the validator rejects it, which is why the
Reject figure above is 67/94 rather than 16/94.

The 27 documents the validator does not yet reject (marked `lax:*` in the manifest) need
deeper structural analysis — cross-line anchor binding, block-scalar content indentation,
flow multi-line implicit keys, malformed tag syntax — and are tracked in
[#223](https://github.com/rust-works/succinctly/issues/223) (`lax:tags` under #224, see
below).

The 27 documents still lax today (neither the loader nor the validator rejects them) break
down as:

| Category           | Cases | What is not checked                                     |
|---------------------|------|-----------------------------------------------------------|
| `lax:mapping`      | 5     | Mapping and implicit-key rules                          |
| `lax:indentation`  | 5     | Indentation consistency                                 |
| `lax:flow`         | 4     | Flow collection syntax                                  |
| `lax:anchors`      | 4     | Anchor and alias rules, incl. an unindented tag target (#224) |
| `lax:documents`    | 3     | Directive and document-marker placement, incl. a mid-stream `%TAG` (#224) |
| `lax:tags`         | 1     | Malformed tag syntax (disallowed flow indicators) — #224 |
| `lax:comments`     | 2     | Comment placement                                       |
| `lax:block-scalar` | 2     | Block scalar header validity (`\|--` parses)             |
| `lax:tabs`         | 1     | Tabs where indentation is expected                       |

This table is the current *remainder* — cases the validator has not yet caught up to — not
a fixed accounting of the original 78; earlier revisions of this page also carried
`lax:other` and `lax:quoting` rows that the validator has since closed out entirely.

The opt-in validation mode now exists ([#223](https://github.com/rust-works/succinctly/issues/223)),
mirroring the JSON side's `succinctly json validate` / `sjq --validate`. Validation is a
separate pass run before indexing, so the default path pays nothing for it — see
[`src/yaml/validate.rs`](../../../src/yaml/validate.rs). The 78 cases above were its
acceptance criteria; it rejects 51 of them today (each `lax:*` line removed from the
manifest is a case now rejected), with the 27 harder cases still tracked in #223 (`lax:tags`
under #224).

### What "accepted" means: the text is kept, not dropped

For an accepted-but-invalid document the loader's job is to produce *some* value without
losing input. Where a construct has no valid reading, the parser absorbs it as scalar text
rather than discarding it — the same principle as the tag handling below.

`- ` followed by content is one such case. It is always the sequence-entry indicator, and
no block sequence can begin inside a flow collection, so `[- x]` is invalid (`yq` rejects
it, and so does the validator). The loader reads it as the plain scalar `"- x"`:

```
$ printf '[- x]\n' | succinctly yq -o json -I0 '.'
["- x"]                   # yq: did not find expected node content
```

Until [#332](https://github.com/rust-works/succinctly/issues/332) this yielded `[null]` —
the content was silently discarded, which is a worse failure mode than either erroring or
keeping the text. A `-` *not* followed by whitespace is an ordinary flow plain scalar and
always was: `[-]` is `["-"]`, `[-1]` is `[-1]`.

The **block** spelling `key: - a` is equally invalid, but it does have a valid reading to
fall back on, so [#325](https://github.com/rust-works/succinctly/issues/325) takes it
rather than absorbing the text: the loader emits the block sequence the author plainly
meant. Absorbing it as scalar text would keep the bytes but still misread the structure,
and unlike the flow case there is a sequence to build here.

```
$ printf 'key: - a\n' | succinctly yq -o json -I0 '.'
{"key":["a"]}             # yq: block sequence entries are not allowed in this context
```

A continuation line at the same column joins that sequence (`key: - a\n     - b` is
`{"key":["a","b"]}`), and a bare `key: -` is an empty item, `{"key":[null]}` — the `-` is
an indicator, so there is no text to preserve. This is the one place the two rules differ,
and the split is deliberate: flow keeps text because no sequence can exist there; block
builds the sequence because one can.

A continuation line can also be invalid the other way: out-dented, sitting strictly between
the sequence's own indent and whatever encloses it, rather than on the mapping key's line.
[#485](https://github.com/rust-works/succinctly/issues/485) takes the same stance as #325 —
the item joins the sequence rather than being dropped:

```
$ printf 'b:\n    - x\n   - y\nc: 2\n' | succinctly yq -o json -I0 '.'
{"b":["x","y"],"c":2}     # yq: did not find expected key
```

Before #485 this lost more than the misaligned item: closing the sequence for the
out-of-range indent reopened a second, untagged sequence as a sibling *child* of the
mapping instead of a value under a key, which corrupted the next entry — here `c: 2` — into
a phantom `"":"c"` pair, with the `2` dropped as well. Content after the misaligned item is
kept too: a further out-dented or realigned line still joins the same sequence.

### Three exceptions: an alias with no usable target, or an illegal decoration, is rejected

Three alias forms are refused at index build:

- **Cyclic.** An alias that would make an anchored value contain itself
  (`a: &x {self: *x}`) fails with `YamlError::AliasCycle` ("anchor value contains itself").
  A cyclic document has no finite tree form; before
  [#153](https://github.com/rust-works/succinctly/issues/153) it aborted the process with a
  stack overflow. YAML 1.2's representation graph technically permits cycles. Unlike the
  unknown-anchor case this is a deliberate divergence from `yq`, which accepts the input
  and emits a depth-limited expansion (`{"a":{"self":{"self":{}}}}`) rather than an error.
- **Unknown.** An alias naming an anchor that is not in scope — never defined, or defined
  *later* — fails with `YamlError::UnknownAnchor` since
  [#372](https://github.com/rust-works/succinctly/issues/372). YAML 1.2 §7.1 requires an
  alias to name a *previous* anchor, so a miss is invalid input rather than a value, and
  `yq` reports `unknown anchor 'x' referenced` for the same input. The strict validator
  enforces this too ([#404](https://github.com/rust-works/succinctly/issues/404)); it did
  not at first, which briefly made the opt-in strict mode *laxer* than the default one for
  this input.
- **Decorated.** An alias node itself carrying an `&anchor` or `!tag`
  (`&y *x`, `!!str *x`) fails with `YamlError::PropertyOnAlias` since
  [#1374](https://github.com/rust-works/succinctly/issues/1374). Unlike the first two, this
  *is* a genuine grammar-conformance check — the YAML 1.2 grammar gives an alias node no
  properties of its own — carved out of the loader's usual "grammar isn't checked" policy
  because leaving it lenient had a specific, concrete cost: `&anchor *alias` is the *only*
  shape that can construct an alias chain more than one hop deep, so every multi-hop-chain
  bug this codebase has ever had (#1193's stack-overflow DoS, #1315/#1319's
  single-hop-only accessor bugs) was reachable only through input real `yq` and PyYAML both
  already refuse. Both are confirmed live to reject every spelling (block value, next line,
  flow, mapping key, `!!str`-tagged) — this is not a case where matching means giving up an
  advantage. (#1318 looked like the same shape at first report but turned out to be a
  different bug — nested `<<` merge-key *traversal*, not an alias chain — and needed its
  own separate fix; see `CHANGELOG.md`.)

```
$ printf 'a: &x {self: *x}\n' | succinctly yq '.'
Error: YAML parse error: cyclic alias 'x' at offset 13 (anchor value contains itself)
$ printf 'a: *x\nb: &x 5\n' | succinctly yq '.'
Error: YAML parse error: unknown anchor 'x' referenced at offset 3
$ printf 'a0: &a0 hello\na1: &a1 *a0\n' | succinctly yq '.'
Error: YAML parse error: alias '*a0' at offset 22 cannot carry an anchor or tag (an alias node has no node properties)
$ printf 'b: &x 5\na: *x\n' | succinctly yq -o json -I0 '.'
{"b":5,"a":5}             # a resolvable, undecorated alias is unaffected
```

Until #372 the second silently yielded `null` — or `""` where the alias was a key — the
same silent-wrong-data mode as the `- ` case above. Before #1374 the third silently built a
resolvable but ungrammatical alias chain instead of erroring.

The first two are the loader's only checks on *document structure*, and neither is grammar
conformance: both are cases where there is no value to produce at all, not cases where the
input breaks a rule the index could otherwise ignore. The third is grammar conformance, but a
single, narrowly-scoped one rather than a step toward general conformance checking — see
above for why it earns an exception. Rejecting the rest of what the suite calls invalid is
the validator's job.

Cyclic and Unknown cost this page's conformance numbers nothing: no case in the corpus
contains an alias to an out-of-scope anchor, so the loader-alone reject figure, the
accepted-but-invalid document count, and the `lax:anchors` row are unmoved by #372. Decorated
does move the loader-alone reject figure (+2, see above) but not the `lax:anchors` row or the
overall Reject figure: both corpus cases it newly catches (SR86, SU74) were already rejected
via the opt-in validator, which has had this exact check
([`check_after_anchor`](../../../src/yaml/validate.rs)) since before #1374.

### A related, differently-timed check: excessively long alias chains (historical — see #1374 below)

Before [#1374](https://github.com/rust-works/succinctly/issues/1374) (Decorated, above), an
alias chain that was merely very long — `a0: &a0 v`, `a1: &a1 *a0`, `a2: &a2 *a1`, ... — was
not rejected at `YamlIndex::build` time; the index built successfully regardless of chain
length. [#153](https://github.com/rust-works/succinctly/issues/153)'s cycle check only rejects
a chain that revisits its own anchor, and a long non-cyclic chain never does. Before
[#1193](https://github.com/rust-works/succinctly/issues/1193), *walking* such a chain (via
`type`, `tostring`, `-o json` output, arithmetic, or any other query that resolves the aliased
value) drove real, uncatchable stack overflow through several independent self-recursive code
paths in `src/yaml/light.rs`, `src/bin/succinctly/yq_runner.rs`, and `src/jq/eval.rs` — the
same underlying failure mode #153 fixed for cycles specifically, recurring for long-but-acyclic
chains that #153's own check does not cover.

#1193's fix was a runtime ceiling, not a build-time rejection: `MAX_ALIAS_CHAIN_DEPTH` (65,536
hops, `src/yaml/light.rs`) bounds every alias-resolving accessor to a clean `panic!` past
that depth, rather than allowing the resolution loop to run unbounded. Because the fix
converts what used to be self-recursion into an explicit iterative loop, the ceiling exists
only to cap adversarial CPU time, not stack depth — the loop itself costs O(1) stack
regardless of chain length.

**This whole scenario is now unreachable through valid input.** `a1: &a1 *a0` — every link in
the example chain above after the first — is exactly the Decorated shape #1374 rejects at
parse time, and it was the *only* way to construct a chain more than one hop deep. A document
`YamlIndex::build` accepts today can therefore never have an alias chain longer than one hop,
so `MAX_ALIAS_CHAIN_DEPTH` and its `panic!` past the ceiling remain in the code purely as
defense in depth against a hand-built index that bypasses the parser's own checks, not as a
mitigation reachable through the CLI on any input the loader itself would accept.

### The other ways `YamlIndex::build` fails

Structure aside, `build` still refuses input it has no reading for at all. None of these is
a conformance check either — they are an absent feature, bytes it cannot tokenize, and the
index's own ceilings:

| Rejected                                        | Example        | `YamlError`           |
|-------------------------------------------------|----------------|-----------------------|
| An unterminated verbatim tag                    | `a: !<foo b: 1`| `InvalidTag`          |
| A tab where block structure expects indentation | `\ta: 1`       | `TabIndentation`      |
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
`lax:flow` cases above survive it.

## Tags — resolved (#224)

`!!str`, `!custom`, and verbatim `!<tag:...>` resolve now, matching real `yq`. The 5
core-schema tags (`!!str`/`!!null`/`!!bool`/`!!int`/`!!float`) force scalar-type coercion —
regardless of quoting style, so `!!int "5"` is the number `5`, not the string `"5"`. Any
other tag (a custom tag, `!!seq`/`!!map`/`!!set`/`!!omap`, a `%TAG`-shorthand tag, verbatim
`!<...>`) does not change resolution — JSON has no way to represent those distinctly from an
ordinary map/seq/scalar anyway.

**Output**: JSON drops every tag, since JSON has no tag syntax. YAML output re-emits the tag
verbatim on the scalar or key it decorated, matching `yq` — but not yet on a whole tagged
mapping/sequence (`!!seq [1, 2]`), which YAML output still drops; JSON was unaffected by this
gap either way, since it drops collection tags too.

```
$ echo 'a: !!str 1' | succinctly yq '.'
a: !!str 1
$ echo 'a: !!str 1' | succinctly yq -o json -I0 '.'
{"a":"1"}
$ echo 'a: [!!str x]' | succinctly yq -o json -I0 '.'
{"a":["x"]}
```

A `!` *inside* plain scalar content is not an indicator and remains ordinary text in both
block and flow context — `[x!y]` is `["x!y"]`, `a: hello!world` is `"hello!world"`. Only a
`!` starting a node is a tag.

`CC74`/`P76L` (`%TAG`-defined shorthand tags applied to a node) and the flow-context cases
[#369](https://github.com/rust-works/succinctly/issues/369) closed (sequence item, mapping
value, mapping key, explicit `? k : v`) all resolve the same way — `%TAG` handle resolution
to a full URI was not implemented, since JSON output never surfaces it; a shorthand tag like
`!e!foo` is tokenized and dropped exactly like any other non-core-schema tag, which is
sufficient for both cases to pass.

One divergence remains, tracked as `tags`/`S4JQ` in the manifest: YAML 1.2's "conventional
resolution" for a bare `!` (the non-specific tag, no suffix) forces `!!str`, but real `yq`
v4.53.3 leaves the content's natural type untouched (`! 12` stays the number `12`, not
`"12"`). We follow `yq`, the same policy as [`FRK4`](#frk4-a-divergence-the-ledger-cannot-see)/
[`JEF9/02`](#jef902-the-one-case-where-we-follow-yq-over-the-suite) below.

**Explicit tag introspection**: `YamlCursor::explicit_tag() -> Option<&str>`
(`src/yaml/light.rs`) returns the literal source tag, distinct from the pre-existing
`YamlCursor::tag()`, which returns an *inferred* type label from the resolved value's shape
(now consistent with `explicit_tag`'s effect, since an explicit core-schema tag changes what
gets inferred). See [docs/parsing/yaml.md § Tag Resolution](../../parsing/yaml.md) for the
storage design (a `BTreeMap<usize, String>` on `YamlIndex`, mirroring the anchor table) and
one documented gap: the lazy, cursor-free half of the jq evaluator (`src/jq/eval_generic.rs`'s
`to_owned`, shared with JSON) is not tag-aware, so `succinctly yq '.a | type'` on a tagged
scalar can still answer from untagged inference even though `succinctly yq '.'`'s JSON output
for the same input is always correct. Tracked as
[#747](https://github.com/rust-works/succinctly/issues/747).

### How the gaps were closed (#664 → #224)

Landing this took two issues. [#664](https://github.com/rust-works/succinctly/issues/664)
audited every scalar/key entry path in `src/yaml/parser.rs` against `check_unsupported` (the
single function that used to reject a leading `!` everywhere) and found 19 distinct entry
paths, 13 of them ungated — meaning 13 different places silently absorbed a tag as scalar
text instead of rejecting it, collapsing into 6 root causes: indentation-skip ordering in
`parse_document_line`; `parse_block_node`'s two scalar-dispatch arms never gating themselves
(the highest-leverage gap, since most deferred-value paths funnel through it — including the
mechanism behind corpus case `J7PZ`, `--- !!omap`, where the tag used to be absorbed as a
complete document-root scalar and the sequence on the following lines started a *second*
document); a next-line branch in `parse_mapping_entry` bypassing its own same-line-only gate;
anchor-then-value ordering checking the gate before consuming the anchor with no second check
after; `parse_explicit_value` having zero gates anywhere; and no block-context key parser
gating the key itself. Flow context was already fully gated at every position by contrast —
[#369](https://github.com/rust-works/succinctly/issues/369) had closed it earlier. #664 was
audit-and-test-only: it pinned every gap with a regression test in `tests/yq_cli_tests.rs`
(`test_yaml_tag_gate_gap_*`) rather than fixing them, so #224 would close *actual* gaps
rather than paper over ones nobody had enumerated.

#224 replaced `check_unsupported` (deleted; every one of its non-`!` arms was already a
no-op, so once `!` stopped erroring the function did nothing at all) with real tag parsing —
`Parser::parse_tag`/`scan_tag_extent` for the lexer, `parse_node_properties`/
`record_key_properties` for consuming `&anchor` and/or `!tag` together, in either order, at
every node-start position (mirroring how anchors were already consumed, and fixing all 6 root
causes as one shared change rather than 13 separate patches). The `test_yaml_tag_gate_gap_*`
tests now assert the closed behavior instead of pinning the gap, under their original names.

Two bugs surfaced only once tags could reach positions anchors already could:
- A tag immediately before a multi-line flow value (`k: !!seq\n  [a, b]`) left the parser
  positioned on the line break, so the following `[`/`{` check missed and absorbed the
  bracket as scalar text (corpus case `EHF6`). The same gap already existed for a bare
  anchor in that position, since `parse_anchor`'s own whitespace skip is inline-only too;
  fixed for both by adding a flow-whitespace-aware property-consumption path
  (`parse_flow_node_properties`) at the two flow call sites that need it.
- A bare tag with nothing after it (`- !!str` as the last sequence item) needs a real BP node
  to resolve against, the same as an anchor does — but the null-value-synthesis check that
  used to gate only on "was there an anchor" silently dropped a tag-only property instead of
  resolving it to `""` (corpus case `LE5A`). Fixed by widening that check to "was there any
  property."

## A flow collection as a same-line explicit key

`? []: x` — a *flow* collection followed by a value indicator on the same line —
still diverges from `yq`. The whole `[]: x` is a compact block mapping used as the
key, so the entry has no value:

```
$ printf '? []: x\n' | succinctly yq -o=json -I=0 '.'
{"":"x"}                             # yq: {"":null}
$ printf '? {a: 1}: v\n' | succinctly yq -o=json -I=0 '.'
{"":"v"}                             # yq: {"":null}
```

Both output `""` for the key, so this is a divergence in the *value*, not the key.

`looks_like_mapping_entry` — the predicate the [#346] fix routes through, shared
with the `- k: v` sequence-item path — deliberately returns `false` for a leading
`[` or `{`, so these two spellings keep the pre-[#346] flow-collection handling and
its mid-line exit. Lifting the restriction means teaching
`parse_compact_mapping_entry` to accept a flow-collection key, which puts a rarer
shape's fix on the hot sequence-item path; that trade was not worth taking blind.

The scalar spellings — `? k: v`, `? "a": v`, `? 'a': v`, and the same shapes in
mapping-value and sequence-item position — are correct since [#346], as is the
mirrored value indicator (`? a` / `: b: c`).

[#346]: https://github.com/rust-works/succinctly/issues/346

## Directives — resolved

`%YAML` and `%TAG` directive lines are now recognized and consumed
([#225](https://github.com/rust-works/succinctly/issues/225)). Previously neither was
recognized at all: a directive fell through to the ordinary plain-scalar scanner, which
then also swallowed the `---` that followed it:

```
$ printf '%YAML 1.2\n--- text\n' | succinctly yq '.'
"%YAML 1.2 --- text"      # before #225; now: "text"
```

The loader does not distinguish `%YAML` from `%TAG` from a reserved directive like `%FOO`
— all three are recognized purely by the leading `%` at column 0 outside a document body
and fully discarded, matching the non-validating loader's general philosophy. This also
means a misspelled directive name (`%YAM`, `%YAMLL`) is skipped exactly like a well-formed
one, with no name matching at all. Full `%YAML`-version and `%TAG`-handle semantics are out
of scope for the default loader; the strict validator
([`src/yaml/validate.rs`](../../../src/yaml/validate.rs), #223) already has the more
detailed grammar for the opt-in validation path.

Fixing this exposed two more pre-existing bugs, unrelated to directives, that were only
visible once a directive line stopped masking them:

- **A document-root plain scalar swallowed a following `---`/`...` even without any
  directive involved.** `Document\n---\nname: Bob` produced the single scalar
  `"Document --- name"` followed by `"Bob"`, instead of two documents. The scalar
  continuation loop had no stop condition for a document marker; it now checks for one at
  true document root (`parse_unquoted_value_with_indent_impl` in `src/yaml/parser.rs`).
- **An empty document never produced a node at all**, rather than a `null` one. `---\n...\n`
  produced zero documents instead of one `null` document, so `6ZKB`'s empty middle document
  (`---\n# Empty\n...`) simply vanished from the output, and any directive-only document
  immediately followed by `---`/EOF (`MUS6/02`-`06`) did too. `end_document` now synthesizes
  a null node when nothing was written for the document, mirroring how
  `close_pending_explicit_key` already synthesizes null for a key with no value — guarded so
  a bare `...`/comment with **no** preceding document (`HWV9`, `QT73`) still produces zero
  documents rather than a phantom one.

Two of the sixteen `directives`-category corpus cases did not fully clear, for reasons
unrelated to directive recognition itself: `CC74`/`P76L` apply the `%TAG`-defined shorthand
to a node, which is `tags`/#224's job (see above); `W4TN` contains a document-root block
scalar with content at column 0 (`--- |\n%!PS-Adobe-2.0\n...`), the same pre-existing
zero-indented-block-scalar gap as `DK3J`/`FP8R` under `scalars`, below.

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

## Full accounting of the 12 load failures

| Category     | Cases | Cause                                                             |
|--------------|-------|-------------------------------------------------------------------|
| `scalars`    | 7     | Zero-indented block scalars; tabs; trailing whitespace            |
| `structure`  | 4     | Document end markers; anchors with colons in the name             |
| `tags`       | 1     | `S4JQ`: non-specific `!` resolution follows `yq` over the spec (above) |

`tags` was 33 (31 load, 2 parse) until [#224](https://github.com/rust-works/succinctly/issues/224)
implemented real tag resolution (see [Tags — resolved](#tags--resolved-224) above); 32 of
those 33 load cases cleared outright, and the 2 parse failures (`FH7J`, `UKK6/02`) cleared
too. Only `S4JQ` remains, for the yq-vs-spec divergence described above.

`directives` was 16 until [#225](https://github.com/rust-works/succinctly/issues/225) (see
[Directives — resolved](#directives--resolved) above); the category no longer exists.
Thirteen cases cleared outright; `CC74` and `P76L` moved to `tags` and `W4TN` to `scalars`,
each blocked on a different, pre-existing gap #225 did not touch. `structure` was 8 until
the same fix: `6XDY`, `7Z25`, `PUW8` and `UT92` were blocked by the document-root
scalar/`---`-swallowing bug #225 fixed alongside the directive gap, not by anything specific
to their own category.

`structure` was 9 until [#407](https://github.com/rust-works/succinctly/issues/407). The
content of a `---` line went through a hand-rolled partial copy of the block-context
dispatch, which opened an empty node for an anchor whose node was on the next line — and a
node at document root *is* a document, so `--- &x` split one document into two and left the
node unanchored. Sharing the one dispatch cleared `FTA2` (`--- &sequence` over a block
sequence) and, off the suite, `--- &x {a: 1}`, `--- ? a` and `--- "a": 1`.

`scalars` was 13 until [#329](https://github.com/rust-works/succinctly/issues/329). Folded
(`>`) block scalars mis-counted the newlines a blank line is worth — a blank line yielded
N+1 where YAML 1.2 §8.1.3 and `yq` give N — and folded blocks with keep chomping (`>+`)
dropped `b-chomped-last` entirely, so `>+` over `a` produced `"a"` rather than `"a\n"`.
Fixing the folding rule in `decode_block_folded` cleared `4Q9F`, `7T8X`, `93WF`, `K527`
and `TS54`. `DK95/00` (a tab used as separation before a plain scalar, retained as
content instead of stripped) was fixed by
[#381](https://github.com/rust-works/succinctly/issues/381). What remains under
`scalars` is unrelated to folding: zero-indented block scalars (`DK3J`, `FP8R`, and
`W4TN` — moved here from `directives` once #225 fixed the directive line that used to
mask this same gap), trailing whitespace (`L24T/00`, `JEF9/02`), explicit indentation
indicators (`M5C3`), and the empty stream (`AVM7`, filed under `scalars` although it
is really a document-level case).

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

### `FRK4`: a divergence the ledger cannot see

`JEF9/02` above is the one case where following `yq` moves a case *across* the ledger.
[#402](https://github.com/rust-works/succinctly/issues/402) added a second place where we
follow `yq` over the spec, and this one does not move the ledger at all — which is why it is
recorded here.

In an explicit **flow** key, a `:` ends the key only when a blank, a line break or end of
input follows it. Before a flow indicator it is ordinary key content, and the scan stops at
the indicator instead:

```
$ printf 'a: [? k :, x]\n' | succinctly yq -o=json -I=0 '.'
{"a":[{"k :":null},"x"]}             # agrees with yq
```

YAML 1.2 §7.3.3 says the opposite — `ns-plain-char` excludes a `:` followed by a
`c-flow-indicator`, so the colon is the value indicator and the key is just `k`. The visible
cost is spec example 7.3, corpus case `FRK4`:

```
$ printf '{\n  ? foo :,\n  : bar,\n}\n' | succinctly yq -o=json -I=0 '.'
{"foo :":null,"":"bar"}              # spec: key is `foo`; yq: rejects the document
```

`yq` rejects `FRK4` outright, so there is no `yq` answer to agree with there — only the rule
it applies everywhere else. `FRK4` is a *parses-only* corpus case (`json: null`), so it still
passes and the known-failures manifest cannot notice the change:
`test_yaml_flow_explicit_key_colon_before_a_flow_indicator_is_content` in
`tests/yq_cli_tests.rs` is the only guard.

The same change made `? ` a node marker in flow mappings rather than key text — `{? k : v}`
used to key on `? k` — and stopped a space before a line break from aborting the key scan, so
`[? k \n  x : v]` now reads the same as `[? k\n  x : v]`. Both of those are plain bug fixes
that move toward `yq` *and* the spec together.

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
