# yq Behavioural Conformance and Known Divergences

[Home](../../../) > [Docs](../../) > [Compliance](../) > yq Limitations

This page records where `succinctly yq` behaves differently from `mikefarah/yq`, and why.
It is the yq-mode counterpart to
[jq Error Message Conformance](../jq/limitations.md), and it exists because
[ADR-0018](../../adrs/adr-0018.md) requires it: that record makes yq-fidelity the rule for
yq mode, permits divergence only under three named conditions, and obliges every divergence
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
against the three permitted conditions — including the one below that fails them, which is
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
is readable, no data is corrupted and no process dies, so none of the three conditions
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

### Duplicate mapping keys

The subject of [ADR-0018](../../adrs/adr-0018.md)'s worked example. Real yq preserves
duplicate keys almost everywhere but collapses them under iteration, and succinctly matches
neither side consistently — and, worse, answers differently depending on whether the same
logical document arrived as JSON or YAML:

```bash
$ printf '{"b":1,"a":2,"b":3}' > dup.json
$ printf 'b: 1\na: 2\nb: 3\n'   > dup.yaml

$ yq            -o=json -I=0 'length' dup.json   # 3
$ succinctly yq -o=json -I=0 'length' dup.json   # 2   <- format leaking into behaviour
$ succinctly yq -o=json -I=0 'length' dup.yaml   # 3

$ yq            -o=json -I=0 '[.[]]' dup.yaml    # [3,2]   — iteration collapses
$ succinctly yq -o=json -I=0 '[.[]]' dup.yaml    # [1,2,3]
```

ADR-0018 rule 2 identifies the cause — `DocumentFields::keys_dedup()`
([src/jq/document.rs](../../../src/jq/document.rs)) gates on input *format* where the
reference tools decide on *mode*.

