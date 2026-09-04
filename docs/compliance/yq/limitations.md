# yq Behavioural Conformance and Known Divergences

[Home](../../../) > [Docs](../../) > [Compliance](../) > yq Limitations

This page records where `succinctly yq` behaves differently from `mikefarah/yq`, and why.
It is the yq-mode counterpart to
[jq Error Message Conformance](../jq/limitations.md), and it exists because
[ADR-0018](../../adrs/adr-0018.md) requires it: that record makes yq-fidelity the rule for
yq mode, permits divergence only under four named conditions, and obliges every divergence
to be written down. **This page is the enumeration of exceptions to ADR-0018.** A divergence
that is not recorded here is not a decision — it is a bug nobody has found yet.

Everything below was captured from the pinned binary at
[`tests/data/yq-golden/YQ_VERSION`](../../../tests/data/yq-golden/YQ_VERSION) (**v4.53.3**),
with the command shown. Per ADR-0018's rule 1, a claim about real yq's behaviour is
inadmissible here unless it came from that binary — never from recall, and never from
succinctly's own output.

```bash
./scripts/sync-yq-golden.sh          # recapture the golden fixtures from the pinned yq
./scripts/sync-yq-golden.sh --check  # verify they have not drifted
cargo test --features cli --test yq_golden_tests --test yq_cli_tests
```

For YAML *spec* conformance rather than yq behavioural fidelity, see
[YAML Test Suite Conformance](../yaml/limitations.md) and
[YAML 1.2 Compliance](../yaml/1.2.md). For feature coverage and the yq-only surface, see
[yq Query Language Reference](../../reference/yq-language.md).

## Scope note: this page is narrative, not a manifest

The jq page is backed by a probe corpus
([`tests/data/jq-error-probes.tsv`](../../../tests/data/jq-error-probes.tsv)) with a
two-sided manifest check, so it cannot silently drift. **The yq side has no equivalent.**
Golden fixtures and the `yq-drift` CI job pin the cases that *are* captured, but nothing
enumerates the divergences the fixtures do not cover. This page is therefore maintained by
discipline, and it records categories with representative live-verified examples rather than
claiming to be exhaustive. The full current list of known yq-mode gaps is the open issue set
whose titles begin `yq:` — forty-five at the time of writing. Building a yq divergence
manifest to close this hole is worth its own issue.

## Deliberate divergences (ADR-0018 rule 4)

These are the cases where succinctly knowingly does not match real yq. Each is measured
against the four permitted conditions — including the one below that fails them, which is
labelled as such rather than grandfathered.

### Anchor soundness: never emit YAML we cannot read back — rule 4(a)

Real yq emits YAML that real yq then refuses to parse. Two ordinary cases:

```bash
$ printf 'a: &x 1\nb: *x\n' | yq 'del(.a)'
b: *x
$ printf 'a: &x 1\nb: *x\n' | yq 'del(.a)' | yq '.'
Error: bad file '-': yaml: line 1, column 5: unknown anchor 'x' referenced

$ printf 'b: &x 1\na: *x\n' | yq 'sort_keys(.)'
a: *x
b: &x 1
$ printf 'b: &x 1\na: *x\n' | yq 'sort_keys(.)' | yq '.'
Error: bad file '-': yaml: line 1, column 5: unknown anchor 'x' referenced
```

