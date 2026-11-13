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
matching `&name` exists, is emitted **earlier**, and holds an **identical** value —
structural identity, not jq value equality, so `NaN` counts as identical to itself even
though `NaN != NaN` under jq's own `==`
([#1360](https://github.com/rust-works/succinctly/issues/1360): under jq equality an
anchored `.nan` looked diverged from itself for no real reason, and lost its mark); otherwise
the mark is dropped and the value printed:

```bash
$ printf 'a: &x 1\nb: *x\n' | succinctly yq 'del(.a)'
b: 1
```

The identical-value clause is also the backstop for alias *node identity*
([#1351](https://github.com/rust-works/succinctly/issues/1351)). Real yq treats `&x`/`*x` as
one node: a write whose path passes *through* an alias and continues (`.b.p = 9` on `b: *x`)
mutates the anchor's node and every position follows, while a path that ends exactly at the
alias (`.b = 5`) rebinds that position only. succinctly's `jq::alias_identity` redirects such
a through-path onto the anchor's own path before the write runs (transitively for an alias
inside an anchored mapping that is itself aliased) and mirrors the result back into every
alias slot that was still an untouched copy, so `b: *x` survives:

```bash
$ printf 'a: &x {p: 1, q: 2}\nb: *x\nc: *x\n' | succinctly yq '.b.p = 9'
a: &x {p: 9, q: 2}
b: *x
c: *x
$ printf 'a: &x {p: 1}\nb: *x\nc: *x\n' | succinctly yq -o=json -I=0 '.[] .p += 1'
{"a":{"p":4},"b":{"p":4},"c":{"p":4}}        # three positions, one node: matches yq
```
A write to the node an update targets (`. = X`, `. |= f`, `del(.)`, `setpath([]; X)`) rebinds that position and leaves the shared node alone, as in yq ([#6861](https://github.com/rust-works/succinctly/issues/6861)): `.b |= (. = {"p": 3})` over `b: *x` changes `b` only, where `.b |= (.p = 9)` (a write below the target) reaches the shared node and every position follows.
Whether the redirect applies is one answer per update, so a body that writes below the target and then replaces it is written positionally as a whole:

- `.b |= (.p = 5 | . = {"p": 3})`, where yq leaves `a` and `c` at `p: 5` and here they keep `p: 1`;
- `.b |= (del(.p) | . = {"p": 3})`, where yq empties the shared node (`a: &x {}`) and here it keeps `p: 1`.


The redirect is gated on structural identity: it applies only while the alias position
still holds the anchor's value, so a position rebound earlier in the pipe (`.b = 5 | .b.p =
9`) is written positionally, as yq treats a rebound position. The gate's one false positive
is a rebind to a value *identical* to the anchor's: `.b = .a | .a.p = 9` (or `.b = {"p": 1, "q":
2} | .a.p = 9`) detaches `b` in yq (`b: {p: 1, q: 2}`), while succinctly cannot tell the
rebound copy from an untouched one and keeps its value in step (`b: {p: 9, q: 2}`; the `*x`
mark itself is cleared by the plain `=`, see the assignment rule below). The written value is
never affected, only which other positions follow it. Two further
consequences are recorded divergences/limitations rather than matches:

- **A value-producing update at an alias node is not discarded — rule 4(b).** Real yq
  no-ops `.b |= . + 1`, `.b += 1`, `.b += [3]` and `.b |= . + {"r": 3}` at an alias node
  (the document comes out unchanged) and errors on `.b |= reverse` (`node at path [b] is
  not an array (it's a )`), `.b |= keys` (`cannot get keys of , keys only works for maps
  and arrays`) and `.b *= {..}` (`cannot multiply  with !!map`). succinctly rebinds the
  position with the computed value, consistent with `.b |= 5` (which yq also rebinds).
  `.b |= (.p = 9)` and `.b |= del(.p)`, whose body is itself a write, mutate the shared node
  in both tools.
- **A declaration deleted earlier in the pipe leaves nothing to redirect to.**
  `del(.a) | .b.p = 9` writes `b` positionally and `c` does not follow (yq prints `b: *x` /
  `c: *x` with no `&x` anywhere — the unreadable-output case above). Likewise an alias
  group's paths are recorded against the pristine document, so an array-shifting stage
  ahead of the write (`del(.items[0]) | .items[0].n = 9`) leaves them one element off
  ([#2501](https://github.com/rust-works/succinctly/issues/2501), the value-level twin of
  #870).

Wherever the redirect declines, the values differ and the identical-value clause drops the
mark and prints the computed value, rather than emitting `*x` and discarding the write.

The redirect runs wherever a write goes through `evaluate_yaml_cursor` — stdout,
`--split-exp`, `--front-matter`, and since
[#1349](https://github.com/rust-works/succinctly/issues/1349) `--inplace` too (`yq -i
'.b.p = 9'` writes `a: &x {p: 9, q: 2}` / `b: *x` back to the file). `--eval-all`
evaluates the collected documents through the owned-value route, which has no redirect, no
comments and no style, so a filter that reads the document *list* (`[.]`, `,`, `.a = .b`,
`|= . + 1`, a builtin, a variable) still writes `b` positionally and drops the marks. A filter
that only navigates (`.`, `.a.b`, `.[0]`, `.[]`, `..`) or writes a literal (`.b.p = 9`,
`.n += 1`, `del(.a)`, and `.a //= 7` under `--jq-extensions`) takes the plain per-document route instead and keeps all of it
([#4380](https://github.com/rust-works/succinctly/issues/4380); the gate is
`eval_all_matches_per_document` in `m2_gate.rs`). The rerouted run first evaluates the list
the old way, so the bridge's refusals (nesting depth, colliding keys, stray commas), a later
file failing `--validate`, and an erroring document that leaves stdout empty are all unchanged;
only a run that succeeds is printed by the plain route. JSON-sourced input stays on the old route.
The gate claims no worse than the owned-value route, not a byte-for-byte match: real `ea`
doubles every result of `.[0]` over a stream of arrays and separates the results of `.[]`
across files its own way, neither of which either route reproduces. The plain route also
keeps a written-into empty `{}`/`[]` in flow style where yq switches to block
(`{}` | `.a = 5` is `{a: 5}`, yq `a: 5`), so a rerouted `ea` inherits that. A multi-document stream applies the redirect and the
anchor marks to every document, on stdout and under `--inplace` alike
([#2520](https://github.com/rust-works/succinctly/issues/2520): the report that it reached the
last document only no longer reproduces; pinned by the `yaml_multidoc_alias_*_2520` goldens and
`test_inplace_write_through_alias_applies_to_every_document_2520`).

The same soundness gate also governs `.c = .b`-style plain assignments
(split out of #1351 as [#2497](https://github.com/rust-works/succinctly/issues/2497)):
`propagate_assign_alias_marks` copies an alias mark onto the assignment's target whenever the
value side is a static read of an aliased node, then leaves `enforce_anchor_soundness` to
decide whether that mark actually survives. Writing an alias *onto* the very node that
declares its own anchor destroys the only remaining declaration, so both marks come out
unresolvable and both are dropped:

```bash
$ printf 'a: &x 1\nb: *x\n' | yq '.a = .b'
a: *x
b: *x
$ printf 'a: &x 1\nb: *x\n' | yq '.a = .b' | yq '.'
Error: bad file '-': yaml: line 1, column 5: unknown anchor 'x' referenced

$ printf 'a: &x 1\nb: *x\n' | succinctly yq '.a = .b'
a: 1
b: 1
```

Discovered mechanically rather than special-cased: `enforce_anchor_soundness` runs
unchanged afterwards and simply finds no `Declares` mark left for `x` to resolve either
alias against.

Since [#2505](https://github.com/rust-works/succinctly/issues/2505) the same pass also reaches the plain shapes #2497 left out:
a target under a parent the write created (`.c.d = .b`, `.c[0] = .b`), a comma of targets (`(.c, .d) = .b`), a negative index on the source (`.c = .b[-1]`, read from the comment tree, which knows the array's length) and a source container that holds an alias (`.c = .b` on a block `b:\n  - *x`, each alias inside it stays one in the copy).
Pinned by `tests/yq_cli/i2505.rs`.
A negative index in the first assignment of a chain, source or target (`.a[-1] = .a[0]`), is counted from the end of the array as the document had it (#5407), so the copy carries the source's quoting, comment and style.
The alias marks of such a target are placed the same way for a lone assignment (`.a[-3] = .a[-4]` over an anchored element).
In a later assignment of a chain it is still not a static path to this pass (`.a += ["z"] | .a[-1] = .a[0]`, where the array length is not the document's), so the copy stays plain.
Since #5249 a construction of static reads keeps each alias among them (`.c = [.b]`, `.c = {"z": .b}`, nested or under a comma of targets).
A construction whose slot index the tree cannot tell stays plain where yq prints `*x` (`.c = [.b[]]`, a dynamic key such as `{("k"): .b}`, a read of a key the tree lacks, which yq leaves out of the array).
Still plain where yq prints `*x`, each listed on #5249 (the residual of #2505): a right side that names several nodes through something other than a comma of static reads (rule 4(b) below for the shapes that change the value), a compound assignment other than `+=`/`-=` into an empty or sequence target (`.c += .b` over a scalar alias keeps it since #5249), and the alias group a copy joins (`.c = .b | .a = 5` leaves `c` an alias of the rewritten anchor in yq).
Since [#5249](https://github.com/rust-works/succinctly/issues/5249) the source's trailing comment and flow style are copied too (`.c = .b` on `b: *x # bc` is `c: *x # bc`, on `b: [*x]` it is `c: [*x]`), and so is everything under a copied container (each item's and field's comment and style).
This is `propagate_assign_copied_presentation`, and it is not specific to aliases: `.c = .b` on `b: [1, 2] # t` copies the comment and the flow style of a document with no alias in it.
The source's comment wins when it has one, and the target's own comment stays when it has none, as in yq.
The node's own style is flow when either node is flow, and otherwise the source's unless the target was a non-empty container of the same kind, which keeps the style it had (`b: [1]` over a block `c` is `c: [1]`, a block `b` over a non-empty flow `c` stays flow, a block `b` over an empty `c: []` is block).
The copy is skipped when a later stage rewrites the source (`.c = .b | .b = 5` leaves `c` plain where yq prints `c: [1] # t`), because presentation read from the finished tree no longer describes the target; the same holds when a later stage edits or replaces the target (`.c = .b | .c[0].a = 2`, `.c = .b | .c = 5`, where yq prints `c: [{a: 2}] # t` and `c: 5 # t`).
A target that is not a static path (`.[] = .b`, `.c[] = .b`) is not reached either.
Since [#5257](https://github.com/rust-works/succinctly/issues/5257) the source scalar's quoting and an explicit tag are copied too, and a container's tag.
The source's style and tag are taken when the target is new, is not a scalar, is empty, is plain with no tag, or changes type (`str` to `int`), and the target keeps its own otherwise (`.c = .b` over `c: "old"` stays double-quoted whatever `b` was, over `c: old` it becomes `b`'s `'q'`).
A custom tag already on the target stays (`c: !foo old` takes `b: x` as `!foo x`), and a custom tag on the source is copied whatever the quoting does.
A tag names a container's type, so a `!foo` mapping over a flow `{y: 2}` is a change of type and takes the source's block style, and a flow `[1]` over a block `c: !!seq` keeps the block layout.
A flow container copied onto a scalar or flow target keeps the target's same-line comment (`c: [1] # tc`); a block copy cannot, and drops it (the [#5258](https://github.com/rust-works/succinctly/issues/5258) rule).
Swept against yq v4.53.3 over 459 source-by-target pairs of scalar and container styles, tags and comments.
Still not copied, where yq copies them: the node's own head and foot lines, and an anchor declaration nested inside a copied container (yq prints the `&y` twice; the pass writes no anchor mark of its own, so only the alias marks of [#2505](https://github.com/rust-works/succinctly/issues/2505) reach the copy).
Still different for a copy: a flow container copied over a block target whose key line carries a comment (`c: # tc` / `  - 9`; yq drops the comment, this prints `c: [1] # tc`), a literal or folded source written over a single-quoted target is `c: "q\n"` here and a multi-line single-quoted scalar in yq (the string emitter, not the copy), a custom-tagged target given a `!!str 5` source reads the text as `5` (an integer) in yq's `-o=json` and stays the string `"5"` here, and under `-P` a container copied onto a target that carries an anchor or tag prints `c: &t # sc` here where yq prints the comment and the tag on separate lines, which it cannot read back.

**Rule 4(b)**, on the same `propagate_assign_alias_marks` pass: a computed `value` side
(`.c = (.b + 1)`) is not a static read of `.b`, so the pass leaves `.c` a plain scalar
rather than aliasing it. Real yq does alias it, and the result is data loss — it prints
`c: *x` but the value underneath is still `.b`'s own `1`, not the computed `2`:

```bash
$ printf 'a: &x 1\nb: *x\n' | yq '.c = (.b + 1)'
a: &x 1
b: *x
c: *x
$ printf 'a: &x 1\nb: *x\n' | yq '.c = (.b + 1)' | yq '.c'
*x

$ printf 'a: &x 1\nb: *x\n' | succinctly yq '.c = (.b + 1)'
a: &x 1
b: *x
c: 2
```

A related gap sits in **truthiness**, and it is now the whole of the rule rather than a
carve-out. Since [#2692](https://github.com/rust-works/succinctly/issues/2692), *a filter
validates only what it materializes* — and a filter that asks whether a value is truthy
answers with `DocumentCursor::is_falsy`, which is O(1) and decodes nothing. So `select`,
`if`, `not`, `and`, `or`, `//` and zero-arity `any`/`all` validate nothing at all, at any
position. This is not a yq-fidelity divergence — real yq rejects the whole document at parse
time, so there is no yq behaviour to match either way — and it is sanctioned by
[ADR-0018](../../adrs/adr-0018.md)'s #2103 amendment, which lists it as an instance.

```bash
$ printf 'a: &a ["bad\\q"]\nb: *a\n' | succinctly yq 'select(.b) | 1'
1
$ printf 'a: &a ["bad\\q"]\nb: *a\n' | succinctly yq 'select(.a) | 1'
1
$ printf 'a: &a ["bad\\q"]\nb: *a\n' | succinctly yq '.b | not'
false
$ printf 'a: &a ["bad\\q"]\nb: *a\n' | succinctly yq '.a'
Error: invalid escape sequence
$ printf 'a: &a ["bad\\q"]\nb: *a\n' | succinctly yq '.b[0]'
Error: invalid escape sequence
```

**The history is worth keeping, because it explains the shape of the code.**
[#1804](https://github.com/rust-works/succinctly/issues/1804) first made `select`/`if` skip
a *container* reached through an alias, fixing a real `O(2^N)` cost on a document shaped as
a chain of anchors each referencing the previous one twice; it deliberately excluded a bare
scalar-target alias (`a: &a "bad\q"`), since resolving one scalar costs `O(1)` and was never
part of that cost. [#2476](https://github.com/rust-works/succinctly/issues/2476) extended
the same trade-off to `not`, `//` and `any`/`all`, which until then reached the cost only by
falling to `eval_single`'s wildcard bridge.

Both of those were narrowings of a validation walk, and both left the answer depending on
*which position* a filter stood on: `.b | not` answered while `.a | not` raised, for two
names of one node. #2692 removed the walk instead, so neither the container/scalar
distinction nor the alias/anchor one has any observable consequence left. Nothing that
raised for a *reading* reason stopped raising — `.a`, `.b[0]` and every write still do.

`and`/`or` are no longer special here either. A bare `.b and true` used to be distinguished
from `.b | (true and true)` because `.b` as a direct operand needs path context and so ran
through `eval_boolean_generic`'s path-context branch (present since #2473), resolving `.b`'s
path and raising as `.b[0]` does. Both spellings now answer `true`: resolving a path is not
decoding the value at it.

**[#2173](https://github.com/rust-works/succinctly/issues/2173) goes one step further, and
this one *is* a new yq divergence rather than an extension of an existing one.** Every
`succinctly yq` filter takes `eval_single`, so unlike jq mode nothing here had a native
streaming arm to escape through: the wildcard bridge materialized the whole document for
`1+1` as much as for `.a`, and validated it in passing. That rejection matched real yq,
which parses eagerly. It no longer does, for a filter that provably reads nothing.

Captured live against yq v4.53.3 on `a: "\x"` / `b: 5` (an invalid escape, which yq
rejects at parse time whatever the filter):

| filter                              | real yq | succinctly before | succinctly now |
|-------------------------------------|---------|-------------------|----------------|
| `1+1`, `[1+1]`                      | exit 1  | exit 1 — matched  | `2`, `- 2`     |
| `true and true`, `false // 1`       | exit 1  | exit 1 — matched  | `true`, `1`    |
| `{"k":1}`, `1 as $x \| $x`          | exit 1  | exit 0            | unchanged      |
| `.b`                                | exit 1  | exit 0            | unchanged      |
| `. and true`                        | exit 1  | exit 1            | `true` (#2692) |

So four spellings stop matching yq here, and `. and true` joins them a step later. The rows
below them are why that is the *uniform* answer rather than a new inconsistency: `.b` and
`{"k":1}` already answered at exit 0 on this document, because navigation validates only what
it reads (#2168) and neither reaches the bad escape. Keeping `1+1` rejecting would have meant a
binary that answers `.b` and `{"k":1}` while refusing `1+1` on one document in one run — the
shape ADR-0018's #2103 amendment exists to rule out. `. and true` raised here under #2173's own
rule, because it reads `.` — but reading `.` as a boolean operand is *testing* it, not decoding
it, so #2692 (recorded above) takes it the rest of the way: `. and true` now answers `true`,
the same as `.b | (true and true)` always did.

Same disposition, same sanction, and the same escape hatch as the jq side — a user who
wants yq's rejection has `succinctly json validate` for JSON input, and the divergence is
confined to documents real yq would refuse outright. Pinned by
`test_closed_terms_do_not_validate_2173`'s yq twin behaviour through
`test_ambient_validation_agrees_with_bridge_2476`'s closed-probe rows, and by
`test_wildcard_bridge_over_alias_fanout_completes_2173`, which is also the memory guard:
on #1804's fan-out shape (`aN: &aN [*a(N-1), *a(N-1)]`) the ambient materialization was
`O(2^N)`, measured at 0.33 s / 1.31 s / 5.38 s / 22.70 s for N=18/20/22/24 and flat
0.00 s after.

**Accepted gap ([#2641](https://github.com/rust-works/succinctly/issues/2641)): a
value-producing read of an alias-bearing node costs its expanded size, on every route.**
#1804/#2476's `is_alias()` short-circuits are a property of *validation* walks, which only
ever need to know a node exists, never what it contains — they do not, and cannot, transfer
to a walk that has to build the value itself. `getpath(["items", {"start":s,"end":e}])`'s
array-slice fast path (`getpath_walk_cursor`, `src/jq/eval_generic.rs`, added by #2604) calls
`to_owned_cursor` once per sliced element, and `to_owned_cursor_at_depth` expands an alias
into a copy of its target (the copy model this file's alias section describes) — on #1804's
fan-out shape, the expanded value of `*aN` *is* `2^N` leaves, so materializing it costs
`2^N` regardless of how directly the walk reaches it. The same cost is paid, identically, by
every other route that materializes the same element: a literal-bound slice
(`.items[(0+0):(1+0)]`, via `eval_slice_expr` → `slice_one_generic` → `to_owned_all`) and
array construction (`[.items[]]`, #2575's territory). None of this is a walk bug to fix —
only a *navigating* read (`getpath(["items", 0])`, a `length`/`select` truthiness test) stays
flat, because it never has to copy the aliased subtree; a read that hands back the subtree's
own value cannot avoid paying for its expanded size. Measured on the same fan-out shape,
release binary: the in-range slice `getpath(["items",{"start":0,"end":1}])` (aliased element
inside the range) took 0.02 s / 0.06 s / 0.24 s / 0.96 s at N=14/16/18/20 — the same `×2`-
per-level curve as #1804's walk cost, before that walk was ever involved — while the
out-of-range slice `getpath(["items",{"start":1,"end":3}])` and the navigated read
`getpath(["items", 0])` both stayed at 0.00 s through N=20. Real `yq` v4.53.3 answers the
in-range case in 0.01 s at N=20 regardless, because go-yaml keeps `*aN` as a reference and
never expands it — closing this gap for succinctly would need an `OwnedValue` able to hold a
shared subtree instead of a copy (the direction ADR-0017 and #1416 point at), not a walk fix.

**Resolved ([#1350](https://github.com/rust-works/succinctly/issues/1350)).**
`enforce_anchor_soundness` takes a `sort_keys` argument and has always handled it correctly
on the DOM path; the cursor-streaming path used to never call it, reproducing the unsound
output it is supposed to prevent. The streaming path now checks `sort_keys &&
index.has_aliases()` once per file/stdin input (an index it already built to decide the fast
path in the first place) and, when true, evaluates that whole input through the DOM
evaluator instead — the same one `-P` already used, so the two paths now agree:

```bash
$ printf 'b: &x 1\na: *x\n' | succinctly yq --sort-keys '.'
a: 1
b: &x 1
$ printf 'b: &x 1\na: *x\n' | succinctly yq --sort-keys '.' | succinctly yq '.'
a: 1
b: &x 1
```

A sound document (no inversion) still streams normally, unaffected:

```bash
$ printf 'a: &x 1\nz: *x\n' | succinctly yq --sort-keys '.'
a: &x 1
z: *x
```

A bare alias *root* (`succinctly yq '.b'` on `b: *x`) used to print `*x` literally on the
streaming path while the DOM path (`-P '.b'`) resolved it — also part of #1350, since a root
alias can never survive `enforce_anchor_soundness` either way (its `&name` would have to sit
inside its own subtree, which index build already rejects as a cycle). Both paths now
resolve it:

```bash
$ printf 'a: &x 1\nb: *x\n' | succinctly yq '.b'
1
```

[#2486](https://github.com/rust-works/succinctly/issues/2486) extended the same fallback to
every M2-streamable filter, not just identity (`--sort-keys '.outer'`, `--sort-keys '.[]'`,
...) — the fallback now evaluates the real filter expression instead of always assuming `.`,
so any shape that can reach the M2 fast path gets the same soundness check.

[#2855](https://github.com/rust-works/succinctly/issues/2855)'s `sort_keys(f)` builtin (as
opposed to this `--sort-keys` flag) takes the identical licensed divergence on the same
inversion shape, through the same `enforce_anchor_soundness` pass — it's a write, so it
always takes the DOM route already:

```bash
$ printf 'b: &x 1\na: *x\n' | yq 'sort_keys(..)'              # real yq: unsound
a: *x
b: &x 1
$ printf 'b: &x 1\na: *x\n' | succinctly yq 'sort_keys(..)'   # sound: value, not the mark
a: 1
b: &x 1
```

**A mapping key's anchor rides the same machinery
([#2598](https://github.com/rust-works/succinctly/issues/2598)).** A key's `&k` is recorded
in the per-key side map next to its comment and quoting, re-emitted before the key
(`&k key: &v 1`), and declared to the soundness gate with the key's own text as its value, so a
value alias that targets it (`b: *k`) survives a write while the key is still emitted before it.
The same licensed divergence applies when it is not: deleting the key, or `sort_keys` moving it
below its alias, expands the alias to the key's text (`c: key`) where yq prints an
unresolvable `*k`.
An alias used *as* a key (`*a : 2` in block context, `{*a: 1}` in flow, [#5469](https://github.com/rust-works/succinctly/issues/5469)) is kept on every route, and on a write it is held to the same three rules with the key's text as its value.
One that fails them prints that text: `del(.b)` on `b: &a xy` / `c: {*a : 1}` prints `c: {xy: 1}` where yq prints an unresolvable `*a`, and so does a write whose output leaves the anchor out (`.c | .zz = 1`).
That text keeps the anchor's quoting, so a key whose anchor is `"true"`, `'1'` or `!!str 12` prints as `"true": 1`, `'1': 1` or `"12": 1` and reads back as a string, not as the bool or integer key a bare `true: 1` is ([#5813](https://github.com/rust-works/succinctly/issues/5813)).
A navigation that streams the node without its anchor (`.c`, `.[]`) prints `*a` as yq does, as it already does for a value alias, and that output names no anchor.
`.b = "q"` prints the key as `xy` as well: yq keeps `*a`, which reads back as the key `q`, but succinctly's key is the text it read and a write to the anchor does not reach it.
An alias to a sequence or mapping used as a key is printed as the key's text too, which is `''` here.
`keys` writes each key as its node ([#5469](https://github.com/rust-works/succinctly/issues/5469)): `- &k !!str key`, `- !!merge <<`, `- *a`, on the streaming route and on the DOM routes `-P`, `--arg` and `--sort-keys` take.
So does a `keys` result built into a construction of reads (`[.c | keys]`, `{x: (.c | keys)}`, `[keys]`, `.c | keys + [.b]`).
A mapping with a `~` key, which `keys` lists as `"~"` where yq lists `null` ([#2802](https://github.com/rust-works/succinctly/issues/2802)), leaves the whole construction bare, and so does a `keys` result taken through `-` (`[(.c | keys) - ["q"]]`).
One that a later stage takes further (`[.c | keys] | .[0]`) or that is written back (`. |= keys`) still prints each key bare.
A plain `<<` anywhere, value as well as key, is the `!!merge` tag yq writes (`a: !!merge <<`) and reports (`tag`, `type`), and a computed `"<<"` is quoted ([#5833](https://github.com/rust-works/succinctly/issues/5833)).
Writing the equal string over a plain `<<` (`.a = "<<"` on `a: <<`) keeps `!!merge <<` where yq writes `"<<"`, since a write of an equal value keeps the node's tag, as it does an explicit one (`!foo x`) in both tools; the value read back is the same.
Clearing the tag of a `<<` (`.a tag = ""`) writes `!!merge <<` where yq writes a bare `<<`, since an untagged `<<` here is a computed string and is quoted; both read back as the merge scalar.
An explicit `!!merge <<` is not told apart from a plain `<<`, so `.a tag = "!!str"` on it writes `"<<"` where yq writes `!!str <<`; both read back as the string `<<`.
On the streaming route and under `--sort-keys` a tagged key that decodes to a line break (`? !!str a` / `` / `  b`) is double-quoted (`- !!str "a\nb"`) where yq writes a literal block (`- !!str |-`), and under `-P` an anchored empty key is `- &k ""` where yq writes `- &k`; both read back as the same key.
A `<<` merge
expands the merged mapping's keys where yq keeps `<<: *m`. The expansion does not re-declare the
source's anchors ([#2513](https://github.com/rust-works/succinctly/issues/2513)): `default: &d` /
`  a: &y 1` / `item: {<<: *d, b: 2}` written with `.item.b = 3` prints `a: 1` under `item`, not a
second `&y` that every later `*y` would resolve to. That holds for a source written elsewhere in the
output, outside the mapping or a sibling of it; an inline source (`<<: {a: &y 1}`, or one inside a `<<`
sequence), which is written nowhere else, keeps its declaration. An alias inside the merged-in fields
(`*y`) is kept.
A `<<` whose value contributes nothing to merge (a scalar, `null`, an alias to a sequence of scalars, a sequence with no mapping in it, an empty mapping) stays an entry of the rewritten mapping since [#6318](https://github.com/rust-works/succinctly/issues/6318).
`- <<: *seq` over `seq: &seq [one, two]` was written `- {}`, losing the key and the alias; it is `- <<: *seq` now, where yq writes `- !!merge <<: *seq`, which reads back as the same merge scalar.
The rendering that `tojson` and a nested value read lists such an entry too (`"<<": 5`), where yq's JSON encoder skips a scalar one and refuses an alias to a sequence (`can only use merge anchors with maps (!!map) or sequences (!!seq) of maps`).
`.[]` and `-o json` skip it, as yq does.

Related open items in the same family, still unresolved:
[#1359](https://github.com/rust-works/succinctly/issues/1359) (a write that changes a node's
kind drops its `&anchor`, where real yq keeps it),
[#1352](https://github.com/rust-works/succinctly/issues/1352) and
[#1353](https://github.com/rust-works/succinctly/issues/1353).

**`sort_keys(f)` does not vivify an absent target, before or past a fan-out
([#2855](https://github.com/rust-works/succinctly/issues/2855),
[#2870](https://github.com/rust-works/succinctly/issues/2870)).** `sort_keys(.nope)` on a
document with no `.nope` leaves it unchanged (unlike plain `|=`, which creates it), and since #2870
so does a field after an `Iterate`: `sort_keys(.a[].b)` over `a: [{b: 2, q: 1}, {q: 3}]` creates no
`b` on the second element, as in yq. A path that fans out through `.[]` is spelled out element by
element and an element that lacks a *field* the path names is dropped; an index past the end still
pads, as it does in yq (`sort_keys(.a[][5])`). A path with a slice keeps the route it always took,
which leaves `sort_keys(.a[0:1][])` unchanged, as in yq.

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
permitted rather than bug-for-bug reproduced. (`succinctly yq` once also had a join-output
flag, `-j`/`--join-output`, with the same guard; real yq's `-j` is a deprecated `--tojson`
alias, so [#1731](https://github.com/rust-works/succinctly/issues/1731) remapped it and `-0`
is the only terminator that can now reach the guard.)

Scope note: this entry covers only the `---`-boundary corruption. `-0` output whose *value
content* itself contains a raw NUL byte is a separate matter, fixed by
[#1709](https://github.com/rust-works/succinctly/issues/1709): succinctly now rejects such
a result the same way real yq does (`can't serialise value because it contains NUL char and
you are using NUL separated output`), matching real yq's own flush-then-error atomicity
(earlier results in a stream are still flushed; the offending result and everything after it
are not) rather than either the whole-document-buffering approach an earlier attempt at that
fix tried and abandoned (PR #1767, +65% peak RSS on a 100MB document) or silently emitting
the raw byte.

Two interactions were left open by #1709 itself (the second since closed by #2004):

1. This check only fires on bytes that are genuinely unescaped in the rendered output.
   `-o=json -0 '.a'` *without* an explicit `-r` prints a properly JSON-escaped `"b\u0000c"`
   and succeeds, as real yq does (live-verified against Homebrew yq v4.53.3: JSON's default
   unwrap-scalar setting is `false`, unlike YAML's `true` — real yq needs `-r` explicit to
   unwrap a JSON scalar). succinctly's `raw_output` used to OR in `-0`/`-j` regardless of
   output format, so this one combination unwrapped (bypassing JSON's own escaping) and
   tripped the NUL check on the raw byte; since
   [#1996](https://github.com/rust-works/succinctly/issues/1996) `-0` no longer implies it,
   and `-o=json -r -0` still raises, as yq does. (`-j` still implies `-r`; see the `-j`
   collision entry below and #1731.)

2. **`--color`/`--colors` combined with `-0` keeps the flush-then-error atomicity**
   ([#2004](https://github.com/rust-works/succinctly/issues/2004)). Real yq's own `--colors
   -0` flushes earlier valid results before erroring on a later NUL-containing one —
   live-verified: `yq --colors -0 '.[]'` on `["hello", "wor\0ld", "x"]` prints `hello\0` to
   stdout, then errors on the second element, exit 1. succinctly used to buffer the whole
   multi-result render for `--colors` and scan that buffer once before writing anything, so a
   NUL anywhere discarded every earlier result too.
   It now writes the results that end before the first NUL and then raises the same error, with
   the same stdout and exit code as yq.
   Since [#808](https://github.com/rust-works/succinctly/issues/808) `--colors` streams, and `--colors -0` checks one result at a time like uncolored `-0`: each clean result is colorized as it is flushed, which is the same text as that slice of the whole colored render, because the colorizer resets its lexer state at each result boundary.

### Anchor-on-alias at document-root level — rule 4(b)

An alias node carrying its own `&anchor`/`!tag` (`&a *b`) is invalid per the YAML 1.2
grammar — an alias node has no properties of its own. Real yq and PyYAML both confirm this
live for the ordinary shape, inside a mapping or sequence:

```bash
$ printf 'a0: &a0 hello\na1: &a1 *a0\nz: *a1\n' | yq .
Error: bad file '-': yaml: while parsing a block mapping at <unknown position>: line 1, column 9: did not find expected key
```

succinctly's default loader now rejects this uniformly ([#1374](https://github.com/rust-works/succinctly/issues/1374),
`YamlError::PropertyOnAlias`; see
[YAML Limitations](../yaml/limitations.md#three-exceptions-an-alias-with-no-usable-target-or-an-illegal-decoration-is-rejected)) —
for the ordinary shape above this is bug-for-bug fidelity, not a divergence: real yq errors,
succinctly errors.

The one shape where this *was* a deliberate divergence (until #6276, see the end of this section) is at document-root level, across a
`---` multi-document separator. There, real yq does not error at all — it silently emits
corrupted output instead, concatenating the first document's scalar value with the literal
*text* of the following `---`-anchor-alias line:

```bash
$ printf -- '--- &a0 hello\n--- &a1 *a0\n' | yq .
---
hello--- &a1 *a0
$ printf -- '--- &a0 hello\n--- &a1 *a0\n' | yq -o=json .
"hello--- &a1 *a0"
```

(Both confirmed live against yq v4.53.3, exit 0 — real yq does not treat this as an error at
all.)
succinctly rejected this shape rather than reproduce the corruption, per rule 4(b).
Since #6276 it reproduces it, with no divergence left to record: yq's header slurp glues the line after a leading `--- text` line onto it (see the header bullet above), so the second `---` is scalar text and the result is the single string `"hello--- &a1 *a0"`, which is no corruption of any alias.
An alias with its own anchor wherever a marker is a marker is still rejected, as in yq.

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

### Evaluator nesting-depth ceiling — rule 4(c)

Real yq has no nesting ceiling: it evaluates a 300-deep block-style mapping like any other
(`sort`, `. as $x | $x`, `-P .` and the rest all exit 0). The evaluator here recurses on the
native stack and refuses a document nested more than 256 containers deep where it has to
materialize it (a scalar inside the 256th container is read, #6618), which is what ADR-0018 rule
4(c) permits (matching would take the host process down).
The YAML parser's own 128 ceiling counts only flow/compact-inline nesting, so block-style
nesting reaches the 256 one:

```bash
$ yq         sort deep300.yaml    # prints the document, exit 0
$ succinctly yq sort deep300.yaml # Error: nesting depth exceeds limit of 256, exit 1
```

The refusal is a diagnostic and exit 1 on every route — default, `-P`, `--arg`,
`--slurp`, `--eval-all`, `--split-exp`, `--inplace` — never a panic (exit 101), and in a
multi-document stream the documents on either side of the offending one are still
evaluated and printed (#1817, #2304). `tests/yq_cli_tests.rs` pins this
(`test_deep_block_yaml_*_1817`). Streamed shapes that never materialize the document
(`.`, `.[]`, `keys`, `length`, `map(.)`) have no ceiling at all.

### Expression nesting ceiling on a chain of comparisons — rule 4(c)

Real yq parses a chain of comparisons of any length: `1 == 1 == ... == 1` with 100,000 operands answers `false`.
`succinctly yq` folds the chain from the right, as yq does, into one nested node per link, and a program nested past the 256 expression-nesting limit (`MAX_EXPR_DEPTH`) is refused at parse time.
`1 == 1 == ... == 1` with about 250 operands still runs; one with 1,000 is `expression nesting exceeds depth limit of 256`, exit 1.
That is ADR-0018 rule 4(c): matching would take the host process down, because the evaluator and `Drop` walk the tree on the native stack.
It is the ceiling a long run of `+` already has.
Pinned by `tests/yq_cli/i5629.rs`.

### A JSON integer past 2^53 keeps its exact digits — rule 4(b) ([#3605](https://github.com/rust-works/succinctly/issues/3605))

yq's JSON decoder reads every number through a `float64`, so an integer a document spelled past
`2^53` that has no exact `float64` is rounded before anything else sees it; the rounded value
then prints, compares and does arithmetic as that value. succinctly reads the integer token as
an exact `i64` and keeps it. Against yq v4.53.3 on
`[999999999999999999,9999999999999999,9007199254740993,123456789012345678]` with
`-p json -o json -I=0`:

| filter                       | yq                      | succinctly yq          |
|------------------------------|-------------------------|------------------------|
| `.[]`                        | `1000000000000000000` `10000000000000000` `9007199254740992` `123456789012345680` | `999999999999999999` `9999999999999999` `9007199254740993` `123456789012345678` |
| `.[0] \| tostring`           | `"1000000000000000000"` | `"999999999999999999"` |
| `.[0] \| length`             | `19`                    | `18`                   |
| `.[0] + 1`                   | `1000000000000000001`   | `1000000000000000000`  |
| `.[0] == 999999999999999999` | `false`                 | `true`                 |

Rule 4(b) permits the refusal: matching rewrites a number the document spelled exactly, on every
print and every write (`-i`, `-p json -o yaml`), so a round trip through a JSON document changes
data that nothing in the query touched — and a comparison against the literal the document
holds is `false`. It is not rule 4(a): yq's own output is readable; the divergence is chosen
because matching corrupts data. The same integer in a *YAML* document is exact in both tools
(`a: 999999999999999999` with `.a + 1` is `1000000000000000000`), so the divergence is the JSON
decoder's alone (a `.json` file read without `-p json` rounds in yq and stays exact here as well). A
value past `i64::MAX` is clamped to `9223372036854775807` by both tools, so it agrees. The same holds
for negatives and on every route that prints or writes: the streaming print, `-o yaml`, `-P` and a
write that leaves the integer alone (`.[3] = 0`). Pinned, with the `2^53`, `2^62` and `i64::MAX`
boundaries, those routes and the YAML control, in
`test_json_integer_past_2_pow_53_keeps_its_exact_digits_3605`.

## Input-spec divergences (below ADR-0018's scope)

ADR-0018 adjudicates *evaluator and CLI behaviour*. A divergence originating in which inputs
the parser accepts, or in how a plain scalar's type resolves against a published spec, sits
below that line: it is settled by the spec succinctly targets and recorded on the relevant
spec page, not by a rule-4 carve-out. This section exists so the case is not mistaken for one.

### A leading byte-order mark is skipped, and yq's header slurp finds nothing after one (#5356, #5357)

A UTF-8 byte-order mark at the front of a YAML file is dropped from each input before it is indexed, as yq does.
Every claim here was captured from yq v4.53.3 and is pinned by `tests/yq_cli/i5356.rs`.
That covers stdin, files, `--eval-all`, `-i`, `load()`, and a file that holds only the mark, which counts as an empty file.
A mark that is not at the front of a file stays data, and a JSON document with a mark is refused, as in yq.
`yq-locate` and `at_offset`/`at_position` count from the first byte after the mark.
yq's default `--header-preprocess` looks for its leading blank, comment, `%YAML` and `---` lines at the first byte of the input, and a mark is not one of them.
A BOM-prefixed file therefore has no header: yq decodes it as an ordinary stream, as `--header-preprocess=false` reads the same text ([#7025](https://github.com/rust-works/succinctly/issues/7025): a text of only comments is the one place they part, below).
The first document's explicit `---` is read, not echoed, so a file whose first line is `---` prints without it where the same file without the mark prints it.
`---` / `---` / `x` is a null document and then `x`, blank lines and a comment's indent ahead of the first content are normalised away, and a text with no content is one empty document, a blank line.
The streaming routes read it that way ([#5357](https://github.com/rust-works/succinctly/issues/5357), `tests/yq_cli/i5357.rs`): stdin and files, with `.`, `-N`, `-I=0`, `-o=json` and a path.
The routes that gather a source before indexing it do too ([#6762](https://github.com/rust-works/succinctly/issues/6762), `tests/yq_cli/i6762.rs`): `-P`, a write such as `.a = 2`, `-i`, `--eval-all` and `--slurp`.
There the first document has no preamble to print back, `line` and `column` count every line (`---` / `a: 1` is line 2), and the blank lines or the marker between a head comment and the content are not part of the comment.
Each file is judged on its own, so a mark on the second file leaves it no header while the first keeps its own.
The comment block at the top of the first document is placed as go-yaml places it ([#7084](https://github.com/rust-works/succinctly/issues/7084), `tests/yq_cli/i7084.rs`): it goes forward onto the first node (`# lead` / `a: 1` is `lead` on `.a`'s key, `# l1` / `# l2` the same, and a comment above a first `---` the same), except the blocks a blank line separates from what follows, which are the head of the document node (`# l1` / blank / `# l2` / `a: 1` is `l1` on `.` and `l2` on `.a`'s key, `# lead` / blank / `a: 1` is `lead` on `.`), so `.` prints `# l1` / `# l2` and a scalar document prints no comment.
That placement was checked on 4,320 runs against yq v4.53.3 (every leading block of up to four comment, blank and `---` lines before a map, a nested map, a sequence, a flow collection and a scalar, each alone, with a second document and with a foot comment after, under `.`, `-o=json` `head_comment`/`foot_comment` and `-o=props`): 1,203 that differed from yq before agree, none that agreed differs, and 236 still differ, which are the shapes below and a foot comment under a key that has a line comment (`a: 1 # one` / `# f1` is the key's foot in yq and the document's here, with or without a mark).
These shapes still differ from yq after a mark, each captured from v4.53.3:
- Some comment blocks around a first `---` or a scalar or flow-collection document: `# c1` / `---` / `# c2` / blank / `a: 1` is `c1` on the key's head and `c2` on its foot in yq, both as the head here; a blank line, `---`, `# c1`, a blank line and `a: 1` has `c1` on the key's foot in yq and on its head here; the head comment of a scalar or flow-collection document that has two blocks (`# l1` / blank / `# l2` / `5`) is `l1# l2` in yq and the two blocks with their blank line here; and `-P` over a scalar document prints its comment where yq prints none.
- A first document written on its marker line (`--- # c`, `--- !!map`, `--- |`) is not read as yq reads it: `--- # c` / `a: 1` prints `# c` / `a: 1` in yq, `--- !!map` / `a: 1` prints `!!map` / `a: 1`, and `--- |` / `  x` is a parse error here.
- `%YAML 1.2` as the first line is an error in yq (`found incompatible YAML document`), and is read here.
- A text with only a comment prints a blank line on the streaming routes, as in yq, and still prints the comment (or, for a write, keeps it above the result) on the DOM routes.

### `--front-matter` splits as yq does; four rows still differ (#6910)

The front matter is the text up to the first line that starts with `---` at column 0, after the input's own leading `---` line (blanks around it allowed); that line starts the body, which `process` writes back as it was.
An input with no front matter, an unterminated one, a commented or indented opening line, and a `...` (which closes nothing) are read as yq reads them, where they were refused with `no front matter found` or `unterminated front matter`.
The opening `---` is the front matter's own line, written back as the input had it (CRLF included) when the result is the front matter document.

Still different:
- a scalar read of a front matter that has an opening `---` (`--front-matter process '.a'` over `---` / `a: 1` / `---` / `body`): yq prints `1` / `---` / `body`, with no `---` before the `1`, and this prints one;
- a filter that fails: yq prints nothing, and the closing line and the body are still written here;
- a write over an empty input (`.a = "N"` over nothing): a blank line before `a: N`;
- a front matter that is not one YAML document (`---` / `a: 1` / `...` / `body`): yq prints the first document and then fails, this fails at once.

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

A zero-padded decimal that holds an `8` or a `9` (`08`, `0018`, `-09`) is a `!!float` in yq, and here since [#6866](https://github.com/rust-works/succinctly/issues/6866).
go-yaml tries a leading `0` as octal, which those digits break, and falls back to the float, so `a: 08` is `8.0` in JSON and keeps its spelling elsewhere.
`007`, `0755` and `010` stay integers in both.
Its JSON is `8.0` on every route since [#6899](https://github.com/rust-works/succinctly/issues/6899): a value that is bound, copied, merged, written through or rebuilt (`. as $x | $x`, `. * .`, `.zz = 1`, `with_entries`) and `tojson`/`@json` of it.
The owned value holds the literal `08` as a float, and the JSON writers ask for the text of a literal by its number, since the same text is the integer `8` under an explicit `!!int` tag.
An explicit `!!int 08` is the integer 8 on those routes too since [#6927](https://github.com/rust-works/succinctly/issues/6927): the reindex bridge writes it under its own token, so it is not read back as the float.

### A tag with no value is the empty string, except `!!int` and `!!float` (#6291)

The one tag that is not the empty string is the non-specific `!`, a null ([#6942](#a-lone--on-an-empty-node-is-null-6942)).

A node that carries an explicit tag and no value (`k: !!seq`, `- !custom`, `--- !!str`) is the empty string in yq's JSON output and in a comparison with `""`.
The tag decides the kind, so the empty text is not a null: `!!bool` is `false`, and only `!!null` is `null`.
This reads it that way for every tag (`resolve_tagged`, and `value_of_empty_node` where the tag is the last thing in the text), where it used to answer `null` for every tag but `!!str` and `!!bool`.

```console
$ printf 'k: !!seq\nj: 1\n' | yq -o=json -I0 .                  # {"k":"","j":1}
$ printf 'k: !!seq\nj: 1\n' | succinctly yq -o json -I0 .        # {"k":"","j":1}   ({"k":null,"j":1} before #6291)
```

`!!int` and `!!float` with no value are a yq encoder failure in JSON output (`json: error calling MarshalJSON ...: strconv.ParseInt: parsing "": invalid syntax`, and `strconv.ParseFloat` for `!!float`), and `yq '.k + 1'` fails the same way.
This answers the empty string for both, as it does for `!!int abc`: the loader absorbs what it cannot read instead of reproducing a Go crash.

Not covered here:
- An operator that reads the node's tag on one of these.
  yq refuses `.k + 1` (`!!seq cannot be added to !!int`, and the same for `!!map`, `!!binary` and a custom tag; a duration error for `!!timestamp`), `.k * 2`, `.k | keys` and `.k | to_number` with errors that name the tag.
  This reads the empty string and answers `"1"` for `.k + 1`, an empty line for `.k * 2`, and the errors of a string for the other two (`!!str` differs from yq the same way for `keys` and `to_number`).
  Before this change `.k + 1` answered `1`, from a null, so it was wrong in another way.
- `!!bool` compared with a string. yq compares a scalar's text, so an empty `!!bool` equals `""` and `!!bool yes` equals `"yes"`, where this compares the boolean ([#6292](https://github.com/rust-works/succinctly/issues/6292)).
- A value-less tag in a flow collection (`[!!str]`, `{a: !!str}`), which yq refuses and this reads as `[""]` ([#6293](https://github.com/rust-works/succinctly/issues/6293)).

### A lone `!` on an empty node is null (#6942)

The non-specific tag `!` (and its verbatim spelling `!<!>`) decides no kind, so on a node with no value it is the empty plain scalar, a null, in yq: `a: !`, `- !`, `{a: ! , b: 1}` and a whole document of `!` are `null` in JSON output, a comparison with `null`, `tag` and `type`.
Every other tag with no value is the empty string (#6291 above).
This reads it that way (`resolve_tagged` answers no kind for an empty `!`, and `value_of_empty_node` leaves a `!` node null; `is_non_specific_tag` is the one definition of the tag), where it answered `""` before.
A `!` with a value is unchanged: `a: ! 12` is the number `12` and `a: ! x` the string `x`, in both tools.
`tag` answers the natural tag for a `!<!>` with a value (`!!int` for `a: !<!> 12`), as it already did for `!`; it answered `!` before.

```console
$ printf 'a: !\nb: 1\n' | yq -o=json -I0 .                  # {"a":null,"b":1}
$ printf 'a: !\nb: 1\n' | succinctly yq -o json -I0 .        # {"a":null,"b":1}   ({"a":"","b":1} before #6942)
```

Not covered here:
- The YAML output of these nodes, since [#6948](https://github.com/rust-works/succinctly/issues/6948).
  yq drops the `!` it reads (`a: !` prints `a:`, `a: ! x` prints `a: x`), and so does this, on every route.
  The cursor reports the non-specific tag as no tag, so the cursor writers and the tag of a written document agree.
  A string written into such a node therefore reads back as itself (`.x = "12"` over `x: !` is `x: "12"`, `.x = ""` over `x: ! a` is `x: ""`), and a write that fills the null prints `a: N`.
  A `!` on a key is gone from the key too, so `! 1: v` has the key `1` in `keys` (it was the string `"1"`), as in yq.
- A document that is only `!` prints nothing under `-o=yaml`, as `!!null` does, where yq prints a blank line (the empty-document output of #5721). It printed the blank line before #6942, because the node was the empty string.

### A zero-padded float across a write, and an underscored number under `!!int` or `!!float` (#6938)

A zero-padded `08` or `09` is a float of a YAML document, because go-yaml tries the leading `0` as octal and falls back to `ParseFloat` (#6866).
After any write the document is rebuilt, and that value crossed the rebuild as a spelling that `tag`, `type` and the integer readers took for the integer 8: `.zz = 1 | .p | tag` and `| type` over `p: 08` were `!!int`, where yq says `!!float` (the value itself, `.p` and `.p + 1`, was already the float `8.0` and `9.0`).
Both are `!!float` now, as is the same value through a construction (`[.p] | .[0] | tag`); `010` (an octal integer) and an explicit `!!int 08` stay `!!int`.

yq drops every `_` of a tag-forced number before it reads it (`parseInt64`), so `!!int 1_000` is `1000`, `!!int 0_8` is `8`, `!!int 0x1_f` is `31` and `!!int 1__0` is `10`, wherever the `_` sits.
`!!float` takes an `_` only between two digits (Go's `ParseFloat`): `!!float 1_0.5` is `10.5` and `!!float 1_0e1_0` is `1e11`, while `1__0`, `_1`, `1_` and `0x_1f` are refused.
This read all of them as the string.
A plain `1_000` is still the string here, the recorded input-spec divergence above.

```console
$ printf 'd: !!int 1_000\n' | yq -o=json -I0 '.d + 1'                   # 1001
$ printf 'd: !!int 1_000\n' | succinctly yq -o json -I0 '.d + 1'         # 1001   ("1_0001" before #6938)
```

Not covered here:
- A form yq cannot encode (`!!int 0b1_0`, `!!int 1_0.5`, `!!int 9_223_372_036_854_775_808`, `!!float 0x1_f`) is an error in yq's JSON encoder and an arithmetic error; this reads it as the string, as it does `!!int abc`.
  A prefixed integer under `!!float` (`!!float 0x1_2`, `!!float 0o1_7`) is the float `18.0`/`15.0` here, as `!!float 0x12` already was, where yq's float parser refuses it.
- An underscored `!!float` (`!!float 1_0.5`, `1_000`, `1_0e1_0`) keeps its text through `tostring`, YAML output, a copy and the JSON writers (`1000.0`), as an underscored `!!int` does (#6958). Three residuals: `-P` over a document that holds one keeps the `!!float` tag, where yq strips it (`a: 1_0.5`); `to_yaml` of one prints the tag under YAML output (`!!float 1_0.5`), where yq prints the text alone (JSON output agrees: `"!!float 1_0.5\n"`); and an untagged `a: 1_0.5` is the string `1_0.5` here, where yq reads the float 10.5 (the recorded input-spec divergence for underscored numbers).

### A scalar under an unknown `!!` tag is its text (#6299)

yq reads a plain scalar under a tag of the secondary handle that names no core type as its text: `a: !!foo 1` is the string `"1"`, and so are `!!binary 1`, `!!set 1`, `!!seq 1` and `!!str: 1`, where the `:` belongs to the tag.
A primary-handle custom tag (`!foo 1`) names nothing the loader knows and resolves like the untagged text, `1`.
The core tags force their own kind as before (`!!int`, `!!float`, `!!bool`, `!!null`, `!!str`).
`resolve_tagged` answers a string for every other `!!` tag, so the JSON, owned and comparison routes agree (88 rows, `tests/yq_cli/i6299.rs`).

```console
$ printf 'a: !!str: 1\n' | yq -o=json -I0 .                  # {"a":"1"}
$ printf 'a: !!str: 1\n' | succinctly yq -o json -I0 .        # {"a":"1"}   ({"a":1} before #6299)
```

The verbatim spelling of a core tag is that tag (`!<tag:yaml.org,2002:int> "1"` is `1`, `!<tag:yaml.org,2002:foo> 1` is `"1"`).

Shapes still read where yq differs:
- A named handle that no `%TAG` directive declares (`a: !x!y 1` is `found undefined tag handle` in yq) and a bare `!!` (`did not find expected tag URI`) are accepted.
- A `%TAG !! <prefix>` directive that redefines the secondary handle is not seen, so `!!foo 1` under it is `"1"` here and `1` in yq.
- An operator that reads the node's tag refuses it in yq and does not here: `.a + 1` on `a: !!foo 1` is `!!foo cannot be added to !!int` there and `"11"` here, `.a * 2` is `cannot multiply !!foo with !!int` and `"11"`, and `.a | keys` names the tag.
  This is the same family as the operators listed under #6291.

### Invalid UTF-8 in a `-p json` document — substituted for JSON output and in a YAML-output key; a bad value still differs on the prefix (#1616, #4536)

Real yq turns each invalid byte of a JSON document into U+FFFD and lets the *output* encoder
decide. succinctly does the first half on the bytes, before indexing, when the output is JSON
(one U+FFFD per invalid byte, Go's count, not jq's or WHATWG's), so `-p json -o json` succeeds
on every route (stream, `--slurp`, `--eval-all`, `-i`) as yq does. With YAML output the string
is left as it is and the decode error it already raised fires when it is printed, which is yq's
`yaml: cannot marshal invalid UTF-8 data as !!str` refusal, including that a filter that never
prints it succeeds (`{"a":"x\xe4y","b":1}` with `.b` is `1` in both). Live-verified against
yq v4.53.3:

| input / filter                       | yq                           | succinctly                     |
|--------------------------------------|------------------------------|--------------------------------|
| `-p json -o json .` on a bad value   | `"a": "x\ufffdy"` (escaped)  | `"a": "x�y"` (raw U+FFFD)      |
| `-p json .` on `{"x\xe4y":1}`        | `x�y: 1` (substituted key)   | `x�y: 1` (#4536)               |
| `-p json .` on a bad value           | the refusal, nothing printed | an error after the prefix, e.g. `a: ` |
| `-p json .a` on a bad value          | the raw bytes `x\xe4y`       | an error                       |

- **A substituted value prints as the raw character, not `\ufffd`.** It is the same JSON value;
  telling a substituted U+FFFD from a literal one through the index is not worth tracking.
- **A bad key with YAML output is substituted like yq's** (#4536): each invalid byte becomes U+FFFD
  in the key, in the printed identity, `keys`, `to_entries`, `with_entries` and `key` alike. It was
  emptied to `""` (the fallback for a key that will not decode,
  [#222](https://github.com/rust-works/succinctly/issues/222)) before. The elements of `keys` read
  the same way when they are read back as values (`keys | .[0]`, `[keys[]]`, `keys | map(.)`,
  `keys | join(",")`, #4741).
- **The refusal can follow output already written** (`a: ` before the error), where yq refuses
  before printing; the streaming writers have no error channel to check first (the "Stage 6" gap in
  [decode-failure-routing.md](../../plan/decode-failure-routing.md)).
- **Offsets count the substituted buffer**: with `-o json`, `at_offset(n)` counts each replacement
  as three bytes, not the one invalid byte of the file.

### A fused slice read does not validate the rest of the range (#4195)

`E[a:b][k]` and `E[a:b] | length` over a computed slice, and (since #4323) over a literal-bounded
slice (`.[2:][0]`, `.a[2:][0]`, and since #4406 `.a[2:] | length`), read the range's resolved bounds and convert only the
element they name, in yq mode as in jq mode. A malformed JSON value elsewhere
in the range (`[1,1.2.3,3]` read as `0 as $i | .[$i:][0]`) therefore does not raise, where the
slice written out in full (`(.[$i:]) | .[0]`) still does. It is an input-validation posture,
not an evaluator rule, and the same as `.[0]` and `length`; the table and the reasoning are in
[jq limitations § A fused slice read does not validate the rest of the range](../jq/limitations.md#a-fused-slice-read-does-not-validate-the-rest-of-the-range-4195).

### A slice of a sequence validates only the elements it keeps (#6231)

A literal-bounded slice of a sequence (`.[:10]`, `.c[2:]`) is read by index, not through the bridge that materialized the whole sequence, in yq mode as in jq mode.
It decodes only the elements it keeps, so a bad value inside an element it drops no longer raises, where yq rejects the document when it reads it.
On the sequence `- a`, `- "bad\x"`, `- 3`, `- 4`, `.[:1]` is `["a"]` (`-o=json`) and `.[2:]` is `[3,4]`; they raised `invalid escape sequence` before, as yq does.
`.[1:2]` and `.[:2]`, which keep the element, still raise.
The structure of the sequence is still checked as before.
The reasoning, the table and the pin are in [jq limitations § A slice of an array validates only the elements it keeps](../jq/limitations.md#a-slice-of-an-array-validates-only-the-elements-it-keeps-6231); the yq rows are in `tests/yq_cli/i6231.rs`.

### Nested explicit keys past 128 levels are refused ([#6956](https://github.com/rust-works/succinctly/issues/6956))

A run of explicit-key indicators (`? ? ? a`, `- ? - ? a`) is a key whose key is a key, and each level is a call of the parser.
yq reads up to 10,000 levels and stops past them (`exceeded max depth of 10000`, captured from v4.53.3).
This parser charged no depth for the recursion, so a few hundred thousand indicators took minutes and about 1.5 million aborted the process with a stack overflow.
It charges the limit a flow collection does now: 129 `? ` indicators in a row are read, the 130th stops the run with `nesting depth exceeds limit of 128` and exit status 1, and `- ? ` runs stop at the 129th.
That is rule 4(c) of the divergence rule, where matching yq would take the host process down.
The budget is the parser's, shared by explicit keys and flow collections, so a key nested 70 levels in explicit keys whose own content is a flow collection nested 70 levels is refused where yq reads it.
It applies to every entry point of the parser: `succinctly jq` over YAML input and the library's `YamlIndex` refuse the same documents.
A document with 130 to 10,000 nested explicit keys is read by yq and refused here; no real document has them.

### An explicit `?` key next to a block sequence — rejected when the key has content (#1104), the bare `?` shape is the key (#6370)

Real yq reads a block sequence at a mapping's indent as the implicit value of an ordinary
`key:` entry, but never as the value of an explicit key: `? k` / `- item` fails with
`did not find expected key`. succinctly now fails the same way for every key that holds a
node on the `?` line (`? k`, `? "k"`, `? [a]`, `? a: b`, `? - a`), with its own message
(`a sequence cannot be an explicit key's implicit value`).

A bare `?` followed by a sequence at its own column is the key's content, as in yq
([#6370](https://github.com/rust-works/succinctly/issues/6370); YAML test suite 6PBE):

```bash
$ printf -- '---\n?\n- a\n- b\n:\n- c\n- d\n' | yq -o=json -I=0 .
{"":["c","d"]}                      # succinctly: the same
$ printf '?\n- item\n' | yq -o=json -I=0 .
{"":null}                            # succinctly: the same
```

Still different, each live-verified against yq v4.53.3:

- **A key whose content starts on the next line** (`?` / `  k`) is read like one that starts on the
  `?` line (#4532): `?` / `  k` / `x: 1` is `{"k":null,"x":1}` and `?` / `  k` / `- item` is an
  error, as in yq. The content takes the same checks as on the `?` line, gaps included (`? : v` and
  `?` / `  : v` are `{"":null,...}` here where yq rejects them). One spelling is not read that
  way: node properties alone on that line (`?` / `  &a` / `  k`) still leave the key content to
  the main loop, which takes the next entry for its value.
- **A `-` line after a completed entry** (`? k` / `: v` / `- x`, and `a: 1` / `- x`) is an error
  here as in yq (#4530).

### A plain scalar on a line of its own is read only up to a line that opens like an item or carries node properties ([#5068](https://github.com/rust-works/succinctly/issues/5068))

A plain scalar goes on across the lines below it until a comment or a less indented line ends it.
A continuation line that opens like a sequence item (`- x`) or with node properties (`&a v`) is text to yq.
Here the scalar stops at that line and the remainder is dropped.
Captured live (yq v4.53.3):

| input                         | real yq          | succinctly yq                      |
|-------------------------------|------------------|------------------------------------|
| `- &a\n   v\n  - v\n  text\n` | `["v - v text"]` | parse error (`["v"]` before #5068) |
| `k:\n  &a v\n  v\n`           | `{"k":"v v"}`    | `{"k":"v"}`                        |

Since #5068 a line left over after such a read is a parse error where it used to be dropped without a word, and that is the only change to these inputs.
The nearest cases of the same family are `k:` / `  a` / `  z: 1` (yq: `mapping values are not allowed in this context`, here `{"k":"a z"}`) and a document root that is a bare plain scalar followed by more content (#5070).

### A syntax error after a document's first node: yq has printed that node already

yq decodes and prints one document at a time.
A syntax error later in the input therefore follows the output of every document before it, and for the same document if the error is in what follows its root node.
succinctly builds the whole index first, so the same input prints nothing and exits 1 with the error.
Captured live (yq v4.53.3, `-o json -I0 .`):

| input                      | real yq                            | succinctly yq |
|----------------------------|------------------------------------|---------------|
| `"q"\nb\n`                 | `"q"`, then the error (exit 1)     | the error     |
| `a: 1\n---\nk: a\n b: 1\n` | `{"a":1}`, then the error (exit 1) | the error     |

The exit status agrees and the message may differ (yq words a flow key that spans lines as `mapping values are not allowed in this context`); the stdout prefix is the difference that matters.
This holds for every parse error, and it is the divergence #5070 adds rows to: a scalar, flow collection or block scalar that is a document's root can have nothing after it before the next `---` (`did not find expected <document start>`).

### Content after a `...` end marker — rejected as yq does, with one shape that still differs ([#5089](https://github.com/rust-works/succinctly/issues/5089))

A document after a `...` marker needs its own `---`: `a` / `...` / `b` stops with `did not find expected <document start>` in yq and here (YAML Test Suite `7Z25` is valid YAML 1.2 and fails for that reason, as `CFD4` and `FRK4` do).
One shape still differs.
A directive after the marker (`a` / `...` / `%YAML 1.1` / `---` / `b`) is read as two documents in yq and here.
With `%YAML 1.2`, which spec examples 6.18, 6.21 and 9.6 are written with, yq stops with `found incompatible YAML document` (go-yaml accepts `1.1` alone), and so does `succinctly yq` since #6419, where it read two documents.
The library loader still reads it, as YAML 1.2 allows.
A stream whose first document is `--- text` followed by `...` and more content reads as one scalar in yq (`--- v` / `...` / `b` is `"v... b"`, the marker folded into the text), and since #6276 here too: yq's header slurp glues the line after the marker line onto it, so the `...` is no longer at the start of a line.

### A `%`, `@` or backtick that starts a node, and a directive line inside a document ([#6402](https://github.com/rust-works/succinctly/issues/6402))

go-yaml's scanner starts no plain scalar, key or value, block or flow, on `%`, `@` or a backtick (`found character that cannot start any token`), and it reads a `%` at column 0 as a directive before that, wherever the line stands.
So a line after a document has started that opens with one is an error in yq: `a: 1` / `%TAG ! x:`, `!!str` / `%YAML 1.2`, `a: %x`, `- @x`, `[a,` / `%x` / `]`.
`succinctly yq` refuses them as well, and reads them as content only where yq does: a `%` that continues a plain scalar, or stands in a quoted or block scalar (`---` / `x` / `%YAML 1.2` is `"x %YAML 1.2"`), and one inside a word (`a: x%y`).
yq prints the documents before the failing one and then the error, as for any later parse error (see above), so the stdout prefix differs.
Over four draws of 1,500 to 2,000 (7,500 in all) generated streams of two to five lines (`%YAML`, `%TAG`, `%x`, `@x` and a backtick at column 0 and after a key, an item and a flow comma, properties, markers, quoted and block scalars), 760 that differed from yq in whether they are refused now agree and none changed the other way.
A valid directive at column 0 where a node would start is not an error, though (#6419): it closes the document as a `...` does, whatever the document held (a pending property or value, a block scalar, a block collection), and a `---` has to follow.
So `a: 1` / `%TAG !e! w` / `---` / `b: 2` is two documents, as in yq, and `!!str` / `%TAG !e! w` / `---` is the empty string.
A `%YAML` that go-yaml sees is checked: it is `1.1` alone, once for a document, with nothing after it but blanks and a comment, and `%YAML 1.2` is `found incompatible YAML document`.
Only a `%YAML` line in the head of the stream, before any other directive, is yq's header and left alone.
An indented `%YAML` in the head of a stream is yq's header, set aside as a column-0 one is, and printed back as a comment (`# ` and the line, indent included) ([#6408](https://github.com/rust-works/succinctly/issues/6408)): ` %YAML 1.2` is a null document, and `  %YAML 1.2` / `a: 1` is `{"a":1}`.
On the route that rebuilds the document (a write such as `.a = 2`, or `-P`) the head's `%YAML` line is not printed back as yq prints it, for a column-0 line as for an indented one: yq keeps `%YAML 1.2` (and `#  %YAML 1.2` for an indented one), where a column-0 line is dropped here and an indented one prints as `%YAML 1.2`.

### Node properties on a line of their own with no owner, and a `- ` in a flow collection ([#6413](https://github.com/rust-works/succinctly/issues/6413))

Properties on a line of their own (`!!map`, `&a`) name the node that follows, and that node needs an owner like any other (#6343).
yq refuses them in the place of a sibling, level with the keys or items of a block mapping or sequence and not under a `key:` or `-` that waits for its value: `k: |` / `!!map`, `a: 1` / `&a`, `- a` / `!!str`, `k:` / `!!str x`.
`succinctly yq` refuses them as well.
The properties of a document (`!!map` / `k: 1`), of a value (`a:` / `  !!map` / `  b: 1`) and of a key on its own line (`a: 1` / `&a b: 2`) are read as before.
A `-` followed by a blank in a flow collection is the block sequence indicator, which go-yaml refuses and no plain scalar starts with: `[- x]`, `[a, - x]` and `{a: - x}` are errors in yq, and were the text `- x` here.
`-x` and a quoted `"- x"` are text in both.
A `-` after a node property on its line is refused in yq since [#6414](https://github.com/rust-works/succinctly/issues/6414): `&t - x`, `- &t - x`, `a: &t - x`, `- !!seq - x` and `? &t - x` are `block sequence entries are not allowed in this context`, and were the sequence `["x"]` here (`&t - a: b` was `{"- a": "b"}`).
A property on a line of its own above the `-` (`&t` / `- x`) is the sequence's property, and is read.
A tab among the blanks after the `-` (`-⇥x`, `- ⇥x`, `-⇥# c`) is `found character that cannot start any token`, where go-yaml skips no tab in the place of a simple key; after a `?` or `:` the blanks may end at a `#` (#6379), and after a `-` they may not.
The library loader reads both as YAML 1.2 does, and `key: - a` with no property still reads as the sequence ([#325](https://github.com/rust-works/succinctly/issues/325), #6355).
Over 518 generated documents (seven property forms, eleven contexts, six tails, and fourteen tab shapes in four contexts) the 409 that differed from yq in whether they are refused now agree, and a sweep of 2,308 valid documents near the two rules changed none the other way.
Over five draws of 2,000 generated streams of two to five lines (properties, keys and items with a value left to the next line, block scalars and flow dashes), 474 that differed from yq in whether they are refused now agree and none changed the other way.
yq prints the documents before the failing one and then the error, as for any later parse error (see above), so the stdout prefix differs.

## Open divergences (bugs, not decisions)

Representative cases, each live-verified. These are gaps to close, listed here so they are
not rediscovered from scratch.

### String style of output — closed on every route (#3575, #3588); the gaps below remain

`yq -p json -o yaml` writes every string and key in go-yaml's own style, not the JSON text's:
`{"name":"a","n":["x","1"]}` is `name: a` / `n:` / `- x` / `- "1"`, where this route used to echo
JSON's double quotes on every key and string (`"name": "a"`). A JSON string has no YAML style of
its own, so the cursor route now asks `yaml::encode_style::go_yaml_string_style`, a port of the
encoder's choice captured from yq v4.53.3 rather than read from go-yaml's source: **double**
quotes for a string that would read back as another type (`"1"`, `"true"`, `"null"`, `"0x1f"`,
`"1_000"`, `"2001-12-14"`, `"<<"`, `""`) or that holds a character the emitter will not write raw
(a tab, a control character, U+007F, the C1 block, U+FEFF, any character outside the BMP as
`\U0001F600`); **single** quotes where plain would not read back (`'- x'`, `'a: b'`, `'a #b'`,
`'#x'`, `'x '`, `'---'`); plain otherwise (`yes`, `no`, `on`, `inf`, `12:30:45` stay plain, as
they do in yq). A string that *starts* with U+FEFF has every following character escaped as
well, which yq does and nothing else explains. Checked against about 30,000 random strings,
values and keys, with no difference, and pinned by the `json_string_style_*_3575` goldens.

A string with a line break and a key with one or over 128 bytes
([#3587](https://github.com/rust-works/succinctly/issues/3587)) are closed on the cursor route and
the DOM route alike. yaml.v3 asks for the literal style for any string holding a `\n`, and the emitter
allows it unless the string ends in a space, has a space before a break, or holds a character it will
not write raw (`\r`, U+0085, DEL, U+FEFF, a character outside the BMP; a tab is fine in a block, not
in a one-line string); then it writes double quotes. A string whose only breaks are U+2028/U+2029 asks
for plain, which a break refuses, so it is single-quoted across lines (the break raw, the next
character indented) unless a space touches the break or it holds a tab. A key with a break, or
longer than 128 bytes, is `? key` with the `:` on a line of its own, and a container value starts on
that line (`: a: 1`). One definition, `yaml::go_yaml_block_scalar` / `go_yaml_multiline_scalar` /
`go_yaml_explicit_key_scalar`, serves both routes (the DOM route's copy was moved, not duplicated).
Captured from yq v4.53.3 with a differential over about 1,800 strings (every combination of up to
three of `a`, a space, `\n`, `\r`, U+0085, U+2028, a tab, `é`, an emoji, `:`, `#`, BEL, NBSP, U+FEFF)
in eight contexts (mapping value, nested, sequence item, sequence in mapping, mapping in sequence,
nested sequence, key, root), through the streaming route and a write; the pinned rows are
`json_string_style_line_break_*_3587` and `test_json_string_with_a_line_break_is_a_literal_block_3587`.

A key *written in YAML* gets the same `? key` form when its decoded value is over 128 bytes or holds a
break ([#4950](https://github.com/rust-works/succinctly/issues/4950)), in the spelling it was written in
(`? "key"`, `? 'key'`; `-P` clears it), on the streaming route and the write route. A container value with
an anchor or a tag keeps it on the `:` line and starts its first entry below. Not matched: a long key in
flow context (yq writes `{? key : 1}`), a key holding a break under `-P` (yq writes a literal block
there), and a key close to the limit once its anchor and tag count toward the 128 bytes (only the
decoded value is measured). A `-P` key holding U+0085, U+2028 or U+2029 is written raw after the `? `,
where yq keeps the escape or quotes it (the output is readable back by this tool, not by yq), and a
block-scalar or folded multi-line key is not written as a literal block on the streaming route. The
streaming route asks once per mapping whether the document has any run of 129 bytes without a line
break, any backslash or any `?` (`YamlIndex::may_have_long_key`), so a document with none never looks
at a key's length.

A *YAML source* scalar that holds U+2028 or U+2029 gets the same spelling
([#4865](https://github.com/rust-works/succinctly/issues/4865)): yq's scanner reads either as a line
break, and its emitter writes the scalar single-quoted across lines, the break followed by the
indent (written here with `\u2028`/`\u2029` for the character itself: `a: 'x\u2028y'` is `a: 'x\u2028  y'`, a flow `[x\u2029y]` is `['x\u2029    y']`, a plain one
becoming single-quoted). The streaming route used to copy the source text. The indent is the
enclosing collection's plus one step, so a flow item under a block mapping sits two steps in and a
root flow collection's items one step past that; the streaming route knows it for the first flow
level only, the DOM route (a write) at every depth. Not covered: a flow collection nested in
another on the streaming route (`a: [[x\u2028y]]`), a scalar
whose source has a space beside the break (yq trims it on decode, this keeps it and so
double-quotes), a multi-line source scalar that holds a `\n` as well, and a *block* plain scalar
with such a break, which yq rejects (`could not find expected ':'`) and the parser has always
accepted (a semi-index does not validate to that depth). The decode half is
[#4913](https://github.com/rust-works/succinctly/issues/4913).
The decode half landed with #4913 for a quoted scalar and a flow collection's plain scalar.
A mapping key that holds U+2028 or U+2029 is an explicit key on the streaming route as on the write route (`? 'x\u2028  y'`, the quotes of a double-quoted one kept).
`keys` over one writes the list item as yq does (`- 'x\u2028  y'`).
Not matched: `-P` drops the quotes of an anchored one (`? &k x\u2028y`), and `keys | . style="flow"` over one (yq `['x\u2028    y']`, a block list here).
U+0085 folds like a `\n` there (`'x\u0085y'` is `x y`).
U+2028 and U+2029 stay in the value.
The blanks beside any of the three are trimmed as beside a `\n`, so the space-beside-the-break case in the list above is fixed.
A backslash before any of the three continues the line in a double-quoted scalar, as it does before a `\n`.
A quoted implicit key that spans such a break is accepted for the same reason as a block plain scalar with one.

A *block* scalar (`|` or `>`) that holds any of the three is read as go-yaml reads it
([#6369](https://github.com/rust-works/succinctly/issues/6369)).
Each of the three ends a line as `\n` does, so the spaces after one are indentation, a line left of the indent after one ends the scalar (in the middle of what a `\n`-only reader sees as one physical line), and a key, a sequence item, a comment or a document marker there is read as structure.
In the value U+0085 is a `\n` and U+2028 and U+2029 stay as they are, and none of the three folds in a `>` scalar.
A tab left of the indent after one is refused (#6287).
The parser notes one on the line walk it already makes, looking at a byte only where a `0xC2` or `0xE2` lead byte shows up, and lists the scalar's header offset in the index (`YamlIndex::block_scalar_holds_unicode_break`).
Only a listed scalar is read line by line, with `src/yaml/block_unicode.rs`, so the decoders never scan for the three and an index rebuilt from its parts (`from_parts`) reads every block scalar the old way.
A scalar whose dedent line is spaces and one of the three is listed too, because go-yaml reads that line as an empty line and the scalar runs on past it.
Checked against yq v4.53.3 on 960 documents (four headers, the three breaks, 0-3 spaces on each side, five tails) and on 40,000 random ones (seventeen headers, with properties, `+`, explicit indents, sequence items and a nested key), with no difference, as long as no break follows a line left of the scalar's indent: those are the shapes below.
Still different, and not block scalars: a plain scalar or key in block context that spans one of the three (`x<LS>b: 1` is `could not find expected ':'` in yq), a comment line, which ends at the break in yq, and the first blank line of a document-root `--- |` when it is spaces and one of the three ([#6820](https://github.com/rust-works/succinctly/issues/6820)).
A block scalar header's own line ends at the three too ([#6820](https://github.com/rust-works/succinctly/issues/6820)): `a: |<NEL>  x` is `"x\n"` as in yq, and so are `a: |<LS>  x` and `a: | # c<PS>  x`.
The parser walks the header line to the first break of any of the five spellings and lists the scalar like one that holds a break in its body, so the decoders end the header at the same byte; a header that ends at `\n` or `\r` costs the one byte it always did.
Also still different ([#6820](https://github.com/rust-works/succinctly/issues/6820)): a comment on a dedent line that begins with one of the three runs to the next `\n` where yq's ends at the break (an invalid document such as that is accepted), and a last content line of the form `x:` followed by a dedented `- z` drops the item without an error.
A document-root `--- |<break>  x` (or `>`, or `|+`) whose text ends in a single `\n` is `"x"` in yq, the final break lost, and `"x\n"` here, as it is for the same scalar after `a:` or without the `---` ([#6820](https://github.com/rust-works/succinctly/issues/6820)): the cause in yq is not traced, and no row pins it.
A document-root block scalar with a comment at column 0 straight after its header and deeper text after that (`|` / `#x` / `  y`, `&r |` / `#x` / `  y`, and the same under a second document's `--- |`) is refused by yq and read as an empty scalar here ([#6843](https://github.com/rust-works/succinctly/issues/6843)).
Only a first document's own `--- |` line reads past the comment in yq (`--- |` / `#x` / `  y` is `"y\n"`), as it does here, and `--- |` / `#x` / `#z` / `  y` and `--- |` / `#x` / `y` are refused there where this reads a scalar.
A compact mapping after `? ` or `: ` whose key has node properties before it (`? &a k:` or `? !!str k:`, then a zero-indented `- 1`) is still refused here (`a sequence cannot follow a completed mapping entry`) where yq reads it ([#6847](https://github.com/rust-works/succinctly/issues/6847)).
A block sequence item whose header is followed by a break of the three and a key on the same physical line (`- |<LS>b: 1`, `- |<LS>  x: 1`) is read as a mapping entry whose key is empty, where yq reads a scalar or refuses the document: `looks_like_mapping_entry` looks for `: ` to the next `\n`, as a plain scalar spanning a break does above.
A scalar that holds one of the three is read about twice as slowly as one that does not: a 10 MB document of 14,240 block scalars that each hold an LS indexed in 28.0 ms against 14.6 ms before, on an AMD Ryzen 9 7950X, where yq's answer for all of them was previously wrong.

**One deliberate divergence (ADR-0018, output the reference cannot read back).** go-yaml tests only the
first character of the string for the indentation indicator, so a string that starts with a break and
then a space (`"\n a"`) is written `|-` / blank / `   a`, and reads back as `"\na"` even in yq. Ours
writes `|2-`, as the source-block path does (#836).

Still open, each separate from this fix:

- **A string with a `\n` that ends in U+2028/U+2029 or has one in its last line** (`"a\n\u2028"`, `"\na\u2028"`). yq writes a literal block with the break raw and no indentation after it (output it reads back differently); this route double-quotes it with `\L`. 16 of the 1,844 strings in the differential above, all with a U+2028 beside the end.
- **An explicit key with a container value inside a sequence item at `-I` other than 2** (`[{"a\nb":{"c":1}}]` at `-I4`). yq puts the value's first key one column after the `:` there (`: c: 1`), where it pads to the indent step everywhere else (`:   c: 1`, which this route matches); ours pads to the step in both.
- **A JSON object key `<<`** ([#3600](https://github.com/rust-works/succinctly/issues/3600)). A JSON
  document has no merge key: `"<<"` is an ordinary string key there, which yq neither expands nor hides
  from `keys`, `to_entries` or `length`, and writes quoted (`"<<": 1`) in YAML. `succinctly yq` resolved it
  as a YAML merge key whatever the source, so a write, `del`, `sort_keys` or `-P` dropped the entry and
  `.a.y` read a merged-in member; a JSON-sourced index (`YamlIndex::mark_json_sourced`) now lists its
  mappings as written. Pinned by `tests/yq_cli/i3600.rs` over the whole sweep. **Still different, on
  purpose (rule 4(b), matching would lose data):** yq's *JSON* encoder drops every mapping entry whose key
  is `<<` -- from JSON input and from a constructed object alike, with a scalar or a mapping value
  (`{"<<":1,"a":2}` is `{"a":2}`, `-n '{"a":1} | .["<<"] = 5'` is `{"a":1}`) -- so a key and its value
  vanish from the output. `succinctly yq` keeps the entry in JSON output (`{"<<":1,"a":2}`) and in the
  strings the JSON encoders build (`to_json`, `@json`). The YAML
  writer's quoting of a `<<` string is right for a JSON-sourced input; for a YAML input it stays an
  approximation (a computed `"<<"` is told from a plain `<<` of the document by its text only, so it
  prints bare, `- <<`, where yq prints `- "<<"`).

The DOM route ([#3588](https://github.com/rust-works/succinctly/issues/3588)) is closed. `-P`, every write
or reshaping filter (`del(.a)`, `.a = "1"`, `sort_keys(.)`, `with_entries(.)`, `style="flow"`) and a
string computed by a filter used their own quoting heuristics (`yaml_quote_string` and
`yaml_quote_key` in `yq_runner.rs`, `needs_yaml_quoting` in `jq/stream.rs`), which agreed with yq on
132 and 133 of 223 values and wrote a date or a number-shaped key bare (`d: 2001-12-14` and
`1: "2"` read back as a timestamp and an integer key). A string with no style to preserve now takes
`go_yaml_string_style` there too, in block or flow context (flow quotes `,[]{}?` and any `:`:
`{v: ['a,b']}`), and the heuristics only remain for a string with a line break. Checked against
about 18,000 random strings, values and keys, through `-P`, a write, `del`, `sort_keys`,
`map_values`, `with_entries` and `to_entries | map(.key)`, with no difference, plus the 215-string
battery in flow style.

The one place the rule is guarded is a YAML *document*, where the DOM holds a key or a plain scalar
only as text with no record of whether it was a string. A spelling yq reads as another type is most
likely a plain scalar of that type, and quoting it would change its type on any write, so
`yaml::go_yaml_dom_scalar` echoes it: a key (`200: ok`, `~: v`, `2001-12-14T01:02:03Z: a`, `-1: b`),
and a value the YAML 1.2 core schema calls a string but yq types (`2001-12-14`, `1_000`, `0b11`).
A value the core schema types too (`true`, `1`, `null`) can only be a quoted or computed string,
and is quoted. A JSON key or value is always a string and takes the encoder's choice. The empty key
of a YAML document keeps its old `''`. Pinned by the `json_string_style_dom_*_3588` and
`json_string_style_flow_*_3588` goldens and the `yaml_typed_{keys,values}_dom_*_3588` ones.

A YAML key's quote style on the DOM route ([#3601](https://github.com/rust-works/succinctly/issues/3601))
is closed. The DOM held a key only as its display text, so any write turned `"a": 1` into `a: 1` and
`"1": 3` into the integer key `1: 3`. A key the source quoted now records `single` or `double` in the
`CommentTree`'s per-key map (`KeyMeta`, beside the key's trailing comment, so the tree is no wider), and
`yaml_quote_key` writes it as it was written and knows it is a string. It survives a write elsewhere,
`del`, `sort_keys` and a write to the key's own value, and is dropped with a closed-literal `=` that
replaces the mapping. `-P` strips the quoting but keeps that the key is a string (`"1": 3` stays quoted,
`"a": 1` becomes `a: 1`). Pinned by the `yaml_key_style_*_3601` goldens. What is still open, each
separate from this:

- **Reshaping provenance** ([#3615](https://github.com/rust-works/succinctly/issues/3615)) now
  preserves key/value quoting for `with_entries(.)`, `to_entries | from_entries`, `keys`, and plain
  mapping `+`/`*`, including static navigation, pure entry renames/updates, and `map` over entries.
  The YAML runner carries a bounded side-tree and accepts it only when its traced value matches
  the authoritative evaluator result. Mapping `+` keeps an existing left key's quoting and takes
  the right value's style; `*` keeps the left scalar style at existing keys. New fields take the
  right key/value style. The streaming `keys` writer also retains each source key's quoting.
  Pinned by `yaml_reshape_style_*_3615` goldens. A construction of plain reads
  ([#865](https://github.com/rust-works/succinctly/issues/865)) is replayed from the nodes it names:
  `{"a": .a, "b": .b}` and `[.a, .b]` (any nesting, with literals, `length` of a read and
  arithmetic on reads among the entries, and behind a path: `.c | [.]`) keep each node's quote
  style, flow style, trailing comment and anchor, a computed entry takes what its left operand
  had, and the constructed container has none, as in yq. The cost is that of the nodes read. yq's
  own placement is followed: a standalone comment above a node stays with its key, and a non-empty
  flow *mapping* is a block once it is not the first entry of a constructed mapping (so is every
  mapping nested in it; sequences keep their style). Still different: the comment lines after a
  sequence element read by index (`{"y": .[1]}` over `- a` / `# f` / `- c` / `# g`: yq prints `# g`, a
  blank line and `# f` after the entry), the blank line yq keeps between a document's leading comment
  and its first entry when the document is wrapped (`[.]`), and a comment inside a flow mapping that
  becomes a block, which yq moves. Context-sensitive or multi-output entry
  filters (`(.a, .b)`, `[.[]]`), any other computed entry, a computed or repeated key, a
  construction a later stage reshapes (`{"a": .a} | .b = 1`) or that is wrapped in a `def`, merge
  flag variants, and typed/duplicate-key reshapes that cannot be represented faithfully by the
  text-keyed side-tree still fall back to metadata-free output. This is a scoped
  provenance fix, not evaluator-wide node identity.
- **`|=` with a constructed mapping** (`.m |= {"a": 7}`) matches pristine metadata by key, so the key
  keeps its quoting where yq's fresh node has none. A value's quoting and comment are carried the same way
  (`a: "x" # cm`), so this is the existing key-matching approximation (#870), not a new one.
- **Two keys with the same text** (`y: 1` and `"y": 4`). The DOM is keyed by text, so they collapse to one
  entry; yq keeps both.

`-P` and the quoting of a string ([#3614](https://github.com/rust-works/succinctly/issues/3614)) is
closed. yq's `-P` strips a quoted string to plain unless its encoder would misread it, and the
DOM writer got two cases wrong: a YAML 1.1 bool spelling (`"yes"`, `'on'`, `"nO"`: `y`, `yes`, `n`,
`no`, `on`, `off`, in any case) keeps its source quote style under yq and was written bare here,
and a quoted string spelled like a timestamp, `1_000` or `0b11` was written bare (so it read back
as another type) where yq writes it double-quoted. The strip pass now walks the value beside the
tree: a quoted bool spelling, key or value, keeps its style, and every other quoted string is
remembered as a string (`VALUE_STYLE_STRING`, `KEY_STYLE_STRING`), written as go-yaml writes any
string. Plain scalars are untouched (`off: 8` stays plain). A node a write *copies* from elsewhere (`.c = .a`, `.l[1] = .l[0]`) has no tree entry of its own, so it loses its quoting with or without `-P`, and `-P` drops a core tag that only restates a type (`a: !!str "yes"` becomes `a: "yes"`, as in yq). Pinned by the
`yaml_pretty_old_bool_*_3614` and `yaml_pretty_quoted_typed_strings_3614` goldens; a randomized
sweep of 720 documents over the spellings, block and flow, matches yq.

The fourth gap first listed here, a JSON surrogate-pair escape
(`"\ud83d\ude00"`), used to fail with `invalid escape sequence` on every route and is closed: it is
now decoded as Go's `encoding/json` does ([#3589](https://github.com/rust-works/succinctly/issues/3589)): a
high half and an immediately following low half are one character, any other surrogate escape is
U+FFFD. It applies to a JSON-sourced scalar only, so a YAML document's surrogate escape stays an
error, as in go-yaml. Pinned by the `json_surrogate_escape_*_3589` goldens, which include the DOM
routes (`-P` and a write). `succinctly yaml validate` still accepts such an escape in a YAML
document, where go-yaml rejects it.

### The `---` between results is per-run, not per-node as real yq's printer makes it (#2427)

Real yq's printer decides the `---` separator **per output node**, from two fields the node
carries — `document` (its index within its own file) and `fileIndex`
([`printer.go`](https://github.com/mikefarah/yq/blob/v4.53.3/pkg/yqlib/printer.go), the
`p.previousDocIndex != mappedDoc.GetDocument() || p.previousFileIndex != mappedDoc.GetFileIndex()`
test). Two things follow that succinctly does not reproduce:

1. **`previousFileIndex` is set once and never updated inside the loop** (only
   `p.previousDocIndex` is assigned at the bottom of the iteration), so every node whose
   file differs from the *first printed node's* file gets a separator, however many have
   already been printed from that same file.
2. **Whether a node has a file/document at all is a per-operator property.**
   `CandidateNode.Copy` carries `document`/`fileIndex` across, but `CreateReplacement`
   builds a fresh node and starts both at `0`
   ([`candidate_node.go`](https://github.com/mikefarah/yq/blob/v4.53.3/pkg/yqlib/candidate_node.go)),
   and the choice between the two is made independently by each operator. So `.`, `.name`,
   `select`, `del`, `=`, `{...}` and `+` all keep it, while `keys`, `length`, `type`,
   `tostring`, `to_entries`, `map`, `[...]`, `has`, `path`, `key`, `parent`, `line`,
   `column`, `filename`, `file_index`, `document_index` and every literal do not.

succinctly's values carry no such field.
It writes `---` before each document's first result when the output is YAML, which agrees with real yq for every filter in group (2)'s first list and for one-document-per-file input.
For a filter every one of whose results yq builds with no document it writes none (#7019): `yq_result_document.rs` reads that from the filter, stage by stage, from yq's rule for each operator (a navigation step keeps its input's document, a replacement of a document's root and a literal have none, `{...}` keeps its input's).
That covers literals, `[...]`, `length`, `has`, `contains`, `tostring`, `to_json`, `to_yaml`, `@json`, `@yaml`, `@props`, `to_entries`, `not`, `key`, `anchor`, `tag`, `type`, `style`, `kind`, `line`, `column`, `path`, `file_index`, `document_index` and `filename` over a document's root; `keys`, `with_entries` and interpolation over any input, which build a new node; `{}`, which yq reads as a literal; a comparison, `and` or `or` whose left operand always yields one result of that kind (yq builds the boolean from the left operand); arithmetic whose left operand is a literal other than `null`; `//` whose left operand is a literal (the left one when it is truthy, else the right one); a `$name` bound to one of those; and any pipe whose last such stage builds one over whatever it is handed (`del(.a) | "x"`).
`.a | length` keeps `.a`'s document, as in yq.
The run's `---` lines go and a document's own leading `---` stays, as in yq.
`-i` keeps the separators, a divergence (ADR-0018: matching would corrupt the file): without them yq rewrites a file of several documents as one, so `yq -i length` over three documents writes `3`, `3`, `3`, which reads back as the one string `3 3 3`, and `with_entries(.)` joins their keys into one mapping.
Live-verified against v4.53.3 on `f1.yaml` (`a: 1`, `name: first`) and `f2.yaml` (`b: 2`, `name: second`):

```bash
$ yq '.'      f1.yaml f2.yaml    # a: 1 / name: first / --- / b: 2 / name: second   (agrees)
$ yq 'length' f1.yaml f2.yaml    # 2 / 2                                             (agrees)
$ yq '.name, .name' f1.yaml f2.yaml   # first / first / --- / second / --- / second
$ succinctly yq '.name, .name' f1.yaml f2.yaml     # first / first / --- / second / second
```

Every other filter keeps the separator before each document's first result.
Where yq's index differs between the results of one filter, or depends on the values, that differs:

- `map` over a document's node copies an empty input (which keeps the document) and builds a fresh sequence from any other, so `.l | map(.)` over `l: [1, 2]` / `---` / `l: [3]` / `---` / `l: []` is `[1, 2]` / `[3]` / `---` / `[]` in yq, and here every result is separated.
- A comma whose branches differ (`.a, "x"`) or `..` (whose first result is the root) separates within a document in yq (`2` / `---` / `x` in the second document), and here only before a document's first result.
- `//` whose left operand is not a literal is that operand where it is truthy and its right one otherwise, so `.z // "x"` with no `.z` has no document in yq and is separated here; arithmetic whose left operand is null copies its right one (`.z + 1`), and a comparison whose left operand yields nothing builds from its right one, the same way.
- Any stage outside the list above (`del`, `sort_keys`, `select` after a construction it cannot see through, `$x`, `reduce`, ...) keeps the separator, whatever yq builds there.
- yq never updates the file index it compares against (point 1), so every result from a file other than the first printed result's is separated, both of `.name, .name`'s second-file results above included; here only a document's first result is.

Closing those needs per-node document provenance threaded from the evaluator to the printer, tracked on [#2427](https://github.com/rust-works/succinctly/issues/2427).

### `filename` and `file_index` are read from the evaluation context, not from the node (#2648)

Real yq stamps `filename` and `fileIndex` on every node as it decodes a file, and which operators
keep that stamp is the same per-operator table the `---` section above describes: `.`, `.name`,
`select`, `=` and `+` keep it, while `[...]`, `map`, `to_entries`, `tostring` and every literal
start a fresh node whose `filename` is `""`. succinctly's values carry no such field, so
`filename` answers the file the evaluation is currently in — the per-file scope the CLI installs
around each file (and the origin each `--eval-all` document carries). That agrees with real yq
for every shape that stays on the file's own nodes, including arithmetic and `tostring` results,
which real yq also gives the file's name (live-verified against v4.53.3), and diverges where yq
builds a node it does not stamp:

```bash
$ yq '1 as $v | $v | filename' f1.yaml             # (empty)
$ succinctly yq '1 as $v | $v | filename' f1.yaml  # f1.yaml
$ yq '[.x] | .[0] | filename' f1.yaml              # (empty)
$ succinctly yq '[.x] | .[0] | filename' f1.yaml   # f1.yaml
```

A `load()`ed document is the same gap from the other side: yq stamps the *loaded* file's name on
its nodes (`yq -n '[load("f1.yaml") | filename]'` is `f1.yaml`, and so is `yq '[load("f2.yaml") |
filename]' f1.yaml`), where succinctly names the file being read, or `""` under `-n`.

`file_index` has the same shape (`yq ea '1 | fileIndex'` is `0` for every file, succinctly's is the
file's own). Closing it needs the per-node provenance the `---` section names, tracked on
[#2427](https://github.com/rust-works/succinctly/issues/2427).
A metadata assignment (`.x
line_comment = filename`) is refused with the `document_index/file_index/filename` error rather
than resolved once against the first document, so a multi-file run never stamps every file with
the first one's name — yq answers each file's own name there.

`document_index` follows the same rule since #6950, for a node that has no cursor of its own.
It answers the document the evaluation is in.
That is what a node a write produced keeps in yq (`.x = 5 | document_index` is `0` then `1` over a two-document file), and so does an absent key (`.zz | document_index`).
It is not what a node the filter builds has: `[.x] | document_index`, `"s" | document_index`, `to_entries | .[0] | document_index` and `with_entries(.) | document_index` are `0` in yq, and the number of the document here.
A `load()`ed document is the same gap from the other side: `load("f2.yaml") | document_index` is `0` in yq, and the number of the document being read here.
A node a cursor still places keeps its own number.

### `. as $x | [$x]` under `--eval-all` collects one copy per document, not one per binding (#2427)

`--eval-all` follows real yq's `EvaluateTogether` rules for `[E]` and for binary operators
(see [the context-list section](#yqs-context-list-evaluation-model-in-a-single-document-2451)), but not for
variable binding. `operator_variables.go` consults the same flag, and the observable effect
is that the *body* of an `as` binding keeps seeing the whole document list even though the
binding itself is per document — so a `[...]` in the body collects one entry per document
in context rather than one per binding. Live against v4.53.3, `f1.yaml`/`f2.yaml` as above:

```bash
$ yq ea -o=json -I0 '. as $x | [$x]' f1.yaml f2.yaml
[{"a":1,"name":"first"},{"a":1,"name":"first"}]
[{"b":2,"name":"second"},{"b":2,"name":"second"}]
$ succinctly yq --eval-all -o=json -I0 '. as $x | [$x]' f1.yaml f2.yaml
[{"a":1,"name":"first"}]
[{"b":2,"name":"second"}]
```

`. as $d | $d` (no collect in the body) agrees, as does every other row of #2427's own
capture table.

### An arithmetic result keeps its left operand's origin under `--eval-all` ([#6969](https://github.com/rust-works/succinctly/issues/6969))

A result of `+`, `-`, `*`, `/` or `%` is read from where its left operand was, as in yq: `yq ea '. * {"z":1} | document_index' md.yaml` over two documents is `0 0 1 1`, and `file_index` and `filename` follow the same way.
Each result had no origin, so they were all `0` (and `""`).
A constructed left operand takes the origin of the context entry it was built for (`{"z":1} * .` is `0 0 1 1` too), and a number built from a read keeps it (`.x + 1`).
A left operand that builds a fresh node, a literal or a builtin such as `length`, has none: `10 + .x | document_index` is `0` for every document in yq and here.
A comparison's boolean has none here, as before; yq keeps the left read's for `.x == 1` (`0 0 1 1`) and gives `. == .` none ([#6973](https://github.com/rust-works/succinctly/issues/6973)).

Not covered here ([#6973](https://github.com/rust-works/succinctly/issues/6973)):
- A map merge or concat keeps `EvaluateTogether` in yq (it copies its left operand), so `. * {"z":1} | [.]` is one array of every result there and one array per result here.
  A scalar result is a fresh node in both.
- A later arithmetic stage crosses the whole list of results in yq: `(. * {"z":1}) | . + {"w":2}` is 16 results over two documents and 4 here.
- A union as the left operand that mixes reads and literals (`(.x, 7) + (10,20)`): yq gives each node its own origin, and this decides it per operand, so the reads lose theirs (as before #6969).
- `line` of a computed result is `0` here; yq keeps its left operand's line.
- `. as $d | $d * {"z":1}` is four results in yq and two here.

### `SRC as $x ireduce (INIT; UPDATE)` and `{}` under `--eval-all` read the whole document list ([#2427](https://github.com/rust-works/succinctly/issues/2427))

`--eval-all` folds every document of every file, as yq's `reduceOperator` does: it reads SRC and INIT against the whole list, then folds each source node into each accumulator.
`yq ea '. as $item ireduce ({}; . * $item)' a.yaml b.yaml` is the two files merged, which is the recipe yq's own documentation gives for merging files.
`succinctly yq` answers it the same, and `. as $i ireduce (0; . + 1)` is the document count once per document, as in yq (`0` is one literal per context node, each its own accumulator).
An empty `{}` is one mapping whatever the context holds (`yq ea '{}' f1 f2` prints one `{}`, where `0` prints one per document, and `yq '.x[] | {}'` is one `{}` on any route), so `ireduce ({}; ...)` starts one fold.
The reads of the loop variable's node (`$i | document_index`, `file_index`, `path`, `line`, `tag`, `kind`) answer from the document the node came from.
Both rules apply to a list whose entries all came straight from the reader, like `[...]` and the binary operators.
After a navigation they do not.
On every route a yq `ireduce` folds a list of accumulators, as yq's does ([#6802](https://github.com/rust-works/succinctly/issues/6802)): each output of UPDATE is an accumulator of the next step, a union in UPDATE is branch-major over the list, an UPDATE with no output drops its accumulator, and several INIT outputs all fold the one source.
`. as $i ireduce (0; (. + $i, . * 10))` over `[1, 2]` is `3 2 10 0`, and `ireduce (0; select(false))` prints nothing.
jq's `reduce` is unchanged: one fork per INIT output, the last UPDATE output wins.
The same rule holds for `reduce` under `--jq-extensions` in yq mode, since the mode decides, for a single pattern (a `?//` alternative list keeps jq's fold).
An absent key read through the loop variable in an arithmetic operand is no output, as in yq: `. as $i ireduce ([]; . + [$i.a])` over a document with no `a` is `[]` (the right operand of `+` is read-only, and the fold's shortcut for `. + <closed expression>` now leaves a read that finds no key to the evaluator).
The same read as a whole UPDATE (`ireduce ([]; [$i.a])`) or as the value of a write (`.x = $i.a`) is a `null` node in both.
A list that grows past 100,000 accumulators is refused with `reduce: maximum iterations exceeded`, where yq grows it until it runs out of memory (`ireduce (0; (. + 1, . + 2))` over 30 elements is 2^30 accumulators), as ADR-0018 allows for a run that would take the host down.

Five things still differ, each in yq v4.53.3 against `succinctly yq`:

- **`{}` over a computed or a written source.**
  The one-`{}` rule holds where the collect reads the document (`.x | [.[] | {}]`, `.x | map({})`) and not where its source is a computed array or a value being written.
  `.x | map(. + 1) | [.[] | {}]` is `[{}]` in yq and `[{},{},{}]` here, and so is `.x | map(. + 1) | map({})`.
  `.x |= map({})` and `.x = [.x[] | {}]` write `[{}]` in yq and three `{}` here.
  The owned-value evaluator never enters the context-list walk for a collect, where the document route does.
  `.x | map({})` over a flow sequence prints `[{}]` in YAML in yq (the result keeps its source's flow style) and `- {}` here ([#6801](https://github.com/rust-works/succinctly/issues/6801)).
- **A constant over an empty context.**
  yq answers a literal from a context that holds no node (`yq ea 'select(fi == 9) | 0'` is `0`, and `select(fi == 9) | . as $i ireduce (0; . + 1)` is `0`).
  A literal, a constructor and an operator over literals do here since [#6943](https://github.com/rust-works/succinctly/issues/6943), in a pipe and in `--eval-all`'s list (see "A pipe's empty context list" below).
  A fold whose INIT is another constant and whose source filter selects no document is `0` in both.
- **A read of an absent position.**
  `.b as $i ireduce ([]; . + [$i | key])` reads `"b"` for every document in yq, where a document without `b` is a `null` node that still has a place.
  Here that `null` has no position, so `key` and `parent` of it answer nothing and `path` answers `[]`.
- **A union of writes over a container accumulator.**
  yq's assignments mutate the accumulator in place and hand back the same node, so the list keeps it once: `.n[] as $i ireduce ({}; (.a += 1, .a += 2))` over `n: [1, 2, 3]` is `{"a":9}` and `ireduce ({}; (.a = 1, .b = 2))` is `{"a":1,"b":2}`.
  Here each branch writes its own copy, so the list holds one accumulator per combination (eight outputs after three elements).
  It is the same pointer rule as the "A union of two assignments" entry in the duplicate-keys section below, in a plain pipe ([#6802](https://github.com/rust-works/succinctly/issues/6802)).
- **An UPDATE that answers the loop variable itself.**
  `. as $i ireduce (null; $i)` is the last document, once, with its own `file_index` and `document_index` in yq, because the accumulators it produces are the same node and yq's list keeps it once.
  Here it is one copy per accumulator, and each has no origin (`file_index` is `0`).

An accumulator the fold computed carries no document origin: `. as $i ireduce ({}; . * $i) | file_index` is `0`, as in yq.

### A `$name` stage after a fan-out answers its value once ([#6821](https://github.com/rust-works/succinctly/issues/6821))

yq's `GET_VARIABLE` returns the list the variable is stored in, and a pipe evaluates its right side once over the whole list its left side produced.
So `$name` as a pipe stage after a fan-out is one node however many reached it, and even when none did, and every stage after it sees that one node.
`.n as $v | .x[] | $v` is one `7`, `.x | map($v)` is `[7]`, `.x[] | select(. > 5) | $v` is one `7` from an empty context, and `.x[] | $v | . + 1` is one `8`.
`succinctly yq` answered one per element.
It follows yq now, in yq mode only: jq reads `$v` once per input, as `.x[] | $v` is three values there.
The rewrite is made by the parser, where the stage is still a variable: the stages before it run to the end for their errors and answer `null` once, and the variable follows (`reduce PREFIX as $_ (null; null) | $v`).
52 rows captured from yq v4.53.3 match, including `map` over a sequence, a mapping, a scalar, `null` and an empty collection, and `path`, `key`, `parent` and `tag` after the variable, which used to answer once per element.
A `$name` that is not a stage is read per node in yq too and is unchanged: `[$v]`, `{"k": $v}`, `. + $v`, `$v + .`, and the body of an `as` (`.x[] | . as $w | $v`).
Rows over a computed source (`.x | map(. + 1) | map($v)`), the right side of a write (`.x |= map($v)`, `.x = [.x[] | $v]`) and `--eval-all` (`.x as $v | .x | $v` over two files) match yq too.
Four things differ.
- **A variable as a branch of a union.**
  `.x[] | (5, $v)` is `5 5 5 7` in yq and `5 5 5 7 7 7` here: the walk that evaluates a union over the list reads a variable branch per node.
- **A prefix that answers one node.**
  `.x | $v` and `. | $v` are left alone, since one node reaches the variable either way, which keeps the bound node's `line` and the path context of `del(. | $y)`.
  So is a write (`.a as $y | $y.b = 5 | $y`), which answers its input once and rebinds `$y` for the stages after it ([#2643](https://github.com/rust-works/succinctly/issues/2643)).
- **The bound node's position.**
  `.n as $v | .x[] | $v | line` is `2` (and `column` is `4`) in yq and `0` here: the stage that runs the prefix builds a new value, so the pipe leaves the route that resolves a bound node's position from the document.
  Before this change the same filter answered `2` three times, the right value for the wrong count.
  An `if` that passes `.` on keeps the position, and leaves the path register at the root, so a write through it (`(. as $x | any | $x) = 5`) replaces the whole document, which the #3758 pins refuse.
  The node's presentation goes the same way: `.m as $v | .x[] | $v` over `m: {a: 1}` prints `a: 1` here where yq prints `{a: 1}`, with the same data.
  A prefix of single-valued navigation is not rewritten, so `.n as $v | .x | $v | line` is `2` and its flow style is kept.
- **A path expression.**
  The argument of `path(f)` (a `--jq-extensions` form, yq's `path` being nullary) keeps its per-node reading, as it had no oracle.
  `del(.x[] | $v)` is `Invalid path expression` here, as before this change, and a no-op in yq ([#2643](https://github.com/rust-works/succinctly/issues/2643)).
  The left side of a write (`(.x[] | $v) = 5`) is a no-op in both.

### Duplicate mapping keys — an owned value keeps every entry (#1344); a few narrower gaps remain

**Since [#1344](https://github.com/rust-works/succinctly/issues/1344) ([ADR-0027](../../adrs/adr-0027.md)) an `OwnedValue::Object` keeps a repeated mapping key's earlier entries beside its collapsed `IndexMap`.**
`succinctly yq` records them where it materializes a document (the owned input of `-s`/`--eval-all`, the value a `[...]`, a binding, `..`, a write or a comma holds) and every writer prints all of them.
`succinctly jq` never records one (a JSON document with a repeated key is the last value to jq), so jq mode is unchanged.
Navigation, `.[]`, `map_values` and every write through a path act on the last entry, as yq does; `del` removes every entry; `+` overwrites the first; `to_entries`, `with_entries`, `from_entries`, `omit` and the writers see all of them.
The 68 oracle rows in `tests/yq_cli/i1344.rs` pin it against yq v4.53.3, on a plain run and under `--eval-all`.
What is still different, each a place that has not been taught the entries (an earlier entry survives until something copies the map through a raw `IndexMap` borrow, which keeps only the last):

- **`sort_keys`** stable-sorts every entry of a repeated key (`a: 1 / a: 2 / b: 3` stays `{"a":1,"a":2,"b":3}`).
  yq copies the last value over the earlier entries (`{"a":2,"a":2,"b":3}`), which would destroy a document value, so this is a recorded divergence (ADR-0018, ADR-0027 P4, #5426).
- **A comment, anchor, tag or quoting on an earlier entry** is not carried on the routes that hold a `CommentTree`, which is keyed by key and describes the last entry: the earlier entry is written plain.
  An alias to an anchor declared on an earlier entry is written as that entry's value, since the anchor is not.
  One thing of an earlier entry is carried (#6335): how its value was spelled, in the key's `KeyMeta` (`a:` as the empty value, `~`, `Null`, `True`).
  A write (`=`, `|=`, `+=`), `del`, `*`, `with_entries(.)` and a `to_entries | from_entries` with nothing between the two print `a:` / `a: 2` as yq does, where they printed `a: null`.
  The spelling is kept by position among the key's earlier entries, and only while the tree holds as many as the mapping does; a key whose count changed prints plain.
  Still plain, each printing `a: null` for `a:` (and `null`/`true` for `~`/`True`): `pick`, which answers the earliest entry of a key (`a:` in yq), and every route that rebuilds a mapping from its parts without the key's entry.
  Those are a filter between `to_entries` and `from_entries` (`reverse`, `sort_by`, `del(.[0])`, a slice), a combination of merge flags (`*+d`) and `*c`, and `. as $x | .y = $x`; all of them lose a spelling with or without a repeated key.
  A `*` with one merge flag (`*+`, `*d`, `*n` or `*?`) keeps the left document's comments, flow style and key spellings, as yq does (#6900); an array the merge replaces keeps the left node's own style around the right items.
  `map(select(c))` and `with_entries(select(c))` keep each surviving item as written since #6674, for a test `c` of field reads, literals, comparisons and `and`/`or`/`not`; another test, a test that fails on one item (`.value.c` over a scalar entry), a filter other than `select` in the same position, `select` over a sequence written in flow style at the document root (`[1, 2]` prints as a block here), a `# header` comment before a `---`, and `with_entries` over a sequence (whose keys are integers) still take the plain route.
  yq keeps a document's head comment on its first key: `with_entries` keeps it whatever its filter drops, and the written-out `to_entries | map(select(...)) | from_entries` drops it with the first entry, as yq does.
  The entries operators over the whole document (`to_entries`, `from_entries`, `with_entries(f)`, and `to_entries | from_entries` with nothing between the two) give their result what yq reads before the document: the comments above it, an explicit `---` and the root's head comment (#7081), as yq's `CreateReplacementWithComments` does, where `keys`, `length`, `pick` and `[.]` build theirs without.
  The carry is for the document as the filter's first stage: a step into it (`.a | to_entries`), a `map` between the two operators and an alias-sensitive write are not it.
  What is carried is the comment block above the first `---` (or above the document where there is none), blank lines between them included: 2,485 random inputs of that shape, over five operators and five flag sets, print as yq prints them.
  A comment after the `---` is the root's head comment in yq and the first entry's here, so it prints inside the first entry; one indented by blanks (`   # c`) is printed as written by yq and without the blanks here; and a root that is `{}` or `[]` loses the comments after its `---`.
  Still different, each a separate presentation rule: a source that starts with a byte-order mark (yq makes the comments the first node's, so they print inside the first entry, #7089), a `%YAML` directive before the `---`, `--eval-all` (which drops every comment, #4380), `-o=kyaml` (the section below), and the same result through a construction (`{"x": .} | .x`, `[., .] | .[0]`) or from a scalar document (`map_values`, `omit`, `@yaml`, `to_yaml`), where yq keeps the leading content too.
  Over several documents `to_entries | from_entries` prints the `---` between them, where yq prints the first and none after it (`to_entries` and `with_entries` agree).
  An anchor a `select` drops while an alias to it stays prints the alias in yq (`b: *x`, unreadable without its anchor) and the alias's value here, the soundness rule of ADR-0017.
  An empty value inside a flow mapping (`{a: , b: 2}`) is `''` in yq and `null` here for every entry, repeated key or not.
- The other `IndexMap` builders that have not been converted.
  Object slicing was one, closed by #5426.

The sections below are the history of how each member was closed; where a bullet there says a route "collapses", this paragraph supersedes it.

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

**Three gaps #1687 deliberately did not close** were closed by #1344, which gave the owned value somewhere to keep the entries:
`group_by` (an array of arrays), `while`/`until` (their state is an `OwnedValue`) and `reduce`/`foreach`'s bindings now print `{"b":1,"a":2,"b":3}` for the element the old text below shows collapsing to `{"b":3,"a":2}`.
Pinned by `test_group_by_and_while_until_keep_a_repeated_key_1687` and `test_reduce_binding_keeps_duplicate_keys_1687`.

**And one deliberate divergence from real yq, not a gap.** The cursor-backed reordering above
is switched off entirely for a document carrying any `*alias`, via
`DocumentCursor::document_has_aliases`. Reordering can lift an alias above the anchor it
resolves to, and `reverse` on `- &x {p: 1}` / `- *x` does: real yq answers `- *x` then
`- &x {p: 1}`, then rejects its own output with `unknown anchor 'x' referenced` when asked to
read it back (both verified live against v4.53.3). `enforce_anchor_soundness` prevents
that on the DOM path, but no per-operation equivalent exists for these cursor-streaming
reordering builtins specifically — rather than building one for every stage that could
invalidate an alias, `DocumentCursor::document_has_aliases` unconditionally routes any
alias-bearing document to the DOM path for them instead, losing the marks rather than
emitting a file succinctly could not read back. The gate is on *aliases*, not anchors: an unreferenced
`&x` is valid YAML wherever it lands.

Three narrower gaps #1398's fix deliberately left alone were filed before it landed (a fourth, [#1342](https://github.com/rust-works/succinctly/issues/1342), `paths(node_filter)` collapsing a duplicate key, is closed: `paths(f)` walks the document and runs `f` on the node each path names whenever the format keeps duplicates, as `paths` does; jq mode, which collapses them anyway, keeps the owned producer):

- - [#1343](https://github.com/rust-works/succinctly/issues/1343) — `-s` with a non-identity filter and `--eval-all` route through `parse_input`/`OwnedValue` for *both* formats.
  Closed by #1344: that owned input keeps the repeated key, and `-s 'paths'`, `--eval-all '.'` and `--eval-all 'keys'` match yq.
  `-i` follows stdout since [#1349](https://github.com/rust-works/succinctly/issues/1349).
- - [#1344](https://github.com/rust-works/succinctly/issues/1344) — `del`/`with_entries`/`map_values`/`walk` (and `recurse(f)`/`recurse(f; cond)`) fell through `eval_generic.rs`'s wildcard bridge (`eval_on_owned`), which materialized via an `IndexMap`.
  Closed (see the paragraph above): `[..]`, `.., 1` and `.. as $x | $x` print the node intact, as does every other shape in this list except `contains` and `sort_keys`.
- - [#1102](https://github.com/rust-works/succinctly/issues/1102) — object slicing (`.[S:E]` on an object) must materialize into an `OwnedValue::Object` to build yq's AST-child-list view before slicing it.
  Closed by #5426: the slice reads the repeated key's every entry.

**[#2186](https://github.com/rust-works/succinctly/issues/2186) records the routes that collapse
by construction, which is a wider set than the issue's own `LazySeq` arm.**
The DOM route's result type is `ResultWithComments = (OwnedValue, CommentTree)` (`yq_runner.rs`).
It has no cursor variant, and `OwnedValue::Object` is an `IndexMap`, so whatever reaches the route has merged a repeated key before it is printed.
That includes a `LazySeq` of cursors, which is why the issue's own fix (`drain_atomic`, as on the jq side) does not transfer: `jq_runner.rs` has a cursor-carrying `JqValue` to hand each element to, and the yq route has nothing equivalent.
Real yq v4.53.3 keeps both entries on every row, run on `a: 1 / a: 2 / b: 3`; every row below printed `a: 2 / b: 3` (or `a: 2 / b: 5`, `a: 2`) before #1344 and prints yq's answer now:
`-P '.'`, `--eval-all '.'` (yq: `ea '.'`), `-P '., .'`, `'.b = 5'` on stdout or under `-i`, and `-i 'del(.b)'`.
`-P` and `--arg` send every filter down the route, so `.` itself used to collapse under them.
`--arg` has no yq twin to compare against, since yq v4.53.3 rejects the flag.
`-s` with a non-identity filter (`-s '.[0]'`) has none either, because yq's `-s` is `--split-exp`, but it takes the same route and keeps the entries now.
`[...]` around a cursor result (`[.[0]]`, [#2063](https://github.com/rust-works/succinctly/issues/2063)) keeps them too.
A top-level comma stopped being a member in [#5283](https://github.com/rust-works/succinctly/issues/5283).
When every branch is navigation (`., .b`, `.a, .zz`, `(.a, .b), .l`), stdout streams the branches one at a time through the M2 writers, so a repeated key and a node's flow style survive as they do for each branch alone.
A branch that errors during evaluation prints nothing for the others, and a document separator is written once, before the first branch that prints.
A failure found while a later branch streams (an undecodable scalar in a document yq rejects whole) comes after the earlier branches were written, as it does for `.[]` (#1615).
A branch that answers an alias prints `*a`, as `.[]` and yq do, where the DOM route's per-result soundness rule expanded it (`.x, .y` on `x: &a {p: 1} / y: {q: *a}`).
The wiring fault the issue's triage also named no longer reproduces: `-P 'map(.)'`, `-P 'sort'` and `-P 'reverse'` print `99999999999999999999` as yq does, where the triage saw `1e+20` (`the_dom_route_keeps_a_large_integers_spelling_through_a_lazy_seq_2186` pins it).

**[#2626](https://github.com/rust-works/succinctly/issues/2626) closed one member that was
missing from that list: arithmetic.** `eval_single`'s `Expr::Arithmetic`/`Expr::Negate` arms
matched only when an operand needed path context, so every other arithmetic fell to the same
wildcard bridge — and collapsed the document before either operand ran. The binary
contradicted itself:

```bash
$ printf 'b: 1\na: 2\nb: 3\n' | succinctly yq 'length'       # 3
$ printf 'b: 1\na: 2\nb: 3\n' | succinctly yq 'length + 0'   # 2  (was), 3 (now); yq: 3
$ printf 'b: 1\na: 2\nb: 3\n' | succinctly yq 'length * -1'  # -2 (was), -3 (now); yq: -3
```

Unary minus needed the same fix twice, because it has two bridging arms: `eval_single`'s and
`eval_each_generic`'s own. Fixing only the first left `-length` at `-3` while `first(-length)`
still answered `-2` — the same filter contradicting itself by position — so
`each_negate_generic` gives the lazy route a native arm too. Real yq has no unary minus, so
the oracle for those rows is its own spelling, `length * -1`.

The same bridge re-rooted the value at a fresh JSON serialization, which stubbed every yq
node-metadata builtin `needs_path_context` does not cover: `.b | (line + 0)` answered `0`
where `.b | line` answers `2`, `(anchor + "")`/`(style + "")` answered `""`, and `di + 0`
answered `0` for every document. All of those now agree with their bare spellings, and with
yq v4.53.3. Pinned by `test_arithmetic_reads_the_real_document_2626`.

Two things it does not fix. `generic_item_into_owned` still folds each *operand* to an
`OwnedValue`, so an operand whose own value is a duplicate-keyed container collapses there
(`. + {}`) — the `OwnedValue::Object` representation limit ADR-0017 declares out of scope
(#796), the same residual `reduce`'s bindings have above. (`column` inside an arithmetic used
to answer the value's column where yq answers the anchor's, a divergence in the builtin itself
rather than the bridge; [#2712](https://github.com/rust-works/succinctly/issues/2712) fixed it
and `.b | (column + 0)` is `4` on `b: &anc [1,2]`, as in yq.)

Leaving the bridge also means arithmetic stops validating the whole document, which is a
deliberate divergence in its own right and is recorded under
[jq Limitations § Arithmetic and unary minus validate only what they read](../jq/limitations.md#arithmetic-and-unary-minus-validate-only-what-they-read-2626)
— the arms are shared, so the rule is one rule.

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
therefore remained silently accepted at the time; `[1,]`/`{"a":1,}` now correctly raise,
confirmed live and CLI-reachable:

**Update (#2403)**: the "no cursor left to find its opening bracket from" premise above no
longer holds for nested containers. `container_gap_ok` needs a cursor to the container
itself, but every recursive call into `to_owned_canonicalizing_numbers_at_depth` already
resolves the child's own cursor (`field.value_cursor`, `elem_cursor`) for an unrelated
reason and previously discarded it — threading it through (via the shared `tail_gap_ok`
helper in `document.rs`, mirroring #2358's identical fix for `eval_generic::to_owned_at_depth`)
lets `container_tail_gap_ok` close `[,]`/`{,}` for every *nested* container the same way
`to_owned_cursor_at_depth` always could. Only the true top level
(`to_owned_canonicalizing_numbers`'s own depth-0 entry point, which has no cursor to give in
the first place) keeps the gap now.

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

**Former cost, closed by #1344: `--inplace`/`--slurp` with well-formed, duplicate-keyed JSON input collapsed those duplicates** (`{"a":1,"a":2}` → `{"a":2}`).
`any_input_is_json` is a blanket, content-independent gate (computed from `--input-format`/file extension alone, never from whether the document is actually malformed), so it declines the fast path for *every* JSON-sourced `--slurp`/`--inplace` invocation, not just the malformed ones this fix exists for.
Accepted deliberately: `--inplace` mutating a file on a false accept is a correctness/data-safety concern.
The fallback's owned value keeps the repeated key since #1344, so the output no longer differs between the two routes, and the duplicate key no longer proves which route ran.
The malformed-input rows of `test_yq_slurp_fast_path_rejects_trailing_comma_2276` and `test_yq_inplace_fast_path_rejects_trailing_comma_leaves_file_untouched_2276` are what guard the gate now.

**A further collateral cost, narrowed by #4394**: `any_input_is_json` is one run-wide boolean with
no per-file M2-vs-DOM switch (a per-file version was not attempted — each of
`can_inplace_fast_path`'s two branches is its own, separately-parsed loop, decided once before
either starts). Real yq decodes every file of a run with the *first* file's decoder, and so does
this, so a mixed run no longer exists: `succinctly yq -i '.' a.yaml b.json` reads both as YAML and
keeps the M2 fast path (and `a.yaml`'s duplicate keys), while `b.json a.yaml` reads both as JSON and
declines it for the whole run, which collapses a YAML file's duplicate mapping keys — the #442/#1343
loss the fast path exists to avoid — but only where the JSON file leads, and then yq fails on the
YAML file anyway. Pinned by `test_yq_inplace_mixed_yaml_json_files_keeps_yaml_duplicate_keys_2276`.

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

**Closed by [#2279](https://github.com/rust-works/succinctly/issues/2279), in the parser
rather than on either route.** Declining the fast path was indeed no help, and neither was
the obvious alternative of giving `YamlCursor` the `DocumentCursor` delimiter overrides
(`container_gap_ok`/`trailing_element_gap_ok`/`preceding_delimiter_ok`): those methods are
never called on this path at all — JSON's own validation lives in `json::light`'s
module-private walk functions, so overriding the trait for `YamlCursor` installs code nothing
calls (verified by instrumenting it). The giveaway is that `length` on `[1,]` also answered
`1`: that count comes from the BP structure, so no output-side check could have reached it.

The fix restores the invariant `preceding_delimiter_ok`'s own doc comment states — *every
format but JSON validates delimiters while parsing*. JSON-sourced YAML was the one case that
did neither. `YamlIndex::build_json_sourced` now parses with `Parser::json_strict` set and
marks the index in one step (a post-build `mark_json_sourced()` has already accepted the bad
input), so `[1,]`, `[,1]`, `[1,,2]`, `[,]` and their nested forms error on every route that
parses through `YamlIndex` — the plain stdout path this issue was filed about, for every
filter.

**Closed on the other family of routes by [#2781](https://github.com/rust-works/succinctly/issues/2781).**
A *top-level* `[,]` (only `[,]` itself — `[[,]]` and `{"a":[,]}` were already refused)
still passed on `--slurp`/`--eval-all`/`--inplace`, which materialize through `JsonIndex`'s
DOM bridge rather than this parser. The premise that the bridge's check "needs a cursor for
the outermost container it does not have" was false: `yq_runner.rs`'s `parse_input` already
holds the root cursor when it builds the initial `OwnedValue`, it simply was not threading it
down to `to_owned_canonicalizing_numbers_at_depth` — every *nested* container already got its
own cursor this same way (#2403). Passing the root cursor through closes the top-level gap
the identical way. `succinctly yq -p json -i '.'` on a file containing `[,]` used to rewrite
it to `[]`; it now raises and leaves the file untouched, matching real yq's own refusal.

Passing the root cursor also makes the DOM-bridge route's **top-level `{,}`** consistent
with its own nested rejection — before #2781 the top level accepted `{,}` (agreeing with real
yq only by construction, since the check simply could not reach it) while `{"a":{,}}` was
already refused one level down. #2781 chose consistency with the route's own nested behaviour
over preserving that accidental top-level agreement; the resulting divergence from real yq
(which does accept top-level and nested `{,}`) is not new to this fix and is recorded with
the DOM bridge's other flow-mapping delimiter gaps in #2777 below, not carved out as a
special case here.

Scope was originally flow **sequence delimiters** only, and never mapping delimiters,
because that is where real yq (v4.53.3, captured live) actually draws the line — the
asymmetry this section already describes above: tightening mappings toward JSON would
refuse `{"a":1,}`/`{,}`, both of which real yq accepts. The flow-**mapping** side (real yq's
own, quite different, token-*pairing* leniency — including the one silent wrong value this
issue originally found, `{"a":1 "b":2}`) is closed separately below by
[#2777](https://github.com/rust-works/succinctly/issues/2777), on the plain route only —
see that section for what it does and does not cover. Genuine-YAML `[,]`/`{,}` (unrelated to
JSON-sourced input at all) is tracked separately as
[#2779](https://github.com/rust-works/succinctly/issues/2779). (Scalar grammar, `[01]` vs
`.5`, was the other half of #2279's original scope note — closed separately below by
[#2778](https://github.com/rust-works/succinctly/issues/2778).)

**Closed by [#3394](https://github.com/rust-works/succinctly/issues/3394):
JSON escape introducers.** Measured against yq v4.53.3 for every byte 0–255
following a backslash (with `0061` supplied after `u`): only `"`, `\`, `/`,
`b`, `f`, `n`, `r`, `t`, and `u` succeed. In particular, backslash followed
by raw LF, CR, or CRLF errors, even in an unselected value or an object key.
`Parser::parse_double_quoted` now enforces that boundary under `json_strict`
before lazy decoding or streaming output can reinterpret the break as a YAML
line continuation and silently discard it. Genuine YAML retains its line
continuations and YAML-only escapes. Raw *unescaped* control bytes in JSON
strings retain yq's leniency and are preserved by #3380; this check applies
only immediately after a backslash. Error wording uses succinctly's existing
structured parser diagnostic rather than Go's decoder message.

**Closed by [#2778](https://github.com/rust-works/succinctly/issues/2778): scalar grammar
at every value position.** #2279 above closed *delimiters*; every scalar `-p json` accepted
that real yq rejects survived it, because delimiter checks never look at scalar text. The
boundary is not "strict JSON" — it is exactly `goccy/go-json` v0.10.6's own two layers,
re-derived from its source and pinned across ~60 probes against v4.53.3:

1. **Token scanner** (`internal/decoder/stream.go` `skipValue`): a value token may start only
   with `{ [ " t f n -` or a digit. `t`/`f`/`n` must spell exactly `true`/`false`/`null`. A
   number token greedily consumes `[0-9.eE+-]`; the first byte after it must be a delimiter.
   Anything else (a bare letter, `'`, `.`, `+`, `~`) never reaches step 2 at all.
2. **Number parse** (`strconv.ParseFloat`): optional `-`, `digits ['.' digits*] | '.' digits`,
   optional `[eE][+-]?digits`, at least one mantissa digit, leading zeros allowed, `ErrRange`
   on overflow (`1e999`/`2e308` reject; Rust's equivalent returns `inf`, so the port checks
   `is_finite()` instead of erroring).

`json_strict_plain_scalar_ok` (`src/yaml/parser.rs`) implements this with Rust's own
`f64::from_str`, which matches Go's `ParseFloat` row-for-row on the pinned matrix — including
every non-RFC-8259 acceptance real yq's leniency still allows: leading zeros (`[01]`→`[1]`,
`[00]`→`[0]`) and a bare trailing `.` (`[1.]`→`[1]`, `[1.e5]`→`[100000]`) all survive, while
`.5`, `+1`, `1e`, `'a'`, `True`, `~` all error — because layer 1 never lets a `.`/`+`/letter
token through to `ParseFloat`, not because `ParseFloat` itself would reject them.

Applied at every scalar *value* position the JSON-sourced parser reaches — flow-sequence
item, flow-mapping value, and bare block/top-level scalar — under the same `json_strict`
flag #2279 introduced, so `a`, `True`, `.5`, `1e999` are rejected identically whether nested
in `[...]`, in `{"k": ...}`, or standing alone as the whole document.

Alongside scalar-grammar validation, four YAML-only *structural* tokens are rejected the
same way, because none of them have a JSON spelling at all regardless of what text follows
them — real yq's token scanner refuses the byte itself, never reaching a "value" to
validate:

- A bare top-level `- item` (no enclosing `[]`) — rejected at `parse_block_node`'s single
  chokepoint for every illegal value-start byte, which also covers `?`/`&`/`!`/`*`/`'`
  uniformly rather than each needing its own arm. A bare top-level `key: value` (no
  enclosing `{}`) is rejected only when the *key* itself starts illegally (`a: 1`, since
  bare `a` was never a legal token) — **not** as a blanket "no bare mapping" rule: `-p json`
  without `--slurp` turns out to read a *stream* of concatenated top-level values (`"a": 1`
  and `true: 1` both parse as two complete values in real yq, confirmed live against
  v4.53.3, not a mapping and not an error), so an earlier version of this fix that rejected
  every bare `key: value` outright was reverted before merge -- it over-rejected input real
  yq accepts. Several top-level values are now read as a stream (#2839): an input that opens with a scalar
  (`"a": 1`, `true: 1`, `1 2`) is cut into its values first, and one that opens with `{`/`[` is cut
  only after its single-value parse has failed (so on the streaming routes a well-formed object or array pays nothing; `--eval-all`/`--slurp`/`--inplace` cut every input first, one byte scan that measured within noise of a 16 MB document's ~700 ms),
  each value (whitespace, `,` or `:` between them) re-read as one document. `1 2`, `{"a":1}{"a":2}`,
  `[1][2]`, `1,2`, `"a": 1` and `true: 1` match yq on stdin and on a file. A stream whose *later* value
  is bad (`1 2 ]`, `{"a":1} x`, `1 {"a":`, `1 [1,,2]`) prints the values before it and then fails, as
  yq does -- the good prefix is evaluated as usual and the error comes last, so a filter error on a
  good value wins, and a later file is never read. `--eval-all`, `--slurp` and `--inplace` read every
  value of a stream and fail before printing or writing anything when one is bad, as yq's do (they
  used to read the first value and drop the rest silently). The error *text* is succinctly's own, as
  for every other `-p json` parse error. One gap remains: the index addresses the rewritten
  `v1\n---\nv2` text, so a byte offset, `line` or `column` of a streamed value counts the inserted
  separators.
- `?` (the explicit-key indicator), in *both* the block-context and flow-mapping (`{? "a":
  1}`) spellings — a structural rejection of the marker itself, not a key-grammar check, so
  it stays in scope even though mapping-key *text* grammar (#2777) does not.
- `&anchor`/`!!tag` prefixing any value — real yq errors on the prefix byte before it would
  ever reach a scalar, an anchor, or a nested container behind it, so `&x True`, `&x 'y'`,
  and even this fix's own new `&x - 1` structural rejection are all refused uniformly.
- `--- |`/`--- >` (a block-scalar indicator right after a document marker) — this shape
  bypasses the ordinary block dispatcher via its own dedicated fast path
  (`parse_inline_document_value`), so it needed its own separate gate alongside the
  chokepoint above.

An implicit single-pair mapping inside a JSON *array* (`[a: 1]`, `["a":1]`) is rejected the
same way, in `parse_flow_sequence_inner` — a JSON array element is a value, never a `key:
value` pair, so the whole construct errors before the key's own text is even examined.

**Mapping *keys* were exempt from this issue — deliberately, not an oversight — and are now
closed by #2777, below.** Real yq's own JSON decoder *panics* (does not cleanly error) on a
non-string key — `{1:1}`, `{true:1}` both crash with `interface conversion: json.Token is
float64, not string` (exit 2, confirmed live). Per ADR-0018 rule 4(c), succinctly does not
reproduce a reference panic; `{"1":1}` (a *string* `"1"` as key, succinctly's existing
behavior) is unchanged and unrelated. #2778 itself only ever touched what a scalar *value*
token may spell, plus the four structural, key-agnostic rejections above; key delimiter and
quoting rules (`{a:1}`, `{'a':1}`) were left to #2777, which now rejects both — a key must be
a double-quoted string, so anything else that isn't even a well-formed non-string JSON
literal (a bareword, a single-quoted string, `?`/`&`/`!`/`*`) is an ordinary "invalid
character as JSON object key" error, distinct from the rule-4(c) panic-class rejection a
well-formed non-string literal (`1`, `true`, `null`) gets instead.

**Closed by [#2777](https://github.com/rust-works/succinctly/issues/2777): flow-mapping
token-pairing grammar, on the plain route.** Real yq's own object grammar is not a stricter
or looser version of JSON's or YAML's flow-mapping grammar — it isn't a grammar for `{...}`
at all. Traced from `goccy/go-json`'s own source (`Stream.Token()`/`PrepareForDecode`, the
machinery `CandidateNode.UnmarshalJSON` actually calls for an object body): the decoder reads
a flat stream of tokens and pairs them up (key, value, key, value, ...), where punctuation is
just noise to skip, not structure to validate:

- Before each key, whitespace and *any* run of `,`/`:` is skipped — both are pure separators,
  so `{,}` → `{}`, `{,"a":1}` → `{"a":1}`, `{"a":1,,"b":2}` → `{"a":1,"b":2}`, and even `{:1}`
  (the leading `:` is skipped the same as a `,` would be, leaving a bare numeric "key" that
  then hits the panic-class rejection below).
- A key must be a JSON string; anything else that scans as a well-formed *non-string* JSON
  literal (`{1:1}`, `{true:1}`, `{null:1}`) is the reference's own interface-conversion panic
  (exit 2) — reproduced as a clean rejection, not the crash, per ADR-0018 rule 4(c). Anything
  that isn't even a well-formed literal at all (`{abc:1}`, `{'a':1}`) is an ordinary
  scanner-level error in the reference too.
- After the key, whitespace and *at most one* `,`/`:` is skipped — a stricter rule than the
  key side, so `{"a" 1}`/`{"a":1}`/`{"a",1}` all reach the value, but `{"a"::1}`/`{"a",,1}`
  (two separators) do not.
- The value must then start on a genuine JSON value byte — `{`, `[`, `"`, or a number/
  `true`/`false`/`null` token whose *boundary* stops at the first byte that cannot continue
  it (not at the next comma/brace the way YAML's own unquoted-scalar scanner does), which is
  what actually fixes the issue's own headline row: `{"a":1 "b":2}` used to read the space and
  the following `"b":2` as part of one long invalid scalar (a silent wrong value before #2778
  taught the scanner to at least *notice* that and reject it cleanly; #2777 is what makes it
  split into two entries and match value-for-value). Nested arrays are unaffected — they keep
  their own, unrelated strict `[...]` delimiter/scalar rules (#2279/#2778): `{"a":[1 2]}`
  still errors, matching the reference's own asymmetry (object bodies are token-paired, array
  bodies go through `json.Unmarshal`'s ordinary strict slice decoder).

`Parser::parse_json_strict_flow_mapping_entries` (`src/yaml/parser.rs`) implements this as a
wholly separate loop from the ordinary YAML flow-mapping entry loop
(`parse_yaml_flow_mapping_entries`), rather than more `if json_strict` branches threaded
through it, since the grammar is different in kind, not just stricter or looser in degree.

**Not reproduced: which bracket ends the loop.** The reference's token scanner treats *every*
bracket character (`{`, `}`, `[`, `]`) as a `Delim` that ends the *current* object's
key-reading loop the instant one is seen — including a `]` or a second `{`/`[` where a key
was expected, with no requirement that it be *this* object's own closing brace (confirmed
live: `{"a":1 ]}` → `{"a":1}`; `{[1]:2}` → `{}`). Whatever is left unconsumed in that case is
then read as the *next* top-level document by yq's own stream evaluator, whose own
trailing-content handling is itself inconsistent — some leftover shapes produce no visible
error at all, others (differing only in which byte is left over) print a stream-decode error
to stderr while still emitting the correct value and exiting 0. Reproducing this would mean
modelling a second, top-level-only decoder loop with the same inconsistency, for a shape (a
bare bracket standing in for a key, with no quotes at all) that essentially never occurs in
real-world JSON — deferred as
[#2883](https://github.com/rust-works/succinctly/issues/2883), and succinctly instead
reports an ordinary parse error for a `{`/`[`/`]`
found where a key is expected, always failing the whole document cleanly rather than
silently producing a truncated one.

**Scope: the plain route only, not `--inplace`/`--slurp`/`--eval-all`.** Those routes
materialize through `JsonIndex`'s DOM bridge (`yq_runner::parse_input`'s `InputFormat::Json`
arm), a separate code path from the `YamlIndex`-backed parser this fix touches, and one that
still applies #1975's own stricter-than-real-yq grammar. `succinctly yq -p json -i '.'` on a
file containing `{"a":1 "b":2}` still refuses and leaves the file untouched, where real yq
rewrites it to `a: 1\nb: 2`. Unifying the two routes onto one parser (so the DOM bridge gets
this same leniency, and so the two stop disagreeing with each other as well as with real yq)
is a separate, larger change that touches the destructive `--inplace` write path — tracked as
[#2883](https://github.com/rust-works/succinctly/issues/2883) rather than folded into this fix.

**Cost: `-p json`/`--input-format json` parsing is measurably slower, isolated entirely to
that path.** Every plain (unquoted) scalar under `json_strict` now costs a byte-charset scan
plus an `f64` parse it did not pay before -- CI's `perf-guard.py` (issue #1523) measured
`users_yq_keys_unsorted` (a `keys_unsorted` query over a 2MB `-p json`-detected fixture,
mostly numeric fields) at +13.5% instructions (x86_64) / +12.5% (ARM64-Linux) against
`main`, both over the 5% regression-guard threshold when #2778 landed; every other query in
that suite — including plain `jq`-mode queries over the identical `.json` fixture, which
never reach `YamlIndex` at all — measured within ±0.1%, cleanly isolating the cost to
`json_strict` itself rather than anything shared. `check_json_strict_scalar`/
`json_strict_plain_scalar_ok` carry `#[inline]` (dispatched from a dozen call sites) as the
one free win available; skipping the redundant `str::from_utf8` re-validation after the
charset check would remove more, but needs `unsafe`, which this crate forbids
(`-D unsafe-code`) -- so the real remaining cost is inherent to the validation work itself,
not an implementation gap. Accepted per ADR-0018 (fidelity over performance is the default; a
performance cost is not among the rule's carve-outs for declining to match the reference) --
CI's `Perf Regression Guard` is deliberately outside the merge queue's own required-checks
subset, precisely so a real, understood, narrowly-scoped correctness cost like this one does
not block merging the correctness fix that causes it.

**#2777 recovered most of that cost, as a side effect, not a deliberate optimization pass.**
The same `users_yq_keys_unsorted` benchmark measured **-10.8% instructions (ARM64-Linux) /
-10.4% (x86_64)** against #2777's own merge-base (both again over the 5% threshold, this time
in the improving direction), landing within ~1% of the pre-#2778 baseline. Mechanism:
`parse_json_strict_flow_mapping_entries` (#2777) is a dedicated loop for JSON-sourced flow
mappings, replacing the generic YAML flow-mapping loop (`parse_yaml_flow_mapping_entries`,
still used for genuine YAML) that `json_strict` input used to route through. That generic
loop's own per-entry overhead -- `looks_like_explicit_flow_key`'s lookahead, `parse_flow_key`'s
broader anchor/tag/alias/nested-container/unquoted-key dispatch -- was paid on every key even
though almost none of it can ever fire under `json_strict` (a JSON-sourced key is either a
`"` or a rejection, never an anchor or a bareword); the dedicated loop's key check is a single
`self.peek() == Some(b'"')`. `keys_unsorted` is exactly the shape most exposed to this (all key
parsing, comparatively little value parsing), which is why this query moved and the others in
the suite (identity-shaped, value-heavy) stayed within noise both times. Not chased further as
a deliberate optimization -- it fell out of writing the grammar-correct implementation the
natural way.

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
fix and remain live for the routes that genuinely materialize — `-P` (with or without `-I0`,
since #2606 fixed `-I0`'s own then-"compact" status wrongly satisfying `can_yaml_fast_path`'s
gate on its own regardless of `-P`; every `-P` invocation now forces the DOM route
unconditionally), `--arg`/`--argjson`, `-r`/`-j`/`-0`, `--eval-all`, `--split-exp` (each
verified live against this binary; `--slurpfile` is a jq flag `succinctly yq` does not
accept) — [#1982](https://github.com/rust-works/succinctly/issues/1982).

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

**Closed by #2606** (previously recorded here as a residual divergence #1575 deliberately
didn't chase): real yq's own `-I0` is not width 2 — it's byte-identical to its own `-I4`
output at every level (verified live against v4.53.3, and swept across all 150
`tests/data/yq-golden` inputs with zero divergences). The "irregular per-level quirk"
this entry used to describe (a compact block-sequence item's inlined mapping not
incrementing its own indent depth for its fields' children) turned out not to be an
unmodeled oracle behavior at all — it's go-yaml's ordinary
`indent = BestIndent * ((indent + BestIndent) / BestIndent)` rounding, which both
succinctly output routes already reproduce correctly at every width once the width itself
is right. Only the `0 -> 4` mapping was ever missing; the width table is now pinned
directly from real yq's own dependency source (`go.yaml.in/yaml/v4`,
`internal/libyaml/representer.go`'s `Representer.init` composed with
`internal/libyaml/emitter.go`'s own clamp) in
[`IndentSpec::for_yaml`](../../../src/jq/document.rs): `0 -> 4`, `1 -> 2`, `2..=9 -> n`
unchanged, `>= 10 -> 2`.

Two related bugs surfaced and closed alongside the width table itself: `-I0`'s own
(then-"compact") status satisfied `can_yaml_fast_path`'s gate on its own regardless of
`-P`, so `-P -I0` on flow-style input wrongly stayed flow instead of expanding to block
style the way `-P` does at every other `-I` value; and `YqCommand::indent`'s CLI parser
capped every value at 7 (jq's own `--indent` limit, wrongly applied to yq mode), where
real yq accepts any non-negative width for JSON output, unbounded above (only YAML is
clamped, per the table above). A negative `-I` value still cannot reach real yq's own
panic path here (`yaml: cannot indent to a negative number of spaces`, exit 2) — clap's
own unsigned-integer rejection is the ADR-0018 rule-4(c) answer (matching would take the
host process down), so succinctly's own clean parse-error rejection for `-I -1` remains a
deliberate divergence, not something #2606 changed.

**One more deliberate divergence, this time between two `succinctly`-internal walks rather
than against real yq**: `path(f)`
([`path_walk_generic`](../../../src/jq/eval_generic.rs), #2075) and `key`/`path`/`parent`
([`path_context_step_generic`](../../../src/jq/eval_generic.rs), #2061) both delegate to the
same shared `path_step_generic`, but pass it different duplicate-key collapse flags —
[#2147](https://github.com/rust-works/succinctly/issues/2147) recorded why, since the two
call sites sit ~500 lines apart and the mismatch reads as an oversight without this:

```bash
$ printf '{"a":1,"a":2}' | succinctly yq --jq-extensions -o=json -I=0 '[path(.[])]'   # [["a"],["a"]]
$ printf '{"a":1,"a":2}' | succinctly yq -o=json -I=0 '[.[] | key]'                   # ["a"]
```

`key`/`path`/`parent` pass `true` unconditionally because they replace a bridge that used to
materialize into an `IndexMap` (which collapses structurally in both modes) — real yq agrees
(`["a"]` in v4.53.3). `path(f)` passes `S::COLLAPSE_DUPLICATE_KEYS` (the mode's own rule)
instead, because it isn't replacing a materialization and has **no yq oracle at all**: real
yq's `path` is the no-argument form only, so `path(f)` is a jq builtin succinctly exposes as
an extension in yq mode (gated behind `--jq-extensions`, #2430), exempt from ADR-0018's
divergence rule. In jq mode both walks agree
with real jq 1.7.1 (`[["a"]]`). So the two flags are a recorded choice, not a bug — this
entry (plus the cross-referencing doc comments on both functions) is what keeps a future
reader from "fixing" the mismatch into a real divergence.

### Bare `first` as a path step — sequences, scalars and `null` agree; a mapping's key and a comma over `null` do not (#3551)

Real yq's bare `first` is a path step at the head of an expression (v4.53.3): on a sequence it
addresses element 0, so `first = 9`, `first |= 5`, `first += 1` and `del(first)` on `[1,2]` are
`[9,2]`, `[5,2]`, `[2,2]` and `[2]`. On an empty sequence, a scalar and `null` it addresses
nothing, and every write and delete leaves the document untouched. `succinctly yq` matches all
of that. It used to drop the write silently on a sequence (`first = 9` stayed `[1,2]`, exit 0)
and raise on `del(first)`.

Two shapes still differ:

- **On a mapping, `first` names the first *key***, and assigning through it renames that key:
  `first = 9` on `{a: 1, b: 2}` is `{"9":1,"b":2}`, `first += 1` is `{"a1":1,"b":2}`, and
  `del(first)` is `{"b":2}`. There is no path component for a key rename here, so every write and
  delete through bare `first` on a mapping is refused (`bare first on a mapping names its first
  key; writing through it (renaming the key) is not supported`) rather than dropped. Reading
  `first` on a mapping is also still `Cannot index object with number`, where yq answers the key.
  The refusal covers every path-resolving use of `first`, not only writes: in a mixed stream
  (`(.[] | first) = 5` over `[[1,2],{a: 1}]`) the one mapping element aborts the whole
  expression with nothing partially applied, where yq renames that key and writes the rest.
- **A comma over `null` or an empty sequence**: `(first, .[1]) = 7` on `null` or `[]` is
  `[7,7]` in yq, because the sibling path creates the sequence `first` then lands on; here it is
  `[null,7]`.

Under `--jq-extensions`, `path(first)` on an empty sequence or `null` yields no path (yq has no
`path`), where `path(nth(0))` yields `[0]`: bare `first` follows yq's operator there, not jq's
`.[0]`.

Not changed here: real yq reads bare `first` only at the head of an expression (`.a | first` is
`'|' expects 2 args but there is 1`), where `succinctly yq` applies it after a pipe as it does in
jq mode. Bare and argument `last`, which real yq's lexer rejects in every form, are gated behind
`--jq-extensions` like the rest of the jq-only surface (#3560): ADR-0018 rule 3 makes matching the
reference the default, and none of rule 4's conditions lets us keep accepting it. Under the flag
they keep jq's meaning (#1521).

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

### `load`, `load_str` and `load_base64`: four rows still differ ([#6985](https://github.com/rust-works/succinctly/issues/6985))

The loaders read their file name as yq does: the argument's first output, a scalar by its text, and `filename expression returned nil` when the argument has no output.
A read failure is worded as Go words it (`failed to load nope: open nope: no such file or directory`, `read adir: is a directory`), and a file that does not parse is `failed to load F: bad file 'F': ...`.
Four rows differ:

- **A literal `null` name.**
  yq opens the file `null` (`load_str(null)` is `open null: no such file or directory`), and an absent key (`load_str(.zz)`) is `filename expression returned nil`.
  An absent key reads as `null` here, so both are the nil refusal: the absent key is the common case.
- **Bytes that are not valid UTF-8 in `load_str`.**
  yq keeps the raw bytes, which its JSON encoder writes as one `\ufffd` each; this substitutes one U+FFFD per byte on the read (#7000), so the string is the same under `-o=json` and the character is printed itself (the rule of the `-p json` section above).
  `length` and YAML output differ as for `@base64d` (see "Decoded bytes that are not UTF-8" below).
- **The reason a file does not parse.**
  After `bad file 'F': ` yq gives go-yaml's message (`yaml: while parsing a flow sequence at <unknown position>`), and this gives its own parser's (`unexpected end of input: flow sequence`).
- **A `.json` name holding YAML.**
  yq reads every file `load` opens with its YAML decoder, so a `.json` file holding `a: 1` loads as `{"a": 1}`.
  This picks the JSON reader by the name's extension and refuses it (`bad file 'f.json': unexpected character`); a file that is JSON loads alike.

### `;` is a parse error here; in yq it is the BLOCK operator, which evaluates to nothing (#2881)

Real yq lexes `;` as a binary operator (`BLOCK`) wherever an expression is allowed, so
`select(1;2)`, `has(1;2)`, `join(1;2)`, `(1; 2)` and `[1; 2]` all parse. `succinctly yq` rejects each with
`expected ')', found ';'` (and a top-level `.a; .b` with `unexpected character ';'`). Captured
against yq v4.53.3, the operator has no value to mirror: a `;` expression yields *no results*
whatever its operands are — `.a; .b`, `1; 2; 3`, `.a = 5; .` and `(.a = 5; .b = 7)` print nothing and
leave the document alone, `[1; 2]` is `[]`, `has("a"; "q")` and `has("q"; "a")` are both `false`, and
`del(.a; .b)` deletes nothing. The only observable effect of accepting it would be to turn a
parse error into silence, so this stays a deliberate, stricter rejection: a script that spells `;`
in a yq filter is far more likely to be a jq habit (`limit(1; f)`, `reduce`, `foreach`) than to want an
empty result, and a loud error is the safer answer to it. The jq-style multi-argument forms that
`--jq-extensions` enables (`limit`, `range`, `sub`, ...) are unaffected.

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
unchanged in jq mode. A non-string input is refused in yq's words, `cannot split !!seq, can only
split strings` (#5836, see below).

### String builtins refuse an input whose tag is not `!!str` (#5836)

yq's `split`, `sub`, `test`, `match`, `capture`, `upcase`, `downcase`, `trim`, `@base64`, `@uri` and `@sh` refuse an input whose tag is not `!!str`, naming the tag, and `succinctly yq` does the same with yq's wording (`cannot change case with !!int, can only operate on strings. `, `cannot encode !!seq as URI, ...`, which `@sh` uses too).
A number, a bool, a null, a container, a plain date (`!!timestamp`), a plain `<<` (`!!merge`) and an explicit core tag on a string (`!!binary aGk=`, `!!float "x"`) are refused; a custom tag reads as its value's own type, so `!foo x` is a string and `!foo 5` is refused as `!foo`.
A bad pattern or flag is still reported first, as yq checks those before the input, and `split` passes a null through.
`tag` and `type` report a verbatim tag as what it spells, a core one in its `!!` form (`!<tag:yaml.org,2002:binary>` is `!!binary`, `!<!foo>` is `!foo`), as yq does.
An alias node keeps its target's value, where yq applies the builtin to the alias's name (`b: *x` with `upcase` is `X`).
The refusal is made where the builtin reads a document node; a value reached through an update (`.b |= upcase`), `to_entries`/`with_entries`, or a literal under `-n` (`5 | @base64`) still gets jq's wording or, for `@base64` and `@uri`, an answer.
Under `--jq-extensions`, `ascii_downcase` and `ascii_upcase` are `downcase` and `upcase` and refuse the same inputs, a plain date among them.

### Regex patterns are read as Go's `regexp` reads them (#5374, #5388)

yq compiles a pattern with Go's `regexp` (RE2), and `succinctly yq` now reads it the same way before handing it to the `regex` crate (`src/jq/go_regex.rs`).
`\w`, `\d`, `\s` and `\b` are ASCII (`\s` has no `\v`), `\Q..\E` is a literal span, `\<` is a plain `<`, a `{` that starts no repeat is a literal, `\0` and `\12` are octal, and `$` is the end of the text only (jq's `$` also matches before a final newline).
What Go refuses is refused with Go's own message (`error parsing regexp: invalid nested repetition operator: `++``): `a++`, `(?x)`, `(?=`, `[a--b]`, `\Z`, `\1`, `\c`, a repeat with nothing to repeat, an unclosed group or class, a counted repeat past 1000, counted repeats that nest to a product past 1000.
Pinned by `tests/yq_cli/i5388.rs` (115 rows captured from yq v4.53.3) and checked against yq over 3,000 random patterns with no difference.
A `\p{Name}` is looked up in Go's table, as yq's Go does (#5388): the name is canonicalised, so `\p{greek}` and `\p{Letter}` are accepted, while `\p{Alphabetic}`, `\p{Grek}`, `\p{IsL}`, `\p{Old_Italic}` and `\p{SignWriting}` are refused as in yq.
A flag set and cleared in one group (`(?i-i)`, `(?-ii)`) is read as Go reads it, and the surrogate category `\p{Cs}` matches nothing.
The `\p{Name}` table is a copy of the one in the Go that built yq v4.53.3, which lacks the Unicode 16 and 17 scripts Garay, Sidetic, Sunuwar and Todhri.
A yq re-pinned to a build with a newer Go needs the table re-derived; it was generated by probing `\p{..}` spellings against the binary.
Still different: a named group whose name starts with a digit (`(?P<1n>a)`) or repeats an earlier one (`(?P<a>x)(?P<a>y)`) is accepted, as in yq, but the group is unnamed here (the crate refuses both), so `capture` does not report that name; and a surrogate escape (`\x{D800}`), which Go reads as a rune no text contains, is read here as U+FFFD.
jq mode keeps Oniguruma's own rules (see the jq limitations).

### A prefix shared by every branch of an alternation stays in each branch (#6134)

Go's `regexp` factors only a simple prefix (a literal, a class) out of an alternation, so `.s | match("a*a|a*b")` on `s: aab` is `"aa"` in yq v4.53.3.
The `regex` crate's parser also factored out a prefix that can match more than one way, and the second branch's `"aab"` won; `succinctly yq` now answers `"aa"` too.
The mechanism, shared with jq mode, and the one shape it leaves (a pattern whose guarded form the `regex` crate refuses for its size) are in the jq limitations' #6134 entry.
Pinned by `tests/yq_cli/i6134.rs`.

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

### Bare 2-arg `sub(re; s)` — the replacement is read once, against the input, as a Go template

[#4905](https://github.com/rust-works/succinctly/issues/4905): real yq evaluates the replacement of
a bare `sub` **once, against `sub`'s own input**, ahead of the input's type check and the regex
compile and whether or not anything matches. It takes the first output (an empty stream is `""`, an
escape anywhere in it raises with no output), reads no capture object, and then runs Go's
`ReplaceAllString` with that text as the *template*, so `$1`, `${name}` and `$$` expand
(`Regexp.Expand`). `succinctly yq` had followed jq's `def sub`, evaluating the replacement per match
against the capture object; the two agree only for a literal replacement. Now (jq mode, and yq's
`gsub` extension behind `--jq-extensions`, keep the per-match model):

```bash
$ yq            -n '"a1b" | sub("a"; .)'               # "a1b1b"   (. is the input, not a capture object)
$ succinctly yq -n '"a1b" | sub("a"; .)'               # "a1b1b"   (matches)
$ yq            -n '"a1b" | sub("(?<x>a)"; .x)'        # "1b"      (.x is absent on the string: "")
$ succinctly yq -n '"a1b" | sub("(?<x>a)"; .x)'        # "1b"      (matches)
$ yq            -n '"abab" | sub("(b)"; "<$1$1>")'     # "a<bb>a<bb>"
$ succinctly yq -n '"abab" | sub("(b)"; "<$1$1>")'     # "a<bb>a<bb>"  (matches; was "a<$1$1>a<$1$1>")
$ yq            -n '"a1b" | sub("z"; error("boom"))'   # Error: boom  (evaluated with no match)
$ succinctly yq -n '"a1b" | sub("z"; error("boom"))'   # Error: boom  (matches)
```

The 3-argument form is unchanged (#1122: the replacement is never evaluated). Non-string
replacement values keep their #1052 coercion. Pinned by `tests/yq_cli/i4905.rs`. Still different,
unrelated to this rule: yq's non-string-input error wording (`cannot substitute with !!int`) and its
regex-compile error text.

### String-literal escapes — yq keeps every backslash sequence it does not decode

[#4916](https://github.com/rust-works/succinctly/issues/4916): real yq's filter lexer decodes only
`\n \t \r \b \f \a \v \" \\` (and `\(` starts an interpolation) in a string literal, and keeps any other
backslash sequence as written, backslash included. `succinctly yq` read the literal as jq does: it
decoded `\u` and `\/` (a silently different value) and failed to parse every other escape, which broke
the common `test("\d+")`. Now (jq mode keeps jq's table):

```bash
$ yq            -n '"a1" | test("\d")'      # true     (the regex engine sees \d)
$ succinctly yq -n '"a1" | test("\d")'      # true     (was: parse error: invalid escape sequence '\d')
$ yq            -n -o=json '"\u0041"'       # "\\u0041"   (six characters, not A)
$ succinctly yq -n -o=json '"\u0041"'       # "\\u0041"   (was "A")
```

The same table applies to object keys, bracket steps (`.x["a\tb"]`) and every other string literal, with one exception: the quoted name of a *dotted* path step (`.x."a\tb"`) is read raw, escapes included, up to the next `"`, so it selects the key `a\tb` (a backslash and a `t`) and `.x."a\"` is the key `a\`. A document's own
double-quoted scalars are a different lexer (YAML, see above) and are unchanged. Pinned by
`tests/yq_cli/i4916.rs` (147 rows captured from yq v4.53.3). Still different: a `\"` inside an interpolation's expression
(`"\(\"a\")"`),
the regex dialect (read as Go's since [#5388](https://github.com/rust-works/succinctly/issues/5388), see "Regex patterns are read as Go's `regexp` reads them" below)
and a string-literal `@format` (`@base64 "x\q"`, where both refuse with different wording).

### Renaming a key with `(PATH | key) = ...` — the rename matches; seven shapes still differ ([#6089](https://github.com/rust-works/succinctly/issues/6089))

`(.a.x | key) = "meow"` and `(.a.x | key) |= "m_" + .` rename the key of the member the path names, in place, as yq does.
The new name keeps the key's quoting, anchor and comments, and the value under it is untouched.
`=` evaluates its right side against the document and every target takes its last output; `|=` binds `.` to each key's text and takes the first.
A target that is a sequence item or the document root renames nothing, as in yq.
A target that does not exist is created as `null` and renamed (`(.a.zz | key) = "q"` adds `q: null`), as in yq.

Not reproduced:

- **A name that is already a member is an error.**
  yq keeps both entries (`(.a | key) = "b"` over `a: 1` / `b: 2` prints `b: 1` and `b: 2`).
  A mapping here cannot hold a repeated key, and overwriting one would lose a value, so the rename is refused with `cannot rename key 'a' to 'b': the mapping already has that key`.
  `(.a[] | key) = "k"` over two members is the same refusal.
- **A key is text.**
  `(.a.x | key) = 7` prints `7:` as yq does, but a later stage reads the key back as the string `"7"` where yq reads the number (`| .a | keys` is `- "7"` here and `- 7` in yq).
- **A name that is a mapping, a sequence or a text with a line break is refused**, where yq writes a complex `? ` key or a `? |-` block key.
- **The key keeps its quoting only where the presentation replay reads the filter.**
  A target of fields, indexes and `.[]`, and a right side of literals, `.` and arithmetic on them, keep the key's quoting, anchor and comments.
  Any other filter (`(.a.x | key) = (.a.y | tostring)`, a target with a `select`) still renames, but the document's comments and quoting are lost, and a string name that reads as another type is printed plain (`4:` where yq prints `"4":`).
- **A `select` over `key` in a write target binds the key only where the select's own input reaches it (#6206).**
  `(.. | select(. as $k | $k | key == "x")) = 5` reads `key` after the head of a pipe, and `(.l[-1] | select(key == 2)) = "X"` has a negative index in the path, whose key is not the index spelled.
  Both write nothing here, where yq writes to the matching nodes.
- **`+=`, `-=`, `*=` and `//=` on a `(PATH | key)` target are a parse error.**
  yq accepts `+=` (`(.a.x | key) += "z"` is `xz`) and errors for the others.
  Without `--jq-extensions`, `//=` is refused earlier still, as an operator yq lacks (#6981), and the key-target refusal is the one with the flag.
  They used to be a silent no-op, which discarded the write (ADR-0018 rule 4(b)); a refusal keeps the write from vanishing.
- **A rename through an alias** (`a: &r {x: 1}` / `b: *r`, `(.b.x | key) = "z"`) renames only the copy under `b`, and prints it as a mapping, where yq renames the anchor's node (`a: &r {z: 1}` / `b: *r`).

### `|=` over a descent writes deepest first; a comma of nested paths keeps its named order (#6087)

yq updates the nodes a descent selects, so an outer node's update cannot move an inner one out of reach.
`succinctly yq` addresses them by path, so for an update whose path expression contains `..`, `recurse` or `recurse(f)` and resolves to several paths, the paths are written deepest first (equal depths keep their order).
`(.. | select(tag == "!!map")) |= with_entries(.key |= "KEY_" + .)` then renames every level, as yq does.
Still different:
- a comma of nested paths, which is not a descent: `(.a, .a.b) |= {"w": .}` over `a: {b: 1}` is `a: {w: {b: 1}, b: {w: null}}` here and `a: {w: {b: {w: 1}}}` in yq, and the reverse order, `(.a.b, .a) |= {"w": .}`, differs from yq the other way (`tests/yq_cli/i6087.rs`);
- a descent reached through a user `def` or a builtin other than `..`, `recurse` and `recurse(f)`, which keeps document order;
- jq mode keeps document order: jq's `_modify` resolves its paths first and updates in order, so the same program answers differently there by design.

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

### `any`/`all(gen; cond)`'s `cond` follows the cursor route's prefix rule (#2968)

Real yq has none of `isempty`/`any(gen; cond)`/`all(gen; cond)`/`IN`/`skip` (lexer-rejected;
all six are behind `--jq-extensions`), so the baseline for them in yq mode is succinctly's own
previous answer. [#2968](https://github.com/rust-works/succinctly/issues/2968) moved all six
off the eager owned bridge onto native cursor-threaded arms — the positional-read fix it was
filed for, `any(.[]; key == "b")` being wrong on every route — and one shape moved with it:
a `cond` that is a pipe whose stage is a generator failing part-way.

```bash
$ printf -- '- [1]\n' | succinctly yq --jq-extensions '[any(.[]; .[0] | (true, error("late")))]'
# before: [true]        (the owned route kept the prefix, jq's rule)
# now:    Error: late   (the cursor route's rule, real yq's)
```

That is the rule this page already records for the cursor route — `first(.[0] | (true,
error("late")))` is `Error: late` here and in real yq, which discards a failing generator's
prefix ([#2326](https://github.com/rust-works/succinctly/issues/2326)) — reached one construct
over: `cond` is now evaluated at the `gen` output's own position, on that route. A bare
`cond` generator (`any(.[]; (true, error("late")))`) still answers `[true]`, exactly as
`first((true, error("late")))` does: the same split `first` already has.

`skip` moved in the same direction, and further: its kept outputs are now the cursors its
body produced rather than owned copies, so `skip(0; .)` keeps comments and anchors, `skip(0;
.b) | tag` on an alias answers `""` as `.b | tag` does, and `[skip(0; .[] | (., . * 2))]` is
`[1,2,3,2,4,6]` — the cursor route's (and real yq's) `[.[] | (., . * 2)]` order, where the
owned route interleaved `[1,2,2,4,3,6]`. Every row that moved in yq mode moved to what the
same expression already answered without the consumer around it.

### Residues

- **`delpaths` count-message wording.** Real yq says `DELPATHS: expected single value but found
  2`; succinctly says `expected a single result but found 2`. The *outcome* — an error rather
  than a fan-out or a silent truncation — is what #1279 preserved; matching the wording is
  unstarted. `setpath` takes yq's per-slot text (`SETPATH: expected single path but found 2
  results instead`, `...single value on RHS but found 2`) since #1380.
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

### A top-level `Partial` result never streams its prefix in yq mode

**Fixed by [#2392](https://github.com/rust-works/succinctly/issues/2392).** The prefix rule
above covers a *gated argument* (`has`, `split`, `contains`, ...) escaping mid fan-out. This is
the same rule applied to a different mechanism: the *entire result of a yq evaluation* escaping
at the runner's own emission boundary, not inside `fanout_arg`.

jq's own #400/#494 contract is "the outputs a generator already produced survive an escape that
happens later in the same stream" — `GenericResult`/`QueryResult::Partial(prefix, control)`
models exactly that, and jq mode's runner streams `prefix` before reporting `control`. Real yq
(v4.53.3, live-verified) has no such contract at the top level: it prints nothing at all before
a top-level error or break, in every shape checked —

```bash
$ yq '(1,2,error("x"))' doc.yaml            # Error: x   (no 1, no 2)
$ yq '.arr[] | keys' mix.yaml               # Error: cannot get keys of !!int, ...
$ yq '.arr | .[(0,"a")] | key' arr.yaml     # Error: cannot index array with 'a' ...
```

succinctly's yq-mode runner (`src/bin/succinctly/yq_runner.rs`) used to mirror jq mode's
`Partial` handling unconditionally at three arms, carrying jq's own #400/#494 doc comment over
without an oracle check for the yq side — the same "mirrored precedent inherited jq's bug"
pattern as the computed-slice-bound section below. #2392 discards the prefix instead, at every
point a yq-mode result leaves the evaluator: `evaluate_yaml_cursor`'s and
`query_result_to_owned_values`'s `Partial(_, Error|Break)` arms merge directly into their
existing bare `Error`/`Break` arms (identical reporting, prefix dropped); the M2 streaming path
has no match statement to merge into, so a small `discard_yq_partial_prefix` helper collapses
the result immediately after evaluation, before `produces_output()`/`stream_yaml`/`stream_json`
ever see it. `Control::Halt` is excluded everywhere — jq's own `halt` contract keeps
already-emitted output (#791/#1897), and real yq has no `halt`/`halt_error` to check against.

This is a distinct mechanism from "A computed comma-bound slice's own prefix is never streamed
in yq mode" below: that one discards inside the *shared evaluator*, at three internal
slice-bound-resolution call sites (gated per-site since only those call sites lack an oracle);
this one discards at the *runner's* emission boundary, unconditionally for every yq-mode result,
since the whole file is yq-only.

**Settled by [#2810](https://github.com/rust-works/succinctly/issues/2810): the discard is not whole-run, but the run stops at the first error.**
The earlier note here said real yq erases an earlier document's output when a later document fails.
It does not; the example behind it, `(.a, (select(.a==2) | error("x")))`, fails in document 1 as well, because yq's `error` fires even when `select` yields nothing.
Every error that really depends on a later document leaves the earlier output in place (yq v4.53.3):

```bash
$ printf 'a: 1\n---\na: 2\n---\na: x\n' | yq '.a | tonumber'   # 1, ---, 2, then Error: cannot convert node value [x] ...
$ printf 'a: x\n---\na: 1\n---\na: 2\n' | yq '.a | tonumber'   # Error: ... and nothing else: documents 2 and 3 are never evaluated
```

What differed was the second row: succinctly kept evaluating the documents after a failing one and printed their results, with one diagnostic per failing document.
It now stops at the first error the program raises, in every route that prints to stdout (the streamed one, the evaluated one and the `--sort-keys`/mapping-sort DOM fallback), across documents and across files, with that one diagnostic and exit 1.
A failure to decode a value (`a: "bad \q escape"`) and an evaluator resource cap are not such errors: yq has no counterpart for either, so the run still reaches the documents after one (#355, #1615, #1817).
`--split-exp` stops the same way and keeps the files of the results before the error, as yq does.
`--inplace` keeps going past an error as before, and leaves a file that failed untouched (#1615).
`--on-error=continue|stop` is a succinctly extension (ADR-0018 rule 5, [#5507](https://github.com/rust-works/succinctly/issues/5507)), the flag `succinctly jq` has had since #5471.
Without it nothing changes, and `stop` spells that default out.
`continue` switches the stop-at-the-first-error rule off, since yq has no reference behaviour for carrying on: every document and file is evaluated on every route above and in `--split-exp`, every error is reported, and the exit code is 1 if any failed.
Under `--inplace`, `stop` also edits no file after the first one that failed, where the default carries on.
The exit code stays 1 in every mode, as yq's does for an error and for `-e`'s falsy result alike, so a script cannot tell them apart.
`--eval-all` and `--slurp` evaluate everything together, so they have no later document to stop before.

The `error` operator itself still differs: yq raises it for a document that `select` already dropped (`printf 'a: 1\n' | yq 'select(false) | error("x")'` is `Error: x`), where succinctly does not evaluate it.
That is the gap of a pipe whose left side is empty, recorded under #5261 in the `as`-binding notes below, not a property of the run.

See [#2427](https://github.com/rust-works/succinctly/issues/2427) for the sibling per-run-vs-per-node streaming residue.

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

### Presentation metadata lost on one whole output route

A filter that is a comma of document nodes (`.x, .y`) keeps comments, style and anchors since
[#1361](https://github.com/rust-works/succinctly/issues/1361): its branches stay cursors, as
`.[]`'s results do. A comma with a branch that is not a node of the document (`.x, (.y | length)`,
`.x, 1`, an absent key as in `.x, .zz`) is answered branch by branch since
[#4856](https://github.com/rust-works/succinctly/issues/4856): the runner evaluates each branch of
a top-level comma on its own (through parentheses and nested commas), so a node keeps what its
cursor carries and a computed value has none. An error in any branch prints nothing, as for the
whole query. A comma nested inside a bigger expression (`(.x, .y) | f`, `[.x, .y]`, `. as $v |
$v, 1`) is read by that expression: it still drops comments, style **and** anchors together. It
predates [ADR-0017](../../adrs/adr-0017.md)'s mechanism and is not anchor-specific.

Two others have been closed. `map(...)` lost all three the same way for the same reason — it
took the DOM route, which carried none of them;
[#757](https://github.com/rust-works/succinctly/issues/757) closed it by streaming `map`'s
elements from their own cursors (see the duplicate-key section above for the full list of what
that one route was dropping). And `--inplace`'s DOM fallback, which never built a
`CommentTree` at all, was closed by
[#1349](https://github.com/rust-works/succinctly/issues/1349): a YAML-sourced `-i` file now
goes through the same cursor-native evaluator stdout uses, so `-i '.a = 99'` on
`a: &x 1\nb: *x` writes `b: *x` rather than the `b: 1` it used to — the same filter no longer
produces a different *value* depending on whether it edited the file or printed it. `-i`'s M2
fast path already shared `stream_cursor!` with stdout, so this only ever concerned the
fallback a non-M2-eligible filter reaches.

**The fork #1349 left between `-i` and stdout is closed for every filter that materializes
the mapping** ([#2519](https://github.com/rust-works/succinctly/issues/2519)). A document with
two *complex* mapping keys whose display spellings collide (`? [1]\n: v1\n? [2]\n: v2`, both
`""` per [#222](https://github.com/rust-works/succinctly/issues/222)) used to make `-i '.x = 1'`
raise while stdout printed `'': v2` and dropped the first entry silently; both routes now
raise the same `object key "" is ambiguous` error, and `-i` leaves the file untouched. `-i`'s
non-M2 branch still refuses such a file *outright*, before evaluating anything
(`validate_yaml_display_keys`), so a filter that stdout streams with both entries intact
(`to_entries`) is refused under `-i` -- the safe direction, since that pre-walk is also what
stops `-i 'to_entries | from_entries'` from writing a merged document. See the display-key
collision section below for which routes raise and which keep both entries.

A JSON-sourced `-i` file stays on the materializing route deliberately: a `YamlIndex` accepts
`[1,]` as a flow sequence, so rerouting it would reopen the hole
[#2276](https://github.com/rust-works/succinctly/issues/2276) closed, and JSON has no
comments, anchors or block style to lose either way. The duplicate-key cost noted above is
also unchanged by #1349 — `evaluate_yaml_cursor` still materializes each result into an
`IndexMap`-backed `OwnedValue` before `output_value` writes it, so the cursor-native route
preserves *presentation* but not duplicate keys.

### An untracked path branch — resolved for `path()`/`=`/`|=`/compound-assigns, including trailing navigation; still open for `del()` of a container's child

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
this file.
succinctly refuses those three forms in yq mode unless `--jq-extensions` is passed, as it does a
jq-only builtin ([#6981](https://github.com/rust-works/succinctly/issues/6981)): the error names
the flag where yq's reads `'/' expects 2 args but there is 1`.
With the flag they parse and inherit this same skip, which is not itself a divergence claim,
since there is nothing in real yq to diverge from.)

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

**Trailing navigation after a constant is covered too (#1868).** An untracked branch that is a
*constant* followed by a static navigation step — `(.a, 1 | .[]) = 5`, `(.a, 1 | .b) = 5`,
`(.a, "s" | .[0]) = 5`, `(.a, 1 | .[0:1]) = 5`, `(1 | .[]) = 5` — is the same silent no-op in real
yq (captured on v4.53.3: `a: 5` / `b: 2`), because yq has no "untracked" notion at all: a literal
is a detached node, navigating into a scalar yields zero candidates, and an update on a detached
node does not touch the document. `succinctly yq` used to raise jq's "Invalid path expression"
from the resolver's own navigation checks ahead of the terminal check above ever running. The
resolver now drops such a constant from the pipe before resolving it (`yq` mode only,
`prune_detached_constants_before_navigation`): a scalar literal (`null`, a boolean, a number or a
string) that sits first in the pipe or as a member of the comma feeding the step, and is followed by a field,
an index, a literal-bounded slice, `.[]` or any of those under `?`. `del(.a, [1] | .[])` is
`b: 2`, as in yq (so is `path(null | .a)`: no path, where jq refuses). jq mode keeps every refusal (#843/#986/#530), and an error raised while
*producing* a navigation key still propagates (`(.a, 1 | .[error("p")]) = 5` raises `p`, as in
yq).

The pruning is deliberately syntactic. Every other untracked value stays refused in yq mode,
because a refusal is the only honest answer where yq would write: a bound `$x` (yq's variables
hold the document's own nodes, so `.a as $y | .a | ($y.b) = 9` reaches the document; #2072, #3037;
the owned routes inline such a value as a literal array or object, which is why only a *scalar*
literal qualifies and `(.a, [1] | .[0]) = 5`, yq `a: 5`, stays refused),
a register or a fold's accumulator (#3456, #4445), a computed value (`(1 | tostring) | .[0]`), a
computed key (`(.a, 1 | .[$k]) = 5`, yq `a: 5`), a slice with a computed bound, and `..`
(`(.a, 1 | ..) = 5`, yq `a: 5`). Those rows are the open residual of #1868.

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

The rule is in yq's `deleteChildOperator`: it walks its targets back to front and returns at
the first one with no parent, keeping whatever it already deleted
([#1865](https://github.com/rust-works/succinctly/issues/1865)). The root, a literal, a
constructor and a bound value have no parent, so only the targets *after* the last such one
are deleted, and if that one is the root itself the document goes (`del(1, .)` prints
nothing, `del(., 1)` keeps the document). `resolve_del_path_branches` applies that scan to the
branches in source order, a fan-out ordered by its resolved targets (`del(1, .a[])` empties
`.a`, `del(.a[], 1)` keeps it). Extending the other three operations' "skip and continue"
fix to `del()` would delete *more* than yq does, so `del()` stays on its own rule. A genuine
error in any target still wins.

The order rule applies when every target is plain navigation or a parentless value, judged from
its syntax (`del_target_shape`): a literal, a constructor, or a pipe ending in one. A lone
parentless target therefore deletes nothing, as in yq (`del(1)`, `del(.a | 1)`, `del(.. | 1)`).
A pipe of navigation that ends in `tostring`, `length`, `not`, `@json`, `tojson` or `type` deletes the navigation, as
yq's derived value keeps its source's slot (`del(.b | tostring)` deletes `.b`, `del(.arr[] |
tostring)` empties `.arr`, [#4476](https://github.com/rust-works/succinctly/issues/4476)): those
stages answer every node, so the rewrite cannot hide a type error. Any other target -- arithmetic
(`del(.a, .b + 1)`, an error in yq when the types differ, which the rewrite would turn into a delete),
a derived document root (`del(.. | tostring)` is `{}` in yq: the root has no parent; a comma group with a root member is refused whole), a bound variable
(`del(.a, (.b as $y | $y))`), `select(false)`, a builtin -- still refuses, because succinctly's
"untracked" is wider than yq's "has no path". yq's `$var` keeps node identity, so
`del(.a as $y | $y)` deletes `.a`
([#2643](https://github.com/rust-works/succinctly/issues/2643)); yq's `tojson` and `tostring`
keep their input's path, so `del(.a | tojson)` and `del(.b | tostring)` delete the slot. Ordering
the rest would turn those refusals into silently dropped deletes. `del(.x[], 1)` on `{"x": null}`
vivifies `.x` to `[]` before the loop stops, as yq's matching pass does.

The one untracked target succinctly does skip is a `null`/`true`/`false` literal
identical to its input ([#3207](https://github.com/rust-works/succinctly/issues/3207)).
When every target is one of those, the result matches real yq: `null | del(null)`,
`[null] | .[] |= del(null)` and `map(del(null))` leave their input unchanged. jq's
`jv_identical` rule answers such a literal with the root path `[]` (in jq,
`null | path(null)` is `[[]]`), and that rule is gated to jq mode. In yq mode the root
path used to reach yq's bare-`del(.)` rule, which printed nothing. A mix with a tracked
target follows the same back-to-front rule: `null | del(., null)` keeps the document and
`null | del(null, .)` prints nothing.

A `del()` target that is a bound variable holding the root keeps the document
([#3244](https://github.com/rust-works/succinctly/issues/3244)): `. as $y | del($y)`,
`del(. as $y | $y)`, `del(($y))`, `del($y | .)` and `del(. as $y | ($y, $y))` print the input
unchanged, as yq does (`{"a":1}` on `a: 1`; `true` and `null` on those scalars), where the
bare `del(.)` prints nothing. The check is syntactic -- the target is a variable and nothing
else -- because the resolved branch does not remember how it reached the root. So a spelling
that reaches the root variable through anything but identities still empties the document:
`. as $y | del($y // .)` and `. as $y | del($y | select(true))` print nothing where yq prints it.
What stays
different, all of it yq's node identity ([#2643](https://github.com/rust-works/succinctly/issues/2643)):
a variable bound below the root (`.a as $y | del($y)` deletes `.a` in yq, refuses here),
and the mixed orders `del(., $y)` / `del(.a, $y)` (yq keeps the document, succinctly empties it): a bound variable
is tracked here, so it does not end the loop the way yq's parentless node does.

A `null`/`true`/`false` literal identical to the input that sits in a `del()` the order rule
does not cover (a derived or bound sibling, so the group refuses) is still skipped only when
*every* target is one ([#3207](https://github.com/rust-works/succinctly/issues/3207)). Every
other literal is parentless and ordered by the rule above: `true | del(null)`,
`{a: {b: 1}} | .a.b |= del(null)` and `[null, 1] | map(del(null))` leave their input unchanged.

### A write whose left side steps into a value that is not a path is a no-op ([#4360](https://github.com/rust-works/succinctly/issues/4360)); a bare computed `del()` target still refuses

The #1764 rule above covers a branch that *ends* on a computed value. A branch that *steps into*
one -- an index or a field applied to a sum, a constructor or a builtin's result -- raised jq's
`Invalid path expression near attempt to access element ...` in yq mode, where yq finds nothing to
write through and returns the document unchanged, exit 0 (`=`, `|=`, `+=` and `del()`; jq mode
raises, as jq does). Against yq v4.53.3 on `{"a":{"b":1},"s":"xyz","n":5,"l":[1,2]}`:

```bash
$ yq -p=json -o=json -I=0 '(.l + [3])[0] = 5'          # {"a":{"b":1},"s":"xyz","n":5,"l":[1,2]}
$ yq -p=json -o=json -I=0 '[.n][0] |= 5'               # unchanged
$ yq -p=json -o=json -I=0 'del({"q": .n}.q)'           # unchanged
$ yq -p=json -o=json -I=0 '(.a, [.n][0], .s) = 7'      # {"a":7,"s":7,"n":5,"l":[1,2]}
$ yq -p=json -o=json -I=0 'del([.n][0], .n)'           # {"a":{"b":1},"s":"xyz","l":[1,2]}
```

The members of a top-level comma are resolved one by one once one of them fails this way, so the
later ones still take the write or the delete. The conversion is exact or absent: it applies only
to a target built from constructs yq's own grammar accepts (path steps, literals, constructors,
arithmetic, `length`, `keys`, `to_entries`, `tostring`) in which no member fans out (a comma, `.[]`
or a slice inside one). A `foreach`, `reduce`, `try`, `getpath` or `as` inside the target is a
jq-only construct with no yq oracle, and a member that fans out may have resolved some targets
before the failing step while yq writes all the others, so both keep the refusal rather than a
silent partial write (pinned by `test_yq_mode_getpath_register_unchanged_2896` and its siblings).
Still refused: `(.m[] | (.x, [.x][0])) = 9` and `del(.a, (.s, [.n][0]))` (a nested comma or an
iterate over a failing step: yq writes every other target), a dynamic index or a `?` on the
failing step (`(.l + [3])[.n] = 5`, `(.l + [3])[0]? = 5`), a bare computed `del()` target --
`del((1 + 2))` is a no-op in yq, `del(.s + "x")` deletes `.s` there (yq reads the sum as its left
operand) -- and, on the CLI's cursor route, a variable-bound target (`.a as $y | .a | ($y.b) = 9`:
the eager evaluator and the library entry answer yq's unchanged document, the cursor route still
refuses); the `//=` spelling is not yq's at all.
Pinned in `test_write_through_a_computed_left_side_is_a_noop_4360`.

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

**Fixed for a field/index step by [#3039](https://github.com/rust-works/succinctly/issues/3039):**
yq's `traverse` ends `default: return list.New(), nil`, so a `Field`/`Index` step into a
string, number or boolean yields no candidate. The static-tail navigator
(`navigate_static_component_ref`) now prunes that branch in yq mode, so
`(.a.b, .x.y) = 99` on `a: 5, x: {}` writes `.x.y` and `del(.c, .a[0])` on `a: ab` keeps its
`.c` deletion. `null` still autovivifies, a container of the wrong kind still raises, and jq
mode still raises (jq 1.7.1 does). Still open: an *iterate* step into a scalar
(`del(.c, .a[])` on `a: ab`), which `resolve_iterate_sink` deliberately leaves raising
([#2376](https://github.com/rust-works/succinctly/issues/2376)).

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
is classified by what it would write into
([#1921](https://github.com/rust-works/succinctly/issues/1921)): over an empty container, a
scalar or `Null` the RHS is never read, as in yq, and over a non-empty one it still is. The
comma form `(.a[], .b[]) = v` and a `select(...)` member (`(.a[] | select(. == 1)) = v`)
take the dynamic-path route; since
[#4401](https://github.com/rust-works/succinctly/issues/4401) a comma holding a static member
resolves member by member in yq mode (so `.b[]` over an absent `.b` is the static path a lone
`.b[] = v` is: vivify `b: []`, write into zero elements, never read an erroring RHS), and
`PATH[] | select(...)` over an absent, `null` or scalar `PATH` is that static `PATH[]` (absent,
`null`) or no target at all (a scalar). Still open: `(.a[] | select(. == 1)) = error("boom")` over
a *scalar* `.a`, which evaluates the RHS where yq does not, and a comma whose members are pipes
ending in a `select` over nothing (`(.a[] | select(. == 1), .b[] | select(. == 2)) = 7` raises
`Cannot iterate over null` where yq writes `.a` and vivifies `b: []`). An index past the end of an
array, or into `null`, before the `Iterate` pads like yq (`.a[3][] = v` on `a: []`). Plain `|=` doesn't share this
mechanism at all and turned out to have a separate, more severe divergence (a hard error
instead of missing autovivification) — see
[#1919](https://github.com/rust-works/succinctly/issues/1919).

### An array read or written by a string key ([#4274](https://github.com/rust-works/succinctly/issues/4274), [#4404](https://github.com/rust-works/succinctly/issues/4404)); a float-spelled key and a mapping's numeric `del` remain

yq reads a string key against an array as the integer it spells (`parseInt64`, the reader `pick`
uses, #4257): `[10,20,30] | .["1"]` is `20`, `.["-1"]` is `30` (a negative index wraps), `.["01"]`,
`.["+1"]`, `.["0x1"]` and `.["0o1"]` are `20`, and `.["1_0"]` (index 10) and `.["5"]` are `null`. A
string that is not an integer (`" 1"`, `"x"`, `"1.0"`, one past `i64`) is an error. This build reads
all of that the same way for every read route -- a literal key, `.[$k]`, `.[("1")]`, a computed array
(`(. + [])["1"]`), the slice of an array, and under `?` -- through `eval::yq_array_string_key_index`
and `eval::yq_parse_int64`. jq mode keeps raising `Cannot index array with string`, as jq does.

**The write side (#4404)** follows the read: `.["1"] = 9`, `|=`, `+=`, `del(.["1"])`, a comma group
(`(.["1"], .["2"]) = 0`, `del(.["1"], .["0"])`), a computed key (`"1" as $k | .[$k] = 9`), a
mid-chain step (`.a["1"].x = 9`) and a metadata write all address the element the integer names, on
JSON and YAML input alike; a string that is no integer keeps the ordinary error.

**Not matched:**

- **A float-spelled integral number key.** `.[1.0]` indexes here; yq raises `cannot index array with
  '1.0'` for a literal or YAML-sourced one but indexes a JSON-sourced `1.0`, and the two cannot be told
  apart in the owned value. A *fractional* key (`.[1.5]`, `.[1/3]`, `.[1.50]`) is refused (#4749) on a
  read, a write and a `del()`, `?` suppressing a read, and so is a computed float of `1e6` or more (`.[1.0*1000000]`,
  where Go's `%g` writes `1e+06`); `.[3/3]` and `.[1.0*999999]` still index. A `null`, mapping or scalar
  target is not an array and keeps its own rule. jq mode truncates, as jq does.
- **The wording of the refusal.** yq's is `cannot index array with ' 1' (strconv.ParseInt: parsing
  " 1": invalid syntax)`; this build keeps its own `Cannot index array with string " 1"`, and
  `Cannot index array with number` for a fractional key.
  The constructors are shared with jq mode, so a key of 30 bytes or more loses its text here too
  (`Cannot index array with string`, #5817), as it does in jq; neither wording is yq's.
- **A mapping indexed by a number**, `{"0":"z"} | del(.[0])` (and `a: {"1": x}` | `del(.a[1])`), which yq
  deletes when the key is a string and this build leaves: a `1:` key is an `!!int`, which yq's delete leaves
  alone (#2353), and the value tree does not keep which spelling a key had.

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
- **An `Index` step mid-chain hits a real `Object`.** Resolved by
  [#4079](https://github.com/rust-works/succinctly/issues/4079): yq finds a mapping's member by the
  *text* of a numeric, boolean or `null` index (`.[1]` is the `1:` member, an absent one is created
  as the key `1`), and the write walkers now do the same, so `.a[0][].b = 9` on `a: {}` is
  `{"a":{"0":[]}}` as in yq. What is left of the example is the right side, which succinctly still
  evaluates when the write has no targets:
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

**The *read* side of the third bullet is resolved ([#2459](https://github.com/rust-works/succinctly/issues/2459)),
and so is the write side ([#4079](https://github.com/rust-works/succinctly/issues/4079)):**
a numeric index landing on a real `Object` -- `.a[5]` on `a: {b: 1}` -- answers `null` in yq
mode when no member has that key text, matching real yq's own read (confirmed live: `.a[5]` is `null`,
`.a[5] | key` is `5`, `.a[5] | path` is `["a",5]`), via `eval::yq_numeric_index_on_object_is_null`;
since #4079 it answers the member whose key text is the index when there is one (`.[1]` on
`1: y` is `"y"`, `.[true]`, `.[null]` and `.[1.5]` likewise), and `.a[5] = 1` inserts the string
key as yq does (`{"a":{"b":1,"5":1}}`) instead of raising `Cannot index object with number`.
A float literal keeps its spelling (`.[1.0]` finds the `1.0:` member and `.[1.9]` the `1.9:` one, as
yq does), but `.[01]` is parsed as the integer `1`, so it finds -- and a write updates -- the `1:`
member where yq answers the `01:` one (the recorded lost-spelling residual).
The matched member is named by its key's own value, as a name is (#2801, #2435): `.a[5] | key` and `.a[5] | path` answer the string `"5"` for a quoted key (`"5": five`) and the number for a plain one (`7: seven`), as yq does, and the index when no member has that text.
A mapping a filter built keeps no key types, so the same index into `(.a | . + {})` still answers the number where yq answers the string.
The same holds for a mapping read inside `map(...)` (`[.a] | map(.[5] | key)`) and one bound after a pipe (`.a | .[5] as $v | $v | key`).

**Grouped and multi-target writes follow the same rule
([#4085](https://github.com/rust-works/succinctly/issues/4085)).** The eager path walker
(`navigate_static_component_ref`) reads a numeric index on a mapping by its key text, so
`(.a[1], .a[2]) = 7`, `(.a[1], .a.x) |= "q"`, `(.a[1] // .z) = 3`, `(.a | select(.[1] == "y") | .[1]) = 1`
and `(.a[1], .l[0]) = 3` update the members as in yq, and `del()` follows yq's two-sided rule: a
*terminal* index on a mapping deletes nothing (`del(.a[1], .l[0])` leaves `1:` and deletes `.l[0]`,
#2353), while a continuation walks into the member (`del(.a[1].b)` and `del(.a[1].b, .l[0])` delete
`b` from the `1:` member), through `delete_path_steps` for one path and `delete_trie_array` for a
comma-grouped set. A grouped `del()` carrying a float-spelled index keeps raising
(`del(.a[1.0].b, .a[1].c)`): the trie's edge keeps the truncated index, not its spelling, so it
cannot tell the `1.0:` member from `1:`, and refusing beats deleting from the wrong one. Two
residues are shared with string keys and not specific to this issue: `(.a[9] // .z) = 3` does not
create the `9:` key that yq's `//` leaves behind as `null`, and a grouped `del()` continuing past a
scalar (`del(.a[1].b[0], .l[0])`, `del(.a.x.y[0], .l[0])`) raises `Cannot index number with number`
where the single-path form and yq no-op.

**A boolean or `null` index into an absent container ([#4087](https://github.com/rust-works/succinctly/issues/4087)).**
yq reads `.a[true]` and `.a[null]` on a document with no `a` as `null`, and a write builds a
*mapping* keyed by the key's text (`.a[true] = 1` is `a: {true: 1}`, `.a[null] |= 5` is
`a: {null: 5}`, `.a[true][0] = 1` is `a: {true: [1]}`; a numeric key builds an array), where
`Cannot index null with boolean` was raised. `eval::yq_bool_null_key_into_null` is the one
predicate behind the eager and cursor reads and the path component; jq mode keeps raising. One
residue: a `del()` through such a key on an *explicit* `null` (`d: ~` / `del(.d[true])`, or a
`null` root) vivifies it to `{}` in yq, as it does to `[null]` for a numeric key (#2323), and leaves
it `null` here. A boolean or `null` key on a
scalar target takes the same no-op a numeric key does (`a: 5` / `.a[true] = 1` is unchanged). A
computed key that is `null` only because its source field is absent (`.a[.b]` with no `b`) is not
told apart from a literal `null`: it reads as `null` and writes `a: {null: 1}`, where yq answers
`[]` for the write; this is the key-expression-context difference that already made `.x[.b]` read
`null` here and the member in yq, and it is pinned by
`test_yq_computed_null_key_from_an_absent_field_is_a_known_residual_4087`.

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

A RHS that *itself* errors partway through (`.x = (1, error("boom"), 3)`) aborts with the
error and **no document**, as in yq, which materialises the whole RHS list before the first
write (`resultsForRHS`) — captured across single and multiple LHS paths and every error
position, and fixed for `=` and the compound forms. `|=` does the same
([#1779](https://github.com/rust-works/succinctly/issues/1779)): yq evaluates the whole
filter before taking its first output, so `.x |= (1, error("boom"))` raises `boom` here too,
where jq's `_modify` never looks past the `1` (`jq` mode is unchanged).

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
completion, gated on `S::TAG == EvalTag::Yq` — and, as for `=` above, an RHS that itself
errors partway through aborts with no document (#1779).
`/=`/`%=`/`//=` have no real yq syntax at all (confirmed live, `'/'`/`'//'` expect 2 args but there is 1).
They are refused in yq mode without `--jq-extensions` (#6981), and with it their yq-mode "take the last value" answer is judged by internal consistency with `|=` rather than an external-compat claim.

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
the instant a materializing route (`--arg`, `-P`, `[.]`) builds an `IndexMap<String, _>`
keyed by that string -- the two are never actually the same key (#1385's "never a
duplicate" rule again), but a plain string-keyed map cannot hold both entries under `""`.
Rather than resurrect the silent-overwrite bug this whole effort exists to close,
`DisplayKeyGuard` ([src/jq/document.rs](../../../src/jq/document.rs)) makes that specific
collision raise instead:

```console
$ printf '"a\qb": 1\n"c\qd": 2\n' | succinctly yq --arg z y '.'
Error: object key "" is ambiguous: a complex or undecodable key's display form collides
with another key of the same name and cannot be represented
```

An *ordinary* repeated key (no decode failure on either side) is unaffected: since #1344 it keeps every entry, as yq does.

**Every materializing route also catches a second, wider trigger: a complex key**
([#1749](https://github.com/rust-works/succinctly/issues/1749) for
`--slurp`/`--eval-all`/`--inplace`, [#2519](https://github.com/rust-works/succinctly/issues/2519)
for everything else — writes, `-P`, `--arg`, `tojson`, `map_values`, `sort_keys`, `. * {}`,
`[.]`, `==` and the rest). `resolve_display_key` asks `DocumentValue::display_key_kind`, which
`YamlValue` overrides with `YamlValue::key_string_kind`: it also flags a **complex** key
(mapping, sequence, `null`, or a non-scalar/dangling alias -- not just a decode-failure
string) as a fallback spelling. Real yq keeps both entries in this case too (its
underlying representation isn't a plain map); succinctly's `OwnedValue::Object`
structurally cannot, so this raises rather than silently discarding one:

```console
$ printf '? [1,2]\n: a\n? [3,4]\n: b\n' | succinctly yq --slurp '.[0]'
Error: object key "" is ambiguous: a complex or undecodable key's display form collides
with another key of the same name and cannot be represented
```

**Which routes keep both entries instead.** A route that never builds a display-keyed map
streams the mapping with both entries, under #222's `""` spelling: `.`, `-o json '.'`, `keys`,
`to_entries`, `length`, `has`, and `select(.)`. `select`/`if`'s own validation walk
deliberately keeps the narrower decode-failure-only flag so it does not refuse what yq itself
streams. This is an [ADR-0018](../../adrs/adr-0018.md) rule-4(b) divergence on the
materializing routes only: yq keeps both keys there too, and a duplicate-capable object
([#1344](https://github.com/rust-works/succinctly/issues/1344)) holds the entries of an *ordinary* repeated key but not a pair of
keys that share one fallback spelling (the guard still refuses those).

**A collection key is written as the key node on the cursor route (#6496).**
`.` and navigation over a mapping with a key that is a sequence, a mapping or a flow collection write `? - p` / `: v` as yq does; they wrote `"": v` before.
An empty `[]` or `{}` key is written `[]: v`, a simple key.
A collection key under a compact sequence entry starts its value, and its own first line, at the column its continuation lines stand at (`: - v` under `-I=4`), and on a line of its own when there is no room for a blank (`-I=3`), as yq does ([#6738](https://github.com/rust-works/succinctly/issues/6738)).
A value's comment on the `:` line of a collection key moves to the end of its text in yq (`: x # c`) and is dropped here.
A comment on the `?` line above a block collection key (`? # c` / `  - p`) is written at the end of the key's first line in yq (`? - p # c`) and is dropped here.
A flow mapping that holds a collection key (`{[a]: v}`) still writes `{"": v}` where yq writes `{? [a] : v}`: the flow writer has no explicit-key form for it.
The routes that build an owned value still write the key as `""`, since the key is a string by then ([#6547](https://github.com/rust-works/succinctly/issues/6547)): `-P`, `keys`, `to_entries`, `sort_keys(.)`, `. as $x | $x` and a write.
`[.]` is one of them only for a document with a comment or an alias; otherwise it is an array of nodes (next paragraph) and writes the key node.

**An array of the document's own nodes is written from their cursors (#3475).**
`[.users[] | select(.age > 30)]`, `[.[]]`, `[.users]` and `.users | [.[]]` print each node from the source, as `.`, `.[]` and `map(.)` do.
They copied the whole sequence into an owned value and a comment tree first: a 100 MB file peaked at 2,350 MB and now peaks at 262 MB (AMD Ryzen 9 7950X).
The body is navigation to one node, or navigation that fans out with `.[]` and goes on only through `.`, `.[]` and `select(...)`.
A stage that computes, a `.a` after a fan-out (which can be missing, and an owned `null` beside nodes is written in the owned route's spelling), a slice and a `,` keep the owned route, and so does any run with `--sort-keys`.
A YAML document with an alias keeps it too: the streamers write an `*alias` whose `&anchor` the array does not hold, which the owned route's soundness pass expands.
A comment does not keep an array off the streamers.
The comments between a sequence's items (#6736) are the streamers' own.
So is a document's own foot comment, which `[.]` holds as its element.
It is written after a blank line when the document is a collection and the foot belongs to the document, with the last item when it belongs to that, and directly under a scalar document, as yq does.
`--slurp` writes documents as elements too and writes no foot comment, as before.
JSON output has no comment or alias to lose, so it always streams.
The nodes are read for the first time while they stream, so each is validated before the first byte: a scalar that fails to decode, a complex key that collides with another, or a node past the depth limit prints nothing and exits 1, as it did.
The route writes each scalar as `.` does, not as the owned route's encoder does.
A plain scalar yq quotes (`- a: ---`, `...`, a lone `-`) is written as it was spelled, and a multi-line plain scalar is a double-quoted string (`"a\nb"`) where yq writes a `|-` block; the same input already printed so through `.`, `.users` and `map(.)`.
A collection key whose value is a block collection, inside a compact sequence entry with `-I=3` or a larger step, pads the value's `:` line as the previous paragraph describes, which the owned route (it wrote `'': ...`) did not reach.

**A streamed sequence keeps the comments between its items (#6736).**
`map(.)`, `sort`, `sort_by`, `reverse`, `unique`, `select` over `.[]` and an array of nodes write each item's head and foot comments, as the sequence arm of `.` does: `- a` / `# c` / `- b` prints its `# c`, a foot comment is followed by a blank line when another item follows, and the comment on a bare `-` (`- # on dash`) stays on the dash.
Before, only the owned route kept them.
What is still not written is pinned by `test_what_a_streamed_sequence_still_does_not_write_6736`; all but the third was never written by a streamer, and the third was written by the owned route for an array of nodes (`[.[0][]]`), which streams over a commented document now:
The document's own foot comment after a sorted or reordered root sequence (`# foot` after `sort`).
The trailing foot comment after a `reverse` or `sort_by` of a sequence of mappings (`- k: 3` / `# c` / `- k: 1` / `# e` drops `# e`; a sequence of scalars keeps it).
The comment on a bare `-` that holds a nested sequence, when the inner elements are streamed on their own (`- # c0` / `  - 1`, then `.[0][]`).
The comments of a chain of two reordering stages (`sort_by(.id) | reverse`), whose second stage is handed owned items.

**A top-level comma of navigation reads such a document the way `.` does (#5283, answering #2645).**
Real yq rejects the document for every filter, as it rejects a single undecodable key, so this is the same divergence as `.`, not a new one.
The materializing routes raise because they must build a map; a comma of navigation builds none, so `.,.` and `.k, .` echo both entries under `""` where they used to raise.
That is the jq-side rule (#2103) applied to the one yq shape that was inconsistent with it: a filter validates what it reads, and one that materializes validates what it materializes.
Four rows moved from matching yq's refusal to echoing (`.,.`, `., .a`, `.b, .`, `.., .a` on `"\ud800": 1 / "\ud800": 2`); `[.]`, `. as $x | $x` and the other materializing filters still raise.

**Residuals.** `with_entries(.)` and `to_entries | from_entries` still merge on stdout
(`'': v2`, exit 0): `to_entries` has already turned both keys into the plain string `""`, so
`from_entries` sees an ordinary repeated key and no fallback flag survives to catch it. `-i`
refuses these (its `validate_yaml_display_keys` pre-walk stays for exactly this case). The
`load()` builtin's YAML-mapping conversion is the same walk since
[#2664](https://github.com/rust-works/succinctly/issues/2664): it spells a complex key `""`
(yq: `load("bk.yaml")` => `{"":"value"}`) and raises on a colliding pair like every route above.
What remains of [#1753](https://github.com/rust-works/succinctly/issues/1753) is `key_hash_of`'s
jq-mode collapse.

**Merge keys (`<<`) keep a merged-in complex key beside a local one
([#3467](https://github.com/rust-works/succinctly/issues/3467)).** Merge resolution overrides
by key spelling, and a complex key is spelled `""`, so a merged-in `? [1]` and a local `? [2]`
(or a genuine `"": z`, or two merge sources) used to collapse into one entry before any map was
built, and even `.b.z = 1` dropped the merged one. A fallback spelling is never a duplicate
there now, so the merged mapping keeps both and behaves like the plain collision above: the
streaming routes show both under `""`, and every materializing route (including a write
elsewhere in the document, `.a.k = 1`, which materializes `b` too) raises the same
`object key "" is ambiguous` error. Ordinary keys and a genuine `""` key still override by
name.

What is the *same key* still overrides, byte for byte as before. A key is looked up by identity,
not by its shared `""` spelling:

- a sequence key is compared by content, after scalar type resolution (so `[0x1]` and `[1]`, or
  `[True]` and `[true]`, are one key, while `[1]` and `[1.0]` or `["1"]` are two), so a merged-in
  `? [1]` and a local `? [1]` are one key (the local one wins), and so are two sources that both
  define it, `? []` against `? []`, a nested or block-style sequence, and an alias used as a key
  (`*k : v`) against another use of the same anchor, or an alias naming a scalar (`*n : v` with
  `n: &n 5`) against another naming an equal one. This applies only when the key is a sequence
  of scalars, nested sequences and aliases to those, every scalar decodes, and it is small:
  more than 64 rendered nodes or 16 levels stops the comparison (an alias hop is free, only the
  node it names counts), which also keeps an alias fan-out (`l1: &l1 [*l0, ... x12]`,
  `l2: &l2 [*l1, ... x12]`, ...) from expanding to 12^n nodes. That cap is a cliff by design:
  an equal key written twice is one key within it and two entries past it;
- anything else is the same key only if it is the same node, or the node an alias key names,
  which covers a source mapping reached more than once (`<<: [*a, *a]`) and two uses of one
  anchor naming a mapping (`*m : v`).

Keys that merely look alike therefore stay two entries, and the materializing routes refuse
them instead of keeping one: `[1]` against `["1"]` (a number and a string), `[]` against `[1]`,
two different anchors, and two different sequences. So do two cases that used to collapse into
one entry and did not need to be: **two equal mappings used as keys in different nodes**
(`? {p: 1}` merged-in and local), and a mapping nested in a sequence key (`? [{p: 1}]`). A
mapping used as a key, at any depth, is compared by node only, because its cursor yields no
fields, so there is nothing reliable to tell two of them apart by. The same holds for the other
fallback-spelled keys (an undecodable string). That is the refusing side of
[ADR-0018](../../adrs/adr-0018.md)'s rule 4.

For reference, real yq's own expansion of a merge (`explode(.) | .b`, v4.53.3) collapses every
merged-in complex key against any local one, even distinct ones: `[1]` merged in and `[2]`
local leaves the local entry alone, because yq matches keys by their text and a complex key has
none. A yq *write* never expands a merge (it keeps `<<: *a` as written), so it loses nothing;
succinctly's writes materialize the expansion, which is why this entry exists. succinctly keeps
both entries on the streaming routes and refuses on the materializing ones, rather than copy
yq's expansion into the user's file. How yq's key-list builtins treat a merge key at all is
the next entry ([#3556](https://github.com/rust-works/succinctly/issues/3556)).

**`keys`, `length` and `has` count the `<<` merge key itself
([#3556](https://github.com/rust-works/succinctly/issues/3556)).** yq (v4.53.3, without
`--yaml-fix-merge-anchor-to-spec`) splices a merge for value reads (`.p`, `.[]`, `map_values`,
the JSON rendering of the mapping) but lists the mapping's own entries everywhere a key list is
built, `<<` included. With `base: &base {p: 1, q: 2}` and `item: {<<: *base, r: 3}`:
`.item | keys` is `["<<","r"]`, `length` is `2`, `has("p")` is `false` and `has("<<")` is
`true`. `succinctly yq` does the same on every route that holds the mapping as a document
node, in any of the five merge shapes (one source, a sequence of sources, an own key written
around the merge key, an inline mapping source, a source that itself merges). It is a rule of
the mode, so `succinctly jq` over a YAML document keeps the merged view
(`["p","q","r"]`, `3`). The listing is `DocumentValue::as_object_listed`; `find`, iteration and
the value reads still use the merged view.

**Iterating a merged mapping and rendering it are two expansions in yq
([#4983](https://github.com/rust-works/succinctly/issues/4983)).** With `a: &a {x: 1, z: 9}` and
`b: &b {y: 2, z: 8}`, yq renders `item: {<<: [*a, *b], k: 0}` as `{"y":2,"z":9,"x":1,"k":0}` (a
sequence of sources folded in reverse, so the earlier source wins a shared key) but iterates it
(`.[]`, `.[] | key`, `map(.)`) in listed order with each field upserting in place: keys
`x, z, y, k`, values `1, 8, 2, 0` (the later source wins `z`, as `.item.z` already answered).
An own key written after the `<<` overrides the merged value in place and one written before it is
overridden by it. `succinctly yq` matches both since #4983 (`MergeView::Rendering` for the JSON writers and
the owned conversion, `MergeView::Iteration` for everything else that reads the fields), over the
13 merge shapes in `tests/yq_cli/i4983.rs`.
A mapping with one source reads the same either way, unless an own key written after the `<<` replaces a member the source has (#6851).
There yq's rendering lists the merged members that key does not replace and then the key (`<<: *g` / `a: 9` over `{a: 1, b: 2, c: 3}` renders `{"b":2,"c":3,"a":9}` and iterates `a, b, c`), and so does `succinctly yq`.
A `<<` with several sources and an own key over one of them keeps the fold's order and differs: `<<: [*g, *h]` / `d: 5` / `a: 4` renders `{"c":3,"d":5,"b":2,"a":4}` in yq, where `d` (from `*h`) stays in place and `a` (from `*g`) moves.
The
layer is shared with `succinctly jq` over YAML, which therefore iterates a sequence merge the way yq
does (and answers `.item.z` the way it already did).
yq's JSON rendering leaves an *inline-mapping* source out (`<<: {q: 1}` renders `{"k":0}`; so do an anchored inline mapping and `{q: *a}`, while an alias source renders; `.item.q` and `.[]` still see them).
`succinctly yq` matches that since [#4987](https://github.com/rust-works/succinctly/issues/4987): the yq runner sets `YamlIndex::render_merges_through_aliases_only` on every index it builds.
The library view `succinctly jq` over YAML reads keeps the inline source's keys in the rendering, because no reference tool drops them there.
The rule applies to the direct JSON writers (`-o json` of a document node, `.`, `.item`, a nested value), not to a value the evaluator materializes first.
`[.item]`, `{a: .item}`, `.item | tojson`, `. * {}` and a write such as `.item.k = 1` still list the inline source's keys in JSON, because a write expands the merge key and dropping the source there would drop its keys from the document.
`..` walks a mapping's own entries (the `<<` key and its aliases among them) and does not descend into the merged members, as yq does, since [#5000](https://github.com/rust-works/succinctly/issues/5000) over a live node, in a read and in `path(..)`.
A write through `..` behind a path step (`.item | (.. | select(tag == "!!int")) |= 7`) and a mapping a write or construction materialized first still walk the merged members (yq changes only `k` there; [#6718](https://github.com/rust-works/succinctly/issues/6718)).
`path`, `key` and `parent` of a merged member name its anchor's location, as yq does, since [#6723](https://github.com/rust-works/succinctly/issues/6723): `.item.x | path` is `["a","x"]` where it was `["item","x"]`, and a step taken from it carries on from there (`.item.x.p | path` is `["a","x","p"]`).
Only the path-context walk (`path`, `key`, `parent`, `parents`) names it that way.
A write through the same path changes the source's node, as yq does, since [#6785](https://github.com/rust-works/succinctly/issues/6785): `.item.x = 5`, `.item.x |= . + 1`, `del(.item.x)`, `(.item | .[]) |= 7` and `setpath(["item","x"]; 5)` change `a`'s `x`, and so the `x` of every mapping that merges `a`.
A key several sources hold is written in the last one (`.item.z = 7` sets `b.z`) while the mapping keeps showing the first (`item.z` stays `9`), the two listings of #4983.
A write to the source reaches the merging mapping too (`.a.x = 5`, `del(.a.x)`), and a key the source loses shows through from the next source that holds it, in listed order.
A source removed whole (`del(.a)`) leaves the members it merged in place, as in yq, and a mapping that is itself anchored and aliased passes a merged member's new value on to its alias slots.
The library does this by position, as an alias write does (`jq::alias_identity`, ADR-0017): `yaml::collect_merged_members` lists each member with the node a write lands on and the node its value is copied from, a write path that reaches one is redirected to the first, and the mapping's copy follows the second while it is still an untouched copy.
A mapping a write replaced (`.item = {"q": 1} | .item.x = 5`) and an own key keep their own entry.
A write an update body makes below the mapping it runs on (`.list[] |= (.x = 7)`, `with(.item; .x = 4)`, `.item |= del(.x)`) reaches the source's node too, since [#6827](https://github.com/rust-works/succinctly/issues/6827).
The body has a path of its own, so there is no position to redirect from: the update compares each merged member under a target with what it held before, and a member the body changed or deleted moves to the node it is written through, while the mapping's other members follow the source as for any write.
When several mappings' bodies write the same source node, the first one decides what every mapping shows, as in yq (`.list[] |= (.x = .i)` leaves each mapping with the first's `x`).
Only a body that is a chain of writes to paths below its target is looked at (`.x = 7`, `.a.b += 1`, `del(.x)`, `.x = 1 | .y = 2`, with groups and `?`).
Still divergent, and unchanged from #6785 (the mapping's own copy changes and the source stays):
- a body that reads the member it writes (`.list[] |= (.x += 1)`) writes the source once, where yq counts each mapping's turn (`3` for two mappings over `x: 1`, `2` here);
- a body that deletes a key and then writes more (`del(.x) | .x = 3`, `.x = 3 | del(.y)`, also behind a group), where yq's write makes a new own entry;
- an update whose target is a slice (`.list[0:1] |= (.[0].x = 5)`);
- a body that is not a chain of writes: `map(.x = 7)` over the list, `if`, `//` and a function call (`.list |= map(.x = 7)` leaves the source at `1` where yq writes `7`).
A comma whose first branch is a write (`.list[] |= (.x = 7, .y = 1)`) applies every write, as in yq, since [#6854](https://github.com/rust-works/succinctly/issues/6854): see the next paragraph.
A body that replaces the mapping (`. = {"x": 5}`) leaves the sources alone, as in yq, and so does a filter that is not a write (`|= {"x": 5}`).
A comma body of `|=` applies every write in it, as yq does (#6854): yq evaluates the body against the target node itself, an assignment in it changes that node in place, and the update takes the first output, so `.a |= (.x = 7, .y = 1)` sets both `x` and `y`.
The comma is rewritten as `with(PATH; BODY)` already reads its update: the writes run in order on the evolving node and any other branch is evaluated for its errors.
It applies in `succinctly yq` only (jq's `|=` keeps the first output of any body), to a comma whose first branch answers the node itself: an assignment, a delete or `.`, alone, grouped, piped or behind an `as` binding, with the comma in a group, a pipe, behind a binding or ahead of a `//`.
A first branch that may be empty (`select(..)`) counts only while every later branch is a write: `.a |= (select(.x == 2), 5)` is `5` in yq and here, and so is a body that starts with a value (`.a |= (5, .x = 7)`, `.a |= (.x, .y = 5)`).
A comma under a construct that is not yq syntax (`if`, `try`, `reduce`, under `--jq-extensions`) keeps its first output.
A write followed by a read stage (`del(.item.z) | .item.z`) is not an alias-sensitive write, so no table is installed for it, as for an alias.
A source replaced by a mapping with other keys (`.a = {"y": 1}`, `.a |= {"y": 2}`) drops the members it held from the merging mapping, where yq shows the new keys (`item` is `{y: 1, k: 0}`): the table lists the members the document had, so a key the source gains is not followed.
A member whose source is a `<<` mapping written in place, or one reached through a mapping that itself merges (`n2: {<<: *n}` with `n: &n {<<: *b}`), has no node of its own to write and keeps the mapping's own entry.
After `del(.b.z)` or `del(.item.z)` over two sources, yq lists the surviving key last (`y, x, z`) where the mapping keeps its place (`y, z, x`).
The YAML output of a write still prints the merged members expanded, without the `<<` entry, as before.
Two shapes keep the mapping's own path because the walk cannot climb to them: a member of an inline `<<: {q: 1}` (`["item","<<","q"]` in yq, `["item","q"]` here), and the entries behind a merge of a mapping that itself merges (`n2: {<<: *n}` with `n: &n {<<: *b, m: 1}`): `[.n2[] | path]` lists a literal `<<` entry where yq lists `b`'s entries (`[["n2","<<"],["n","m"]]` here, `[["b","x"],["b","w"],["n","m"]]` in yq), though a member named directly (`.n2.x | path`) is placed.
`to_entries`, `with_entries`, `paths` and `tostream` keep the rendering's order.

Still divergent, because the mapping is no longer a document node by the time the builtin runs
and an owned object has no `<<` entry to list:

- `to_entries`, and so `with_entries` and the `to_entries | from_entries` round trip, list the
  merged members. A `<<` key from `to_entries` would have to survive `from_entries` as a merge,
  which a computed `<<` key does not do in yq (`{"<<": {"p": 1}}` drops it from JSON output),
  and the key carries no provenance here.
- `pick(["p"])` matches the merged member (yq: `{}`), and `.. | length` counts the merged
  members of a mapping it reaches.
- A mapping that a write, `+`, `*`, `del`, `sort_keys`, `map_values` or `to_entries | ...`
  materialized first (`.item | del(.r) | keys` is `["<<"]` in yq) lists the merged members.
- The `<<` key of a `keys` result prints as `"<<"` in YAML output; yq prints `!!merge <<`
  (the key node keeps its tag). JSON output is identical.
- A complex key in a listed mapping is spelled `""`, as everywhere else ([#1753](https://github.com/rust-works/succinctly/issues/1753)).

**`-i` refuses a file holding a complex key
([#3463](https://github.com/rust-works/succinctly/issues/3463)).** Every `-i` route wrote a
complex key back under its `""` spelling, so `-i '.'` turned `? [1]\n: v1\nb: 3` into
`"": v1\nb: 3` with exit 0 -- the key gone from the user's own file, where real yq
round-trips it. Emitting the `? key` form is the real fix and is not done; instead `-i` now
exits 1 and leaves the file byte-identical for a YAML-sourced file with a complex key anywhere
in it, one key or a colliding pair (`refuse_yaml_complex_keys_for_inplace`, the same
`display_key_kind` flag as the pre-walk above). This is an
[ADR-0018](../../adrs/adr-0018.md) rule-4 divergence: matching real yq is impossible without
the writer, and the alternative is a silent write that corrupts data. It over-refuses a filter
whose output omits the key (`-i '.b'`, which real yq performs), since proving that needs
per-output provenance. Scalar keys that merely look unusual (`~`, `null`, `1`, `true`, a quoted
`""`) are unaffected.

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
  the seven builtins above, whose wording never names a path.
  Since [#1998](https://github.com/rust-works/succinctly/issues/1998) `sort`, `sort_by`, `reverse` and `shuffle` raise that wording (`reverse` and `shuffle` without `or map`) with the right tag, in yq mode, on every route (`EvalError::yq_node_not_an_array`).
  The path is always `[]`: `builtin_sort`/`builtin_sort_by` receive only a bare value with no path context, so a navigated input (`.a | sort` on `a: 5`) prints `node at path [] ...` where yq prints `node at path [a] ...`.
  Real path text needs path-tracking plumbing threaded into these builtins (it would also fix `tonumber`'s `cannot convert node value [x] at path a.b of tag !!str to number`), which is not done.
- **`.[]`'s own read-side behavior on a scalar**: real yq treats this as a **silent no-op**
  (exit 0, no output) rather than an error at all (`5 | .[]` prints nothing on real yq).
  succinctly raises `cannot_iterate_with` here in yq mode too, matching jq's own `.[]`
  semantics (correct for jq mode, not yq's). Whether this shares one root cause with the seven
  builtins above or needs its own fix is unresolved — bare `Expr::Iterate` has 20+ dispatch
  arms across `eval.rs`'s value/path/assignment-mode evaluators, so scoping this properly
  needs its own investigation.

Both filed together as [#1998](https://github.com/rust-works/succinctly/issues/1998).

**Two more items found reviewing the fix itself, not fixed in that PR (the first was closed by #2005, below):**

- **`any(cond)`/`all(cond)`/`any(gen; cond)`/`all(gen; cond)`** (the predicate-argument forms,
  dispatched from `Builtin::AnyF`/`AllF`/`AnyCond`/`AllCond`) leaked jq's own wording and let a
  mapping through, and real yq's own lexer rejects this syntax entirely regardless of arity
  (`bad expression, please check expression syntax`). *Closed* by gating the syntax behind
  `--jq-extensions` like its neighbour `min_by` ([#2005](https://github.com/rust-works/succinctly/issues/2005),
  `test_yq_any_all_predicate_forms_gated_2005`): without the flag the four spellings are refused at
  parse time, with it they are jq's own surface, whose wording already matches jq 1.7.1 (`5 |
  any(. > 3)` is `Cannot iterate over number (5)`), so no yq-style wording is invented for a
  shape real yq never accepts. Bare `any`/`all` stay native. Real yq's own predicate spelling,
  `any_c(cond)`/`all_c(cond)`, was a separate, unimplemented builtin; it is implemented since
  [#3966](https://github.com/rust-works/succinctly/issues/3966) (see
  [`any_c(f)` / `all_c(f)`](../../reference/yq-language.md#any_cf--all_cf) in the yq language
  reference), with two recorded residuals: a bare `any_c` or an empty `any_c()` is succinctly's
  generic parse error where yq says `'any_c' expects 1 arg but received none` (#2237's carve-out
  for every required-argument builtin), and the second and later arguments of `any_c(f; g; h)`
  are parsed and dropped, so only a user `def any_c(f; g)` of arity two shadows the builtin (a
  `def` of arity three does not). Not specific to these builtins: an
  undefined variable is *no output* in yq (`any_c($nope)` is `false`, `[.[] | $nope]` is `[]`),
  which `succinctly yq` now matches in every position
  ([#3976](https://github.com/rust-works/succinctly/issues/3976),
  [#2981](https://github.com/rust-works/succinctly/issues/2981)); see the unbound-variable entry
  below.
- **`with_entries(f)` rejects numeric keys** that real yq coerces to strings on reassembly
  (`[1,2] | with_entries(.)` succeeds on real yq, errors `Cannot use number (0) as object key`
  on succinctly) — a functional bug on well-formed input, unrelated to this section's
  malformed-input wording fix. Filed as
  [#2006](https://github.com/rust-works/succinctly/issues/2006).

### yq-mode `fromjson` retains the hand-written decoder and its narrow surrogate gate

Before #3032, `Builtin::FromJson` dispatched to the same hand-written parser
(`parse_json_string_value` in `src/jq/eval.rs`) in both modes. Jq-mode `fromjson`
now uses the shared lenient JSON validator and cursor; yq mode retains the
hand-written parser. `tonumber` uses a separate number-conversion path and
calls `parse_complete_json` only to classify non-numeric strings for jq-mode
diagnostics. [#2008](https://github.com/rust-works/succinctly/issues/2008) (a lone
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

That gate was deliberately narrow, and it is superseded: [#2018](https://github.com/rust-works/succinctly/issues/2018)
made yq-mode `fromjson` reject **every** `\uD800`-`\uDFFF` escape before any pairing runs, as go-yaml's
quoted-scalar scanner does -- a lone high, a lone low and a **valid** high+low pair alike -- with yq's
own wording, `yaml: while scanning a quoted scalar at line L[, column C]: line L, column N: found
invalid Unicode character escape code`. `L`/`C` locate the opening quote (`, column C` is left off at
column 1) and `N` the first hex digit of the escape, both counted in characters from the start of the
string, a `\n` starting a new line. jq mode is not on this decoder (#3032) and still pairs surrogates
as real jq does. An earlier PR ([#2013](https://github.com/rust-works/succinctly/issues/2013)) had
already made a lone **high** surrogate an error in both modes, which also stopped two `fromjson`-parsed
object keys differing only by an unpaired high surrogate from silently collapsing.

Yq-mode `fromjson` skips up to two leading byte-order marks ([#5325](https://github.com/rust-works/succinctly/issues/5325)).
Real yq drops one in its reader and go-yaml another, so `[1]` behind one or two U+FEFF marks reads as `[1]`, and a third mark is a character of the text.
Jq mode skips exactly one ([#5312](https://github.com/rust-works/succinctly/issues/5312)).
What stays different is the rest of go-yaml's decoding, which this hand-written parser does not do (yq v4.53.3):
- Trailing content after the first value is ignored (`"[1] 2" | fromjson` is `[1]`); here it is an error.
- Text that is not JSON is decoded as YAML (`"abc"` is the string `abc`, `"a: 1"` is `{"a":1}`, a mark behind whitespace stays in the string); here it is an error.
- Three marks before `[1]` leave the third in the text, so yq answers the string; here it is an error.
- Whitespace alone is `null`; here it is an error.
- An error is worded by go-yaml (`Error: EOF`, `yaml: while parsing a flow sequence ...`); here it is `Invalid numeric literal` or `Expected JSON value`, with the same exit status.

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

### An absent key read inside a read-only context — resolved (#2470); two neighbouring shapes remain

[#2460](https://github.com/rust-works/succinctly/issues/2460) gave yq mode real yq's rule
for a binary operator whose operand produces **zero outputs**, and left two matrix cells
open: an operand that navigated to a **missing key** (`.zzz * 2`, `(.zzz | key) + 1`),
which real yq also treats as empty even though the same read is a `null` node everywhere
else. [#2470](https://github.com/rust-works/succinctly/issues/2470) closed both, together
with the whole assignment-side half of the same mechanism.

The mechanism is real yq's `Context.DontAutoCreate` (`pkg/yqlib/context.go`), traced to
source and re-verified live against v4.53.3. `traverse`/`traverseMap`
(`pkg/yqlib/operator_traverse_path.go`) fabricate a missing child as a real `!!null` node
with its `Key` set — which is what lets `key`/`path` answer for a position that does not
exist — only while the flag is off; with it on the miss contributes no candidate at all,
before `operator_keys.go` is ever reached. Three constructs force it on, and three
pointedly do not:

| construct                  | probe                       | real yq   | read-only? |
|----------------------------|-----------------------------|-----------|------------|
| `=`-family right side      | `.x = (.zzz \| key)`         | `x: null` | yes        |
| arithmetic operand         | `(.zzz \| key) + 1`          | `1`       | yes        |
| `and`/`or` operand         | `(.zzz \| key) and true`     | `false`   | yes        |
| comparison operand         | `(.zzz \| key) == "zzz"`     | `true`    | no         |
| `//`                       | `(.zzz \| key) // 5`         | `"zzz"`   | no         |
| `\|=`'s right side         | `.x \|= (.zzz \| key)`       | `x: "zzz"`| no         |

`assignUpdateOperator` (`pkg/yqlib/operator_assign.go`) is where the last two rows part:
plain `=` runs its right side through `crossFunction(d, context.ReadOnlyClone(), ...)`,
which forces the flag on; `|=` runs it through `context.SingleChildContext(candidate)`,
which inherits the ordinary auto-creating setting.

Implemented once as `jq::eval::yq_read_only_context` (the ambient scope) plus
`jq::eval::yq_absent_key_read_is_empty` (the mode gate), consulted by every navigation
site in both evaluators, and `jq::eval::yq_prepare_assign_targets` for the other half of
the assignment rule: a zero-output right side still emits one document, with the target
path auto-created and never written, so a new target survives as an explicit `null` while
an existing one keeps its old value. Only **key** lookups are covered — real yq
auto-creates through arrays even read-only (`.x = (.n[9] | key)` on `n: [1, 2]` is
`x: 9`), so an out-of-range array index stays the ordinary `null` read. succinctly
answers `x: 9` there but does **not** write yq's padding back into `.n` (`{"n":[1,2],
"x":9}` against yq's ten-element `n`) — a separate, pre-existing read-side gap, unchanged
by #2470 and #2481 alike.

One neighbouring shape was captured alongside and is **not** part of this rule; it is a
separate divergence (four more — ordering comparisons against a real `null`, `|=` with a
zero-output filter, the evaluation *order* of an assignment's two sides, and indexing a
*scalar* with a key — were captured here too but are now resolved: see
[#2483](https://github.com/rust-works/succinctly/issues/2483),
[#2484](https://github.com/rust-works/succinctly/issues/2484) below, the section
immediately after this one, and
[#2482](https://github.com/rust-works/succinctly/issues/2482)):

| filter                                    | input             | real yq        | succinctly                             |
|-------------------------------------------|-------------------|----------------|----------------------------------------|
| `.a \| with_entries(.value = key)`        | `a: {b: 1, e: 2}` | `{"b":0,"e":1}`| `{"b":1,"e":2}`                        |

This row is what `key` reports for an element of a constructed array: real yq answers the
index, succinctly has no path context for a value it built itself, so the `=` right side
is empty and the write is skipped.

**Resolved (#2482):** indexing a *scalar* (string/number/boolean) with any key -- a field
name (`.s.zzz`), a numeric index (`.s[0]`), or a computed key of either kind -- yields
nothing everywhere in real yq, not only in a read-only context (it is what makes
`.s.zzz + 1` also `1`), where succinctly used to raise jq's own `Cannot index <type> with
<key>` in yq mode too. Fixed as `jq::eval::yq_field_index_on_scalar_is_empty`, consulted
by both evaluators at every scalar-target index site; see that predicate's own doc
comment for the full call-site list and `tests/yq_cli_tests.rs`'s
`test_yq_index_scalar_with_key_is_empty_2482` for the full captured matrix. `null` is
unaffected (it keeps its own separate, unconditional-null rule), and a real container
target keeps its own structural error.

#### A bracketed key read off a `+` or `*` result (#4361, #5460) — resolved

The same flag explains `(A + B)["k"]` and `(A * B)["k"]`: the arithmetic operator leaves its result in a context that does not auto-create, so a key the mapping lacks answers nothing where `(A + B).k` and `(A + B) | .["k"]` answer `null`.
It is a property of the bracket spelling directly after the operator, so the parser keeps a string key there as a computed key instead of folding it into a field (as for a `del()` argument, #2126), and both evaluators read it through `index_one_owned_read_of`.
A write through the same spelling (`(A + B)["k"] = 1`) is unchanged: an assignment target is folded back to the field it always was.
A whole-number key (`(A + B)[0]`, `[-1]`, `[1.0]`) is kept as a computed key the same way (#5460), and an object-valued or array-valued key (`(A + B)[.a]`, `[[0]]`) reads nothing on a mapping instead of raising `Cannot index object with object`.
An array head reads as before (`([1,2] + [3])[9]` is `null`, `[-1]` is `3`).
A `del()` argument, an assignment target (`=`, `|=`, the compound operators, `style =`, `tag =`, `anchor =`) and a `with()` path, comma lists of them included, are folded back to the path step they always were, so each stays the no-op it is in yq.
Not matched, all as before: `del((A + B)[.k])` with a computed key, `del((A + B)[0] | select(.))` with the key piped into another stage, and a write through a whole-number key past 2^63 (`(A + B)[9.3e18] = 1`) fail with `Invalid path expression` where yq leaves the document alone.
`(A + B)[.k]` over an array head with a mapping-valued key words its error differently.

### `//` maps per left output and keeps a falsy left value when the right side is empty — resolved ([#2817](https://github.com/rust-works/succinctly/issues/2817)); yq's auto-creating key lookup remains

Real yq v4.53.3's `L // R` is not jq's collect-then-fallback. Captured live when
[#2782](https://github.com/rust-works/succinctly/issues/2782) added `//` to
`scripts/jq-path-context-oracle-sweep.sh`'s alphabet, and implemented for yq mode by
`EvalSemantics::ALTERNATIVE_IS_PER_LEFT_OUTPUT` (jq mode keeps its rule):

| filter (`yq -n -o=json`)            | yq v4.53.3    | jq 1.7.1 / `succinctly jq` |
|-------------------------------------|---------------|----------------------------|
| `(1, null) // 2`                    | `1` `2`       | `1`                        |
| `(null, 1) // 2`                    | `2` `1`       | `1`                        |
| `(null, 1, null) // 2`              | `2` `1` `2`   | `1`                        |
| `(false, null) // (3, 4)`           | `3` `4` `3` `4` | `3` `4`                  |
| `(null, 1) // (3, 4)`               | `3` `4` `1`   | `1`                        |
| `{} \| (null // .[])`               | `null`        | *(nothing)*                |
| `{"a":1} \| (null // key)` (root)   | `null`        | *(nothing)*                |
| `select(false) // 7`                | `7`           | `7`                        |

Each left output is decided on its own: a truthy one passes, a falsy one is replaced *in place*
by every output of `R` -- or stays itself when `R` yields nothing -- and `R` runs on its own only
when `L` yielded nothing at all. `R` is not evaluated while every left output is truthy
(`1 // error("y")` is `1`), a left error raises (`(null, error("x")) // 3`), and the outputs stay
where they stood (`(.a, .b) // .c | key` is `"c"`, `"b"` for a null `.a`; `(.a, .b) //
select(false) | key` is `"a"`, `"b"`). It is implemented on all four routes: the eager value
route, its streaming twin, the cursor route (`each_alternative_per_left_output_generic`) and the
owned identity pipe (`eval_owned_identity_alternative_per_left_output`), and a pipe whose first
stage is such a `//` is driven by the streaming pipe so a document node and a computed
replacement can reach the next stage together (`.a[] // 9 | line`). Checked against yq on 2,590
filter/input rows and 24 position rows, on the cursor and the DOM route.

In path position (`((.a, .b) // .c) = 9`, `del((.a, .b) // .c)`) the rule is the same
(`resolve_alternative_per_left_output_sink`): the truthy `b` and the replacement `c` are written, not `b` alone.

One residual is *not* about `//`: yq's key lookup **auto-creates** a missing key in the tree it
is reading, so `.zz // .[]?` on `{"a":1}` iterates a document that now holds `zz: null` and
prints `1`, `null` (it prints `null`, `1`, `null` for `(.zz, .[]?)` with no `//` at all). The
25 rows of that grid that still differ are all this (`.a.b // .a`, `(.a // .b) = 5` on a
document with no `a`, ...) plus an unrelated `map` over a mapping; and `del((.a, .b) // .zz)`, where
yq deletes the key it just created and loses `a` as well (`{"c":2}`; here `{"a":null,"c":2}`).

yq's update-or-create idiom (`alternative-default-value.md`) is read as yq reads it since #7068: an update whose target is a `//` chain of operands that each name one node by fixed steps (`.a`, `.a.b`, `.l[3]`), ending in an operand that writes, applies that write to the document the update works on and then tests the left operands again.
`(.a // (.a = 0)) += 1` creates `a: 1`, and `(.a // (.c.d = 0)) += 1` adds `1` to the document, the error yq raises.
The parser writes it as the writes, `(L |= .) | ([L] | .[0] | not | not) as $t | select($t | not) |= NEXT` for each left operand, followed by the update of `if L0 then L0 elif ... else . end`, so the update's value is written once and comments, anchors and flow style are kept as for any write (`yq_alternative_target`).
Still differing:

- A chain that ends in anything but a write does not create its left operands: `(.a // .z) += 1` leaves `a: null` beside `z: 1` in yq, and only `z: 1` here; `(.a // "x") = 1` writes `a: null` in yq and nothing here.
- A `//` inside a larger target (`(.a // (.a = 0)).y = 1`, `((.a // .z) | .y) = 1`) keeps the path resolver's reading.
- In value position the write is not seen by the left operand: `.a // (.a = 5)` is the document with `a: 5` here and `5` in yq, which reads `.a` again after the write; yq does so only where its context is editable (a pipe stage), and `(.a // (.a = 0)) * 2` multiplies the document in both.
  An idiom bound as a value (`((.a // (.a = 0)) += 1) as $v | $v`) is the updated document here, where yq, reading the binding's source in a read-only context, raises the error of adding `1` to the document.
- A comma behind a binding in the last operand (`(.a // (.x as $v | .a = 0, .b = 1)) += 1`) targets the left node once here; yq gets it twice, one for each output of the comma, and adds `1` twice (`a: 2`).
- A binding over several values in the last operand (`(.a // (.x[] as $v | .a = $v)) += 10` over `x: [1, 2]`) writes each value in turn since #7094, but targets the left node once (`a: 12`); yq targets the node once per value (`a: 22`).
- A last operand that makes the document `null` (`(.a // (. = null) // .z) += 1`) is followed here to the operands after it (`z: 1`), where yq adds `1` to the null document, an error.
- `with(PATH; SRC as $v | WRITES)` over a source of several values applies the writes of each value in turn to the same node since #7094, as yq does (`with(.k; .x[] as $v | .y += $v)` over `x: [1, 2]` is `y: 3`, and `.y = $v` leaves the last value), and so does a `|=` body that is such a binding (`.k |= (.x[] as $v | .y += $v)`).
  The values are the source's as the node stood before the first write, where yq copies each bound node when its turn comes: a write to a node a later value is bound to is seen by that value in yq and not here (`with(.k; .x[] as $v | .x[] = $v)` over `x: [1, 2]` is `x: [1, 1]` in yq, which copies `x[1]` after the first value rewrote it, and `x: [2, 2]` here).
  A write that removes or replaces the bound node (`del(.x[0])`, `.x = [7, 8]`) agrees, since yq's copy is of the node that was bound.
  Each such value loses its comment and tag on the way (`with(.k; .x[] as $v | .y = $v)` over a block sequence whose last item is `"2" # two` writes `y: "2"` here and `y: "2" # two` in yq), as `[.[]]` does (#5429).
  Each value is one step of a fold whose state is the node, and a yq-mode fold step of this shape re-indexes its state, so a source of many values inside a large node takes time that grows with their product (10,000 values in the node: seconds, where yq takes 0.1 s), as a yq-mode `ireduce` with an UPDATE other than an accumulator does (#7108).

### A pipe's empty context list — a stage that builds a node emits once (#6943)

The rule of the next section is not only an operand's.
In a pipe, a stage that builds a node of its own (a literal, a constructor, an operator over literals, `error("x")`, a union of them) is emitted once when the stage before it matched nothing: `.a[] | 1` over `a: []` is `1` in yq, and so is `select(false) | 1`, `.zz[] | "x"` and `.c.b | {"k": 1}`.
It is the list that is empty, not a node: `.d[] | select(false) | 1` is one `1` however many members `d` has, and `.d[] | 1 | select(false)` is nothing because the `1` was emitted.
The first such stage whose input list is empty is the one that emits, and the stages after it run on what it emitted, so a later stage that follows another empty one emits in its turn (`.d[] | select(false) | 1 | select(false) | 2` is `2`).
A union is evaluated over the empty list branch by branch (`.a[] | (1, 2)` is `1`, `2`), and `map(f)` is `[.[] | f]` to the rule, so `map(select(false) | 1)` is `[1]`.
`succinctly jq` is unchanged: jq's pipe loops zero times over an empty stage.

It is answered where the pipe is evaluated: the sink route counts what the pipe emitted and, when that is nothing, asks which stage's input was empty (the stages before it run once more, only then).
A pipe with outputs pays a counter and nothing else, and the lazy and cursor routes keep their results.
Pinned by `tests/yq_cli/i6943.rs`, whose rows were captured from yq v4.53.3.

Not matched: the right side of a write (`.x = (.a[] | 1)` is `x: 1` in yq and `x: null` here), the body of `map_values` and of `|=` (`.d[] |= (select(false) | 1)`), and a source that is a computed list (`.d | to_entries | map(select(false) | 1)` is `[1]` in yq and `[]` here).
A collection `[...]` after a stage whose input list is empty is `[]` in yq and is answered here only when the whole pipe was empty.
A pipe nested in an object or a call (`{"k": (select(false) | [1])}`) is asked once per node, as before.
A literal bound to a variable (`.a[] | 1 as $x | $x` is `1` in yq and nothing here) is not a re-emitting stage.
Under `--eval-all`, an alternative whose left side is a pipe that re-emits is still evaluated per document: `(.a[] | 1) // 9` over `a: []` and `a: [1]` is `1`, `1` here and `1` in yq.
The library built without `std` has no hand-off between the context-list walk and the rest of a pipe it owns, so there the walk asks the question once per entry of its list.
A pipe whose answer is empty is evaluated up to three times (the route, the count and the probe of the stages before the re-emitting one, then the rewritten pipe), which a pipe with outputs never pays.

### An operand's evaluation context — a context-free stage re-emits against an empty context (#2540, #2588)

A literal or constructor is not really "empty" against a read-only, zero-node context in
real yq the way `.`/`length`/a plain navigational read are: `valueOperator`
(`pkg/yqlib/operator_value.go`) special-cases an empty `context.MatchingNodes` by
re-emitting a copy of the literal node instead of looping zero times, and `[...]`/`{...}`
(`operator_collect.go`/`operator_create_map.go`) carry the identical special case in their
own operators — but not identically to each other. The rule is **per operator, not per
pipe**: the *first* context-free operator restarts the stream from a node of its own, and
every later stage then runs normally on it. Captured live against yq v4.53.3 on
`a: {b: 1}` / `x: 5` (`.a.zz` genuinely absent, not `null`), as `.x = (.a.zz | EXPR)`:

| `EXPR` | produces (not "empty" for #2460's rule) |
|---|---|
| `true` / `5` / `"s"` / `null` / `-1` | itself, unconditionally |
| `[.]` / `[.a]` / `[1,2]` / `[]` | `[]`, **regardless of the array's own body** — it loops zero times over the empty context, but the collected array is still emitted once |
| `{"k": 1}` | `{"k": 1}` — every field's value also independently qualifies |
| `{"k": [1]}` | `{"k": []}` — each field value is itself re-emitted |
| `{"k": .}` | nothing — `.` does not have this special case (`operator_self.go` returns its input context unchanged), so the *whole* object construction aborts, not just that field |
| `{"k": 1, "j": .}` | nothing — one disqualifying field is enough; this is not a per-field union |
| `1 + 1` / `"s" + "t"` / `1 == 1` / `1 // 3` | the operator applied to the re-emitted operands (`2`, `"st"`, `true`, `1`) |
| `(true, false)` / `(1, .b)` / `(.b, 1)` | the re-emitting branches only: `=` keeps the last (`false`, `1`, `1`), `and`/`or` see every one |
| `true \| not` / `1 \| tostring` / `[1] \| length` / `{"q": 1} \| .q` | `false` / `"1"` / `0` / `1` — the first stage re-emits and the tail runs on it |
| `error("x")` / `error("x" + "y")` | raises `x` / `xy` — `errorOperator` ignores its context, so the stage runs and raises (#5261) |
| `.` / `.b` / `length` / `tostring` / `select(true)` / `. // 3` | nothing (propagates, matching the pre-existing #2460 oracle rows for `key`/`parent`) |

Before [#2540](https://github.com/rust-works/succinctly/issues/2540), succinctly propagated
the zero-node emptiness straight through the pipe for every `EXPR` shape, so `and`'s left
operand registered as empty and #2460's own empty-operand rule short-circuited to `false`
without ever consulting the literal/constructor's real value: `(.a.zz | true) and true` was
`false` instead of `true`. #2540 modelled the rule as "the pipe's *last* stage re-emits a
static value", which was too narrow in both directions (`(.a.zz | 1 + 1) and true`,
`(.a.zz | true | not) and true` and `(.a.zz | (true, false)) and true` were still wrong), and
it only reached `and`/`or`: `=`'s right side evaluates through a different mechanism
(`jq::eval::collect_rhs_outputs`), so `.x = (.a.zz | true)` stayed `x: 5` where yq has
`x: true`.

[#2588](https://github.com/rust-works/succinctly/issues/2588) replaces the classifier with
`jq::eval::yq_empty_context_reemit`, which returns the *expression* yq would emit
(a pure `Expr` rewrite: a pipe is classified by its first re-emitting stage and keeps the
tail, an array becomes `[]`, a comma keeps only its re-emitting branches, an operator needs
every operand to re-emit). The rewrite reads nothing from its input, so it is evaluated
wherever the original produced nothing, inside the read-only scope that produced the
emptiness:

- `collect_rhs_outputs` — the shared prologue of `=`, `+=`, `-=`, `*=` and the other
  compound forms, in both evaluators (the generic one prefetches through it);
- the `and`/`or` fanout (`boolean_fanout_each`), both operands, driving **every** bit the
  rewrite yields;
- the arithmetic and comparison fanout, both evaluators
  (`binary_fanout_each_with` and `binary_fanout_each_generic_with`), for an operand of an
  arithmetic expression or any operand under an enclosing read-only scope
  (`.x = ((.a.zz | 1) == 1)`). A bare top-level comparison is not read-only in yq
  (`compareOperator` does not clone its context read-only), so it is left alone.

Gated on yq mode and a read-only scope, so jq mode (where `.a.zz` is a `null`, not zero
nodes) never enters the path. The admitted set is closed (literals, `-`, arithmetic,
comparison, `and`/`or`, `//` (left operand only, the right becomes `empty` when it does not
re-emit), `[...]`, `{...}` with literal keys, `,`, `|`); `Expr::Shared`
is deliberately not unwrapped, and a dynamic object key declines unless it is itself a
literal (anything else would have to be evaluated to be stringified, and raising where yq
emits nothing is worse than staying empty). See `tests/yq_cli_tests.rs`'s
`test_yq_assign_rhs_reemits_from_a_context_free_stage_2588`,
`test_yq_assign_rhs_non_reemitting_stages_stay_empty_2588` and
`test_yq_empty_context_reemit_in_boolean_and_arithmetic_operands_2588` for the captured
matrix, and the #2540 tests for the original `and`/`or` rows.

**Residual, not fixed by #2588** (each reaches the same yq rule through a route that is not
an operand of the shapes above; all checked live against v4.53.3):

- an operand *nested inside a collect*: `.x = ([.a.zz | true])` is `x: [true]` in yq,
  `x: []` here. The re-emission has to happen at the pipe inside `[...]`, which is the
  ordinary pipe evaluator rather than a fanout;
- `|=`: `.x |= (.zz | true)` is `x: true` in yq (its filter runs per matched node through
  `SingleChildContext`, see the end of this section), `x: 5` here;
- `//` with a parenthesised re-emitting left operand: `.x = ((.a.zz | 1) // 3)` is `x: 1`
  in yq and `x: 3` here (`.x = (.a.zz | 1 // 3)` agrees). `//` has its own evaluation
  routes (`eval_alternative_per_left_output` and its cursor, lazy and path twins) that
  do not share the fanout loop;
- a **second restart** after the stream empties again: `.x = (.a.zz | true | select(false) |
  1)` is `x: 1` and `.x = (.a.zz | 1 | .zz | 2)` is `x: 2` in yq (every context-free
  operator restarts an empty stream, not only the first), `x: 5` here. The rewrite keeps
  the tail after the first re-emitting stage verbatim, because whether a later stage's
  input is empty is only known while evaluating;
- `error(.q)` and the bare `error`: yq raises `aborted` for the first, because the message reads its input and is empty, and an arity error (`'|' expects 2 args but there is 1`) for the second; the rewrite declines both, so they stay empty (#5261);
- an operator with **one** re-emitting operand and one that reads its input, which yq
  answers asymmetrically: `.x = (.a.zz | 1 == .b)` (and `!=`, `<`, `>`, `<=`, `>=`) is
  `false`/`true` in yq while `.b == 2` and `1 + .b` are nothing. The operator declines
  (both operands must re-emit), so these keep their pre-#2588 answer (`x: 5`).

Real yq's `assignUpdateOperator` (`pkg/yqlib/operator_assign.go`, v4.53.3) resolves and
auto-creates the **left** side first, then evaluates the right side — through
`ReadOnlyClone`, whose own half of the rule is #2470 above — against the document that
traversal has already mutated. So the right side sees the node the assignment is about to
write into. succinctly used to evaluate the right side against the pristine input;
[#2481](https://github.com/rust-works/succinctly/issues/2481) closed that, and every row
below now agrees with yq v4.53.3:

| filter                    | input  | real yq (and now succinctly) | succinctly before #2481 |
|---------------------------|--------|------------------------------|-------------------------|
| `.x = (keys)`             | `a: 1` | `{"a":1,"x":["a","x"]}`      | `{"a":1,"x":["a"]}`     |
| `.x = (length)`           | `a: 1` | `{"a":1,"x":2}`              | `{"a":1,"x":1}`         |
| `.zzz.q = (.zzz \| key)`   | `a: 1` | `{"a":1,"zzz":{"q":"zzz"}}`  | `{"a":1,"zzz":{"q":null}}` |
| `.zzz.q += (.zzz \| key)`  | `a: 1` | `{"a":1,"zzz":{"q":"zzz"}}`  | `{"a":1,"zzz":{"q":null}}` |
| `.zzz.q -= (.zzz \| key)`  | `a: 1` | `{"a":1,"zzz":{"q":"zzz"}}`  | `{"a":1,"zzz":{"q":null}}` |
| `(.x, .y) = (keys)`       | `a: 1` | both `["a","x","y"]`         | both `["a"]`            |
| `.[2] = (length)`         | `[]`   | `[null,null,3]`              | `[null,null,0]`         |

`.zzz.q = (.zzz | key)` is the row that shows why this had to follow #2470 rather than
precede it: succinctly's old answer matched yq only because its absent read fabricated a
`null` node whose key happened to be spelled the same. Once that read was correctly
empty, the underlying order difference was the only thing left holding the row up.

`|=` is deliberately excluded: its filter runs through
`context.SingleChildContext(candidate)`, per matched node with `.` bound to that node,
not against the document — the same split #2470's table records.

Implemented as `jq::eval::yq_prepare_assign_targets`, one definition shared by
`eval_assign`, `eval_compound_assign` and `eval_alternative_assign`, which also carries
#2470's zero-output-right-side document. Auto-creation is `path |= .` through the
existing `update_path` walk rather than a second hand-written one, so yq's scalar-target
no-op comes along for free (`.a.b = 1` on `a: 1` creates nothing and writes nothing,
matching yq).

Two consequences worth naming:

- yq mode now resolves a **dynamic** left side before the right side too, which is real
  yq's own order (`.[error("p")] = error("r")` reports `"p"`, where jq reports `"r"`), and
  a left side whose every resolved target is a scalar-target no-op, or that resolves to no
  target at all, never evaluates the right side ([#1412](https://github.com/rust-works/succinctly/issues/1412)).
  A left side that **navigates and then filters** and matches nothing answers the document after the steps yq created
  while walking, without reading the right side ([#4593](https://github.com/rust-works/succinctly/issues/4593):
  `yq_filtered_target_creation` runs the same creation walk as the write; `(.b | select(.)) = error("boom")` on `{}` is
  `{"b":null}`, and `(.a[] | select(. > 5)) = error("boom")` on `a: [1, 2]` is `{"a":[1,2]}`; pinned by
  `test_filtered_assignment_target_that_matches_nothing_skips_the_right_side_4593`). Still evaluating the right side: a
  left side that is not a plain navigation followed by `select`s (`.m | to_entries | select(false)`), and an iteration
  over a missing or `null` key (`(.zz[] | select(.)) = v`, where yq turns the key into `[]`).
  Also unchanged: `|=` (`(.zz | select(.)) |= 5` on `{}` is `{}` here and `{"zz":null}` in yq, as the walked steps are not created
  by the update route), and a path that walks through an alias (`(.b.p | select(.)) = 1` on `b: *x` creates `p` on the anchor's
  node in yq; the shortcut declines while an alias table is installed, so the previous route runs).
- A right side that reads the assignment's **own target** still diverges, because
  succinctly has no node identity (the same limitation the anchor/alias section above
  records). Real yq's right-side candidates are pointers into the document, so writing
  through one and then assigning it to itself is a no-op; succinctly collects values and
  applies the last. Live on `x: 5` (v4.53.3):

  | filter                | real yq   | succinctly |
  |-----------------------|-----------|------------|
  | `.x = (1, .x)`        | `{"x":1}` | `{"x":5}`  |
  | `.x = (1, .x, 2, .x)` | `{"x":2}` | `{"x":5}`  |
  | `.x = (.x, 1)`        | `{"x":1}` | `{"x":1}`  |

  Pre-existing on an existing target, and #2481 extends it to a *vivified* one, where
  succinctly used to agree by coincidence: `.x = (1, .x)` on `a: 1` was `x: 1` only
  because the absent `.x` read contributed nothing at all. Now that the target exists,
  the read yields the vivified `null` and last-wins stores it (`x: null`, against yq's
  `x: 1`). Pinned in both shapes in
  `test_yq_assign_rhs_reading_its_own_target_lacks_node_identity_2481`.

### An update filter's evaluation position — resolved (#2522); one `parent` row still diverges

Real yq's `assignUpdateOperator` runs `|=`'s filter through
`context.SingleChildContext(candidate)`, per matched node with `.` bound to that node — so
`key`/`path`/`parent` inside the filter are the **target's**, not the assignment's input's.
(A compound assignment is the other case: it evaluates its right side once, against the
input, which is the `=` rule #2481 above records.) succinctly had no route that named the
target's position, so every read in an update filter answered from nowhere: `key` produced
nothing at all and the target was left untouched, `path` was `[]`.
[#2522](https://github.com/rust-works/succinctly/issues/2522) closed that, and every row
below now agrees with yq v4.53.3 (`-o=json -I0`, on `a: {b: 1, e: 2}` unless noted):

| filter                                     | real yq (and now succinctly)         | succinctly before #2522 |
|--------------------------------------------|--------------------------------------|--------------------------|
| `.a \| .b \|= key`                           | `{"b":"b","e":2}`                    | `{"b":1,"e":2}`          |
| `.a \| .b \|= path`                          | `{"b":["a","b"],"e":2}`              | `{"b":[],"e":2}`         |
| `.a \| .b \|= (path\|join("/"))`              | `{"b":"a/b","e":2}`                  | `{"b":"","e":2}`         |
| `.a.b \|= (path\|length)`                    | `{"a":{"b":2,"e":2}}`                | `{"a":{"b":0,"e":2}}`    |
| `.a \| to_entries \| .[0] \| .value \|= path`  | `{"key":"b","value":["a",0,"value"]}` | `{"key":"b","value":[]}` |
| `.a \| .zzz \|= key`                         | `{"b":1,"e":2,"zzz":"zzz"}`          | `{"b":1,"e":2,"zzz":null}` |
| `.a \| (.b,.e) \|= key`                      | `{"b":"b","e":"e"}`                  | `{"b":1,"e":2}`          |
| `.a \| .b \|= (parent\|keys)`                | `{"b":["b","e"],"e":2}`              | `{"b":1,"e":2}`          |
| `.a \| .zzz \|= (parent\|keys)`              | `{"b":1,"e":2,"zzz":["b","e","zzz"]}` | `{"b":1,"e":2,"zzz":null}` |
| `.a \| .b += key`                           | `{"b":"1a","e":2}`                   | `{"b":1,"e":2}`          |
| `.a \| .b -= (key\|length)`                  | `{"b":0,"e":2}`                      | `{"b":1,"e":2}`          |
| `.n[] \|= key` (on `n: [1, 2]`)             | `{"n":[0,1]}`                        | `{"n":[1,2]}`            |
| `.n[] \|= path` (on `n: [1, 2]`)            | `{"n":[["n",0],["n",1]]}`            | `{"n":[[],[]]}`          |
| `.n[] \|= (key + 10)` (on `n: [1, 2]`)      | `{"n":[10,11]}`                      | `{"n":[10,10]}`          |

`.a | .zzz |= (parent|keys)` is the row that shows `parent` climbs a snapshot with the
whole left side already auto-created: real yq resolves and vivifies its targets before any
filter runs (#2481 above), so the key the write is about to fill is already in the parent
the filter is handed.

Implemented as `jq::eval::UpdatePos`, threaded through `update_path`/`update_path_steps` so
the components the walk takes name the target, plus an ambient prefix
(`jq::eval_generic::path_base`) for where the assignment's *input* itself sits — a
mid-pipe `.a | .b |= path` has to answer `["a","b"]`, not `["b"]`. The rewrite is
`path_context_resolve_constants`, the same one the owned identity pipe applies, so a
position named here means what a position named anywhere else does.

**The rows still divergent** are a bare `parent` written straight into its own target, by
`|=` or by `map_values`:

| filter                       | real yq                                                          | succinctly                                              |
|------------------------------|------------------------------------------------------------------|---------------------------------------------------------|
| `.a \| .b \|= parent`         | `{"b":{"b":{},"e":2},"e":2}`                                     | `{"b":{"b":1,"e":2},"e":2}`                             |
| `.a \| map_values(parent)`    | `{"b":{"b":{},"e":{"b":1,"e":{"b":1}}},"e":{"b":1,"e":{"b":1}}}` | `{"b":{"b":1,"e":2},"e":{"b":1,"e":2}}`                 |

Real yq's `parent` is a *pointer* to the node it is in the middle of mutating, so assigning
it into that node's own child makes the document self-referential and its printer emits the
cycle truncated (`{}` at the second level) — and `map_values` rewrites the container one
member at a time, so its second member's `parent` already holds the first member's
self-reference. succinctly has no node identity (the same limitation the anchor/alias
section above records), so it writes the parent's pre-write value: the container as it was
before the stage ran, for every member. Every `parent` row that *reads* the node rather
than embedding it — `parent|keys`, `parent|type`, `parent|length`, `map_values(parent |
length)` (`{"b":2,"e":2}` in both) — matches. Pinned in
`test_yq_update_filter_reads_the_target_position_2522`; the `map_values` row was `{}` in
succinctly until spine 2416's identity pass ran map-family bodies at each member's own
position (before it, a `parent` inside such a body was refused by every route and answered
from no position at all).

`line`/`column` inside an update filter are **not** covered: they are answered from a
cursor, not from a path, and no cursor reaches the filter. `.a | .b |= line` is `2` in real
yq and `0` in succinctly — a separate, pre-existing gap, not one #2522 touches.

The same gap shows on much of what is downstream of a write, and on `-R` and `--slurp`: those
evaluate a value succinctly has re-indexed into throwaway text, whose positions are not the
user's file's, so the cursor-metadata builtins answer their fixed defaults there (#3479), except
for the field read straight off a kept value (#3641, below).
`--arg` alone does not: a filter that only reads stays on the cursor route (`--arg v x '.c |
line'` is `2`), and it joins the gap only through a write (`--arg v x '.c = $v | .c | line'`
was `0` before #3641). Against yq v4.53.3 on `a: 1` / `c: 2`:

| query                        | yq | succinctly yq    |
|------------------------------|----|------------------|
| `.c \| line`                 | 2  | 2 (cursor route) |
| `.c = 3 \| .c \| line`       | 2  | 2                |
| `.c = 3 \| .a \| line`       | 1  | 1                |
| `.c = 3 \| .c \| column`     | 4  | 4                |
| `.c \|= . + 1 \| .c \| line` | 2  | 2                |
| `.d = 3 \| .d \| line`       | 0  | 0                |
| `.c = 3 \| [.[] \| line]`    | 1, 2 | 0, 0           |

Since [#3641](https://github.com/rust-works/succinctly/issues/3641) the field read straight
off a value that kept its position (`.c = 3 | .c`, `del(.b) | .c`, `sort_keys(.) | .b`) is the
document's own node: its line, column, anchor and line comment survive the write, also where
the write replaced the field's value (`c: &x 2 # t` with `.c = 3`: `.c | anchor` is `x`,
`.c | line_comment` is `t`), and a field the write created (`.d = 3 | .d`), a field `del`
removed and a field of a sum (`. + {"c": 9} | .c`, the right operand's node) have none. `0` is
still deliberate rather than the position in the re-indexed text (`1`), which would look right
and be wrong; `test_dom_route_cursor_metadata_is_positionless_3479` pins it. What remains: the
fields a `.[]` fans out to, a field of a field (`.d.e`, `.c[0]` after a write; yq answers `0` where
a container was overwritten by a container, so it is not safe to read the document's), the
untouched fields of a sum, and a getter nested in a comma, an operand or a body
([#4712](https://github.com/rust-works/succinctly/issues/4712)).

### An `and`/`or` operand's evaluation context -- resolved as precedence ([#2506](https://github.com/rust-works/succinctly/issues/2506)); a literal against an empty context remains a residual gap

Captured under [#2473](https://github.com/rust-works/succinctly/issues/2473) as "real yq
resolves an `and`/`or`'s right operand from the document root". That premise was wrong.
The oracle capture for #2506 settled it with one probe:

```console
$ yq -o=json -I0 '.a | (true and .b)' d.yaml     # true
$ jq -c            '.a | (true and .b)' d.json   # true    <- identical
$ yq -o=json -I0 '(.a | true) and .b'   d.yaml   # false
$ jq -c            '(.a | true) and .b' d.json   # false   <- identical
```

Once the grouping is written out, the two tools agree on every row. The divergence was
**parser precedence**: `pkg/yqlib/operation.go` (v4.53.3) gives `and`/`or` `Precedence: 20`
and `|` `Precedence: 30`, so `.a | true and .b` is `(.a | true) and .b` in yq and
`.a | (true and .b)` in jq -- the right operand reaches the document root only because the
pipe was consumed into the *left* operand. `=`/`==` (40) and `//` (42) also outrank
`and`/`or` in yq where jq ranks them looser, and `and`/`or` share one precedence and chain
right-associatively (yq's shunting yard pops only on strictly greater precedence).

`succinctly yq` now implements that ladder (`parse_yq_boolean_expr`, `src/jq/parser.rs`);
the rows are pinned in `test_and_or_keep_path_context_2473` and
`test_yq_and_or_precedence_2506` (`tests/yq_cli_tests.rs`). No evaluator rule changed --
#2460's empty-operand rule and #2470's read-only scope are what make the reparsed operands
answer as they do, and both were already correct.

**Resolved** ([#2540](https://github.com/rust-works/succinctly/issues/2540)). One row in the
#2506 sweep was a different mechanism and diverged:

| filter                       | real yq | succinctly (before #2540) |
|------------------------------|---------|----------------------------|
| `(.a.zz \| true) and true`   | `true`  | `false`                     |
| `(.a.zz \| 5) and true`      | `true`  | `false`                     |
| `(.a.zz \| [.]) and true`    | `true`  | `false`                     |
| `(.a.zz \| .) and true`      | `false` | `false`                     |
| `(.a.zz \| length) and true` | `false` | `false`                     |

Inside `and`'s read-only scope `.a.zz` yields nothing, and yq's `valueOperator`
(`operator_value.go:11-14`) special-cases an **empty** context by emitting the literal once
anyway -- as do the `[...]` and `{...}` constructors. succinctly used to propagate the
empty, so the left operand registered as empty and #2460's rule short-circuited to
`false`; `.` and `length` loop the context and so correctly stayed empty in both tools
throughout. See "An `and`/`or` operand's evaluation context" above (#2470's section) for
the fix and its own residual (`=`'s right side, a separate code path #2540 did not reach).

### Ordering comparisons against a real `null` -- resolved ([#2483](https://github.com/rust-works/succinctly/issues/2483), containers [#5631](https://github.com/rust-works/succinctly/issues/5631))

Real yq v4.53.3's ordering comparators (`<`/`<=`/`>`/`>=`) treat a real `null` operand as
orderable only against another `null` -- never against any other value -- unlike jq's
total order, which sorts `null` below everything (`null < 1` is jq's `true`). succinctly
reproduced jq's total order in yq mode too before this fix. Captured live (`-o=json -I0`
on `a: 1`, plus `.zzz` for a genuine `null` *node* rather than a `null` literal --
comparison operands are not read-only, unlike an arithmetic/`and`/`or` operand under
`yq_absent_key_read_is_empty`, the previous section's own rule):

| filter                                          | real yq |
|--------------------------------------------------|---------|
| `null == 1` / `1 == null`                        | `false` |
| `null != 1` / `1 != null`                        | `true`  |
| `null < 1`, `<= 1`, `> 1`, `>= 1` (either order) | `false` |
| `null == null`                                   | `true`  |
| `null != null`                                   | `false` |
| `null < null`, `null > null`                     | `false` |
| `null <= null`, `null >= null`                   | `true`  |
| `null <op> "a"`, `null <op> false`               | same as vs a number |
| `.zzz < 1`                                       | `false` |

Fixed as `eval::yq_null_ordering_is_false`, consulted from `eval::apply_compare_op` --
the one shared comparison-operator implementation every evaluator (the ordinary
cursor evaluator, the generic/CLI evaluator, and -- until spine 2416's exit deleted it --
the eager path-context evaluator) already routes through, so this is a single call site,
not three.

**`null` against an array/object (resolved by [#5631](https://github.com/rust-works/succinctly/issues/5631)).**
Real yq raises a Go-internal error there instead of answering at all, and the wording depends on operand *order*, because its ordering comparator is implemented as a subtraction internally:

```console
$ yq 'null < []'
Error: !!seq () cannot be subtracted from !!null
$ yq '[] < null'
Error: arrays not yet supported for comparison
```

succinctly yq raises the same two errors; see the next section.

### Ordering comparisons of unlike kinds, a bool or a container raise in yq -- resolved ([#5631](https://github.com/rust-works/succinctly/issues/5631))

yq's `<`, `<=`, `>` and `>=` are not jq's total order.
`compareOperator` (`pkg/yqlib/operator_compare.go`, v4.53.3) tries these rules in order:

| operands                                       | yq 4.53.3                                                                           |
|------------------------------------------------|-------------------------------------------------------------------------------------|
| a container on the left (any right operand)    | `arrays not yet supported for comparison` / `maps not yet supported for comparison` |
| a scalar on the left, a container on the right | `!!seq () cannot be subtracted from !!int` (right tag, then left tag)               |
| a `null` on either side                        | `false` (`<=`/`>=` of two `null`s is `true`)                                        |
| two numbers (int or float), or two strings     | compared                                                                            |
| anything else (a bool, or unlike kinds)        | `<left tag> not yet supported for comparison`                                       |

So `2 > true` is `!!int not yet supported for comparison`, `"a" < 1` is `!!str not yet supported for comparison`, and `true > false` is `!!bool not yet supported for comparison`; succinctly yq used to answer `true`/`false` from jq's order.
`==` and `!=` never raise, in either tool.
A 17 x 17 x 4 matrix (null, bools, ints, floats, strings, empty and non-empty containers, `~`) was captured from yq v4.53.3 and agrees with `succinctly yq` on every row that does not involve a timestamp or a hex literal.
jq mode keeps its total order (`succinctly jq -n '2 > true'` is `true`).

The refusal is `eval::yq_ordering_refusal`, applied by `eval::try_compare_op` at every call site that can report a failure (the ordinary and generic evaluators, the owned fast path, the `yq` context-pipe route); `direct_cond_verdict` declines to the general route on a refused pair so the error is raised there.

An integer written with a radix prefix (`0x10`, `0o17`) against a float raises `strconv.ParseFloat: parsing "0x10": invalid syntax`, because yq compares an int and a float through `ParseFloat` of each node's text; `succinctly yq` raises the same (#5651).
Two integers compare as integers, and a decimal spelling (`007`, `+5`, `1e3`) parses, in both tools.

**Not reproduced:**

- A string that parses as a timestamp (RFC 3339 or `2006-01-02`, as Go's `time.Parse` reads them) on the left of `<`, `<=`, `>` or `>=` is compared as a time, and its right operand is parsed the same way ([#5651](https://github.com/rust-works/succinctly/issues/5651)).
  `"2001-01-01T00:00:00Z" < 1` is `parsing time "1" as "2006-01-02": cannot parse "1" as "2006"` in both tools, and so is a `null`, a bool or a string in another layout (`parsing time "2001-01-01 00:00:00": extra text: " 00:00:00"`).
  Two timestamps compare as instants, so the same moment in two zones is equal.
  The reading is `go_time::yq_datetime`, a port of `time.Parse`, including its looseness (a one-digit hour, a comma before the fraction, a zone offset of 24 hours); `scripts/yq-time-fuzz.py` replays mutated timestamps against a real yq.
  Under `with_dtf(layout; f)` both sides are read under `layout`, and a string that reads under it is a time too, as in yq (#5863, see the datetime operators section).
  `OwnedValue` has no timestamp kind, so the reading goes by the text, and what yq decides from the `!!timestamp` tag is still different:
  an unquoted document value whose text no layout reads (`2001-01-01 10:00:00`) is a time for yq, which raises on it, and a plain string is `!!str not yet supported for comparison` against a `!!timestamp` on its right, where this answers by string order;
  the error for a right operand names its text as written in yq and as the value reads here, so an empty value (`key:`) is `parsing time ""` there and `"null"` here, a `~` is `"~"` and `"null"`, `True` is `"True"` and `"true"`, and a literal `007` in the filter is `"007"` and `"7"`;
  a number past int64 (`100000000000000000000`) raises `strconv.ParseInt: ... value out of range` in yq and a time error here;
  and the type tag in a container error reads `!!str` here for a document date, which `tag` reports as `!!timestamp` (#5836).
- A signed prefixed integer (`-0x10`), `1_000` and `0b101` read as strings here, so they raise the kind error instead of comparing or raising `ParseFloat`.
- `sort`, `min` and `max` over an array holding a prefixed integer and a float use their own comparators here: yq's `min`/`max` raise the same `ParseFloat` error, and its `sort` panics with it, where this orders them by value.

### Datetime operators: date arithmetic, `format_datetime`, `tz` and `with_dtf` -- resolved ([#5863](https://github.com/rust-works/succinctly/issues/5863)); a quoted date and the timestamp tag remain

yq keeps a timestamp as text tagged `!!timestamp`.
Its datetime operators read that text with Go's `time.Parse` under the current layout and write a result back with `Format`: RFC 3339 by default, with `2006-01-02` tried when RFC 3339 fails, or the layout `with_dtf(layout; f)` sets for everything `f` evaluates.
`succinctly yq` does the same through `go_time`, a port of `time.Parse`, `time.Format` and `time.ParseDuration` (Go 1.26, the toolchain yq v4.53.3 is built with), and `yq_datetime`, which applies them as yq's operators do:

- `+` and `-` add a duration to a timestamp (`.t += "3h10m"`, `.t -= "24h"`), keeping its zone, and raise Go's error for a duration that does not parse (`unable to parse duration [bogus]: time: invalid duration "bogus"`, with `-bogus` for `-`).
  Before #5863, `.t += "3h10m"` concatenated the text and exited 0.
  Any other string on the left of `-` is `strings cannot be subtracted`, or `!!seq () cannot be subtracted from !!str` against a container, in yq's words.
- `*`, `/` and `%` refuse a timestamp on either side ([#5930](https://github.com/rust-works/succinctly/issues/5930)): `.t * 2` is `cannot multiply !!timestamp with !!int`, `.t / 2` is `!!timestamp cannot be divided by !!int`, `2 % .t` is `!!int cannot modulo by !!timestamp`, naming the other operand's tag, as yq does.
  A timestamp times `null` is the timestamp.
  Before #5930, `.t * 2` repeated the text and exited 0.
- `format_datetime(f)` writes the time with Go's layout `f` and reads the result back as a YAML snippet, as yq does: `format_datetime("2006")` is the number `2026`, and `format_datetime("[2006]")` a sequence.
- `tz(zone)` moves a timestamp to the zone `time.LoadLocation` loads: `UTC` or the empty name, `Local`, or an IANA name.
- `now` and `from_unix` are RFC 3339 text in the local zone, with no fraction, and `to_unix` reads under the layout.
- The ordering comparisons read both sides under the `with_dtf` layout.

The local zone is `TZ` or the system zone, as Go's `time.Local` is.
Every row of the issue is pinned against yq v4.53.3 in `tests/yq_cli/i5863.rs`, among 104 rows captured under `TZ=UTC` and `TZ=Australia/Sydney`; the 10 that differ are the ones listed below.

**Not reproduced:**

- `OwnedValue` has no timestamp kind, so the text stands in for the tag: a string is a timestamp when it reads as RFC 3339 or `2006-01-02`, the rule the ordering comparison already follows (#5651).
  A quoted (or JSON) string spelled like one is a `!!str` in yq, which concatenates it, so the two can only both be served by what the right operand is.
  `+` of text that can begin a duration (after a sign, a digit or `.`: `3h`, `1`, `3hrs`) to such a string adds or refuses it as for a timestamp, and any other text concatenates as for a string.
  So `"2026-01-01" + "T00:00:00Z"` is `"2026-01-01T00:00:00Z"` in both tools, and `.t += "3hrs"` raises yq's `unknown unit "hrs"` rather than appending.
  What remains: a quoted date plus a duration (`"2026-01-01T00:00:00Z" + "3h"`) is `"2026-01-01T03:00:00Z"` here and `"2026-01-01T00:00:00Z3h"` in yq, and a plain date plus text that cannot begin one (`.t + "bogus"`, `.t + ""`, `.t + null`, `.t + true`) concatenates here (`null` adds nothing) where yq raises `unable to parse duration [bogus]`.
  `-` has no such split, since yq cannot subtract from a string at all: a quoted date minus a duration subtracts it here, and minus anything else raises the duration error, where yq raises `strings cannot be subtracted` for both.
  Under a custom layout the same text is still a timestamp here, so `with_dtf("2006-01-02"; .q + "1h")` on a quoted RFC 3339 `.q` raises the layout's parse error where yq concatenates.
  Following yq on the quoted string in every case would make a plain date plus a duration concatenate silently on the write routes (`|=`, `+=`), which see the value without its node, and a silently wrong date is the failure this replaced.
  For the same reason `now | type`, `from_unix | tag` and `tz(z) | tag` are `!!str` here and `!!timestamp` in yq, and a string builtin (`test`, `split`) accepts them here where yq refuses them (#5836).
- yq names the node's path in the `format_datetime`, `tz` and `to_unix` errors (`could not parse datetime of [a.b]`).
  A value here carries no path, so the brackets are always empty, as yq writes them for a computed value.
- A missing key as the layout, format or zone argument (`with_dtf(.nope; f)`, `format_datetime(.nope)`) reads as `null` here, so it is the layout `null`; yq navigates it read-only, finds no node and raises `could not find layout for format_time`.
  An empty layout, or a container's, is RFC 3339 in both tools.
- A stage after `with_dtf` that reads path context (`with_dtf(f; .a) | path`) reads the position of `with_dtf`'s input (`[]` where yq has `["a"]`).
  A write through it (`with_dtf(f; .a) = 5`, `|=`, `del`) lands where yq's does.
- `tz` with an IANA name needs the `local-zone` feature (on in `cli`, not on Windows); without it only `UTC`, the empty name and `Local` load, and every other name is `unknown time zone`.
- A `no_std` build has no thread-local layout, so `with_dtf` raises `with_dtf requires the std feature` there.
- `with_dtf(layout, f)` with a comma between its two arguments (yq's `withDateTimeFormat` accepts a union as well as a block) is a parse error here; only the `;` form is read.
- A `with_dtf`, `format_datetime` or `tz` argument is evaluated once per input node here, and once for the whole context in yq, which uses the first result for every node.
  So `.[] | format_datetime(.)` over several timestamps formats each with its own text here and every one with the first node's text in yq.
- `with_dtf` in path position (`with_dtf(error("boom"); .a) |= 1`) reports a layout argument's error without yq's `could not get date time format: ` prefix.
- A `format_datetime` result that YAML reads back as empty (`""`, `" "`, `"#x"`, `".999"` on a time with no nanoseconds) prints `null` here where yq prints an empty scalar, and `format_datetime("[2006]")` prints block style here where yq keeps flow style.
- `.t + "1h" | tag` is `!!str` here and `!!timestamp` in yq, the same split as the tags above.
- `from_unix` on a non-number names its tag in yq's words since #5930: `from_unix only works on numbers, found !!str instead` (`!!bool`, `!!seq`, `!!map`, `!!null`, and `!!timestamp` for a string that reads as one).
  In jq mode it keeps `math function requires number`.
- The `/` and `%` refusals of #5930 write a container or `null` operand differently: yq puts each operand's key in parentheses after its tag when the left operand is a container or `null`, or the right one is a container (`!!timestamp (t) cannot be divided by !!map (m)`), and a value here carries no key, so both are `()`.
  The `*` refusal has no parentheses in yq and none here.
  Only the message differs; the error and the exit status are the same.
- An arithmetic operator over two kinds that do not combine words its error as yq does since #6904, in yq mode (`!!float cannot be added to !!null`, `!!seq () cannot be added to a !!float (f)`, `cannot multiply !!int with !!seq`, `!!float cannot be divided by !!null`, `!!int cannot modulo by !!str`, and yq's `-` over two scalars, `!!int cannot be added to !!str`, as it prints it).
  jq mode keeps jq's `number (1.5) and null (null) cannot be added`.
  A key yq writes in parentheses after a tag is `()` here whenever the operand is a container or the other operand of a container (`+`, `-`, `/`, `%`): a value carries no key, so `.f + .l` is `!!seq () cannot be added to a !!float ()` where yq says `!!seq (l) ... !!float (f)`.
  A literal operand has no key in yq either, so those rows agree exactly.
  Only the message differs; the error and the exit status are the same.
- A quoted date times a number (`"2026-01-01T00:00:00Z" * 2`) is yq's string repetition and raises here, since the text stands in for the tag, as above.
- A quoted string that reads as a date is named `!!timestamp` in the arithmetic errors above, where yq names it `!!str`, for the same reason: `.i - "2001-01-01T00:00:00Z"` is `!!int cannot be added to !!timestamp` here and `!!int cannot be added to !!str` in yq, and `.l - "2001-01-01T00:00:00Z"` is `!!timestamp () cannot be subtracted from !!seq` against `!!str ()`.
  The same holds for `+`, `/` and `%` (#6904 review).
  Only the message differs; the error and the exit status are the same.
- `tz("Europe")` and `tz("zone.tab")` (a directory and a file that is not a zone) raise a different message here (`is a directory`, `malformed time zone information`) from yq's.
- Go matches a zone abbreviation read by a layout's `MST` against every zone the local zone has ever used; here only the zones in effect at the date and half a year either side are asked, which covers standard and daylight time.
- `with_dtf("2006")` and `format_datetime` with no argument are parse errors in succinctly's wording, not yq's (`must provide a date time format string and an expression, ...`, `'format_datetime' expects 1 arg but received none`).
- `from_unix` keeps succinctly's out-of-range error for a number past the calendar it converts (`1e20`), where yq prints the year Go's arithmetic wraps to.

### `|=` with a zero-output update filter -- resolved ([#2484](https://github.com/rust-works/succinctly/issues/2484))

A three-way divergence per ADR-0018 (mode decides): jq 1.7.1 (since jq 1.7) deletes the
key/element a zero-output `|=` filter targets (`_modify`'s own `delpaths` fallback);
real yq instead leaves an already-existing target completely untouched; succinctly's
pre-fix yq mode wrote `null` in both cases, matching neither. Captured live against yq
v4.53.3 (`-o=json -I0`):

| filter                                  | input          | real yq            |
|------------------------------------------|----------------|---------------------|
| `.a \|= (1 \| select(false))`             | `a: 1`         | `{"a":1}` (untouched) |
| `.x \|= (1 \| select(false))`             | `a: 1`         | `{"a":1,"x":null}` (absent target: already agreed, #2470) |
| `.a[] \|= select(. > 5)`                  | `a: [1,6,2,7]` | `{"a":[1,6,2,7]}` (every element untouched, none removed) |
| `.a[0] \|= select(. > 5)`                 | `a: [1,6,2,7]` | `{"a":[1,6,2,7]}` (single index, same rule) |
| `.a[10] \|= select(. > 5)`                | `a: [1,2,3]`   | padded with `null` out to index 10 (#1916's own bounds rule, not this one -- unaffected) |
| `.a[1:2] \|= (. \| select(false))`        | `a: [1,2,3]`   | `{"a":[1,2,3]}` (slice form untouched too) |

Fixed in `update_path`'s own terminal `Expr::Identity` arm -- the base case every
`Field`/`Index`/`Iterate`/`Slice` arm's leaf write already funnels through -- by simply
not assigning anything when the filter is zero-output in yq mode: `root` at that point
already *is* the value being updated (the pre-existing one for an already-populated slot,
or the freshly-autovivified/padded default for one that didn't exist before this write),
so "leave it as it was" needs no separate before/after snapshot. `update_path_steps`'s own
terminal case (`.a[N] |=`/`.a.b |=`-style multi-step chains) used to keep an independent,
identical copy of the same three lines; it now calls into the `Expr::Identity` arm instead
of duplicating it, so this fix (and any future one) cannot drift between the two the way
CLAUDE.md's "duplicated predicates diverge silently" warns about.

**Not chased here, but since resolved by a different fix:** `.a |= (.zzz | key)` on `a: 1`
(also asked for in the issue) was not part of this fix at the time. Real yq answers
`{"a":1}` because `.zzz` on the number `1` -- `|=`'s right side runs against the *target's
own value*, not the whole document -- yields nothing there (yq indexes a scalar with a key
as "nothing everywhere" everywhere, not only read-only). succinctly used to raise `Cannot
index number with string "zzz"` for that shape instead, so the update filter never got the
chance to become zero-output in the first place -- that was the pre-existing
scalar-indexing gap, not this one. [#2482](https://github.com/rust-works/succinctly/issues/2482)
has since closed it directly (see the entry above), and succinctly now answers `{"a":1}`
here too.

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
as every other fix in this section. Three more copies were left over, and
[#2335](https://github.com/rust-works/succinctly/issues/2335) settled them: `path()`'s own
walker (`navigate_static_component`, `src/jq/eval.rs`) is not reached by yq's grammar (real yq
rejects every `path(<expr>)` form as `bad expression`, so `path(expr)` is gated behind
`--jq-extensions`, and `.a.b | path` takes no argument); `get_value_at_path` only ever sees
document-harvested or gated paths and cannot panic; and `getpath`'s own path-array walk
(`resolve_read_index`/`getpath_walk_owned`) keeps jq's `null` under `--jq-extensions`, the
flag being the jq-compatible surface (an extension only has to leave filters the reference
accepts alone -- ADR-0018 rule 5). What #2335 did change is the **write side**: a write through
a still-negative index (`.a.b[-5] = 9`, `|=`, `+=`, `*=`, a deeper path, a comma target,
`setpath`, an index reached through an auto-vivified `null`) now says `index [-5] out of range,
array size is 3` as yq does (it said jq's `Out of bounds negative array index`), and `?` does not
suppress it. A float key still differs in wording (`.a.b[-5.5] = 1` is
`cannot index array with '-5.5'` in yq).
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

**Closed for plain indices (#4749).**
yq's `deleteChildOperator` evaluates every path of a `del()` batch left to right into nodes: an index past the end grows the array with `null`s as it is evaluated, and a negative index counts back from the length at that moment.
It then deletes the nodes from the back, each by the key it holds at that time.
A deletion renumbers the nodes that stay, but a node already deleted keeps its old key, so naming one twice deletes whatever has moved into its place, and a key past the end deletes nothing.
`yq_delete_index_sequence` does the same: `del(.[5], .[0])` is `[20,30,null,null]` in either order, `del(.[-1], .[5])` is `[10,20,null,null]` while `del(.[5], .[-1])` is `[10,20,30,null,null]`, and `del(.[0], .[0])` is `[30]` (`[10,20,30]` in each case).
A string spelling an index (`.["5"]`) names the same node as the number.
Distinct nodes are one pass over the grown array; a repeated one is replayed, bounded to 2^27 element-times-index steps, past which the repeat is deleted once.
`DeleteTrieNode::terminal_order` keeps the order and the repeats that the interned trie edges lose.
Pinned by `tests/yq_cli/i4749.rs`; a one-off differential sweep of about 2,500 random batches (indices -8 to 12 on arrays of 0 to 5 elements, with string spellings, nested under a key or at the root) against yq v4.53.3 (not kept in the repo) found no difference in any batch both tools answer.
Still open: a slice in the batch (`del(.[5], .[0:2])` is `[10,20,30,null,null,null]` in yq, where a slice has no parent and ends the deletion, so nothing before it in the batch is deleted).
Still open: a negative index that is out of range of the array as it entered but in range once an earlier past-the-end index has grown it (`[10] | del(.[2], .[-2])` is `[10]` in yq).
That one is the read side, not the delete: yq's `.[2], .[-2]` grows the array as it reads (#2435), and the read here does not, so the path resolves before `del()` sees it and raises `index [-2] out of range`.
The same cause gives the size in the message of a negative index that is out of range in both (`[10,20,30] | del(.[5], .[-9])` says `array size is 6` in yq and `array size is 3` here).
A repeated index on an array past 2^22 elements, or past 2^27 element-times-index steps, is deleted once where yq deletes again (the replay holds two words per element and memmoves per repeat).
A `null` or absent target is vivified for the batch as for a single index (`null | del(.[2], .[3])` is `[null,null]`).

**Gated off when 2+ *terminal* indices share one node, unless they are plain indices (#4749).**
A batch of plain indices is replayed by `yq_delete_index_sequence` (above) and vivifies a `null` target, so `null | del(.[2], .[3])` is `[null,null]` as in yq; what follows describes the gate as it still stands for a batch with a slice, and its numbers predate that.
`delete_keys`'s own #2305/#2306
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

### A bracket key turns a `null` into a container in `del()` — but not in a plain read (#2126)

Real yq reads `.["k0"]`, `.[0]` and `.[("k0","k1")]` through `traverseArrayIndices`, which
rewrites a `!!null` node it indexes into the empty container the key implies (`{}` for a
non-integer first key, `[]` for an integer) *before* looking anything up, while the dotted `.k0`
(`traverse()`, which honours `DontAutoCreate`) leaves the `null` alone. `del()` evaluates its
argument read-only, so only the bracket forms show it:

```bash
$ echo 'null' | yq -o=json 'del(.k0)'              # null
$ echo 'null' | yq -o=json 'del(.["k0"])'          # {}
$ echo 'null' | yq -o=json 'del(.[(0,1)])'         # []
$ echo '{"a":null}' | yq -o=json 'del(.a["k0"])'   # {"a":{}}
```

`succinctly yq` matches: inside a `del()` argument (yq mode only; jq mode and an integer key fold as
before) a string bracket key is parsed as the bracket it is rather than folded into the `.k0` field
step, `del()` runs a pre-pass (`vivify_del_null_bracket_targets`, then `vivify_apply`) that walks each
branch and vivifies a `null` that a bracket, a position or an iteration `.[]` indexes, and the bracket
form is folded back into `.k0` before the delete itself runs, so no other delete rule can tell the
two spellings apart. [#4922](https://github.com/rust-works/succinctly/issues/4922) made the walk
multi-valued, as yq's is: `.[]` over arrays and objects (and over a `null`, which becomes `[]` when
something follows), `..`, a comma ahead of a bracket (`(.a,.b)["k"]`), a multi-key bracket
(`{"a":null} | del(.a[(0,1)]["k"])` is `{"a":[{},{}]}`), a computed key evaluated against the whole
document (`{"a":null,"n":"x"} | del(.a[.n])` is `{"a":{},...}`, a number or an absent key makes an
array), and a too-short array padded out to the position a later bracket vivifies
(`{"a":[1,null]} | del(.a[5]["k"])`). A computed key is read against the node the bracket's target started from (`.a[.n]` reads the document's `.n`, `.a | .[.n]` reads `.a`'s) and only when it is plain navigation and literals (a key with a builtin call, `range(..)` or `input` is not evaluated by the pre-pass). Under `..` a step that would make an array from a `null` is left alone, because the delete that follows walks `..` over what the pre-pass made (`{"a":null} | del(.. | .[(0,1)].x)` is `{"a":[null,null]}` in yq, whose `..` is lazy), and `..` goes 256 levels down. Padding is kept only if something is vivified, so
`del(.a[2])` on a `null` stays `{"a":[]}`. Not vivified: a computed key that is itself absent or an
array when the bracket is the *last* step (`{"a":null} | del(.a[.n])` is `{"a":[]}` in yq and an
error here, as before), a slice (`del(.a[1:2]["k"])` is a no-op in yq and an error here), a bracket
behind `select(...)`/`first(...)`/`//`, a position past 1024 (the padding is capped), a float or
exponent key (`null | del(.[1.5])` is `{}` in yq and `[null]` here, from the older `Index` fold),
and sibling comma branches that disagree on a `null`'s kind: yq evaluates them in sequence, here
the pre-pass has already made the container, so `del(.[].q, .[][.m])` on `{"a":null,...}` errors
(`Cannot index array with string "q"`) where yq prints `{"a":[],...}`.

Still different: the same mutation happens in yq on a plain read, because the node is rewritten
in place. `echo null | yq -o=json '(.["k0"]) as $x | .'` and `select(.["k0"] == 2), .` print `{}`;
`succinctly yq` prints `null`, since reproducing it would need a read to mutate the streamed
document. The same mutation is visible through an assignment: `{"x":null} | .y = (.x | del(.["k"]))`
is `{"x":{},"y":{}}` in yq and `{"x":null,"y":{}}` here.

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

**Fixed since ([#2380](https://github.com/rust-works/succinctly/issues/2380)):** a
comma-grouped sibling whose shape is `.[]` *followed by more path* (`del(.missing[].x,
.y)`, `.missing[].x.z[0]`, `.missing[].x[]`, `.n[].x` over a stored `null`, `.[5][].x`
past the end of an array) used to fall through to the read-based comma-branch resolution,
which raises "Cannot iterate over null". `iterate_then_static_suffix` now recognises it
beside the bare trailing `.[]`, in either position of the group, and the same
`contains_length_dependent_index` guard applies: a sibling with a negative or computed
index still declines the whole pre-pass.

The pre-pass also takes a prefix that reads as any other non-container (`{"s":5,"y":1} |
del(.s[], .y)`, a string or a bool), which yq's `.[]` skips just the same (#2346): that
shape raised `Cannot iterate over number (5)` before.

### `del()` with a comma group stops at a bare slice of the root ([#1325](https://github.com/rust-works/succinctly/issues/1325))

yq's `deleteChildOperator` walks the matched nodes from the last to the first and returns at the
first one that has no parent. A slice result is a replacement of the node it slices, so a slice
of the root (`.[2:3]`, `.[2:3][0:1]`, `(.[2:3])?`, `.[2:3] | .[0:1]`) has none, and the delete stops
there: the branches written after it are deleted, it and everything before it are not
(`del(.a[0:1], .[2:3])` deletes nothing, `del(.[2:3], .a[0:1])` deletes `.a`, over an object or an
array root alike). `succinctly yq` reproduces that. A slice further down (`.a[0:1]`) has a parent,
and a tail after the run (`.[2:3][0]`, `.[2:3].x`) is inert without stopping anything.

yq also *creates* while it resolves the paths, before the stop: an index past the end of an array
extends it with `null`s, and an index or `.[]` on a `null` makes an array
(`del(.x[3], .[2:3])` over `{"x":[1]}` is `{"x":[1,null,null,null]}`). A group in which a branch
before the stop could do that keeps the behaviour it had before #1325, which deletes that branch
and creates a little differently. "Could" is judged on the path before its first slice: a computed
key, an index outside the array or below its start, a `.[]` that is not the last step, an index or
`.[]` on `null`, a recursive descent, or a step on the wrong type. Every other group follows the
rule above, over an object, array, string, number or `null` root alike. (A branch like
`.x[3][0:1][0]`, whose slice makes it inert, creates `.x[3]` in yq and does not here; that predates
#1325.)

A bare root slice whose bounds are computed (`.[(1+1):3]`, `.[:(.b)]`, `.[(.b):3][0:1]`) is the stop too ([#5397](https://github.com/rust-works/succinctly/issues/5397)).
Its bounds are read against the root, and it counts when each one is exactly one integer, which is what yq's `ParseInt` takes.
A bound that gives none, several, `null`, a string or a fractional number is not the stop, and raises or is resolved as it was before.

Still different: a computed bound on a slice that follows another (`.[0:1][(1+1):3]`) is not recognised as the stop (the group is deleted as if it were absent), because yq reads that bound against the earlier slice's result.
A branch that raises before the stop raises with this tool's wording rather than yq's, as everywhere else.

### `del()` through a slice-run and a field tail: per element, one gap ([#2344](https://github.com/rust-works/succinctly/issues/2344), [#4542](https://github.com/rust-works/succinctly/issues/4542))

`del(.a[0:2].x)` fails in yq for every target but a string: yq slices a mapping, int, bool, `null`
or array to an empty sequence (a mapping to its alternating key/value children), and `.x` is then
parsed as an index. A `.[]` ahead of the run (`del(.a[][0:2].x)`) is classified per element, and
one failing element fails the call. A `.[]` *after* the run (`del(.a[0:2][].x)`) is applied to
each sliced element: an array element (even an empty one) fails a following field, a mapping,
scalar or string element does not, and an index tail never does. The steps before the run are
type-checked too (`del(.a[0:2][].x)` over a root array fails on `.a`, `del(.a[].b[0:2].x)` over
`a: [[1]]` fails on `.b`), including when the path ends in a further slice behind the field
(`del(.a[0:2][].x[0:1])`). Only `.field`, `[n]`, `.[]` and slice steps are walked; a computed
index or recursive descent in the tail ends that walk as a no-op. `.x?` suppresses and a `[0]`
tail stays a no-op. A negative index below the start of the sliced sequence raises yq's
`index [N] out of range, array size is S` (`del(.a[0:2][-3])`; a scalar, `null` or mapping target
slices to size 0 or to its key/value children, and a trailing `?` does not suppress it,
[#4568](https://github.com/rust-works/succinctly/issues/4568)), while a positive index past the end
stays a no-op but walks on over `null`, so `del(.a[0:2][5][-1])` fails with size 0.

One shape still differs, because the information is not in the value tree: an
unquoted YAML timestamp (`a: 2024-01-01`) or a `!!binary` scalar is a string to succinctly but not
to yq, so `del(.a[0:2].x)` no-ops here where yq fails (a *quoted* `"2024-01-01"` matches).
A *read* of such a slice now matches (#4569).
yq slices only a plain string as text and every other scalar to an empty sequence.
So `.a[0:2]`, `.a[1+0:]` (a computed bound) and what follows (`| length`, `[0]`, `.x`) answer `[]`, `0`, `null` and an error for an unquoted timestamp, a `!!binary`, an explicit core tag other than `!!str` (`!!int "5"`) and an alias to a string.
A quoted date, `!!str` and a custom-tagged scalar still cut their text.
That is read from the cursor, where the style and the tag are.
A write or `del()` evaluates over the owned value, which holds the text and nothing else, so `|= .[0:2]`, `with_entries(.value |= .[0:2])`, a slice of a comma's copies (`(.a, .b)[1+0:]`) and the `del()` row above still cut the text or no-op.
Closing them needs a scalar kind carried with the value, the same question as #2802's spellings.
Under `--eval-all` the document is read back from the owned value, which has lost the quotes, so a quoted date there reads as a plain one, as it already does for `tag` (`.a | tag` of `"2024-01-01"` is `!!timestamp`), and `map(.[0:2])` slices it to `[]`.
The slice's own tag (`!!timestamp []` in yq, `!!seq []` here, as for an integer) is the missing tag slot of #1416.
An alias to an array or a mapping is still sliced through to its target here (`y: *a` over `a: [1, 2, 3]` with `.y[0:2]` is `[1,2]`), where yq answers `[]` for every alias.

A comma-grouped `del()` with an `.[]` branch no longer raises on a non-null scalar
([#4464](https://github.com/rust-works/succinctly/issues/4464)): `5 | del(.a, .[])` and
`del(.[], .[])` are `5`, as in yq, and `a: 5` with `del(.a[], .b)` is unchanged. The path resolver
that backs this treats `.[]` over a non-null scalar as yielding nothing in yq mode everywhere,
so `--jq-extensions` `path(.[])` over `5` is empty rather than jq's raise. A `null` still raises
(`null | del(.b, .[])` is `[]` in yq, and `del(.a[][], .c)` over `a: [null, [1]]` is `[[], []]`):
the resolver cannot tell a stored `null` from the `null` placeholder an out-of-range index or
a missing key reads as, and dropping the branch for the placeholder would be silently wrong
(`del(.[-1], .[4][])`).
A `del()` with one target is not that case (#6892): `del(.zz[] | select(.n == 1))` over a document without `zz`, or with `zz: ~`, deletes nothing and leaves the document as it is, as yq does.
There is no sibling for a placeholder to stand in for, so the resolver yields no branch for a `null` there.
Over a stored `null` yq also turns it into `[]` (`a: ~` becomes `a: []`), which this does not.
With a comma (`del(.zz[] | select(.n == 1), .kind)`) the raise stays.
A field write through the same path (`(.zz[] | select(.n == 1)).x = 5`, and `|=`, `+=`, `-=`) adds the `zz: []` yq adds instead of raising (#6902).
The write raises where a comma names a second target (`(.zz[] | select(.n == 1), .kind).x = 5`), where an iteration follows the select (`(.zz[] | select(.n == 1)).x[] = 5`, where yq creates `x: []` under each node the select passed), in a document with an anchor or alias, and under `sort_keys(f)`, which never creates what it walks (yq leaves the document as it is).

### A slice keeps an explicit `!!str`/`!!int` tag, but not `!!binary` or an alias ([#4328](https://github.com/rust-works/succinctly/issues/4328))

A computed-bound slice (`1 as $i | .[$i:]`) converts each element through its cursor, so
`- !!str 2` stays the string `"2"` and `- !!str true` stays a string, as in yq. The scalar kinds
`OwnedValue` cannot carry still come out of *any* slice, literal bounds included: `!!binary aGk=`
reads as a plain `!!str`, and an alias element (`*x`) as the value it names (`map(tag)` over
`[!!float 1, !!binary aGk=, &x !!str 5, *x]` is `["!!float","!!binary","!!str",""]` in yq,
`["!!float","!!str","!!str","!!str"]` after a slice here; without the slice both agree). How a
sliced value's tag is *rendered* in YAML output (`- !!str 2` vs `- "2"`) is the missing explicit-tag
slot, [#1416](https://github.com/rust-works/succinctly/issues/1416).
That holds for a computed-bound slice.
A literal-bound slice (`.[1:]`, `.a[:2]`) keeps each element's tag, comment, anchor and quoting (#5429).

### `.[]` over a non-container is now a silent no-op — two residual gaps remain (#2346)

Real yq's `.[]` (iterate) produces zero output — not an error — for *any* non-container
target (number, string, bool, `null`), not just `null`, confirmed live against yq v4.53.3
for plain reads, `del()`, and assignment alike (`echo '{"x":5}' | yq -o=json '.x[]'` is
empty, exit 0). jq has no such rule (`.[]` on a scalar always raises there, confirmed
against jq 1.7.1), so this is yq-mode only — fixed at every `Expr::Iterate` site reachable
from a plain read, `del()`, or a `path()`-style path computation (#2346). `=`/`|=`'s own
`set_path`/`update_path` machinery already had this right beforehand.

Two adjacent sites were deliberately left unfixed, since a naive copy of the same widening
would be wrong (not just incomplete) or risks a worse regression. The first has since been
fixed on its own terms (#2375); the second is still open:

- **`map(f)` used to still raise** when path-tracking is needed inside `f`
  (`eval_stage_with_path_context`'s own `Expr::Builtin(Builtin::Map(f))` arm) — `map`'s
  real yq-mode rule is *asymmetric* (`null` -> `[]`, but any other scalar passes through
  **unchanged**), not the uniform "always empty" rule `.[]` itself gets, which is why
  #2346 left it alone rather than copying its own widening onto it. Fixed in #2375 by
  giving that arm the same rule the value-only `builtin_map` has had since #1907, captured
  live against yq v4.53.3 (note the spaced `+`: yq's lexer reads the unspaced `map(.+1)`
  as a different query entirely, yielding `[]` for *any* input):

  ```bash
  $ printf 'a: null\n' | yq -o=json -I=0 '.a | map(key)'            # []
  $ printf 'a: 5\n'    | yq -o=json -I=0 '.a | map(key)'            # 5
  $ printf 'a: 5\n'    | yq -o=json -I=0 '.a | map(key) | . + 100'  # 105
  $ printf 'a: 5\n'    | yq -o=json -I=0 '.a | map(. + 1) | key'    # "a"
  ```

  The *container* rows of the same arm used to diverge in the opposite direction: yq's
  `map(f)` over a container builds a new, detached node whose path is `[]`, so a downstream
  `key` emits nothing at all (`printf 'a: [1,2]\n' | yq -o=json '.a | map(. + 1) | key'`
  prints nothing, exit 0), where succinctly reported the *pre-map* node's key (`"a"`).
  Closed by the owned identity pipe (spine 2416 step 3, ADR-0021 decision 7): `map`'s rule
  there detaches a container result and passes a scalar through with its position, which
  is the same mechanism seen from both sides — yq never replaced the scalar node, so it
  kept its own path. Pinned in `test_owned_identity_rules_match_yq_2416`.
  An *empty* container is the exception, as a scalar is: yq keeps its position through `map(.)` (`a: []` with
  `.a | map(.) | path` is `["a"]`, `key` is `"a"`, `parent` is the document), so the rule detaches only a container with members
  ([#3450](https://github.com/rust-works/succinctly/issues/3450); pinned in `test_map_over_an_empty_container_keeps_its_position_3450`).
  Since [#4690](https://github.com/rust-works/succinctly/issues/4690) the cursor site decides "has members" from what the node
  presents rather than from its subtree: an *alias* of a non-empty container detaches (`a: &x [1]` / `b: *x`: `.b | map(.) | path` is `[]`),
  and a mapping whose only content is a `<<` entry of an empty source keeps its position (`a: {<<: {}}`: `["a"]`), as yq does
  (pinned in the same test; captured from yq v4.52.2 and the issue's v4.53.3 rows, which agree).
- **`first(.a[])` on a genuinely-resolved scalar still raises** (`resolve_iterate_sink`,
  shared by `resolve_node`'s own `Expr::Iterate` arm and `first`/`limit`'s fast path) —
  this function is also reached by `resolve_del_path_branches`'s comma-fanout fallback for a
  declined vivify pre-pass, where an out-of-range index read is a placeholder for a write
  real yq's own vivify would perform, not a genuinely-resolved scalar; naively widening this
  function turns an honest raise (`del(.[-1], .[4][])`, a documented, tracked coverage gap)
  into a silently wrong answer instead (confirmed live: `[1]` instead of real yq's `[1,
  null, null, []]`). Tracked as
  [#2376](https://github.com/rust-works/succinctly/issues/2376).

### A computed slice bound must produce exactly one number in yq mode (#2056, #4197)

`.[(K1,K2,...):E]`/`.[S:(K1,K2,...)]` — a slice bound written as a parenthesized comma
generator, computed rather than a single literal or expression — streams each
already-produced bound's own slice result before a later bound's error/break/halt/type
failure in jq mode (jq's own `S as $s | T as $t | E | .[$s:$t]` desugaring), confirmed
against jq 1.7.1: `path(.[(0,1,"x"):(2,3)])` on `[10,20,30]` prints all four
`{start,end}` combinations of the two valid `start` values before raising on `"x"`.

yq mode has no prefix to stream: real yq wants exactly one number from each bound and rejects
anything else before it slices, confirmed live against yq v4.53.3: `.[(0,"x"):3]` on
`[10,20,30]` is `Error: expected to find 1 number, got 2 instead`, on every target kind
(`null` and scalars included), for a read, `=`, `|=` and `del()` alike, with `?` not
suppressing it, and the count is checked before any value is parsed (`.[(0,1,"x"):3]` is
`got 3`, not a type error). `succinctly yq` raises the same text ([#2056](https://github.com/rust-works/succinctly/issues/2056)),
at the three sites that resolve a computed bound (`each_slice_bound`, `each_slice_bound_generic`,
`drive_slice_bound` — the bridge-route read, CLI-route read, and `=`/`|=`/`del()` write-path
resolvers, #2372/#2385) and in the lazily-pulled path-context walks (`key`, `parent`).
It is yq's slice operator that checks the count, once per node its left side produced, so the
verdict is raised only once the target produced a node: an error the target raises comes first
(`([1,2],[3,4],error("x"))[(0,1):(1,2)]` is `Error: x`), a target with no node never checks
(`5 | .a[(0,1):2]` prints nothing), and a bound generator's own `error(...)` outranks the count
(`.[(0,1,error("y")):3]` is `Error: y`). jq mode keeps its generator model and streams each
bound's slice. A target that produces no node in yq is no error here either
([#4461](https://github.com/rust-works/succinctly/issues/4461)): the lazily-pulled path-context walks
(`key`, `parent`, `path`) step the target before they pull a bound, so `{"a":1} | .a.b[(0,1):2] | key` prints
nothing and `{"a":[1,2,3]} | .a.b[(0,1):2] | key` reports the target's own error; `del()` of a key the document
does not hold (`{"b":1} | del(.a[(0,1):2])`, `{"a":null} | del(.a.b[(0,1):2])`) leaves the document alone,
because the slice resolver, told it is resolving a `del()` target (`yq_delete_context`), checks the target's
path against the document (`names_document_node`) where a write would have created the key. That check reads
`.field` and `.[n]` steps (through a `?`) only; a target path with any other step (a slice, a computed key) keeps the
old answer, the count error. Still different: yq pads a sequence a `del()` target indexes past its end with `null`
as a side effect (`{"a":[1]} | del(.a[3].b[(0,1):2])` prints `{"a":[1,null,null,null]}`, and `.a[0].b` through an
empty sequence or a `null` prints `[null]`), which is not reproduced (this prints the document as it was, or still
raises the count error for the `[0].b` shapes); and a negative index before the start of a sequence raises
`index [-5] out of range` in yq and the count error here.

The same three sites keep their *eager* shape in yq mode after
[#2546](https://github.com/rust-works/succinctly/issues/2546) made jq mode's bound pull
lazy and deferred its bound-type check to the slice step: real yq evaluates a bound
expression in full before it slices, and parses every bound before it looks at the
target — `.["x":]` is `Error: strconv.ParseInt: parsing "x": invalid syntax` on an
array, a mapping, a scalar *and* `null` alike, and `.["x":]?` raises the same
(confirmed live against yq v4.53.3; jq answers `null` for the null target, `Cannot
index object with object` for the mapping, and suppresses all three under `?`). So in
yq mode a non-numeric bound still raises succinctly's `Array/string slice indices must
be integers` for every target kind, `?` does not suppress it, and a bound generator's
later `error(...)` still outranks an earlier non-numeric value
(`.[(0,"x",error("y")):3]` is `Error: y`, `.[(0,1,error("x")):(2,3,error("y"))]` is
`Error: x` — `end` is never reached) — only the message text differs from real yq's, as
above.

The same classification covers a *fractional* or float-*spelled* bound (#3415): real yq
refuses `.[0:1.5]`, `.[(1.5):]`, `.[1.0:]`, `.[1e0:]` and `.[("1.0"|tonumber):]` with
`strconv.ParseInt: parsing "1.5": invalid syntax` (any target, any read or write, `?`
included), while a float that is integral *by computation* renders as an integer and is
accepted (`.[(1.0+1):]` is `.[2:]`). `succinctly yq` raises its usual `Array/string slice
indices must be integers` for the first group — again only the message text differs —
and no longer floors/ceils as jq does (`succinctly jq` still does).

A computed integral float is accepted only while yq renders it as plain digits: Go's shortest
`%g` switches to exponent form at a magnitude of `1e6` (#3970), so `.[(1.0*999999):]` slices
and `.[(1.0*1000000):]` raises `strconv.ParseInt: parsing "1e+06": invalid syntax` (past `i64`
the text is `value out of range` or an exponent form depending on how the operands were spelled,
and raises either way). Swept against yq v4.53.3 for both signs; `succinctly yq` refuses every
float of that size with its usual text. An *integer* product of any size is accepted by both.
The plain *index* path follows the same rule since #4749: `.[(1.0*1000000)]` raises `Cannot index
array with number` (yq: `cannot index array with '1e+06'`) and `.[(1.0*999999)]` reads `null`.

A bound with **no** output is not part of that divergence: yq raises `expected to find 1 number,
got 0 instead` for it wherever the slice is read or written (`.b[select(false):]`, `.b[$typo:]`,
`.b[1:select(false)] = [9]`, `del(.b[select(false):])`), on every target kind, and neither `?` nor
`//` suppresses it, so `succinctly yq` raises the same text (#4197, confirmed live against yq
v4.53.3). A bound with two or more values raises the same error with its count (#2056, above).
jq mode keeps its own rule (an empty bound is "no slice").

### An integer past `i64` echoes its digits, in any spelling; what still differs (#1129, #4921)

Real yq keeps a scalar's text as its value, so `a: 99999999999999999999` answers
`99999999999999999999` through `tostring`, string interpolation, `join`, `length` and a YAML
re-emit, while its JSON encoder prints the double it parsed (`100000000000000000000.0`).
`succinctly yq` matches both for an integer past `i64` in every spelling: signed with `-` or `+`,
with leading zeros (`0099999999999999999999`), and forced by a tag (`!!float 99999999999999999999`):

```bash
$ printf 'a: +99999999999999999999\n' | yq '.a | tostring'
+99999999999999999999
$ printf 'a: +99999999999999999999\n' | succinctly yq '.a | tostring'
+99999999999999999999
```

The digits ride in a `NumberLiteral` whose text the reindex bridge reads as number syntax, and
neither `+9...` nor `0099...` is JSON number text.
A `+1.0` or `1.` crosses the bridge the same way, as the spelling token since #1356.
The bridge reads the leading zeros as it reads `007` (#1149) and keeps a `+` on this one shape
when a bridge number is read (`JsonNumber::bridge_signed_overflowed_int`). jq mode follows jq, which
stores `+X` as `X` (#2877): a `load()`ed YAML `+99999999999999999999` carries its `+` in the value
but `succinctly jq` prints `99999999999999999999`, as for the plain spelling. A run past 309 digits, leading zeros
counted, is not kept: past 309 significant digits it is infinite as a double and resolves to a
string, as in yq, and a longer zero-padded run keeps the rounded double it printed before.

Two differences remain, none of them about the digits of an integer past `i64`:

- **`tonumber` re-reads the text with its own rules** (#1356): `.a | tostring | tonumber` is
  `0099999999999999999999` in yq, and `succinctly yq` answers `1e+20` (`+99999999999999999999`
  loses its `+`).
  An integer spelling that fits `i64` (`+5`, `007`, `0x10`, `0X10`, `0o7`, `1_000`) keeps its text since #1356's integer slice, so `tostring`, interpolation and YAML output echo it as yq does and arithmetic keeps its base.
  A float spelling (`"+2.0"`, `"2."`, `".5"`, `"1.e5"`) keeps its text the same way since the float slice of #1356, as does a document float written so (`a: +2.0`, `a: .5`); the JSON writers print yq's `FormatFloat` of it (`2.0`, `0.5`, `100000.0`).
  An underscored decimal or a hex float (`"1_0.5"`, `"0x1p3"`) still normalizes to its value (`10.5`, `8`).
  An explicit `!!float 9223372036854775807` prints the rounded double.
  `"-0" | tonumber` is a float here (`-0` in JSON, `!!float`) and an `!!int` in yq, which prints `0` there.
  A `-0` expression literal is the integer zero in both tools: `tostring` and YAML output echo `-0`, and JSON output prints `0` ([#6051](https://github.com/rust-works/succinctly/issues/6051)).
  A `-0` index bound to a variable and used on a literal array (`-0 | . as $i | [10, 20] | .[$i] | path`) still echoes `-0` in the path when it is written as YAML (`- -0`, where yq prints `- 0`); JSON output prints `[0]`.
  A signed or underscored integer past `i64` (`"+9223372036854775808"`, `"1_000_000_000_000_000_000_0"`) normalizes to its double.
  An underscored or upper-case-prefixed spelling (`1_000`, `0X10`) written to a file and read back by `succinctly yq` is a string, the recorded input-spec divergence; yq reads it as an integer.
- **A tag is not printed on a nested value.** yq keeps an explicit tag on the node, so a `!!float`
  scalar prints it when it is nested or copied (`[.a]` is `- !!float 99999999999999999999`, `.b = .a`
  writes `b: !!float 99999999999999999999`); a bare `.a` and `tostring` do not. `succinctly yq` drops
  the tag there for every tagged float whose literal it keeps (`!!float 5.0` is `5.0`, `!!float 1e3`
  is `1e3`), and now the overflowed integer too. The scalar's value and text are right.

### Comma binds looser than pipe in real yq — the grouping is resolved (#2420), the evaluation model with it (#2451); one residual remains (#2452)

Real yq's parser is a shunting-yard operator-precedence parser
([`pkg/yqlib/expression_postfix.go`](https://github.com/mikefarah/yq/blob/v4.53.3/pkg/yqlib/expression_postfix.go)),
and its own precedence table
([`pkg/yqlib/operation.go`](https://github.com/mikefarah/yq/blob/v4.53.3/pkg/yqlib/operation.go))
assigns `pipeOpType` precedence 30 and `unionOpType` (`,`) precedence 10 —
**pipe binds *tighter* than comma**. jq's grammar ranks the two operators the
other way around (comma binds tighter than pipe), and until
[#2420](https://github.com/rust-works/succinctly/issues/2420) `succinctly`
applied jq's ranking in *both* modes. `succinctly yq`'s parser now uses yq's
own table (`parse_yq_comma_expr` in `src/jq/parser.rs`: a comma of
pipe-without-comma operands, in `ParserMode::Yq` only); the `succinctly yq`
rows below record the pre-fix output and are kept so the capture stays
readable next to the two references. Confirmed live against the pinned v4.53.3 binary and `/usr/bin/jq` 1.7.1 on
`printf 'a: 1\nb: 2\nc: 3\n'`:

```bash
$ printf 'a: 1\nb: 2\nc: 3\n' | yq -o=json -I0 '.a, .b | . + 10'
1
12
$ echo '{"a":1,"b":2,"c":3}' | jq -c '.a, .b | . + 10'
11
12
$ printf 'a: 1\nb: 2\nc: 3\n' | succinctly yq -o=json '.a, .b | . + 10'
11
12

$ printf 'a: 1\nb: 2\nc: 3\n' | yq -o=json -I0 '.a | ., .b'
1
2
$ echo '{"a":1,"b":2,"c":3}' | jq -c '.a | ., .b'
1
jq: error (at <stdin>:1): Cannot index number with string ("b")   # on stderr

$ printf 'a: 1\nb: 2\nc: 3\n' | yq -o=json -I0 '.a, .b | ., .c'
1
2
3
$ echo '{"a":1,"b":2,"c":3}' | jq -c '.a, .b | ., .c'
1
jq: error (at <stdin>:1): Cannot index number with string ("c")   # on stderr

$ printf 'a: 1\nb: 2\nc: 3\n' | yq -o=json -I0 '.a | .b, .c | . + 1'
4
$ echo '{"a":1,"b":2,"c":3}' | jq -c '.a | .b, .c | . + 1'
jq: error (at <stdin>:1): Cannot index number with string ("b")   # on stderr, no stdout

$ printf 'a: 1\nb: 2\nc: 3\n' | yq -o=json -I0 '(.a, .b) | . + 10'
11
12
$ echo '{"a":1,"b":2,"c":3}' | jq -c '(.a, .b) | . + 10'
11
12

$ printf 'a: 1\nb: 2\nc: 3\n' | yq -o=json -I0 '.a, (.b | . + 10)'
1
12
$ echo '{"a":1,"b":2,"c":3}' | jq -c '.a, (.b | . + 10)'
1
12

$ printf 'a: 1\nb: 2\nc: 3\n' | yq -o=json -I0 '[.a, .b | . + 10]'
[1,12]
$ echo '{"a":1,"b":2,"c":3}' | jq -c '[.a, .b | . + 10]'
[11,12]

$ printf 'a: 1\nb: 2\nc: 3\n' | yq -o=json -I0 '.a, .b | length'
1
1
$ echo '{"a":1,"b":2,"c":3}' | jq -c '.a, .b | length'
1
2

$ printf 'a: 1\nb: 2\nc: 3\n' | yq -o=json -I0 '.a, .b | key'
1
"b"
```

Given `pipe`=30 > `union`=10, `A, B | C` parses as `A, (B | C)` in real yq:
only the *rightmost* comma branch is fed forward through a following pipe;
every earlier comma branch evaluates independently against whatever context
enclosed the whole comma expression (the top-level input, or the LHS of an
outer pipe if the comma expression sits on a pipe's right-hand side). This
holds **everywhere in the grammar, not only at top level** — it is a fixed
entry in yq's operator table, not a special case for the first token:

- `.a | ., .b` groups as `(.a | .), .b`. `.b` runs against the *original*
  document (yielding `2`), not against `.a`'s value (`1`), which is why real
  yq raises no error here where jq's `.a | (., .b)` grouping raises "Cannot
  index number with string" evaluating `.b` against the number `1`.
- `.a | .b, .c | . + 1` groups as `(.a | .b), (.c | . + 1)` — two independent
  pipe chains joined by one comma, not one three-stage chain starting from
  `.a`. Real yq's first branch (`.a | .b`, indexing the number `1` with
  `.b`) fails exactly as jq's does, but yq drops the failing branch silently
  (no stderr, exit code 0) rather than jq's fail-loud behaviour, and prints
  only the second branch's `4`.

The `.a, .b | length`/`.a, .b | key` rows are captured verbatim for
completeness: the `1`/`1` `length` result (not jq's `1`/`2`) is real yq's own
scalar-`length` semantics (`2 | length` is `1` in yq, not jq's absolute-value
`2`) — a separate, unrelated divergence, not evidence against the grouping
rule above.

**Explicit parentheses restore jq's grouping**, because they are a hard
boundary the shunting-yard algorithm cannot see across: `(.a, .b) | . + 10`
and `.a, (.b | . + 10)` both match real jq's answer, regardless of which side
of the comma the parens wrap. **Construction brackets do not** —  `[...]`
only scopes evaluation to its contents, it does not change the relative
comma/pipe precedence inside them, so `[.a, .b | . + 10]` is
`[.a, (.b | . + 10)]` = `[1,12]`, not jq's `[11,12]`.

Per [ADR-0018](../../adrs/adr-0018.md) rule 2, mode decides: `succinctly yq`
follows yq's grammar here, not jq's. The parser change landed under
[#2420](https://github.com/rust-works/succinctly/issues/2420): `,` now binds
looser than `|` in yq mode at every depth (pinned by
`test_yq_pipe_precedence_holds_at_every_depth_2420`), parentheses and
construction brackets behave as captured above, and `succinctly jq` is
untouched (`test_jq_comma_still_binds_tighter_than_pipe_2420`). Three
existing yq-mode tests that had encoded jq's grouping were re-captured against
the pinned binary rather than kept.

Two divergences the capture exposed are **not** precedence. The first is now
fixed; the second stays open, pinned as the residual in
`test_yq_pipe_precedence_residual_divergences_2420`:

- [#2451](https://github.com/rust-works/succinctly/issues/2451) — **resolved.**
  Real yq applies a pipe's right side to the whole *list* its left side
  produced, so `(.a, .b) | (.c, .d)` on `a: {c: 3, d: 4}` / `b: {c: 5, d: 6}`
  prints `3 5 4 6`; succinctly printed jq's per-element `3 4 5 6`. Both sides
  are parenthesised, so no grouping rule reaches it — this was the evaluation
  model, and yq-mode now implements it (see
  [the context-list section](#yqs-context-list-evaluation-model-in-a-single-document-2451)).
  The oracle sweep's `comma_stream` class is gone with it.
- [#2452](https://github.com/rust-works/succinctly/issues/2452): under yq's
  own grouping `.a | .b, .c | . + 1` fails in its first branch; real yq drops
  that branch silently and exits 0, succinctly raises as jq does.

The `length` row above is also two things at once: real yq's rendered-width
rule for scalars, and a succinctly-internal inconsistency where the comma
fan-out path still applies jq's absolute-value rule
([#2453](https://github.com/rust-works/succinctly/issues/2453)).

### yq's context-list evaluation model in a single document (#2451)

Real yq has no scalar evaluation mode. `Context.MatchingNodes`
([`pkg/yqlib/context.go`](https://github.com/mikefarah/yq/blob/v4.53.3/pkg/yqlib/context.go))
is always a *list*, `|` hands its left side's whole list to its right side in one call
([`operator_pipe.go`](https://github.com/mikefarah/yq/blob/v4.53.3/pkg/yqlib/operator_pipe.go)),
and each operator decides for itself how to fan out over it.
[#2451](https://github.com/rust-works/succinctly/issues/2451) implements the three rules
that follow from that for a single document, in `succinctly yq` only (ADR-0018 rule 2 —
mode decides; `succinctly jq` keeps jq's model unchanged):

1. **A pipe into a union is branch-major.** `,`
   ([`operator_union.go`](https://github.com/mikefarah/yq/blob/v4.53.3/pkg/yqlib/operator_union.go))
   evaluates each of its branches against the same whole list and concatenates the branch
   results in order, so `(.a, .b) | (.c, .d)` is `3 5 4 6`, not jq's `3 4 5 6`.
2. **A union of two bare identities yields the list once.** `,` appends its right operand
   only when the two operands return different backing lists (a Go pointer comparison,
   there so two self-modifying operands are not double-counted), and `.` returns its input
   context object verbatim. With right-associative chaining that gives `2 2 4 6 8` output
   lines for one to five `.` branches over a two-element context.
3. **Binary operators fan out left-major.** `doCrossFunc`
   ([`operators.go`](https://github.com/mikefarah/yq/blob/v4.53.3/pkg/yqlib/operators.go))
   loops the left operand's matches outermost and re-evaluates the right operand inside
   that loop, so `(.a.c, .b.c) + (1, 10)` is `4 13 6 15` where jq gives `4 6 13 15`.

`map(f)` takes rule 1 too ([#6770](https://github.com/rust-works/succinctly/issues/6770)).
yq's `mapOperator` splats the node and runs `f` once over the whole list of members, so `map(. + 1, . + 2)` over `[1, 2, 3]` is `[2, 3, 4, 3, 4, 5]`.
`succinctly yq` writes a union-bodied `map` over a sequence or a non-empty mapping as `[.[] | f]`, which the context-list walk answers that way.
A scalar, `null` and an empty mapping keep the `map` arm, and `succinctly jq` still runs the body per member.
Three shapes still interleave, and the plain pipe `[.[] | (a, b)]` shares each: a union-bodied `map` on the right of an assignment (`|=`, `=`, `with`), one over a value the program computed first (`[.[]] | map(., 0)`), and a body that reads its own path or key (`map(path, key)`).
In a write target (`del(.a | map(., 0))`) it is as it was: yq changes nothing there, and the call is refused here.

Four related divergences are **not** covered and stay open:

- **`EvaluateTogether`** ([#2427](https://github.com/rust-works/succinctly/issues/2427)) is
  what makes `[...]` collect once across documents and binary operators go cartesian under
  `--eval-all`. It is a flag on the document nodes the all-at-once multi-file reader
  builds, never set inside a single document, so it changes nothing here. Implemented for
  `[...]` and for binary operators by #2427; `as` still consults it in real yq and does not
  here, which is the one row of that issue's capture table still open (see
  [that section](#-as-x--x-under---eval-all-collects-one-copy-per-document-not-one-per-binding-2427)).
  `ireduce` and an empty `{}` follow it too since #2427 ([that section](#src-as-x-ireduce-init-update-and--under---eval-all-read-the-whole-document-list-2427)).
- **A union of two assignments.** yq's pointer guard also fires for two operands that both
  hand back the *same, mutated* context, which is the case its own source comment names:
  `yq '(.a.c = 1), (.b.c = 2)'` prints **one** document carrying both writes. succinctly's
  assignments each return an independently edited document, so it prints two, each with one
  write. Collapsing the pair the way rule 2 does would *discard* the second write rather
  than de-duplicate it, so rule 2 is deliberately restricted to identity-transparent
  operands. Shared-node assignment is the same missing mechanism as the alias-node identity
  [the anchor-soundness section](#anchor-soundness-never-emit-yaml-we-cannot-read-back--rule-4a)
  records.
- **An operand that is a bare variable.** Rule 2's pointer guard fires for a `$var`
  operand too, and much harder than for `.`: real yq drops a union branch whose value comes
  from a bare variable, and collapses a whole fan-out piped into one. Found by #2072's
  differential fuzz and minimised to a *literal* binding, so it is not about node identity
  and #2072 does not touch it (confirmed live against v4.53.3 on `a: {b: 1}` / `c: {b: 1}`):

  ```console
  $ yq          '5 as $x | 1, $x'          # 1          <- the `$x` branch is dropped
  $ succinctly yq '5 as $x | 1, $x'        # 1 5
  $ yq          '5 as $x | (1,2,3) | $x'   # 5          <- three inputs, one output
  $ succinctly yq '5 as $x | (1,2,3) | $x' # 5 5 5
  ```

  A variable in the *first* branch is kept (`5 as $x | $x, 1` is `5 1` in both), which is
  what places this with rule 2's backing-list comparison rather than with variables as such.
  Not chased: collapsing a fan-out to one output is the direction that loses data, and
  nothing in the corpus needs it.
- **A missing-key stage in front of a union.** Rule 1 carries its context list as cursors,
  and an absent position (`.a.x` where `x` is not a key) is not a cursor, so a pipe whose
  prefix can land on one keeps spine 2416's path-context walk instead — which is exact for
  those pipes but element-major. `.a.x | (key, path)` is therefore right (`"x"`,
  `["a","x"]`), while `(.a, .b) | .c | (key, path)` keeps jq's interleaving. Every
  multi-valued shape the oracle sweep exercises has either an empty prefix or an iterating
  one (`.a[]`), neither of which can yield an absent node, so the sweep is clean.
- **A comma-fed or `//`-fed stage in front of a union (#2831).** A stage that produces several
  nodes through a comma (`.x | (.a, .b)`) or through `//` (`.x | ((.a, .b) // 9)`) hands the
  union that follows an element-major context, so `(key, key)` and `(path, path)` after it come
  out as `"a"` `"a"` `"b"` `"b"` where yq prints the whole context twice (`"a"` `"b"` `"a"` `"b"`).
  Captured on `x: {a: 1, b: 2}`:

  | filter                                 | yq v4.53.3                          | `succinctly yq`                     |
  |----------------------------------------|-------------------------------------|-------------------------------------|
  | `.x \| (.a, .b) \| (key, key)`         | `"a"` `"b"` `"a"` `"b"`             | `"a"` `"a"` `"b"` `"b"`             |
  | `.x \| ((.a, .b) // 9) \| (key, key)`  | `"a"` `"b"` `"a"` `"b"`             | `"a"` `"a"` `"b"` `"b"`             |
  | `.x \| ((.a, .b) // 9) \| (key, path)` | `"a"` `"b"` `["x","a"]` `["x","b"]` | `"a"` `["x","a"]` `"b"` `["x","b"]` |

  The same union after a navigated source is right (`(.x.a, .x.b) | (key, key)`, `.x[] | (key, key)`,
  `.x | (.a // 9, .b // 9) | (key, key)`), and a value-only union agrees on the same shape
  (`.x | (.a, .b) | (., .)` is `1` `2` in both: yq's union drops a right operand that is the very same
  list, #4183). Only the path-tracked builtins (`key`, `path`) show the order, because only they read
  where a node stands. The `//` case is #2817's per-left-output rule composed with a multi-output
  rest, which its sweep's `alt_l`/`alt_r` wrappers did not generate.

### `from_entries`/`with_entries` want exact `key` and `value` fields — resolved ([#4399](https://github.com/rust-works/succinctly/issues/4399)); a container key and duplicate entries remain

Real yq's `from_entries` reads exactly a `key` and a `value` child of each entry (`parseEntry`),
case-sensitively, with no alias set and no truthy-fallback chain, and refuses an entry that
lacks either, or is not a mapping at all (`null` and scalars included):

```console
$ yq -n '[{"name":1,"value":1}] | from_entries'   # expected to find one 'key' entry but found 0 in position 0
$ yq -n '[{"key":1,"Value":9}] | from_entries'    # expected to find one 'value' entry but found 0 in position 0
$ yq -n '[{"key":false,"value":1}] | from_entries'  # {"false":1} -- a falsy key is a key
```

`succinctly yq` matches all of these since #4399 (`entries_to_object`'s yq branch,
`yq_entry_key_and_value`; the `'key'` is checked first, and the position is the entry's index).
`succinctly jq` keeps jq's own alias chain and truthy fallback (`ENTRY_KEY_ALIASES`),
unchanged. Four things remain: a `key` that is a non-empty array or mapping (yq: `{"":2}`; here
still `Cannot use array ... as object key`, see `yq_object_key_stringify`); duplicate `key`
entries in one source mapping, which collapse before the count (yq reports `found 2`);
duplicate *output* keys (`[{"key":"a","value":1},{"key":"a","value":2}] | from_entries` is
`{"a":1,"a":2}` in yq and `{"a":2}` here, as `OwnedValue` holds each key once); and a YAML
`~` key, which yq spells `"~"` and this renders as `"null"`.

### `type` answered jq's type name instead of the YAML tag — resolved ([#2516](https://github.com/rust-works/succinctly/issues/2516)); an alias through a constructed array/object is a residual gap

Real yq's `type` is a plain alias of `tag`: it answers the YAML tag (`!!str`, `!!int`,
`!!map`, ...), not jq's type name (`"string"`, `"number"`, `"object"`, ...). succinctly
reproduced jq's naming in yq mode too before this fix. Captured live against yq v4.53.3
(`-o=json -I0`) on `a: 1`:

```console
$ yq 'type'          !!map        $ succinctly yq 'type'        object   (was)
$ yq '.a | type'     !!int        $ succinctly yq '.a | type'   number   (was)
$ yq 'tag'           !!map        (already correct — `tag` already answered the YAML tag)
$ yq 'kind'          map          (unaffected — `kind` is a different builtin, not a `type` alias)
```

On other tagged/untagged scalars (`-n`, `--jq-extensions` makes no difference to any row):
`!!str 1` => `!!str`; `!!int "5"` => `!!int`; `!!float 3` => `!!float`; `!!bool "yes"` =>
`!!bool`; `!!null "~"` => `!!null`; a custom `!mytag hello` => `!mytag` (echoed back
verbatim, not `!!str` — real yq has no notion of "recognized" vs "unrecognized" tags for
this purpose); a quoted `"1"` => `!!str`.

Fixed as `eval_generic::yq_type_tag` (the cursor-aware route, correct for both standard
*and custom* explicit tags) and `eval::yaml_type_tag`/`owned_yaml_type_tag` (the two
`OwnedValue`-only routes, implicit tags only — see below), consulted by `Builtin::Type`'s
own yq-mode arm in both evaluators. `Builtin::Tag` gained a native cursor-aware arm in
`eval_generic.rs` too (it previously fell to the generic reindex-bridge fallback, which
already had this exact same-shaped custom-tag gap) — both builtins now agree with each
other and with real yq.

**Custom tags are only correct when a cursor is available** (the overwhelming majority of
call sites — any read through a real YAML document).
Most filters that build a value out of document nodes keep the node's tag, as yq does:
`map(.)`, `[.a]`, `[.a, .b]`, `{"k": .a}`, `to_entries`, `with_entries(.)` over a mapping, `sort`, `reverse`, `unique` and `. + []` over a sequence.
So does a sequence built from nodes ([#5429](https://github.com/rust-works/succinctly/issues/5429)): `[.[]]`, `[.[] | select(...)]`, `[(.a, .b)]`, `[.a] + [.b]`, a literal-bound slice (`.[1:]`, `.a[:2]`), `flatten`, `flatten(n)` and `map_values(f)`, so `[.Outputs[]]` over a CloudFormation template prints `Value: !Ref Bucket`.
A custom tag is still lost when the value leaves cursor tracking on one of the routes
[#5429](https://github.com/rust-works/succinctly/issues/5429) still lists (a variable, `.a as $x | [$x]`; a further stage after a construction, `[.a] | .`; an iterating operand inside one, `[.a[].b] + [.c]`; a computed slice bound; `map_values(select(...))`), on `with_entries(.)` over a sequence, on most filters under `--eval-all` ([#4380](https://github.com/rust-works/succinctly/issues/4380)), and in a `load()`ed document.
The tag is lost there together with the node's comment, anchor and quoting, because those routes keep none of a node's presentation.
Input that is not YAML carries no tag to lose.
This is not a regression from this fix: `tag`'s own prior, unconditional (non-cursor) implementation
already had the identical limitation; this fix only *adds* the cursor-aware case that
real yq always has, it does not remove the old one.

**A residual gap, discovered alongside, not fixed:** real yq's `type`/`tag` answer the
empty string for an alias occurrence (`*name`) rather than dereferencing to its target's
tag — confirmed live, `y: *a` where `a: &a !!str 1` gives `.y | type` => `""` while `.x |
type` => `"!!str"`. succinctly's `yq_type_tag` matches this for a direct, cursor-forwarded
read (checked via `DocumentCursor::is_alias` ahead of the explicit-tag lookup). It does
**not** survive array/object *construction*, though: `[.x, .y] | map(type)` answers
`["!!str","!!str"]` in succinctly against real yq's own `["!!str",""]`, because
construction resets cursor/position context for its own elements before `type` ever runs
— the same reset `key`/`path` already get inside `{...}`/`[...]` (this doc's own "Partial
Implementation Notes" #3 in `docs/reference/yq-language.md`), not something new to this
fix. `(.x, .y) | type` (no construction) answers correctly. See
`tests/yq_cli_tests.rs`'s `test_yaml_explicit_tag_resolves_through_alias_903`.

### A bound variable is a value, not the node it was bound to — resolved ([#2072](https://github.com/rust-works/succinctly/issues/2072)); an absent-key binding and an alias's flow style are residual gaps

Real yq's variables hold *nodes*: `E as $x | body` binds the candidate nodes `E` produced,
parent pointers and all, so a bare `$x` later in the pipe still knows where it came from.
succinctly used to substitute the bound **value** into the body (`substitute_bound_var`,
[src/jq/eval.rs](../../../src/jq/eval.rs)), and a value has no position of its own, so
every position builtin read through `$x` answered from the ambient node or from nothing at
all. #2072 closed that: `Expr::TrackedVar` now carries a `BoundVar` whose `BindOrigin`
names the node the value came from — a document node as its
`DocumentCursor::node_id` plus the owning document's `document_token`, or an owned-tree
position for a binding made inside the owned identity pipe — and the use sites re-resolve
it to a live cursor (`each_as_generic`, the path-context walk's variable step, and
`OwnedIdentityRule::Bound`, all in
[src/jq/eval_generic.rs](../../../src/jq/eval_generic.rs)). The value-equality
approximation survives only as the fallback for a binding made where no cursor domain
exists, and the document token is checked before an id is trusted, so a variable bound in
one document and used in another (`--eval-all`, the reindex bridge) falls back to its
value rather than to a same-numbered node of the wrong tree.

Re-captured 2026-09-08 from yq v4.53.3 (`-o=json -I=0`) on `a: {b: 1, e: 2}` /
`c: {b: 1}` — the same eight rows the open version of this section listed, every one of
them now agreeing:

| filter                              | real yq     | succinctly  |
|-------------------------------------|-------------|-------------|
| `.a.b as $x \| $x \| key`           | `"b"`       | `"b"`       |
| `.a.b as $x \| $x \| path`          | `["a","b"]` | `["a","b"]` |
| `.a.b as $x \| .a \| $x \| key`     | `"b"`       | `"b"`       |
| `.a.b as $x \| $x \| parent \| key` | `"a"`       | `"a"`       |
| `. as $x \| $x \| key`              | (nothing)   | (nothing)   |
| `.a \| . as $x \| $x \| key`        | `"a"`       | `"a"`       |
| `.a.x as $x \| $x \| key`           | (nothing)   | (nothing)   |
| `.a as $x \| $x \| .b \| key`       | `"b"`       | `"b"`       |

`line`/`column` agree too (`.a as $x | $x | line` is `2` and `| column` is `3` in both on
that document), which the value model could not answer at all: they are cursor-only and are
never derivable from an accumulated path.

Read-side residues remain, all of them predating #2072 and none caused by the
binding carrying an origin; the write side is #2643's (below).

**A variable bound from an absent key — resolved (#2650).**
Real yq's `as` binds a *candidate list*, and a key lookup that finds nothing contributes no candidate, so the variable is never bound while the same key read without a binding is `null` at a real position.
`succinctly yq` now reads a bare `as` source under the same read-only absent-key rule as a binary-operator operand (#2470), on every route that binds one: no candidate, so the body runs once with the variable unbound (#4139) and `$x` reads as no output.
Captured on `a: {b: 1}` / `c: {b: 1}` (`-o=json -I=0`), both tools now agree:

```console
$ yq '.c | (.a as $x | $x)'          # (nothing)
$ yq '.c | (.a as $x | $x) | path'   # (nothing)
$ yq '.c | .a | path'                # ["c","a"]   <- unbound, both tools agree
```

Only the *source* is read-only: the body keeps the scope it had, so `.zz as $x | .zz = 3` still creates `zz`.
A numeric index on a mapping (`.[0] as $x | [$x]`) is not covered, since the read-only rule is keyed on a key lookup, not on an index.
A stage after the absent key that does not read its context still runs against the empty context the lookup leaves, as in yq (#5261).
`(.zz | error("x")) as $x | 1` raises, `(.zz | "s") as $x | $x` binds `"s"`, `(.zz | [.]) as $x | $x` binds `[]`, and `(.zz | length) as $x | [$x]` binds nothing, all as in yq v4.53.3.
It is the rule an assignment's right side and an `and`/`or`/arithmetic operand already follow (#2588, above): a source that bound nothing is re-run once as `yq_empty_context_reemit` rewrites it, inside the scope that emptied it.
The root cause the issue named was wrong: yq does not run the later stage on a `null` node it creates, it runs it on zero nodes, which is why `length`, `tostring` and `+` bind nothing while `error` and a literal still run.
A comma source where one branch comes up empty and another does not (`(.a, (.zz | error("x"))) as $x | [$x]`, `((.zz | "a"), "b") as $x | [$x]`) still sees only the branch that produced output, since the rewrite applies only to a source that bound nothing.
That holds on the path and write routes too: `del(((.zz | "a"), "b") as $k | .[$k])` deletes `.a` in yq and nothing here.
The general form of that gap is a pipe whose left side is empty, outside any `as`: `select(false) | error("x")` raises in yq and is no output here.
The retry replays the stages from the first context-free one on, so a stage there with an effect (`debug`, `stderr`) runs again when the first pass had already reached it and still bound nothing, on the path and write routes as on the value routes.
An `as` that sits in a path position or the left side of a write retries the same way (#5278): `del((.zz | error("x")) as $x | .a)` raises, and `del((.zz | "a") as $k | .[$k])` deletes `.a`, as in yq.
A constant bound there and then read as a path (`del((.zz | "s") as $x | $x)`) refuses with `Invalid path expression`, where yq leaves the document alone, exactly as the non-empty spelling `del("s" as $x | $x)` does; the retry moved the first from a coincidental match into that existing divergence.
A rewritten source that reads its position (`.zz | [.] | key`) is not run on the owned-identity route, since the node it constructs has no position of its own there; the body runs unbound, as yq answers.

**An anchor's flow style read through an alias binding.** Captured on
`a: "1" # keep` / `b: &anc [1, 2]` / `c: *anc` (YAML output):

```console
$ yq          '.b as $x | $x'   # &anc [1, 2]
$ succinctly yq '.b as $x | $x' # &anc [1, 2]   <- the anchor and its flow style survive
$ yq          '.c as $x | $x'   # *anc
$ succinctly yq '.c as $x | $x' # - 1
                                # - 2
$ succinctly yq '.c'            # [1, 2]        <- unbound, the flow style survives
```

The `*anc` half is the anchor-soundness rule working as intended, not a gap: real yq emits
an alias whose anchor appears nowhere in the output and cannot read its own answer back
([the anchor-soundness section](#anchor-soundness-never-emit-yaml-we-cannot-read-back--rule-4a),
rule 4(a)). What *is* a gap is the flow style: `.c` alone prints
`[1, 2]`, and through the binding the alias's origin resolves to the alias node, whose own
style mark is not the anchor node's, so the value is re-emitted in block style.

**A write through a variable changes the bound copy ([#2643](https://github.com/rust-works/succinctly/issues/2643)).**
Real yq's `as` copies the node into the variable (`variableLoopSingleChild` stores `candidate.Copy()`).
A write through `$x` (`$x.b = 5`, `$x[0] |= f`, `$x.b += 1`, `del($x.b)`) therefore changes the copy and leaves the document alone.
Every later read of `$x` sees the change, the write stage answers its input, and the right side of `=` and `op=` is evaluated against that input and not against `$x`.
It was refused (`Invalid path expression`, exit 1) until #2643, apart from an identity-bound `$x` (`. as $x | $x.a.b = 5`), which wrote the document.
Captured on `a: [1, 2, 3]`:

| filter                          | real yq (exit 0) | succinctly      |
|---------------------------------|------------------|-----------------|
| `.a as $x \| $x[0] = 9`         | `a: [1, 2, 3]`   | `a: [1, 2, 3]`  |
| `.a as $x \| del($x[0])`        | `a: [1, 2, 3]`   | `a: [1, 2, 3]`  |
| `.a as $x \| $x[0] \|= 9`       | `a: [1, 2, 3]`   | `a: [1, 2, 3]`  |
| `.a as $x \| $x[0] \|= 9 \| $x` | `[9, 2, 3]`      | `[9, 2, 3]`     |

The parser does it in yq mode (`yq_var_write_rebind` in [src/jq/parser.rs](../../../src/jq/parser.rs)): the first stage of an `as` body that writes a path rooted at `$x` becomes a rebinding of `$x` to the written copy, and a later such stage is rewritten the same way, so both evaluators and every output route see one shape.
`succinctly jq` matches jq 1.7.1 (`Invalid path expression ...`, exit 5) and is not affected.

Still different, each in yq v4.53.3 against `succinctly yq`:

- **A bare `del($x)`.**
  yq deletes the node `$x` was bound to from its live parent, by key or index (`.a as $y | del($y)` is `{"c":{"b":1}}`, `.a as $y | .c | del($y)` is `{"b":1}`, `.a.b as $y | .a | del($y)` is `{}`), and keeps the document when `$y` is the root.
  `succinctly yq` refuses a variable bound below the root with `Invalid path expression`.
- **A variable-rooted path inside the left side of a write.**
  `(.a as $y | .c | $y.b) = 5`, `(.a as $y | .a | $y.b) = 5` and `del(.a as $y | .c | $y)` keep the document, or delete `.a`, in yq, and refuse here.
  `(. as $y | $y.a) = 5` keeps the document in yq and writes `.a` here.
- **A write inside a construction.**
  `.a as $y | [$y.b = 5]` and `.a as $y | {"k": ($y.b = 5)}` collect the unchanged document in yq, and refuse here.
  Only stages of the `as` body's pipe, of a pipe nested in it, and of the body of a nested `as` are rewritten.
- **A stale index.**
  `.a[1] as $y | del(.a[0]) | del($y)` deletes by the live parent and the old index in yq, and refuses here.
- **`.[] as $y | del($y)`.**
  Both results are the one mutated root in yq (`{}` twice), where each binding here has its own copy.
- **The presentation of the copy.**
  A written copy of a flow mapping prints in block style (`.a as $y | $y.b = 5 | $y` over `a: {b: 1}` is `{b: 5}` in yq and `b: 5` here), as `.a as $y | $y | .b = 5` already did.

### Metadata assignment (`PATH <slot> = value`): what `succinctly yq` refuses or drops (#798 PR1)

`succinctly yq` parses real yq's juxtaposed metadata-assignment grammar (`PATH <slot> =
value` / `PATH <slot> |= filter`, e.g. `.a style = "flow"`) for all seven of yq's metadata
op-tokens (`line_comment`, `head_comment`, `foot_comment`, `comments`, `style`, `tag`,
`anchor`) — `src/jq/parser.rs`'s `try_parse_meta_op`/`Expr::MetaAssign`. The whitespace is
the discriminator, as in yq's own lexer: `.style = 2` is a write to a field named `style`,
`. style = "flow"` is the root's own style. `line_comment =`, `style =` and `anchor =` are
applied by `src/bin/succinctly/yq_runner.rs`'s `resolve_meta_assign_writes`/
`apply_meta_assign_writes` (modeled on the existing `propagate_assign_alias_marks`): the
target's candidates come from evaluating `path(TARGET)` against the document the stage
sees, so `.[]`, `..`, `.a[-1]` and a computed index all write every candidate, a missing
target is created (`.a.b line_comment = "y"` on `c: 1` prints `a:\n  b: null # y`, `.a[2]`
pads with `null`s), `=`'s right-hand side is evaluated once against the stage's input and
`|=`'s once per candidate against that candidate's value, and an earlier write in the
pipe is visible to a later stage's right-hand side (`.a = 2 | .b line_comment = (.a |
tostring)` writes `# 2`) — every one of those live-verified against pinned v4.53.3 and
pinned by a `meta_assign_*_798` golden.

A right-hand side that produces no node takes the empty text (#4526), as in yq: an absent path
(`.a line_comment = .zz`, `.b.zz`, `.zz.y`), a container (`= .b` over a mapping or sequence, `= []`),
a variable bound to one, and no output at all (`select(false)`, `|= .zz`) write nothing where there was
nothing and clear an existing comment, style or anchor, exactly like `= ""`. `null`, a scalar, a
`{}` the expression builds (`{}`, `{} | .`; yq reads it as its text, where a `{}` read from the document is no node) and the first node of several are nodes. One shape differs:
absentness is read from a right-hand side that is a path (`path(RHS)` answers one path per output),
so it does not survive a stage after it, and `.a line_comment = (.zz | tostring)` writes `# null`
where yq writes nothing; an out-of-range positive index (`.b[5]`) reads as absent here where yq
pads `b` with `null`s and writes `# null`.

What is refused explicitly (an error, never a silent no-op — the outcome #798's triage
ruled out), even though real yq supports every one of them:

- **`head_comment =`/`foot_comment =`/`comments =` with a non-empty text on
  anything but the document root, a scalar mapping value / sequence item in a block
  collection, or a mapping key (`(.a | key) head_comment = "x"`)** raise `<slot> = ... is
  not yet supported`. Real yq's `.a head_comment = "hi"`/`.a foot_comment = "bye"` insert standalone comment
  lines, and `.a comments = "x"` sets head, line and foot together. Where yq *prints* a comment set on a node follows
  go-yaml's emitter state rather than the node (a scalar mapping value's head lands after
  its line, before the next key's own head, which then replaces it; a container's foot lands
  after the next entry's first line), which the emitters here do not model. See the
  identity-round-trip entry below.
- **`tag =` is written on a value node** ([#2796](https://github.com/rust-works/succinctly/issues/2796)),
  applied as go-yaml's encoder treats an assigned tag: written unless it is the tag the scalar's
  text resolves to anyway (`go_yaml_resolved_tag`: `!!int` on `1` and `!!timestamp` on `2001-12-14`
  print nothing, `-0` is a float), `!!str` on a scalar that reads back as another type quotes it and
  makes it a string (`a: 1` is `a: "1"`, and `"1"` under `-o json`), a tag that does not start with
  `!` is `!<tag>`, a node that already has a source tag prints the new one even where it is its own
  (`!!str 1` with `!!int` is `!!int 1`), and `""` clears it. 180 text/tag pairs over 15 scalar spellings
  and 120 more over source-tagged scalars, containers, sequence items, several targets, `-o json` and
  `|=` match yq v4.53.3 byte for byte. Still different: `-o json` does not type a value by a
  non-`!!str` tag (`!!float` on `1` is `1.0` in yq, and `!!int` on `x` is an error there), the
  `tag = ""` of a source-tagged `!!float`/`!!int` scalar keeps its tag/quoting where yq prints the
  plain text, a tag on a key node (`(.a | key) tag = ...`) and on `...` is refused, a tag written through an alias
  position reaches the alias copy (yq leaves the alias and changes the anchor's node), and the
  write is not visible to a later stage of the same pipe (`.a tag = "!!str" | .a | type`), and two
  `tag =` writes to one node in a pipe (`.a tag = "!!float" | .a tag = "!!str"`) are not composed the
  way yq composes them. A `tag =` on a route that places no metadata write (behind a comma, an array
  construction, a bind) or under `--eval-all` is an error (`tag = ... is only supported as a top-level
  pipe stage of a plain write`), never a silent no-op.
- **The document root takes a text** ([#2796](https://github.com/rust-works/succinctly/issues/2796),
  part 2): `. head_comment = "x"`, `. foot_comment = "x"` and `. comments = "x"` (and `|=`)
  replace the root's own standalone lines, a multi-line text becoming one `# ` line each, and
  `comments` also sets the line comment a flow or empty container root prints (`# x\n{a: 1} #
  x\n# x`). A block root keeps the lines that belong to its entries: a trailing comment after
  a bare last entry is that entry's and stays (`a: 1\n# oldfoot` + `. foot_comment = "x"`
  prints both), while one after a last entry that carries a line comment is the root's own
  and is replaced, exactly as yq's parser attaches them. A scalar root takes none of it (`42`
  stays `42`); a root head write replaces the first document's `---` marker and the comments
  around it, as yq does ([#6903](https://github.com/rust-works/succinctly/issues/6903): `... comments = ""` strips them), and a head, foot or `comments` write on a
  route that keeps no comments of its own (a comma, a constructed value, a bind) is refused
  rather than printed without it. Only a write whose every candidate is the root: `.. head_comment = "x"` visits
  every node and still raises. Checked by 11 `meta_assign_root_*_2796` goldens, the
  document-shape sweep (flow, empty, scalar and multi-document roots) and a differential fuzz
  of ~2,800 root writes over random commented documents, none differing.
- **A scalar mapping value or sequence item takes a text** ([#2796](https://github.com/rust-works/succinctly/issues/2796),
  part 3): `.a.b head_comment = "x"`, `foot_comment` and `comments`, where yq prints it is
  where go-yaml's emitter flushes it, not above or below the node (all measured against
  v4.53.3). A mapping value's head waits for the next key and prints just above it at its
  indent (after the last value it trails the entry), and a head the next key already owns
  replaces it, so the write is lost; its foot follows the entry with a blank line unless it
  is the last; a sequence item's head is above the item and its foot below with the same
  rule; `comments` is the line comment, then the foot, then the head (`a: 1 # x`, `# x`, a
  blank, `# x`, `b:`), and after a last entry head, a blank line and foot. In this tree the
  lines are the member's own head and foot (what `# mid` above `b:` is): the next member's
  head for a value head, this member's foot for the rest. A node, or its successor, that
  already owns head or foot lines is refused rather than merged (the order yq flushes them
  in is not modelled), as is anything under a flow collection, a container target's foot or line comment (its head is part 5, below) and a text of
  newlines alone.
  Several targets (`.[] head_comment = "x"`, `(.a, .b) comments = "x"`, `|=` too) are written one by one.
  Each is judged against the comments the document held before the assignment, so the head a value leaves on the next key is not taken for a head that key owns.
  A target whose neighbour held lines before the assignment is refused as a single target is.
  Targets that overlap (a collection and an entry inside it) are refused in either order.
  Checked by 13 `meta_assign_entry_*_2796` goldens, 16 `meta_assign_multi_*_2796` ones, and a differential fuzz of ~5,300 accepted writes
  over random documents with and without comments (none differed; ~4,300 more were refused).
  A 348-row matrix of several targets (12 setter forms x 29 target shapes over documents with and without comments) has 304 rows that match yq, and the rest are refused; none is silently different.
- **A mapping or sequence value takes a head** ([#2796](https://github.com/rust-works/succinctly/issues/2796),
  part 5): `.a head_comment = "x"` on a non-empty block mapping or sequence prints inside it,
  above its first entry at that entry's indent (`a:\n  # x\n  b: 1`), and above the dash for a
  sequence item (`# x\n- a: 1`); a head the first entry already owns wins in yq, and the write
  is refused here instead. Its foot and line comment, an empty collection and a flow one
  stay `not yet supported` (yq prints a container's foot after the *next* entry's first
  line). An alias target (`c: *x`) is refused too, and so is clearing the head of a
  container whose head the same expression just wrote (it sits on the first entry, which the
  clear does not reach). Checked by 7 `meta_assign_container_head_*_2796` goldens and a differential fuzz
  (~2,800 accepted writes, none differing).
- **A mapping key takes a text** ([#2796](https://github.com/rust-works/succinctly/issues/2796),
  part 4): `(.a | key) head_comment = "x"`, `foot_comment`, `line_comment` and `comments`, the
  natural way to put a comment above a key. yq keeps the comments above and below an entry on
  its key, which is what this tree stores as the member's own head and foot lines, so the head
  goes above the key (replacing what was there: `# mid` above `b:` is `b`'s head), the foot
  below the entry (blank line after a non-last one, none after the last) and the line comment
  on the key's line, which for a scalar value is the value's own; `comments` is all three,
  head first (`# x`, `a: 1 # x`, `# x`, a blank, `b:`). A foot or a line comment already
  there (go-yaml prints a key's line comment on a later line when the value has one), a line
  comment on a container value (a different slot), anything under a flow collection,
  `style`/`anchor` and a newline-only text are refused.
  Several key targets are written when every target is a key node (`(.[] | key) head_comment = "x"`, `((.a | key), (.c | key)) comments = "x"`, `|=` too), since a key keeps its comments on its own member ([#6792](https://github.com/rust-works/succinctly/issues/6792)).
  A key node among value targets (`((.a | key), .b) head_comment = "x"`) is refused: a value's head is the next key's, and the order go-yaml's emitter resolves the two in is not modelled.
  Before #6792 that key target was dropped without a word, as was a `(PATH | key)` branch of any union.
  The same key named twice among the targets (`((.[] | key), (.a | key))`) is written as yq writes it.
  Checked by 11 `meta_assign_key_*_2796` goldens, 9 `meta_assign_keys_*_6792` ones (two of them the refused mixed shapes) and, for #2796, a differential fuzz of ~7,200 accepted writes over
  random documents with and without comments (none differed; ~2,000 more were refused, counting several key targets, which #6792 writes).
  A 224-row matrix of key targets against yq has 116 rows that match, and none silently different (before: 120 rows exited 0 with a different result).
- **The clearing forms are written** ([#2796](https://github.com/rust-works/succinctly/issues/2796),
  part 1): `head_comment = ""`, `foot_comment = ""`, `comments = ""` and `line_comment = ""`,
  `=` or `|=`, on any target, and `...` (yq's recursive descent that also visits mapping
  keys) as the target of a comment write, so the idiom `... comments = ""` strips every
  comment in the document. yq keeps a standalone comment on the node it sits next to: the
  lines above `b:` in a mapping belong to the key `b`, the ones above a sequence item to the
  item, the ones after the last entry to the document root. `...` clears the key nodes'
  too; `..` and a path address values only, so `.a head_comment = ""` leaves the `# mid`
  above `b:` alone and `.. head_comment = ""` clears the root's and the sequence items'
  lines. `...` as a target takes the comment slots only and refuses
  `style`/`anchor`, whose key-node forms are not modelled; as a value it is the walk of #7069 (below).
  Checked against pinned v4.53.3 by
  the `meta_assign_clear_*_2796` goldens and a differential fuzz of ~5,900 clearing
  operations over random commented documents (none differed). Text of newlines alone
  (`head_comment = "\n"`) is a text, not the clearing form: yq writes blank comment lines for
  it, so it raises the `not yet supported` error above. A metadata write is resolved once,
  against the first document, so a filter that asks which document it is on
  (`select(di == 1) | .b comments = ""`) is refused rather than silently writing nothing for
  the later ones (this also covers `line_comment`/`style`/`anchor`, which used to drop the
  write quietly); and a route that keeps no comments at all (`-o json`, a filter with several
  top-level outputs such as `(.a comments = ""), 1`) ignores a comment write as it always has.
  A comment after a sequence item's dash (`- # c\n  k: 1`) is lost on any DOM write here, a
  gap that predates this and that the fuzz skips.

- **`...` as a value is the walk** ([#7069](https://github.com/rust-works/succinctly/issues/7069), `tests/yq_cli/i7069.rs`): the node, then each mapping key before its value and each sequence element, depth first, so `[...] | length` is 7 over `a:` / `  b: thing` / `  c: something` where `[..] | length` is 4, and `... | select(tag == "!!str")` names the keys too.
  It was a parse error outside a metadata assignment.
  Checked against pinned v4.53.3 on 32 rows over a mapping, a nested one and a sequence (`[...]`, `[... | tag]`, `map(...)`, `.a | [...]`, `select` and `length` over it).
  A key is a string here and nothing more: `[... | key]`, `[... | path]` and `[... | line]` over the nodes answer `[]`, empty paths and line 0 where yq answers each node's own (`[... | path]` is `[[], ["a"], ["a"], ["a", "b"], ...]`, `[... | line]` the lines of the nodes), and a key's style, tag and comment are not there.
  A key keeps the type of its spelling in yq and is a string here (`1: a` is the int `1` and `true: b` the bool there, `"1"` and `"true"` here), and a `<<` merge key is walked as its members here, where yq walks the `!!merge` key and the alias.
  A value that is an alias is walked into, where yq stops at it (`b: *x` over `a: &x {p: 1}` is 9 nodes here and 7 in yq), as `..` does (#4223).
  `(... | select(...)) |= f` is refused (`Invalid path expression`): the walk is not a path expression, and yq writes through it.
- **`style` of a tagged node** is `"tagged"` when only the tag styles it and `"<unknown>"` when it also has quotes, a block scalar or a flow collection ([#6881](https://github.com/rust-works/succinctly/issues/6881)), as in yq.
  Over an owned value (`with_entries(.value |= style)`) there is no node to read, and the getter answers `""` as it did for an untagged one.
  A tagged mapping key read through `to_entries` is the same (yq `"tagged"`, here `""`).
- **`style = "tagged"`** renders the tag of the node's current value type
  ([#4066](https://github.com/rust-works/succinctly/issues/4066), the last piece of #2707):
  `!!int`/`!!float`/`!!bool`/`!!null`/`!!str` before a scalar, `!!seq`/`!!map` on the header
  line of a block container (before a flow or empty one), after any anchor. An unknown name
  still raises real yq's own `unknown style <name>`. `literal` and `folded` render since
  [#2707](https://github.com/rust-works/succinctly/issues/2707): a string with a line break
  and no style of its own is also written as a block scalar, and an existing one survives a
  DOM write (`a: |\n  x` + `.b = 1` keeps `a: |`), all ported from libyaml's emitter and
  checked against yq v4.53.3 on ~5,000 random strings (tagged strings on another ~2,000).
  Two go-yaml folded-writer quirks are reproduced by the write route rather than fixed, so a folded value does
  not always read back identically in yq either: a kept (`>+`) trailing run gains one empty
  line, and a more-indented line (`x\n y`) gets an empty line before it.
  The streaming route (`.`) reproduces the first and not the second: it writes the more-indented line without the empty line, because yq's output reads back as a different value (an extra newline, and it is not idempotent), which ADR-0018 does not follow (#836).
  So `a: >` / `  x` / `    y` differs between `.` and a write here, and from yq on `.`.
  A scalar that starts with blank lines is written as yq writes it on both routes (#6446), and a `>+` of nothing but blank lines gains no trailing empty line.
  The divergences that
  remain for `tagged`: `1_000` is a string here where yq types it `!!int`;
  `2024-01-01` is `!!str` where yq has `!!timestamp` (no timestamp type); an explicit source
  tag is not kept (`a: !!float 1` prints `!!int 1`, [#1416](https://github.com/rust-works/succinctly/issues/1416));
  `~` prints `null`; and a style is applied to the node's *final* value (the known
  no-read-after-write gap), so `.a style = "tagged" | .a = 3.5` prints `!!float 3.5` where yq
  drops the tag when the new value's type differs.
- **Explicit tags survive a write** ([#4078](https://github.com/rust-works/succinctly/issues/4078)).
  A DOM write (`=`, `|=`, `del()`, `-P`, `-i`, `--arg`) re-emits every explicit tag of the
  document it read: `!Ref`/`!GetAtt`/`!Sub` and application tags on scalars, mappings and
  sequences, and `!!str 1`, `!!float 1`, `!!int "2"`, before a scalar, on the header line of a
  block container behind any anchor, and before a flow or empty one. They used to be dropped,
  which turned a CloudFormation template into a different document on any edit. A written node
  keeps a custom tag and loses a core tag its new value no longer has (`!!str 1` written `5` is
  `5`, written `"x"` stays `!!str x`; `!!int "5"` written `6` is `!!int "6"`); `-P` drops the
  core tags that only restate a type and keeps the rest. Checked against yq v4.53.3 on ~1,900
  random tagged documents.
  A node a write copies from a plain path (`.b = .a`) keeps its tag, as in yq ([#5257](https://github.com/rust-works/succinctly/issues/5257)).
  A node copied from a constructed right-hand side (`.c = [.a]`) loses it, with its comment and quoting
  ([#5249](https://github.com/rust-works/succinctly/issues/5249)).
  A right-hand side that is computed but names one node (`.c = (.a // 1)`, `.c = (.a | select(true))`) is read as that node, and keeps all three.
  Not matched: `!!set` members print `null` where yq prints `''`; and a tagged `null` that a path write walks through
  (`a: !Foo null` + `.a.b = 5`) becomes a mapping where yq leaves the document alone. A tagged
  empty value (`a: !Foo` with nothing after it) is written `!Foo null`. A tag on a mapping *key*
  (`!Foo k: v`, `? !K key`, an implicit pair `[!Foo k: v]`) is kept too
  ([#4088](https://github.com/rust-works/succinctly/issues/4088)), as is a tag on a flow
  collection value (`a: !!seq [1, 2]`) on the streaming route; `-P` keeps a core tag on a
  container it does not restate (`!!str []`, `!!seq {}`). Still not matched: a core `!!null`
  on a container (`a: !!null [1]`) is dropped by `-P` and kept by the default route, where yq
  turns the value into `null` on both ([#4111](https://github.com/rust-works/succinctly/issues/4111)). A key's tag
  is kept when a write copies its mapping from a plain path (`.a = .b` over a tagged key in `.b`),
  and lost when the right-hand side is constructed or bound (`.a = [.b]`, `. as $x | .a = $x.b`,
  [#5249](https://github.com/rust-works/succinctly/issues/5249)).
  A key deleted and written again in the same filter is a new node and keeps
  none of the old key's anchor, tag, quoting or key comment, and nothing beneath it either: a
  literal, a compound (`.k += "z"`) or a nested (`del(.m) | .m.x = 5`) write there starts from a
  new node ([#4113](https://github.com/rust-works/succinctly/issues/4113));
  `.a |= <closed literal>` is the same write as `=`, as in yq. Still not matched: a value copied from
  another node (`del(.k) | .k = .j`, `.k as $x | del(.k) | .k = $x`) takes that node's comment and
  anchor in yq, where the by-key reconciliation hands it the deleted key's own.
- **`anchor = "<name>"` with a name go-yaml's emitter refuses** raises real yq's exact
  `yaml: yaml: anchor value must contain valid characters only`. The accepted set is the
  measured one, not YAML 1.2's: printable ASCII except `,`/`[`/`]`/`{`/`}`/`:` and
  whitespace, and nothing non-ASCII. Without the check the emitter wrote `&bad name 1`
  for `"bad name"`, which reads back as a different value.
- **A metadata write under a plain `as $var` binding works** (`.b as $x | .a line_comment =
  $x`, [#2705](https://github.com/rust-works/succinctly/issues/2705)): the write pass
  evaluates the binding's source against the document the stage sees and resolves the body
  once per output with `$x` substituted, exactly as the real evaluation reaches it. Every
  fork's writes are kept in order and applied to every result, which is yq's own behaviour
  (the forks share the node, so `.[] as $x | .a line_comment = $x` on `a: 1` / `b: 2`
  prints `# 2` in both documents).
- **A metadata write anywhere else than a pipe stage** (inside a `reduce`/`foreach` body, the
  *source* of an `as` binding — `(.b as $x | .a line_comment = "x") as $y | ...` — a
  destructuring `as` pattern (behind `--jq-extensions`, #5433), a `def`, ...) raises `metadata assignment (...) is only
  supported as a pipe stage, or under a plain `as $var` binding`. The write pass resolves
  stages against the document each one sees and can't see the input such a nested write
  would run against; real yq handles the source case (it has no `reduce`/`foreach`, a `def`
  or `?//`, and refuses a destructuring `as` itself).

A line comment on an element of a flow sequence or mapping (`.a[-1] line_comment = "y"` on
`a: [1, 2]`) is written the way go-yaml writes it, since
[#2708](https://github.com/rust-works/succinctly/issues/2708): `a: [1, 2, # y` / `]`, a comma
first for a scalar, the comment after the closing bracket for a nested collection, `]` at the
enclosing indent and `}` at column 0, each flow level continuing at go-yaml's
`indent = width * ((indent + width) / width)` (one more step in, rounded to a multiple of the
width, so `- [` under `-I3` continues at 3, not 5). Before it the collection fell back to block
form, and a comment below the container's direct children was dropped without a word. Checked
against yq v4.53.3 on ~1,000 random flow collections (mappings and sequences, three levels deep,
under a root, a mapping, a nested mapping and a sequence item, `-I1`/`3`/`4`/`6`, multi-line
comments). One related gap stays: the cursor-streaming identity route (`yq .` with no write)
still prints `2 # child` / `  ]` for the `[1, 2 # child\n]` source where yq prints
`[1, 2, # child` / `]`; the DOM route (any write, `-P`, `--arg`) matches. And a comment on a flow element does not survive
extracting the container (`.a[0] line_comment = "y" | .a`, `.b = .a`): the reshaped value carries no
flow style or comment tree (#3615), where yq keeps both.

One rendering divergence, readable back and pinned:

- **`style = "double"`/`"single"` on a non-string scalar** rewrites the value into the
  quoted string (`a: 1` becomes `a: "1"`, `a: null` becomes `a: 'null'`), which is what
  real yq's output amounts to — the result reads back as a string in both tools, and a
  later stage in the same pipe still sees the original type in both (`.a style = "double"
  | .a + 1` is `2`). Done to the value rather than in the emitter so an existing
  tagged scalar whose cursor reports a quoted style (`a: !!int "5"`, #747) keeps its
  current (already divergent, pre-#798) rendering.

### Scalar `==`/`!=` compare by text with a wildcard -- resolved ([#2785](https://github.com/rust-works/succinctly/issues/2785)); lost spellings remain

Real yq's `==`/`!=` is not typed equality. `isEquals` (`pkg/yqlib/operator_equals.go`,
v4.53.3) answers `rhs is !!null` for a `!!null` left operand and otherwise, for two scalar
nodes, `matchKey(lhs.Value, rhs.Value)` -- the node *texts*, with the right operand read
as a wildcard pattern (`pkg/yqlib/matchKeyString.go`: `*` is zero or more bytes, `?`
exactly one byte, nothing else is special, and the same matcher serves `.["a*"]`
traversal). succinctly answered jq's typed equality for every one of these before #2785.
Captured live (`-o=json -I0`):

| filter                            | real yq | why                                        |
|-----------------------------------|---------|--------------------------------------------|
| `1 == "1"`, `"1" == 1`            | `true`  | text `1` both sides                        |
| `true == "true"`                  | `true`  | text                                       |
| `!!str 1 == 1`                    | `true`  | the tag does not reach `matchKey`          |
| `1 == 1.0`, `1e3 == 1000`         | `false` | text (`STRICT_NUMERIC_EQUALITY`, #950, agrees) |
| `"abc" == "a*"`, `1 == "*"`       | `true`  | the right operand is a pattern             |
| `"a*" == "abc"`                   | `false` | the left never is                          |
| `null == "null"`, `null == "*"`   | `false` | a `!!null` left operand needs a `!!null` right |
| `"null" == null`                  | `true`  | a `!!null` *right* operand is the text `null` |
| `"~" == null`, `0 == null`        | `false` | that text is its spelling                  |
| `.[] \| select(key == "ab*")`     | matches `abc`, `abd` | the everyday use              |
| `[1] == [1]`, `{} == {}`, `. == .` | `false` | a container pairing is never equal (#2799) |
| `[1] != [1]`                      | `true`  | the same, negated                          |
| `[1] == null`, `[1] == 1`, `[1] == "*"` | `false` | no container is `!!null`, a scalar or a pattern match |

**Containers** ([#2799](https://github.com/rust-works/succinctly/issues/2799)): `isEquals`
answers `false` for any pairing that holds an array or object, so `==` is `false` and `!=` is
`true` there. succinctly answered structural equality before, so `select(. == .)` kept every
node and now keeps only the scalars; any script that compared whole arrays or objects with
`==` now sees yq's answer (comparing `tojson` of both sides gives a structural comparison in both tools,
except that the right-hand text is still a glob: a `*` or `?` in a string value matches there).

Fixed as `eval::yq_scalar_text_eq`, consulted from `eval::apply_compare_op` beside
#2483's `yq_null_ordering_is_false`, so all three evaluators take it; the matcher is
`glob::yq_match_key` (`src/jq/glob.rs`), a byte-for-byte port. String-against-string
stays a plain byte compare unless the pattern holds a wildcard; only a number allocates
its text (`numeric_display_string`, so `==` and `tostring` agree about a computed value).

**Not reproduced**, each its own issue:

- **`contains` and array `-`** keep typed equality where yq compares by text; `min_by`/`max_by` (not yq
  builtins) keep jq's ordering. `min`/`max` of an *array* follow yq's own comparator
  ([#4055](https://github.com/rust-works/succinctly/issues/4055)): a scan from the first element that
  replaces the pick only when the next is strictly smaller (larger), so a tie keeps the first (jq's `max`
  keeps the last); two numbers or two strings (bytewise) compare and nothing else does, so
  `[2, "10", 3] | min` is `!!str not yet supported for comparison`, as is any boolean, with the later
  element's tag; a container is refused (`arrays not yet supported for comparison`, or `!!seq ([0])
  cannot be subtracted from !!int` when the pick is the container); and a `null` element never takes part
  (`[null, 1] | min` is `null`, `[1, null]` is `1`).
- **`min`/`max` of nothing, and `to_entries`/`with_entries`/`split` of `null`, answer nothing in yq
  mode (#4236).** yq has no element to pick, so `[] | min`, `{} | max` and `1 | min` print
  nothing (jq: `null`, and an error for the other two), a mapping is the extremum of its values
  (`{a: 2, b: 1} | min` is `1`), and `null | to_entries`, `null | with_entries(.)` and
  `null | split(",")` print nothing where jq raises. Every other input keeps its answer. Inside a
  `{...}` such an operand is an empty entry (#4193). jq mode is unchanged
  (`EvalSemantics::MIN_MAX_NEED_AN_ELEMENT`, `NULL_ENTRIES_AND_SPLIT_YIELD_NOTHING`), and so are
  `min_by`/`max_by`, which are not yq builtins. A sweep of 100 builtins over ten inputs found no
  other builtin whose emptiness differs. Not reproduced: yq's `min`/`max` also turn a `null`
  node into `[]` when it is *written* through (`[null, {a: 2}] | .[] |= min` is `[[], 2]`, and
  `(.[] | min) = 5` rewrites the `null` too), where this leaves the node as it was; and yq's
  comparator rejects the values of a mapping that are maps, booleans or of mixed types
  (`{a: true, b: false} | min` is `!!bool not yet supported for comparison`, and a map value among
  scalars is `!!seq (a) cannot be subtracted from !!int`) exactly as for an array, where this keeps
  jq's ordering for a mapping (#4055 covers the array).
- **`pick` on an array and `omit` (#4257).** In yq mode `pick` indexes an array by an integer, by a
  string `yq`'s own `parseInt64` accepts (`"1"`, `"0x1"`, `"0o1"`, `"1_0"`), or by an integral float;
  any other key raises, and a negative index matches nothing. `omit` removes only non-negative integer
  indices from an array and leaves a scalar alone. One gap: an integral float key is an index here,
  while yq raises for a YAML-sourced or filter-literal one (`pick([1.0])`; a JSON-sourced `1.0` is an
  index in yq too, so the two cannot be told apart in the owned value). jq mode is unchanged.
- **A string repeated by an integral float count (#4269).** In yq mode `"s" * n` raises for a negative
  or a fractional `n` (as yq does) and repeats for an integer. An integral float (`2.0`) repeats here,
  and in yq when it comes from JSON (`{"n": 2.0}`); a YAML-sourced or filter-literal `2.0` is
  `cannot multiply !!str with !!float` in yq, which cannot be told apart in the owned value.
- **A write keeps the blank lines between a head-comment block and the node below it**
  ([#4093](https://github.com/rust-works/succinctly/issues/4093)). For a node at column 0 the
  blank lines after the block are recorded as trailing empty entries of the head comment and
  printed back; an indented node, and a block that follows a blank line itself, print none, as
  in yq. A head comment with a blank line after it on a *nested* key
  (`m:\n  # a\n\n  k: 1`) is still mangled on a write: the comment moves to the next top-level
  key (pre-existing; yq prints `m:\n  # a\n  k: 1`).
- **A write keeps the first document's `---` marker**
  ([#4086](https://github.com/rust-works/succinctly/issues/4086)). A DOM write (`=`, `|=`,
  `del()`, `-P`, `-i`) prints back the text before the first document's content when it holds a
  `---` marker on a line of its own: the comments and blank lines above it, verbatim, the
  marker (trailing blanks dropped), and the blank lines straight after. It used to be dropped,
  so editing a manifest or playbook that opens with `---` changed its first line. Only the first
  document of a file gets it (later ones keep the ordinary separator), `-N` drops the marker
  lines and keeps the comments, and a root a write replaces keeps it. Not matched: content on
  the marker's own line (`--- 5`, `--- |`; yq prints `---` then the content), a comment on
  the marker's line (`--- # x`, where yq glues the comment onto the first key: an upstream quirk
  not reproduced), and a `%YAML` directive (yq drops it; here the document is rejected). The preamble is
  attached to the first document's root as read, so a result built from it (`to_entries`, a
  reshaped value) carries none where yq still prints it; and a null first document
  (`---\n---\na: 1`) keeps its pre-existing divergence.
- **A write keeps the source spelling of an untouched plain scalar**
  ([#3028](https://github.com/rust-works/succinctly/issues/3028)). On the DOM route (`=`, `|=`,
  `del()`, `-P`, `-i`) a null, bool or number that was not spelled the canonical way is
  printed back as it was written: `push:` (an empty value, the GitHub Actions idiom, which
  used to become `push: null`), `~`, `Null`, `True`, `0x1F`, `017`, `+1`, `.5`, `2.`, `.inf`.
  The spelling rides the node's metadata beside its comment, style, anchor and tag (the
  ADR-0017 side-tree, not `OwnedValue`), is recorded only when it differs from the way the
  value prints, and is printed only while it still resolves to the node's value, so a node a
  write replaces takes the new value's text (`b: 0xff` written `5` is `5`). Checked against
  yq v4.53.3 on ~3,000 random documents plus a sweep of 2,178 real YAML files. It does not
  touch #2802's *read* side (`==`, keys), listed below.
  An integer's `tostring` has kept its spelling since #2802's integer slice.
  A closed literal written
  over a node (`.a = true` over `True`) brings its own text, as in yq; `style = "double"` and
  `"single"` quote the spelling (`"~"`, `"0x1F"`). Not matched:
  - a negative hex or octal arithmetic result (`0x10 - 100` is `0x-54`) is tagged `!!int` wherever it is nested, as in yq, but not at the top of a string encoder: `.a - 100 | to_yaml` is `0x-54` here and `!!int 0x-54` in yq, and `from_yaml` of it reads a string where yq reads the integer;
  - an upper-case prefix (`0X10`) is a string here and an integer in yq, so `.a + 1` concatenates where yq prints `0x11`;
  - the spelling belongs to the *position*, not the node, so equal-valued scalars that a write
    moves (`[~, Null] | reverse`) swap spellings, where yq carries each node's own;
  - a node copied by a plain `=` from a static read of it (`.b = .a`, `.c.d = .a.x`) and a bare
    result (`-P '.a'`, a comment-only or bare `---` document) carry the spelling since #2875
    (`a:` copies as `c:`, not `c: null`; `-P '.a'` is an empty line, not `null`); a copy made by
    anything else does not: a construction wrapped in a `def` or built from several outputs (a
    construction of plain reads keeps it since #865), `map(.)` over a mapping,
    and a source a later stage rewrites (`.c = .a | .a = 5`) all print the plain value where yq
    keeps the text. yq copies the whole node, so quoting, flow style and a trailing comment ride
    along there (`.c = .a` on `a: "x" # n` is `c: "x" # n`); the trailing comment and the flow
    style of a plain `=` from a static read are copied since #5249, the quoting is copied since #5257;
  - an empty value inside a flow collection (`{x: , y: 1}`) prints `null` where yq prints `''`.
- **Spellings `OwnedValue` cannot keep** ([#2802](https://github.com/rust-works/succinctly/issues/2802)):
  `True == "true"` and `!!bool "yes" == true` are `true` here (yq `false`, its text is
  `True`/`yes`), and `"~" == null` is `true` here (yq `false`).
  Every one of these already shows in `tostring`.
  An integer of a *document* written `0x1f`, `0o17`, `01`, `+5` or `007` keeps its spelling since the integer slice of #2802, so `==`, `tostring`, `join`, `sort`, `unique` and `split` by such a separator (#4263) read its text as yq does (`.a == 31` on `a: 0x1f` is `false`).
  The same integer written as a filter literal still compares by value here (`1 == 01` is `true`, `false` in yq).
  An underscored integer (`1_000`) is a string here, the recorded input-spec divergence.
- **`split`'s separator literals** ([#4374](https://github.com/rust-works/succinctly/issues/4374)):
  the `{}` literal splits on the text `{}` as in yq (`"a{}b" | split({})` is `["a","b"]`), recognised
  syntactically, so a form that merely *evaluates* to it (`{} | .`, `{}?`, `first({})`, `{} // 1`)
  is read as a document-kind empty mapping and splits into characters; and `.nan`/`.inf` as
  separator literals are parsed as field accesses (`"a1b" | split(.nan)` prints nothing, yq
  `["a","1","b"]`). A u64-range integer (`18446744073709551615`) is tagged `!!float` by `tag` and in the new
  `cannot split !!float` error (`yaml_type_tag` tags by `i64` fit; yq says `!!int`).

A third entry used to sit here -- the reindex bridge re-spelling a computed float, so that
`[(0.5+0.5)] | .[0] == 1` was `false` (yq `true`) and `[(0.5+0.5)] | .[0] | tostring`
printed `1.0` (yq `1`). Fixed by [#2902](https://github.com/rust-works/succinctly/issues/2902):
the bridge now writes a bare `Float` as a token the reparse hands back as a bare `Float`.

Pinned by the `yq_text_equality_2785` module (`tests/yq_cli_tests.rs`), the
`scalar_text_equality_2785`/`scalar_wildcard_equality_2785`/`container_equality_2799` goldens, and the
`glob` unit tests.

### `unique`/`unique_by`/`group_by` key on text, in first-occurrence order -- resolved ([#2799](https://github.com/rust-works/succinctly/issues/2799)); a container's key encoding remains

Real yq's `unique`, `unique_by` and `group_by` are not jq's sort-and-dedup. They are an ordered map
keyed on a **text** (`getUniqueKeyValue`/`processIntoGroups`, `pkg/yqlib/operator_unique.go` and
`operator_group_by.go`, v4.53.3), so the output is in first-occurrence order and two elements are
one key whenever their texts match, whatever their type. succinctly sorted first and compared
typed values. Captured live (`-o=json -I0`):

| filter                                         | real yq               | why                                           |
|------------------------------------------------|-----------------------|-----------------------------------------------|
| `[3,1,2,1] \| unique`                          | `[3,1,2]`             | first occurrence, not sorted                  |
| `[3,1,2,1] \| group_by(.)`                     | `[[3],[1,1],[2]]`     | groups in order of first occurrence           |
| `[1, "1"] \| unique`                           | `[1]`                 | text `1` both sides                           |
| `[0, false, "0", "false"] \| unique`           | `[0,false]`           | `"0"` is `0`'s text, `"false"` is `false`'s   |
| `[1.5, "1.5", 1.50] \| unique`                 | `[1.5,1.50]`          | `1.50` keeps its spelling, so its own key     |
| `["abc", "a*"] \| unique`                      | `["abc","a*"]`        | no wildcard in a map key (unlike `==`)        |
| `[.nan, .nan] \| unique \| length`             | `1`                   | the same text                                 |
| `[{"a":1,"b":2},{"b":2,"a":1}] \| unique \| length` | `2`             | a container's key is its encoding, order-sensitive |
| `[[1],[2]] \| group_by(.) \| length`           | `1`                   | every container shares `group_by`'s empty key |
| `[[1],[2]] \| unique \| length`                | `2`                   | but `unique` keys on the encoding             |
| `unique_by(.a, .b)` / `group_by(.a, .b)`       | keyed on `.a` only    | yq reads the key filter's first output        |

Fixed as `eval::yq_dedup_key` (the key), `yq_first_of_each_key` and `yq_group_in_order` (the
ordered map), used by `builtin_unique`/`builtin_unique_by`/`builtin_group_by` and the owned
`relocate` path in `eval.rs` and by the cursor route in `eval_generic.rs`. jq mode is
untouched. The key is `tostring`'s text, so `==` and `unique` agree about a number.

**Not reproduced**:

- **A container's `unique` key is compact JSON here, the node's YAML in yq.** The equivalence
  classes agree (equal contents dedup, different ones do not, field order matters) and the bytes
  do not. yq's key is also style-sensitive (a flow `[1]` and a block `- 1` can be two keys), which
  a value-level key cannot see.
- **A `null`'s spelling** ([#6103](https://github.com/rust-works/succinctly/issues/6103)): yq keys a node on its text, so `~`, `null`, `Null` and an empty value are four keys, and `[~]` and `[null]` are two.
  `unique` and `unique_by(.)` over the elements of a document keep them apart, as yq does: `printf -- '- ~\n- null\n- ~\n- 1\n- null\n' | yq unique` is `~`, `null`, `1`, and a container is keyed by the spellings of the nulls in it as well as its encoding.
  A string with the same text is the same key, as in yq: `"~"` and `~` are one element.
  Where the key is not a node of the document, no spelling is left to read, and `~` and `null` are one key:
  a key computed from the element (`unique_by(.a)` on `a: ~` and `a: null` has 1 element here and 2 in yq), `group_by` (`group_by(.) | length` over `~` and `null` is 1 here and 2 in yq), an array the filter builds or copies (`map(.) | unique | length` and `. + [] | unique | length` are 1 here and 2 in yq), and `--eval-all`, whose elements are no longer document nodes.
  An anchored or aliased element has no plain spelling to read either (`- &a ~`, `- *a`, `- null` has 1 element here and 3 in yq).
- **`pivot`'s padding** ([#6102](https://github.com/rust-works/succinctly/issues/6102)): yq pads a row that is too short with an empty node, which prints as nothing (`-` with no value, `key:` bare) and as `null` in JSON.
  `succinctly yq 'pivot'` as the whole filter does the same, and moves every other cell with its comment, quoting, anchor, alias and tag, with the keys of a mapping new strings that stay quoted when they read as another type.
  Where the `pivot` is part of a larger filter, the padding is a `null` that prints as `null`: `[pivot]` and `.x |= pivot` print `null` where yq prints nothing, and `pivot | .[1][1]` prints `null` where yq prints an empty line.
  `.x |= pivot` also keeps the flow style of the rows it replaces, where yq prints block sequences.
- **`pivot` of an alias row, and of a row with a `<<` merge key** ([#6202](https://github.com/rust-works/succinctly/issues/6202)): yq reads the row nodes as they are.
  An alias row (`*name`) has no tag, so it is neither a sequence nor a mapping: `.l | pivot` over `[*row, [c]]` is `sequence contains elements of  and !!seq types`, and over `[*row, *row]` it is `can only pivot elements of !!seq or !!map types, received ` (the tag is empty).
  `succinctly yq` raises the same errors, in yq's words, for an alias row on the document route, and for every other refusal on that route and on the computed one: rows of different tags (`sequence contains elements of !!int and !!float types`), rows that agree and are not collections (`received !!str`, `received !foo`), an empty array (`received `, which yq refuses and `succinctly yq` used to answer `[]`) and a pivot of something that is not an array (`cannot pivot node of type !!int`).
  The rows are compared by tag, so `[[a], !!seq [b]]` pivots and `[!foo [a], [b]]` does not, as in yq.
  An array the filter builds or copies (`[.l[]] | pivot`, `.l | map(.) | pivot`) holds the same nodes in yq, aliases included, where here the copy has resolved them (the owned routes keep no alias mark), so those still pivot the anchor's value instead of raising.
  A mapping row that holds a `<<` merge key is a row with a `<<` key to yq, whose value is the alias, so `{<<: *base, z: 3}` and `{x: 9}` pivot to a `"<<"` column of `[*base, null]`, a `z` column and an `x` column; `succinctly yq` merges the row first and pivots `x`, `y` and `z`.
  Reading the row's own entries is the gap of `to_entries`, `pick` and `..` ([#4954](https://github.com/rust-works/succinctly/issues/4954)), and a JSON encoding of that pivot in yq is itself a merged view (`{"y":2,"z":[3,null],"x":[null,9]}`), so no row of it is reproduced here.
  Pinned by `tests/yq_cli/i6202.rs`.
- **Alias nodes.** yq keys an alias element (`*x`) differently from the anchor's node, since its
  container key is the YAML encoding and an alias encodes as itself: `anch: [&x {p: 1}, *x, {p: 1}]`
  gives `.anch | unique | length` of `3` in yq and `1` here (`group_by(.) | length` is `2` there and
  `1` here). Aliases are resolved before the key is built, so the identity is gone.
- **Spellings `OwnedValue` cannot keep** ([#2802](https://github.com/rust-works/succinctly/issues/2802)):
  the text a `True` keys on is its resolved text here, as is a `01` or `0x1f` written as a filter literal (`[01, 1] | unique` is `[1]` here and `[1,1]` in yq).
  A document `[01, 1, 0x1f, 31, True, true] | unique` has 5 elements here and 6 in yq: its integers key on their text since the integer slice of #2802, the `True` does not.
  The same residual `==` has.
- **Cost.** yq mode builds one `String` key per element and probes a hashed `IndexSet`/`IndexMap`
  where it used to sort values in place. Interleaved A/B against `main`, release builds, minimum of
  9 runs, 1M-element arrays (`yq '.a | unique | length'` and friends on 5-10 MB documents; the
  `sort` row is the unchanged control; outputs identical):

  | filter (1M elements)             | M4 Pro (Mac mini) | Ryzen 9 7950X (Zen 4) |
  |----------------------------------|-------------------|-----------------------|
  | `unique`, ints, 50k distinct     | 0.85x             | 1.11x                 |
  | `unique`, ints, ~1M distinct     | 0.86x             | 1.11x                 |
  | `unique`, strings, 289k distinct | 0.80x             | 0.69x                 |
  | `group_by`, ints                 | 0.70x             | 0.76x                 |
  | `group_by`, strings              | 0.58x             | 0.77x                 |
  | `unique_by(.k)`, 250k records    | 0.90x             | 0.85x                 |
  | `sort` (control)                 | 1.00x             | 1.01x                 |

  Peak memory of `unique` over ~1M distinct integers rises by the key strings, about 90 MB on
  the Mac (133 to 225 MB) and 80 MB on the Ryzen (118 to 196 MB); over strings it rises by
  3-33 MB. A first cut with a `BTreeSet` was +94% on the distinct-integer row, which is why the
  set is hashed, and building every key before probing any (rather than in one loop) is worth
  10% on x86 for the same row. The only slower rows are `unique` over integers on x86 (+11%),
  where the old in-place numeric sort is cheap; yq's own order and keys take precedence
  (ADR-0018).

Pinned by `dedup_builtins_key_on_text_in_first_occurrence_order_2799` (`tests/yq_cli_tests.rs`)
and the `dedup_text_keys_2799`/`dedup_containers_2799` goldens.

### `sort`/`sort_by` follow yq's comparator and Go's stable sort -- resolved ([#2799](https://github.com/rust-works/succinctly/issues/2799)); a literal `{}` and NaN remain

Real yq's `sort` and `sort_by` are not jq's total order. `sortableNodeArray.compare`
(`pkg/yqlib/operator_sort.go`, v4.53.3) decides in this order, and the first rule that applies
wins: `null` sorts before everything; a bool before every non-bool (`false` before `true`); two
integers numerically; two numbers (an integer and a float, or two floats) as `f64`; anything else
by the node **texts**, byte-wise (`strings.Compare(lhs.Value, rhs.Value)`). A container's text is
empty, so containers tie with each other and sort after the bools and before every non-empty text,
whatever they hold. Captured live (`-o=json -I0`):

| filter                                        | real yq                          | why                                       |
|-----------------------------------------------|----------------------------------|-------------------------------------------|
| `[2, "10", 3] \| sort`                        | `["10",2,3]`                     | an integer against a string compares by text |
| `[.b, .a] \| sort` (`"1"`, `1`)               | `["1",1]`                        | equal texts tie, in input order           |
| `[[2],[1]] \| sort`, `[{"a":2},{"a":1}] \| sort` | unchanged                    | containers tie                            |
| `[1,[1],"1",{"a":1},null,true] \| sort`       | `[null,true,[1],{"a":1},1,"1"]`  | null, bool, containers, then texts        |
| `[10,9,100,1] \| sort` / the strings          | numeric / `["1","10","100","9"]` | by kind                                   |

**The comparator is not a strict weak order** (two integers compare numerically, an integer
against a string by text, so `2 < 3`, `"10" < 2` and `3 < "10"` can all hold). The answer for a
mixed-type array therefore depends on the sort algorithm, and yq runs Go's `sort.Stable`
(insertion sort on blocks of 20, then `symMerge`). succinctly ports exactly that algorithm
(`src/jq/go_sort.rs`) and uses it whenever the array mixes kinds, so a mixed array comes out in
the order yq gives. When every element (every key position, for `sort_by`) is of one kind, the
comparator is a total preorder and any stable sort returns the same answer, so Rust's faster
`slice::sort_by` is used. Rust's sort is not a substitute on a mixed array: it orders it
differently and, since Rust 1.81, may panic on the inconsistent comparison.

Fixed as `eval::yq_compare_values` and `eval::yq_sort_by`, used by `builtin_sort`,
`builtin_sort_by` and the owned `relocate` path in `eval.rs` and by `sort_keyed_elements` in
`eval_generic.rs`. A `sort_by` key is the filter's full output list, compared position by
position and then by length, as `sort_by(.a, .b)` is in yq. jq mode is untouched.

**Not reproduced**:

- **NaN and the infinities.** yq **panics** (`strconv.ParseFloat: parsing ".nan": invalid
  syntax`, a Go runtime panic) when a sort reaches a document `.nan` or `.inf`. This build
  orders NaN before every other number and the infinities by value, rather than crash (the
  ADR-0018 exception for output that would take the process down). The ordering operators and
  `min`/`max` raise yq's own error for such a scalar instead of ordering it
  ([#6887](https://github.com/rust-works/succinctly/issues/6887), below).
- **A literal `{}` in the filter** sorts as the text `{}` in yq (its literal node carries that
  text) and as the empty text of any other mapping here: `[.z, {}, .b] | sort` puts the literal
  last in yq. An empty mapping read from a document sorts as empty text in both.
- **Timestamps are read from the text, not from a tag** ([#7058](https://github.com/rust-works/succinctly/issues/7058)).
  yq compares two `!!timestamp` nodes as times, in `sort`, `sort_by`, `min` and `max`; `OwnedValue` has no timestamp type, so two strings that both read as one (`2006-01-02` or RFC 3339, or the layout `with_dtf` set) compare as instants, as the ordering operators already did ([#5651](https://github.com/rust-works/succinctly/issues/5651)).
  A date that was quoted in the document is `!!str` to yq and compares as text there, where it is read as a time here ([#7057](https://github.com/rust-works/succinctly/issues/7057)); the two agree unless the layouts or offsets differ.
  A sort that mixes timestamps with other strings or numbers is not a total preorder, so it goes through the Go-ported stable sort, as the other mixed arrays do.
  In `min` and `max` a timestamp element reads the running pick as a time too: a pick that is no timestamp raises the error reading it as one gives (`[plain, 2001-12-14] | min`), and a timestamp pick against a string that is none is refused with `!!str not yet supported for comparison`.
- **An absent key in a `sort_by` key filter** is no output, as in yq ([#4054](https://github.com/rust-works/succinctly/issues/4054),
  [#4387](https://github.com/rust-works/succinctly/issues/4387)). yq evaluates the key through `ReadOnlyClone`,
  so an element lacking the key has a shorter key list (`[]` for `.k`, `[9]` for `.x, .y`) rather than one
  holding a `null`: `[{k: 3}, {k: null}, {n: 1}] | sort_by(.k)` puts the absent one first, and
  `sort_by(.x, .y)` over `[{x: 2}, {x: 1}, {y: 9}]` puts `{y: 9}` last. `unique_by`, `group_by` and a
  mapping's `sort_by` read their keys the same way (`by_key_scope`; the cursor route enters it around
  `sort_family_array_generic`). The gated `min_by`/`max_by` extensions follow jq and read the absent key as `null`.
- **Spellings `OwnedValue` cannot keep** ([#2802](https://github.com/rust-works/succinctly/issues/2802)):
  an underscored integer (`1_000`) is a string here and sorts after the numbers, where yq reads it as `1000` (the recorded input-spec divergence).
  A leading-zero, hex or octal integer of a document sorts by its value as in yq, and the node it returns keeps its spelling.
- **A mapping** (#2064, #4386). yq's `sort` and `sort_by(f)` reorder a mapping's entries by their
  values and answer a mapping, as `sort_by` does an array; so does this build, with the same
  comparator and the same stable sort. The result is an owned mapping, which carries no
  comments, style, anchors or tags of its own, so a YAML output of a *mapping* sort takes the DOM
  route (the per-file fallback `--sort-keys` already uses over aliases) instead of the streamers,
  and `reshape_presentation` carries each entry's comments, flow style, anchor, alias, tag and
  key style to its new place: `# top`, `a: 1 # one`, `k: {z: 1}`, `b: &x 2` / `a: *x`,
  `b: !!str 2` and a numeric key all print as yq prints them
  ([#4386](https://github.com/rust-works/succinctly/issues/4386)). The route is chosen per file, by
  evaluating the path in front of the `sort` on each document and looking for a mapping, so an
  array sort keeps streaming its elements as cursors (an array's per-item comments survive).
  Only a query that *ends* in the sort is recognized (`sort`, `sort_by(f)`, `.a.b | sort`):
  a mapping sort that another stage reads (`sort | .a`, `.a | sort | keys`), one behind an
  iteration (`.[] | sort`) or inside `map`/`with_entries`/`if` keeps the owned route's loss.
  An owned map holds a key once, where yq lists every occurrence, so **a mapping with a repeated
  key is refused** (`cannot sort a mapping with the repeated key "a"`) instead of answering
  with an entry gone, which a sort written back with `-i` would make permanent (the ADR-0018
  data-loss exception).
  `sort_by(key)` and `sort_by(key | f)` sort by the entry's key ([#6099](https://github.com/rust-works/succinctly/issues/6099)).
  The key is the scalar its text resolves to, as a plain key is in yq (`10` is an integer and follows `9`, `null` and `true` sort by their kind).
  The mapping reaches the sort as JSON text, which no longer says whether the key was quoted, so a quoted `"10"` sorts as the integer here and as a string in yq.
  A `key` below another step (`sort_by(.a | key)`) is that step's and is left as it was.
  Comments, anchors and styles follow their entries for `key`, `key | length`, `key | tag` and `key | type`; another `f` after the `key` (`key | downcase`) sorts correctly but takes the route that carries none of them.
- **An array's head comment** ([#4926](https://github.com/rust-works/succinctly/issues/4926)).
  yq's `sort`, `sort_by`, `reverse`, `unique` and `unique_by` reorder the sequence node in
  place, so the comment block above a document's sequence (a config file's banner) stays on
  top. The items stream from their own cursors and the result has no node to read that block
  from, so the runner prints the document's head (the verbatim header of the first document and
  the comments above the sequence) before the items when the query *is* one of these builtins
  over a document whose root is a sequence.
  A flow-style sequence, at the root or behind a static path (`.a | sort`), is answered through the write form `(.a |= sort) | .a` instead.
  It keeps its flow style, line comment, quoting and anchor, as yq does (#6877).
  A document of the input holding such a sequence sends the whole input to the DOM route, as a sort of a mapping does (#4386).
  `-i` and a prefix that is not a static path (`select(...) | .l | sort`) keep the streamers' block style.
  Not covered: a block-style sequence reached through a path (its items stream without the node's own comments), a reorder that is only part of the read (`[.a | sort]`, `.a | sort, .b | sort`), the comment block above a later document,
  which yq attaches to the item that ends up first, the foot comment below the sequence,
  which yq moves with its last item, and a sequence whose items include an alias (reordered as
  owned values, so it keeps neither the head nor the `&anchor`/`*alias` marks).

Cost. Interleaved A/B against `main`, release builds, minimum of 9 runs, 1M-element arrays on
5-10 MB documents (outputs identical; `sort_by` on 250k records):

| filter                                | M4 Pro (Mac mini) | Ryzen 9 7950X (Zen 4) |
|---------------------------------------|-------------------|-----------------------|
| `sort`, ints, 50k distinct            | 1.04x             | 1.01x                 |
| `sort`, ints, ~1M distinct            | 1.03x             | 1.01x                 |
| `sort`, strings                       | 1.08x             | 0.98x                 |
| `sort`, ints and floats               | 1.05x             | 1.00x                 |
| `sort`, ints and strings (mixed)      | 2.19x             | 1.89x                 |
| `sort_by(.k)`                         | 1.02x             | 0.99x                 |

A single-kind array (and integers with floats, while every integer is exactly an `f64`) is a
total preorder, so it keeps `slice::sort_by`. Only an array that mixes kinds in the
non-transitive way (integers or floats with strings, bools or containers) takes the port of Go's
stable sort, which is the price of yq's order: `symMerge` does O(n log^2 n) swaps where Rust's
merge sort does O(n log n).

Pinned by `sort_follows_yqs_comparator_and_go_stable_sort_2799` (`tests/yq_cli_tests.rs`), the
`sort_mixed_2799`/`sort_by_mixed_2799` goldens and the `go_sort` unit tests.

### A typed mapping key is a node -- resolved ([#2785](https://github.com/rust-works/succinctly/issues/2785)); its spelling, the path register and an owned rebuild remain

`key` emits the key **node** of a string-keyed member since #2763, so `line_comment`,
`head_comment`, `foot_comment`, `line`, `column`, `style` and `anchor` after it read the
key's own metadata, matching yq v4.53.3. #2785 extends that to a *typed* key: `1: x`,
`true: y`, `null: z` and `1.5: w` are `!!int`/`!!bool`/`!!null`/`!!float` nodes through
`key`, `keys`, `to_entries` and `with_entries`, as in yq -- `[.[] | key]` prints
`[1, true, null, 1.5]`, `key | tag` answers `!!int`, `.["1"] | key | line` answers `1`,
`to_entries | .[0].key + 1` is `2`, and `with_entries(.key |= . + 1)` on `1: x` is `2: x`.
`select(key == 1)` and `select(key == "1")` both match it, because yq-mode `==` compares
scalars by text (the entry above) -- the divergence #2763 had stopped at.

The type is decided where a key is materialized (`key_owned_value` and the walk's
`path_context_item_to_owned`, `src/jq/eval_generic.rs`), behind #2763's
`key_spelling_may_retype` prefilter, so the `as_i64`/`as_f64` parse attempts are still
paid only by a key spelled like a number, bool or null. A complex `? [a, b]` key, an
explicitly tagged key and a key whose bytes do not decode keep the display-string route,
unchanged from #2763.

**The canonical-spelling gate.** `OwnedValue` keeps no text of its own for a null, a
bool, or a non-canonical integer -- `~`, `True` and `01` all resolve to the same
`Null`/`Bool(true)`/`Int(1)` their canonical spellings `null`/`true`/`1` do. Retyping
every retypable spelling regardless would therefore materialize the *same* key for two
document members that are distinct in yq (`null: a` and `~: b`), and reassembling them
(`from_entries`, `with_entries`) would silently collapse one member into the other -- a
real regression an earlier draft of this fix shipped and #2804's review caught live
against the oracle. So a key only actually retypes when [`yq_scalar_text`] of its
resolved value reproduces the raw spelling byte-for-byte (`key_owned_value_spells_
canonically`); a non-canonical spelling falls back to the display string instead, the
same route an explicitly tagged or complex key already takes. Confirmed live against
v4.53.3 on `null: a\n~: b\n` and `1: a\n01: b\n`: `with_entries(.)` and
`to_entries | from_entries` keep both members on both tools, and
`[.[] | select(key == 1)]` matches only the `1:` member on both, `01:` included.

Three residuals, each captured live from v4.53.3:

- **The spelling is gone, and readers now disagree about the type too**
  ([#2802](https://github.com/rust-works/succinctly/issues/2802)). A non-canonically
  spelled key (`01`, `~`, `True`, ...) stays on the display-string route once
  *eagerly materialized* into an `OwnedValue` (`key_owned_value`: `keys`, `to_entries`,
  `with_entries`, a `select(key == ...)`/comparison read via a live cursor), so it never
  loses another member on `from_entries`/`with_entries` -- but it also never gains the
  type there: `keys`/`to_entries | map(.key)` print `"01"`/`"~"`/`"True"` rather than
  `1`/`null`/`true`. **The gate only reaches eager materialization, not every cursor
  read**, so two different readers of the identical key now answer differently:
  `keys | map(tag)`/`[keys[] | tag]`/`[.[] | key | tag]` (a `tag` read straight off a
  still-live *lazy* key cursor, not an already-materialized `OwnedValue`) answers the
  fully-resolved `!!null`/`!!int`/`!!bool` for a `01`/`~`/`True` key, matching yq -- while
  `to_entries | map(.key | tag)` (the key is already a plain object field by that point,
  no cursor left) answers the gated `!!str`. Likewise `[.[] | key]` (array construction
  directly over `.[] | key`, nothing else reading `key` in between) still prints the
  fully-resolved `null`/`1`/`true`, matching yq, where `keys` on the identical document
  prints the gated string. `select(key == null)` inherits the gated side's answer: real
  yq's `!!null`-vs-`!!null` rule is type-based (matches `~:` regardless of spelling), but
  a gated `~:` materializes as `OwnedValue::String("~")`, which fails that check -- so
  `select(key == null)` on a document holding both `null:` and `~:` answers only the
  `null:` member's value here, where yq answers both. Fully resolving either gap needs
  `OwnedValue` to carry the source spelling the way `NumberLiteral` already does for a
  float, which is #2802's own scope.
- **`path` types an integer-spelled key as an integer** ([#2801](https://github.com/rust-works/succinctly/issues/2801)).
  yq's `GetPath` renders a key through `getParsedKey`: a `!!str` key is a string, and any other
  key is `parseInt64(text)` when that parses and the raw text when it does not. So `1:`, `01:`,
  `0x1f:`, `0o17:`, `+4:`, `-3:`, `1_0:` and `!!float 9:` are the components `1`, `1`, `31`, `15`,
  `4`, `-3`, `10` and `9`; `true:`, `~:`, `1.5:`, `1.0:`, `0b11:` and an `i64`-overflowing digit
  string stay strings, as does any quoted or `!!str`-tagged key and every key of JSON input. The
  component is the parsed value, never the spelling, and `succinctly yq` now reports it for `path`,
  `..`/`.[]` walks, a literal lookup (`.["1"] | path` and `.b.2 | path` give `[1]` and `["b",2]`,
  the matched key's parsed value rather than the name the filter spelled) and, under
  `--jq-extensions`, `paths`/`leaf_paths`. `setpath`, and `getpath` under the flag, read an integer
  over a mapping by its decimal text (`setpath([1]; "Q")` updates the `1:` member, as in yq), so a
  `path` result round-trips for a key spelled in plain decimal. A key spelled `0x1f`, `01`, `+4` or
  `0o17` reports `31`, `1`, `4` or `15`, and yq's own `.[31]` does not find it either (it compares
  text), so `setpath([31]; ...)` adds a `31` member instead. What remains, each still a string
  component: a value that left the document (a mapping after a DOM write or `with_entries`, or
  `.[] | tostring | path`, `[.] | .[0] | path`), because an `IndexMap<String, _>` keeps no key type;
  the `path(f)` extension routes that evaluate `f` on an owned copy (`path(.. | select(...))`,
  `paths(f)`); and a quoted key carrying an explicit `!!int` tag (`!!int "5": x`). Also unchanged:
  `1_0:` is `!!str` to succinctly's key *node* (`key | tag`) where yq says `!!int`, a resolver gap,
  and `pick(.[1])` is `{}` in yq but refused here.
- **An owned object has string keys again.** After a DOM write (`.["1"] = "X" | .[] | key
  | tag` is `!!int` in yq, `!!str` here) or through `with_entries`'s own result
  (`with_entries(.key |= . + 1) | keys | .[0] | tag`), the keys live in an
  `IndexMap<String, _>` and their type is not recoverable. `entries_to_object`
  stringifies a typed key on reassembly in both tools (#2521), so `with_entries(.)` itself
  agrees.
- **Traversal matches a key by text** ([#2800](https://github.com/rust-works/succinctly/issues/2800),
  [#4079](https://github.com/rust-works/succinctly/issues/4079)). yq's `matchKey` compares texts, so
  `.ab*`, `.*`, `.1`, `has(1)`, `.[1]`, `.[true]` and `.[null]` find the `ab*`-matching members and
  the `1:`, `true:` and `null:` members, and so do `.["ab*"]` and `.["1"]`; a pattern also finds a
  typed key (`*` matches `1: y`, `true: t`, `1.5: w`), and `.[1] = "z"` updates or creates the member.
  What remains: a mapping key whose spelling the owned value has lost (`~` and `null:` are `null`,
  `0x10` is the number 16, so a pattern cannot rebuild the text yq matches; such a key is left out of
  a `.*` rather than answered with a `null`), a literal index's own spelling (`.[01]`, above),
  and yq's own quirks, recorded rather than copied: `del(.["1"])` and `del(.[1])` on an int-spelled
  key are no-ops in yq (succinctly deletes), and `pick(.["ab*"])` is `{}` there. Not a key-node
  question, but the same `matchKey`.

Pinned by `key_node_metadata_2763::a_typed_key_is_a_node_too_2785` and the
`typed_key_node_2785` module (`tests/yq_cli_tests.rs`), plus the `typed_key_*_2785`
goldens.

Three narrower residuals of the same fix, each captured live from v4.53.3:

- A `key | <metadata>` nested inside a bounded consumer (`first`/`limit`/`last`) or an `as`
  *source* still answers the default when a later stage also reads path context
  (`.a.b | first(key | line) | path`). The bounded consumers are not real yq builtins
  anyway — its lexer rejects `limit`, `last` and `try`, and its own `first(f)` is the first
  child for which `f` is truthy, which `succinctly yq` implements as `first(.[] | select(f))`
  (#2377) — so these shapes (spelled `limit(1; …)`) are succinctly extensions.
  Two residuals of #2377: yq vivifies an absent key read inside `f` into the child it
  returns (`[{"a":0}] | first(.a, .b)` is `{"a":0,"b":null}`; the #2435 family), and
  `first(f).a` (a postfix after the call) is a parse error here. yq also raises its own comparison
  errors for a child `f` cannot compare (`[[],[1]] | first(. > 1)` is `arrays not yet supported
  for comparison`; here `[]`), and a write through `first(f)` on a `null` context answers `[]`
  in yq and `null` here. The change applies under `--jq-extensions` too: `first(f)` is a real yq
  builtin, so the flag does not restore jq's meaning (spell it `limit(1; f)`).
- Metadata through a *constructed* container is the container's, not the key's:
  `map_values(key | line)` is `a: 1` in yq and `a: 0` here, as are
  `to_entries[0].key | line` and `[.a | key] | .[0] | line`.
- Whether an ordinary owned value that *kept* its position answers metadata from it. #2763
  answered only the key-node case, because only it is what `key` emits. On `a:\n  b: 1\n`,
  `.a | del(.b) | line`, `.a | .b = 3 | line` and `.a | key as $k | . | line` are all `2` in yq
  (the line `.a` itself stands on) and were `0` here.
  [#4696](https://github.com/rust-works/succinctly/issues/4696) settled it for `line` and
  `column` in `succinctly yq` and [#4711](https://github.com/rust-works/succinctly/issues/4711)
  for `style`, `anchor` and the comments, for the stages that hand yq's own node on: a write
  (`=`, `|=`, the compound forms), `del`, `sort`, `sort_by`, `flatten`, `flatten(n)`, `shuffle`,
  `to_number`, `sort_keys`, `map_values`, `pick`, `omit`, arithmetic on the node (`. + {}`,
  `. * {}`) and `map` where it keeps the node (an empty container, a scalar), plus the stages
  that pass their input through (`select`, ...). The list is an allow-list
  (`stage_keeps_node_metadata`), one bit per getter: a stage that builds a fresh node
  (`tostring`, `length`, `[.]`, `type`, `tag`, `not`, `any`/`all`, `group_by`) keeps the position
  `path`/`key`/`parent` read and reports `0`/`""`, in yq and here; `unique`, `unique_by` and
  `reverse` report line `0` but keep the style and the line comment, and `to_entries` the line
  comment; and a stage nobody has captured falls back to what an owned value always answered.
  yq also resets two things on the way, which are modelled: a write that fills an *empty* flow
  container makes it block (`.b = 3` on `{}`, `. + []` and `del(.[0])` on `[]`; `del(.b)` on
  `{}`, `. * {}` and `-` keep it), and the merge `*`/`*=` drops the anchor. `succinctly jq` is
  unchanged (`line` and the rest are not jq's). What is still `0`/`""` here and not in yq: the
  style of `tostring` (`double`) and of `map` over a non-empty flow container (yq keeps it), a
  getter nested in a comma, an operand or a body (`(line, column)`), and the child of a written
  value (`.a | (.b = 3 | .b) | line` is `4` in yq; tracked in
  [#4712](https://github.com/rust-works/succinctly/issues/4712)).

### Standalone comments print on both routes now (#2795, #798)

The parser captures head/foot comments and the getters read them (#2758); the *streaming*
emitter (`stream_yaml_value_at`/`stream_yaml_as_document`, `src/yaml/light.rs`, PR A) and
the **DOM** emitter (`emit_yaml_value_at_depth`, `yq_runner.rs`, PR B) both print them now.
On `# lead\na: 1\n# mid\nb: 2\n\n# trail\n`, real yq and succinctly agree byte-for-byte on
`.`, `select(true)`, `-P '.'`, `.a = 5` and `del(.b)` (which keeps `# lead`/`# trail` but
drops `# mid`, owned by the deleted key); `-o=json` drops them on both routes, correctly.

`NodeMeta.head_foot_comment` (`src/jq/eval_generic.rs`) is populated by
`to_owned_with_comments_at_depth`, reading `DocumentCursor::head_comment_raw`/
`foot_comment_raw` (new trait methods, empty-by-default so JSON pays nothing) off the
node the parser actually keys head/foot to — the *key* cursor for a mapping entry, not the
value cursor `to_owned_with_comments_at_depth` otherwise recurses with (`.b | key |
head_comment` is non-empty, `.b | head_comment` is `""`) — and both
`reconcile_presentation_at_depth` and `strip_presentation_style_at_depth` already carried
it through a write for free, via their existing whole-`NodeMeta` clones.

A comment under `key:` followed by a blank line and then the key's first child
([#4099](https://github.com/rust-works/succinctly/issues/4099)) is that child's head, as in yq (`m:` / `  # a` /
blank / `  k: 1` keeps `# a` above `k`, and drops the blank line): the node opening after the detached comment
is deeper than the block the key is in, so it is the key's first child. Not matched: an *indentless* sequence child
(`m:` / `  # c` / blank / `- 1`, the `-` at the key's column) is not deeper, so its comment stays `m:`'s foot where
yq makes it the item's head.

One DOM-route-only residual, not shared with the streaming route: real yq's own root-vs-
navigated distinction for a bare *scalar* result (a container keeps its head/foot
regardless of navigation, matching the streaming route's rule below, but a navigated-away
scalar drops both even though its own cursor may still carry them structurally — e.g. a
sequence item's scalar value keys head/foot off its own position the same way whether or
not it's ever reached by navigation). The document-root/first-document/scalar-vs-collection
gating for a bare result's standalone head/foot lives in `to_owned_with_comments` itself
(`src/jq/eval_generic.rs`, #2795 PR B review) — not in `evaluate_yaml_cursor`'s
`owned_with_comments` closure (`yq_runner.rs`), which only consumes the already-gated
result — clearing them immediately for a non-root or non-first-document scalar cursor (new
`DocumentCursor::is_document_content`, mirroring `light.rs`'s private method of the same
name), and the pre-existing #852 style/anchor-clearing pass for a bare scalar result now
preserves whatever head survives that gate while always dropping foot (matching the
streaming route's own root-only asymmetry below). Pinned by `dom_standalone_comments_2795`
(`tests/yq_cli_tests.rs`).

What the streaming route reproduces, all measured against pinned v4.53.3:

- a head prints above its node at the node's own indent, a foot below it; a foot followed
  by a sibling in the same collection is followed by one blank line (`a: 1\n# mid\n\nb: 2`
  round-trips as-is), a foot that ends its collection by none (`b: 2\n\n# trail` loses
  its blank); a blank line *inside* a head block is reproduced, a blank *after* one is not
  (`a: 1\n# f\n\n# g\n\nb: 2` prints `# f`, blank, `# g`, `b: 2`). The DOM route reproduces
  the interior-blank-line half of this too (#2795 review): `head_comment_raw`'s `YamlCursor`
  impl derives it from the raw text the same way `write_head_comment_lines` does
  (`head_comment_lines_with_blanks`, `src/yaml/light.rs`), rather than the plain decoded-line
  iterator it started with, which had no way to represent a blank at all
  (`a:\n  # h1\n\n  # h2\n  b: 1` lost the blank under `-P '.'` until this fix, live-verified
  against pinned v4.53.3). The **header**'s own blank-line handling remains streaming-only —
  see the next bullet;
- the leading **header** real yq's default `--header-preprocess` slurps — every line before
  the first content line that is blank, a `#` comment at any indent, a `%YA..` directive or
  a `---` marker — is re-emitted verbatim in front of the first document whenever the
  result is that document itself (`# lead\n\na: 1` keeps its blank; `---\na: 1` and
  `%YAML 1.2\n---\na: 1` keep their markers; `-N` drops the markers, as in yq; `.a` prints
  no header). `--header-preprocess=false` reads the input with no header at all ([#7025](https://github.com/rust-works/succinctly/issues/7025), below).
  The DOM route (`-P`, a write, `-i`) reproduces the header too (#2795 PR B, #4086, #5008), but through the first document's head comment lines and its preamble, not the raw byte range, so it differs in two ways.
  A `%YAML` line of the header is not printed back there (see #6408 above).
  And a result that is a new value built from the root (`map(. + 1)` over a sequence, `-P map(.)`) carries the header here and not in yq, as a header of comments does (the derived-from-root gap); `to_entries` drops it where yq keeps it.
  The blank lines a header without a marker starts with, and a run of two or more between its comments, are kept since #5008 (`\n\na: 1` under `-P` or a write keeps both, and so does `# h` / two blanks / `# g`), built from the header text: a result that is a part of the root (`.a`, `.[0]`) carries none, as in yq.
  The empty document made up for an input that holds none has no blank line to keep, while a file of one line break does.
  Pinned by `header_verbatim_reproduction_is_streaming_only_2795` (`tests/yq_cli_tests.rs`, now a test that the DOM route keeps the blank and the marker) and `tests/yq_cli/i5008.rs`;
- the header slurp also **loses a line break**, and `succinctly yq` reproduces the loss (#6276).
  yq pushes the text after a leading `--- ` back onto its reader without the marker line's own break,
  so the next line continues it: `--- x` / `y` is `"xy"`, `--- x` / `y: 1` is `{"xy": 1}`, and `--- # c` / `x` / `y` is the comment `# cx` and the document `"y"`.
  The loop runs over the head of the input only, so a marker after the first content line, a bare `---` line and a lone CR break are unaffected.
  Each file of a run is a stream of its own; under `--eval-all` only the first file's head is slurped.
  The input is indexed with that one break cut out and the marker kept (`yaml_runner/leading_marker.rs`), so the line numbers of the nodes after it are one lower, as in yq.
  `--header-preprocess=false` reads `--- x` / `y` as `"x y"` (#7025, below), and so does a byte-order mark in front of it.
  `load()` reads its file without the slurp, in yq and here.
  `--front-matter` input is not glued (its own extraction runs first and was not captured against yq).
  Blanks in front of the first marker and a second marker the pushed-back text starts with (`--- --- x`) are stripped as in yq.
  A run of leading bare `---` lines is one header and one document in yq, and since #6349 here too (`---` / `---` / `x` is `"x"`, with all three lines in the printed header): `succinctly yq` indexes its input with `YamlIndex::build_yq`, and the library's `build` keeps the YAML spec's reading (a null document, then `x`).
  A marker after the first content line is a marker in both, and so is every later empty document (`a: 1` / `---` / `---` / `b: 2` is three documents).
  Not reproduced: `---` / `---` / `...` and `---` / `...` / `---` / `x` are errors in yq and read as null documents here, and the `line` of an empty document is `0` in yq and a line number here (`---` alone is `1`, as before);
- a scalar root prints its head and *drops* its foot, a flow or block collection root
  prints both (`# lead\n42\n# foot` is `# lead\n42` in yq; the getter still answers
  `foot`). The DOM route reproduces this asymmetry too (#2795 PR B).

Divergences, each classified by `scripts/yq-comment-oracle-fuzz.py`:

- **Comments real yq loses are kept** (`yq-drops-we-keep`): a blank-detached block at the
  very end of a document after nested content (`- c: x\n  f g: 2\n#\n\n# c5`), and a block
  right after `---` that a run of blank lines separates from the next block, are dropped by
  yq's own printer and printed by succinctly, where they attach as the document's foot or
  head. Refusing to reproduce that loss is ADR-0018 rule 4 (matching would discard data).
- **A `--- # c` header line** is glued by yq onto the following line (`---\n# ca: 1`,
  which yq cannot read back); succinctly prints `---` and drops the marker's comment.
  A whitespace-only line in the head of the first document is a comment to go-yaml, and prints as `# ` plus the line,
  where it sits between comments, a `---` or a `%YAML` line (#6401); the header covers it, so it prints the same.
  The DOM route (`-P`, a write) prints it too, but not the `---` and `%YAML` lines of the header.
  A whitespace-only line ahead of a leading `---` is printed as it is there, without the `# `.
- **A block between a bare `-` and its value, followed by a blank line, is the foot of the value's first key or scalar** in yq once a node has opened earlier in the stream and the dash line holds no comment (#7044), and yq prints it below that node.
  `- 1` / `-` / `  # c` / blank / `  n1: 1` is `.[1].n1 | key | foot_comment`, printed `- n1: 1` / `  # c`; a block after the blank line is the key's head, and a blank line above the block, a comment on the dash line, or nothing before the item keeps the whole run the head.
  succinctly reproduces this, on every route (`Parser::defer_first_block_below_bare_dash`).
  Not reproduced: a flow collection value, a sequence whose first item is one (`- [1]`) or a key written as one (`[1]: 2`), with or without an anchor or tag before it (`&x [1]`, `!!map {a: 1}`), before each of which yq drops the block (it stays the first node's head here, as for the run `defers_foot_to_next_item` places), and a dash line that holds the value's properties (`- !t` / `  # c` / blank / `  n1: 1`), where the block stays `n1`'s head.
  A later block that yq makes the same key's foot goes to the key's value in yq once the key holds this one (`n1: 1` / `  # f` / blank / `- 2` after it: `1`'s foot `f`, printed above `# c` with a blank line between), and is appended to the key's foot here.
  A dedented block deferred to the value's first scalar from the item before (`- a:` / `    - 1` / ` # x` / blank) joins this one there in yq (`x` and `c`), and stays on the earlier item here, as before #7044.
  After a flow item's own foot and a blank line (`- [1]` / `# e9` / blank / `-` / `  # d1` / blank / `  k0: 1`), yq prints no blank line before the next item and succinctly prints one; it did before #7044 as well.
- **A CRLF after a standalone comment line is a blank line to yq's scanner** (go-yaml reads the `\r` as one break and the `\n` as a blank line of its own), so yq prints a blank line after every standalone comment of a CRLF file that the source never had (#5053).
  `a: 1\r\n# mid\r\nb: 2\r\n` prints `a: 1\n# mid\n\nb: 2\n`, and a block of comments gets a blank line between its lines (`# c1\n\n# c2`).
  succinctly reproduces this, the same placement included: the lone `# mid` is the foot of `a`, not the head of `b`, as it is in yq (`.a | key | foot_comment`).
  The rule is the one shared predicate `blank_line_between` (`src/text/line_break.rs`, `crlf_comment_blank_between`), applied where the parser decides a comment's placement.
  Measured on 196 documents over seven shapes (a sibling, a nested key, a dedent, a sequence, a first child, the end of the document, a sequence under a key), one to three comments and every blank-line gap: yq on the CRLF text equals yq on the LF text with one blank line added after each standalone comment line, and so does succinctly.
  A comment after a node (`a: 1 # t`), a `#` line inside a block scalar and a lone `\r` break are not affected, as in yq.
  Two older gaps that the extra blank line made reachable are fixed with it, in LF as well.
  A comment below a key that has no value yet (`b:` / `# c` / blank / `h: 1`) is the next sibling's head in yq and no blank line follows it, where succinctly made it `b`'s foot with a blank line; the `-P` route still prints that blank line after an LF `b:` / `# c1` / blank / `h: 1`.
  A write that changes a nested node's kind (`.a = 1` over a mapping, `.a = {}` over a scalar) dropped the standalone comments around the key, where yq keeps them; they are kept now.
  Not reproduced: two or more standalone comment lines at differing columns at the end of a document (`# c28\r\n  # c1\r\n` after a nested sequence), a CRLF comment block after a sequence item that holds a block scalar (`- run: |` with the block's `#` lines, then `# tail` and `# of the step`), a navigated sequence item's comment lines after a write (`.z = 2 | .items[1]`), and a blank line below a block of comments on `-P` and writes (`# c1\r\n# c2\r\n\r\nb: 2`, where succinctly adds a blank line after `# c2`).
  The first of these is the LF gap that a blank line between comments at differing columns already has (`k:\n  - v\n# c28\n\n  # c1\n` puts `# c28` at column 2, yq at column 0), which the extra blank line now reaches in a CRLF file.
  The first document's *header* is reproduced as
  yq prints it ([#5053](https://github.com/rust-works/succinctly/issues/5053), item 3): its
  line breaks verbatim (not normalized to `\n` like the rest of a document's content), every
  comment line keeping its `\r`, and a blank CRLF line in it a literal `# \r` comment line
  (`# lead\r\n\r\na: 1\r\n` prints `# lead\r\n# \r\na: 1\n`), on the streaming route
  (`write_yq_header`), the DOM route (`crlf_header_head_lines`, the verbatim preamble) and a
  write alike (`tests/yq_cli/i5053_crlf.rs`). The nine `meta_assign_*_2796` cases are
  therefore listed with the other CRLF exemptions: they keep a leading `# top` and yq
  prints `# top\r\n` for it under CRLF, as succinctly now does.

[#2811](https://github.com/rust-works/succinctly/issues/2811) closed the placement gap
this section used to record here: a comment above a compact `- k: v` item or a nested
`- - x` item now attaches to the item as a whole rather than its first key/inner item, a
column-0 or dedented comment after nested content (including one flush against a
`---`/`...` boundary) is placed by column rather than handed to the outermost node at
that column, a quoted/flow/line-commented value's trailing block stays that node's own
foot instead of forwarding, and a scalar document root gets a foot target of its own
(plain, anchored, tagged or a standalone alias alike) so its head and foot no longer
merge. All measured against pinned yq v4.53.3; see `tests/yq_cli_tests.rs`'s
`standalone_comment_attribution_2811_review` module for the exact shapes.

The same gap covers a bare sequence item's own trailing comment when its value is
*absent* (`- # c` followed by a sibling item, a dedent, or end of input — #1079's
deferred half). Real yq has no node for that comment and floats it: forward onto the
next item of the same sequence as its `head_comment` (accumulating across consecutive
absent items), or — when the sequence closes past a dedent first — the last one becomes
a `foot_comment` (of the key whose value the sequence is, or of the next outer item when
the sequence is nested straight inside an item) while the rest go forward, or at end of
document all of them become the root's `foot_comment`. The parser now attributes these
the way yq reports them, so the getters agree (`- # c\n- 2\n` answers `c` for
`.[1] | head_comment` and nothing for `.[0] | line_comment`), and both emitters now print
the floated comment itself in the right place (#2795 PR A/B) — `- # c\n- 2\n` prints its
`# c` between the two items on both routes. A separate, pre-existing, unrelated gap
remains on the item's own *value*: real yq prints a bare `-` for the absent item (`-\n#
c\n- 2\n`), while succinctly's DOM route materializes it as an explicit `null` (`- null\n#
c\n- 2\n`) — an `OwnedValue` representation gap, nothing to do with comment placement, out
of scope here. One placement is a recorded
divergence, not a gap: real yq *drops* every floated comment but the last when a
sequence holding several closes to an outer item or to end of input inside a mapping
(`- k:\n  - # c\n  - # d\n- 2\n` keeps only `d`), which ADR-0018 rule 4 forbids
matching — the rest go forward as the next node's head instead of being discarded to
match. A comment *floated* off an absent bare item heads a compact-mapping or nested-sequence
sibling in the same sequence itself, as in yq (`- # c\n- b: 1\n` is `.[1] | head_comment`; it used to
land on the sibling's first key, #2835). Two neighbouring shapes still differ and are not licensed
by that rule: a second absent item between them (`- # c\n-\n- b: 1\n` puts `c` after `b: 1`, where
yq heads `.[2]` with it) and a float from a nested sequence that closed at the sibling
(`- - # c\n- b: 1\n`, yq prints it as the foot of the sibling's entry). An explicit-key sibling
(`- # c\n- ? k\n  : v\n`) is headed too. Pinned by
`test_2835_c_neighbouring_shapes_still_differ` in `tests/yq_cli/i2835.rs`. Blank lines in any
of these shapes are unmodelled on both sides (yq keeps a `\n` inside the slot value and
reorders).

A tag or anchor alone on the line under a bare `-` or `key:` names an empty node (#2835), and
a comment on that bare `-` goes forward as the next node's head, as in yq. So does a comment after
the properties themselves (`a:` / `  !!str # d` / `b: 1` prints `b: 1 # d`, #4851), and a tag or an
anchor given on one line cannot be given again on the next (`a:` / `  !!str` / `  !!int` is refused,
as in yq; the error reads `inconsistent indentation` where yq's reads `did not find expected key`).
A comment after the properties is claimed by a block scalar header on the next line and prints on the
header line, as in yq (`a: !!str | # d`, #5001). When a comment is already pending from the line above
(`- &x # c1` / `  !!str # c2` / `  v`), or the value line has a comment of its own (`!!str # d` /
`v # e`), only the comment of the value's own line survives, where yq keeps both.
Comment lines between a bare `-` and the properties (`k:` / `  - # c` / `    # x` / `    !!str` /
`z: 1`) head whatever opens after the empty node, as the dash line's own comment does (#4851), where
they used to be placed by their column: the foot of `k`, printed with a blank line after it. One
small difference remains, pinned by `test_4851_g` in `tests/yq_cli/i4851.rs`: at the
end of the document (`k:` / `  - # c` / `    !!str` and nothing after it) yq drops the comments
outright and this keeps them as the document's foot, since matching would discard them; the same
before a `---` or `...` (`k:` / `  -` / `    # x` / `    !!str` / `---`). One more shape differs,
pinned by `test_4851_h`: a comment after the properties line
(`    !!str` / `  # y`) prints after the block where yq prints it first, then a blank line, then the
block. A blank line between the properties and the next key matches yq since #5053.

A bare item's comment before a value that *does* follow (`- # note` / `  k: v`, #1079's other
half) is the head comment of that value's first node in yq: the first key of a block mapping or the
first element of a block sequence (rendered on the dash's line), and the item itself for a scalar,
an empty or a flow container (rendered above the dash). The DOM write route now carries it
through (#4231: `.a[0].b = 5` over `- b: 1\n- # own\n  c: 2\n` keeps `# own`; it used to drop it).
One gap remains on the cursor route: printing such an item by itself (`.a[1]` over that document)
omits the comment, where yq prints `# own` above `c: 2` ([#4256](https://github.com/rust-works/succinctly/issues/4256)).
One other shape keeps losing it: an alias item (`- # al` / `  *x`; the DOM writer has no head
comment for an alias, the explicit #2796 error). A scalar or flow item answers its dash comment as
its head, and comment lines between the dash and such a value head it too
([#4268](https://github.com/rust-works/succinctly/issues/4268), a blank line between the own-line
comment and the value included since #4996); one shape still differs, pinned by `test_4268_c` in
`tests/yq_cli/i4268.rs`: a comment on the dash line together with one below it (the printed order is
the own-line one first, yq's the dash one first; the `head_comment` getter answers in yq's order).

A block scalar whose `|`/`>` header is on a line of its own below the owning `-` or `key:`
(`a:` / `  >` / `  text`) reads its text since #4259, and so does one with an anchor or tag in
front of the header on that line (`&x |`, `!!str >`), or on a line of their own above it (`&x` / `|`;
[#4271](https://github.com/rust-works/succinctly/issues/4271)). A comment after properties that stand
alone (`&x # c` / `|`) still reads wrong, as it did before.
An explicit indentation indicator on such a header is measured from the key's own column below a compact key too, with a property after the colon or without (`- k: &y` / `  >2` / `    t` is `[{"k":"t\n"}]`, as in yq; [#6490](https://github.com/rust-works/succinctly/issues/6490)).
One shape is still read where yq refuses it: a header at the column of an inner dash of a nested compact key (`- - k:` / `  |` / `    t`, [#6540](https://github.com/rust-works/succinctly/issues/6540)).
yq answers `did not find expected '-' indicator` there, and `succinctly yq` reads an empty item beside a key with a null value.

Four known gaps this write shares with every other write form, none specific to #798:

- **No read-after-write within one pipe.** The `line_comment`/`style`/`anchor` GET-forms
  still read the original YAML cursor, never the write pass's `CommentTree`, so `.a anchor
  = "z" | .a | anchor` returns `""` where real yq returns `"z"` (live-verified). A written
  root `line_comment` never renders in either tool, but on succinctly's side that is the
  block-container rule below at work (`. line_comment = "q"` on `[1, 2]`, a flow root,
  does render `[1, 2] # q` in both).
- **Only a trailing static navigation keeps the presentation after the write** (#2706).
  `.a.b style = "double" | .a`, `.a.b = "y" | .a.e` and the like print what the same
  navigation prints on the written document, as yq does: the write runs as before and the
  tail (`.a`, `.a.b`, `.a[0]`, `.a?`) is then walked over the result and its tree. A
  navigation that is *not* the tail still drops every comment, style and anchor, because the
  result's `CommentTree` is lost once the pipe stops being shape-preserving: `.a.b = "y" |
  .a | .f = 2` prints `b: y` where yq prints `b: y # c`. A tail that lands on an alias
  prints the resolved value, as a root `*name` is never emitted (`.c = 2 | .b` on `a: &x
  [1, 2]` / `b: *x` is `*x` in yq), and one that lands on a `<<: *base` mapping prints the
  merged members where yq prints the `!!merge <<: *base` key: both are the write routes'
  existing alias and merge-key behaviour, not specific to the tail.
- **A multi-result filter carries no `CommentTree` at all** (`.a line_comment = "y" | .,
  .` prints two uncommented copies) — #1349's `GenericResult::Many` limitation, above.
- **A metadata write behind a navigation that is not a static path is refused, where real yq applies it**:
  `.a[] | .b line_comment = "hi"` on `a:\n  - b: 1\n` prints `b: 1 # hi` in yq (live-verified) and raises here (#2679).
  A static path (`.a`, `.a.b`, `.[0]`, a pipe of them) is applied: the rest of the pipe runs on the navigated node as a document of its own, with that subtree's presentation, as `.a | .b line_comment = "hi"` does in yq.
  A write that reads the node's own context (`.a | .b line_comment = (key)`, `path`, `parent`, `line`, `column`, a comment, style, anchor or tag) is refused as well, since the navigated snapshot cannot answer it.
  A path whose node is absent (`.zz | .b line_comment = "x"`, where yq writes onto the `null` it leaves) and a navigation that answers several nodes (`.[]`, `.a[]`) are refused.
  `resolve_meta_assign_writes` only runs when the whole expression passes `is_shape_preserving` (`src/jq/eval.rs`), whose fixed operator list has no bare navigation stage, and a write after one belongs on the navigated subtree, which no route here has a presentation tree for.
  Such a write used to exit 0 with the write silently dropped, the outcome #798's triage ruled out; the refusal is the divergence ADR-0018 allows when matching would discard a write.
  A leading user-defined `def` that a stage ahead of the write reads through (`def f: .; f | .a line_comment = "hi"`) is refused the same way, since each stage is re-evaluated alone with no `def` in scope; real yq has no `def`, so there is no oracle.
  A document that holds an alias anywhere is refused too (`.y | .m line_comment = "kk"` on `x: &k\n  m: 1\ny: *k\n`): the navigated node can be an alias, or hold one whose anchor lies outside it, and yq prints those as `*k`, which is unreadable on its own and which a root `*name` never is here.
  Pinned by `metadata_write_behind_navigation_or_def_is_refused_not_dropped_2679` and `tests/yq_cli/i2679.rs`.
  An ordinary write behind a static navigation (`.a | .b = 5`, `.a | del(.b)`, `.a | sort_keys(.)`) takes the same route, so the subtree keeps its comments, flow style and quoting as in yq (#5286).
  [#2855](https://github.com/rust-works/succinctly/issues/2855)'s `sort_keys(f)` was the first case of the loss to be noticed; it is pinned by `test_yq_sort_keys_composes_with_pipe_2855` (`tests/yq_cli_tests.rs`) and `tests/yq_cli/i5286.rs`.
  What remains is the loss that already exists without a navigation: a write that replaces a node with a different shape drops the comment on the node it replaced (a trailing `# c` on `d: [1]`, a standalone `# note` above `d:` or above a scalar a container replaces), where yq keeps it ([#5258](https://github.com/rust-works/succinctly/issues/5258)).
  A write followed by a read behind the navigation (`.a | (.b = 1) | .c`) is not a presentation-preserving write as a whole and still takes the old route, which loses the subtree's flow style.

Finally, one rule real yq decides by the target's value, reproduced rather than "fixed":
a `line_comment` written onto a **block-rendered container** is dropped (`.a line_comment
= "y"` on `a:\n  b: 1` prints it unchanged; so does a block root), while a flow or empty
container keeps it on its one line (`a: {b: 1} # y`, `a: [] # y`). The emitter's own
`defers_to_own_block` is exactly that split. A multi-line comment (`"m\nl"`) renders
go-yaml's way: first line after the value, each further line as its own comment line at
the key's indent, an empty line left empty, and — top level only — one blank line before
the continuation (`a: 1 # m` / blank / `# l`, but `  b: 1 # m` / `  # l` when nested).

### Object construction: yq's `{...}` is `COLLECT_OBJECT`, not jq's sugar (#2783)

Real yq's `{...}` grammar is `{ expr : expr , expr : expr , ... }`. `:` is the binary
`CREATE_MAP` operator, `,` is `UNION`, and the braces are `COLLECT_OBJECT`. Both sides of a
pair are ordinary expressions, a bare identifier is not a token at all, and an entry with no
`:` is legal -- it is simply not a pair. Confirmed live against yq v4.53.3:

```console
$ printf 'x: 1\n' | yq '{x}'
Error: 1:2: lexer: invalid input text "x}"
$ yq -n '{b: 2}'
Error: 1:2: lexer: invalid input text "b: 2}"
$ yq -n '{.a: 5}'            # key is an expression
1: 5
$ printf 'x: 1\n' | yq '1 as $a | {$a}'
$ echo $?
0
```

`succinctly yq` follows that grammar:

- **Keys are expressions.** `{.a: 5}`, `{1: 2}`, `{"a" | "b": 1}` and `"k" as $a | {$a: 3}`
  all parse. A scalar key stringifies (`{1: 2}` is `"1": 2` under `-o json`, as in yq).
- **Bare identifier keys and the `{x}` shorthand are rejected** by default, matching yq's lexer
  error. They are succinctly's jq-styled sugar, so they sit behind `--jq-extensions` with the
  rest of that surface ([#1512](https://github.com/rust-works/succinctly/issues/1512)); the flag
  keeps `{b: 2}` and `{x}` meaning what they did. `{$a}` is a bare *entry* in both modes, never
  jq's `{a: $a}` sugar -- the mode decides, not the flag.
- **A bare entry is reproduced bug-for-bug.** Each entry is evaluated, the entries are
  `UNION`ed, and `COLLECT_OBJECT` reads the first node's child count as `N`, errors with
  `CollectObject: mismatching node sizes; ...` if a later node has fewer children, and for
  each `i < N` splats the `i`-th children (a sequence's items, a mapping's values, nothing for
  a scalar) and cross-multiplies them with `*`. The visible consequences:

  | filter (`-n`)                                    | yq and `succinctly yq`                          |
  |--------------------------------------------------|-------------------------------------------------|
  | `1 as $a \| {$a}`, `{"k"}`, `{[1,2]}`           | no output, exit 0                               |
  | `{$undefined}`                                   | `{}`                                            |
  | `{"b", "a": 1}`                                  | no output (the first node is a scalar, `N` = 0) |
  | `{"a": 1, "b"}`, `{"z": (1,2), 5}`               | `CollectObject: mismatching node sizes` error   |
  | `[[{"k":1}]] as $a \| {$a}`                     | `k: 1`                                          |
  | `[{"k":1}] as $a \| {$a}`                       | `1`                                             |
  | `[{"k":1}] as $a \| {"z": 0, $a}`               | error (`cannot multiply !!map with !!int`)      |

  yq evaluates every entry before combining any, so an error inside one entry aborts the whole
  construction with no prefix, unlike jq's streaming fan-out.
- **A repeated key deep-merges (#4182).** yq folds the entries of a `{...}` with `*`, so
  `{"a": {"x": 1}, "a": {"y": 2}}` is `a: {x: 1, y: 2}` where jq (and a pair-only construction
  here before #4182) keeps the last. Only a map over a map merges; for anything else the later
  value wins, as in yq. The merge happens where a construction's pairs become an object, so it
  covers a literal key that repeats and a computed key that turns out to (`{($k): .., ($k): ..}`)
  alike, on every route, and costs nothing when no key repeats. It is the same fold the
  `COLLECT_OBJECT` route above runs.
- **A pair with no maps (#4193).** A `key: value` pair whose key or value yields nothing
  contributes no map to the fold, and the fold restarts after it: `{"a": empty, "b": 2}` is
  `b: 2`, `{"a": 1, "b": empty, "c": 3}` is `c: 3`, and `{"a": 1, "b": empty}` is nothing, where
  jq (and this fan-out before #4193) ends the whole construction at the first empty entry. yq
  also evaluates every entry before it folds, so `{"a": empty, "b": error("boom")}` raises
  `boom` rather than printing nothing. An empty operand always empties the cross product, and when
  none is empty the two are the same thing, so a construction whose operands might be empty is built
  as the cross product first and rebuilt as the fold only when that comes out empty (#4240): a record
  pays the fold only if its construction is empty. A construction whose operands are all provably
  total (`yields_at_least_one_value`: literals, variables, `length`, `map(..)`, operators and pipes
  of those, ...) is never rebuilt. A path is not total -- yq yields nothing, not `null`, for a key
  or an index applied to a string, number or boolean, so `{"a": .m.id, "b": .n}` over `{m: s, n: 2}`
  is `b: 2` -- and nor is anything else that can come back empty (`select`, `.[]`, `?`). An operand
  that might be empty and has a side effect (`debug`, `stderr`, `input`, a user function) goes straight
  to the fold rather than run twice. Cost, M4 Pro, interleaved, 7 and 14 MB `users` documents,
  against the fold-first route #4193 shipped: a `select` in a pair -5% to -8% (and -1% to +3% against
  the fan-out it replaced); a construction that is empty for every record is built twice, +54%
  (`.users[] | {"n": .name, "s": (.score | select(. > 1e8))}`). jq mode never takes the fold.
  One shape stays on the fan-out although yq folds it: `$x` with no `as` binding it (yq
  yields nothing for it, so `{"a": 1, "b": $x, "c": 3}` is `c: 3` there and nothing here) -- a
  variable is judged total because a bound one is by far the usual case.

Residual divergences remain, all in cases yq itself reaches through its node model:

- **Non-scalar keys.** `{([1]): 1}` and `{"k":1} as $a | {($a): 3}` render a complex `? key`
  mapping in yq; `OwnedValue` object keys are strings, so `succinctly yq` raises its usual
  `Cannot use array as object key` error instead (rule 4(a): there is nothing to write the key
  into that we could read back).
- **Several inputs.** After a fan-out (`.[] | {.k}`), yq runs `COLLECT_OBJECT` once over every
  matching node, so `N` is the node count and the size check spans all of them;
  `succinctly yq` builds each input's object on its own. `.[] | {.k}` over elements whose `k`
  has different lengths is the `CollectObject: mismatching node sizes` error in yq and no output
  here, and `[.l[] | {.nope}]` is `[{}]` in yq and `[{},{}]` here (#4184). Pair-only
  constructions are unaffected (one object per node either way) unless a pair can be empty
  (#4193): then the fold spans every node too, so `.v[] | {"n": .p, "m": (.q | select(. > 1))}`
  over three nodes of which the middle one has no `m` is nothing in yq, and `{"n":1,"m":5}
  {"n":3,"m":7}` here.
- **`,` over two reads of one variable (#4183).** A `,` drops its right operand when both operands
  are the very same list (a pointer comparison in `operator_union.go`), and a variable read hands back
  the list the variable is stored in, so `[1] as $z | ($z, $z)` is `[1]` once, as `., .` is, and
  `[1] as $z | [$z, $z]` is `[[1]]` (`succinctly yq` agrees; two different variables keep both). A
  stage after the first of a pipe runs in a child context that copies each variable's list, so
  `[$z, . | $z]` keeps both, and `[($z | .), $z]` collapses. The earlier reading of this
  (`1 as $z | 2 as $y | $z, $y` printing only `1`) was the scope of an `as` body, which ends at a
  comma in yq as a pipe binds tighter than `,`: `(1 as $z | 2 as $y | $z), $y`, so `$y` is unbound
  and reads as nothing (`.items[] as $i | $i.name, $i.id` prints only the names); `succinctly yq`
  parses it that way now. The collapse is keyed on the variable's name, so a variable the host binds
  (`--arg`, an extension real yq has no flag for, and `$index` under `--split-exp`) follows the same
  rule unverified. Not matched: a bare entry of an object, where the collapse shows differently
  (`[[1,2],[3,4]] as $z | {$z, $z}` is `1 2 3 4` in yq and `1 2 2 4 9 12 12 16` here).
- **A parenthesized pair (#4185).** `{"x": 1, ("a": 2)}` is `{"x":1,"a":2}` in yq (`:` is an
  ordinary binary operator). `succinctly yq` flattens a parenthesised run of pairs at an entry
  position into the object's own entries, in order (`{("a": 2, ("b": 3))}` is `{"a":2,"b":3}`);
  a group holding a non-pair (`("a")`, `("a": 1, 2)`) or followed by an operator
  (`("a": 2) | length`) is the parenthesised expression it always was. Still different: a pair
  outside an object (`("a": 2)` alone is `[[{"a":2}]]` in yq, a sequence nested in a sequence, a
  shape with no `Expr` here, so it stays a parse error), a chained pair (`{"x": 1, ("a": 2: 3)}`
  is `{"x":1,"a":[[{"2":3}]]}` in yq and a parse error here), and a group mixing a bare entry with
  a pair (`{("a","b": 3)}` prints nothing in yq, a parse error here). Only a
  group with a `:` at its own level is tried (a parenthesised key, a slice and a string are not),
  and a failed attempt parses the text again, so a parse draws on a budget of 256 attempts; past
  it a group is parsed as the parenthesised expression it is.
- **The `*` error text.** The error a failed multiply raises is yq's `cannot multiply !!map with
  !!int` (#6904), the same as every other `*`; it was succinctly's `object (...) and number (1)
  cannot be multiplied` until then.

### `envsubst` ([#4872](https://github.com/rust-works/succinctly/issues/4872))

The substitution table was fitted to yq v4.53.3 by a fuzz over about 2,600 strings (tokens: `$`,
`${`, `}`, names set / empty / unset, the operators `-` `:-` `=` `:=` `+` `:+`, `$$`, text) under every
flag combination, not read from the Go source: every string whose `${` is followed by a variable name
matches. Only two shapes differ, both a lexer artifact in `a8m/envsubst`: a `${` followed by something
that is not a name (`${${MISSING-d}...`, `${ =...`, `${:=...`), where yq's own answer depends on a nested
brace scan this does not model (a `${-}`, `${@}`, `${.}` or `${}` standing alone is literal text, as in
yq); and a lone `${_` (yq prints it literally, `${_U}` is a variable). `${FOO^^}`, `${FOO:1}`,
`${FOO/a/b}` and `${FOO:?m}` are `$FOO` in yq, which is mirrored. An operand (`d` of `${X:-d}`) has its own `$Y` and `$$` substituted, as in yq; a nested `${` in an operand (`${X:-${Y}}`, which yq reads as `${Y` with a stray `}`) is left as written. `envsubst` reads the process
environment, so it needs the `std` feature.

### `with`, `from_json` and `from_yaml` residuals (#4206)

`with(path; update)` is `path |= <what update changes>` (`with_update_effect`): assignments,
commas, `select(f) | ...`, a navigation in front of an assignment, and a binding
(`.b as $v | .z = $v`, whose source is evaluated and whose body contributes its changes). A
value-producing stage is evaluated and its value discarded, as in yq, so its *error* counts
(`#4327`): `{"a":{"b":1}} | with(.a; . + 1)` fails in yq (`!!int () cannot be added to a !!map
(a)`) and here (`!!int () cannot be added to a !!map ()`; the key in parentheses is the general
arithmetic-error difference recorded above, not specific to `with`). A third or later argument is accepted and
ignored, with only the path evaluated, and the input comes back untouched, as yq does
(`with(.a; . = 1; 2)`). Still different: a stage after a discarded value-producing stage is not
evaluated (yq runs it on the discarded value); `(.b = 2) as $x | .c = $x` (a binding whose
source is an assignment) does not apply the source's change; a binding whose source yields several
values applies its body once (yq: once per value, `with(.a; .[] as $v | .[0] = $v)`); and a bare `empty` update is accepted where yq's lexer rejects it
(#4808's gap). The evaluation costs a `reduce` frame per node `path` selects for a discarded
stage: `with(.[]; . + 1)` over a million-element array, a no-op in yq too, takes 649 ms where it took
299 ms; an update with an assignment is unchanged.
A `del(...)` or `delpaths(...)` in the update is applied like an assignment (#6840): `with(.a; del(.x))` removes `x`, and an assignment piped after it still runs.
Still different: `with(.b[]; del(.))` empties the array in yq and changes nothing here.
A value-producing head that yq changes in place and hands on (`with(.a; sort_keys(.) | .z = 1)` writes `z` in yq) drops the assignment after it here.
A write wrapped in `//` (`with(.a; del(.x) // 1)`, `with(.a; (.z = 1) // 2)`) is dropped here and applied in yq.

`from_yaml` of text the semi-index does not reject still decodes where yq fails: `"--- \n..."` is
`null` and `"a: !!int x"` is the string `x` (`"a: b: c"` used to be `{"a":"b"}` and is yq's
`mapping values are not allowed in this context` since #4463) (the general
minimal-validation trade-off of semi-indexing, not specific to the builtin).

`to_yaml` is the `@yaml` encoder, so it has `@yaml`'s gaps (a multi-line string is double-quoted,
not a literal block; a container the filter builds is block; see the `@yaml` table below), and
`to_json` of a string holding a backslash-`u` escape in a filter literal differs because filter
literals are unescaped here (`"\u0001"`) where yq keeps the backslash. `to_json`/`to_yaml` print
the same bytes as `@json`/`@yaml` plus yq's indent and trailing line break; `tojson` is
`to_json` (#6158).
An indent past 100000 is refused at parse time (yq would try to build it).

`from_json`/`from_yaml` decode through the same scalar resolution as input documents, so an
implicit-tag scalar yq types specially keeps succinctly's type: `"2020-01-01" | from_json | tag`
is `!!timestamp` in yq and `!!str` here (a date in a *document* has the same gap).

### Other categories

Float and number formatting ([#1071](https://github.com/rust-works/succinctly/issues/1071),
[#1356](https://github.com/rust-works/succinctly/issues/1356),
[#1358](https://github.com/rust-works/succinctly/issues/1358)), error-message rendering
(succinctly's previews format via jq's rules rather than yq's verbatim echo —
[#1055](https://github.com/rust-works/succinctly/issues/1055)), comment placement
([#1079](https://github.com/rust-works/succinctly/issues/1079); chained anchored keys,
[#1080](https://github.com/rust-works/succinctly/issues/1080), are fixed) — #1085's gap (a floated
anchor comment and a key's own comment couldn't coexist) is fixed on the streaming/get-form
route and, since [#2806](https://github.com/rust-works/succinctly/issues/2806), on the DOM
route too, for a key whose value is a block container or a scalar (`-P` and every write form
print the floated comment on the key's line and the key's own comment on a line of its own at
the key's indent, as yq does). An explicit key under such an anchor keeps the floated comment
too ([#4384](https://github.com/rust-works/succinctly/issues/4384)): a scalar, quoted, block-scalar
or empty `? b # own` shows it ahead of its own on every route, and a key that is itself a
collection (`? [1, 2]`) does not take it, as in yq, but still reads as `""`. Still different: a flow-valued or empty-valued key (yq moves both
comments onto the next entry, `b: {c: 1}` / `d: 3 # floated` / `# own`; succinctly keeps the
floated one on `b`'s line and drops the other), and a key whose value is
`null`.
A node's tag and quoting style are lost only on the routes that rebuild a sequence or mapping from document nodes and keep none of its presentation, which
[#5429](https://github.com/rust-works/succinctly/issues/5429) still tracks (a variable, a further stage after a construction, an iterating operand inside one, a computed slice bound, `map_values(select(...))`).
`-o=json` output is unaffected, since neither appears in JSON.
An object slice is the one case that stays a recorded divergence: yq prints the source mapping's `!!map` and the key's quoting on the sequence it returns (`{"a":1,"b":2} | yq
'.[0:2]'` is `!!map\n- "a"\n- 1` on real yq, plain `- a\n- 1` on succinctly; [#1102](https://github.com/rust-works/succinctly/issues/1102), [#1416](https://github.com/rust-works/succinctly/issues/1416)).
`OwnedValue` has no tag slot and ADR-0017 rules out adding one for presentation data.
Also see
[yq Query Language Reference § Known Limitations](../../reference/yq-language.md#known-limitations)
for the feature-level gaps (position builtins after DOM conversion; `file_index`/`key`/
`document_index` inside object literals or `any`/`all`).

### An empty mapping in `join` is `{}` only when the expression spelled `{}` ([#1791](https://github.com/rust-works/succinctly/issues/1791), [#1047](https://github.com/rust-works/succinctly/issues/1047))

`join` writes a mapping element or separator as the empty string, except an empty one that is the expression's own `{}` literal, which it writes as the text `{}`.
`[{}] | join(",")` is `{}` and `[1,2] | join({})` is `1{}2`, while a `{}` node of the document is `""`: `- {}` / `- x` joins as `,x`, and so does the same node reached through `$x`, `[.[0]]` or `del` (`[{"a":1} | del(.a)] | join(",")` is `""` too).
Captured from yq v4.53.3.
An element that is a document `{}` is `""` here too since #1791 (it was `{}`, from a mis-read oracle in #1047).
Not matched: an array reaches `join` the same way whether the document or the expression made it, and a value carries no origin, so the literal `{}` *element* (`[{}] | join(",")`, `[{}, 1] | join("-")`) is `""` here and `{}` in yq.
It follows the document case because that is the one real inputs hit.
The separator is an evaluated expression: `join({})` is `1{}2` as in yq, and a `{}` separator that came from the document through `$s` is `1{}2` here for the same missing-origin reason (`. as $s | [1,2] | join($s)` over `{}` is `12` in yq).

## `tostring` of a container is YAML since #1130 ([#2902](https://github.com/rust-works/succinctly/issues/2902), [#1130](https://github.com/rust-works/succinctly/issues/1130))

Real yq's `tostring` on a container is its YAML encoder (block style, `!!float` tags
where the spelling needs one, a node written in flow style kept in flow).
Its `tojson` is `to_json`, its indented JSON encoder, which ends every string it produces --
scalar or container -- with a newline of its own; `succinctly yq` reads `tojson` as `to_json` since [#6158](https://github.com/rust-works/succinctly/issues/6158), `tojson(0)` included.

`tostring` now matches (#1130): a container becomes the YAML the CLI would print for it, with
the one trailing line break chomped (`[1, 2] | tostring` is `"- 1\n- 2"`, `[1.0 * 3]` is
`"- !!float 3"`), through the CLI's own writer, so the two cannot drift. A container read from
a node the source wrote in flow style (`b: [1, 2]`) stays flow (`"[1, 2]"`). jq mode is
unchanged: it JSON-encodes.
Since [#6901](https://github.com/rust-works/succinctly/issues/6901) a container that is a document node is encoded with its own presentation: a flow container nested in a block one stays flow, and the quote style, trailing comments and the `*a` of an alias the source carried survive into the text.
Not matched:

- `-I4` is ignored (yq indents the text by the flag; ours is always two spaces);
- a container the filter builds, rebuilt by `with_entries`, behind a write or read under `--eval-all`
  takes the block style whatever its source, and carries none of the comments, quotes or aliases;
- a string with a line break is double-quoted (`"l1\nl2\n"`) where yq writes a literal block
  (`|`), the CLI writer's own rendering of a built value.

String interpolation matches too (#6092): the slot of `"I \(.m)"` is a `to_string` stage, so a
mapping or sequence interpolates as the YAML text `to_string` makes of it (`I {an: apple}` over
`m: {an: apple}`, `"x\([1, 2])y"` is `"x- 1\n- 2y"`), and a node written in flow style stays flow.
The slot is read at its cursor for plain navigation and `//` of it (`"\(.m)"`, `"\(.a // "d")"`).
Any other slot (a call, `..`, an assignment's right-hand side) is read from an owned copy, which carries
no style, so a flow container there interpolates as block: `.x = "got \(.m)"` writes `got an: apple`
where yq writes `got {an: apple}`.
jq mode is unchanged: it interpolates `tostring`'s JSON text, as jq does.

`tojson` is unchanged and still renders compactly, without that trailing newline. Captured
live (v4.53.3, `null` on stdin):

| filter                   | real yq                | succinctly yq |
|--------------------------|------------------------|---------------|
| `[1] \| tojson`          | `[\n  1\n]\n` + `\n`   | `[1]`         |
| `"x" \| tojson`          | `"x"\n` + `\n`         | `"x"`         |
| `(0.5+0.5) \| tojson`    | `1.0\n` + `\n`         | `1.0`         |

Scalar `tostring` agrees in both (the #2902 fix). The `tojson` tests in
`tests/yq_cli_tests.rs` compare trimmed output for this reason.

## `succinctly yq -o=json -C` colorizes with jq's own scheme, not real yq's (pre-existing, recorded while verifying #3413)

`succinctly yq`'s JSON output colorizing (`-o=json -C`, and the JSON side of `-P -C`) reuses
`colorize_json_per_token` -- the same tokenizer and `ColorScheme::default()` `succinctly jq` uses,
but with the per-token framing `succinctly jq` had before #5473 (jq's own bytes now differ from it) --
rather than a yq-specific scheme, confirmed by `test_colorized_json_output_is_token_aware`'s
own comment ("goes through the shared jq colorizer"). This predates #3413; #3413 only changed
which color that shared default assigns to `null` (`1;30` to jq 1.7.1's own `0;90`), which
this entry's own scope doesn't touch. Real yq v4.53.3's `-o=json -C`/`--colors -o=json` output
is a different scheme entirely -- confirmed live:

```console
$ printf 'a: null\nb: true\n' | yq --colors -o=json '.'
{
  "a": null,
  "b": true
}
```

(the real bytes carry `\x1b[36m`/`\x1b[0m` around each *key*, and `\x1b[95m` around `true` --
but `null` itself is never colored at all, unlike `succinctly yq`'s bright-black wrap adopted
from jq). No rule-4 condition licenses this on its own merits (the output is readable, nothing
is corrupted, and matching real yq's own JSON-coloring scheme is not a resource cost anyone has
measured) -- it is recorded here because it was never written down, not because it was newly
decided. `succinctly yq`'s YAML-target coloring is a separate code path that has matched yq byte for
byte since #5416, so closing this would mean either giving JSON-target output its own yq-specific
colorizer or accepting the jq-styled one as a permanent, documented extension under ADR-0018
rule 5 -- a decision this entry doesn't make, only surfaces.

## Evaluator resource caps apply in yq mode too, and are uncatchable (#2132)

The caps `succinctly jq` documents -- `MAX_RANGE_TOTAL`, `MAX_LOOP_ROUNDS` (which a tail-recursive `def` shares with `while`/`until`, #5379), `WHILE_UNTIL_MAX_STEPS`,
`REDUCE_FOREACH_MAX_STEPS`, `repeat`'s `MAX_ITERATIONS`, `MAX_EVAL_FRAMES` -- are shared
evaluator limits, so they apply to `succinctly yq` as well; real yq v4.53.3 has none of them.
The reachable ones are `while`/`until` and a recursive `def`, both behind `--jq-extensions` since #4808
(`range`/`repeat` are gated too, #1512). Since #2132 the raise is `ErrorKind::ResourceLimit`,
which `?`/`try`/`catch` never swallow, in both modes alike:

```console
$ printf 'a: 1\n' | succinctly yq --jq-extensions '[.a | while(true; (1, 2))] | length'
Error: while: maximum iterations exceeded
$ printf 'a: 1\n' | succinctly yq --jq-extensions 'def f: f; try f catch "caught"'
Error: f/0 exceeded maximum recursion depth
```

This is a rule 4(b) decision, not a mode difference: the caps are not a reference behaviour in
either tool, so there is no yq catch semantics to match, and letting `try` swallow one would
report a truncated result as a clean answer. See the jq page's "Every resource cap is
uncatchable" section for the full account and oracle rows; pinned by
`test_resource_limit_caps_are_uncatchable_in_yq_mode_2132` (`tests/yq_cli_tests.rs`).
A tail call that never ends (`def f: f; ...`) stops at the round budget (`MAX_LOOP_ROUNDS`, #5379) with the same refusal, pinned by `test_a_call_that_never_ends_still_ends_in_yq_mode_5379`.

## Where the two modes deliberately differ from each other

Not divergences — these are ADR-0018 rule 2 working correctly. The same filter text means
different things in `sjq` and `syq` because the two reference tools disagree, and succinctly
follows each one in its own mode. The behavioural axis is `EvalSemantics`
([src/jq/eval.rs](../../../src/jq/eval.rs)) — twelve `const`s plus an `EvalTag` identity tag
— together with a handful of `S::TAG` tests at individual call sites:

| Behaviour | `succinctly jq` (jq 1.7.1) | `succinctly yq` (yq v4.53.3) |
|---|---|---|
| Bare 2-arg `sub(re; s)` | first match only (`"aaa"` → `"Xaa"`) | **every** match (`"aaa"` → `"XXX"`) |
| `@uri`/`@base64`/`@html` on a container | JSON-encodes first (`[1,2]` → `"%5B1%2C2%5D"`) | errors, as real yq does (`@html` is not a yq format; it parses only under `--jq-extensions`, #5444) |
| `@base64d` on malformed/misplaced padding | truncates at the first `=`, decodes the rest (`"===="` → `""`) | yq's base64 decoder, over Go's stream decoder: misplaced padding is refused (`"===="` → error), and a line break inside the text is `unexpected EOF` (#6997) |
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
surface. The same gate applies to jq's generic postfix `?` after a parenthesized
expression, builtin, literal, or variable
([#3378](https://github.com/rust-works/succinctly/issues/3378)); yq's field and bracket
optionals (`.x?`, `.[0]?`) remain available by default. `leaf_paths` is grouped here even
though it isn't real jq syntax either — real jq
itself rejects it too (`leaf_paths/0 is not defined`; it's a succinctly-only invention
modeled on a jq community recipe, see CLAUDE.md) — because, from `succinctly yq`'s
syntax-surface point of view, it's the same kind of thing as the rest of this list: extra,
off by default.

**The rest of the jq-only names, gated by [#4808](https://github.com/rust-works/succinctly/issues/4808).**
Real yq's lexer rejects every one of these with `invalid input text` (pinned yq v4.53.3), and `succinctly yq` now refuses them without the flag as well.
Until #4808 they were the "known gap" table of the #1887 sweep, parsed without the flag.
Control flow and definitions: `if`/`then`/`elif`/`else`/`end`, `try`/`catch`, `label`/`break`, `def`.
Loops and recursion: `while`, `until`, `repeat`, `recurse`, `recurse_down`.
Output and process control: `empty`, `halt`, `halt_error`, `stderr`.
Type selectors and tests: `values`, `nulls`, `booleans`, `numbers`, `strings`, `arrays`, `objects`, `iterables`, `scalars`, `normals`, `finites`, `isnull`, `isboolean`, `isnumber`, `isstring`, `isarray`, `isobject`, `isvalid`.
The rest: `in`, `keys_unsorted`, `ltrim`, `rtrim`, `utf8bytelength`, `toboolean`, `tojsonstream`, `fromjsonstream`, `builtins`, `modulemeta`.
The owner chose to gate all of them in one change, before the next release, rather than group by group or keep them ungated: nothing released has the gate, so one release note covers it, and the old split (`reduce`, `limit` and `last` gated while `def`, `if` and `try` were not) had no principled line.
Little capability is lost: yq's `keys` already keeps document order, `..` is its recursion, `select(tag == "!!str")` is its type selector, `trim` covers `ltrim`/`rtrim`, and `select`, `//` and `with` cover `if`.
A script that uses one of these under `succinctly yq` adds `--jq-extensions`, and the error already names it.
`every_parsed_keyword_is_classified_for_the_jq_extensions_gate_1887` (`src/jq/parser.rs`) enforces the classification: every keyword the parser matches is gated, gated by call form (`all`, `any`, `path`), gated with its opener (`then`, `elif`, `else`, `end`, `catch`), a yq-native name, or listed in `BY_DESIGN`.
`succinctly jq` is unchanged.

The same gate covers the jq-styled **object-construction sugar** real yq's lexer rejects:
bare identifier keys (`{b: 2}`) and the field shorthand (`{x}`, meaning `{x: .x}`)
([#2783](https://github.com/rust-works/succinctly/issues/2783)). `succinctly yq` rejects them
by default; with `--jq-extensions` they keep their jq meaning. The flag decides on the entry's
first character: an entry that starts like an identifier (`{length, "k": 1}`) is jq's field
shorthand (`{length: .length, ...}`), not yq's bare entry.

The same gate covers jq's **destructuring patterns** in `as` (`. as {a: $v}`, `. as [$a]`, `. as {$a}`) ([#5433](https://github.com/rust-works/succinctly/issues/5433)).
Real yq's `as` takes a variable name and nothing else.
A pattern with a bare-identifier key (`{a: $v}`) fails at its lexer (`invalid input text`).
Every other pattern (`[$a]`, `{$a}`, `{"a": $v}`, `{(.k): $v}`) fails with `RHS of 'as' operator must be a variable name`.
`succinctly yq` rejects every non-`$name` pattern by default, with the same error that names `--jq-extensions`.
With the flag a pattern keeps its jq meaning, and `?//` stays refused either way.

The same gate covers bare `env` and the `@text` and `@html` formats ([#5444](https://github.com/rust-works/succinctly/issues/5444)).
Real yq's lexer rejects all three (`invalid input text`), while `env(NAME)`, `strenv(NAME)` and its other formats are accepted and stay available by default.
`@dsv` is wholly new syntax and stays unconditional.

Two more jq-syntax forms real yq's lexer rejects sit behind the flag ([#5452](https://github.com/rust-works/succinctly/issues/5452)): a string literal inside a `\(...)` interpolation slot (`"\(.a // "d")"`, yq ends the slot at the first `"`) and a number with a trailing dot (`5.`, `1. + 1`; yq answers `bad expression`).
Whitespace between a `.` and a name, a quote or a bracket (`. a`, `. "a"`, `.a . b`) is refused outright in yq mode with or without the flag, as jq refuses it too (#5447).
The root metadata write (`. style = "flow"`, #798) and the terminators after a gap (`. and`, `. as`, `. == 1`) are unchanged.

yq's `~` null literal, its `toyaml`/`fromyaml` spellings and `SRC as $v ireduce (INIT; UPDATE)` (`reduce SRC as $v (INIT; UPDATE)`) are yq syntax and parse without the flag ([#5456](https://github.com/rust-works/succinctly/issues/5456)).
A `~` written into a document prints as `null` in YAML output where yq keeps `~` (`.a = ~` is `a: ~` in yq), the null-spelling loss tracked on [#2802](https://github.com/rust-works/succinctly/issues/2802); `-o=json` agrees.

`array_to_map` ([#5458](https://github.com/rust-works/succinctly/issues/5458)) is a parser-level desugar over `to_entries`/`from_entries`, so it keeps `from_entries`'s string keys where yq's are ints (`.b | array_to_map | keys` is `["0","1","2"]`, yq `[0,1,2]`) and prints a nested container in block style where yq prints flow style; values and `-o=json` output agree.

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

### `--argjson`'s validator changed, and two number renderings moved with it (#2052)

`--argjson` is a jq-inherited extension: real mikefarah/yq v4.53.3 has no such flag
(`Error: unknown flag: --argjson`), so nothing here is measured against an oracle -- it is
recorded because the behaviour changed.

Until #2052 the flag validated through `serde_json::Value` and materialized through it too.
#2052 replaced the validation gate crate-wide with `json::validate::validate_jq_lenient`
(see the jq-mode limitations doc for why) and moved yq's materialization onto the
`JsonIndex` + `to_owned_canonicalizing_numbers_at_depth` pair its `--input-format json` path
already uses. #978's convention is intact -- `--argjson` still discards a literal's source
spelling (`1.500` is `1.5`, `0099999999999999999999999` is `1e+23`) and still does not
preserve it the way `succinctly jq`'s own `--argjson` does (#1058 was deliberately
jq-mode-only). Since [#2902](https://github.com/rust-works/succinctly/issues/2902) that same
canonicalizer types a whole-valued float as an int, as real yq's own JSON decoder does
(`1.0` is `!!int` under `-p json`), so `--argjson x 1.0` and `1.e5` render as `1` and
`100000` where they rendered `1.0` and `100000.0` before.

Two renderings do move, both because `serde_json` is no longer the one producing them
(review of #2880 -- the PR text originally claimed nothing moved):

| value | before | after | why |
|---|---|---|---|
| `-0`, `-00` | `-0.0` | `0` | what `-p json` already answered for the same literal |
| `1.0e50`, `1.e50` | `9.999999999999999e+49` | `1e+50` | `serde_json`'s pow10 path was 1 ULP off |

**A magnitude-overflowing literal stays rejected, and this is where the two modes
deliberately part company** (ADR-0018 rule 2 -- the mode decides). jq 1.7.1 accepts
`--argjson x 1e400` as `1E+400`, and `succinctly jq` now matches it; yq mode keeps
refusing, because real yq's own JSON input path refuses it too and this materializer has
nowhere to put an infinity:

```console
$ printf '{"a":1e400}' | yq -p json '.a'
Error: bad file '-': strconv.ParseFloat: parsing "1e400": value out of range
$ printf 'a: .inf\n' | yq -o=json '.a'
Error: json: error calling MarshalJSON ...: strconv.ParseFloat: parsing ".inf": invalid syntax
```

Accepting it would render `.inf` in YAML output -- which real yq then cannot read back --
and a silent `null` under `-o=json`, which is precisely the outcome #1095 added the
`serde_json` gate to prevent. `has_non_finite_number` (`yq_runner.rs`) now provides
deliberately what that gate used to provide incidentally, and checks the materialized value
rather than the literal, so a nested `[1e400]` is refused too. Pinned in
`test_argjson_reject_set_and_overflow_after_2052` (`tests/yq_cli_tests.rs`).

The same split holds for jq's decNumber *spellings*, which
[#2877](https://github.com/rust-works/succinctly/issues/2877) taught the shared lenient
validator and JSON dispatchers to read (`nan`, `NaN5`, `sNaN`, `Infinity`, `-inf`, and a
leading `+`). `succinctly jq` reads them as jq does; yq mode does not move. Plain `-p json`
goes through the YAML parser and rejects every one of them, as real yq does (`json: invalid
character a as null`, `invalid character '+' looking for beginning of value`). The
`--slurp`/`--eval-all`/`--inplace` DOM route and this flag *do* reach the JSON dispatchers,
and their materializer (`to_owned_canonicalizing_numbers_at_depth`) refuses a non-finite
word outright -- the same "nowhere to put it" reasoning as the overflow rule above, and the
same outcome the old `Error: invalid null` gave. A leading `+` is finite and is admitted
there with the spelling dropped (`--argjson x +1` is `1`, `[+1.500]` is `[1.5]`), which is
the `007`/`.5` mirroring convention this section already records, not a new rule. Pinned in
`test_p_json_input_is_unmoved_by_2877` and the same reject-set test.

A bare trailing dot (`5.`, `-0.`, `007.`, `12345678901234567.`) is read as the number the `-p json` reader reads
([#6063](https://github.com/rust-works/succinctly/issues/6063)).
`printf '{"n":5.}' | yq -p json -o json -I0 .` is `{"n":5}` in real yq v4.53.3, and `--argjson n 5.` answers `5` too, where it was refused before.
A whole-valued span is the integer, so it prints `5` and not `5.0` (`12345678901234567.` is `12345678901234568`, as in yq; one past `i64` stays a float, `1.2345678901234568e+20`).
A dot with no integer part (`.`, `-.`) or a second dot (`1.5.`, `1..`) is still refused.
Pinned in `tests/yq_cli/i6063.rs` against the `-p json` reader on the same nine spellings.

The swap also brings the stray-comma rejection that materializer already carries
(`--argjson x '[1,]'`, `'{,}'`, #2262/#2781) to this flag -- previously `serde_json` was the
only thing refusing them, so this is the same guarantee from a different place, not a new
one.

### Unquoted field question marks (#3356)

Pinned yq v4.53.3 treats the final adjacent `?` in an unquoted field as optional
navigation and earlier `?` bytes as part of the field pattern. Thus `.x?` reads
`x` optionally, `.x??` reads `x?` optionally, and `.x???` reads `x??` optionally.
For `x: 1`, `.x??` yields `null`, rather than `1`; for a mapping with an `x?`
key, it yields that key's value. `src/jq/parser.rs` now keeps those field names
intact in yq mode, including hyphens directly after `?` (`.x?-y?` reads
`x?-y` optionally); jq's repeated optional parsing is unchanged. The pinned
`field_question_*_3356` goldens and CLI test capture the distinction.

**Resolved (#3370):** adjacency also decides whether `?//` belongs to the
field pattern. Pinned yq parses `.x?//1` as a lookup using `x?//1` and
`.x??//1` using `x??//1`; a space or newline before the question mark, as in
`.x ?// 1` or `.x ??// 1`, is a lexer error. The same unquoted-field token
rule includes `/` without a question mark (`.x//1` reads key `x//1`). The
parser and CLI tests pin these spellings while jq's pattern-alternative
grammar remains unchanged. Bracket navigation similarly requires an adjacent
optional suffix: `.[0]?` is valid, while `.[0] ?` is a yq lexer error.

**Resolved ([#3374](https://github.com/rust-works/succinctly/issues/3374)):** yq
also interprets `*` and `?` inside a mapping key as wildcards and emits the value
of every matching key in document order (`.["x??"]` on `{"x??":3,"x?y":4,"xyz":5}`
is `3`, `4`, `5`). When no key matches, a read answers `null` and a write creates
the pattern as a literal key. In yq mode the parser desugars a wildcard `.name` or
`.["name"]` into an index over `Expr::yq_wildcard_keys`, so reads, `=`, `|=`, `+=`
and `del()` all follow the matching keys. **Resolved ([#2800](https://github.com/rust-works/succinctly/issues/2800)):** an
*unquoted* `*` is part of the name too (`.ab*`, `.*`, `.*c`, `.a**`, `.a*b`), as is a leading digit
(`.1`), because yq's unquoted field is any run of name bytes; so `.a*2` is the key `a*2`
and not a product (`.a * 2` is one), and `.a*=2` is `.a* = 2`. A pattern matches a key by its text,
so `*` also finds `1: y`, `true: t` and `1.5: w` (the generator spells a matching key with
`tostring` and the ordinary string lookup finds it), and `has(k)` on a mapping compares the key
text for any scalar `k` (`has(1)`, `has(true)`, `has(1.5)`); a leading-zero literal compares by
its resolved text (`has(01)` is true here, false in yq, the recorded `01` residual). One gap
remains: a *computed* string key (`.[ "x?" + "?" ]`) is still an exact lookup, because it is not
a literal in the filter text. Once a name holds a `*` the rest of the run stays in it, as in yq (`.a*+=3` writes the key `a*+`).
**Resolved ([#4237](https://github.com/rust-works/succinctly/issues/4237), the rest of
[#4079](https://github.com/rust-works/succinctly/issues/4079)):** the same holds for a plain name:
yq's `PathElement` token is `\.[^ ;\}\{\:\[\],\|\.\[\(\)=\n!]+\??`, so every operator byte
straight after a `.` or a name stays in it, and `.a+1`, `.a%1`, `.a<1`, `.a>1`, `.a@1`, `.>1` and
`.+1` are the keys `a+1`, ... where `succinctly yq` read operations before. A space ends the name
(`.a + 1`, `.a >1` and `. > 1` still compare), `=` and `!` are never in one (`.a==1`, `.a!=1` compare,
`.a>=1` is the assignment `.a> = 1`, and `.a+=1` writes a key `a+` instead of incrementing `a`), and
`succinctly jq` is unchanged. `.> 1` is a lexer error in yq
and a parse error here (the message differs).

**Resolved ([#6699](https://github.com/rust-works/succinctly/issues/6699)):** the quoted name of a dotted step is that token too.
It takes the opening quote and ends at the first space, so `."a b"` is `."a` followed by `b"`, which no rule takes: `lexer: invalid input text "b\""` in yq, and the same refusal here (the message follows yq's wording).
The refusal is for U+0020 alone, and holds wherever the step is read: a read, `?`, `//`, a write target, `del()`, after another step, in a pipe (`."p q".r`).
Every other character is inside the token, a tab and a newline included (`."a\tb"` is a key with a backslash and a `t`), and a name with no space is read as before.
`.["a b"]` is the spelling both accept for a key with a space.
An empty name is not unquoted: `.""` is the key made of two quote characters, so it reads `null` over a document whose key is `""`, and `.x."" = 9` creates that key.
`succinctly jq` is unchanged: jq reads `."a b"` as a string index and `.""` as the key `""`.
A quoted name with no closing quote (`."a`) is still a parse error here, where yq's token is the key `"a` and answers `null`; that is the same token rule, left open.
Pinned in `tests/yq_cli/i6699.rs` and `tests/jq_cli/i6699.rs`.

A wildcard key costs one pass over the mapping's keys plus one keyed lookup per match, and
a keyed lookup into a document object is linear in its key count ([#3913](https://github.com/rust-works/succinctly/issues/3913)),
so a pattern matching *m* keys of an *n*-key mapping is O(m·n): `.["k1*"]` over 8,000 keys
matching 1,111 of them takes about 5.6 s in a debug build. A pattern matching a handful of
keys, or a document of many small records, stays linear.

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

### Unbound `$variable` is no output in yq mode (#2981, #2776, #2767, #3976)

yq has no compile-time variable check: a `$name` no binding reaches is a zero-output generator,
so `$nope` prints nothing (exit 0), `[1, $nope]` is `[1]` and the empty-operand rules apply as for
any other empty operand (`1 + $nope` is `1`, `$nope // 5` is `5`). `succinctly yq` matches that
through `EvalSemantics::UNBOUND_VARIABLE_YIELDS_NOTHING`; `succinctly jq` keeps jq's compile error
(`$nope is not defined`, exit 3). Two spellings fall out of the same rule: `$__loc__` is no builtin
in yq (it is an unbound variable, so it prints nothing here too), and `$x-1` is the variable named
`x-1` (the hyphen extension of #1890), not a subtraction.

An empty operand that binds or indexes is not "no operator" in yq either, and `succinctly yq`
matches (#4139; captured against yq v4.53.3, on any empty operand such as `select(false)`):

- `A as $v | B` runs `B` once, with `$v` unset, when `A` has no output (`select(false) as $z | .a`
  is `.a`, and `$nope as $y | 3` is `3`). jq runs `B` per output of `A`, so nothing.
- An index operand with no output stands for *all children*: `.[$nope]` on `[1]` is `1`, on `a: 1 /
  b: [1,2,3]` `.b[$typo]` prints every element, `.[$typo] = 1` sets every member, `.b[$typo] += 1`
  is `b: [2,3,4]`, and `del(.[$typo])` is `{}`. A typo'd name used as a computed index or key
  therefore rewrites or deletes data, as it does in yq. The children of a scalar are none, so
  `.b[$typo] = 1` over `b: 3` stays a no-op. This holds for a write or `del` through an `as` with
  no source too (`(select(false) as $x | .b) = 9` sets `b`).
- `has(E)` with no output is `false`.

jq mode keeps its generator model for all three (no key, no output).

Still diverging: over a `null` or absent operand yq's empty index creates the container
(`.n[$typo] = 1` over `n: null` is `n: []`, as plain `.n[] = 1` is, which `succinctly yq` already
does), where this tool leaves the document unchanged; a slice bound with no output is an error in yq (`.[$nope:]` is `expected to find 1
number, got 0 instead`) and nothing here (#4197); `1 as $y | select(false) as $y | $y` is `1` in yq
and nothing here, because the outer `$y` is not substituted into a body that rebinds the name;
`$ENV` is an unbound variable in yq, so `$ENV.x` prints nothing, where it is the environment object
here (#3029), and a `--arg __loc__` is not honoured in yq mode.

### `..` stops at an alias node — resolved for reads ([#4214](https://github.com/rust-works/succinctly/issues/4214)); the write path, owned routes and merge keys are residual ([#4223](https://github.com/rust-works/succinctly/issues/4223))

yq's recursive descent yields an alias node and does not descend into it, so on
`anchors: {base: &b {p: 1, q: 2}, copy: *b}` `[..] | length` is 6 and `.. | path` never lists
`anchors.copy.p`. `succinctly yq` matches that on the cursor-preserving read routes (`..`,
`.. | path`, `.. | key`, `.. | parent`, `.anchors.copy | ..`); `.anchors.copy[]` still iterates the
aliased mapping, as in yq. `recurse` and `recurse(.[]?)` are `..` (#3719) and stop with it; yq has
no `recurse`, so under `--jq-extensions` they follow the `..` rule rather than a reference.
So does `recurse(.[]?; cond)` for a `cond` that yields at most one value (#4095), and a `key`, `path` or `parent` stage after it reads each node's position.
Its remaining divergences are recorded under `recurse(f; cond)` in [the jq limitations](../jq/limitations.md): `path(recurse(.[]?; cond))`, a write through it, and a `cond` outside the admitted grammar (one that reads `key`, `path`, `tag` or `line`, is passed through a `def` parameter, or uses a builtin the allow-list does not know) still expand aliases and give no position.

An assignment after `..` in a pipe (`.. | select(tag == "!!int") |= . + 1`) writes every node the list selects before the first node is printed, as yq's pipe does ([#6860](https://github.com/rust-works/succinctly/issues/6860)).
It is read here as one update per node of the list over that node's own subtree, so it covers one `..` with navigation before it and `select`s after it, and `|=`, or `=` and the compound forms with a literal right side.
An assignment after another fan-out (`.[]`, a union, `map`) or with a right side that reads the node (`.. | select(...) = .a`) still runs per node, which differs from yq only where the targets of different nodes overlap.

A write through `..` reaches an anchored node once too (#4223): `(.. | select(tag=="!!int")) |= . + 1`
is `p: 2, q: 3` in yq and was `p: 3, q: 4` here, the shared node incremented twice. The paths a `..`
resolves below an alias position that is still its anchor's copy are not targets
(`alias_identity::drop_below_alias`); explicit paths through an alias are, as before (yq's rule 6).

Still diverging: a `..` write behind a navigation stage
(`.y | (.. | select(tag == "!!int")) |= . + 5`) still goes into an alias node inside the navigated
subtree, since the alias table names positions from the document root; `..` over an owned value
(`to_entries | .[] | ..` expands the alias); a merge key (`<<: *b`, which yq also treats as the
stop, listing `m.<<` where this lists the merged `m.p`/`m.q`, and which it writes through to the
anchor: `.m.p |= . + 1` also changes `base.p` there); and `recurse(f; cond)` as a write target
(`(recurse(.[]?; true) | select(tag == "!!int")) |= . + 1` reaches a shared node once per alias), as `path(recurse(.[]?; cond))`, or with a `cond` the cursor walk does not take.

### An empty input ([#4099](https://github.com/rust-works/succinctly/issues/4099))

An input with no bytes at all reads no document in yq, and a run that read none evaluates once against one
empty document (`printf '' | yq '.a'` is `null`, `.a = 1` is `a: 1`; `yq '.a' empty.yaml one.yaml` prints
`1`; `yq '.a' e1.yaml e2.yaml`, both empty, prints `null` once with `file_index` 0). `succinctly yq` reads that
document as an input of one line break, so it answers like yq wherever a line-break document
does, and differs where one does not: `[.]`, `{"a": .}` and `. as $x | [$x]` print `- null` / `a: null` where yq
prints a bare `-` / `a:`, `line` is `1` where yq has `0`, and `@yaml` is `null` where yq's is empty.
`filename` is empty over it, as in yq ([#5721](https://github.com/rust-works/succinctly/issues/5721)).
A result that is a document with no content (`# c`, an empty input) is written once, with no blank line after it, by `select`, `(.)` and `..` as by `.`.
Under `-0` the NUL replaces the line break the header ends in (`# c<NUL>`, and a lone NUL over an empty input), as in yq; with `--colors` a comment's span keeps its break, as yq's does.
`--eval-all` over an empty input still differs: `select(true)` prints `null` for a blank line, and `.a, filename` writes a `---` between its results.
Not covered: `-` cannot be known to be empty before it is read, so an
empty stdin mixed with empty files reads nothing at all; and `-i` over several files edits the file each
document came from, where yq writes the result into the first file. A file holding only comments, `---` or a line
break is a document and is read as one. A write to a comment-only file prints the write without the comment,
where yq keeps it above (`# c` / `a: 1`).

An empty or whitespace-only JSON input is the same document ([#5478](https://github.com/rust-works/succinctly/issues/5478)), not a JSON `null`.
Space, tab, line feed and carriage return are whitespace to yq's JSON reader, so `printf '  \n' | yq -p=json .` prints a blank line and `[.]` prints `-`.
Only a JSON output spells it `null`, and `-o auto` over a `.json` file is one.
A whitespace-only JSON file among others reads no document, as an empty one does, and takes no `file_index`.
A form feed, a vertical tab, a no-break space, a byte-order mark or a comment is not whitespace there, and is refused as before.
A NUL byte ends a JSON input: what precedes it is read and what follows is not, so a lone NUL is the empty document and `1<NUL>2` is `1` ([#5721](https://github.com/rust-works/succinctly/issues/5721)).
An explicit `-p=json` makes `-o auto` YAML, as in yq, where `-o auto` over a `.json` file without `-p` is JSON.
Not matched: `--eval-all` over such an input prints `null` where yq prints a blank line, and `- null` where yq prints `-` for `[.]`.
The owned-value route it takes for a JSON input has no spelling for an empty node, as over an empty YAML input with a filter beyond a bare navigation (`ea '[.]'`).

### A write that changes the kind of a document root keeps its comments ([#4997](https://github.com/rust-works/succinctly/issues/4997))

`# c` / `null` (or `~`, or a file of comments alone) written `.a = 1` is `# c` / `a: 1`, and a scalar or
container root rewritten to the other kind keeps the document's head comment and its foot, as in yq. Not
matched: the head comment of a *non-first* document (`x: 1` / `---` / `# c` / `5` written `. = [1]` loses `# c`, which
yq keeps), and a root with an explicit tag (`# c` / `!!null` written `.a = 1` prints a blank line after the comment).

### A `del` that renumbers an array, then another write into it: three shapes still keep the wrong comments ([#2512](https://github.com/rust-works/succinctly/issues/2512))

A pipe that deletes an array slot and writes into the same array by a static index is resolved stage by stage.
Each element keeps its own comment and style, as in yq: `del(.arr[0]) | del(.arr[0])`, `.arr[2] = "Z" | del(.arr[0])` and `del(.arr[0]) | .arr[1].x = 9` all match.
Three shapes are not resolved, and print the comment of the slot an element moved into rather than its own.
A negative index into the array (`del(.arr[0]) | del(.arr[-1])`), because which slot it names depends on a length the walk does not know.
A write at the end of the array next to a `del` (`.arr[5] = "Z" | del(.arr[0])`, and a write further out that pads it with `null`), because the array that comes out is longer than the survivors the walk predicts.
A single `del` whose comma branches renumber each other's array (`del(.arr[0], .arr[1].x)`), because its branches are read together against the array as it was.
Values are right in all three; only the comments and styles of the elements differ.
`tests/yq_cli/i2512.rs` pins each with yq's own answer.

### A block scalar header's comment ([#5001](https://github.com/rust-works/succinctly/issues/5001))

A comment after a block scalar's `|`/`>` indicator stays on the header line in the places the value is
written where it stands: `.`, `-P`, and a write that leaves the scalar alone or replaces it.
A plain copy (`.b = .a`), `pick` and `omit` keep it since [#5257](https://github.com/rust-works/succinctly/issues/5257), [#5003](https://github.com/rust-works/succinctly/issues/5003) and [#6906](https://github.com/rust-works/succinctly/issues/6906): `omit(["a"])` prints the rest of the document with its comments, flow style and quoting, as `pick` does.
A key of the wrong kind for the node (`omit(["a"])` over a sequence, `omit([0])` over a mapping with string keys) matches nothing and the node comes through whole, as in yq.
Only an `omit` that runs on the document itself (the whole filter, or the first stage of its pipe) keeps the presentation: `.b | omit(["x"])` and a document over 1 MiB print the plain block YAML, as before.
Still divergent: a head comment after a `---` marker goes with the first key when that key is omitted (`---` / `# head` / `a: 1` / `b: 2` with `omit(["a"])`), where yq keeps it with the document; `pick` does the same.
Still divergent: yq reads an integer key as the text of a mapping key (`1: a` / `2: b` with `omit([1])` is `2: b`), which `omit` and `pick` here match only for a string key.
A copy made through a variable (`. as $v | {"z": $v}`), a recursive merge over it (`. *d {"a": "n"}`) and a slice (`.[0:1]`) carry no such comment, where yq keeps it ([#5003](https://github.com/rust-works/succinctly/issues/5003)).

`pick` copies the document's nodes, so what it keeps is printed as written: trailing comments, flow style, quoting, anchors, tags, a key's own quoting and comment, and the document's head comment.
`pick` takes the keys of a mapping or the indices of a sequence, in the order listed.
Swept against yq v4.53.3 over twelve documents (comments, flow collections, anchors and aliases, tags, explicit keys, merge keys, sequences, several documents) and the key lists a mapping or sequence takes, plain and under `-P`.
Still different:

- **A document that opens with `--- # comment`** and has an entry picked: yq prints the comment fused to the entry (`# firsta: 1 # one`), which it cannot read back.
  This build prints the entry alone.
- **A document opened by `---` over a comment line** (`---` / `# h` / `a: 1` / `b: ...`) with a later entry picked: yq keeps `# h` under the marker, this build prints the marker alone.
- **A picked alias whose anchor is not picked** (`pick(["d"])` over `c: &k {z: 1}` / `d: *k`) prints the resolved mapping here and `d: *k` in yq, which then cannot resolve the alias (the anchor soundness rule).
- **A picked mapping that holds a merge key** (`c:` / `  <<: *x`) prints the merged members here and `!!merge <<: *x` in yq ([#4954](https://github.com/rust-works/succinctly/issues/4954)).
- **A key listed twice** (`pick(["a", "a"])`) prints the entry once here and twice in yq, which a mapping cannot hold.
- **A `pick` that is not the whole filter** (`.. | pick(...)`, `.c = pick(...)`, `{"z": pick(...)}`, `.[] | pick(...)`) and a pick over a computed value carry no presentation: the replay reads the document, not a computed value.
- **A document past 1 MiB**: the replay converts the whole document with its comments, where the plain route reads only what is picked (2.7x the time and 3x the memory on a 10 MB document), so a larger one prints the result without the kept nodes' comments and styles, as before.

### `line` and `column` residuals ([#4363](https://github.com/rust-works/succinctly/issues/4363))

A node that carries an `&anchor` or `!tag` reports the position of its first property token, as in
yq ([#2712](https://github.com/rust-works/succinctly/issues/2712)): `b: &anc [1,2]` is column 4.
yq numbers `line` from the first *content* line (#4363): its header preprocessing slurps the leading
blank, whitespace-only, comment, `%` directive and first `---` lines off the input and parses the rest,
in every document of the stream. `# c` / `k: 1` / `m: 2` has `.m` on line 2 and the root on `[1,1]`, as
does the same text opened by `---`, `%YAML 1.2` / `---`, or a blank line; a later document counts from the
same origin (`---` / `k: 1` / `---` / `b: 2` / `c: 3` has `.c` on line 4, not 5). A first content line
that is `--- 42` also loses the marker and the blanks after it from its columns (`--- [1, 2]` has `[` on
`[1,1]`). `yq-locate`, `at_offset` and the library's `YamlCursor::line` keep the editor line; only the
`line`/`column` builtins renumber (`yq_position_preamble`).

Still different (#4548): `--- # c` / `&b [1]` is `[0,0]` in yq (the comment on the marker line), `[2,1]` here; a
second `---` straight after the first (`---` / `--- 5`) is `[1,1]` in yq and `[1,5]` here; and a document with
lone-CR line breaks is `[0,0]` in yq. Only a `%YA..` directive is slurped, as in the printed header: `%TAG` is content.
The renumbering lives in the `DocumentCursor` impl that only the `line`/`column` builtins call; `succinctly jq` never holds
a `YamlCursor`.

`column` counts characters, as yq does ([#4364](https://github.com/rust-works/succinctly/issues/4364)):
`é: [1]` is `[1,4]`, where it used to read `[1,5]` (the byte column). A character is a Unicode scalar
value, so `e` plus a combining accent is two columns and an astral character one (checked against yq
v4.53.3, which counts runes, not grapheme clusters); bytes that are not valid UTF-8 count one each.
`YamlIndex::to_line_char_column` does it, once the text is known not to be ASCII (an ASCII document
reads the byte column, with no scan of the line); `YamlIndex::to_line_column`, `to_offset` and
`yq-locate` stay in bytes, which an editor offset round-trips. The JSON cursor's `column`
(a succinctly extension with no jq oracle) stays in bytes.

`line` and `column` of a node read from **JSON input** are `0`, as in yq
([#4382](https://github.com/rust-works/succinctly/issues/4382)): yq decodes JSON without keeping
positions, so `echo '{"a":1}' | yq -p json '.a | [line,column]'` and the same over a `.json` file are
`[0,0]`, where the text read as YAML (no `-p json`, or a `.yaml` file) keeps `[1,6]`. A YAML index marked
JSON-sourced (`YamlIndex::mark_json_sourced`) answers `0` from `YamlCursor::line`/`column`. The decoder is
picked once per run, from the *first* file's extension, for every file, as in yq (`yq '.a | line' t.json t.yaml` is
`[0,0]` twice, `t.yaml t.json` is `[1,6]` twice; an explicit `-p` is not looked at per file;
[#4394](https://github.com/rust-works/succinctly/issues/4394)). `--eval-all` answers `[0,0]` for a JSON input too
(#4544).

`--eval-all` evaluates each document through a reindexed copy of its value, so the cursor a node stands on indexes
the regenerated text, not the file ([#4646](https://github.com/rust-works/succinctly/issues/4646)). For YAML input
`line` and `column` are therefore asked of the source instead: the node's path from its document's root is followed
again on the document's own index (`yq_runner/source_positions.rs`, installed around the run when the filter reads
`line` or `column`), so a merge key, an alias, an integer key and a mapping key (`.l | key | line`) answer as they do
on the streaming route, and a stream's second document counts from its own origin. A node with no counterpart in the
source answers `[0,0]`: a value the filter rebuilt (sliced, mapped, sorted, constructed or assigned, `.a[1:] | .[0] |
line`), which the streaming route answers `0` for as well, except that it still reads a plain collect (`[.a[]] | .[1] |
line` is `3` there, `0` under `--eval-all`). yq keeps the position through all of these. Still different: yq numbers the
lines of a file that **opens with a comment** one too high when it is not the first file of the run (`# c` / `k: 1` as
the second file reports `.k` on line 2 where the same file alone reports 1), which is not reproduced.

### Comment after a sequence item's dash on collecting routes ([#4256](https://github.com/rust-works/succinctly/issues/4256))

A sequence item printed by itself keeps the comment after its bare dash when the item is a container:
`a:` / `  - # own` / `    c: 2` with `.a[1]`, `.a[-1]`, `.a | .[1]`, `.a[]` or `.a[] | select(.)` prints `# own` /
`c: 2`, as yq does. A scalar item's comment is dropped, as in yq. The two routes that collect items first
(`[.a[]]`) or slice (`.a[1:]`) still print the container without that comment; yq keeps it there. So does
any route that binds or wraps the item before the final print, since the comment is recovered only when a
navigated item is printed: `.a[1] as $v | $v`, `.a[1] | [.]`, `.a[1] | {"k": .}`, `.a | map(select(.c))` and the
first result of `.a[1] | ..`. A block container's `&anchor`/`!tag` line prints above the comment and a flow
container's below it, as in yq. An alias item (`- # own` / `  *x`) prints its target after the comment, where yq
prints `*x` (the pre-existing root-alias expansion, #1350). Separately, `.a` on a sequence whose dash-comment item
carries an `&anchor` drops that anchor (`- &x` is printed as `-`), which predates this change.

### `-P` and the trailing comment of a flow collection ([#4377](https://github.com/rust-works/succinctly/issues/4377))

`-P` rewrites a flow collection as block, and the comment after its closing bracket (`a: [1, 2] # ec`) then has
no line of its own. yq's encoder holds it and writes it at the end of the next line that carries no comment: the
next sibling's key line when that is a collection (`b: # ec`), its value line otherwise (`b: 1 # ec`), the dash
line of a collection item (`- # ec` / `  x: 1`); and it is lost when nothing follows in the document, so
`yq -P '.a'` on `a: [1, 2] # ec` prints no comment. A scalar that has a comment of its own keeps its line and the
held comment waits for the next one, a comment on a collection's key overwrites it, an alias is skipped, and an
empty `[]`/`{}` stays flow with its comment.
Between the elements of a flow sequence (#6334) the held comment is overwritten by an element that has a comment of its own, and an alias takes it on its line. This build follows that for the DOM route that `-P`, a write and
`--arg` take, on JSON-shaped and YAML input alike (`defer_flow_line_comments`). Swept against yq v4.53.3 over 14
documents x 8 filters. Still different:

- **A comment written inside a flow collection** is kept when it follows an element on its line (`[1, # note` / `  2]`, #6334) and when it stands on a line of its own, and is printed as yq prints it.
  The identity route, a write and a navigation lay the collection out across lines with the comment after the element's comma, and the closing bracket on a line of its own when the last element is commented.
  A comment on a line of its own is the head of the element after it (`.v[0] | head_comment`), or the foot of the last element when the closing bracket follows it; for a mapping, of the entry's key.
  The head sits on a line of its own before the element, and a foot follows a trailing comma, except after a sequence element that is a sequence (no comma) or a mapping (the comma goes inside its `}`).
  `-P`, which writes the collection as a block, keeps the head, the foot and a comment on a scalar element's line, and holds the comment of an element that is a collection for the next line without a comment, as it does one after the whole collection (an element with a comment of its own overwrites the held one).
  Still different, each seen against yq v4.53.3 over generated documents:
  A comment on a line of its own before a comma that is followed by the closing bracket (`[a, b` / `# f` / `,` / `]`) is dropped here; yq moves it, and the comments after that comma, past the collection to whatever follows (the next node, or the document's foot).
  yq writes an empty line before the first comment of a collection nested in a sequence after a sibling mapping that ends with a foot comment (`{k: v` / `# c` / `}, [` / `# d`), and before the foot of a collection that ends a block sequence under `-P`; some of these this build does not write.
  Under `-P` the foot of a last element that is itself a collection is written below the sequence when the sequence has an element after it in an outer sequence, or a key after it in a mapping (yq: `w: # c` / `# f` / `  - 1`), where this build writes it right below the element.
  A comment held by an entry of a flow mapping that is an item of a block sequence, and left waiting at the end of the mapping by an entry with a comment of its own after it (`- {k0: {x: 1}, # c5` / `m1: b # c6` / `}` / `- 2`), is written on the next item's line here (`- 2 # c5`); yq drops it.
  A comment on the line of a comma that starts its own line, after own-line comments (`x` / `# c3` / `, # c4` / `~`), is the next scalar's line comment in yq (`~, # c4`) and is dropped here.
  A comment on a line of its own between a flow mapping key's colon and its value is held like any other (the next entry's head, or the value's first element's), and one on the colon's line is dropped, as in yq for a collection value; for a scalar value yq keeps it as the value's line comment.
  A comment after a single pair written in a flow sequence (`[a: 1, # c`), after an empty value (`{a: , # c`) or on a key is dropped, and so is a second comment on the line of a comma (`[a # c1` / `, # c2`) when the first stands on a line of its own.
  With `-I4`, the closing bracket of a commented collection nested in a `- [` item is at column 0 where yq puts it at the item's column.
- **An empty `{}`/`[]` value that follows a held comment**: yq writes `b: # ec` and the `{}` on the next line;
  this build writes `b: {} # ec`.
- **A collection built by an expression** (`[.]`, `{"k": .a}`, `to_entries`) carries no comment tree at all
  (#1361), so no comment is held or printed there.

### A write that changes a node's kind keeps its same-line comment only beside a value that fits on the line ([#5258](https://github.com/rust-works/succinctly/issues/5258))

A node's own trailing comment (`b: [1, 2] # t`, `b: 1 # t`, `b: {} # t`) stays beside its new value when a write changes the node's kind and the value still fits on that line.
That is a scalar or an empty `{}` / `[]`: `.b = 5` over `b: [1, 2] # t` is `b: 5 # t`, and `.b = {}` over `b: 1 # t` is `b: {} # t`, as in yq.
A document root follows the same rule except that a root scalar keeps no comment (`1 # t` written `. = 5` is `5`), as before.
`-P` keeps an empty collection flow, and the comment with it.
Swept against yq v4.53.3: four container sources (flow sequence, flow mapping, empty `[]`, empty `{}`) against ten writes (a number, `null`, a quoted string, a string that needs quoting, a multi-line string that becomes `|- # t`, a float, a boolean, `length`, `tostring`, a copy of another node), five sources including a scalar against `{}` and `[]`, the document root, a node with an anchor, an alias or a tag, and a last key.
Still different:

- **A non-empty block container written over the node** (`.b = {"y": 2}`, `.b = [9]`).
  The comment has no line of its own once the value is a block, so yq's encoder holds it and writes it at the end of the next line that carries no comment (`c: 3 # t`), the mechanism [#4377](https://github.com/rust-works/succinctly/issues/4377) describes for `-P`, and drops it when nothing follows.
  This build drops it in every case: it would otherwise print a comment beside an entry it never described.
- **A block mapping whose key line carries the comment** (`b: # t` / `  x: 1`) overwritten with an empty `{}` / `[]`.
  yq writes `b: # t` and the `{}` on the next line, which it cannot read back as the same document; this build writes `b: {} # t`.
- **An explicit-key entry** (`? [1] # t` / `: 2`) written through `.[] = 5` prints `'': 5` here and `? [1] # t` / `: 5` in yq: the complex key is spelled `""` ([#3992](https://github.com/rust-works/succinctly/issues/3992)), a different mechanism from the comment.
- **The same block mapping overwritten with `null`** prints `b: null # t` in yq and `b: # t` here (the key comment is kept and the value is the empty scalar).

### `kind` of an alias on the owned routes ([#4866](https://github.com/rust-works/succinctly/issues/4866))

`kind` asks the YAML cursor, so a `*name` reference is `alias` (`.b | kind`, `[.[] | kind]`, `map(kind)`,
`.. | kind`), and a collection tagged `!!null` is `scalar`, as in yq. The routes that hold an owned copy of the
value have no alias left to name and answer the target's kind: `to_entries | map(.value | kind)`, `kind` inside an
assignment's right-hand side (`.z = (.b | kind)`, `.[] |= kind`), a value built from the alias
(`{"k": .b} | .k | kind`), and `--eval-all`/`-s`/`-i`. Separately, `..` still descends
into a `!!null`-tagged collection, where yq stops at the `scalar` (`[.. | kind]` has one more element).

## `@yaml` of a container the filter builds, a multi-line string, or a root computed float ([#1183](https://github.com/rust-works/succinctly/issues/1183))

`@yaml` of a value runs the same owned-value YAML streamer as the CLI's own output, so it inherits that
streamer's known gaps; each is pinned in `recorded_gaps_1183` (`tests/yq_cli_tests.rs`).
`@yaml`, `to_yaml` and `tostring` of a container that is a document node are the exception since [#6901](https://github.com/rust-works/succinctly/issues/6901): they encode the node as it was written, with the writer the CLI prints that node with.
A flow collection stays flow, a nested one inside a block node too, a trailing comment stays, a sequence item keeps its own head and foot comment lines, and an alias node is `*name`.
That is pinned by `tests/yq_cli/i6901.rs`.
What is left is a value with no node to read: `[.a] | to_yaml` over `a: {b: 1}` is `- b: 1` here and `- {b: 1}` in yq, and so is a node behind a write (`.z = 1 | .a | to_yaml`), one read back out of a constructor (`{"k": .a} | .k | to_yaml`), and one behind `with_entries`, `sort`, `reverse`, `unique`, `pick` or an array `+`, which yq reorders or copies in place and so leaves in its source style.
Captured live (v4.53.3):

| filter                           | real yq           | succinctly yq | why                                                                                                                     |
|----------------------------------|-------------------|---------------|-------------------------------------------------------------------------------------------------------------------------|
| `a: {b: 1}` then `[.a] \| @yaml` | `- {b: 1}\n`      | `- b: 1\n`    | an owned value carries no source style (#739, #865)                                                                     |
| `"x\ny" \| @yaml`                | `\|-\n  x\n  y\n` | `"x\ny"\n`    | a line break is double-quoted, not a literal block (the computed-value streamer; #3587 closed the JSON route)           |
| `2.0*1 \| @yaml`                 | `!!float 2\n`     | `2\n`         | yq tags a computed float but not a document-sourced `!!float 2` (the #1090 test); an owned float cannot tell them apart |

## A write that selects by node metadata, only where the document is the write's input ([#5479](https://github.com/rust-works/succinctly/issues/5479))

A write resolves its left side against the owned document, which holds values and nothing else, so `(.. | select(style == "double")) = 1` selected nothing and was a silent no-op.
The left side is now resolved on the document's cursor first, and the write runs on the literal paths it found.
That covers `style`, `anchor`, `kind` (an alias), `tag` against a scalar tag in a document that has an explicit tag ([#6895](https://github.com/rust-works/succinctly/issues/6895): a custom tag such as `!Ref` is the node's tag on the cursor and `!!str` on the owned value; a comparison with `!!map` or `!!seq` is not resolved on the cursor, because literal paths cannot take targets that nest, so a custom-tagged collection is still selected as a plain one), `line`, `column`, `line_comment`, `head_comment`, `foot_comment`, `document_index`, `file_index` and `filename` read anywhere in the left side of `=`, `|=`, `+=`-style updates, a metadata write (`style =`, `anchor =`, ...) or a `del(...)`, and of succinctly's own `//=` (yq has none).
A metadata builtin that is the last stage of the left side (`(.a | key | line_comment) = "x"`) is the slot being written, a different route, and is unchanged, so a `select(...)` on metadata before that last stage (`(.. | select(style == "double") | line_comment) = "x"`) still selects nothing.
A metadata read hidden in a user-defined function (`def dq: select(style == "double");`, `--jq-extensions`) is not seen by the check and still selects nothing.
The write has to be the whole filter, or the first stage of a pipe: a later stage reads the document the stages before it left, which has no cursor.
`.e = 1 | (.[] | select(style == "double")) = 5` therefore still selects nothing here where yq writes `5`, and so does a second metadata-selecting write in the same pipe, and one inside a comma, `with(...)`, `reduce` or a function body.
So does metadata read in the right side of `|=` (`.[] |= (select(style == "double") | 5)`).
When the paths cannot be resolved on the cursor (the left side errors, or a path step is neither a string key nor an integer index) the write runs as written, which is the same silent no-op.
`--eval-all` and `--slurp` evaluate through the owned-value route and are unchanged.
A write that clears an anchor leaves yq's dangling `*name` alias behind, where succinctly expands it (the anchor soundness rule above), exactly as `.a anchor = ""` already does.
Pinned by `tests/yq_cli/i5479.rs`, whose rows were captured from yq v4.53.3.

## `explode(f)` expands where yq does, and writes every alias it can read back ([#5483](https://github.com/rust-works/succinctly/issues/5483))

yq's `explode(f)` expands every alias in the nodes `f` selects, and in their descendants, into a copy of its anchor's node, drops the anchors there and expands `<<` merges there.
An alias or anchor outside those nodes stays.
`explode(f)` parses here as `(f | ..) anchor = ""`, and `explode(.)` and `explode(..)` as `(..) anchor = ""`.
Clearing the anchor of every selected node makes the aliases that named one print as the nodes they stood for (`enforce_anchor_soundness`), an alias node that is selected clears its own mark and prints as its value, and a write expands merges.
Rows over a selected alias, a selected nested alias whose anchor is outside the selection, several selected nodes, a selection that holds nothing and a merge under the selection match yq v4.53.3.
jq's own `explode` is the nullary string builtin and is unchanged; the call form only exists in yq mode.
Four things differ from yq.
An alias *outside* the selection that named an anchor the selection cleared is written as its value (`explode(.a)` over `a: &x {p: 1}` / `b: *x` prints `b:` / `  p: 1`), where yq leaves `b: *x` with no `&x` anywhere, a document it then refuses to read ([ADR-0018](../../adrs/adr-0018.md) rule 4: it would emit output it cannot itself read back).
A write expands every `<<` merge in the document, not only those under the selection, so `explode(.n)` also turns an unrelated `m: {<<: *x, q: 2}` into its keys where yq keeps `!!merge <<: *x`; every write does the same, and the family is listed above.
An argument that is no path (`explode(1)`, `explode("a")`) is an error (`Invalid path expression with result 1`) where yq explodes a node that is in no document and answers the input unchanged.
A write after `explode` in the same program still follows an alias that `explode` expanded (`explode(.) | .a = 3` over `a: &x {k: 1}` / `b: *x` writes `b` too, where yq's `b` is by then a copy of its own): the alias groups the write follows come from the source document, not from the marks `explode` cleared.
`.a | explode(.)` is refused by the existing rule that a metadata write cannot sit behind a navigation.
The presentation of an expanded alias copy is lost: its flow style (`b: *x` over `a: &x {p: 1}` prints a block mapping where yq prints `b: {p: 1}`), its quotes (`b: *s` over `a: &s "q"` prints `b: q`) and its explicit tag (`a: &x !!str 1` prints `b: "1"`), the family tracked on [#2651](https://github.com/rust-works/succinctly/issues/2651), and a comment after the alias moves with it.
An anchor that an expanded copy carried from the node it copies is dropped from the copy, and what named it is written as a value: `explode(.b)` over `a: &x {k: &w 1, l: *w}` / `b: *x` / `c: *w` is `b: {k: &w 1, l: *w}` in yq and `b:` / `  k: 1` / `  l: 1` here, with `a` and `c: *w` as they were (`inner: *x` over `x: &x {n: &w [7]}` is `inner: {n: &w [7]}` in yq).
Inside a collect or a binding (`[explode(.b)]`, `explode(.b) as $x | $x`) the program runs on the owned value, which holds no anchor or alias mark, so an anchor outside the selection is dropped there as well (`[explode(.b)]` over `a: &x [1]` / `b: *x` prints `a:` / `  - 1` where yq keeps `&x`); the data is the same.
The value is the same each time; only how it is written differs.
The `alias` getter and the `alias =` setter have their own sections below.
Pinned by `tests/yq_cli/i5483.rs` and `tests/yq_cli/i5483_explode_path.rs`, whose rows were captured from yq v4.53.3.

## The `alias` getter answers a node's `Value`, as yq does ([#5483](https://github.com/rust-works/succinctly/issues/5483))

yq's `alias` returns the node's `Value`, not whether it is an alias: the anchor's name for an alias node (`*name`), the scalar's own text for any other scalar, and `""` for a mapping, a sequence and an empty value.
So `[.. | alias]` over `a: &x {p: 1}` / `b: *x` / `c: &y 2` is `["", "", "1", "x", "2", "y"]`, not a column of empty strings with two names.
`succinctly yq` follows that bug-for-bug (ADR-0018): the text is the scalar as written (`1.50`, `0x1F`, `~`, `.inf`, `2001-12-14`), a quoted or block scalar decoded (`"q\tx"`, `|` literals), and the alias name for an alias node.
It is yq-mode only: jq has no `alias`, and `succinctly jq alias` is `alias/0 is not defined` as in jq 1.7.1.
Rows over a plain document, a 25-spelling scalar table, `select(alias != "")` in a read, a write and a `del`, and a two-document stream match yq v4.53.3.
Where the node is an owned copy rather than a node of the document (the value inside `|=`, `with_entries(.value |= alias)`, a slice, a computed value), the evaluator holds no alias mark and no source spelling.
There `~` answers `""` where yq answers `~`, an alias node answers the text of its target where yq answers the anchor's name (`.[] |= alias` over `a: &x 5` / `b: *x` is `b: "5"`, yq `b: x`), a number answers its canonical text (`1.50` is `1.5`), and an empty value (`a:`) written through `|=` answers `"null"` where yq answers `""`.
An absent key and a computed `null` answer `"null"`, as in yq.
`-p=json` input prints a number as the source wrote it (`1.50`, `1e3`) where yq prints the canonical form (`1.5`, `1000`).
`.[] | select(alias != "")` prints the resolved target of an alias node where yq prints `*x`, the read-route alias echo tracked with the alias-mark family (#2505, #5249).
Under `--jq-extensions`, `path(...)` and `paths(...)` run with no cursor, so they cannot see an alias node (`paths(alias != "")` lists every scalar), as they cannot see an `anchor`; no yq oracle exists for that surface.
Pinned by `tests/yq_cli/i5483_alias.rs`, whose rows were captured from yq v4.53.3.

## The `alias =` setter writes `*name` only where `&name` holds the same value ([#5483](https://github.com/rust-works/succinctly/issues/5483))

yq's `PATH alias = "name"` (and `|=`) turns each node `PATH` selects into an alias node whose `Value` is `name`, and prints it `*name` whether or not an `&name` exists.
`succinctly yq` writes the same mark, then holds it to the soundness rule of every alias mark (anchor/alias section above): it prints `*name` only where an earlier `&name` holds an identical value, and otherwise prints the value the node has.
That is a divergence under the rule that `succinctly yq` never emits YAML it cannot read back, and each row below is one the rule refuses.
- No such anchor: `.b alias = "z"` over `a: &x 1` / `b: 1` is `b: *z` in yq, `b: 1` here.
- An anchor holding another value: `.c alias = "x"` over `c: 2` is `c: *x` in yq, which reads back as `1`, and `c: 2` here.
- The text of a number, a boolean or `null` is a name too (`.b alias = 5` is `b: *5` in yq), and meets the same rule.
- A path the write creates is `null`: `.d alias = "x"` is `d: *x` in yq, `d: null` here.
- Aliasing the declaring node (`.a alias = "x"` over `a: &x 1`) leaves no declaration, so every alias of it, the node itself included, prints the value.
- `-o json` prints `null` in yq for an alias node the setter made (it has no target) and the node's value here.
- A bare scalar result keeps no mark, so `.b alias = "x" | .b` is `*x` in yq and `1` here, as for any bare scalar alias (see the soundness section above); the same holds for the document root, `. alias = "x"`.
- `--eval-all` evaluates through the owned-value route, which writes no metadata: `alias =` is a no-op there, as `anchor =` is (the route keeps no anchors at all, #4380).
- A document whose anchor is aliased elsewhere keeps yq's other alias: `.a alias = "x"` over `a: &y 1` / `b: *y` is `a: *x` / `b: *y` in yq, and `a: 1` / `b: 1` here, the declaration being gone.
- A later write does not clear the mark where it leaves the value alone: `.c alias = "x" | .c = 1` is `c: 1` in yq and `c: *x` here, since `1` is still the anchor's value.

What matches yq, byte for byte, in the rows captured from v4.53.3: an equal anchor (`.b alias = "x"` over `a: &x 1` / `b: 1` is `b: *x`), `|=`, several targets, a container aliased to an equal container, and the refusals.
An empty text writes nothing and does not clear an alias already there (unlike `anchor = ""`).
A sequence or non-empty mapping right side has no text and writes nothing; `{}` and a name with whitespace, `,`, `:`, a bracket, a brace or a non-ASCII character is refused with `yaml: yaml: alias value must contain valid characters only`, the rule of an anchor name.
The right side of `.e alias = .b` is read as the value of `.b`, not as the alias node it is, so it writes nothing where yq writes `*x`.
It is yq-mode only: in jq mode `alias =` is not a write.
Pinned by `tests/yq_cli/i5483_alias_setter.rs`.

## A value the program builds is bounded by the native-stack floor, not by 384 levels ([#5876](https://github.com/rust-works/succinctly/issues/5876))

yq has no limit on the depth of a value a program builds but Go's own stack.
`succinctly yq` refused such a value past 384 levels (`MAX_VALUE_TREE_DEPTH`) at its output and in most of the filters after the fold, where yq answers.
Since #5876 `run_yq` arms the same floor `succinctly jq` has had since [#5554](https://github.com/rust-works/succinctly/issues/5554): a walk over a built value that is past 384 levels checks ADR-0025's native-stack floor once per level instead of refusing.
`.[] as $i ireduce (0; [.]) | length` over 385 and 2000 items is `1`, as in yq, and the value prints as yq prints it, as YAML (`- - - 0`), as `-o json` and under `-P`, byte for byte (`-C` keeps its own colour difference, [#5416](https://github.com/rust-works/succinctly/issues/5416)).
The same holds at those two depths for `to_yaml`, `flatten`, `==`, `.[0]`, a field of a deep object (`{"a": .}` folded 2000 times, then `.a | length`) and the rest of the filters that walk the value.
Where a value built by the program is handed back to the evaluator whole (a field read out of it, `select`, a bind) the document ceiling of 256 levels no longer applies to it either: it was a bound for an input document, and a built value is bounded by the value-tree depth, or by the floor.
The bridge's converter keeps the 256-level document ceiling for a caller that has not armed the floor (a library embedder), so only the CLI's armed run reads a built value back past it.
A path is bounded the same way since [#6356](https://github.com/rust-works/succinctly/issues/6356): `.a.a... | path | length` over 385 and 1000 components is what yq prints, where the walk stopped at 384.
Past the floor the run refuses with `Error: nesting depth exceeds limit of the native stack` at exit 1, where yq's own Go stack is far deeper.
Where that is depends on the evaluation thread's stack and on what each level costs: the fold that builds the value is the first step to refuse, ahead of every emitter and presentation walk, so the emitters have not been seen to refuse on their own.
Checked on a 16 MiB evaluation thread (a throwaway build): the build and every consumer above answered at 8,000 levels and refused cleanly at 11,000, with no crash at any size.
A fold that builds a deep value is quadratic in yq mode, as it is in jq mode before [#5875](https://github.com/rust-works/succinctly/issues/5875): 100,000 levels does not finish inside two minutes.
The reshape of a result's presentation (its comments and styles) gives up on a navigation construction or a merge deeper than 384 levels and falls back to the plain result, so such a result loses the comments it would otherwise carry; the value itself is unaffected.
A document whose aliases expand to a deep value is not covered: `-P` and `explode(.)` over a chain of 385 anchors that each wrap the previous one still refuse at 256 levels, where yq answers.

## `-C` colors YAML as yq's go-yaml printer does, with one deliberate difference ([#5416](https://github.com/rust-works/succinctly/issues/5416))

yq has no color scheme of its own.
It tokenizes the text it just encoded with go-yaml's scanner and prints each line of a token's origin text in the color of the token's type: keys cyan, strings green, numbers and booleans bright magenta, anchors and aliases bright yellow, comments grey, everything else (null, hex and octal integers, `.inf`, tags, the dashes and colons) as it is.
Where a span opens and closes follows from how the scanner cuts the text: on the space before a value, across the line break a token carries to the next line's indentation (the empty span at the start of a line), and over the indentation of a continuation line.
`succinctly yq -C` runs a port of that scanner and printer (`src/bin/succinctly/yq_runner/go_yaml_color.rs`) over the text it writes, so those spans fall where yq's do.
Before #5416 it colored with a fixed three-color re-lexer that matched yq on almost no document.
Results are colored one at a time, as yq does: a custom tag makes the value after it a string (`!custom 5` is green), a reserved tag does not (`!!str 1` is magenta), a trailing comment of the document is added to the text that is tokenized, and a document's comments from before its content are printed apart from it, in grey, with their line break inside the span.
A scalar result (`.a` over `a: 1`, `.a + 1`) is printed bare and not colored at all, as in yq.
The text is fed to the scanner a line at a time, and a line is scanned once the next complete line has arrived, so a streamed `-C` still holds a line and its successor rather than the output (#808); a block scalar is one token, and is held whole.
Several thousand generated documents (comments, anchors, aliases, tags, block and flow styles, quoted and plain scalars of every type, several documents to a stream) through about twenty filters and the `-P`, `-I`, `-N`, `--no-doc`, `-e` and `-0` flags agree with yq v4.53.3 byte for byte wherever the uncolored output already agrees.
Pinned by `tests/yq_cli/i5416.rs`, whose rows were captured from yq v4.53.3.

One difference remains: **a double-quoted escape keeps its digits** (ADR-0018 rule 4(b): matching would corrupt data).
go-yaml leaves the `x`, `u` or `U` and the hex digits of `\x01`, `\u0001` and `\U0001F600` out of the token's origin text, so yq's `-C` prints `a: "x\u0001y"` as `a: "x\y"`, a different string.
`succinctly yq -C` prints the escape whole.
Every other character go-yaml drops from a token's text is dropped here too, because the line it prints would otherwise differ from yq's.

A very long line is the one shape that is not streamed in constant memory.
A line is scanned once it is 64 KiB long even if its end has not arrived, so a long line of many small tokens (a flow sequence of millions of numbers) is read as it comes.
A single token is held whole while it is scanned, as go-yaml holds it: a quoted or plain scalar of tens of megabytes on one line, or a block scalar, takes several times its size in memory until it ends.

yq's own colorizer also repeats the line break after a tag that ends its line (`a: !tag` / `  b: 1` prints a blank line between them), and `succinctly yq -C` repeats it as well.

## An `ireduce` loop variable answers a node read only when the read is its first stage ([#6088](https://github.com/rust-works/succinctly/issues/6088))

yq binds `$i` in `SRC as $i ireduce (INIT; UPDATE)` to the node `SRC` selected, so `$i | path`, `$i | key`, `$i | line` and the other node reads answer from where it sits.
`succinctly yq` binds the element's value, and takes each of those reads before the fold, at the one place the node is still in hand: `SRC | [., READ, ...]` feeds the fold, and the update's `$i | READ` is replaced by what that read answered.
The reads are `path`, `key`, `parent`, `line`, `column`, `document_index`, `file_index`, `filename`, `tag`, `anchor`, `alias`, `style`, `kind`, `line_comment`, `head_comment` and `foot_comment`, and the answer is yq's for each.
Only the first stage after `$i` is read from the node: in `$i | parent | key` the `parent` is, and the `key` of the parent sees a value, so it is `[]` where yq prints the parent's key.
A `$i | READ` stays a value read in an update that binds `$i` again anywhere (the rewrite cannot tell which `$i` a read means), in a nested `ireduce` over the same name, and in a fold whose pattern is not a single `$i` (`as [$a, $b] ireduce`).
Every read the update names is taken for every element, whether or not the step reaches it.
`parent` copies the parent once per element, so it costs the parent's size per step.

## `@csv` and `@tsv` spell a `null` as `null` ([#6131](https://github.com/rust-works/succinctly/issues/6131))

yq's `@csv` and `@tsv` are Go's `encoding/csv` over the node, and a field is the node's text.
`succinctly yq` encodes the same way (minimal quoting, arrays of arrays and of flat objects, a scalar input as its text, yq's error lines), pinned by `tests/yq_cli/i6131.rs`.
The one difference is a `null` element: yq writes the spelling the document used (`~` for `[~, a]`, `null` for `[null, a]`), and a value does not hold it, so `succinctly yq` writes `null` for both.
A number keeps its source text (`1.50`, `0x1f`, `1_0`) in both.

## `eval(expr)` ([#6081](https://github.com/rust-works/succinctly/issues/6081))

`eval(expr)` is yq's: each string `expr` yields is parsed as a yq expression and evaluated against the current node.
It is new in `succinctly yq` since #6081, and stays undefined in jq mode, where jq has no such builtin.
A stored expression, a computed one, several at once (`eval(.list[])`), the empty expression, a comment, and a write through `eval` (`eval(".a") = 5`, `del(eval(".a"))`, `(eval(".c[]")) += 1`) answer as yq v4.53.3 does.
`tests/yq_cli/i6081.rs` pins 45 rows of it, in JSON output.
These differ:

- **The nodes `eval` reaches print in the style of a value the program built.**
  `eval(".a")` on `a: {b: 1}` is `b: 1` here and `{b: 1}` in yq: a node's flow style, comments and quoting do not follow it through `eval`.
  JSON output agrees.
- **Nothing follows the node's position out of `eval`.**
  `eval(".a") | key` prints nothing here and `a` in yq, `eval(".a") | path` is `[]` and not `["a"]`, and `line` and `column` answer 0.
  A write that names the nodes through `eval` does land.
- **A variable of the enclosing program is not visible inside the expression.**
  `.s as $x | eval("$x")` is `.a.b` in yq and nothing here.
- **`null` yields no expression.**
  yq reads an absent key as no node (`eval(.zz)` prints nothing) and the node `null` as the expression `null` (`eval(null)` is `null`), and this tells the two apart only by the value, so both give nothing.
- **yq repeats `eval`'s outputs when the context holds several nodes.**
  `.c[] | eval(". * 2")` on `c: [1, 2]` is `2 4 2 4 2 4 2 4` in yq and `2 4` here, since `eval` is run once per node.
- **`--jq-extensions` reaches the text `eval` runs ([#7012](https://github.com/rust-works/succinctly/issues/7012)).**
  The text is read with the same opt-in as the program that holds the call, so a name yq's lexer rejects (`paths`, `limit`) is a parse error in it by default, as in yq, and runs under the flag.
  `succinctly yq --jq-extensions 'eval("paths")'` prints the paths, where yq refuses `paths` in both positions.
  This is an extension (ADR-0018 rule 5), off by default, and a nested `eval` carries the flag on.
- **Expressions nest 32 deep.**
  A stored expression that evaluates itself (`s: "eval(.s)"`) runs yq out of stack, and stops here with `eval: expressions nest more than 32 deep` (ADR-0018 rule 4, a crash).
- **The text of the error for an expression that does not parse differs.**
  yq says `bad expression - probably missing close bracket on (` or reports the lexer's message; both fail with exit 1.
- **`is_key` is still not defined.**
  [#6081](https://github.com/rust-works/succinctly/issues/6081) stays open for it.
- **The wording of the refusal of a block scalar header left of its owner can differ (#6543).**
  yq words it by the container it stops in (`did not find expected '-' indicator`, or `did not find expected key`).
  `succinctly yq` words it by the last marker of the line above: `'-' indicator` after a dash, `key` otherwise.
  The two differ on 28 of the 96 explicit-key and dash-chain documents of #6543's grid (for example `? x` / `: - k:` / `|`).
  Both fail with exit 1 and print nothing.
- **An explicit indentation indicator on a block scalar under `- ?` is measured from the wrong indent (#6553, #6490).**
  `- ?` / `   |2` / `     t` is the key `" t\n"` in yq and `"   t\n"` here: the indicator counts from the indent of the enclosing sequence (0 here), where yq counts from the indent of the mapping the `?` opens (2 here).
  Only the scalar's text differs; the document is accepted and the key is the same one.
- **An explicit indentation indicator on a block scalar under a chain of dashes is measured from the wrong indent (#6550, #6490).**
  `- - &x` / `  |2` / `    t` is `[["t\n"]]` in yq and `[["  t\n"]]` here, and `- - !!str` / `  |1` / `    t` is `[[" t\n"]]` in yq and `[["   t\n"]]` here: the indicator counts from the indent of the enclosing sequence, where yq counts from the indent of the innermost one.
  Only the scalar's text differs; the document is accepted.
- **An environment variable whose value is not valid UTF-8 is refused by `strenv()` and `env()` (#6566).**
  yq reads the bytes: `strenv(BADVAR)` with `BADVAR` set to `\xff\xfe` prints those two bytes, and `env(BADVAR)` reads them as a YAML document (`Error: EOF`).
  A string value here cannot hold non-UTF-8 text, so both stop with `value for env variable 'BADVAR' is not valid UTF-8 in strenv()` (or `env()`) and exit status 1, where they said `not provided` before #6566.
  An unset variable is `not provided` for `env()`, as in yq, and `""` for `strenv()`, which yq never checks ([#6771](https://github.com/rust-works/succinctly/issues/6771)).
- **`env(NAME)` decodes the value as YAML through `from_yaml`'s decoder, so it shares that decoder's gaps ([#6771](https://github.com/rust-works/succinctly/issues/6771)).**
  `12` is a number, `true` a boolean, `{b: fish}` a mapping and `~` a null, an empty value is `not provided`, and the first document of several is the answer, as in yq.
  A timestamp (`2024-01-02`) keeps the tag `!!str` and not `!!timestamp`, and the YAML 1.1 spellings `0b11` and `1_000` stay strings (`from_yaml` shares both, and [#2802](https://github.com/rust-works/succinctly/issues/2802) tracks the spellings).
  A mapping with a repeated key reads as one entry for `length`, where yq counts both.
  A value with a leading byte-order mark keeps it in its first key (`\ufeffa`), where yq drops it.
  A value nested more than 128 levels deep is refused (`nesting depth exceeds limit of 128`), where yq reads up to 10000.
  `!!binary aGVsbG8=` has the tag `!!str`, not `!!binary`.
  The node keeps no style, anchor or spelling of its own: `[1, 2]` has no `style` (yq: `flow`), an alias reads as its value, and a comment-only value is `null` where yq prints the comment.

### A multi-line plain scalar in a document with a trailing comment keeps the double-quoted spelling ([#6737](https://github.com/rust-works/succinctly/issues/6737))

A plain scalar the source folds across blank lines holds a line feed, which yq writes as a literal block (`k: |-` then the lines).
The cursor streamers write that block when the document has no trailing same-line comment.
A document with one keeps the spelling the streamers used before #6737, a double-quoted string with `\n` (`k: "a\nb"`), for any such scalar.
They write a line comment after the value, which a literal block would read as the last line of its text, and yq puts it on the block's header instead (`k: |- # c`).
Captured live (yq v4.53.3):

| input                     | real yq                        | succinctly yq           |
|---------------------------|--------------------------------|-------------------------|
| `k: a\n\n  b # c\n`       | `k: \|- # c\n  a\n  b\n`       | `k: "a\nb" # c\n`       |
| `a: x\n\n  y\nb: 1 # c\n` | `a: \|-\n  x\n  y\nb: 1 # c\n` | `a: "x\ny"\nb: 1 # c\n` |

Both read back as the same value, and `-P` (the DOM route) is unaffected.

## `head_comment` keeps a block's blank lines; an indented continuation is still read as a comment line ([#6772](https://github.com/rust-works/succinctly/issues/6772))

yq's head comment is the text above the node with its blank lines kept.
`# c` / blank / `a: 1` is `c` followed by a line feed, a blank line inside the block stays, and a first document's comment above a leading `---` is one text with the comment under it (`# c` / `---` / `# d` is `c` and `d` on two lines).
`succinctly yq` answers the same, and the first key the comment under the marker was indexed on has none of its own (`.a | key | head_comment` is empty there, as in yq).
Still different: a comment line indented under the first one (`# c` / `  # d`) is `c` and `d` here, where yq keeps the indentation and the `#` (`c` and `  # d`).

## `-o=json` of a non-finite float fails as yq's encoder does; a streamed result keeps its prefix ([#6867](https://github.com/rust-works/succinctly/issues/6867))

yq's JSON encoder refuses a float that is `+Inf`, `-Inf` or `NaN`, and `succinctly yq -o=json` fails the same way instead of printing `null`.
Two failures exist, and the message tells them apart (yq v4.53.3):

```console
$ yq -n -o=json '1 / 0'
Error: json: error calling MarshalJSON for type *yqlib.CandidateNode: json: unsupported value: +Inf
$ printf 'a: .inf\n' | yq -o=json '.'
Error: json: error calling MarshalJSON for type *yqlib.CandidateNode: json: error calling MarshalJSON for type *yqlib.CandidateNode: strconv.ParseFloat: parsing ".inf": invalid syntax
```

A computed number fails in `encoding/json` (`+Inf`, `-Inf`, `NaN`).
A document `.inf`/`.nan` scalar, in any of its twelve spellings or under `!!float`, fails earlier, parsing its own text.
yq wraps the failure once per node from the number up to the root, the number's own node included, and `succinctly yq` repeats the wrap the same number of times (`1 / 0` once, `[1 / 0]` twice, `{"a": [0 / 0]}` three times; a sequence the program builds counts as a node).
`-r` on a bare scalar skips the encoder in both and prints the number's own text (`+Inf`, `NaN`, or the document's `.inf`).
A quoted `".inf"`, an unknown spelling such as `.infx` and every non-JSON output format are unchanged.

The difference that remains is on the streamed route, where `-o=json` copies a document's scalars straight from the source (#808): the part of the failing result before the number has been written when the failure comes, where yq writes none of it.
`printf 'a: [1, .NaN]\n' | succinctly yq -o=json '.'` leaves `{`, `"a": [` and `1,` on stdout and then fails; yq prints nothing.
The results before the failing one are whole in both, and the run stops there, as it does for any other decode failure on a streamed route (#1615).
The route that evaluates a result first (a computed number, `-P`) writes nothing of it, as yq does.
Buffering each streamed result to match would give up the constant-memory write that route exists for, the same trade as the decode failures above.
The exit code and the error line are identical either way, so a caller that checks the status sees the failure, and it is pinned by `tests/yq_cli/i6867.rs`.
`to_json`, `to_json(N)`, `tojson` and `@json` fail the same way ([#6887](https://github.com/rust-works/succinctly/issues/6887)).
They raise the marshal error with the wrap count of the value they are given: `.a | to_json` over `a: .inf` has one wrap, `[.a] | to_json` two, `[1 / 0] | @json` two.
A document `.inf`/`.nan` used as a number fails in yq while it computes, in every output format, because yq reads each operand's text with `strconv.ParseFloat`.
`+`, `-`, `*`, `/` and `%`, the ordering operators `<`, `<=`, `>` and `>=` (in an assignment too), and `min`/`max` over two numbers one of which is such a scalar raise `strconv.ParseFloat: parsing ".inf": invalid syntax`.
`succinctly yq` raises it too, naming the left operand when both fail, and a hex or octal integer on the left names itself first, as it does for `0x10 + 1.5`.
`==`, `!=`, `//`, a string or `null` on the other side, `length`, and a computed infinity (`1 / 0`) are unchanged, and `[.a] | max` has no comparison to make.
When an update (`|=`, `map_values`) fails on several nodes, yq reports the last node's text and `succinctly yq` the first: over `a: .inf` and `b: -.inf`, `map_values(. + 1)` names `"-.inf"` in yq and `".inf"` here.
`sort` and `sort_by` over such a scalar panic in yq with a Go stack trace, and order it here (see the NaN and infinities entry in the sort section).

### `-o props|shell|csv|tsv|base64|uri` — the first slice of yq's output formats; six rows still differ ([#6850](https://github.com/rust-works/succinctly/issues/6850))

`-o=props`, `-o=shell`, `-o=csv`, `-o=tsv`, `-o=base64` and `-o=uri` (and the short names `p`, `s`, `c`, `t`) run the filter and write each result through that format's encoder, with no `---` between documents.
`--shell-key-separator`, `--properties-separator`, `--properties-array-brackets` and `--csv-separator` are read (#6922); `--csv-separator` takes one ASCII byte (yq's `char` takes any single byte, so a lone byte above 0x7f, which cannot be an argument here, is refused), and `-o=tsv` ignores it, as yq does.
The `@props` / `to_props` and `@csv` / `to_csv` operators read the same flags, as in yq, wherever they are parsed (the program, the text an `eval` reads, a `--split-exp` expression), and so does `format("csv")` under `--jq-extensions` (#6922).
`-o=lua` is read as well, with `--lua-prefix`, `--lua-suffix`, `--lua-globals` and `--lua-unquoted` (#6924); where it differs from yq is in its own section below.
`-o=ini` is read as well (#6924), as yq's go-ini encoder writes it: it agrees with yq v4.53.3 on 3,200 random JSON documents and on YAML input except for a null written `~` (see the list below).
`-o=toml` is read as well (#6924); where it differs from yq is in its own section below.
`-o=kyaml` is read as well (#6924); where it differs from yq is in its own section below.
`-o=xml` is read as well (#6924), with `--xml-attribute-prefix`, `--xml-content-name`, `--xml-proc-inst-prefix` and `--xml-directive-name`; where it differs from yq is in its own section below.
The other formats yq lists (`hcl`) and every input format but YAML, JSON, `kyaml`, `props`, `csv`, `tsv`, `ini`, `toml`, `base64` and `uri` (see the `-p kyaml`, `-p base64|uri`, `-p props`, `-p csv|tsv`, `-p ini` and `-p toml` sections below) are still refused at the flag.
Six rows differ from yq v4.53.3, each pinned out of `tests/yq_cli/i6850.rs`:

- A comment of a value the filter builds or writes is not written by `-o=props` or `@props` (`{"z": .a}`, `.a = 5`), where yq keeps the node's comment: the encoder reads comments from the document, as `to_yaml` does (#6933).
  A value read from the document gets them as yq writes them (#6922): above each property, its key's head and line comment and then its value's, each with its first `#` dropped and nothing between them (`# h` over `a: 1 # c` is `# h c`), after a blank line unless it is the first; the comment after a bare `-` goes to the item's first key, or to its first element if that is a scalar; a mapping's or sequence's own comment is dropped; and the first document's leading comment block comes first when the result is that document.
  Where succinctly attributes a comment to a different node than yq does, which `head_comment` and `line_comment` show too, the property gets it as succinctly attributes it (the comment after a bare `-` and the line break a blank line leaves under a head comment agree since #7034, and a block above the first key of a later bare `-` item, which yq reads as that key's foot, since #7044, and a byte-order mark's first comment since #7084).
  `head_comment` reads both from the document node, so a key read inside parentheses, after a `select` or under a non-YAML `-o` loses them (#7043), and after a write (`.[0].b = 2 | .[0].a | key | head_comment` over `- # m` / `  a: 1` is `""`, yq `"m"`), under `--eval-all` and on a built value they are not there (#7034).
  `@props` in the text an `eval` reads, or on the right of an assignment (`.x = (. | @props)`), writes no comment either: that evaluation holds the value without the document node.
  Two keys whose paths join to the same property (`a.b: 1` beside `a: {b: 2}`) write two lines here, each with the later key's comment, where yq writes the property once, with the later value and comment.
- A null written `~` (or `!!null ""`) prints `null` with every encoder, `ini` included, where yq echoes the spelling (`c = ~`, `c='~'`, `a = `, #2802).
- An encoder that fails after earlier results printed still prints those results first, as yq does (#6923): `printf 'a: x\nb: [1]\nc: {d: e}\n' | yq -o=csv '.[]'` prints `x` and `1`, then the error, exit 1.
  A top-level comma (`.a, .b`) still prints nothing before the error, and the error line is written when it is raised, so the two streams can interleave differently on a terminal.
  The results are kept on the plain, `--eval-all` and `-n` routes; the other input routes were not checked.
  An error the program itself raises still prints nothing, in both tools.
- The NFKD table behind a `shell` key is Unicode 15.0.0's, the version of yq v4.53.3's `golang.org/x/text`: a character added by a later Unicode release with an ASCII decomposition loses its letters here where a newer yq keeps them.
- A `<<` whose value is a literal mapping, not an alias, is merged into its mapping by the encoders, where yq leaves it a plain key that props, shell, ini and lua skip: `printf 'a: 1\n<<: {b: 2, a: 5}\n' | yq -o=props .` is `a = 1` in yq and `a = 5`, `b = 2` here.
  An alias (`<<: *anchor`) is merged by both, and `-o=json` agrees (the owned route; see #5521 for the same family).
- `-o=csv` and `-o=tsv` with `-0` write each result's row here and a bare NUL in yq: yq's CSV writer is not flushed before the NUL, so the row is lost.
  Writing it matches nothing yq can be asked to read back (rule 4(b)).

### `-o=lua` — a value's source style, a key's type and a comment are not read (#6924)

`-o=lua` writes a table literal as yq's Lua encoder does (tab indent, `,` after an array element and `;` after a map entry, yq's string escapes, `nil`, `(1/0)`, `(-1/0)` and `(0/0)`, a `0o` integer in decimal), and agrees with yq v4.53.3 on 2,400 random JSON documents (under six combinations of the four options).
The encoder runs over the value model, which keeps no node style, key tag or comment, so on a YAML input these still differ:

- A literal (`|`) or folded (`>`) string is written `"l1\nl2\n"`, where yq writes a long bracket (`[[`, a line break, the text, `]]`).
  A single-quoted string is written double-quoted, where yq keeps its single quotes.
- A key that is not a string (`1: a`, `true: b`) is written `["1"] = "a"`, where yq writes `[1] = "a"` (its tag survives).
- A comment is dropped, where yq writes a line comment as ` -- text` after the entry.
- A timestamp (`2001-01-01`) and `!!binary` are written as strings, where yq stops with `lua encoder NYI -- !!timestamp`.
- The integer spellings `-0o17` and `0O17` and the YAML 1.1 ones `1_000`, `0b11` and `0XfF` are strings here (see the YAML 1.1 section above), so they are quoted, where yq writes them bare.

### `-o=toml` — comments and a value's source spelling are not read (#6924)

`-o=toml` is a port of yq's TOML encoder (attributes before tables, `[a.b]` tables, `[[a]]` arrays of tables, inline tables inside arrays, Go `%q` strings) and agrees with yq v4.53.3 on 3,800 random JSON documents.
Go's `%q` escapes every code point `unicode.IsPrint` rejects, and that set follows the Unicode version of the Go release that built yq, so `src/jq/go_print.rs` is generated from the pinned binary (`scripts/gen_go_print.py`) rather than from a Unicode table here.
The encoder runs over the value model, which keeps no comment, node style or source spelling, so on a YAML input these still differ:

- A head, line or foot comment is dropped, where yq writes a head comment above its key and a line comment after the value (`a = 1  # l`).
- A boolean written `True`, `FALSE` or `yes` is `true` or `false`, where yq writes the source text.
- The YAML 1.1 integers `1_000` and `0b11` are strings here (see the YAML 1.1 section above), so they are quoted, where yq writes them bare.
- A timestamp (`2001-01-01`) and a `!!binary` scalar are quoted strings, where yq writes their text bare.
- yq's inline-table hint is set only by its TOML decoder, and `-p toml` writes the nodes as YAML text that is read back, so a TOML file written back as TOML (`*.toml` under `-o auto`, `-i`) writes an inline table as a table and drops its comments (see `-p toml` below).

### `-o=kyaml` — comments and an opening `---` are not read (#6924)

`-o=kyaml` is a port of yq's KYAML encoder and agrees with yq v4.53.3 on 3,500 random JSON documents under `-I0`, `-I1`, `-I4` and `-N`, and on multi-document input, where the `---` between documents is printed as for YAML.
The encoder runs over the value model, so on a YAML input these still differ:

- A head, line or foot comment is dropped, where yq writes it (`a: 1, # c`).
- An input whose first document opens with `---` prints that `---` first in yq (it keeps the document's leading separator as leading content), and here it does not; the same goes for the comments above it, and for the result of `to_entries`, `from_entries` and `with_entries`, which carry them in YAML output since #7081.
- The YAML 1.1 integer `1_000` is a string here (see the YAML 1.1 section above), so it is quoted, where yq writes it bare.
- `-C` colours the output in yq (`-o=kyaml` and `-o=toml` have colour printers of their own) and does nothing here.
- With `-0`, the separator between documents is `\n---` here and `---` in yq, the existing difference of the `-0` multi-document section above.

### `-p base64|uri` — read as the string they decode to; invalid UTF-8 and `--front-matter` differ ([#6925](https://github.com/rust-works/succinctly/issues/6925))

`-p base64` and `-p uri` decode each input whole to one string, as yq's decoders do, and the run then reads that string as `-p json` reads a JSON string, so every route after the read (streaming, `--eval-all`, `-i`, `filename`, `file_index`) is the JSON one.
A JSON string and yq's decoded node evaluate and print alike: they agree with yq v4.53.3 on 27 decoded strings under 13 filters and output formats, and the decoders agree on 1,300 random inputs (`tests/yq_cli/i6925.rs` pins a sample).
`-p base64` follows Go's stream decoder: the input is trimmed of white space and padded with `=` to a multiple of four bytes counting any line break inside it (so `aGk` decodes, and `aG` / `k=` on two lines is `unexpected EOF`), and an `illegal base64 data at input byte N` counts `N` from the start of the chunk the decoder was reading, which is past the first 680 bytes of a longer input.
Padding that ends a chunk is accepted with data after it, as in yq, so the answer for a long input with `=` in the middle depends on where it falls.
`-p uri` is Go's `url.QueryUnescape`: `+` is a space, a trailing line break stays, and a bad escape is `invalid URL escape "%zz"`.
An empty input, and a second `-` that finds stdin drained, is the empty string, a document, where an empty YAML or JSON input has none.
Under `-p auto` the first input file named `*.base64` or `*.uri` (in any case, and under `--from-file` too) picks that format for every file of the run and, with `-o auto`, prints in it too, as yq does; an explicit `-p` prints YAML.
The extension is Go's `filepath.Ext`, lowercased, and its aliases are read for JSON as well, so `a.JSON`, `a.j` and a file named `.json` are JSON input and output.
`-R` reads raw lines whatever `-p` says, and `--front-matter` on a file picked by its extension reads its front matter as YAML, as for a `*.json` file.

Two rows differ:

- Decoded bytes that are not UTF-8 meet the `-p json` handling above (#1616): `-o json` writes each as U+FFFD, as yq does, and YAML output stops with `invalid UTF-8 in string`.
  The U+FFFD is substituted before the filter runs, so `length`, which counts bytes in both tools, counts three for it where yq counts the one byte (`printf 'a%FFb' | yq -p uri -o json length` is `3` in yq and `5` here), as it already did for a `-p json` string.
  yq prints the raw bytes for a document that is the string itself (`printf /w== | yq -p base64 .` writes the byte `0xff`) and refuses a nested one with `yaml: cannot marshal invalid UTF-8 data as !!str`.
- `--front-matter` with an explicit `-p base64` or `-p uri` is refused (`--front-matter and --input-format base64 are incompatible`), as `-p json` is, where yq decodes the extracted front matter.

### `-p props` — read as the YAML yq prints for the file; blank comment lines and collection tags differ ([#6925](https://github.com/rust-works/succinctly/issues/6925))

`-p props` (alias `p`, `properties`) loads each input with a port of magiconair/properties v1.8.10, the library yq's decoder uses, and builds yq's nodes from it with a port of the traversal yq assigns through, then writes them as the YAML text yq prints for them; the run reads that text as `-p yaml`, so every route after the read is the YAML one.
The loader is magiconair's: `=`, `:` or white space between key and value, `#` and `!` comments at the start of a line, `\` escapes and line continuations, a comment that ends the file with no line break dropped, and its errors (`properties: Line 4: "3"` for a value with no key, `premature EOF`, `invalid unicode literal`), each passed through Go's `fmt.Errorf` as yq's is, so a `%` in it reads as a verb (`"50%!"(MISSING)`).
Every value is checked for `${key}` expansion as magiconair does at load time (`malformed expression`, `circular reference in:`, `expansion too deep`) and then kept unexpanded; of several bad values yq names one in Go's random map order, and this names the first in the file.
A key is split on `.` into a path: a segment that is an `int32` indexes a sequence, a `null` becomes a mapping or a sequence by the segment that first indexes it, a segment is a key glob (`ab*` matches every key it fits and, with a comment, renames them), a string is never indexed into (`a=1` then `a.b=2` keeps `a: "1"`), an index on an existing sequence is read by yq's `parseInt` (`0x1`, `1_0`), and `--properties-array-brackets` reads `name[0]` as a name and an index.
A comment is assigned to the key (or the sequence element) the path ends at, and a comment that cannot be assigned skips its property silently, as yq's `return nil` does.
An empty input has no document; a file holding only comments or blank lines is `{}`.
Under `-p auto` a first input file named `*.properties`, `*.props` or `*.p` (any case) reads and, with `-o auto`, prints as props.
yq's nodes have no style, and the text read here is marked to have none either (#7031), so a write prints its result with the quoting yq's encoder picks (`a=1` with `.a |= . + "x"` prints `a: 1x`) and `style` reads `""`, while the text's quoting still decides the tag (a date and a `<<` key stay strings).
These agree with yq v4.53.3 on 120 crafted files under `-o yaml`, `-o json`, `--properties-array-brackets` and three filters, and on 1,600 random files under `-o yaml` and `-o json`, apart from the rows below (`tests/yq_cli/i6925.rs` pins a sample).

These rows differ:

- A mapping or sequence the decoder made has an empty tag in yq (`.a | tag` on `a.b=1` is empty, and `type` too); here it is `!!map` or `!!seq`.
- Blank lines that end a comment block are dropped, and two or more inside one are one, as yq itself reads the YAML it prints for them; yq's props decoder keeps them in the node.
- `--front-matter` with an explicit `-p props` is refused, where yq decodes the front matter as props; on a file picked by its extension the front matter is read as YAML.
- An index that would pad the sequences of one document past 2^24 elements is refused (`Cannot grow array to N elements`), where yq allocates some 545 bytes for every padded `null` until it runs out of memory, and the `${key}` check stops once it has substituted 16 MiB, where yq would expand a chain that doubles at each link without end.
- `-o props` writes no comment, as for any input (the `-o props` section above).
- The first property's comment opens the YAML text, so it is read as the document's head comment, where yq assigns it to that key: `del(.a)` on `# top` / `a=1` keeps `# top` here and drops it in yq, and `. | head_comment` is `top` here and empty in yq.
- A key printed in the `? ` explicit-key form (over 128 bytes) whose value is a mapping loses the comment above that mapping's first key.
- A key glob that a comment renames into two keys spelled alike (`abc=1`, `abd=2`, `# c`, `ab*=9`) prints as yq does, but a lookup reads both keys here (`.["ab*"]` is `9` in yq and `9` twice here), where yq matches one key per spelling.
- A comment holding U+0085, U+2028 or U+2029 is written as one line; go-yaml's emitter breaks the line there.
- A sequence nested directly in a sequence more than 128 deep (`0.0.0...=x`) stops with the YAML reader's `nesting depth exceeds limit of 128`, and a key of more than 10,000 segments, or a document whose YAML text passes 1 GiB, is refused; yq reads all three, since it never writes the text.

### `-p csv` and `-p tsv` — a cell go-yaml rejects, a marker in a cell and a comment around a scalar differ ([#6925](https://github.com/rust-works/succinctly/issues/6925))

`-p csv` (alias `c`) and `-p tsv` (alias `t`) read each input with a port of Go's `encoding/csv` reader, the one yq's decoder uses, and write the nodes yq makes of its records as YAML text; the run reads that text as `-p yaml`, so every route after the read is the YAML one.
The first record is the header and every later record is a mapping from the header's names to its cells, so a document is a sequence of mappings; a header alone is `[]` and an input with no record is `null`.
The reader is Go's with its defaults: the separator is `--csv-separator` (a tab for TSV, which ignores it), blank lines are skipped, a quote is only allowed in a quoted field, a record must have the header's field count, a `\r\n` is a `\n` even inside a quoted field, and a byte-order mark is dropped.
Its errors are Go's, with the line and column Go counts, behind `bad file '<name>': ` (`parse error on line 2, column 6: extraneous or missing " in quoted-field`, `bare " in non-quoted-field`, `record on line 2: wrong number of fields`).
A header repeated becomes a key repeated, as in yq.
A cell is a snippet of YAML under the default `--csv-auto-parse` / `--tsv-auto-parse`, as yq's `parseSnippet` reads it: a snippet that is a string (go-yaml's typing of a plain scalar, so `yes`, `007` is an integer and a date is a timestamp; a quoted, tagged-`!!str` or aliased one is a string), that is not YAML, or that holds more than one document stays the cell's own text, `'x'` and `a # c` with their quote and comment, while any other node is the node the snippet makes (`5 # c` the integer `5`, `0x1f` and `1_000` integers that keep their spelling, `[1,2]` a sequence, `a: b` a mapping, `&a 5` an anchored integer, a bare `!` a null).
`--csv-auto-parse=false` reads a cell as a plain scalar of its text instead.
Under `-p auto` a first input file named `*.csv` or `*.tsv` (or `*.c`, `*.t`, any case) reads and, with `-o auto`, prints as a table, and `-i` writes the table's format back.
These agree with yq v4.53.3 on 6,500 random tables under `-o yaml`, `-o json`, `--csv-auto-parse=false` and `-p tsv`, and on 7,000 random byte strings (every error of the reader agreed, with its line and column), apart from the rows below (`tests/yq_cli/i6925.rs` pins a sample).

These rows differ:

- A cell go-yaml rejects but this YAML reader accepts is a node here and a string in yq: a block sequence item or key opened by a plain scalar that starts with `]` (`- ]x`), and a line that starts with a tab used as indentation (` \t1` is the integer `1` here, the string ` \t1` in yq).
- A byte that is not UTF-8 in a cell or a header is U+FFFD here, so `length` counts it as three bytes; yq keeps the byte (`length` 1), escapes it as `\\ufffd` under `-o json` and refuses it under `-o yaml` (`cannot marshal invalid UTF-8 data as !!str`).
- A U+FEFF inside a cell, where go-yaml skips one at the start of a line, and a lone `\r` in a place YAML does not allow a line break, read differently.
- A cell that opens with `---` or carries a `%YAML` directive is read as one document here (or a string); yq leaks its own `# $yqDocSeparator$` marker into the node, which is not reproduced.
- A comment before or after a scalar cell (`# c` / `5`, `5` / `# c`) is dropped here, where yq keeps it as a comment of the node, and a cell with a leading line break keeps no trailing blank.
- A header that needs the `? ` explicit-key form (it holds a line break or is over 128 bytes) loses a comment-only or blank cell's comment: `? |-` / `  : # c` prints `:` here.
- `--front-matter` with an explicit `-p csv` or `-p tsv` is refused (`--front-matter and --input-format csv are incompatible`), as `-p json` is, where yq decodes the front matter as a table.
- A number that go-yaml types as an integer or float but this YAML reader does not (`1_000`, `0b1`, an integer past 2^63) prints as yq prints it under `-o yaml` and differs under `-o json`, the same difference as for a YAML input.
- yq's warning that `-o auto` defaults to YAML when `-p csv` is explicit is not written.

### `-p ini` — a byte that is not UTF-8 and `--front-matter` differ ([#6925](https://github.com/rust-works/succinctly/issues/6925))

`-p ini` (alias `i`) loads each input with a port of go-ini v1.67.0's reader, the library yq's decoder uses with its default options, and writes the nodes yq makes of the file as YAML text; the run reads that text as `-p yaml`, so every route after the read is the YAML one.
A line is a comment (`#`, `;`), a section (`[name]` up to the line's last `]`, the name untrimmed) or a key (`name = value` or `name : value`, the name optionally in `"`, `"""` or backticks, a `-` an auto-numbered `#1`, `#2`, ...).
A value ends at the first `#` or `;`, loses one pair of surrounding `'` or `"` (kept under `--ini-preserve-quotes`), goes on over lines while it ends in `\`, and runs over lines when it opens with `"""` or a backtick; a value left unclosed at the end of the input is go-ini's `missing closing key quote from ... to ...`, quoted as Go's `%q` writes it.
A key written twice keeps its first place and its last value, a section written twice is one section, and the keys before the first section belong to `DEFAULT`, which is the document's root, followed by one mapping per other section (an empty one is `{}`); every key and value is a string.
`%(name)s` in a value is expanded from the key's section, its parent sections (`a.b` has `a`) and `DEFAULT`, as go-ini's `Key.String()` does, up to 99 times.
The errors are go-ini's behind `bad file '<name>': failed to parse INI content: ` (`key-value delimiter not found: k`, `unclosed section: [s`, `empty key name: =v`, `empty section name`).
An empty input is `{}`.
Under `-p auto` a first input file named `*.ini` or `*.i` (any case) reads and, with `-o auto`, prints as INI, and `-i` writes it back.
These agree with yq v4.53.3 on 4,200 random files under `-o yaml`, `-o json` and `--ini-preserve-quotes` (`tests/yq_cli/i6925.rs` pins a sample).

These rows differ:

- A byte that is not UTF-8 is U+FFFD here, so `length` counts it as three bytes and a go-ini error that quotes the line holding it shows U+FFFD; yq keeps the byte.
- A value whose `%(name)s` expansion grows past 16 MiB is refused (`the value of key "a" expands past 16777216 bytes`), where go-ini expands a chain of values that each repeat the next twice (`a=%(b)s%(b)s`, `b=%(a)s%(a)s`) until it runs out of memory.
- `--front-matter` with an explicit `-p ini` is refused (`--front-matter and --input-format ini are incompatible`), as `-p json` is, where yq decodes the front matter as INI.
- A document whose text passes the YAML reader's limits is refused, where yq builds the nodes without writing the text.

### `-p xml` — read as the YAML yq prints for the file; comment placement, a nil-element panic and multi-byte charsets differ ([#6925](https://github.com/rust-works/succinctly/issues/6925))

`-p xml` (alias `x`) reads each input with a port of yq's `decoder_xml.go` over Go's `encoding/xml` (Go 1.27.1's source; yq v4.53.3 is built with Go 1.26.4), and writes the nodes yq makes of the file as YAML text; the run reads that text as `-p yaml`, so every route after the read is the YAML route's.
The tokens come from a port of the Go decoder: `RawToken` by default (`--xml-raw-token`, so an end tag is never matched against its start tag and a prefix is not translated), `Token` under `--xml-raw-token=false`, lenient unless `--xml-strict-mode` (an unknown entity is left as written, an attribute may be unquoted or have no value, an unclosed element is closed at the end of the input).
An element is a mapping of its attributes (under `--xml-attribute-prefix`, `+@` by default) and of its children in document order, a name that repeats is a sequence, its text is the `--xml-content-name` entry (`+content`, a sequence when the text comes in several pieces), and an element with neither children nor attributes is its text or `null`.
Every value is a string: nothing is read as a number or a boolean.
Text is trimmed to its first and last graphic character (yq's `trimNonGraphic`, so a no-break space and a zero-width space at the ends go), a processing instruction is `+p_<target>`, a directive `+directive`, and a namespace prefix stays in the name unless `--xml-keep-namespace=false`; `--xml-skip-proc-inst` and `--xml-skip-directives` drop those.
A comment becomes a head, line or foot comment by where it sits relative to the element's start and its text, by yq's rules.
A declared `encoding` is opened as `golang.org/x/net/html/charset` opens it: the UTF-8 labels (an invalid byte is U+FFFD) and every single-byte encoding (`iso-8859-1` and `us-ascii` are Windows-1252, `windows-1251`, `koi8-r`, `iso-8859-15`, `ibm866`, ...), with its tables generated by `scripts/gen_yq_xml_tables.py`.
The errors are Go's behind `bad file '<name>': ` (`XML syntax error on line 1: unexpected EOF`, `invalid XML: Encountered chardata [x] outside of XML node`, `xml: unsupported version "1.1"; only version 1.0 is supported`).
An empty input is `null`.
Under `-p auto` a first input file named `*.xml` or `*.x` (any case) reads and, with `-o auto`, prints as XML, and `-i` writes it back; in `-o xml` a `null` read from XML is the empty element it was (`<c/>` is `<c></c>`, where it is `<c>null</c>` for any other input).
`--xml-keep-namespace`, `--xml-raw-token`, `--xml-skip-directives`, `--xml-skip-proc-inst` and `--xml-strict-mode` are read, and the prefix and name flags of `-o xml` are read here too.
These agree with yq v4.53.3 on random documents and their mutations under `-o json` and the flag sets above (`scripts/yq-xml-input-sweep.py`; `tests/yq_cli/i6925_xml.rs` pins a sample).

These rows differ:

- **yq dies of a Go panic** when a token other than an end tag follows an end tag that closes the document's root (`</a><b/>`, `</a>x`): it dereferences a nil element.
  This reads such an input as an error (`invalid XML: markup after the end of the document's root`), as a panic would take the process down.
- **A comment's blank line and its depth** are yq's node printer's, and the text this writes is read back by the YAML reader, which attributes a comment by its indentation and what follows it.
  yq writes a blank line before the continuation of a multi-line line comment on a top-level key (`<a><!--x\ny--></a>` is `a: # x`, an empty line, `# y`) and before the foot comment of a multi-piece text, where this prints none.
  A foot comment after an element that has attributes or children, when a less indented line follows it (`</dependency><!-- logging --></dependencies><properties>` in a `pom.xml`), prints at the depth of the inner block's last scalar, where yq prints it at the key's.
  A comment before a `null` item of a sequence moves after the next item.
  The same text read as `-p yaml` prints the same way in yq, so this is the cost of reading nodes through their text; about 4% of random documents that put comments in every position differ in one of these ways.
- **A node's source form** is that of the YAML text: an empty element is a `null` that prints `null` in every output but `-o xml` (`-o props`, `-o shell`, `-o lua`, `tostring`, `@csv`), where yq's node has an empty value and prints nothing; and a string has the double-quoted style the text gave it, so `style` is `double` where yq's is empty and `.a.b |= . + "x"` keeps the quotes (`b: "1x"`, yq `b: 1x`).
- **`head_comment`, `line_comment` and `foot_comment`** report the comment's attribution in the text that was read: a foot comment yq puts on a value (`<b>1</b><!--f-->`) is the key's here.
- **Elements nested deeper than 2,000 levels** are refused (`invalid XML: elements nested deeper than 2000 levels`): the tree is written by recursion, and yq, which has no bound, takes minutes on 100,000 levels.
- **A multi-byte encoding** declared in the prolog (`shift_jis`, `euc-jp`, `euc-kr`, `gbk`, `gb18030`, `big5`, `iso-2022-jp`, `utf-16`) is refused (`xml: opening charset "shift_jis": unsupported charset: "shift_jis"`), as a label that is no encoding is; yq converts them.
- **A byte that is not UTF-8 in a name** is U+FFFD in the error message here (`invalid XML name: <U+FFFD>`), where Go prints the byte.
- **A character Go 1.27 classifies differently from Go 1.26** at the end of a text node (a code point assigned in Unicode 17 or later) is trimmed or kept by the newer table.
- `-o xml` drops a comment (the `-o=xml` row above), so `-p xml -o xml` loses the comments of its input, and a `null` that a filter assigns (`.a = null`) is written empty when the input was XML, where yq writes `null`.
- `--front-matter` with an explicit `-p xml` is refused (`--front-matter and --input-format xml are incompatible`), where yq decodes the front matter as XML.
- yq's warning that `-o auto` defaults to YAML when `-p xml` is explicit is not written.

### `--unwrapScalar=false` with `-o=yaml` — a scalar result prints as a node, with these differences ([#7093](https://github.com/rust-works/succinctly/issues/7093))

`-r` / `--unwrapScalar` takes its value after `=` (`--unwrapScalar=false`, `-r=true`), and the value yq resolves is its own when given and else true for `-o=yaml` and `-o=props` and false for every other format.
It decides whether `-o=shell` writes a value unquoted, whether `-o=props` and `@props` put a value with a space in it in Go's `%q` quotes, and whether a string result of the JSON and KYAML encoders is written raw; `@props` follows it for every output format, so `-o=json '{"a":"x y"} | @props'` quotes `x y` as yq does.
With `=false`, `-o=yaml` prints a scalar result as the node it is: its source style (`"a b"`, `'it''s'`, a block scalar as `|` with its text two spaces in), its `&anchor`, its line comment (`x # c`), and a quote where its text asks for one (`""`, `"12"`, `"true"`).
A computed string has no style and prints plain.
A document, mapping or sequence is unaffected.
This needs the streaming routes off and the bare-scalar style strip skipped, so it is one process-wide switch set from the flags.
`tests/yq_cli/i7093.rs` and `tests/yq_cli/i7093_yaml.rs` pin the rows captured from yq v4.53.3; a sweep of 22 filters over 11 formats and 7 spellings of the flag over a clean input differs only for `-o=lua` over a multi-line string (a long bracket in yq, #6924).

Still different under `=false`:

- **An alias result** is `*x` in yq (`.b` on `b: *x`, unreadable without its anchor) and the value here, the soundness rule of ADR-0017.
- **A value reached through `to_entries`** (`to_entries | .[0].value`) loses its style, quotes and line comment, which a scalar that is navigated to keeps: the entries replay places only what a mapping or sequence holds.
  A folded scalar (`>`) reached that way prints as `|`.
- **A comma of two different scalars under `-P`** (`.a, .b` on `a: "@at"` / `b: "- dash"`) writes the first in double quotes in yq (`"@at"`, then `'- dash'`) and in single quotes here; the same scalar twice, and `.[]`, agree.
- **`-0` over several documents** writes `\n---` between them, as for every other YAML output.

### `-p kyaml` — read as YAML, as yq reads it ([#6925](https://github.com/rust-works/succinctly/issues/6925))

`-p kyaml` (alias `ky`) reads each input with the YAML reader, which is what yq's KYAML decoder is, so KYAML (YAML's flow subset with explicit commas) and block YAML both load and every route after the read is the `-p yaml` one.
Under `-p auto` a first input file named `*.kyaml` or `*.ky` (any case) reads as YAML and, with `-o auto`, prints as KYAML, and `-i` writes KYAML back.
`-o=kyaml` itself is under `-o=kyaml` above: its comment and `---` rows apply to KYAML read back as well, since a comment in the input is dropped by the KYAML printer here and kept by yq.

### `-p toml` — a date-time's tag, an infinity in JSON, a comment's place and nesting past a bound differ ([#6925](https://github.com/rust-works/succinctly/issues/6925))

`-p toml` loads each input with a port of go-toml v2.3.1's `unstable` parser (the one yq's decoder drives with `KeepComments`) and of yq's `decoder_toml.go` over it, and writes the nodes yq makes of the file as YAML text; the run reads that text as `-p yaml`, so every route after the read is the YAML one.
The parser checks TOML's grammar and nothing else, so a key written twice, a table opened twice and `1__0` are all accepted, and its errors are its own words behind `bad file '<name>': ` (`expected value, not eof`, `expected newline but got U+0062 'b'`, `invalid character at start of key: =`).
yq then decodes each value: a number or boolean keeps its source text under `!!int`, `!!float` or `!!bool` (so `0xDEAD_beef`, `+17` and `1__0` print as written), a string is a `!!str`, a date-time is checked with Go's `time.Parse` and has no tag, and what Go's `strconv` refuses is an error (`0b101` is `strconv.ParseInt: parsing "0b101": invalid syntax`, `+nan` and `1e999` are `strconv.ParseFloat` errors).
A local date, local time and local date-time are `unsupported type LocalDate` (`LocalTime`, `LocalDateTime`), and a date-time that Go does not read as RFC 3339 (a space instead of the `T`, a lowercase `t` or `z`, a month of 13, a 30th of February, an hour of 25, a second of 60) is the error of reading it as `2006-01-02` (`parsing time "1979-05-27 07:32:00Z": extra text: " 07:32:00Z"`).
A key and a table are written into the document with yq's own assignment (`DeeplyAssign`), through its traversal: a key is a glob (`"a*" = 1` writes into every key it fits, and a glob that is a key of a table renames the keys it matched), a plain `=` replaces what the path names, a table is merged in with its arrays appended, `[[a]]` is `a += [table]`, and a segment that names an array with an element continues into its last element, so `[a.b]` after `[[a]]` writes into the last `a`.
A path through a scalar writes nothing and raises nothing, and `a = 1` followed by `[a]` is `cannot multiply !!int with !!map`.
The two quirks of yq's own that a port has to keep: the keys of an inline table are listed in an ordered map by their text, so of two spelled alike only the last is merged (`x = {a.b = 1, a.c = 2}` is `x: {a: {c: 2}}`, though `x = [{a.b = 1, a.c = 2}]` keeps both), and every comment run inside an array contributes only its first comment.
Comments gather in a list that the next value, table or array element takes as its head comment (an array element keeps one), the comments above the first key are the document's, a comment on the line of a value is that value's line comment, and one that is the last thing in a table, or in the file, belongs to the table or is dropped, as in yq.
A document with no key (an empty file, or only comments) holds no document: `-o yaml` prints an empty line and `-o json` prints `null`.
Under `-p auto` a first input file named `*.toml` (any case) reads and, with `-o auto`, prints as TOML, and `-i` writes it back; the format has no other name here, as in yq (`tml` is not one, and `-p tm` is refused).
These agree with yq v4.53.3 under `-o yaml` and `-o json` on 5,036 TOML files found on a developer machine (`pyproject.toml`, `Cargo.toml`, tool configuration), and on 14,000 random documents (every scalar spelling above, quoted, dotted and glob keys, tables, arrays of tables, inline tables, comments and CRLF, and the same documents with one or two characters deleted, inserted or replaced); `tests/yq_cli/i6925_toml.rs` pins 190 rows.

These rows differ:

- **A date-time's tag.** yq's node has none, so `.a | tag` prints an empty line; the text is a plain timestamp here, which is `!!timestamp`.
  The value, `-o json` and `-o yaml` agree.
- **An infinity or NaN in JSON.** `a = inf`, `-inf`, `+inf` and `nan` print as `!!float inf` (and so on) under `-o yaml`, as yq prints them, but `-o json` writes the string here, where yq stops with `json: error calling MarshalJSON ...: json: unsupported value: +Inf`.
  The same difference is there for `a: !!float inf` in a YAML input: the JSON writers here refuse a plain `.inf` as yq does and not a tag-forced one.
- **An underscored number.** `1_000`, `1__0`, `0xDEAD_beef` and `1_0.5` are plain scalars in yq's output but strings to the YAML reader here (the recorded input-spec divergence for underscored numbers), so they are written with their tag (`!!int 1_000`, `!!float 1_0.5`) and read back as numbers; `-o json` and arithmetic agree (`.a + .b` is `1000 + 3735928559`), and `-o yaml` shows the tag.
- **Where a comment is attributed.** The nodes are written as text and read back, so `head_comment`, `line_comment` and `foot_comment` show the YAML reader's attribution, not yq's: yq prints the head comment of a scalar value after its line (`b: x` then `# before b`), and the reader here attributes that line to the next key; the printed text agrees.
  A table's or an array element's head comment, and a scalar's line comment, agree.
- **A comment on an empty table.** yq writes the head comment of an empty table inside braces (`t: {` / `  # c` / `}`), which no YAML reader keeps; here it is written above its key.
- **A line comment on a collection.** yq's emitter writes it on the next line that has no comment of its own (`a = {x=1} # c` then `b = 1` is `b: 1 # c`), and the same is done here, including for the next key whose value is a collection (`b: # c`) and a `- ` item that is a mapping (`- # c`); when that line is an empty `[]` or `{}`, yq writes broken YAML (`b: # c` and `{}` at column 0) and here the comment follows the value (`b: {} # c`).
- **CRLF.** go-toml keeps the `\r` of a CRLF file in every comment and yq writes it back as a bare `\r` that ends the line, so its output has no line break there; the comment is without it and the line break is `\n` here.
- **`-o toml`.** A TOML file read and written back as TOML (`yq -i` on `*.toml`) loses its comments and writes an inline table as a table, as every `-o=toml` input does (see `-o=toml` above); yq keeps both.
- **Nesting.** An array or inline table nested more than 400 levels, or a key of more than 400 dotted parts, is refused (`the document nests arrays, inline tables or dotted keys more than 400 levels deep`), where yq's stack grows as far as memory allows; the YAML reader's own limit (128, see above) is reached first by a document that nests that many levels in the YAML text.
  A path that indexes an array past 2,097,152 elements (`a = [1]` then `a.99999999 = 1`) is refused (`Cannot grow array to N elements`) rather than padded with `null`.
- `--front-matter` with an explicit `-p toml` is refused (`--front-matter and --input-format toml are incompatible`), as `-p json` is, where yq decodes the front matter as TOML.
- **Speed.** A run of `[[array]]` tables is written in place here, so 20,000 of them read in half a second; yq copies the array once per table and takes minutes (500 tables take 0.4 s there, 2,000 take 11 s).

### `-o=xml` — comments, a source spelling and an empty document are not read (#6924)

`-o=xml` is a port of yq's XML encoder over Go's `encoding/xml` printer (its indentation state, its text and attribute escapes, the refusals of a start tag with no name, of an attribute that is not a scalar, of a processing instruction with a bad target or a `?>` and of a malformed directive) and agrees with yq v4.53.3 on 5,000 random JSON documents.
It runs over the value model, which keeps no comment or source spelling, so on a YAML input these still differ:

- A head, line or foot comment is dropped, where yq writes it as an XML comment (`<!-- c -->`, `<a>1<!-- l --></a>`).
- A null written `~` is `null` and a boolean written `True` is `true`, where yq writes the source text.
- An empty document, which yq prints as nothing, is `null` here (#5721).
- A processing-instruction target is checked with an approximation of Go's XML name tables (Unicode letters, digits, `-`, `.`, the middle dot and the common combining marks), so a character the tables treat differently is accepted or refused here and not in yq.
- The `--xml-keep-namespace`, `--xml-raw-token`, `--xml-skip-directives`, `--xml-skip-proc-inst` and `--xml-strict-mode` flags are read by `-p xml` (see `-p xml` below); `-o xml` has no use for them.

### Decoded bytes that are not UTF-8 (`@base64d`, `@urid`, `load_str`): `length` and YAML output differ ([#7000](https://github.com/rust-works/succinctly/issues/7000))

yq keeps the bytes `@base64d`, `@urid` and `load_str` decode as they are, and its JSON encoder writes one U+FFFD for each byte that is not part of a valid sequence (Go's `encoding/json`).
A string here holds UTF-8 only, so each such byte becomes one U+FFFD when it is decoded (`substitute_invalid_utf8_per_byte`, the count of the `-p json` reader, #1616), and `-o json` agrees with yq (`"4oI=" | @base64d`, the bytes `e2 82`, is two U+FFFD in both; an `@base64d` fuzz over 500 random binary payloads agrees on every one).
Two things still differ, because yq has the raw bytes and this has the substitutes:

- `length` counts bytes in both tools, so each substitute counts three where yq counts one (`"4oI=" | @base64d | length` is `2` in yq and `6` here).
- YAML output prints the substitutes, where yq prints the raw bytes for a document that is the string itself and refuses a nested one with `yaml: cannot marshal invalid UTF-8 data as !!str`.

Matching either needs a string that can hold bytes that are not UTF-8, or the substitution moved to the JSON encoder, as the `-p json` reader does with `-o json` only.

## The argument-level flags of [#7025](https://github.com/rust-works/succinctly/issues/7025) answer as yq does, with these differences

`--expression`, `--split-exp-file`, `-f`, `-v`/`--verbose`, `--security-disable-env-ops`, `--security-disable-file-ops`, `--string-interpolation`, `-c`/`--yaml-compact-seq-indent` and `--header-preprocess` are yq's, and `tests/yq_cli/i7025.rs`, `tests/yq_cli/i7025_string_interpolation.rs`, `tests/yq_cli/i7025_compact_seq_indent.rs` and `tests/yq_cli/i7025_header_preprocess.rs` pin the rows captured from yq v4.53.3.
[#7025](https://github.com/rust-works/succinctly/issues/7025) stays open for `--security-enable-system-operator` and `system`, `--yaml-fix-merge-anchor-to-spec` and `--debug-node-info`.

- **`--security-disable-env-ops` and `--security-disable-file-ops` refuse an operator when it runs, where yq refuses it when its handler is called.**
  `select(false) | strenv(X)`, `.zz[] | strenv(X)` and `.a | select(. == 2) | strenv(X)` fail with `env operations have been disabled` in yq, because the handler is called with an empty context.
  Here they print nothing and exit 0, since no value reaches the operator and nothing reads the environment.
  The operators that yq does not call (`.a // strenv(X)` with a truthy `.a`, `map(strenv(X))` over an empty list, `with(select(false); strenv(X))`) agree.
- **The env flag also refuses the environment readers yq has no token for.**
  Under `--jq-extensions`, `$ENV`, `env` and `env.X` read the environment, and `--security-disable-env-ops` refuses them with the same message.
- **`-v` writes no debug log.**
  yq writes `time=... level=DEBUG ...` lines to stderr; standard output is the same either way.
  `--verbose=false` is accepted, as in yq, in the spellings of Go's `strconv.ParseBool`.
- **A boolean flag's value that is not a boolean exits 2 here and 1 in yq.**
  `--string-interpolation=maybe` and `--verbose=zz` are refused at the flag in both, and neither writes output.
  yq says `invalid argument "maybe" for "--string-interpolation" flag: strconv.ParseBool: parsing "maybe": invalid syntax` and prints its usage; this says the same cause in clap's wording, and exits 2 like every other flag clap refuses (`-p badformat`).
- **`--string-interpolation=false` is one setting for the whole run, as in yq.**
  A `\(` in a string literal stays the two characters, in the program and in an `eval(..)` text, and the backslash pair in front of a `(` is left alone (`"\\(1)"` is two backslashes).
  A string inside an interpolation slot is a lexer error in yq whatever the flag says, and here too.
- **`-c` writes the whole run through the emitter that keeps comments, anchors and styles, so a large input does not stream.**
  The cursor streamers do not know a sequence indented by less than a whole step, so a `-c` run takes the route `-P` takes (`can_stream_pretty_or_colored`) and builds the document, where the same filter without it streams it.
  The cost is memory and time in proportion to the document; it has not been measured.
  What that route writes differently from the streamers (the merge key `<<` listed as its merged members, an explicit `? key`) is the route's own and shows under `-c` too.
  Over 588 YAML files of the repository at `-I 2`, 3, 4 and 6 no difference comes from the flag itself.
- **`-c` is yq's flag, which is `--compact-output` in `succinctly jq`.**
  `syq -c` changes the indent of block sequences, and does not compact JSON output (`-o json` ignores it, as in yq).
- **`--tab` with `-c` is not compacted.**
  yq has no `--tab`, so there is no reference for it.
- **`--header-preprocess=false` reads a YAML input with no header, the reading a byte-order mark gives, with the same shapes still differing.**
  The leading comments are the first node's head comment (`# lead` / blank / `a: 1` prints `# lead` / `a: 1`, the blank line gone), a first `---` is read and not echoed, `%YAML 1.1` / `---` is read, and the break after a leading `--- text` line is kept (`--- x` / `y` is `"x y"`, where the default's glue gives `"xy"`).
  A text of only blank lines and comments is printed back as it was, and one with a `---` in it is the empty document, a blank line.
  Every row was captured from yq v4.53.3 (`tests/yq_cli/i7025_header_preprocess.rs`), and 10,360 generated runs (every leading block of up to three of `# c`, `---`, `%YAML 1.1`, `--- # c` and `--- x`, before a map, a sequence, two documents and a commented map, under `.`, `.a`, `-P`, `-N`, a write, `-o=json` and `head_comment`/`foot_comment`/`tag`/`line`) differ only in:
  - a comment or text on a `---` line: `--- # c` / `a: 1` keeps `# c` as the first node's head comment in yq and drops it here, and `# c` / `--- x` / `y` drops the comment in yq and keeps it here (the shape listed under the byte-order mark above);
  - `%YAML 1.1` or `1.2` with no `---` after it, an error in yq and read here;
  - a blank line made of a tab or a carriage return among comment lines (CRLF text): yq writes it back as a `#` line, and this reads the text as a document and prints a blank line;
  - the `line` of a document with no content (`---` / `---` / `a: 1` is `[0,2]` in yq and `[0,3]` here).
  A scalar or flow-collection document with two comment blocks, not in that sweep, differs as after a byte-order mark ([#7089](https://github.com/rust-works/succinctly/issues/7089)).
  The flag is one setting for the run, so under `--eval-all` and with several files every input is read with no header, as yq does.
  `--front-matter` keeps its own reading, and a JSON input has no header either way.
- **A missing `--split-exp-file` is worded differently.**
  yq says `Error: open nope.exp: no such file or directory`; this says `failed to read split expression file: nope.exp`, with the cause.
  Both exit 1 and write nothing.

## `--split-exp` names a file as yq does ([#7029](https://github.com/rust-works/succinctly/issues/7029)), with these differences

`tests/yq_cli/i7029.rs` pins the rows captured from yq v4.53.3.
The name is the first value the expression produces, read as its scalar text, with the output format's extension appended when the name has none ([#7028](https://github.com/rust-works/succinctly/issues/7028)).

- **A name read from a source `~`, `NULL` or `True` is `null.yml` or `true.yml`, where yq writes `~.yml`, `NULL.yml` and `True.yml`.**
  The source spelling of a null or a boolean is not kept ([#2802](https://github.com/rust-works/succinctly/issues/2802)).
  A number keeps its spelling (`0x10.yml`, `007.yml`, `1e3.yml`, `1.50`), as does a quoted string.
- **A number literal such as `0.1e1` in the expression is accepted, where yq's lexer refuses it** (`bad split document expression`).
  The same literal is accepted in the main filter, so this is the expression grammar and not the naming.
- **A directory that cannot be made or a name that cannot be written is worded differently.**
  yq says `Error: mkdir blocker: not a directory`; this says `failed to create --split-exp directory: blocker`, with the cause.
  Both exit 1 and write nothing.
- **The second and later files of `-o=yaml` and `-o=kyaml` start with `---` in yq**, where here no file does (a YAML printer shared by every result writes the document separator before each after the first).
  An encoder format's files agree: `-o=props`, `-o=xml`, `-o=csv` and the rest are named by the result and hold only the encoding ([#7036](https://github.com/rust-works/succinctly/issues/7036)).

## yq's `eval` and `eval-all` subcommands are accepted; its other subcommands and `--version` text are not ([#7040](https://github.com/rust-works/succinctly/issues/7040))

`succinctly yq e`, `eval`, `ea` and `eval-all` behave as yq's, and `tests/yq_cli/i7040.rs` pins the rows captured from yq v4.53.3.
`--eval-all` stays as succinctly's own spelling of `ea`; yq has no such flag (`Error: unknown flag: --eval-all`), so `e --eval-all` is accepted here.

- **`help`, `completion` and `man` are not subcommands here.**
  `succinctly yq help` reads `help` as the expression and says `help/0 is not defined`, where yq prints its usage.
- **`--version` prints this crate's own name and version.**
  yq prints `yq (https://github.com/mikefarah/yq/) version v4.53.3`.
- **`ea -i` and `ea -f` are refused.**
  yq accepts both (`ea -i '.k = 1' a.yaml b.yaml` writes every result into the first file), and `succinctly yq` stops with `--eval-all and --inplace are incompatible` or `--eval-all and --front-matter are incompatible` ([#7042](https://github.com/rust-works/succinctly/issues/7042)).

## A key written as a literal block scalar (`? |-`) is written back as yq does; a folded one is not ([#7032](https://github.com/rust-works/succinctly/issues/7032))

yq keeps the style of a key node.
A literal block key with a line break or over 128 bytes is an explicit key (`? |-` / `  a` / `  b` / `: v`, with the anchor and tag it carries), and one short line is double-quoted.
`tests/yq_cli/i7032.rs` pins the rows captured from yq v4.53.3, under `-I` 1, 3 and 4 and `-P`.
These still differ:

- **A folded block key (`? >`) is not kept.**
  yq writes `? >` / `  a b` / `` / `: 1` for the key `"a b\n"` and `"a b": 1` for the one-line key `a b`; this writes `"a b\n": 1` and `a b: 1`.
- **`keys`, `to_entries` and the other reads of a key node print it without the style.**
  yq prints `- |-` / `  a` / `  b` for `keys` over the key above, where this prints `- "a\nb"` ([#5450](https://github.com/rust-works/succinctly/issues/5450)).
- **`-P` over an anchored literal key writes `? &k "a\nb"`**, where yq keeps `? &k |-`.

## `env(..)` and `strenv(..)` take the raw text up to the `)` as the variable name, as yq's lexer does ([#7070](https://github.com/rust-works/succinctly/issues/7070))

`strenv("NAME")` looks up a variable called `"NAME"` with the quotes in it, as yq v4.53.3 does (the docs' own `yq -n 'strenv("MYENV")'` finds none and prints an empty line), `strenv( X )` one called ` X `, `strenv(a;b)` one called `a;b`, and `strenv(.a)` one called `.a`.
`env("NAME")` raises `value for env variable '"NAME"' not provided in env()` for the same reason.
`tests/yq_cli/i7070.rs` pins 19 rows captured from yq.
In jq mode (`env(VAR)` is succinctly's extension there) the argument stays an identifier.

- **`strenv (X)` with a blank before the `(` is accepted here.**
  yq's lexer refuses it (`invalid input text "strenv (PLAIN)"`).
- **The wording of the errors differs.**
  An empty argument (`strenv()`, `env()`) and an extra `)` (`strenv(a(b))`) fail in both; yq says `invalid input text "strenv()"` and `bad expression, got close brackets without matching opening bracket`, this says `parse error ... expected identifier, found ')'` and `unexpected character ')'`.

## A scalar appended to a sequence takes the first element's quote style in a construction, not yet in a write ([#7070](https://github.com/rust-works/succinctly/issues/7070))

yq copies the first element's `Style` onto a scalar that `+` appends to a sequence: `a: ['x']` with `.a + "z"` is `['x', 'z']`, `[x, 'y']` is `[x, 'y', z]`, and a block sequence stays a block sequence.
A construction does the same (`.a + "z"`, `[.a + "z"]`, `{"k": (.a + "z")}`); it used to lose the sequence's flow style and every quote.
`tests/yq_cli/i7070_append_style.rs` pins 165 rows captured from yq v4.53.3, among them an appended scalar that carries an anchor, a comment, a tag or a block style.
These still differ:

- **A write: `.a += "z"`, `.a = .a + "z"` and `.a |= . + "z"`** print the appended scalar plain (`['dog', cat]`) where yq copies the quote (`['dog', 'cat']`).
  The write route replays the presentation of the old document and the appended element has none.
- **A number or a boolean** appended (`.a + 5`, `+= true`) prints bare here and quoted by yq (`['x', '5']`, `['x', 'true']`): yq's copied style turns the scalar into a quoted string.
- **A right side that is a variable or an assignment's value** (`.a as $x | [$x + "z"]`, `.c = (.a + "z")`) loses the sequence's presentation, as it did before.
- **`.a + null`** and **`[] + ["z"]`** print a block sequence where yq keeps the left sequence's flow style.

## A run of backslashes before `\(` and an unclosed `\(` are read as yq reads them ([#7063](https://github.com/rust-works/succinctly/issues/7063)), with these differences

`tests/yq_cli/i7063.rs` pins the rows captured from yq v4.53.3; the rule is in its header.
yq does not pair backslashes left to right: of `n` backslashes before `(`, an interpolation starts when `n / 2` is even, and a string with an unclosed `\(` is read as written, interpolations that did close included.

- **yq's warning for an unclosed `\(` is not printed.**
  yq writes `level=WARN msg="unclosed interpolation string, skipping interpolation"` to stderr and exits 0; this prints the same text on stdout and nothing on stderr.
- **An empty slot, `"\()"`, is the empty string in yq and a parse error here.**
- **With `--jq-extensions` a string with an unclosed `\(` is still a parse error**, because a slot may hold a string literal there and the end of the text is not known up front.