**Both divergences above are [#1398](https://github.com/rust-works/succinctly/issues/1398).**
[#1385](https://github.com/rust-works/succinctly/issues/1385) is scoped to **jq mode** (its
own body: *"In jq mode, `succinctly jq` emits duplicate JSON object keys verbatim"*) and
names [#1342](https://github.com/rust-works/succinctly/issues/1342) (`paths(node_filter)`),
[#1343](https://github.com/rust-works/succinctly/issues/1343) (`-s`/`--eval-all`/`-i` bypass
the fix) and [#1344](https://github.com/rust-works/succinctly/issues/1344)
(`tostream`/`walk`/`recurse`) as the yq-side continuation. None of the four covers the format
leak or the missing `.[]` collapse, which is why #1398 exists — recording them here is the
first half of ADR-0018 rule 6, and filing them is the second.

Object slicing (`.[S:E]` on an object, [#1102](https://github.com/rust-works/succinctly/issues/1102))
is another surface this same root cause reaches: it must materialize the target object into
an `OwnedValue::Object` (an `IndexMap`) to build yq's AST-child-list view before slicing it,
which silently collapses a genuine duplicate key the same way `to_owned()` does everywhere
else in this list — `a: 1\nb: 2\na: 3` sliced with `.[0:6]` real yq returns `["a",1,"b",2,"a",3]`
(6 children, both `a` entries present); succinctly returns `["a",3,"b",2]` (4 elements, the
first `a`/`1` pair gone). Unlike the other surfaces above, there is no cursor-preserving
alternative available here — the operation inherently needs to reorder/slice the entries, not
just stream them — so this one is a real limit of `OwnedValue::Object`'s representation, not
a missed wiring like #1343/#1344.

Pulling the other way: [#442](https://github.com/rust-works/succinctly/issues/442),
[#478](https://github.com/rust-works/succinctly/issues/478) and
[#868](https://github.com/rust-works/succinctly/issues/868) are closed decisions that
deliberately made output *preserve* duplicate keys. They stand for yq mode; ADR-0018 rules 2
and 3 revise them for jq mode only.

### 3-argument `sub(re; s; flags)`, and a `split(re; flags)` sibling mystery

The bare 2-arg form matches (see the next section). The 3-arg form does not, and real yq's
own behaviour here has resisted every hypothesis tried
([#1122](https://github.com/rust-works/succinctly/issues/1122)) — it returns an empty string
for a `"g"` flag and ignores `"i"` entirely, and its `gsub` does not accept a third argument
at all — indeed does not exist as a builtin at all, along with `scan`/`splits`
([#1436](https://github.com/rust-works/succinctly/issues/1436), found during #1426's own
investigation below):

| Filter on `"aaa"` | real yq | succinctly |
|---|---|---|
| `sub("a";"X";"g")` | `""` | `"XXX"` |
| `sub("A";"X";"i")` | `"aaa"` | `"Xaa"` |
| `gsub("a";"X";"g")` | `Error: 1:1: lexer: invalid input text` | `"XXX"` |

`split`'s 2-arg `split(re; flags)` form has an unrelated, equally unexplained mystery of its
own ([#1439](https://github.com/rust-works/succinctly/issues/1439)) — it doesn't do a regex
split at all:

```bash
$ echo '"a1b2c"' | yq            -o=json 'split("[0-9]";"g")'   # ["a","1","b","2","c"]
$ echo '"a1b2c"' | succinctly yq -o=json 'split("[0-9]";"g")'   # ["a","b","c"] -- jq-modeled regex split
```

### Regex flag grammar — `test`/`match`/`capture` fixed, three siblings still open

**Fixed by [#1426](https://github.com/rust-works/succinctly/issues/1426):** real yq doesn't
use jq's flag grammar at all for `test`/`match`/`capture` — only `g` is a real flag; every
other jq-style character (`i`/`x`/`s`/`m`/`n`/`l`/`p`, including `l`/`n`, ADR-0019's own
permanent *jq*-mode gaps) is rejected, with `i` getting a distinct message pointing at yq's
inline-pattern alternative:

```bash
$ echo '"abc"' | yq            -o=json 'test("abc";"l")'   # Error: unrecognised match params 'l', ...
$ echo '"abc"' | succinctly yq -o=json 'test("abc";"l")'   # was: true (silently accepted); now: the same error
```

Deliberately **not** extended to `sub`/`split` (see the mysteries just above — applying this
rule there without first understanding their actual argument semantics would be a new,
unverified guess) or to `gsub`/`scan`/`splits` (not real yq builtins at all, per #1436 —
flag validation is moot for a call real yq would reject before ever reaching it).

### Presentation metadata lost on two whole output routes

Neither route builds a `CommentTree`, so both drop comments, style **and** anchors together:
`--inplace` never builds one ([#1349](https://github.com/rust-works/succinctly/issues/1349)),
and any filter yielding multiple results — anything containing a comma — loses its cursor
before one can be captured
([#1361](https://github.com/rust-works/succinctly/issues/1361)). Both predate
[ADR-0017](../../adrs/adr-0017.md)'s mechanism and neither is anchor-specific.

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
no-ops), but only for the *static*-path walkers (`get_path_mut`/`set_path`/`update_path`).
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

**Fixed by [#1298](https://github.com/rust-works/succinctly/issues/1298):** `get_path_mut`
(the walker #1232 widened) used to have no `Expr::Iterate` arm at all, so a mid-chain `.[]`
under plain `=` (`.a[].b = 99`) always errored "invalid path component" — in both jq and yq
mode, and predating #1232 entirely (`|=`/`+=`/`-=`/`*=`/`del()` were unaffected, their own
recursive-descent walkers already supported fan-out). #1298 added `split_at_iterate`/
`set_path_through_iterate` so `=` fans out per element like every other operator, and gave
`navigate_read_only`'s prefix walk (`yq_assign_is_total_noop`'s own eager-RHS-discard
pre-check, #1232's `PrefixNavOutcome`) an `Expr::Iterate` arm too — but only for the
narrowest case, `.a` itself being a genuine scalar (`.a[].b = error("boom")` on `a: 5` now
no-ops silently, RHS never evaluated, matching real yq).

**Still open ([#1432](https://github.com/rust-works/succinctly/issues/1432)):** real yq's
actual RHS-discard rule for a mid-chain `Iterate` is broader — a real container whose own
elements *all* individually no-op also discards the RHS, and so does an empty or
null-autovivified container (vacuously). The underlying *write* already gets all of these
right (verified against real yq); only the RHS-discard optimization is narrower than real
yq's own rule:

```bash
$ printf 'a: [1, 2]\n' | yq            -o=json '.a[].b = error("boom")'   # {"a":[1,2]}, no-op
$ printf 'a: [1, 2]\n' | succinctly yq -o=json '.a[].b = error("boom")'   # Error: boom
```

### Other categories

Float and number formatting ([#1071](https://github.com/rust-works/succinctly/issues/1071),
[#1129](https://github.com/rust-works/succinctly/issues/1129),
[#1356](https://github.com/rust-works/succinctly/issues/1356),
[#1358](https://github.com/rust-works/succinctly/issues/1358)), error-message rendering
(succinctly's previews format via jq's rules rather than yq's verbatim echo —
[#1055](https://github.com/rust-works/succinctly/issues/1055)), comment placement
([#1079](https://github.com/rust-works/succinctly/issues/1079),
[#1080](https://github.com/rust-works/succinctly/issues/1080),
[#1085](https://github.com/rust-works/succinctly/issues/1085)), the regex engine's
zero-width-match handling
([#1255](https://github.com/rust-works/succinctly/issues/1255) — real yq uses Go's
`regexp`), and a missing explicit-tag slot: `OwnedValue` has no field for it, so any
computed/constructed value — including an object-slice result (#1102) — loses the source
node's `!!map`/`!!seq` tag and quoting style in YAML output (`{"a":1,"b":2} | yq
'.[0:2]'` is `!!map\n- "a"\n- 1` on real yq, plain `- a\n- 1` on succinctly); `-o=json`
output is unaffected, since neither appears in JSON
([#1416](https://github.com/rust-works/succinctly/issues/1416)). Also see
[yq Query Language Reference § Known Limitations](../../reference/yq-language.md#known-limitations)
for the feature-level gaps (`*`/`+` are not cartesian generators; position builtins after DOM
conversion).

## Where the two modes deliberately differ from each other

Not divergences — these are ADR-0018 rule 2 working correctly. The same filter text means
different things in `sjq` and `syq` because the two reference tools disagree, and succinctly
follows each one in its own mode. The behavioural axis is `EvalSemantics`
([src/jq/eval.rs](../../../src/jq/eval.rs)) — eleven `const`s plus an `EvalTag` identity tag
— together with a handful of `S::TAG` tests at individual call sites:

| Behaviour | `succinctly jq` (jq 1.7.1) | `succinctly yq` (yq v4.53.3) |
|---|---|---|
| Bare 2-arg `sub(re; s)` | first match only (`"aaa"` → `"Xaa"`) | **every** match (`"aaa"` → `"XXX"`) |
| `@uri`/`@base64`/`@html` on a container | JSON-encodes first (`[1,2]` → `"%5B1%2C2%5D"`) | errors, as real yq does |
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

Ten of these rows are `EvalSemantics` constants, each carrying its live-verification note in
the trait's doc comments — `7 + null` / `null - 7` is one row over two constants,
`ADD_RIGHT_NULL_REQUIRES_CONCAT_TYPE` and `SUB_LEFT_NULL_IS_IDENTITY`. **Three are not.**
Bare `sub` and container `@uri`/`@base64` are `S::TAG == EvalTag::Yq` tests at their call
sites in [src/jq/eval.rs](../../../src/jq/eval.rs), and `keys` is rewritten to
`KeysUnsorted` at parse time under `ParserMode::Yq`
([src/jq/parser.rs](../../../src/jq/parser.rs)). More than forty `S::TAG` sites exist in
`src/` in total.

That split is the debt ADR-0018 rule 2 is aimed at, not a counterexample to it: a constant is
discoverable from the trait definition, whereas a call-site test is discoverable only by
grep — which is how a mode difference ends up re-derived instead of looked up. Prefer a
constant for a new row, and lift a branch to one when you are already editing it. Either way,
because a builtin generic over `S: EvalSemantics` is shared by both modes, any change to one
must be verified in the other (ADR-0018 rule 2).

## Extensions

succinctly adds capabilities neither reference has — `at_offset`/`at_position`,
`leaf_paths`, `@dsv`. Under ADR-0018 rule 5 these are a third category: permitted where
marked as extensions and where they change the behaviour of no filter the reference also
accepts. They are documented in
[yq Query Language Reference](../../reference/yq-language.md) and in
[CLAUDE.md](../../../CLAUDE.md), not here — an extension is not a divergence.

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