succinctly refuses to produce that output. `enforce_anchor_soundness`
([src/bin/succinctly/yq_runner.rs](../../../src/bin/succinctly/yq_runner.rs), from
[#763](https://github.com/rust-works/succinctly/issues/763)) emits a `*name` only when a
matching `&name` exists, is emitted **earlier**, and holds an **equal** value; otherwise the
mark is dropped and the value printed:

```bash
$ printf 'a: &x 1\nb: *x\n' | succinctly yq 'del(.a)'
b: 1
```

The equal-value clause also covers a genuine gap rather than papering over it: succinctly's
alias sync is one-directional (anchor → aliases), so a write *through* an alias (`.b.p = 9`)
updates only that position where real yq mutates the shared node. Emitting `*x` there would
silently discard the write, so the mark is dropped and the computed value printed — rule
4(b). True alias node identity remains unimplemented
([#1351](https://github.com/rust-works/succinctly/issues/1351)).

**Known gap in this rule.** `enforce_anchor_soundness` takes a `sort_keys` argument and
handles it correctly — but only on the DOM path. The cursor-streaming path never calls it,
so succinctly currently reproduces the unsound output it is supposed to prevent. The two
paths disagree on the same document, which is what makes the gap unambiguous:

```bash
$ printf 'b: &x 1\na: *x\n' | succinctly yq --sort-keys '.'        # streaming
a: *x
b: &x 1
$ printf 'b: &x 1\na: *x\n' | succinctly yq --sort-keys -P '.'     # DOM-forced (-P)
a: 1
b: &x 1

$ printf 'b: &x 1\na: *x\n' | succinctly yq --sort-keys '.' | succinctly yq '.'
Error: YAML parse error: unknown anchor 'x' referenced at offset 3
```

That is [#1350](https://github.com/rust-works/succinctly/issues/1350) — a bug against this
carve-out, not a second carve-out. Related open items in the same family:
[#1359](https://github.com/rust-works/succinctly/issues/1359) (a write that changes a node's
kind drops its `&anchor`, where real yq keeps it),
[#1360](https://github.com/rust-works/succinctly/issues/1360) (NaN false-positives the
equal-value rule), [#1352](https://github.com/rust-works/succinctly/issues/1352) and
[#1353](https://github.com/rust-works/succinctly/issues/1353).

### `-0`/`--nul-output` multi-document separator — rule 4(a)

Real yq's own `-0` output on multi-document input is not readable by real yq itself: the
configured terminator (`\0` here) lands directly before the next document's `---`, with no
newline between them, and real yq's own parser then rejects that byte sequence outright:

```bash
$ printf 'a: 1\n---\nb: 2\n' | yq -0 '.' | od -c
0000000    a   :       1  \0   -   -   -  \n   b   :       2  \0
$ printf 'a: 1\n---\nb: 2\n' | yq -0 '.' > /tmp/x.bin && yq '.' /tmp/x.bin
Error: bad file '/tmp/x.bin': yaml: offset 4: control characters are not allowed
```

succinctly inserts a newline before every `---` document/front-matter separator whenever the
previous write's own terminator (`Terminator`,
[src/bin/succinctly/yq_runner.rs](../../../src/bin/succinctly/yq_runner.rs)) wasn't already
one — `write_doc_marker_newline_guard`, from
[#1701](https://github.com/rust-works/succinctly/issues/1701):

```bash
$ printf 'a: 1\n---\nb: 2\n' | succinctly yq -0 '.' | od -c
0000000    a   :       1  \0  \n   -   -   -  \n   b   :       2  \0
```

This is rule 4(a), not a new carve-out — same shape as anchor soundness above: real yq emits
`---`-boundary output real yq cannot itself re-read, so inserting the missing newline is
permitted rather than bug-for-bug reproduced. The long form, `--join-output`, has no real-yq
counterpart at all (`Error: unknown flag: --join-output`), so for *that* spelling there was
never a reference behaviour to diverge from in the first place. The short form, `-j`, is a
different story — it collides with a real yq flag of the same name but an unrelated meaning
(see "`-j`/`--join-output` collides with real yq's own `-j`" below) — but the guard's
own justification doesn't change either way: it's applied to both spellings purely for
internal consistency between them, not for fidelity to anything real yq does with `-j`.

Scope note: this entry covers only the `---`-boundary corruption. `-0` output whose *value
content* itself contains a raw NUL byte is a separate matter, fixed by
[#1709](https://github.com/rust-works/succinctly/issues/1709): succinctly now rejects such
a result the same way real yq does (`can't serialise value because it contains NUL char and
you are using NUL separated output`), matching real yq's own flush-then-error atomicity
(earlier results in a stream are still flushed; the offending result and everything after it
are not) rather than either the whole-document-buffering approach an earlier attempt at that
fix tried and abandoned (PR #1767, +65% peak RSS on a 100MB document) or silently emitting
the raw byte.

Two interactions remain open, neither closed by #1709 itself:

1. This check only fires on bytes that are genuinely unescaped in the rendered output.
   `-o=json -0 '.a'` *without* an explicit `-r` should print a properly JSON-escaped
   `"b\u0000c"` and succeed (live-verified against the pinned oracle, Homebrew yq
   v4.53.3: JSON's default unwrap-scalar setting is `false`, unlike YAML's
   `true` — real yq needs `-r` explicit to unwrap a JSON scalar), but succinctly's own
   `raw_output` resolution unconditionally ORs in `-0`/`-j` regardless of output format, so
   this one combination instead unwraps (bypassing JSON's own escaping) and correctly
   triggers this same NUL check on the resulting raw byte. That's a distinct, pre-existing
   root cause — [#1996](https://github.com/rust-works/succinctly/issues/1996) — not
   something #1709 itself introduced or is scoped to fix.

2. **`--color`/`--colors` combined with `-0` loses the flush-then-error atomicity described
   above.** Real yq's own `--colors -0` still flushes earlier valid results before erroring
   on a later NUL-containing one — live-verified: `yq --colors -0 '.[]'` on `["hello",
   "wor\0ld", "x"]` prints `hello\0` to stdout, then errors on the second element, exit 1.
   succinctly's own `--colors -0` combination instead buffers the *entire* multi-result
   render into one string (the pre-existing, `--color`-only mechanism `colorize_yaml`/
   `colorize_json` need to re-lex ANSI spans across result boundaries) and scans that whole
   buffer once before writing anything — so a NUL anywhere in the stream discards every
   earlier, already-valid result too, printing nothing at all where real yq printed
   `hello\0`. Fixing this properly needs the buffered/color rendering path restructured to
   flush (and re-colorize) per result rather than once for the whole document — a
   materially larger change than #1709's own scope, filed separately as
   [#2004](https://github.com/rust-works/succinctly/issues/2004).

### Merge-flag `+` and `d` combined — **no carve-out; this one is out of policy**

Real yq's `*+d` applies the deep-merge *and* the append, doubling the right operand.
succinctly gives `+` clean priority instead:

```bash
$ printf 'a: [1, 2]\nb: [3, 4]\n' > arr.yaml
$ yq         -o=json -I=0 '.a *=+d .b | .a' arr.yaml    # [3,4,3,4]
$ succinctly yq -o=json -I=0 '.a *=+d .b | .a' arr.yaml # [1,2,3,4]
```

This was accepted as a "documented simplification" of behaviour that is surprising and
untested upstream. Under ADR-0018 rule 4 that is **not** a valid justification — the output
is readable, no data is corrupted and no process dies, so none of the four conditions
applies. It is recorded here as a divergence to be either fixed or re-justified, not as a
settled decision. The plain (non-combined) flags all match real yq:

| Filter | real yq | succinctly |
|---|---|---|
| `.a *= .b` | `[3,4]` | `[3,4]` ✓ |
| `.a *=+ .b` | `[1,2,3,4]` | `[1,2,3,4]` ✓ |
| `.a *=d .b` | `[3,4]` | `[3,4]` ✓ |
| `.a *=+d .b` | `[3,4,3,4]` | `[1,2,3,4]` ✗ |

## Input-spec divergences (below ADR-0018's scope)

ADR-0018 adjudicates *evaluator and CLI behaviour*. A divergence originating in which inputs
the parser accepts, or in how a plain scalar's type resolves against a published spec, sits
below that line: it is settled by the spec succinctly targets and recorded on the relevant
spec page, not by a rule-4 carve-out. This section exists so the case is not mistaken for one.

### YAML 1.1 legacy numeric forms

succinctly resolves plain scalars per the YAML **1.2** core schema
([src/yaml/scalar.rs](../../../src/yaml/scalar.rs)); real yq still accepts several YAML 1.1
numeric spellings:

```bash
$ for v in 1_000 0X2A 0o17 0b101 +0x1A; do printf "a: $v\n" | yq -o=json -I=0 '.a'; done
1000
42
15
Error: json: error calling MarshalJSON for type *yqlib.CandidateNode: … parsing "0b101": invalid syntax
Error: json: error calling MarshalJSON for type *yqlib.CandidateNode: … parsing "+0x1A": invalid syntax
```

succinctly answers `"1_000"`, `"0X2A"`, `15`, `"0b101"`, `"+0x1A"` — the 1.2 reading, with
the non-1.2 spellings staying strings. So it agrees with real yq on `0o17` and diverges on
the other four. The full table lives in
[YAML 1.2 Compliance § Differences from System yq](../yaml/1.2.md#differences-from-system-yq);
it is cross-referenced rather than duplicated here.

An earlier draft filed this under rule 4(a), which was wrong twice over: 4(a) is about output
the reference cannot re-read, and an *error* is not such output; and even read generously it
would cover only the last two spellings, leaving `1_000` → `1000` and `0X2A` → `42` —
perfectly consumable output that succinctly simply declines to match — with no justification
at all. The justification is the spec target, which is why the case belongs here.

## Open divergences (bugs, not decisions)

Representative cases, each live-verified. These are gaps to close, listed here so they are
not rediscovered from scratch.

### `-j`/`--join-output` collides with real yq's own `-j`

Not an [extension](#extensions) for the `-j` spelling — real yq's arg parser does accept
`-j`, so that spelling fails rule 5's own test ("changes the behaviour of no filter the
reference also accepts"), and none of [ADR-0018](../../adrs/adr-0018.md) rule 4's carve-outs
apply either (the output is readable, nothing is corrupted or discarded, and no process
dies). Taken alone, the long form `--join-output` *would* pass rule 5's token test — real yq
has no such flag at all — but `-j` and `--join-output` are two spellings of the one clap
argument, sharing one implementation and behaviour, so the pair is recorded as a single open
divergence here rather than split into "one extension, one bug" by spelling. It's a plain,
unaddressed name collision: `succinctly yq`'s `-j`/`--join-output` implements jq-style "no
separator, concatenate raw" output (apparently ported from `JqCommand`'s own legitimate
`-j`, which does match real jq's `-j`). Real yq's own `-j` means something else entirely — a
deprecated alias for `--tojson` (forces `-o=json`, prints a deprecation warning to stderr).
Confirmed live against the pinned v4.53.3 binary:

```bash
$ yq -j '.' <<< 'a: 1'
Flag --tojson has been deprecated, please use -o=json instead
{
  "a": 1
}
$ yq --join-output '.' <<< 'a: 1'
Error: unknown flag: --join-output
```

A user who knows real yq's `-j` and reasonably expects `succinctly yq -j` to behave the same
way gets something unrelated instead. Long-standing (predates #1701, ported alongside
`JqCommand`'s own `-j`), found and recorded by
[#1710](https://github.com/rust-works/succinctly/issues/1710) — a documentation-only fix
(see [docs/guides/cli.md](../../guides/cli.md) for the matching caveat on the flag's own
listing). Remapping `-j` to real yq's actual `--tojson` meaning, which #1710 also raised, is
a bigger, more disruptive change not attempted there — tracked separately as
[#1731](https://github.com/rust-works/succinctly/issues/1731). "`-0`/`--nul-output` multi-document
separator" above discusses this same flag from a different angle — why `-0`/`-j` output
getting a newline-guarded `---` is a *permitted* rule-4(a) divergence, independent of the
name-collision question here.

### Duplicate mapping keys — the format leak, `.[]` collapse and the sort family are resolved; narrower gaps remain

The subject of [ADR-0018](../../adrs/adr-0018.md)'s worked example.
[#1398](https://github.com/rust-works/succinctly/issues/1398) resolved the two divergences it
was filed against: real yq preserves duplicate keys almost everywhere but collapses them
under iteration, and succinctly used to match neither side consistently — worse, it answered
differently depending on whether the same logical document arrived as JSON or YAML:

```bash
$ printf '{"b":1,"a":2,"b":3}' > dup.json
$ printf 'b: 1\na: 2\nb: 3\n'   > dup.yaml

$ yq            -o=json -I=0 'length' dup.json   # 3
$ succinctly yq -o=json -I=0 'length' dup.json   # 3   (was 2 -- format no longer leaks)
$ succinctly yq -o=json -I=0 'length' dup.yaml   # 3

$ yq            -o=json -I=0 '[.[]]' dup.yaml    # [3,2]   — iteration collapses
$ succinctly yq -o=json -I=0 '[.[]]' dup.yaml    # [3,2]   (was [1,2,3])
```

ADR-0018 rule 2 attributed the format leak to `DocumentFields::keys_dedup()`, which gated on
input *format* where the reference tools decide on *mode*. **That attribution was wrong**:
[#1385](https://github.com/rust-works/succinctly/issues/1385) removed the predicate entirely —
the collapse rule now rides `EvalSemantics::COLLAPSE_DUPLICATE_KEYS`, on the mode axis rule 2
asks for — and the JSON column above did not move, confirming the leak sat upstream of the
evaluator the whole time.

The real cause was `parse_input`'s `InputFormat::Json` arm
([src/bin/succinctly/yq_runner.rs](../../../src/bin/succinctly/yq_runner.rs)): it materialized
JSON input through `to_owned_canonicalizing_numbers`, an `IndexMap`, before any filter ran, so a
repeated key had already collapsed by the time a duplicate-key rule could apply. #1398 closes it
by giving that arm a cursor-native path instead — JSON input now routes through the same
`evaluate_yaml_direct_filtered`/`YamlIndex::mark_json_sourced` cursor evaluator YAML input (and
JSON's own M2 fast path) already used, JSON being a syntactic subset of YAML's flow grammar.

The iteration divergence (`[.[]]`) is a separate axis: real yq collapses under `.[]`/`map(f)`
traversal alone, an inconsistency `COLLAPSE_DUPLICATE_KEYS` deliberately doesn't reproduce (it's
`false` for yq, matching every *other* builtin's preserve behavior) — its own doc comment names
this exact gap as #1398's to fix, not its own. #1398 makes `.[]`/`map(f)` collapse
unconditionally in both modes instead, reusing #1385's `effective_fields`/`collapsed_fields`
rather than adding a second mechanism; the change lands on shared `eval_generic.rs` code, so it
also fixes jq mode's own `[.[]]` as a side effect — one of #1385's five listed jq-mode gaps
(`.`, `length`, `keys`, `keys_unsorted` remain open there).

[#1687](https://github.com/rust-works/succinctly/issues/1687) then closed the next batch of
wildcard-bridge casualties. `sort`, `sort_by`, `unique`, `unique_by`, `min`, `min_by`, `max`,
`max_by` and `reverse` all answer a permutation or subset of their input's *own* elements, so
they now keep those elements as cursors (a `LazySeq` for the array-valued ones, a bare
`OneCursor` for `min`/`max`) instead of decoding them. Real yq preserves duplicates through
every one of the seven it implements; succinctly now matches, and — per #757's own lesson that
a construct on the DOM route loses everything the DOM cannot carry at once — recovers comments,
anchors and flow style through them too:

```bash
$ printf -- '- b: 1\n  a: 2\n  b: 3\n' > dup_arr.yaml

$ yq            -o=json -I=0 'sort_by(.a)' dup_arr.yaml   # [{"b":1,"a":2,"b":3}]
$ succinctly yq -o=json -I=0 'sort_by(.a)' dup_arr.yaml   # [{"b":1,"a":2,"b":3}]  (was [{"b":3,"a":2}])
```

The same change gave `reduce`/`foreach` their first arm in `eval_generic.rs`, and `limit`/`nth`
a real fan-out for a generator `n` — closing an internal contradiction in the first case
(`[keys|.[]] | length` answered 3 while `reduce (keys|.[]) as $k (0; .+1)` answered 2 on the
same document) and the last of `limit`/`nth`'s duplicate-key loss in the second.

**Three gaps #1687 deliberately did not close**, alongside the four below:

- **`group_by`** returns an array *of arrays*. `LazySeq` has no nested-lazy form and
  `OwnedValue::Array(Vec<OwnedValue>)` cannot hold a cursor, so there is no lossless
  representation for it today — it keeps the bridge, and real yq preserves where succinctly
  collapses. The same representation limit as #1102 below, one level up.
- **`while`/`until`** compute their state from step 1 onward, so only the seed `.` could ever
  stay a cursor; folding through an `OwnedValue` state is inherent, not a wiring gap.
- **`reduce`/`foreach`'s bindings.** The fix recovers the *number and order* of the input
  stream's elements, not each element's own shape: the accumulator and every `$x` a pattern
  binds are `OwnedValue` in both evaluators (`substitute_bound_var` takes `&OwnedValue`, and no
  duplicate-key-capable owned type exists in this crate), so an element that gets bound is
  collapsed at the bind. `reduce .[] as $x (null; $x)` therefore still answers
  `{"b":3,"a":2}` where `first(.[])` on the same document answers `{"b":1,"a":2,"b":3}`.

**And one deliberate divergence from real yq, not a gap.** The cursor-backed reordering above
is switched off entirely for a document carrying any `*alias`, via
`DocumentCursor::document_has_aliases`. Reordering can lift an alias above the anchor it
resolves to, and `reverse` on `- &x {p: 1}` / `- *x` does: real yq answers `- *x` then
`- &x {p: 1}`, then rejects its own output with `unknown anchor 'x' referenced` when asked to
read it back (both verified live against v4.53.3). `enforce_anchor_soundness` is what normally
prevents that, but it is a DOM-path pass over a `CommentTree` and the cursor-streaming path has
none to run it over ([#1350](https://github.com/rust-works/succinctly/issues/1350)) — so an
alias-bearing document takes the DOM path unchanged, losing the marks rather than emitting a
file succinctly could not read back. The gate is on *aliases*, not anchors: an unreferenced
`&x` is valid YAML wherever it lands.

Four narrower gaps #1398's fix deliberately left alone remain open, each already filed before
it landed:

- [#1342](https://github.com/rust-works/succinctly/issues/1342) — `paths(node_filter)` still
  collapses (a YAML-only DOM-representation gap, unrelated to the format leak).
- [#1343](https://github.com/rust-works/succinctly/issues/1343) — `-s`/`--eval-all`/`-i`'s DOM
  fallback still routes through `parse_input`/`OwnedValue` for *both* formats (a pre-existing,
  format-symmetric bug, not a new one #1398 introduced).
- [#1344](https://github.com/rust-works/succinctly/issues/1344) — `del`/`with_entries`/
  `map_values`/`tostream`/`walk`/`recurse` fall through `eval_generic.rs`'s wildcard bridge
  (`eval_on_owned`), which still materializes via an `IndexMap` and collapses duplicates for
  both formats today. #1687 closed the same class for the sort family and `reduce`/`foreach`
  above; this list is the remaining membership.
- [#1102](https://github.com/rust-works/succinctly/issues/1102) — object slicing (`.[S:E]` on
  an object) must materialize into an `OwnedValue::Object` to build yq's AST-child-list view
  before slicing it, with no cursor-preserving alternative available (the operation inherently
  needs to reorder/slice entries, not just stream them) — a real representation limit, not a
  missed wiring like the three above.

**[#1975](https://github.com/rust-works/succinctly/issues/1975) found this same `parse_input`/
`to_owned_canonicalizing_numbers` bridge — #1343's still-open DOM fallback for `--slurp`/
`--eval-all`, and `--inplace`'s DOM-forcing flags — had neither the #1194 unpaired-tail check
nor the #1677 malformed-`,`/`:` delimiter check at all, unlike every other route into the
evaluator.** Its own doc comment already claimed to mirror `eval_generic::to_owned_at_depth`
"exactly"; this was the one place that claim wasn't true:

```console
$ printf '{"a" 1, "b": 2}' | succinctly jq -c 'keys_unsorted'
jq: error (at <stdin>:0): Invalid JSON text: expected ':', found '1'         # correct
$ printf '{"a" 1, "b": 2}' | succinctly yq --input-format json --slurp '.[0] | keys_unsorted'
- a
- b                                                                          # WRONG (before this fix)
```

Fixed by adding the same `key_delimiter_ok`/`value_delimiter_ok` checks (object arm) and
`element_gap_ok` check (array arm, #1597's already-extracted helper) `to_owned_at_depth`
already had, plus the missing `ends_unpaired()` check after the object loop.
`key_delimiter_ok`/`value_delimiter_ok` (`src/jq/document.rs`) went from `pub(crate)` to
`pub` for this — `src/bin/succinctly` is a separate crate from the library and had no way
to call them otherwise.

Review of that first pass found the "mirrors `to_owned_at_depth` exactly" claim had two more
exceptions, both the same #1194 class as the delimiter/unpaired-tail gap above and both fixed
the same way:

- A key JSON's grammar never allowed at all (a bare, non-string key like `{123: 1}` — not a
  *decode* failure, which is preserved via its raw span rather than raised on, same as
  `to_owned_at_depth`) used to be silently dropped along with its whole field — the
  `if let Some(key) = ... {}`-with-no-`else` pattern [#1679](https://github.com/rust-works/succinctly/issues/1679)
  already fixed at five other call sites, missed here.
- A structurally malformed value token the semi-index accepted as a span but couldn't
  classify as any JSON token (`[xyz123]`) used to materialize as `null` instead of raising —
  `to_owned_at_depth`'s own `string_decode_error()`/`is_error()` arms (#1247) were missing
  from this function entirely.

[#2262](https://github.com/rust-works/succinctly/issues/2262) extended this same bridge with
the two remaining checks `eval_generic::to_owned_at_depth` — the function this bridge's own
doc comment has always claimed to "mirror ... exactly" — had gained since #1975 landed:
[#2211](https://github.com/rust-works/succinctly/issues/2211)'s `container_gap_ok` (a stray
`,` with *zero* real children, `[,]`/`{,}`) and
[#2243](https://github.com/rust-works/succinctly/issues/2243)'s `trailing_element_gap_ok` (a
stray `,` *after* a real last child, `[1,]`/`{"a":1,}`). Only the second is addable here:
`container_gap_ok` needs a cursor to the container itself, and this function (like
`eval_generic::to_owned_at_depth`) is only ever given a bare `value: &V` — once a container's
child walk is exhausted there is no cursor left to find its opening bracket from. `[,]`/`{,}`
therefore remain silently accepted; `[1,]`/`{"a":1,}` now correctly raise, confirmed live and
CLI-reachable:

```console
$ echo '[1,]' | succinctly yq --slurp --input-format json -o json '.[0]'
Error: Invalid JSON text: expected JSON value, found ']'                    # correct (was: 1, exit 0)
$ echo '[1,]' | yq -p json '.[0]'
json: null unexpected end of JSON input                                    # real yq agrees
```

**Code review on #2262's own PR found the fix above was reachable only for a filter
that forces the DOM path — `.[0]` above, matching this issue's own test suite, not the far
more common plain identity (`.`)** — filed and fixed as
[#2276](https://github.com/rust-works/succinctly/issues/2276). `--slurp` and `--inplace` each
have their own M2 streaming fast path for a plain identity (`--slurp`) or M2-streamable
(`--inplace`) filter that bypasses `to_owned_canonicalizing_numbers_at_depth` entirely,
parsing via `YamlIndex::build`/`mark_json_sourced` instead (JSON is a syntactic subset of
YAML's flow grammar, and #996 already routes JSON through it for M2 streaming's own
duplicate-key/number-formatting reasons) — whose flow-sequence grammar *legitimately* allows
a trailing `,`, so none of #2262's new checks were ever reached on that route. For
`--inplace` this is not just wrong output — it is silent, incorrect data loss:

```console
$ printf '[1,]' > f.json && succinctly yq -i --input-format json '.' f.json; echo $?; cat f.json
0
- 1                                                          # WRONG -- file rewritten with a "healed" value
$ printf '[1,]' > f2.json && yq -i -p json '.' f2.json; echo $?; cat f2.json
Error: bad file 'f2.json': json: null unexpected end of JSON input
1
[1,]                                                         # real yq refuses, file untouched
```

**Two approaches were tried.** Extending `YamlCursor`'s own `DocumentCursor` trait methods
(`container_gap_ok`/`trailing_element_gap_ok`/`preceding_delimiter_ok`) to validate commas in
place when JSON-sourced (via the already-exposed `canonicalize_numbers()` flag) was actually
*built* — it would have kept the same parser and its same parse-time depth guard, so no depth
regression (below) could ever have arisen — and then confirmed live *not to work*: `--slurp`'s
and `--inplace`'s M2 fast path streams cursor results directly
(`stream_json_sequence`/`stream_yaml_sequence`, `YamlCursor::stream_json`/`stream_yaml`)
without ever materializing through `to_owned_cursor_at_depth`, the only place those trait
methods are consulted. The whole point of M2 streaming is to avoid exactly that materialization
step, so the overrides were simply never reached by the code path that needed fixing — `echo
'[1,]' | succinctly yq --slurp --input-format json -o json '.'` still printed `[[1]]` at exit 0
with those overrides in place, confirmed live before reverting them.

Reverted in favor of declining the fast path instead: `any_input_is_json` (reusing #978's own
original run-wide boolean, briefly removed by #996 once #996 fixed *its* own reason for needing
it) now gates `can_inplace_json_fast_path`/`can_inplace_yaml_fast_path` and
`can_slurp_fast_path` specifically (factored into one `fast_path_json_comma_safe` term rather
than `&& !any_input_is_json` copy-pasted into each gate separately, #2276 review), declining
the fast path for JSON-sourced input so both routes fall back to their own `else` arm —
`parse_input`, i.e. this now-fixed materializer — rather than silently accepting a malformed
document. Confirmed live that both `else` arms already rejected the same input correctly before
this fix (reached via a non-M2-streamable filter, `. + []`/`.[0]`), so declining the fast path
needed no further changes to either fallback for the *comma* class of gap. Correctness over
performance given `--inplace`'s destructive potential on a false accept: reverting to the
smaller, already-proven-correct surface rather than risk a second subtle miss in the larger
one under time pressure.

**Declining the fast path this way silently loosened a *different* guard, though**: `YamlIndex`'s
own parser enforces a 128-deep parse-time nesting limit, while
`to_owned_canonicalizing_numbers_at_depth`'s own guard (`assert_nesting_depth`) is a looser,
*panicking* 256-deep one — a different ceiling for a different reason (stack-overflow safety
for the conversion step itself, not fidelity with what the fast path used to reject). Declining
the fast path therefore silently *accepted* JSON nested 129–255 levels deep that the fast path
used to reject at parse time — confirmed live before this correction: 150 levels of `[...]` via
`--slurp` printed the full nested structure at exit 0, where the pre-fix binary (still
unconditionally on `YamlIndex`) correctly rejected it at exit 1. Closed by
`parse_input_m2_parity` (`yq_runner.rs`), a depth-128 pre-check specific to `--slurp`'s and
`--inplace`'s own DOM fallback — deliberately *not* `--eval-all`'s, which never had this
128-deep guarantee to begin with (no M2 fast path of its own to have declined), so tightening
it there would be an unrelated, unasked-for behavior change. Verified to match a pre-fix build
exactly at every boundary value (127/128/129/255/256/257), including an off-by-one an earlier
revision of this same check got wrong in the *stricter* direction (rejecting 128 levels of
plain nesting when `YamlIndex` itself accepts it — its own `nesting_depth` counter is checked
*before* incrementing and never counts a leaf scalar as its own nesting level, so 128 levels of
`[...]` peaks at `nesting_depth == 127` on the innermost bracket, not 128). This incidentally
also means `--slurp`/`--inplace` no longer reach `assert_nesting_depth`'s 256-deep panic at all
(previously reachable, and — for the few minutes the buggy fix stood — silently *more*
reachable, through this same fallback); `--eval-all`'s own, unrelated, pre-existing instance of
that exact panic was unaffected by any of this and was tracked separately as
[#2282](https://github.com/rust-works/succinctly/issues/2282), per this project's own #1098
precedent that a CLI-reachable panic on untrusted input is a real robustness concern worth a
tracking issue even when not newly introduced. Closed at the panic's own source:
`to_owned_canonicalizing_numbers_at_depth` (`yq_runner.rs`) now calls `check_nesting_depth`
(`eval_generic.rs`, #1818's existing catchable sibling of `assert_nesting_depth`) instead of
the panicking guard directly — the same panic-to-`Result` conversion `jq_runner.rs`'s
`json_bytes_to_owned_value_checked` already made for jq mode's own analogous pre-filter parse
path (#1818), for the identical reason: this call sits ahead of any user filter evaluation,
parsing raw external JSON text for `--slurp`/`--eval-all`/`--inplace`, not inside the
evaluator's own hot recursion where a panic is deliberately kept uncatchable. An earlier
revision of this fix instead added a second, parallel pre-check walk ahead of `--eval-all`'s
own call site (mirroring `parse_input_m2_parity`'s shape) — reworked during review in favor of
fixing the guard directly: the parallel-walk approach needed its own, different
depth-counting convention to match `assert_nesting_depth`'s real per-node semantics (which
checks *every* visited value, leaf included, unlike `parse_input_m2_parity`'s container-only
count), got that convention wrong on the first attempt (silently under-rejecting a
leaf-terminated subtree by one level — 256 levels of `[...]` wrapped around a scalar leaf still
panicked even after a naive generalization of the M2-parity walk was in place), duplicated
~130 lines of tree-walking logic that fixing the guard's own call site avoids entirely, and
risked a fidelity mismatch of its own on structurally malformed input (a non-string object key
containing deep nesting would have been walked by the pre-check but never reached by the real
evaluator, which rejects a non-string key immediately without descending into it) — all
avoided by converting the panic at its source instead of predicting it from a second walk.

**Known cost: `--inplace`/`--slurp` with well-formed, duplicate-keyed JSON input still
collapses those duplicates** (`{"a":1,"a":2}` → `{"a":2}`) — `any_input_is_json` is a blanket,
content-independent gate (computed from `--input-format`/file extension alone, never from
whether the document is actually malformed), so it declines the fast path for *every* JSON-
sourced `--slurp`/`--inplace` invocation, not just the malformed ones this fix exists for. This
is the exact #996 regression that gate caused before #996 fixed it at the source for the
*other* two JSON M2 gates (`can_json_fast_path`/`can_yaml_fast_path`, the plain stdout path,
both left untouched by this fix — see below). Accepted deliberately: `--inplace` mutating a
file on a false accept is a correctness/data-safety concern; a collapsed duplicate key on
*well-formed* input is a narrower, already-tracked one
([#1343](https://github.com/rust-works/succinctly/issues/1343), "the DOM fallback for
`--slurp`/`--eval-all`/`--inplace` still collapses duplicate keys for both formats" — already
true before this fix for any filter forcing that fallback; this fix just widens which filters
do). This same collapsing is also what the CLI test suite uses to *prove* the fast path
actually declined for well-formed JSON input, not merely that ordinary output happened to match
either way (`test_yq_slurp_fast_path_rejects_trailing_comma_2276`/
`test_yq_inplace_fast_path_rejects_trailing_comma_leaves_file_untouched_2276`'s own duplicate-
key assertions) — a `[1,2,3]`-shaped well-formed check alone can't distinguish "the fast path
ran" from "silently fell back and produced the same bytes."

**A further collateral cost, spelled out explicitly rather than left implicit in "conservative
for mixed-format"**: `any_input_is_json` is one run-wide boolean with no per-file M2-vs-DOM
switch (a per-file version was not attempted — each of `can_inplace_fast_path`'s two branches
is its own, separately-parsed loop, decided once before either starts, so routing within one
file at a time would mean restructuring both loops into one, a materially larger change for a
cost real yq has no equivalent to weigh against). `succinctly yq -i '.' a.yaml b.json`, where
`a.yaml` alone (genuinely YAML, well-formed) would be safe on the M2 fast path and `b.json` is
what forces the whole run onto the DOM fallback, now collapses `a.yaml`'s own duplicate mapping
keys too — the exact #442/#1343 loss the M2 fast path exists to avoid, even though `a.yaml` in
isolation would not lose them. Pinned by
`test_yq_inplace_mixed_yaml_json_files_collapses_yaml_duplicate_keys_too_2276`.

**The plain stdout fast path (`can_json_fast_path`/`can_yaml_fast_path`, no `--slurp`/
`--inplace`) has the identical underlying gap and was deliberately left alone**, confirmed
live: `succinctly yq --input-format json -o json '.[0]'` on `[1,]` also wrongly returns `1` at
exit 0, with neither flag involved. Its own `else` arm is not this materializer at all, but
`evaluate_yaml_direct_filtered` (#1398's cursor-native evaluator, used for *every* ordinary
`--input-format json` filter regardless of M2 eligibility) — which routes through the same
`YamlIndex`/`mark_json_sourced` flow-grammar path and shares the identical gap, so declining
the fast path here would cost M2's performance for JSON input with no correctness gain. This
is a real, broader, non-destructive sibling gap — tracked as a further follow-up rather than
folded into #2276.

**A pre-existing divergence this widens by one case, not a new one.** Real yq's own
`-p json`/`eval-all -p json --input-format json` path (confirmed live against v4.53.3) is far
more lenient about a comma or colon inside a JSON *object* than #1975's own delimiter checks
already were — it accepts a trailing comma (`{"a":1,}`), a leading one (`{,"a":1}`), a doubled
one (`{"a":1,,}`), and even a missing colon (`{"a" 1}` → `{"a": 1}`) — apparently because its
JSON input is routed through a YAML flow-mapping parser rather than a strict JSON one; a JSON
*array* gets none of that leniency (`[1,]`/`[1 2, 3]` both error). #1975 already rejected the
missing-colon and missing-comma object cases before #2262 touched anything (confirmed: that
divergence predates this issue, was never gated behind `--jq-extensions` or otherwise
softened, and was accepted implicitly by #1975's own goal of mirroring
`eval_generic::to_owned_at_depth`'s *strict* JSON grammar rather than real yq's specific
JSON-via-YAML-flow routing). #2262's `{"a":1,}` addition is the same design choice applied to
one more comma shape, not a new departure from it.

**`length` on an object had the identical missing-check shape, in yq mode too, but this one
is *not* a real-yq divergence fix** — found and fixed as
[#2307](https://github.com/rust-works/succinctly/issues/2307)/
[#2316](https://github.com/rust-works/succinctly/pull/2316), the yq-mode half of a fix
`jq`'s own limitations.md documents in full (its "`length` (objects)" entry). `document.rs`'s
`checked_len` (this mode's `collapse: false` field-count walk, shared with `census`'s jq-mode
`collapse: true` one) never checked for a trailing stray comma after the object's real last
field at all. The same `census`/`checked_len` code is reached from *both* routes -- the
plain-stdout cursor-native path (`evaluate_yaml_direct_filtered`) evaluates through the same
`eval_generic.rs` `Length` arm as `--slurp`/`--eval-all`/`--inplace`'s own DOM materialization
bridge (`to_owned_canonicalizing_numbers_at_depth`) -- but the *cursor type* differs: the
plain path's `YamlCursor` never overrides `trailing_element_gap_ok`/`scalar_text_end`, so this
fix is a genuine no-op there (verified live, unchanged before/after); the DOM bridge's
`JsonCursor` does override them, so the fix's only observable effect is on that one route --
the same deliberately *stricter-than-real-yq* JSON grammar #1975/#2262 already established
above:

```console
$ printf '{"a":1,}' | succinctly yq --input-format json --slurp '.[0] | length'
1                                                            # WRONG on this bridge's own strict-JSON policy (was: silently accepted)
$ printf '{"a":1,}' | succinctly yq --input-format json 'length'
1                                                            # unaffected, correct -- checked_len runs but the check is a no-op on a YamlCursor
$ printf '{"a":1,}' | yq -p json 'length'
1                                                            # real yq is lenient here too (no --slurp equivalent exists to compare against)
```

Real yq has no oracle to check the `--slurp` bridge's own behavior against at all (it has no
`--slurp` flag, and its one JSON-input route, `-p json`, is the same lenient one #1975 already
chose not to mirror for other comma shapes) -- this fix is internal consistency with the DOM
bridge's own established strictness policy, not a new real-yq-parity claim, exactly like every
other #1975/#2262 comma check on this bridge. Only reachable via `--input-format json`: native
YAML flow-maps *legitimately* allow a trailing comma on every route, confirmed live against
the pinned v4.53.3 (`{a: 1,} | length` answers `1` at exit 0 there too), so this fix leaves
YAML's own leniency untouched everywhere. Same residual gap as the jq-mode fix: a trailing
comma before a *container*-typed last value (`{"a":[1,2],}`) still slips through on this
bridge, inherited from `trailing_element_gap_ok`'s own unconditional container skip (#2243) --
pinned by `test_yq_length_object_trailing_comma_container_last_value_still_a_known_gap_2307`.

Pulling the other way: [#442](https://github.com/rust-works/succinctly/issues/442),
[#478](https://github.com/rust-works/succinctly/issues/478),
[#868](https://github.com/rust-works/succinctly/issues/868) and
[#757](https://github.com/rust-works/succinctly/issues/757) are closed decisions that
deliberately made output *preserve* duplicate keys. They stand for yq mode; ADR-0018 rules 2
and 3 revise them for jq mode only.

#757 is the same shape as #442/#478 one route further out, and worth reading as the general
lesson rather than a fourth incident: `map(...)` was absent from `can_use_m2_streaming`, so
every `map` query fell to the `OwnedValue` DOM path — and an `IndexMap` cannot represent a
repeated key. Because the loss was a *side effect of routing* rather than of any rule about
duplicates, it arrived bundled with four more losses that have nothing to do with duplicate
keys (comments, anchors/aliases, flow style, quoted-scalar style) plus, under `-I0`, nested
containers emitted at their parent's indent — output that reads back with the whole nested
value gone. `.[]` and `select()` on the identical input were already correct, because they
already streamed. Every one of those was fixed by the same one-line gate change plus a
cursor-level sequence writer, and is now pinned by
[tests/data/yq-golden/cases/map_*](../../../tests/data/yq-golden/cases). **When auditing
what a construct loses, the question to ask is which output route it takes, not which rule it
implements** — a construct on the DOM route loses everything the DOM cannot carry, all at
once.

[#1693](https://github.com/rust-works/succinctly/issues/1693) and
[#1700](https://github.com/rust-works/succinctly/issues/1700) are that lesson arriving again,
and the cleanest confirmation of it, because the routing change was made deliberately and for
an unrelated reason. `--ascii-output` was routed to the DOM emitter because that was the only
renderer implementing `\uXXXX` escaping — and it immediately lost three things that have
nothing to do with ASCII: duplicate mapping keys (`a: 1 / a: 2` came back as `{"a":2}`), an
explicit `!!str` tag's typing (`!!str 5` came back as the number `5`, where both real yq and
succinctly's own streaming path give `"5"`), and `\L`/`\P`'s `\u2028`/`\u2029` escaping.
#1700 moved the escaping to the output *sink* (`AsciiEscapeWriter`, `src/jq/escape.rs`)
instead of into the streamers, which returns the flag to the streaming route and recovers all
three at once. The DOM emitter's own `!!str` and `\L`/`\P` divergences are unaffected by that
fix and remain live for the routes that genuinely materialize — `-P` without `-I0`, `--arg`/
`--argjson`, `-r`/`-j`/`-0`, `--eval-all`, `--split-exp` (each verified live against this
binary; `--slurpfile` is a jq flag `succinctly yq` does not accept, and `-P -I0` streams,
because `-I0`'s own `compact` already satisfies the gate) —
[#1982](https://github.com/rust-works/succinctly/issues/1982).

`scripts/streaming-dom-diff.py` mechanizes the routing-divergence lesson above: it runs a
corpus of documents through both output routes (forcing DOM via `--arg unused 1`, which
changes nothing else about the output config) and reports which documents diverge, so a flag
or filter under review can be checked against the pre-existing divergence set instead of
requiring a reviewer to remember it ([#1994](https://github.com/rust-works/succinctly/issues/1994)).

The `-I0` nesting bug that surfaced along the way was *wider* than `map`: any filter
`can_use_m2_streaming` rejects (`to_entries`, `with_entries`, `walk`, `--arg`-bearing
queries, ...) reached the DOM emitter with an empty per-level indent string at `-I0`,
corrupting nested containers on read-back regardless of which construct routed it there —
the earlier claim that `-P -I=0` was already correct for this case was itself false
(verified live against the pre-#1575 binary: `-P` doesn't force the DOM path for an
M2-streamable filter, since `can_yaml_fast_path`'s gate is already satisfied by `-I0`'s own
`compact` flag regardless of `-P`), not merely superseded by the fix below. #757 only
closed the route `map` took into it; the underlying DOM-emitter bug itself was fixed
generally, for every construct that reaches the DOM path, by
[#1575](https://github.com/rust-works/succinctly/issues/1575) — `OutputConfig::compute_indent_str`
now clamps `-I0`'s YAML indent width to 2 (the same convention the M2 streaming path already
used for its own `-I0`) instead of collapsing to an empty string.

**Residual divergence, deliberately not chased by #1575**: real yq's own `-I0` is not width
2 — it's empirically identical to its own `-I4` output (verified live against v4.53.3),
plus an irregular per-level quirk beyond even that (a compact block-sequence item's inlined
mapping doesn't increment its own indent depth for the purpose of computing its fields'
children, so the jump from the mapping's own line to its first nested level is narrower
than every jump after it). Neither succinctly output route models this — both settle for a
uniform width-2 step, matching each other but not the oracle. Widening to width 4 alone
would still not close the gap (the irregular per-level part is unmodeled by both routes at
every non-default `-I` width, not just `-I0`), so this remains open rather than folded into
#1575's fix.

**One more deliberate divergence, this time between two `succinctly`-internal walks rather
than against real yq**: `path(f)`
([`path_walk_generic`](../../../src/jq/eval_generic.rs), #2075) and `key`/`path`/`parent`
([`path_context_step_generic`](../../../src/jq/eval_generic.rs), #2061) both delegate to the
same shared `path_step_generic`, but pass it different duplicate-key collapse flags —
[#2147](https://github.com/rust-works/succinctly/issues/2147) recorded why, since the two
call sites sit ~500 lines apart and the mismatch reads as an oversight without this:

```bash
$ printf '{"a":1,"a":2}' | succinctly yq -o=json -I=0 '[path(.[])]'   # [["a"],["a"]]
$ printf '{"a":1,"a":2}' | succinctly yq -o=json -I=0 '[.[] | key]'   # ["a"]
```

`key`/`path`/`parent` pass `true` unconditionally because they replace a bridge that used to
materialize into an `IndexMap` (which collapses structurally in both modes) — real yq agrees
(`["a"]` in v4.53.3). `path(f)` passes `S::COLLAPSE_DUPLICATE_KEYS` (the mode's own rule)
instead, because it isn't replacing a materialization and has **no yq oracle at all**: real
yq's `path` is the no-argument form only, so `path(f)` is a jq builtin succinctly exposes as
an extension in yq mode, exempt from ADR-0018's divergence rule. In jq mode both walks agree
with real jq 1.7.1 (`[["a"]]`). So the two flags are a recorded choice, not a bug — this
entry (plus the cross-referencing doc comments on both functions) is what keeps a future
reader from "fixing" the mismatch into a real divergence.

### Stacked compact block-sequence items — one narrow, pre-existing width anomaly

[#1485](https://github.com/rust-works/succinctly/issues/1485) fixed the general rule for a
compact block-sequence item's inlined field/element (`- ` sharing its line with a mapping
value's first field, #785): content nested *inside* that first field's own value steps from
the item's own pre-compact indent (`recursion_base`) by an ordinary `indent_spaces` amount —
not from the item's 2-column-wider visual column — with one width-dependent correction at
real yq's own default (`-I=2`, where a single ordinary step wouldn't clear the compact visual
column). This also fixed a **stacked** chain, where a block sequence is itself the compact
value nested directly inside *another* compact sequence element (`- - b: ...`, a sequence
whose sole element is itself a sequence whose sole element is a mapping): every level must
keep forwarding the *original* pre-compact base through every compact level, not just its
own immediate one.

**One narrow anomaly survives, unfixed, at a specific (nesting depth, `-I`) intersection**:
for a chain of exactly `D` stacked compact sequence levels, real yq's own column for the
first nested field is non-monotonic in `-I` right at `-I = D + 1` — verified live against yq
v4.53.3 across `-I=2` through `-I=6`, both before and after #1485's fix, and confirmed
byte-identical on pre-#1485 `main` too (this is not something #1485 introduced or worsened).
For `D = 2` (`printf -- '- - b:\n      c:\n        d: 1\n' | yq -I=<n> '.'`), the observed
column of `c` is 6/6/8/5/6 for `-I` = 2/3/4/5/6 — genuinely non-monotonic (`-I=5`'s column is
*smaller* than `-I=2`'s), not just a different linear step per width. The anomaly sits at
`-I=3` (`D+1`): real yq's own column there (6) breaks what would otherwise be a strictly
increasing sequence (2→6, 4→8, 5→…, 6→…) by repeating `-I=2`'s value instead of continuing
upward to 7.

Both succinctly output routes give column 7 at `-I=3` there (an ordinary, monotonic step
matching the `-I=2`→`-I=4`→... progression), matching each other and matching pre-#1485
`main`'s own (also non-matching) value — neither has ever modeled this specific anomaly. The
pattern (`-I = stacked_depth + 1`) suggests a genuine upstream go-yaml quirk tied to some
internal indent-tracking edge case rather than a spec-conformant rule, but the exact
mechanism is not yet understood; tracked as
[#1881](https://github.com/rust-works/succinctly/issues/1881) rather than blocking #1485's
otherwise-verified fix (which is strictly more correct than what it replaced: it repairs a
real regression at `-I=4` introduced mid-fix and two cases pre-#1485 `main` never got right
at all, `-I=5`/`-I=6`).

### `input`, `inputs`, `input_line_number` — resolved as a call target, matching real yq's lexer

Real yq has no such builtins at any arity — its lexer rejects the identifiers exactly as it
rejects a name that does not exist
([#1507](https://github.com/rust-works/succinctly/issues/1507)):

```bash
$ printf 'a: 1\n' | yq 'input'       # Error: 1:1: lexer: invalid input text "input"
$ printf 'a: 1\n' | yq 'inputs'      # Error: 1:1: lexer: invalid input text "inputs"
$ printf 'a: 1\n' | yq 'no_such_fn'  # Error: 1:1: lexer: invalid input text "no_such_fn"
```

succinctly now matches this unconditionally *as a call target*, in the parser
(`reject_in_yq_mode`, [src/jq/parser.rs](../../../src/jq/parser.rs)) rather than at dispatch
(`input_builtins_unsupported_in_yq_mode`, [src/jq/eval.rs](../../../src/jq/eval.rs), added by
#723):

```bash
$ printf 'a: 1\n' | succinctly yq 'input'
Error: parse error: parse error at position 0: input is not supported in yq mode
```

This closes the call-target divergence this section used to record: dispatch only fires for a
call site that's actually *reached*, so `if false then input else . end` used to be silently
accepted instead of rejected — pinned by
`test_yq_unreached_input_builtin_now_rejected_1507`
([tests/yq_cli_tests.rs](../../../tests/yq_cli_tests.rs)). It does **not** close every route to
these three names: a bare object-construction key (`{input: 5}`) bypasses `reject_in_yq_mode`
entirely, since that parses through a different code path
(`parse_object_construction`'s bare-identifier-key branch) that never reaches
`try_parse_builtin`. That's one instance of a wider, pre-existing, undocumented divergence —
succinctly's object-key grammar accepts jq's permissive bare identifiers unconditionally,
with no yq-mode restriction, for *any* name (`{foo: 5}` succeeds the same way) — tracked
separately as [#1966](https://github.com/rust-works/succinctly/issues/1966) rather than folded
into this fix, since properly closing it means deciding how strict `succinctly yq`'s
object-key grammar should be relative to jq's, not a small patch scoped to 3 names.

**Deliberately *not* routed through the `--jq-extensions` gate** the ~65 other jq-only
builtins use, even though real yq lacks all of them the same way. That flag's contract
elsewhere is "pass this and the builtin becomes usable" — but no flag value makes these three
usable, since yq mode's document loop is cursor-native (`YamlValue::Sequence(docs)` walked by
`uncons_cursor`, [src/bin/succinctly/yq_runner.rs](../../../src/bin/succinctly/yq_runner.rs))
precisely so duplicate mapping keys (#1398) and ADR-0017's comment/anchor side-trees survive;
jq's queue is `Vec<(OwnedValue, u32, u32)>`. Reusing jq's queue would push YAML documents
through `OwnedValue` and silently lose all three, so real support needs a second,
cursor-native queue — tracked as its own follow-up, not attempted here (#1507's own "Option
A"). An earlier version of this fix *did* route them through `--jq-extensions`, and that
reopened the exact divergence above one layer down: the flag let the keyword parse, so the
dispatch-time check's reachability-dependence came right back for the unreached-branch case,
just gated behind an opt-in flag instead of the unconditional default. Rejecting in the
parser regardless of the flag closes it for good, and matches real yq's own behavior more
precisely besides -- there is no `--jq-extensions`-shaped escape hatch in real yq either.

`input_builtins_unsupported_in_yq_mode` stays in `eval.rs` even though the parser now catches
every CLI-reachable case: `Expr`, `Builtin`, `eval`, and `YqSemantics` are public API
(`succinctly::jq`), so a library consumer can construct `Expr::Builtin(Builtin::Input)`
directly and evaluate it with `YqSemantics`, bypassing the parser (and any CLI flag) entirely
— deleting the check would reopen #723's bug for that surface specifically.

### 3-argument `sub(re; s; flags)` — resolved, real yq ignores everything past the pattern

[#1122](https://github.com/rust-works/succinctly/issues/1122) resolved what had looked like an
inconsistent mystery: real yq's `sub(re; replacement; flags)` never evaluates `replacement` or
`flags` at all (near-certainly an upstream Go implementation bug reading its replacement from a
fixed AST slot that's empty once arity exceeds 2, not a designed feature) — it always performs a
global replace-with-empty-string using only the pattern. Confirmed live: an `error(...)` placed
in either `replacement` or `flags` never fires. Per [ADR-0018](../../adrs/adr-0018.md) rule 3,
succinctly now reproduces this bug-for-bug rather than "fixing" it into jq's model:

```bash
$ echo '"aaa"' | yq            'sub("a";"X";"g")'   # ""   (global match, empty replace)
$ echo '"aaa"' | succinctly yq 'sub("a";"X";"g")'   # ""   (matches)
$ echo '"aaa"' | yq            'sub("A";"X";"i")'   # "aaa" (flags never read, no match)
$ echo '"aaa"' | succinctly yq 'sub("A";"X";"i")'   # "aaa" (matches)
```

### 2-argument `split(re; flags)` — resolved, real yq ignores every argument once arity exceeds 1

`split`'s 2-arg `split(re; flags)` form had an unrelated mystery of its own, now resolved
by [#1439](https://github.com/rust-works/succinctly/issues/1439): the same fixed-AST-slot
shape as `sub` above, not a designed feature. Real yq's `split` ignores every argument once
arity exceeds 1 and behaves exactly as `split("")` — splitting the input into individual
Unicode characters, not a regex split, and not jq's `split/2`. Confirmed live: an
`error(...)` placed in either the pattern or the flags argument never fires, and neither a
present nor absent literal pattern changes the output at all:

```bash
$ echo '"a1b2c"' | yq            -o=json 'split("[0-9]";"g")'   # ["a","1","b","2","c"]
$ echo '"a1b2c"' | succinctly yq -o=json 'split("[0-9]";"g")'   # ["a","1","b","2","c"] (matches)
$ echo '"a,b,c"' | yq            -o=json 'split(",";"g")'       # ["a",",","b",",","c"] -- pattern is present, still irrelevant
$ echo '"a,b,c"' | succinctly yq -o=json 'split(",";"g")'       # ["a",",","b",",","c"] (matches)
```

Per [ADR-0018](../../adrs/adr-0018.md) rule 3, succinctly reproduces this bug-for-bug rather
than performing an actual regex split; arity 3+ is accepted and discarded (parser leniency),
matching `sub`'s own arity 4+ handling above. The 1-arg `split(s)` form is unaffected — its
own argument really is evaluated and used (`split(error("boom"))` still raises, confirmed
live) — and `succinctly jq`'s `split/2` keeps its real jq-modeled regex-split behavior
unchanged in jq mode. Non-string-input error wording (`array (...) cannot be matched, as it
is not a string` vs. real yq's `cannot split !!seq, can only split strings`) is a separate,
pre-existing gap unrelated to arity, not addressed here.

### Global regex zero-width-match iteration — resolved, real yq uses Go's `regexp`

[#1255](https://github.com/rust-works/succinctly/issues/1255) resolved a divergence in
`global_captures` (the shared iteration `sub`, `match(re;"g")`, and `capture(re;"g")` all use):
real yq's Go `regexp` engine skips an empty match that begins exactly where the previous
*emitted* match ended; Oniguruma (jq's own engine, and jq mode here) allows it. Everything
else — advance one rune past an empty match, leftmost-first, left-to-right scan — is
identical between the two engines, so this is one extra skip condition in the loop, not a
second regex engine:

```bash
$ echo '"bab"' | yq            'sub("a*"; "X")'   # "XbXbX"  (Go skips the empty match at pos 2)
$ echo '"bab"' | succinctly yq 'sub("a*"; "X")'   # "XbXbX"  (matches)
```

Deliberately **not** extended to `gsub`/`scan`/`splits` (not real yq builtins at all, per
#1436 — there's no oracle for succinctly's own yq-mode extensions to diverge from, so they
stay on the jq-style iteration; see [Extensions](#extensions) below — all three are gated
behind `--jq-extensions` since #1512) or to `split(re;flags)` (a real yq builtin, but with its own
separate, still-unresolved mystery, #1439 above — #1255's fix alone wouldn't make it
oracle-correct given that deeper algorithm mismatch, so the two are deliberately decoupled
rather than guessed at together).

### String interpolation with a multi-valued `\(...)` slot — yq takes the first value only

[#1403](https://github.com/rust-works/succinctly/issues/1403) fixed jq mode's `"\(...)"` string
interpolation to fan out over a multi-valued embedded generator, matching real jq's cartesian
product across every slot (`"\(1,2)-\(3,4)"` → 4 strings, the *first* slot varying fastest).
yq deliberately does **not** get the same fix — live-verified against yq v4.53.3 that a
multi-valued slot silently collapses to its first value alone, not a fan-out:

```bash
$ printf 'a: 1\n' | yq            -o=json '"\(.a,2)"'   # "1" — only one output
$ printf 'a: 1\n' | succinctly yq -o=json '"\(.a,2)"'   # "1" — matches
```

`succinctly yq` keeps its pre-#1403 single-value-taking behavior (`eval_string_interpolation`'s
own doc comment has the exact byte-for-byte reasoning); only jq mode became a genuine generator.

### Generator-argument fan-out — per-builtin, because real yq is not uniform

[#1279](https://github.com/rust-works/succinctly/issues/1279) made jq mode emit one result per
output of a builtin's generator argument, matching jq's own `f(x)` == `x as $b | body`
desugaring. Real yq does **not** do this uniformly, so the gate is per-builtin rather than
per-mode-wholesale — all rows live-verified against yq v4.53.3:

| real yq | fans out? | `succinctly yq` |
|---|---|---|
| `contains` (scalars, arrays, objects) | **yes** — `[.x \| contains(("a","zz"))]` on `abc` is `[true,false]` | fans out; this *closed* a pre-existing divergence |
| `has`, `test/1`, `test/2`, `match`, `capture`, `sub/2`, `sub/3`, `split`, `tz` | no — first output only | gated off (`ArgFanout::yq_native`) |
| `setpath`, `delpaths` | **error** — `SETPATH: expected single path but found 2 results instead` | still errors (`ArgFanout::reject_many_in_yq`); wording differs, see below |
| `flatten(n)` | literal depth only — `bad expression, please check expression syntax` | gated off |
| `getpath`, `range`, `nth`, `limit`, `combinations`, `paths`, `ltrimstr`, `rtrimstr`, `startswith`, `endswith`, `index`, `rindex`, `indices`, `inside`, `splits`, `scan`, `gsub`, `strftime`, `strptime`, `bsearch`, `pow` | **lexer-rejected — the builtin does not exist** | fans out; unopposed extension |

There are **three** gate predicates, so the audit is all three greps —
`grep -E 'ArgFanout::(yq_native|reject_many_in_yq|contains_gate)' src/jq/eval.rs`. Each is named
rather than inlined precisely so that stays a grep (CLAUDE.md's #106 lesson). `contains_gate`
is `contains`'s own gate (#1553): unlike the other two, it does not change whether the argument
fans out (`contains` genuinely fans out in both modes, the table row above) — it only adds the
no-prefix escape rule below.

### The prefix rule is where the two modes deliberately part

jq emits the outputs a prefix earned and *then* fires the argument's trailing control (#1277's
rule 2). Real yq does not: an escape anywhere in a gated argument produces the escape alone,
with nothing printed first — live-verified against v4.53.3 for `has`, `split` and `join`
(#1534), for `delpaths` (#1533), and for `contains` (#1553, `ArgFanout::AllClearedOnEscape` —
the one gate that keeps every value on the *non*-escaping path, unlike the other two):

```bash
$ printf 'a: 1\n' | yq            'has(("a","b", error("boom")))'   # Error: boom, stdout empty
$ printf 'a: 1\n' | succinctly yq 'has(("a","b", error("boom")))'   # Error: boom, stdout empty
```

`clear_values_when_yq_argument_escaped` implements that, called only from `fanout_arg`. So the
jq-mode prefix rule and the yq-mode no-prefix rule are both live, per mode, exactly as ADR-0018
requires — an earlier draft of this section claimed the rule applied "uniformly in both modes",
which was true only before #1534.

### Residues

- **`setpath`/`delpaths` count-message wording.** Real yq says `SETPATH: expected single path but
  found 2 results instead` (and `...single value on RHS but found 2`, and `DELPATHS: expected
  single value but found 2`); succinctly says `expected a single result but found 2`. The
  *outcome* — an error rather than a fan-out or a silent truncation — is what #1279 preserved;
  matching the per-slot wording is unstarted.
- **Two-argument escape ordering** ([#1533](https://github.com/rust-works/succinctly/issues/1533),
  now fully closed). The escape-clearing above is deliberately *not* applied to
  `fanout_two_args`: emptying one slot's values skips the body, which is where the other slot
  gets validated, and which slot real yq reports there is per-builtin and does not follow
  succinctly's outer/inner order (`test` wants the flags, `setpath` wants the path) — that part
  stays a known limitation with no shared rule. But the *specific* case #1533 was filed for —
  `RejectMany`'s own `args.len() > 1` count check masking a real `error(...)` in the same
  slot — needed no per-builtin probe: a count violation there is itself a symptom of the
  escape, not competing evidence against it, so `fanout_two_args` now defers to that slot's
  trailing control whenever its own count check is what's about to fire. Confirmed by
  `test_yq_setpath_two_argument_reject_many_propagates_an_embedded_error_1533`.

  Review found this alone still misattributed the error when *both* slots violate `RejectMany`
  at once — reporting whichever slot (outer) it happened to check first, rather than real yq's
  own consistent "inner always wins" rule (for `setpath`, path over value — live-verified across
  every escaping/clean combination). `fanout_two_args` now always evaluates inner before
  reporting outer's own violation, matching real yq's own evaluation order rather than just its
  final answer. See `test_yq_setpath_reject_many_prefers_inner_violation_over_outer_1533`.
- **`contains` on an escaping argument** ([#1553](https://github.com/rust-works/succinctly/issues/1553),
  now closed). `contains` stays ungated for fan-out — real yq genuinely fans it out, unlike its
  `yq_native`-gated neighbours — but needed its own gate for the no-prefix escape rule, since
  neither existing gate fit: `FirstOnly` would wrongly truncate the ordinary non-error
  multi-output case, and `RejectMany` refuses multi-output outright, which `contains` never
  does. `ArgFanout::AllClearedOnEscape` keeps every value like `All` on the non-escaping path,
  but routes through the same eager, escape-clearing `fanout_arg` machinery `FirstOnly`/
  `RejectMany` already use. Confirmed by
  `test_yq_contains_gate_emits_nothing_when_the_argument_escapes_1553` and its siblings.
- **`contains`/`inside` on a top-level kind mismatch** ([#1649](https://github.com/rust-works/succinctly/issues/1649),
  now closed). `f_contains`'s `jq_kind(a) != jq_kind(b)` screen raises
  `EvalError::containment_check` unconditionally in jq mode — correct there — but real yq
  (v4.53.3, live-verified across every kind pairing, not just the string-vs-number case the
  issue was filed for) only errors when **at least one** operand is container-shaped
  (array/object); a mismatch between two scalars (including a `true`/`false` pairing, which
  `jq_kind` itself still treats as a "mismatch" per #358) answers `false` instead. This is the
  *opposite* of "both operands must be containers to error" — a single container operand
  (array-vs-string, object-vs-string, null-vs-array) is already enough. `contains`/`inside`
  are now gated per `S::TAG` via the shared `containment_kind_mismatch_is_error` (`inside`
  follows the same rule for internal consistency with `contains`, even though real yq has no
  `inside` at all to verify it against — see the fan-out table above). Confirmed by
  `test_yq_contains_scalar_vs_scalar_kind_mismatch_answers_false_1649` and its siblings.

### Regex flag grammar — `test`/`match`/`capture` fixed

**Fixed by [#1426](https://github.com/rust-works/succinctly/issues/1426):** real yq doesn't
use jq's flag grammar at all for `test`/`match`/`capture` — only `g` is a real flag; every
other jq-style character (`i`/`x`/`s`/`m`/`n`/`l`/`p`, including `l`/`n`, ADR-0019's own
permanent *jq*-mode gaps) is rejected, with `i` getting a distinct message pointing at yq's
inline-pattern alternative:

```bash
$ echo '"abc"' | yq            -o=json 'test("abc";"l")'   # Error: unrecognised match params 'l', ...
$ echo '"abc"' | succinctly yq -o=json 'test("abc";"l")'   # was: true (silently accepted); now: the same error
```

Also covers, live-verified: a non-string *scalar* flags value (`test("abc";null)`,
`test("abc";true)`, `test("abc";5)` — real yq stringifies these the same way `tostring`
does and grammar-checks the result, rather than treating `null` as "no flags"), and
ordering against a simultaneously-invalid pattern type (`test(1;"z")` reports the flags
error, matching real yq, not `succinctly`'s own "number (1) is not a string").

Deliberately **not** extended to:
- `sub` — moot rather than unverified now that #1122 resolved its mystery: 3-arg `sub`
  never evaluates its `flags` argument at all, so there is no flags *grammar* to check in
  the first place, valid or garbage.
- `split` — moot rather than unverified now that #1439 resolved its mystery (above):
  2-arg `split(re; flags)` never evaluates its `flags` argument (or its pattern) at all
  once arity reaches 2, so there is no flags *grammar* to check in the first place,
  valid or garbage — the same reasoning as `sub` immediately above.
- `gsub`/`scan`/`splits` (not real yq builtins at all, per #1436 — flag validation is moot
  for a call real yq would reject before ever reaching it).
- The array-unpack form (`test(["abc","i"])`, no explicit flags argument) — real yq's own
  array-unpack support for these three builtins is a no-op that always succeeds regardless
  of the unpacked flags element's content, live-verified even for a flag character (`i`)
  the explicit 2-arg form correctly rejects.
- A non-scalar (`Array`/`Object`) flags value (`test("abc";["g"])`) — real yq returns
  `true` here with no error at all, ruling out "stringify and grammar-check" the way the
  scalar case works; its actual behavior for a container flags value is unconfirmed and
  left as a known, undocumented-elsewhere gap rather than guessed at.

### Regex pattern coercion — scalars fixed, containers still open

**Fixed by [#1443](https://github.com/rust-works/succinctly/issues/1443):** real yq
silently coerces a non-string *pattern* argument to its string representation and
compiles *that* as an ordinary regex — not a literal-text match: the coerced
string's own characters still act as regex metacharacters (a coerced `Float`'s `.`
is a wildcard, not an escaped decimal point). Where succinctly raised
`is_not_a_string`/`not_string_or_array` in both jq and yq mode, it now coerces too
— live-verified against yq v4.53.3 across `test`/`match`/`capture`/2-arg `sub`/
3-arg `sub`/`split` (a real yq builtin, unlike `gsub`/`scan`/`splits`, which share
the same coercion helper but have no real yq behavior to diverge from):

```bash
$ echo '"a1c"' | yq            'test(1)'          # true (1 stringified to "1")
$ echo '"a1c"' | succinctly yq 'test(1)'          # was: Error: number not a string or array; now: true
$ echo '"a1c"' | yq            'sub(1;"X")'       # "aXc"
$ echo '"a1c"' | succinctly yq 'sub(1;"X")'       # was: Error: number (1) is not a string; now: "aXc"
$ echo '"a1X5c"' | yq            'test(1.5)'      # true -- "." is a wildcard, not a literal dot
$ echo '"a1X5c"' | succinctly yq 'test(1.5)'      # was: Error: number (1.5) is not a string; now: true
```

`Null`/`Bool`/`Int`/`Float`/`NumberLiteral` now coerce (`owned_to_string`-equivalent)
before the existing type-check runs; jq mode is unaffected (real jq 1.7.1 keeps
strict typing here too, matching ADR-0018 rule 2). This coercion applied to
2-arg `split(re;flags)`'s pattern too at the time — moot as of #1439 (above): once
arity reaches 2, yq mode never evaluates the pattern at all (it dispatches to the
same `split("")` behavior as every other arity-2+ call, regardless of what the
pattern argument is), so there is no pattern *value* left for this coercion to
apply to in yq mode any more, the same way #1439 also mooted the flags-grammar
question for `split` above. jq mode's own `split/2` is unaffected either way — it
never went through this coercion (strict typing, matching real jq).

Deliberately **not** extended to a container (`Array`/`Object`) pattern — this fix's
own live probes showed real yq doesn't simply error there either, but its exact
stringification rule is a separate, unverified question:

```bash
$ echo '"a1c"' | yq            'sub([1];"X";"g")'   # "a1c" (no match, no error)
$ echo '"a1c"' | succinctly yq 'sub([1];"X";"g")'   # Error: array ([1]) is not a string -- unchanged, known gap
$ echo '"a{}c"' | yq            'sub({};"X";"g")'   # "ac" (coerces to "{}", also compiled as an ordinary regex)
$ echo '"a{}c"' | succinctly yq 'sub({};"X";"g")'   # Error: object ({}) is not a string -- unchanged, known gap
```

### Presentation metadata lost on two whole output routes

Neither route builds a `CommentTree`, so both drop comments, style **and** anchors together:
`--inplace` never builds one ([#1349](https://github.com/rust-works/succinctly/issues/1349)),
and any filter yielding multiple results — anything containing a comma — loses its cursor
before one can be captured
([#1361](https://github.com/rust-works/succinctly/issues/1361)). Both predate
[ADR-0017](../../adrs/adr-0017.md)'s mechanism and neither is anchor-specific.

There used to be a third: `map(...)`, which lost all three the same way for the same reason —
it took the DOM route, which carries none of them.
[#757](https://github.com/rust-works/succinctly/issues/757) closed it by streaming `map`'s
elements from their own cursors (see the duplicate-key section above for the full list of
what that one route was dropping). `--inplace` is *not* an exception here despite #1349:
its M2 fast path shares `stream_cursor!` with stdout, so an M2-eligible `map` keeps comments
and style through `-i` too — #1349 is about the `--inplace` **DOM fallback**, which a
non-M2-eligible filter still reaches.

### An untracked terminal path branch — resolved for `path()`/`=`/`|=`/compound-assigns, still open for `del()` and for trailing navigation

[#1764](https://github.com/rust-works/succinctly/issues/1764): `reject_untracked_at_terminal`
(`eval.rs`) is the shared check answering "is a `path()`/`del()`/`=`/`|=` branch that resolved
to a *computed* value, rather than a real navigation, actually an error" — jq's answer is
always yes (`Invalid path expression with result <v>`), and this check used to raise
unconditionally for both modes.

Real yq's answer for `path()`/`=`/`|=`/the compound-assign operators (`+=`/`-=`/`*=`) is no:
an untracked terminal branch is a silent no-op, and every *other* branch is still written
normally, regardless of position:

```bash
$ echo 'a: 1
b: 2' | yq            '(.a, 1) = 5'   # a: 5\nb: 2 -- untracked "1" contributes nothing
$ echo 'a: 1
b: 2' | succinctly yq '(.a, 1) = 5'   # matches, was: Error: Invalid path expression with result 1
```

(Real yq has no `/=`/`%=`/`//=` syntax at all — confirmed live, `'/'`/`'%'`/`'//'` all
report "expects 2 args but there is 1" rather than being recognized as compound-assign
operators, the same "no real yq syntax to check against" gap already recorded elsewhere in
this file. succinctly's own support for those three forms is a pre-existing, unrelated
extension-surface question — they parse and inherit this same skip in yq mode, but that is
not itself a divergence claim, since there is nothing in real yq to diverge from.)

Confirmed position-independent (untracked first, middle, or last; all-branches-untracked;
a computed value from an arbitrary expression rather than a bare literal) — every case gives
the same "skip the untracked branch, write every other one" result. **Not specific to a
multi-branch `Expr::Comma` despite the check's own name**: a single bare untracked
expression with no comma at all is the identical no-op — `(1) = 5` on `{a: 1}` leaves it
unchanged too, and neither is it specific to a bare-literal *origin* — an untracked value
produced by an upstream mechanism like a `try`/`catch` handler run on a caught error's
payload is computationally identical by the time it reaches the check, and gets the same
"write every trackable branch, skip this one" result rather than aborting the whole write
the way it did (and, in jq mode, still does) before this fix. A genuine error/break/halt
produced *while computing* what would otherwise be an untracked value is not itself
untracked and still propagates normally: `(.a, error("boom")) = 5` still raises `boom`.

**`path()`'s own trailing-iterate case is covered; `=`/`|=`/compound-assign's is not.**
`path()` strips a trailing bare iterate off its own path expression before resolving it
(`defer_trailing_iterate`, #888) and re-splices it after the terminal check runs — so
`path(1 | .[])` *is* covered by this fix (confirmed live: now no-ops in yq mode, matching
the same general rule above, where it used to raise jq's "near attempt to iterate through 1"
wording). `=`/`|=`/compound-assign never defer a trailing iterate at all, so the identical
shape in *their* context — `(.a, 1 | .[]) = 5`, still real yq's own no-op — never reaches
the terminal check: `resolve_node`'s own `Expr::Iterate` arm (and `resolve_index_expr`'s
analogous dynamic-key check) each raise independently and unconditionally first. Fixing that
needs the same kind of skip-untracked awareness threaded into `resolve_node`, a 21-call-site
function with its own scattered, independently jq-mode-verified checks — a materially larger
and riskier surface than this fix's single terminal-position check, so left open rather than
guessed at here. Tracked as [#1868](https://github.com/rust-works/succinctly/issues/1868).

**`del()` does not share this model and is deliberately excluded from the fix.** Its real
behaviour, confirmed live across argument-order permutations, is order-*dependent* in a way
none of its three siblings are:

```bash
$ echo 'a: 1
b: 2' | yq 'del(.a, 1)'      # a: 1\nb: 2  -- nothing deleted
$ echo 'a: 1
b: 2' | yq 'del(1, .a)'      # b: 2       -- .a IS deleted (same two arguments, reversed)
$ echo 'a: 1
b: 2
c: 3' | yq 'del(.a, .c, 1)'  # a: 1\nb: 2\nc: 3 -- nothing deleted, even though .a/.c precede "1"
$ echo 'a: 1
b: 2
c: 3' | yq 'del(.a, 1, .c)'  # a: 1\nb: 2       -- only .c deleted, not .a
```

The pattern across all four is consistent with `del()` processing its targets in *reverse*
of the given argument order and aborting every remaining one — without undoing whatever
already completed — the instant it hits an untracked target. Simply extending the other
three operations' "skip and continue in given order" fix to `del()` would make it delete
*more* than real yq does for some orderings (`del(.a, 1)` would delete `.a`, which real yq
leaves untouched) — a data-loss-shaped divergence in the wrong direction, not a cosmetic
one. `del()` therefore keeps raising `reject_untracked_at_terminal`'s pre-existing error
(via `resolve_del_path_branches`, `del()`'s own path-resolution entry point, which is
what distinguishes its call shape from its three siblings') until the real reverse-order/
abort-without-rollback algorithm is implemented, tracked as
[#1865](https://github.com/rust-works/succinctly/issues/1865) rather than guessed at here.

### `del()` with a field key against a scalar root: errors instead of no-op

Deleting a field key from a scalar document raises `Cannot index <type> with string
"<key>"` in succinctly yq; real yq (v4.53.3) silently no-ops instead, returning the
input unchanged at exit 0. Confirmed live for a static key, a single-branch computed
key, and (after [#2049](https://github.com/rust-works/succinctly/issues/2049)'s fix
for a live `unreachable!()` on this same shape) a mixed field+index multi-branch
computed key:

```bash
$ echo '2.5' | yq            'del(.k0)'            # 2.5, no-op
$ echo '2.5' | succinctly yq 'del(.k0)'             # Error: Cannot index number with string "k0"

$ echo '2.5' | yq            'del(.[("k0",0)])'     # 2.5, no-op
$ echo '2.5' | succinctly yq 'del(.[("k0",0)])'     # Error: Cannot index number with number
```

A purely-field multi-branch computed key (`del(.[("k0","k1")])`) *does* already
no-op correctly — #2049's fix landed there as a side effect of removing the panic,
since that shape reaches `delete_trie_object`'s trie walker directly rather than
`resolve_node`'s earlier, always-erroring field-indexable check. The static,
single-branch-computed and mixed-key shapes above still go through that earlier
check (or, for the mixed case, `delete_trie_array`'s own separate non-array gate)
and still error.

A `null` root against that same purely-field multi-branch key is a third,
non-erroring divergence in the same family: real yq materializes an object shape
(`{}`) rather than leaving it `null`.

```bash
$ echo 'null' | yq            'del(.[("k0","k1")])'   # {}, materializes an object
$ echo 'null' | succinctly yq 'del(.[("k0","k1")])'   # null, unchanged
```

`delete_trie_object`'s pre-existing `Null` branch (untouched by #2049's fix) always
returns the root unchanged. Tracked, along with the field-key-on-scalar-root shapes
above, as [#2106](https://github.com/rust-works/succinctly/issues/2106).

### Comma-grouped scalar-target assignment no-op

[#1233](https://github.com/rust-works/succinctly/issues/1233) taught `=`/`+=`/`-=`/`*=`/`//=`
that real yq's field/index/iterate scalar-target no-op ([#1181](https://github.com/rust-works/succinctly/issues/1181))
discards the RHS entirely rather than merely skipping the write — but only for a *static*
`path_expr` (no computed key, and no `Comma`). A comma-grouped LHS where every branch is
itself a scalar-target no-op is real yq's identical behaviour, live-verified, that
succinctly does not yet replicate:

```bash
$ echo 5 | yq            -o=json '(.a, .b) = error("boom")'   # 5, RHS never runs
$ echo 5 | succinctly yq -o=json '(.a, .b) = error("boom")'   # Error: boom
```

Deliberately not covered by #1233's own fix: resolving a `Comma`-containing path *before*
the RHS (which the fix needs to do, to decide whether to skip it) is real evaluation for
that shape, unlike a bare static path (a pure, non-evaluating AST clone) — and moving real
evaluation ahead of the RHS risks reordering two independently-observable things. That risk
is not hypothetical: real jq's own `.[error(P)] = error(R)` reports `R` (RHS evaluated
first) while real yq's identical query reports `P` (path first) — succinctly currently
matches jq's ordering in both modes, a second, related divergence from real yq neither
tracked before now. Both filed together as
[#1412](https://github.com/rust-works/succinctly/issues/1412), which also notes a real fix
for the ordering divergence would likely resolve the comma-LHS gap as a side effect (a full
path-before-RHS reorder no longer needs the static-only safety gate #1233's own narrower
fix relies on).

### Dynamic-key and comma-grouped scalar-target assignment: the *write itself* fails

[#1232](https://github.com/rust-works/succinctly/issues/1232) widened #1181's scalar-target
no-op to a scalar hit *before* the last path component (`.a.b = 99` on a scalar `.a`
no-ops), but only for the *static*-path walkers (`set_path`/`update_path` — at the time
`=`'s own share of that lived in `get_path_mut`, folded into `set_path_steps` by
[#1429](https://github.com/rust-works/succinctly/issues/1429)).
A path that needs the full `resolve_dynamic_indexes` pre-pass — a computed key, or a
`Comma`-grouped LHS — resolves each component through a plain read evaluator with no yq
scalar-noop awareness at all, so the boundary this section's *previous* entry describes
("no computed key, and no `Comma`") is wider than just the RHS-discard optimization: for
these paths the write genuinely fails, not just the optimization of skipping RHS
evaluation:

```bash
$ printf 'a: 5\n' | yq            -o=json '"a" as $k | .[$k].b = 99'   # a: 5, no-op
$ printf 'a: 5\n' | succinctly yq -o=json '"a" as $k | .[$k].b = 99'   # Error: Cannot index number with string "b"

$ printf 'a: 5\nx: {}\n' | yq            -o=json '(.a.b, .x.y) = 99'   # a: 5, x: {y: 99} -- .x.y still writes
$ printf 'a: 5\nx: {}\n' | succinctly yq -o=json '(.a.b, .x.y) = 99'   # Error: Cannot index number with string "b" -- .x.y write lost too
```

Both live-verified against yq v4.53.3, and confirmed unaffected by #1232's own fix (identical
on `main` immediately before that PR). Filed as
[#1419](https://github.com/rust-works/succinctly/issues/1419), which also covers the
narrower, already-fixed-write case where only the RHS-discard *optimization* is missing
(`0 as $k | .[$k] = error("boom")` on a scalar root still raises `boom` where real yq no-ops
silently, even though the write itself — `.[$k] = 99` — already correctly no-ops).

A pipe-literal reaches the same `resolve_leaf` fallback, not just a computed key or
`Comma`-grouped LHS: `(null | .a) = 5` on `null` errors "Invalid path expression near
attempt to access element \"a\" of null" in `succinctly yq`, where real yq no-ops to `null`
unchanged (live-verified against yq v4.53.3, [#2044](https://github.com/rust-works/succinctly/issues/2044)).

```bash
$ printf 'null\n' | yq            '(null | .a) = 5'   # null, no-op
$ printf 'null\n' | succinctly yq '(null | .a) = 5'   # Error: Invalid path expression near attempt to access element "a" of null
```

**Fixed by [#1298](https://github.com/rust-works/succinctly/issues/1298):** `get_path_mut`
(the walker #1232 widened) used to have no `Expr::Iterate` arm at all, so a mid-chain `.[]`
under plain `=` (`.a[].b = 99`) always errored "invalid path component" — in both jq and yq
mode, and predating #1232 entirely (`|=`/`+=`/`-=`/`*=`/`del()` were unaffected, their own
recursive-descent walkers already supported fan-out). #1298 added `split_at_iterate`/
`set_path_through_iterate` so `=` fans out per element like every other operator (both since
folded into `set_path_steps`' own mid-chain `Iterate` arm by
[#1429](https://github.com/rust-works/succinctly/issues/1429), which replaced the whole
pre-scan with one peel-and-recurse walk — same behaviour, verified byte-identical), and gave
`navigate_read_only`'s prefix walk (`yq_assign_is_total_noop`'s own eager-RHS-discard
pre-check, #1232's `PrefixNavOutcome`) an `Expr::Iterate` arm too — but only for the
narrowest case, `.a` itself being a genuine scalar (`.a[].b = error("boom")` on `a: 5` now
no-ops silently, RHS never evaluated, matching real yq).

**Fixed by [#1432](https://github.com/rust-works/succinctly/issues/1432):** real yq's actual
RHS-discard rule for a mid-chain `Iterate` is broader than #1298's own narrowest case — a real
container whose own elements *all* individually no-op also discards the RHS, and so does an
empty container (vacuously). New `assign_path_all_noop` recurses into
`.iter().all(...)`/`.values().all(...)` at such an `Iterate` instead of unconditionally
deferring, a read-only dry run of the `=` walker's own per-element recursion (then
`set_path_through_iterate`, now `set_path_steps`' `Iterate` arm):

```bash
$ printf 'a: [1, 2]\n' | yq            -o=json '.a[].b = error("boom")'   # {"a":[1,2]}, no-op
$ printf 'a: [1, 2]\n' | succinctly yq -o=json '.a[].b = error("boom")'   # {"a":[1,2]} (fixed)
```

A `null` target is deliberately excluded from this fix — real yq autovivifies `null` to `[]`
as part of the write itself (already correct on both sides before and after this fix), so it
is not a *total* no-op the way an empty/all-scalar container is, and this predicate's
all-or-nothing `Skip`/`Continue` caller has no way to express "skip the RHS, but still
perform the write." That narrower gap was tracked separately as #1857 -- see the entry below.

**Fixed by [#1857](https://github.com/rust-works/succinctly/issues/1857):** a *mid-chain*
`Iterate` autovivifying `null` into `[]` still evaluated the RHS eagerly, where real yq's
equivalent write never needs it (zero elements to write into). New `assign_path_rhs_unused`
answers a broader question than `assign_path_all_noop`'s total-no-op check -- "is the RHS
ever actually read" -- recognizing `null` under a mid-chain `Iterate` as one more case where
it isn't, and `yq_assign_noop_check` performs the (already fully determined) write with a
placeholder value instead of evaluating the RHS at all:

```bash
$ printf 'a: null\n' | yq            -o=json '.a[].b = error("boom")'   # {"a":[]}, RHS never runs
$ printf 'a: null\n' | succinctly yq -o=json '.a[].b = error("boom")'   # {"a":[]} (fixed)
```

Reaches every yq-mode assignment operator routed through this mechanism (`=`, `+=`, `-=`,
`*=`, `/=`, `%=`, `//=`). A **terminal** `Iterate` (`.a[] = v`, the assignment target itself)
is a different, still-open case neither this fix nor #1432's own covers — see
[#1921](https://github.com/rust-works/succinctly/issues/1921). Plain `|=` doesn't share this
mechanism at all and turned out to have a separate, more severe divergence (a hard error
instead of missing autovivification) — see
[#1919](https://github.com/rust-works/succinctly/issues/1919).

### A mid-chain `Field`/`Index` step hitting the wrong container type or an out-of-range array index

Found alongside [#1432](https://github.com/rust-works/succinctly/issues/1432)/tracked as
[#1863](https://github.com/rust-works/succinctly/issues/1863) — three still-open gaps in the
same mid-chain-`Iterate` write path #1298/#1432/#1857 progressively fixed above, all
live-verified against v4.53.3, none a quick fix (each needs its own deeper write-path
change — array bounds-autovivification, index-to-string-key coercion — not just an
RHS-discard predicate tweak like its siblings above):

- **A `Field` step mid-chain hits a real `Array`.** Real yq raises its own structural error
  *before* the RHS ever evaluates; succinctly evaluates the RHS first, surfacing its
  error/side-effect instead of the structural one:
  ```bash
  $ printf 'a:\n  - 1\n  - 2\n' | yq            -o=json '.a.b[].c = error("boom")'
  Error: cannot index array with 'b' (strconv.ParseInt: parsing "b": invalid syntax)
  $ printf 'a:\n  - 1\n  - 2\n' | succinctly yq -o=json '.a.b[].c = error("boom")'
  Error: boom
  ```
- **An `Index` step mid-chain is out of range on a real `Array`.** Real yq autovivifies the
  array out to that length (padding with `null`), then continues the write into the
  newly-created tail (empty, so the fan-out RHS never runs) — no error at all; succinctly has
  no such padding and evaluates the RHS instead:
  ```bash
  $ printf 'a:\n  - 1\n  - 2\n' | yq            -o=json -I=0 '.a[5][].b = error("boom")'
  {"a":[1,2,null,null,null,[]]}
  $ printf 'a:\n  - 1\n  - 2\n' | succinctly yq -o=json -I=0 '.a[5][].b = error("boom")'
  Error: boom
  ```
- **An `Index` step mid-chain hits a real `Object`.** Real yq coerces the numeric index to a
  string key and inserts it; succinctly has no such coercion and evaluates the RHS instead:
  ```bash
  $ printf 'a: {}\n' | yq            -o=json -I=0 '.a[0][].b = error("boom")'
  {"a":{"0":[]}}
  $ printf 'a: {}\n' | succinctly yq -o=json -I=0 '.a[0][].b = error("boom")'
  Error: boom
  ```

Pinned as known-divergent by `test_yq_assign_all_noop_mismatched_element_type_1432`
(`tests/yq_cli_tests.rs`), which also confirms the one sibling shape that *does* already
match yq: an `Index` step hitting a genuine scalar mid-chain permanently no-ops the whole
write (#1232), same as every other position.

### `=`'s multi-output RHS: real yq takes only the last value, no fan-out

[#1430](https://github.com/rust-works/succinctly/issues/1430) started as a narrower report
("a self-referencing multi-path assignment prints one extra document") whose own claimed
expected output turned out not to match live yq at all. Re-verified against v4.53.3: `=`'s
RHS is not special to self-reference — a multi-output RHS of *any* shape collapses to its
**last** output, applied once to every resolved path, producing exactly one document. Real
jq's own `=`, by contrast, genuinely forks — one whole document per RHS output (#392,
unaffected by this fix):

```bash
$ echo '{"a":[1,2]}' | yq            -o=json -I0 '.a[] = .a[] + 1'   # {"a":[3,3]}
$ echo '{"a":[1,2]}' | succinctly yq -o=json -I0 '.a[] = .a[] + 1'   # {"a":[3,3]}  (fixed)

$ echo '{"x":0}'     | yq            -o=json -I0 '.x = (10,20,30)'   # {"x":30}
$ echo '{"x":0}'     | succinctly yq -o=json -I0 '.x = (10,20,30)'   # {"x":30}  (fixed)
```

Fixed in `eval_assign` (`src/jq/eval.rs`) by collapsing `rhs_values` to its last element
before the fan-out loop, gated on `S::TAG == EvalTag::Yq` — the loop itself, and every other
mode, is untouched.

**Deliberately still open:** the collapse only applies when the RHS stream completes
cleanly (`terminal.is_none()`). A RHS that *itself* errors partway through
(`.x = (1, error("boom"), 3)`) still uses jq's pre-existing partial-fan-out behavior in yq
mode too, which has not been verified against real yq (live-checked only that real yq
raises the error with no document printed at all — not the shape needed to characterize
what succinctly should do instead). Not folded into this fix, since redefining an
unverified error-interaction shape risked a second, differently-wrong divergence rather than
fixing one — filed as [#1779](https://github.com/rust-works/succinctly/issues/1779).

### `+=`/`-=`/`*=`/`/=`/`%=`/`//=`'s multi-output RHS: same jq-forks/yq-takes-last split as `=`

Live-probing #1430 also surfaced that real jq's own `+=`/`-=`/`*=`/`/=`/`%=` (unlike `|=`)
genuinely fork over a multi-output RHS the same way `=` does, and real yq's own answer for
these operators isn't "first" either — it takes only the **last** output, exactly like
`=`'s own rule above:

```bash
$ echo '{"x":1}' | jq            -c '.x += (10,20,30)'          # {"x":11} {"x":21} {"x":31}
$ echo '{"x":1}' | succinctly jq -c '.x += (10,20,30)'          # {"x":11} {"x":21} {"x":31}  (fixed)

$ echo '{"x":1}' | yq            -o=json -I0 '.x += (10,20,30)' # {"x":31}
$ echo '{"x":1}' | succinctly yq -o=json -I0 '.x += (10,20,30)' # {"x":31}  (fixed)
```

`succinctly`'s `eval_rhs_once` (shared by all of `eval_compound_assign` and
`eval_alternative_assign`) used to always collapse to the *first* output in both modes — a
pre-existing gap in that function's own doc comment (referencing #392), separate from and
predating #1430's yq-mode scope for `=`. Fixed in
[#1778](https://github.com/rust-works/succinctly/issues/1778) by replacing
`eval_rhs_once` with `collect_rhs_outputs`/`eval_update_multi`, mirroring `eval_assign`'s
own #392/#1430 shape: jq mode forks, yq mode collapses to the last output on a clean
completion, gated on `S::TAG == EvalTag::Yq` — the same "deliberately still open" carve-out
above (an RHS that itself errors partway through) applies here too, for the same reason
(#1779). `/=`/`%=`/`//=` have no real yq syntax at all (confirmed live, `'/'`/`'//'` expect
2 args but there is 1), so their yq-mode "take the last value" answer is judged by internal
consistency with `|=` rather than an external-compat claim.

### `-o=auto` on a genuinely mixed-format multi-source run doesn't match per-element

[#1493](https://github.com/rust-works/succinctly/issues/1493) made `-o=auto` resolve
against the input's own format instead of always rendering JSON — YAML input renders as
YAML, JSON input as JSON, per-source (`--split-exp`, the standard multi-file path,
`--eval-all`, `--inplace`) or per-invocation-uniform-format (`--slurp`/`--eval-all` when
every source agrees). A genuinely *mixed*-format run is the one case left unresolved:
real yq treats the whole output as one YAML stream with each JSON-sourced document
embedded flow-style, `---` between every document regardless of source format —
succinctly instead gives each document its own correct format independently, with no
separator between differently-formatted documents and no flow-style JSON embedding:

```bash
$ printf 'a: 1\n' > a.yaml && printf '{"b":2}' > b.json

$ yq            -o=auto '.' a.yaml b.json   # a: 1
                                             # ---
                                             # {"b": 2}
$ succinctly yq -o=auto '.' a.yaml b.json   # a: 1
                                             # {
                                             #   "b": 2
                                             # }

$ yq            -o=auto --slurp '.' a.yaml b.json   # - a: 1
                                                      # - {"b": 2}
$ succinctly yq -o=auto --slurp '.' a.yaml b.json    # - a: 1
                                                       # - b: 2
```

Live-verified against yq v4.53.3. Pinned as known gaps (current, not the desired end
state) by `test_yq_auto_output_mixed_format_multi_file_known_gap_1493` and
`test_yq_auto_output_slurp_mixed_format_known_gap_1493` in `tests/yq_cli_tests.rs`.

### A key that will not decode is preserved as `""` rather than raising

Real yq rejects a document containing an undecodable scalar outright (`found unknown
escape character`, exit 1), whatever the scalar's position. succinctly matches that for a
**value**, on every route, and deliberately does not for a mapping **key**.

For values this was once a route-dependent split — the *materializing* routes (`--arg`,
`-P`, `to_entries`, `length`) raised via
[#1247](https://github.com/rust-works/succinctly/issues/1247), while the *streaming*
writers silently substituted `null`/`""` at exit 0, because their only error channel was
`core::fmt::Result`, which carries no message. That was the design's deferred "Stage 6"
(`docs/plan/decode-failure-routing.md`) and is **closed** by
[#1615](https://github.com/rust-works/succinctly/issues/1615), which gave those writers a
real error type (`StreamFailure`). Both spellings of one document now give one answer:

```console
$ printf 'a: 1\nb: "bad \q escape"\n' | succinctly yq -o=json '.'
Error: invalid escape sequence                    # exit 1, matching yq's own rejection
$ printf 'double: "quoted \q scalar"\n' | succinctly yq '.'
Error: invalid escape sequence                    # exit 1
```

Whatever prefix had already been written before the failure is left on stdout rather than
buffered and discarded — the same truncate-then-diagnose trade
[#1641](https://github.com/rust-works/succinctly/issues/1641) and
[#1679](https://github.com/rust-works/succinctly/issues/1679) settled for their own
streaming sites. Buffering the whole record instead would reverse P9 (direct YAML-to-JSON
streaming, a 2.3x win) for a malformed-input edge case, and is an explicit non-goal of the
design doc.

Because that prefix can be left unterminated mid-value, a decode failure on a *streamed*
route **stops the run** rather than continuing to the next document: the alternative welds
the following document's `---` onto the truncated line, yielding output that re-reads as
valid YAML with a fabricated value. Real yq rejects the whole file for these inputs too,
so this is also the closer answer. `--inplace` leaves the file byte-identical, matching
both real yq and succinctly's own materializing `-i` path.

This does **not** narrow [#355](https://github.com/rust-works/succinctly/issues/355)'s
divergence — that a malformed value does not cost you the good documents around it. #355
lives on the *materializing* routes, which write nothing partial and still process every
document (`--arg z y '.'` on the file above still prints the later documents after the
diagnostic). An ordinary uncaught *evaluation* error continues on the streamed route too,
for the same reason: it reaches stderr without having written anything to stdout.

A bad *key* is a different story, on both routes, since [#1642](https://github.com/rust-works/succinctly/issues/1642):
`to_entries`/`keys`/`length` all preserve it (as `""`, `YamlValue::key_string`'s existing
convention for a mapping key with no scalar form -- issue #222) rather than raising, on
*every* route, streamed or materialized alike (`"a\qb": 1` → `"": 1`), and jq mode's own
analogous fix for a JSON key (see the "A key
that will not decode is never a duplicate" note under [jq Limitations § Duplicate object
keys collapse, except under `--preserve-input`](../jq/limitations.md#duplicate-object-keys-collapse-except-under---preserve-input)).
`has`/`in` agree too, for the same reason as jq mode: neither has native handling and both
fall back to materializing the whole mapping first, which no longer fails on an unrelated
bad key. This was a live inconsistency prior to #1642 -- `to_entries`/`keys` used to raise
on a bad key while the default streamed identity output already preserved it as `""`, the
same one-document-many-answers problem #1642's JSON-side fix closes, on the mapping-key
axis instead of the object-key one.

**One exception, on the materializing routes only.** Every decode-failure key's display
fallback is the fixed constant `""`, so a mapping with *two* decode-failure keys collides
the instant a materializing route (`--arg`, `-P`, `.,.`) builds an `IndexMap<String, _>`
keyed by that string -- the two are never actually the same key (#1385's "never a
duplicate" rule again), but a plain string-keyed map cannot hold both entries under `""`.
Rather than resurrect the silent-overwrite bug this whole effort exists to close,
`DisplayKeyGuard` ([src/jq/document.rs](../../../src/jq/document.rs)) makes that specific
collision raise instead:

```console
$ printf '"a\qb": 1\n"c\qd": 2\n' | succinctly yq --arg z y '.'
Error: object key "" is ambiguous: an undecodable key's display form collides with
another key of the same name and cannot be represented
```

An *ordinary* repeated key (no decode failure on either side) is unaffected and still
collapses to its last value, matching yq's normal duplicate-key handling.

**`--slurp`/`--eval-all`/`--inplace`'s own DOM fallback catches a second, wider trigger
the paragraph above's routes do not, as of
[#1749](https://github.com/rust-works/succinctly/issues/1749).** Those three flags
materialize through a separate, `YamlCursor`-native conversion
(`yaml_to_owned_value` in
[src/bin/succinctly/yq_runner.rs](../../../src/bin/succinctly/yq_runner.rs)), not the
`DocumentValue`-generic path `--arg`/`-P`/`.,.,` use. It reuses the same
`DisplayKeyGuard`/`colliding_display_key_error` machinery, but drives it with its own
`YamlValue::key_string_kind` classification, which also flags a **complex** key
(mapping, sequence, `null`, or a non-scalar/dangling alias -- not just a decode-failure
string) as a fallback spelling. Real yq keeps both entries in this case too (its
underlying representation isn't a plain map); succinctly's `OwnedValue::Object`
structurally cannot, so this raises rather than silently discarding one:

```console
$ printf '? [1,2]\n: a\n? [3,4]\n: b\n' | succinctly yq --slurp '.[0]'
Error: object key "" is ambiguous: an undecodable key's display form collides with
another key of the same name and cannot be represented
```

**This wider trigger is currently `--slurp`/`--eval-all`/`--inplace`-only.** The
`--arg`/`-P` route's own `key_display_string_kind` (JSON-oriented, shared across every
`DocumentValue` implementor) does not yet recognize a YAML complex key as fallback --
only a decode-failure string key, same as the paragraph above -- so the identical input
through `--arg`/`-P` still silently drops one entry rather than raising. Tracked as
[#1753](https://github.com/rust-works/succinctly/issues/1753), along with a related gap
in the `load()` builtin's own separate YAML-mapping conversion, which has no collision
guard at all.

### `any`/`all`/`flatten`/`group_by`/`unique`/`unique_by`/`from_entries` on a non-array — resolved, real yq has its own wording per builtin, not jq's "Cannot iterate" template

Found during code review of [#1494](https://github.com/rust-works/succinctly/issues/1494)/PR
#1900 (the `cannot_iterate_with` `EvalTag` threading fix): that PR correctly threads the real
evaluation mode through to `cannot_iterate_with`'s value-preview formatting, but real yq
doesn't use this jq-pinned "Cannot iterate over `<type>` (`<value>`)" *template* at all for
these seven builtins — it has its own, unrelated, per-builtin wording with no value preview,
confirmed live against pinned Homebrew yq v4.53.3:

```console
$ printf 'null\n' | yq '5 | any'                          # any only supports arrays, was !!int
$ printf 'null\n' | yq '5 | all'                          # all only supports arrays, was !!int
$ printf 'null\n' | yq '5 | flatten'                      # only arrays are supported for flatten
$ printf 'null\n' | yq '5 | group_by(.)'                  # only arrays are supported for group by
$ printf 'null\n' | yq '5 | unique'                       # only arrays are supported for unique
$ printf 'null\n' | yq '5 | unique_by(.)'                 # only arrays are supported for unique (not "unique by")
$ printf 'null\n' | yq '5 | from_entries'                 # from entries only runs against arrays
```

[#1901](https://github.com/rust-works/succinctly/issues/1901) also found real yq rejects an
**object** input the same as a scalar for all seven — a second axis jq disagrees with it on:
jq's own semantics let `any`/`all`/`flatten` succeed on an object (iterating its
values/entries) and give `group_by`/`unique`/`unique_by` a different, jq-specific "object and
array cannot be sorted" pairing error (`{"a":3,"b":1} | group_by(.)` — a quirk of jq's own
`map([f])`-based definition) — none of that carries over to yq mode, where every one of these
seven raises the identical wording for an object as for a scalar. Fixed via
`EvalError::yq_only_supports_arrays`/`yq_only_arrays_supported_for`/
`yq_from_entries_requires_array` (`src/jq/error.rs`), gated by `S::TAG == EvalTag::Yq` at each
of the seven builtins' object and scalar arms in `src/jq/eval.rs` — jq mode's own
`cannot_iterate_with`/`object_pair_type_error` paths are untouched.

**Two related items found but not fixed here, needing materially different implementation
shapes:**

- **`sort`/`sort_by` on a non-array/map**: real yq's wording is `"node at path <path> is not
  an array or map (it's a <tag>)"`, where `<path>` genuinely varies with navigation depth
  (confirmed live: `[]` at the top level, `[a.b]` after `.a.b`, `[[2]]` after `.[2]`) — unlike
  the seven builtins above, whose wording never names a path. `builtin_sort`/`builtin_sort_by`
  receive only a bare value with no path context today, so this needs real path-tracking
  plumbing threaded into these builtins, not just a wording swap.
- **`.[]`'s own read-side behavior on a scalar**: real yq treats this as a **silent no-op**
  (exit 0, no output) rather than an error at all (`5 | .[]` prints nothing on real yq).
  succinctly raises `cannot_iterate_with` here in yq mode too, matching jq's own `.[]`
  semantics (correct for jq mode, not yq's). Whether this shares one root cause with the seven
  builtins above or needs its own fix is unresolved — bare `Expr::Iterate` has 20+ dispatch
  arms across `eval.rs`'s value/path/assignment-mode evaluators, so scoping this properly
  needs its own investigation.

Both filed together as [#1998](https://github.com/rust-works/succinctly/issues/1998).

**Two more items found reviewing the fix itself, also not fixed here:**

- **`any(cond)`/`all(cond)`/`any(gen; cond)`/`all(gen; cond)`** (the predicate-argument forms,
  dispatched from `Builtin::AnyF`/`AllF`/`AnyCond`/`AllCond`) still leak jq's own wording and
  still allow full object iteration, unlike the now-fixed bare forms — and real yq's own lexer
  rejects this syntax entirely regardless of arity (`bad expression, please check expression
  syntax`), which `succinctly yq`'s parser doesn't gate behind `--jq-extensions` the way
  neighboring jq-only builtins like `min_by` do. Filed as
  [#2005](https://github.com/rust-works/succinctly/issues/2005).
- **`with_entries(f)` rejects numeric keys** that real yq coerces to strings on reassembly
  (`[1,2] | with_entries(.)` succeeds on real yq, errors `Cannot use number (0) as object key`
  on succinctly) — a functional bug on well-formed input, unrelated to this section's
  malformed-input wording fix. Filed as
  [#2006](https://github.com/rust-works/succinctly/issues/2006).

### `fromjson`/`tonumber`'s shared decoder is jq-modeled, only narrowly gated for yq mode

`Builtin::FromJson`/`Builtin::ToNumber` dispatch to `builtin_fromjson`/`tonumber_from_str`
in `src/jq/eval.rs` with no mode split at all -- the same hand-rolled JSON-string decoder
(`parse_json_string_value`) backs `fromjson`/`tonumber` in both `succinctly jq` and
`succinctly yq`. [#2008](https://github.com/rust-works/succinctly/issues/2008) (a lone
*low* surrogate escape, `\uDC00`-`\uDFFF`, should substitute U+FFFD rather than error,
matching real jq) initially applied that fix unconditionally, which broke yq-mode fidelity
here: real yq's `fromjson` doesn't use jq's JSON string grammar at all -- it decodes
through go-yaml's own quoted-scalar scanner, which rejects *any* `\u` escape encoding a
surrogate codepoint outright (confirmed live against yq v4.53.3, including a **valid**,
correctly-paired surrogate escape, not just a lone one). Fixed during that PR's own review
by gating the new low-surrogate arm behind `S::TAG == EvalTag::Yq` (threaded as a plain
`yq_mode: bool` through `parse_complete_json`/`parse_json_value`/`parse_json_array`/
`parse_json_object`/`parse_json_string_value`, since none of those carried an
`EvalSemantics` type parameter before): yq mode keeps erroring on a lone low surrogate,
unchanged from before #2008; only jq mode gained the new leniency.

That gate is deliberately narrow -- it does not make yq-mode `fromjson` match real yq's
actual model, which rejects the entire surrogate-pairing mechanism, not just the lone-low
case. A **valid** surrogate pair still silently decodes successfully in yq mode today
(`succinctly yq -n '"\"\\ud83d\\ude00\"" | fromjson'` → the emoji, real yq → a parse
error). A lone **high** surrogate was also wrongly *accepted* (substituted U+FFFD) in both
jq and yq mode alike -- a separate, pre-existing bug unrelated to #2008's own scope
(also caused real data loss: two `fromjson`-parsed object keys differing only by an
unpaired high surrogate silently collapsed, last-value-wins) -- fixed in both modes by
[#2013](https://github.com/rust-works/succinctly/issues/2013): `parse_json_string_value`'s
high-surrogate arm now raises unconditionally (no mode gate needed there, since both real
jq and real yq reject it -- unlike the lone-low case above, where the two oracles
disagree). Fully aligning yq-mode `fromjson` with real yq's stricter, non-pairing model
(the still-open valid-pair gap) is filed separately as
[#2018](https://github.com/rust-works/succinctly/issues/2018), since it needs its own
yq-mode branch checked ahead of all of jq's pairing logic, not an arm-by-arm patch.

### `parent` doesn't auto-vivify the missing node it navigated through

[#2146](https://github.com/rust-works/succinctly/issues/2146) (defect 3, still open --
defects 1 and 2 there were fixed by
[#2213](https://github.com/rust-works/succinctly/issues/2213)). `key`/`parent`/`path`/
`file_index` are real yq builtins with no jq counterpart, so real yq v4.53.3 is the only
oracle for them. Navigating through a missing object key or an out-of-bounds array index is
"not an error, propagate `null`" in both tools -- but real yq's `parent` goes one step
further and auto-vivifies the missing node *into the object it returns*, as if the write had
already happened:

```bash
$ echo '{}' | yq            -o=json -I=0 '.a | parent'
{"a":null}
$ echo '{}' | succinctly yq -o=json -I=0 '.a | parent'
{}
```

succinctly has no vivification machinery anywhere in the codebase -- `parent`'s return value
is always a real node already present in the document, walked to via the accumulated path.
Post-#2213, that's the *un-vivified* ancestor (`{}`, the actual root) rather than the
pre-#2213 stub (`null`, `key`/`path`/`parent`'s shared fallback for any path-tracking
failure) -- a real improvement, since `{}` is at least a genuine document node consistent
with what `path()`/`key` now report for the same navigation, but still short of yq's
synthetic `{"a":null}`. Implementing this needs `parent`/`parent(n)` to be able to construct
a value that isn't a snapshot of anything in the document -- a different kind of change than
#2213's path-threading fix, which only ever continues with a value already known to be
correct (`Null`, jq's own answer for what the missing/OOB read itself evaluates to).

### A negative out-of-range array index (`.a[-N]`) raises -- resolved for ordinary reads, a few call sites remain

[#2254](https://github.com/rust-works/succinctly/issues/2254). Real yq disagrees with real
jq on a negative index whose magnitude exceeds the array length: jq treats it the same as a
positive out-of-range index (`null`, not an error); yq raises. A positive out-of-range index
is `null` in both tools -- the asymmetry is specific to the negative-magnitude case:

```bash
$ echo '{"a":[1,2]}' | yq -o=json '.a[-1]'   # 2   (in-bounds negative wraparound: fine)
$ echo '{"a":[1,2]}' | yq -o=json '.a[-2]'   # 1   (in-bounds: fine)
$ echo '{"a":[1,2]}' | yq -o=json '.a[-3]'   # Error: index [-3] out of range, array size is 2
```

Eight independent implementations of the same resolve-index arithmetic across
`src/jq/eval.rs` and `src/jq/eval_generic.rs` treated "index doesn't resolve" as one outcome
with no `EvalSemantics`-based dispatch on sign -- fixed for every ordinary `.a[-N]`/`.a[$n]`
read, both the literal and computed-index forms, through both jq's primary CLI dispatch
(`eval_generic.rs`) and the library-route/path-context evaluator (`eval.rs`). Unlike every
other read-time indexing error here, `optional` does *not* suppress it: confirmed live that
real yq's own lexer accepts `?` after a bare bracket index (`.a[-5]?` parses fine; only the
*parenthesized* form, `(.a[-5])?`, is lexer-rejected, an unrelated construct) and the error
still raises through it (`.a[-5]?` on `[1,2]` still exits 1 with the same message in real
yq). `EvalError::is_yq_negative_index_error` is consulted by every `?`/`try` dispatch point
on the read side -- `eval_try`/`each_try` in `eval.rs`, `try_single_generic`/
`each_try_generic` in `eval_generic.rs` (the generator-argument pair matters separately from
the single-value pair: `any(.a[-5]?; .)` runs its generator argument through `each_try`, not
`eval_try`, and was found still swallowing the error until this same sweep covered it too) --
and via `EvalError::is_uncatchable`, extended to include it, at `resolve_node`'s own
`Expr::Try`/`Expr::Optional` pair (`eval.rs`) for `path()`/`getpath`'s path-tracking context,
and at `eval_stage_with_path_context`'s own general `Expr::Optional` arm (`eval.rs`), which
governs a bare `?` combined with a path-context-triggering builtin elsewhere in the same
pipe (`key`/`parent`/`file_index`) -- confirmed live that `.a[-5]? | key` wrongly exited 0
with no output before this arm was covered too. Two further computed-index call sites
(`eval_index_expr`'s and `eval_index_expr_with_path_context`'s own `Owned`-target loops --
reached via `--slurp` against a constructed rather than cursor-backed array, and via a
computed bracket key piped into a path-context builtin, respectively) had the identical
gate-on-`optional` bug and are fixed the same way. This keeps the error unsuppressible the
same way `is_decode_failure` already does for a decode failure.

A few more independent copies of the same arithmetic were found, in lower-traffic call
sites, and tracked as [#2264](https://github.com/rust-works/succinctly/issues/2264).
Reverification there found `pick(paths)` already matches real yq exactly (both the
expression form and the array-literal-keys form), not a gap after all. `keys`/
`keys_unsorted`'s own lazy `.[n]` fast path (`fold_lazy_keys_stage` for object keys,
`fold_lazy_index_range_stage` for array keys, `src/jq/eval_generic.rs`) *was* a real,
live-confirmed gap -- `keys[-5]`/`keys_unsorted[-5]` silently answered `null` instead of
raising, since each already walks (or already knows) the whole container's length to
resolve a negative index at all, so the check now rides along for free, the same shape
as every other fix in this section. Three more copies remain genuinely unverified --
`path()`'s own walker (`navigate_static_component`, `src/jq/eval.rs`; real yq's `path()`
turns out to reject essentially any bracket-index argument outright, regardless of sign,
so what "fixed" even means here needs its own grammar investigation first),
`get_value_at_path` (`src/jq/eval.rs`), and `getpath`'s own path-array walk
(`resolve_read_index`/`getpath_walk_owned`, `src/jq/eval.rs`; jq-only syntax, so this
only matters under `--jq-extensions`) -- filed as
[#2335](https://github.com/rust-works/succinctly/issues/2335).
`del()`/`delpaths()` silently no-op on this shape instead of raising at all (more severe than
a suppression gap -- a missing error, not a differently-worded one) -- filed separately as
[#2268](https://github.com/rust-works/succinctly/issues/2268). `eval_stage_with_path_context`'s
own `Expr::Optional`/`Expr::Try` arms (`--eval-all` combined with a path-context-triggering
builtin like `file_index`) had no uncatchable-error guard at all -- filed and fixed as
[#2270](https://github.com/rust-works/succinctly/issues/2270), whose own review found the
first attempt (`EvalError::is_uncatchable()`, matching `resolve_node`'s own path-tracking
resolver) was itself too broad for this specific function: `is_uncatchable()` also covers
`is_invalid_path_expression()`, correct for `resolve_node` but not for
`eval_stage_with_path_context`, which runs whenever *any* sibling in the same pipe needs path
context, not only when the branch actually being evaluated is itself a path expression --
confirmed that `.a | (try path(1) catch "x"), key` wrongly let the error escape uncaught
purely because of the unrelated `key` sibling. `key` is a succinctly-only path-context
extension real jq's own lexer rejects outright, so only the isolated `try path(1) catch "x"`
half of that claim has a real-jq oracle to check against -- confirmed live it always catches
there; the combined query is an internal-consistency claim against succinctly's own build
instead (the unrelated `key` sibling must not change what `try path(1) catch "x"` alone
already does). Fixed with `EvalError::is_uncatchable_at_value_position` (`src/jq/error.rs`),
a narrower sibling of `is_uncatchable()` that excludes `is_invalid_path_expression()` --
consolidating what `eval_try`/`each_try`/`try_single_generic`/`each_try_generic` (`eval.rs`/
`eval_generic.rs`) already hand-copied inline at six sites onto one shared definition,
matching this function's own two new call sites too.

### `del()`/`delpaths()` on a negative out-of-range index now raise too (#2268)

[#2268](https://github.com/rust-works/succinctly/issues/2268). #2254 above fixed every
*ordinary read* (`.a[-N]`); `del()`/`delpaths()` had the identical gap but a more severe
symptom -- not a differently-worded error, but silently no-op (exit 0, document unchanged)
where real yq aborts the whole operation:

```bash
$ echo '{"a":[1,2]}' | yq -o=json 'del(.a[-5])'          # Error: index [-5] out of range, array size is 2
$ echo '{"a":[1,2]}' | succinctly yq 'del(.a[-5])' -o json  # {"a":[1,2]} -- WRONG, before this fix
```

Five independent array arms shared the gap, none `EvalSemantics`-aware (all five take a
plain `yq_mode: bool`, not generic `S`, so the fix reuses a new `bool`-based sibling of
`yq_negative_index_check`/`yq_negative_index_error` rather than threading `S` through the
whole delete-path recursion): `delete_at_path`'s own `Expr::Index` array arm (`del()`'s
single-step literal-index dispatch), `delete_path_steps`'s own mid-chain `Expr::Index` arm
(a pipe-chained `del()` path, e.g. `del(.a[-5].x)` -- missed in the first pass, found by code
review), `delete_trie_array`'s own `ArrayStep::Index` arm (a comma-grouped `del()` path, e.g.
`del(.a[-5].x, .c)`, which real yq aborts *entirely* on -- `.c` is not deleted either -- also
missed in the first pass), `delete_paths_under`'s array arm (`delpaths()`'s mid-path
navigation, e.g. `delpaths([["a",-5]])`), and `delete_keys`'s array arm (`delpaths()`'s
terminal per-container batch, e.g. `delpaths([[-5]])`). Unlike the read-side fix, this one is
**not** suppressible by an earlier `?` in the path chain -- confirmed live, `del(.a?[-5])`
still raises the identical error in real yq, matching `del()`'s own `optional` parameter's
established scope (a `?`/type-mismatch suppression, never extended to this new check).

**Was a residual divergence, closed by [#2306](https://github.com/rust-works/succinctly/issues/2306)**:
when a negative-out-of-range index is grouped in the *same* `delpaths()` call as an earlier
deletion on the same array, real yq's own error reports the array's size *after* that earlier
deletion already happened -- confirmed live, `delpaths([[0],[-5]])` on a 2-element array
reports "array size is 1", not 2, meaning real yq's own `delpaths` resolves indices
*sequentially*, not against the array's length on entry. `delpaths_one` (`src/jq/eval.rs`) now
applies a yq-mode multi-path batch one path at a time instead of handing the whole (sorted)
list to `delete_keys` in one call -- see the "batch of two or more keys" entry below for the
fuller writeup -- so this now reports "array size is 1" too, matching real yq exactly.

**A separate, unrelated divergence found investigating this one**: real yq's `del()` on a
*positive* out-of-range index doesn't no-op the way jq does -- it extends the array with
`null`s up to (not including) that position (`del(.[5])` on `[1,2]` yields a 5-element array,
`[1,2,null,null,null]`, in real yq, confirmed live), where succinctly (matching jq) used to
leave the array unchanged. Was its own issue, [#2305](https://github.com/rust-works/succinctly/issues/2305),
since it's about a positive index and has nothing to do with #2268's own negative-index
scope -- closed by the next section below.

### `del()`/`delpaths()` on a positive out-of-range array index extends with `null` instead of no-opping

[#2305](https://github.com/rust-works/succinctly/issues/2305). Real yq's own asymmetry between
positive and negative out-of-range indices (the section above covers the negative half) has a
second half specific to `del()`/`delpaths()`: a **positive** out-of-range index -- where a plain
*read* answers `null` in both jq and yq -- makes yq's own `del()` *grow* the array with `null`
up to the requested length, where jq (and, before this fix, succinctly) leaves it unchanged:

```bash
$ echo '[1,2]' | yq -o=json 'del(.[5])'      # [1, 2, null, null, null] -- length 5, not a no-op
$ echo '[1,2]' | jq -c 'del(.[5])'           # [1,2]                    -- jq: genuine no-op
```

The extension target is the *requested* index itself, not `index + 1` -- there is nothing to
place *at* the deleted position, only a gap to fill *before* it, so `del(.[2])` on a
length-2 array (index == current length exactly, no gap) is a true no-op in both tools, and
only `index > length` triggers the growth. Closed by `extend_array_with_nulls_for_delete`
(`src/jq/eval.rs`), a fallible-reserve-then-resize sibling of `pad_with_nulls` (`setpath`'s own
array-growth helper, #1670) shared by `delete_at_path`'s `Expr::Index` arm (`del(.[N])`) and
`delete_keys`'s own single-key case (`delpaths([[N]])`, confirmed live to share the identical
extension behavior) -- gated on `yq_mode`, since jq has no such rule at all.

### `delpaths()`'s multi-key batch applies sequentially, not as a union

[#2306](https://github.com/rust-works/succinctly/issues/2306). Real yq's own `delpaths()`
applies a 2+-path batch **sequentially** -- one path at a time, each seeing whatever state
every earlier path already left behind -- not jq's own simultaneous/union model, which
resolves every path against the array's *starting* length. This isn't only about
out-of-range indices: even an all-in-range batch is order-dependent, confirmed live:

```bash
$ echo '[1,2,3,4]' | yq -o=json 'delpaths([[0],[1]])'   # [2,4] -- delete index 0, then index 1 of what's left
$ echo '[1,2,3,4]' | yq -o=json 'delpaths([[1],[0]])'   # [3,4] -- same keys, reversed order, different result
```

(Real jq's own `delpaths` gives the order-independent `[3,4]` for *both* orderings.) Mixing an
in-range delete with an out-of-range one shows the same order-dependence -- the same *set* of
keys, given in a different literal order, produces a *different*-length result:

```bash
$ echo '[1,2,3]' | yq -o=json 'delpaths([[0],[5]])'   # [2,3,null,null,null] -- length 5
$ echo '[1,2,3]' | yq -o=json 'delpaths([[5],[0]])'   # [2,3,null,null]      -- length 4
```

`delpaths_one` (`src/jq/eval.rs`) used to sort the whole path list before deleting anything --
a step that exists to support jq's own simultaneous/union model, applied unconditionally
regardless of mode. It now skips that sort in yq mode and instead applies each path
individually, via a loop calling the existing `delete_paths_sorted` once per path (in the
caller's own given order) rather than handing the whole (sorted) list to it in one call. jq
mode's own code path -- sort, then one batched call -- is unchanged.

A negative out-of-range index is otherwise unaffected by this fix -- it still raises (#2268,
the section above), just with an array-size figure that's now correct for a grouped batch too,
per that section's own updated note.

**Still open: `del()`'s own comma-separated multi-target form** (`del(.[0], .[5])`, a
different call path than `delpaths()`'s path-array form -- each target reaches `delete_keys`
through its own separate call, not a shared batch, so #2306's fix above doesn't reach it).
Confirmed live that real yq's `del()` comma form is *also* order-dependent the identical way,
which succinctly does not yet reproduce -- not yet filed as its own issue.

**Also extended to a mid-chain index, not just a terminal one**
([#2314](https://github.com/rust-works/succinctly/issues/2314)). A positive out-of-range
index with more path after it (`del(.[5].a)`, `delpaths([[5,"a"]])`) extends too in real
yq, but to `index + 1` rather than `index` -- there *is* something at the out-of-range
position once the rest of the path needs to navigate into it, so this reuses
`pad_with_nulls` directly (the same `setpath`-side helper #2305's own
`extend_array_with_nulls_for_delete` wraps) rather than the terminal case's `target_len`
rule:

```bash
$ echo '[1,2]' | yq -o=json 'del(.[5].a)'   # [1,2,null,null,null,null] -- index 5 itself now exists
```

`del(.[5][])` is a further, third shape: the freshly-created slot can't stay `null`, since
`.[]` never tolerates one (#527) -- real yq auto-vivifies it straight to `[]` instead, the
same way `setpath` auto-vivifies a `null` into whatever container its own next step needs:

```bash
$ echo '[1,2]' | yq -o=json 'del(.[5][])'   # [1,2,null,null,null,[]]
```

Closed across the three sites that resolve a mid-chain array index for a delete --
`delete_path_steps`'s `Expr::Index` arm (the plain AST walk), `delete_trie_array`'s
`ArrayStep::Index` arm (the comma-grouped multi-target walk, once that walk's own
read-based fan-out has already resolved a concrete branch reaching it -- see the residual
gap below for when it hasn't), and `delete_paths_under`'s array-key arm (`delpaths`' own
recursion) -- all in `src/jq/eval.rs`. `delpaths`' own path components are always concrete
keys, never a `.[]` wildcard, so there is no `[]`-vivify case to mirror at that site.

**A residual gap found during this fix's own review is now closed too**
([#2324](https://github.com/rust-works/succinctly/issues/2324)). A **comma-grouped**
target whose own `.[]` fan-out crosses the out-of-range index (`del(.[5][], .[0])`) never
reached `delete_trie_array` at all -- the read-based validation that enumerates concrete
branches for the trie read `.[5]` as a plain `null` (correct for an ordinary read) and
raised iterating `.[]` over it, rather than extending+vivifying the way the direct,
non-comma-grouped form already did:

```bash
$ echo '[1,2]' | yq -o=json 'del(.[5][], .[0])'   # [2, null, null, null, []]
```

Fixed not by teaching `resolve_node`'s shared, read-only `Expr::Iterate` arm (or the
delete trie) a del()-specific exception, but by a narrow pre-pass
(`vivify_del_comma_iterate_targets`, called from `builtin_del`) that peels every
top-level comma branch shaped as a static `Field`/`Index` prefix plus a bare trailing
`.[]` whose prefix currently reads `null` off the comma group, and runs it through the
very same `delete_at_path` a single-target `del()` call already uses -- before
`resolve_del_path_branches` ever sees the rest. That ordering matches real yq's own
observed behaviour **when no sibling's own resolution can depend on another sibling's
array-extending side effect**: every such branch resolves against the pristine input
ahead of any sibling's own structural (index-shifting) deletion, and, among themselves,
in any relative order -- confirmed live, `del(.[5][], .[0])` and `del(.[0], .[5][])` give
the identical answer above regardless of order. The same root cause also covered a
genuinely-existing `null` (not just an out-of-range index) reached the identical way:

```bash
$ echo '{"a":null,"c":9}' | yq -o=json 'del(.a[], .c)'   # {"a": []}
```

`delpaths()`'s own multi-key batch form was checked and confirmed unaffected --
its path components are always concrete keys, never a `.[]` wildcard (as already noted
above), so it never reaches `resolve_node`'s `Expr::Iterate` arm at all; the analogous
mid-chain-index batch shape (`delpaths([[5,0],[0]])`) already matched real yq before this
fix, via the same `delete_paths_under` machinery #2314 covers.

(A second gap found during the same review -- `null` never vivifying into `[]` ahead of
*any* `Index`/`Iterate` del() step, not just a slot #2314 itself just padded -- is now
closed; see the next section.)

**Two further regressions this pre-pass itself introduced were found and fixed during its
own mandatory review**, both confirmed live against yq v4.53.3:

- **Order-dependence when a negative index is mixed in.** A comma sibling containing a
  negative array index (anywhere in its path, matched-shaped or not) can resolve to a
  different position depending on whether it is evaluated before or after a sibling
  elsewhere in the group extends the same array -- real yq itself is order-dependent here,
  and an earlier version of this pre-pass always applied its own vivify branches first
  regardless of textual order, silently giving one order's answer for both:
  `del(.[-1], .[4][])` is `[1, null, null, []]` in real yq but `del(.[4][], .[-1])` is
  `[1, 2, null, null]` -- genuinely different, not just reordered. Fixed by declining the
  *whole* pre-pass (`contains_length_dependent_index`, checked over every sibling before
  any matching or mutation) whenever any sibling anywhere contains a negative index, a
  negative slice bound, or a computed key/slice that can't be proven non-negative --
  falling through entirely to the unmodified `resolve_del_path_branches` path, the same
  fallback every other unrecognized shape here already gets. That path raises the same
  "Cannot iterate over null" (or a genuine "index out of range") this whole pre-pass exists
  to avoid for the shapes it *does* handle -- a coverage gap for this specific corner case
  (never in #2324's own scope to begin with), not a wrong answer.
- **A dropped `?`.** The pre-pass detected a trailing `.[]` shaped as either bare or
  `?`-suppressed for matching purposes, but always reconstructed a bare `Expr::Iterate` and
  hardcoded `delete_at_path`'s own `optional` parameter to `false`, discarding which one it
  actually was. At the time, a *genuinely missing* key (as opposed to a real key holding
  `null`) routed the delete walk through `delete_at_path_through_absent`, which then raised
  on a trailing `.[]` unless that `.[]`'s own `?` suppressed it -- so a real yq no-op turned
  into a wrongly raised error: `{"x":1} | del(.missing[]?, .x)` is `{}` in real yq. Fixed by
  threading the trailing `.[]`'s own optionality through (`trailing_bare_iterate_prefix` now
  returns it alongside the prefix) instead of hardcoding it away. (#2347, below, later made
  the `?` on a missing-key's trailing `.[]` unnecessary in the first place -- a bare
  `del(.missing[], .x)` no-ops too now -- but threading it through here remained correct and
  is unchanged.)

### `del()` auto-vivifies a `null` into `[]` ahead of an `Index`/`Iterate` step, matching `setpath`

[#2323](https://github.com/rust-works/succinctly/issues/2323), found during #2314's own
review. Real yq's actual null-vivify rule is broader than -- and predates -- #2314's own
"a slot this fix just padded" case: **any** `null` a `del()` walk reaches, freshly padded
or not, vivifies into `[]` the moment the next step is `Index` or `Iterate`, the same
auto-vivify philosophy `=`/`|=`'s own write path already applies (`.a[].b = v` on
`{"a":null}` is `{"a":[]}`, confirmed pre-existing precedent -- see
`test_yq_iterate_pipe_chain_null_autovivifies_under_update_assign_1919`):

```bash
$ echo 'null' | yq -o=json 'del(.[2])'          # [null, null] -- no out-of-range extension involved at all
$ echo 'null' | yq -o=json 'del(.[])'           # []
$ echo '{"x":null}' | yq -o=json 'del(.x[2])'   # {"x": [null, null]}
$ echo '[1,2]' | yq -o=json 'del(.[3][2].a)'    # [1, 2, null, [null, null, null]] -- recurses at each level
```

Applies to `del()`'s terminal form (`delete_at_path`'s own `Index`/`Iterate` arms) and its
mid-chain form (`delete_path_steps`'s own `Index`/`Iterate` arms) alike. Both are
implemented the same way: mutate the `null` root into `Array(Vec::new())` immediately
before the existing container-handling match, rather than duplicating that match's own
logic -- in `delete_path_steps`'s loop this means `continue`ing without advancing `steps`,
so the very same path component re-matches against the freshly vivified array on the next
pass, which is what makes the recursive `del(.[3][2].a)` case above fall out for free
(each level's own out-of-range index re-triggers #2314's existing extension logic against
an array that used to be `null`) without any explicit lookahead or recursion of its own.
`?` does not suppress the vivify (`null | del(.[2]?)` still vivifies, confirmed live), and
neither does a `Pipe`-wrapped continuation (`del(.[5] | (.[].a))` reduces to a literal
`Expr::Iterate` against the padded `null` once the existing `Expr::Pipe` splice logic
runs, so it is covered by the same general arm without its own check).

**Does not apply to:** `Field` steps (`{"x":null} | del(.x.a)` stays `{"x":null}`,
confirmed live -- matches `=`/`|=`'s own precedent, which likewise never vivifies for
`Field`), `Slice` steps (`null | del(.[0:2])` stays `null`, matching jq), `delpaths()` at
all (`{"x":null} | delpaths([["x",2]])` stays `{"x":null}` -- this rule is specific to
`del()`'s expression-based target, not `delpaths()`'s path-array form), or jq mode (no
such rule exists there at all: `null | del(.[2])` stays `null`, `null | del(.[])` still
raises "Cannot iterate over null").

**Vivify eligibility tracks provenance, not just "is `root` currently `null`."** A `null`
reached by *tolerating* a `Field`/`Slice` step against it (#476/#527's own established
exemption) is not vivify-eligible for whatever comes after, even when it sits in an
otherwise-real slot -- confirmed live, `{"x":null} | del(.x[2])` (Field *found* the real
key `"x"`) vivifies, but `[1,2] | del(.[5].missing[2])` (`.[5]` itself is a real
#2314-padded slot, but `.missing` only *tolerates* that slot's `null` rather than finding
a real key on it) does not: `[1,2,null,null,null,null]`, not
`[...,[null,null]]`. Both `delete_at_path` and `delete_path_steps` thread an extra
`real_slot: bool` parameter through every recursive call to track this -- `true` from the
document root or any real navigation (a found object key, an in-range or #2314-padded
array element), flipped to `false` by a `Field`/`Slice` step's own null-tolerant arm, and
never flipped back except by this fix's own vivify. `delete_at_path_through_absent`'s own
synthetic scratch value is a related but separate case: its whole contract is "the
container is untouched" (a missing field never materializes), so `real_slot` is hardcoded
`false` for its own recursive walk -- this alone blocks every vivify site regardless of
`yq_mode` (each is gated `yq_mode && real_slot && ...`), confirmed live,
`{} | del(.missing[-1])` stays `{}`, not the out-of-range error a wrongly-vivified scratch
would raise. (`yq_mode` itself is *not* hardcoded alongside it -- see #2347, below, which
found and fixed a real bug in an earlier version of this function that did hardcode both.)

**Also extended to `del()`'s comma-grouped form**, with one additional gate.
`delete_trie_array`'s own null-skip branch shares the same vivify rule
(`delete_trie_object`'s sibling branch does not — `Field` steps never vivify, per the
scope above): confirmed live, `{"x":null,"y":1} | del(.x[2], .y)` is
`{"x":[null,null]}`, with `.y` still deleting independently either way. Vivifying is safe
even when the only array-level step is a `Slice` (no `Index` at all) — a slice range
against a freshly empty array is itself a harmless no-op — or when one terminal index
shares a node with one *continuation* (`null | del(.[2], .[3].a)` is
`[null,null,null]`, order-independent, confirmed live both ways). One surprising,
*already-correct, pre-existing and unrelated* behavior worth flagging so a future reader
doesn't mistake it for this fix's own doing: `{"x":null,"y":1} | del(.x[0:2], .y)` is
`{}` in real yq — not just `x` cleared, `y` gone too — reproduced identically by
`succinctly yq` both before and after this fix, via the pre-existing #1116/#1219
chained-slice-parent-drop rule elsewhere in this same trie walk.

**Gated off when 2+ *terminal* indices share one node.** `delete_keys`'s own #2305/#2306
gate only extends a single-key batch — real yq's own multi-key extension is
order-*dependent* (confirmed live: `null | del(.[2],.[3])` and `null | del(.[3],.[2])`
both give `[null,null]` here, but #2306's own filed example, a mixed in-range/out-of-range
batch, does differ by order — a materially larger, already-deferred piece of work).
Vivifying `delete_trie_array`'s source unconditionally regardless of this gate would trade
one wrong answer (staying `null`, the pre-existing gap) for a *different* wrong one (`[]`,
since `delete_keys` still refuses the 2+-key batch, just now against an already-empty
array) rather than reproducing real yq's actual multi-key answer — a new, previously
uncharacterized divergence shape this fix should not introduce. `terminal_index_count =
node.indices.len() - node.index_groups.len()` (total array-level steps at this node minus
the ones that have their own children, i.e. continuations) distinguishes this from the
mixed terminal+continuation case above, which is a different shape and already correct:
vivify only fires when `terminal_index_count <= 1`. Confirmed live both orderings give the
same (still-`null`, not-yet-fixed) answer, so this gate doesn't itself introduce any new
order-dependence of its own.

This fix does not reach the comma-grouped **`.[]` fan-out** gap #2324 tracks separately —
that one fails earlier, in the read-based validation that builds the trie in the first
place, before `delete_trie_array` is ever called.

### `del()`'s trailing `.[]` no-ops through a *tolerated* (not just a genuinely found) `null`

[#2347](https://github.com/rust-works/succinctly/issues/2347), found during #2324's own
implementation. The vivify rule two sections above only covers a `null` that is itself the
navigated value (found, or #2314-padded) -- a `null` reached instead by *tolerating* a step
against it (a missing object field, or an already-`null` slot navigated further, #476/#527)
is a different provenance (`real_slot: false`) that real yq treats differently again: not
vivify, but a silent no-op for the whole rest of the chain, `.[]` included:

```bash
$ echo '{"y":1}' | yq -o=json 'del(.x.a[])'      # {"y": 1} -- tolerated null, .[] terminal
$ echo '{"y":1}' | yq -o=json 'del(.x.a.b[])'    # {"y": 1} -- three levels of tolerance
$ echo '{"y":1}' | yq -o=json 'del(.x.a[].b)'    # {"y": 1} -- .[] mid-chain, not terminal
```

Before this fix, `delete_at_path`'s `Expr::Iterate` arm (terminal `.[]`) and
`delete_path_steps`'s own mid-chain `Expr::Iterate` arm (`.[]` followed by more path) both
had no fallback for this case, falling through to the same generic "cannot iterate over
null" error jq mode correctly raises there (jq has no such exemption at all, `#527`
remains a real, unaffected jq/yq divergence). Fixed by adding a `yq_mode`-gated
`OwnedValue::Null => Ok(())` arm to each, deliberately *not* unconditional the way the
sibling `Index` arm's own `Null` fallback already is.

A second, deeper bug surfaced during this fix's own verification:
`delete_at_path_through_absent` (walked once a `Field` step has tolerated a *genuinely
missing* object key, as opposed to a found key holding `null`) had hardcoded `yq_mode =
false` unconditionally, in addition to the still-correctly-hardcoded `real_slot = false`
covered above -- silently downgrading every yq-mode call through it to jq's stricter rule
the instant a tolerance chain started from a missing key rather than a found-but-`null`
value. Fixed by threading the caller's own `yq_mode` through instead; safe because
`real_slot` alone (unchanged) already blocks every vivify site regardless of `yq_mode`.
This also retroactively closed a gap #2324's own review had explicitly flagged as
known-but-out-of-scope at the time (`del(.missing[], .x)`, no `?` needed, now correctly
gives `{}` rather than raising) -- see the "dropped `?`" bullet above, which predates this
fix.

**Residual gap, not yet fixed:** a comma-grouped sibling whose own trailing shape is `.[]`
*followed by more path* (not a bare trailing `.[]`) still raises, because
`vivify_del_comma_iterate_targets`'s `trailing_bare_iterate_prefix` only recognizes a
prefix ending in a bare (optionally `?`-suppressed) `.[]` -- a sibling like
`.missing[].x` falls through to the ordinary, read-based comma-branch resolution instead,
which raises the same way #2324's own fan-out gap does:

```bash
$ echo '{"y":1}' | yq -o=json 'del(.missing[].x, .y)'   # {} in real yq
$ echo '{"y":1}' | succinctly yq -o json 'del(.missing[].x, .y)'   # still raises
```

The non-comma single-target form (`del(.missing[].x)` alone) is fixed and does not have
this gap -- only the comma-grouped combination of "suffix after the tolerated `.[]`" *and*
"another sibling in the same call" is affected. Tracked as
[#2380](https://github.com/rust-works/succinctly/issues/2380).

### `.[]` over a non-container is now a silent no-op — two residual gaps remain (#2346)

Real yq's `.[]` (iterate) produces zero output — not an error — for *any* non-container
target (number, string, bool, `null`), not just `null`, confirmed live against yq v4.53.3
for plain reads, `del()`, and assignment alike (`echo '{"x":5}' | yq -o=json '.x[]'` is
empty, exit 0). jq has no such rule (`.[]` on a scalar always raises there, confirmed
against jq 1.7.1), so this is yq-mode only — fixed at every `Expr::Iterate` site reachable
from a plain read, `del()`, or a `path()`-style path computation (#2346). `=`/`|=`'s own
`set_path`/`update_path` machinery already had this right beforehand.

Two adjacent sites were deliberately left unfixed, since a naive copy of the same widening
would be wrong (not just incomplete) or risks a worse regression:

- **`map(f)` still raises** when path-tracking is needed inside `f` (`eval_stage_with_path_
  context`'s own `Expr::Builtin(Builtin::Map(f))` arm) — `map`'s real yq-mode rule is
  *asymmetric* (`null` -> `[]`, but any other scalar passes through **unchanged**, confirmed
  live: `5 | map(.+1)` is `5`, not `[]` or empty), not the uniform "always empty" rule `.[]`
  itself gets. Tracked as [#2375](https://github.com/rust-works/succinctly/issues/2375).
- **`first(.a[])` on a genuinely-resolved scalar still raises** (`resolve_iterate_bounded`,
  shared by `resolve_node`'s own `Expr::Iterate` arm and `first`/`limit`'s fast path) —
  this function is also reached by `resolve_del_path_branches`'s comma-fanout fallback for a
  declined vivify pre-pass, where an out-of-range index read is a placeholder for a write
  real yq's own vivify would perform, not a genuinely-resolved scalar; naively widening this
  function turns an honest raise (`del(.[-1], .[4][])`, a documented, tracked coverage gap)
  into a silently wrong answer instead (confirmed live: `[1]` instead of real yq's `[1,
  null, null, []]`). Tracked as
  [#2376](https://github.com/rust-works/succinctly/issues/2376).

### Other categories

Float and number formatting ([#1071](https://github.com/rust-works/succinctly/issues/1071),
[#1129](https://github.com/rust-works/succinctly/issues/1129),
[#1356](https://github.com/rust-works/succinctly/issues/1356),
[#1358](https://github.com/rust-works/succinctly/issues/1358)), error-message rendering
(succinctly's previews format via jq's rules rather than yq's verbatim echo —
[#1055](https://github.com/rust-works/succinctly/issues/1055)), comment placement
([#1079](https://github.com/rust-works/succinctly/issues/1079),
[#1080](https://github.com/rust-works/succinctly/issues/1080),
[#1085](https://github.com/rust-works/succinctly/issues/1085)), and a missing
explicit-tag slot: `OwnedValue` has no field for it, so any
computed/constructed value — including an object-slice result (#1102) — loses the source
node's `!!map`/`!!seq` tag and quoting style in YAML output (`{"a":1,"b":2} | yq
'.[0:2]'` is `!!map\n- "a"\n- 1` on real yq, plain `- a\n- 1` on succinctly); `-o=json`
output is unaffected, since neither appears in JSON
([#1416](https://github.com/rust-works/succinctly/issues/1416)). Also see
[yq Query Language Reference § Known Limitations](../../reference/yq-language.md#known-limitations)
for the feature-level gaps (position builtins after DOM conversion; `file_index`/`key`/
`document_index` inside object literals or `any`/`all`).

## Where the two modes deliberately differ from each other

Not divergences — these are ADR-0018 rule 2 working correctly. The same filter text means
different things in `sjq` and `syq` because the two reference tools disagree, and succinctly
follows each one in its own mode. The behavioural axis is `EvalSemantics`
([src/jq/eval.rs](../../../src/jq/eval.rs)) — twelve `const`s plus an `EvalTag` identity tag
— together with a handful of `S::TAG` tests at individual call sites:

| Behaviour | `succinctly jq` (jq 1.7.1) | `succinctly yq` (yq v4.53.3) |
|---|---|---|
| Bare 2-arg `sub(re; s)` | first match only (`"aaa"` → `"Xaa"`) | **every** match (`"aaa"` → `"XXX"`) |
| `@uri`/`@base64`/`@html` on a container | JSON-encodes first (`[1,2]` → `"%5B1%2C2%5D"`) | errors, as real yq does |
| `@base64d` on malformed/misplaced padding | truncates at the first `=`, decodes the rest (`"===="` → `""`) | validates padding placement in place, rejects anything else (`"===="` → error) |
| `keys` | sorted | document order (yq's `keys` *is* `keys_unsorted`) |
| Integer overflow | converts to float | wraps |
| Division by zero | errors | infinity |
| `%` on floats | truncates operands | float modulo |
| `has()`/`in()` on a negative index | `false`; type mismatch errors | `true` unconditionally; type mismatch is `false` |
| `array * array` | type error | replaces (plus the merge-flag suffixes) |
| `array + non-array` | type error | appends as one element |
| `null` in a `*` merge | every pairing errors | acts as an empty container |
| `2.0 == 2` | `true` | `false` (strict int/float distinction) |
| Bare `halt_error` exit code | `5` | `1` |
| `7 + null` / `null - 7` | `7` / error | error / `7` |
| String-repeat (`s * n`) allocation limit | none — refuses only once the allocation genuinely can't be made (#1612) | explicit ~10MiB cap (`MAX_STRING_REPEAT_BYTES`), matching real yq's own deliberate refusal before any allocation is attempted |

Eleven of these rows are `EvalSemantics` constants, each carrying its live-verification note
in the trait's doc comments — `7 + null` / `null - 7` is one row over two constants,
`ADD_RIGHT_NULL_REQUIRES_CONCAT_TYPE` and `SUB_LEFT_NULL_IS_IDENTITY`. **Four are not.**
Bare `sub`, container `@uri`/`@base64`, and `@base64d`'s malformed-padding strictness are all
`S::TAG == EvalTag::Yq` tests at their call sites in
[src/jq/eval.rs](../../../src/jq/eval.rs), and `keys` is rewritten to `KeysUnsorted` at parse
time under `ParserMode::Yq` ([src/jq/parser.rs](../../../src/jq/parser.rs)). More than forty
`S::TAG` sites exist in `src/` in total.

That split is the debt ADR-0018 rule 2 is aimed at, not a counterexample to it: a constant is
discoverable from the trait definition, whereas a call-site test is discoverable only by
grep — which is how a mode difference ends up re-derived instead of looked up. Prefer a
constant for a new row, and lift a branch to one when you are already editing it. Either way,
because a builtin generic over `S: EvalSemantics` is shared by both modes, any change to one
must be verified in the other (ADR-0018 rule 2).

## Extensions

succinctly's extension surface falls into two groups. Under ADR-0018 rule 5 both are a
third category: permitted where marked as extensions and where they change the behaviour of
no filter the reference also accepts — an extension is not a divergence.

**Wholly new syntax, unconditional** — `at_offset`/`at_position`, `@dsv`. Neither jq nor yq
has anything resembling these; there is no reference token to gate them against.

**jq-styled syntax real yq's lexer rejects, gated behind `--jq-extensions`, off by default
([#1512](https://github.com/rust-works/succinctly/issues/1512))** — `paths`, `getpath`,
`leaf_paths`, `tostream`/`fromstream`/`truncate_stream`, `IN`, `ltrimstr`, `limit`,
`isempty`, `debug`, `infinite`, `isnan`, and `gsub`/`scan`/`splits`. `succinctly yq` matches
real yq's rejection of all of these by default; the flag opts back into the jq-compatible
surface. `leaf_paths` is grouped here even though it isn't real jq syntax either — real jq
itself rejects it too (`leaf_paths/0 is not defined`; it's a succinctly-only invention
modeled on a jq community recipe, see CLAUDE.md) — because, from `succinctly yq`'s
syntax-surface point of view, it's the same kind of thing as the rest of this list: extra,
off by default.

`gsub`/`scan`/`splits` specifically: real yq's lexer rejects all three outright, at any
arity ([#1436](https://github.com/rust-works/succinctly/issues/1436)) — this isn't "3-arg
`gsub` diverges," it's that yq's grammar has no `gsub`/`scan`/`splits` token at all:

```bash
$ echo '"aaa"' | yq 'gsub("a";"X")'      # Error: 1:1: lexer: invalid input text "gsub(\"a\";\"X\")"
$ echo '"aaa"' | yq 'gsub("a";"X";"g")'  # Error: 1:1: lexer: invalid input text "gsub(\"a\";\"X\";\"g\"..."
$ echo '"aaa"' | yq 'scan("a")'          # Error: 1:1: lexer: invalid input text "scan(\"a\")"
```

| Filter on `"aaa"` | real yq | succinctly (default) | succinctly `--jq-extensions` |
|---|---|---|---|
| `gsub("a";"X";"g")` | `Error: 1:1: lexer: invalid input text` | parse error, names the flag | `"XXX"` |

The full builtin list is documented, with examples, in
[yq Query Language Reference](../../reference/yq-language.md#gated-jq-builtins---jq-extensions)
and in [CLAUDE.md](../../../CLAUDE.md) — not enumerated a second time here.

**Not extensions, though a draft of this page listed them as such:** `--front-matter`,
`--split-exp`, and cross-file evaluation. All three are real yq surface —
`yq --help` lists `-f, --front-matter` and `-s, --split-exp`, and yq evaluates across
multiple files under `eval-all` — and
[#715](https://github.com/rust-works/succinctly/issues/715) filed all three together as
*missing* yq features, not as succinctly inventions. They therefore carry an ordinary
fidelity obligation. succinctly's `--split-exp` is long-only because its `-s` is already
`--slurp`, which is a spelling divergence and belongs above, not here.

Getting this backwards is worse than it looks: rule 5 exempts extensions from rule 4, so
labelling reference surface an "extension" silently retires a fidelity obligation. Check
`yq --help` before adding to the list above.

## Provenance

| Artifact | Path |
|---|---|
| Version pin | [`tests/data/yq-golden/YQ_VERSION`](../../../tests/data/yq-golden/YQ_VERSION) |
| Golden fixtures | [`tests/data/yq-golden/cases/`](../../../tests/data/yq-golden/cases/) |
| Golden harness | [`tests/yq_golden_tests.rs`](../../../tests/yq_golden_tests.rs) |
| CLI behaviour tests | [`tests/yq_cli_tests.rs`](../../../tests/yq_cli_tests.rs) |
| Sync script | [`scripts/sync-yq-golden.sh`](../../../scripts/sync-yq-golden.sh) |
| Drift detector | `yq-drift` job in [`.github/workflows/ci.yml`](../../../.github/workflows/ci.yml) |
| Mode-behaviour axis | [`src/jq/eval.rs`](../../../src/jq/eval.rs) (`EvalSemantics`) |

The goldens are committed, so `cargo test` runs hermetically with no `yq` on PATH; the
`yq-drift` job re-checks them against the pinned binary, so a yq upgrade surfaces as fixture
churn rather than a silent mismatch.

## Depends On

- [ADR-0018](../../adrs/adr-0018.md) - the fidelity rule this page enumerates exceptions to
- [ADR-0017](../../adrs/adr-0017.md) - presentation-metadata side-trees (anchors, comments, style)
- [yq Query Language Reference](../../reference/yq-language.md) - feature coverage
- [YAML 1.2 Compliance](../yaml/1.2.md) - scalar type resolution, incl. the 1.1 numeric forms

## Used By

- [yq benchmarks](../../benchmarks/yq.md) - comparison against `yq`

## Source & Docs

- [`src/jq/eval.rs`](../../../src/jq/eval.rs) - `EvalSemantics`, the per-mode behaviour axis
- [`src/bin/succinctly/yq_runner.rs`](../../../src/bin/succinctly/yq_runner.rs) - `enforce_anchor_soundness`
- [`src/jq/document.rs`](../../../src/jq/document.rs) - `DocumentFields`, incl. the `keys_dedup()` violation
- [jq Limitations](../jq/limitations.md) - the jq-mode counterpart to this page
- [mikefarah/yq](https://github.com/mikefarah/yq) - upstream reference
