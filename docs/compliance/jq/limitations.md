# jq Error Message Conformance and Known Limitations

[Home](../../../) > [Docs](../../) > [Compliance](../) > jq Limitations

This page records how closely succinctly's evaluator reproduces jq's *error messages*,
measured against the pinned `jq` binary rather than asserted. Since
[#158](https://github.com/rust-works/succinctly/issues/158) bound the raised value as
`catch`'s input, the message text is readable from a filter — `try f catch (if
test("Cannot index") then … else … end)` is a real jq idiom — so the wording is part of
the observable surface, not stderr decoration.

Every expectation is captured from jqlang/jq at the version in
[`tests/data/jq-golden/JQ_VERSION`](../../../tests/data/jq-golden/JQ_VERSION); regenerate
and verify with:

```bash
./scripts/sync-jq-error-messages.sh          # recapture from the pinned jq
./scripts/sync-jq-error-messages.sh --check  # verify the table has not drifted
cargo test --features cli,regex --test jq_error_message_tests -- --nocapture
```

For jq *feature* coverage rather than error wording, see
[jq Language Support](../../reference/jq-language.md).

## Summary

Measured against jq-1.7.1 over the 190 probes in
[`tests/data/jq-error-probes.tsv`](../../../tests/data/jq-error-probes.tsv), through
**both** evaluators — the full one (`src/jq/eval.rs`) and the generic one
(`src/jq/eval_generic.rs`, which the CLI uses):

| Dimension                                    | Result               | Meaning                                            |
|----------------------------------------------|----------------------|----------------------------------------------------|
| **Message text** (both evaluators, verbatim) | **190/190 = 100.0%** | Byte-identical to jq                               |
| **Wording divergences**                      | **0**                | Every probe that errors in both errors identically |
| **Behaviour / parser gaps**                  | **0**                | succinctly does not raise the error at all         |

These three numbers are asserted, not maintained by hand: `jq_error_message_tests.rs`
parses them back out of this page and fails if they drift from the corpus (they went stale
twice while #356 was being written).

The non-passing probes are enumerated individually, with a category, reason and issue
link, in
[`tests/data/jq-error-known-divergences.txt`](../../../tests/data/jq-error-known-divergences.txt).
That file is the machine-readable source of truth; the test asserts it matches reality
exactly in both directions — a newly diverging probe and a newly matching one both break
the build — so it cannot silently drift from this page.

Crucially, **none of them is a wording bug**. In each case succinctly returns a value
or fails to compile the filter, so there is no message to compare; the wording is already
correct in `src/jq/error.rs` and will be reached once the underlying bug is fixed.

What the corpus *cannot* see is the mirror image — a filter on which succinctly raises an
error and jq returns a value — because a probe is only admitted if jq errors on it. Those
are listed in [Where succinctly errors and jq does not](#where-succinctly-errors-and-jq-does-not).

## The message vocabulary

jq reuses a small number of sentence shapes across many operations — `.a`, `.a = 1`,
`del(.a)`, `getpath(["a"])` and `. as {a:$a}` all report the *same* indexing error. Each
shape has one named constructor in [`src/jq/error.rs`](../../../src/jq/error.rs), so the
vocabulary is enumerable in one place rather than inlined at the ~300 raise sites where
it used to live. That inlining is how the two evaluators drifted from each other as well
as from jq: before #356 they reported `expected array or object, got number` and `cannot
iterate over number` for the same condition, and `cannot parse 'a' as number` against
`cannot convert 'a' to number` for `tonumber`.

| Shape                                                    | Constructor                                   |
|----------------------------------------------------------|-----------------------------------------------|
| `Cannot index <t> with string "<k>"`                     | `cannot_index`, `cannot_index_with_field`     |
| `Cannot index <t> with <key-type>`                       | `cannot_index_with_type`                      |
| `Cannot iterate over <t> (<v>)`                          | `cannot_iterate`                              |
| `<a> and <b> cannot be added\|subtracted\|…`             | `binary_op`                                   |
| `… because the divisor is zero`                          | `divisor_is_zero`                             |
| `<a> and <b> cannot have their containment checked`      | `containment_check`                           |
| `<a> and <b> cannot be iterated over`                    | `pair_cannot_be_iterated`                     |
| `<v> has no keys` / `has no length`                      | `has_no_keys`, `has_no_length`                |
| `<v> cannot be sorted, as it is not an array`            | `cannot_be_sorted`                            |
| `<v> cannot be matched, as it is not a string`           | `cannot_be_matched`                           |
| `<t> not a string or array`                              | `not_string_or_array`                         |
| `<v> cannot be parsed as a number`                       | `cannot_parse_as_number`                      |
| `<v> only strings can be parsed`                         | `only_strings_can_be_parsed`                  |
| `<v> only strings have UTF-8 byte length`                | `no_utf8_byte_length`                         |
| `Cannot check whether <t> has a <key-type> key`          | `cannot_check_has`                            |
| `Cannot use <t> (<v>) as object key`                     | `cannot_use_as_object_key`                    |
| `Invalid numeric literal at EOF at line 1, column <n> …` | `invalid_numeric_literal`                     |
| `Invalid path expression with result <v>`                | `invalid_path_expression`                     |

`EvalError::type_error` ("expected X, got Y") survives for the raise sites jq has no
counterpart for — succinctly extensions (`at_offset`, `@dsv`, `pick`/`omit`, module
loading) and builtins jq does not define. Anything jq also reports should use a named
constructor instead.

### One sentence covers a family, so probe the whole family

jq derives many builtins from others (`ascii_upcase` and `ascii_downcase` are both
`explode | map(…) | implode`; `to_entries` and `keys` both go through `keys_unsorted`;
`indices`, `index` and `rindex` all index their input with the pattern), so a sentence
fixed for one member is owed by every member. Fixing only the member a probe named left
`1 | with_entries(.)` saying `number (1) has no keys` while `1 | to_entries` beside it
still said `expected object, got number`. The corpus now carries at least one probe per
member for these families, and the sites that shared wording share a definition —
`non_string_pattern` and `unsearchable_input` in `src/jq/eval.rs` are the two refusals
behind all three string searches.

The families are worth naming, because "fix the site the probe names" caught this twice
in a row: the first pass fixed the *pattern* half of `indices`/`index`/`rindex` and left
their *input* half saying `expected string or array, got number`, one arm below. The rule
that holds is per family, not per raise site.

Sometimes jq's own source names the family outright. `strings` on the pinned binary yields:

```jq
def from_entries: map({(.key // .Key // .name // .Name): (if has("value") then .value else .Value end)}) | add | .//={};
def with_entries(f): to_entries | map(f) | from_entries;
```

So `from_entries` *is* object construction, and `with_entries` *is* `from_entries` — one
raise site in jq behind `{(0):1}`, `[{"key":0}] | from_entries` and
`{"a":1} | with_entries(.key = 0)` alike, which is why all three share
`cannot_use_as_object_key`. Succinctly had it as three: two hand-written copies of the
entry lookup (`from_entries` and `with_entries` reimplemented it separately, and neither
called the other) plus `key must be a string` in `eval_object_construction`. Both copies
*dropped* the entry rather than refusing it, so the caller got a smaller object with no
indication anything was lost — #391. The lookup now has one definition,
`entries_to_object` in `src/jq/eval.rs`, and `with_entries` is composed from
`builtin_to_entries` and it rather than restating either.

That composition is also what fixed `[1,2] | with_entries(.)`. jq reaches
`Cannot use number (0) as object key` because `to_entries` accepts an array — its keys are
the indices — and hands those number keys to `from_entries`. Succinctly's `with_entries`
matched only an object, so it reported `array ([1,2]) has no keys` from a type check jq
has not got. Deriving the builtin from the two it is defined over is what makes the right
sentence arrive without anyone choosing it.

The same source line pins the *alias* semantics, and its two halves disagree on purpose:
the key is a `//` chain (`key`, `Key`, `name`, `Name` — an alias holding `null` or `false`
is passed over in favour of a later one), while the value is a presence test (`value`, then
`Value` — an explicit `"value": null` beats a `"Value"` beside it). Succinctly had accepted
`k` and `v`, which jq does not, and neither `Key`/`Name`/`Value`, which it does. Correcting
the chain was a precondition for raising the error rather than a tidy-up beside it:
refusing a non-string key while still reading the wrong aliases would have failed
`[{"Key":"a","value":1}]` and `[{"key":null,"name":"a","value":1}]`, both of which jq
answers. The chain is pinned by golden cases (`from_entries_key_aliases`,
`from_entries_alias_falls_through_null`, `from_entries_value_aliases`) rather than probes,
because it is value behaviour and no probe can hold a filter jq does not error on.

"Passed over in favour of a later one" is exact, and *not* the same as falling through: the
chain's **last** alias has nothing later to be preferred, so `a // b` yields `b` whatever
`b` is. A falsy `.Name` is therefore the key, and `[{"Name":false}] | from_entries` is
`Cannot use boolean (false) as object key` — not the `null (null)` a uniform fall-through
would produce. Reading the tail as falling through too is the easy mistake, and the case
that hides it is a tail that is merely *absent*, which really is `null`; the probe pair
`from_entries_falsy_tail_key` / `from_entries_absent_tail_key` exists to separate them, with
`from_entries_alias_falsy_tail` pinning the same case end-to-end through the CLI. This half
of the chain *can* be probed, unlike the rest of it, precisely because jq errors here.

### Value rendering and truncation

Most shapes embed the offending value as `<type> (<json>)`. jq truncates that dump to a
fixed-width buffer (`jv_dump_string_trunc` with `char errbuf[15]`): a dump of at most 14
bytes is used verbatim, anything longer keeps its first 11 bytes and gains a `...`
suffix. Reproduced exactly:

```bash
$ echo '"abcdefghijkl"'  | sjq -c 'try .[] catch .'
"Cannot iterate over string (\"abcdefghijkl\")"     # 14 bytes — verbatim
$ echo '"abcdefghijklm"' | sjq -c 'try .[] catch .'
"Cannot iterate over string (\"abcdefghij...)"      # 15 bytes — truncated
```

String *keys* in indexing messages are not truncated; `.["aaaaaaaaaaaaaaaaaaaa"]` on a
number reports the whole twenty-character key.

## Truncation that splits a multi-byte character

jq cuts the dump at a byte offset and will happily split a UTF-8 sequence, emitting
invalid UTF-8:

```bash
$ echo '"あああああ"' | jq '.[]' | cat -v
jq: error (at <stdin>:1): Cannot iterate over string ("M-cM-^AM-^BM-cM-^AM-^BM-cM-^AM-^BM-oM-?M-=...)
                                                     # "あああ + a lone 0xE3 byte
```

A Rust `String` cannot hold that, so `dump_truncated` snaps back to the nearest character
boundary and emits `"あああ...` — one byte shorter, no replacement character. The two
agree whenever the cut lands on a boundary, which includes every 2-byte character
(`"ααααα...`, exactly 11 bytes).

This case is deliberately **absent from the probe corpus**: the captured table is a UTF-8
file read with `include_str!`, so jq's byte-exact output here is not representable. It is
recorded in prose instead rather than dropped silently.

## Conversion diagnostics beyond a single token

jq implements `tonumber` and `fromjson` by handing the string to its JSON parser, so a
failure surfaces as that parser's diagnostic. succinctly reproduces the single-token
form exactly:

```bash
$ echo '"0x10"' | sjq -c 'try tonumber catch .'
"Invalid numeric literal at EOF at line 1, column 4 (while parsing '0x10')"
```

and distinguishes it from a string that *is* valid JSON but is not a number, which jq
reports as the string itself:

```bash
$ echo '"null"' | sjq -c 'try tonumber catch .'
"string (\"null\") cannot be parsed as a number"
```

Inputs that fail *after* a complete token get a different jq diagnostic that succinctly
approximates to the EOF form:

| Input     | jq                                                     | succinctly                                                           |
|-----------|--------------------------------------------------------|----------------------------------------------------------------------|
| `"1 2"`   | `Unexpected extra JSON values (while parsing '1 2')`   | `Invalid numeric literal at EOF at line 1, column 3 (while parsing '1 2')` |
| `"1,2"`   | `Expected value before ',' at line 1, column 2 …`      | as above, with the EOF column                                        |
| `"{"`     | `Unfinished JSON term at EOF at line 1, column 1 …`    | as above                                                             |
| `"  a  "` | `Invalid numeric literal at line 1, column 4 …`        | as above, but with `at EOF` and column 5                             |

Matching these needs a position-reporting JSON parser reporting jq's exact failure
classes; the hand-rolled `parse_complete_json` in `src/jq/eval.rs` does not carry offsets.
The shapes a filter is likely to branch on (`Invalid numeric literal`, `cannot be parsed
as a number`) are exact, so this is left as a deliberate approximation.

Both builtins do require the *whole* string to be one JSON value, which is the part that
matters for the result rather than the message: `"0x10" | fromjson` errors as jq does
instead of returning `0`, and `"1 2" | fromjson` errors instead of returning `1`.

## Float literals lose their source spelling

jq's arithmetic messages echo the literal as written, because it keeps the number's
original text:

```bash
$ echo null | jq '1 / 0.0'
jq: error (at <stdin>:1): number (1) and number (0.0) cannot be divided because the divisor is zero
```

`OwnedValue::Float(0.0).to_json()` renders `0`, so succinctly says `number (0)` there.
This is the general "JSON numbers are re-rendered, not echoed" property of the evaluator
rather than anything specific to errors, and it only shows for float literals whose
shortest rendering differs from their source spelling.

## Behaviour and parser gaps

None remain open in this section's own narrative, tracked in
[`tests/data/jq-error-known-divergences.txt`](../../../tests/data/jq-error-known-divergences.txt).
That file's check is two-sided — a probe that starts diverging without a line there fails
the build, and so does a line for a probe that starts matching — so this section cannot
silently drift from the corpus.

What used to be listed here, and what closed it:

`repeat_error_swallowed` (`repeat(if . > 3 then error("boom") else .+1 end)` on `5`) was
here: jq propagates the generator's error the first time it is raised — here, on the very
first iteration, since `5 > 3` immediately. `eval_repeat` in
[`src/jq/eval.rs`](../../../src/jq/eval.rs) discarded the error (`Err(_) => break`) instead
of propagating it, so both evaluators produced no output at all rather than erroring.
[#495](https://github.com/rust-works/succinctly/issues/495) closed it: an error with no
prior output now surfaces as `QueryResult::Error` (via the same `partial()` helper #494 used
for `while`/`foreach`/`limit`), and output already produced before the error is no longer
discarded either.

`optional_write_negative_oob` (`.[-5]? = 9` on `[1,2]`) was here: `?` only suppresses
errors raised while *collecting* a path, not the write-time bounds check on a
still-negative array index, but succinctly treated `?` as suppressing the write too, so
`.[-5]? = 9` silently left the array unchanged instead of raising.
[#498](https://github.com/rust-works/succinctly/issues/498) closed it, landing together
with [#486](https://github.com/rust-works/succinctly/issues/486) — see "Where succinctly
errors and jq does not" below for the auto-vivification gap #498 depended on.

`slice_assign_non_array` and `slice_indices_not_integers` were the last two.
[#366](https://github.com/rust-works/succinctly/issues/366) made a slice a real path
component — `{"start":s,"end":e}` now comes out of `path()` and goes into
`getpath`/`setpath`/`delpaths`, `=`, `|=` and `del()` — so both sentences have somewhere to
be raised from. See "A slice is a path component" below for what that does and does not
cover.

`index_null_key_on_object`, `index_bool_key_on_object` and `index_object_key_on_object`
were on this list too, as parser gaps: `.[null]`, `.[true]` and `.[{}]` did not parse, so
no runtime error was reached. [#360](https://github.com/rust-works/succinctly/issues/360)
made index brackets take an arbitrary expression, and all three now raise jq's
`Cannot index object with <type>`.

`setpath_on_number` was on this list too. [#359](https://github.com/rust-works/succinctly/issues/359)
fixed it: `setpath` now auto-vivifies only `null`, as jq does, and refuses to index
anything else at any depth. The `setpath_*` probes added alongside it pin the rest of that
surface — wrong-key-type on a real container, out-of-bounds negative and NaN indices, and a
non-array path argument.

## Where succinctly errors and jq does not

A probe is only admitted to the corpus if jq errors on it, so the corpus is blind to the
opposite divergence: a filter that jq answers with a value and succinctly refuses. Those
have to be recorded here.

| Filter            | Input         | jq                          | succinctly                            |
|-------------------|---------------|-----------------------------|---------------------------------------|
| `.[1:2] = ["x"]`  | `null`        | `["x"]`                     | `Cannot index null with object`       |
| `.[1:2] \|= ["x"]`| `null`        | `["x"]`                     | `Cannot index null with object`       |

Both rows are a slice write walking a path *in place*, and the gap is the same
auto-vivification jq performs everywhere else it writes through a path: `null` grows into
whatever container the path names. Four non-slice rows used to sit above these — `.a = 1`
on `null`, `.a.b = 1` on `{}`, `.[5] = 9` and `.[5] |= 9` on `[1,2]`, all reported as
`Cannot index …`/`index N out of bounds (length M)` where jq builds or pads the container —
closed together by [#486](https://github.com/rust-works/succinctly/issues/486)
(`set_path`/`get_path_mut`/`update_path` now vivify `null` and pad past the array end, the
way `set_value_at_path` already did for `setpath()`) and
[#498](https://github.com/rust-works/succinctly/issues/498) (the write-time negative-index
bounds check now survives `?`, since padding is what makes the positive case succeed rather
than needing `?` to swallow a failure). No write operator produces `index N out of bounds
(length M)` any more; a numeric index past the end is not an error jq raises, so there is no
longer a positive case for succinctly's own wording to cover.

`del()` used to sit here too — `del(.[5])` and `del(.[-5])` on `[1,2]`, plus a missing
intermediate key or an out-of-range index — but every one of those is a silent no-op now,
matching jq, after [#477](https://github.com/rust-works/succinctly/issues/477),
[#527](https://github.com/rust-works/succinctly/issues/527) and
[#529](https://github.com/rust-works/succinctly/issues/529). A step that reaches nothing
reads as `null` and the rest of the path is walked against it, so only an `[]` tail still
raises (`{"a":{"x":1}} | del(.a.b[])` and `[1,2] | del(.[5][])` are both `Cannot iterate over
null (null)`).

The two slice rows were added by [#366](https://github.com/rust-works/succinctly/issues/366)
deliberately. Writing through a slice could have vivified `null` on its own — `setpath`
does, and the shared code was right there — but that would have left `.[1:2] = ["x"]`
growing a container while `.a = 1` beside it still refused, inside one feature; they matched
their neighbours instead. Now that #486/#498 closed the rest of this table, the slice rows
are the odd ones out rather than the ones fitting in — `.a = 1` and `.[5] = 9` vivify `null`
today, and `.[1:2] = ["x"]` still deliberately does not. See "A slice is a path component"
below for why that gap stays open.

An object key that yields something other than exactly one value is a second, unrelated
group — it is the key half of
[#354](https://github.com/rust-works/succinctly/issues/354):

| Filter        | Input       | jq                       | succinctly             |
|---------------|-------------|--------------------------|------------------------|
| `{(empty):1}` | `0`         | *(no output)*            | `key must be a string` |
| `{(.[]):1}`   | `["a","b"]` | `{"a":1}` then `{"b":1}` | `key must be a string` |

jq's object construction takes the cartesian product over each key's outputs, so a key
producing nothing produces no object and a key producing two produces two. Succinctly
evaluates the key to a single value and refuses anything else, with wording of its own —
the one sentence left in `eval_object_construction`, because reaching jq's answer here
means generating a stream, not renaming an error. The sentence stays succinctly's until
#354 is built.

`setpath` is the same operation without the syntax, and after #359 it does follow jq —
`[1,2] | setpath([5]; 9)` is `[1,2,null,null,null,9]`, and `null | setpath(["a"]; 1)` is
`{"a":1}`. The two used to disagree with each other in-tree; [#486](https://github.com/rust-works/succinctly/issues/486)
closed that by teaching `set_path`/`get_path_mut`/`update_path` to vivify the way
`set_value_at_path` already did, so `=`/`|=`/the compound operators/`//=` now agree with
`setpath` on every one of the four shapes removed from the table earlier in this section.
`delete_at_path` deliberately keeps its own,
different mechanism — a step that reaches `null` or an out-of-range index is a no-op rather
than a container to build (#476/#477), since `del` never needs to invent structure to delete
through — so it is not a fourth function taught the same rule, it is a different rule for a
different operation.

Where the same walk *does* error in jq, the sentence matches. A still-negative index is
jq's `Out of bounds negative array index` in `=` and `|=` as well as `setpath`, pinned by
the probes `assign_negative_index_oob`, `update_negative_index_oob` and
`assign_negative_index_nested`; `del` raises the same sentence for a negative index too.
Unlike the positive case above, `?` does not suppress this one —
[#498](https://github.com/rust-works/succinctly/issues/498) — because it is a write-time
bounds check, not a failure to collect the path.

## Refusing an allocation jq does not survive

`setpath` takes its array index from the document, so the array it pads is sized by the
input: `null | setpath([1e30]; 9)` asks for 9.2e18 elements. jq dies on that filter (it is
killed on the allocation, with no message to reproduce), and succinctly used to panic with
`capacity overflow`, which for a library means taking the embedder's process down. It now
refuses with `Cannot grow array to <n> elements` — succinctly's own wording, since there is
no jq sentence to copy. Only the impossible is refused; every length that fits in memory
still pads, so `[1,2] | setpath([5]; 9)` still agrees with jq.

## A slice is a path component

jq models `.[1:2]` as indexing with `{"start":1,"end":2}`, and treats that object as a
first-class path component: it comes out of `path()`, goes into `getpath`/`setpath`/
`delpaths`, and drives `=`, `|=` and `del()`.
[#366](https://github.com/rust-works/succinctly/issues/366) built that, closing
[#469](https://github.com/rust-works/succinctly/issues/469) with it. Before it,
`path(.[1:2])` answered `[1]` — one path per element, a wrong answer rather than a refusal,
inherited by everything built on `path()` — while `setpath` and `delpaths` silently left
the value alone.

The bounds a path carries are the ones written, not the ones resolved: `[1,2,3] |
path(.[-2:-1])` is `[{"start":-2,"end":-1}]`, because a path is only resolved against a
container when it is applied. Resolution then follows jq's `parse_slice` — floor the start,
ceil the end (`.[1.7:2.9]` on five elements is `[2,3]`, reachable through a runtime
descriptor even though the parser only folds integer literals), fold negatives against the
length, clamp, and pull `end` up to `start` if they crossed. That last step matters only
when writing, where crossed bounds are an insertion point: `[1,2,3] |
setpath([{"start":2,"end":1}]; ["x"])` is `[1,2,"x",3]`.

Deleting is the case worth knowing about. Every key naming an element of one array is
resolved against the length it had on entry and removed in a single pass, so overlapping
ranges union rather than compound — `[1,2,3,4] | del(.[0:2], .[1:3])` is `[4]`, and a slice
naming the same element as a bare index deletes it once
(`delpaths([[1],[{"start":1,"end":2}]])` is `[1,3,4]`). Doing the deletions one at a time
would resolve the second range against an already-shortened array.

`indices`/`index`/`rindex` came with it, since jq defines all three over `.[$i]` and so an
object pattern is the slice rather than a search: `"abcabc" | indices({"start":1,"end":2})`
is the substring `"b"`, and `index`/`rindex` are `.[0]`/`.[-1:][0]` of that, which is why
they report `Cannot index string with number`.

Two sentences arrived with the feature — `A slice of an array can only be assigned another
array` and `Array/string slice indices must be integers` — and both were already pinned as
probes, so the two-sided manifest check forced them to start matching in the same change.

What #366 did *not* build: computed bounds. `.[$a:$b]` is still a parse error, because the
parser folds slice bounds to integer literals; see
[docs/reference/jq-language.md](../../reference/jq-language.md). And writing through a
slice does not vivify `null` — see the table above for why that was deliberate.

## Reading a path is indexing

`path(f)` used to walk `f` through its own copy of jq's indexing rules, and that copy
disagreed with the value path in four ways at once — all fixed by
[#489](https://github.com/rust-works/succinctly/issues/489), which replaced both walkers
with one that asks the value evaluator for every step's verdict:

| Filter          | Input     | was       | jq, and now       |
|-----------------|-----------|-----------|-------------------|
| `[path(empty)]` | `{"a":1}` | `[[]]`    | `[]`              |
| `[path(.a?)]`   | `"s"`     | `[["a"]]` | `[]`              |
| `[path(.b.c)]`  | `{"a":1}` | `[[]]`    | `[["b","c"]]`     |
| `[path(.a)]`    | `"s"`     | `[["a"]]` | the sentence below |

The first row was the severe one: `[]` is a real answer — it is what `path(.)` returns —
and the one path that always resolves, so rendering "no paths at all" as it aimed a
caller's `getpath`/`setpath`/`delpaths` at the document root.

Three rules, and none of them lives here any more:

- **A step that reads `null` keeps its component.** A missing key, an out-of-range index
  and any step through `null` all read as `null`, and `null` accepts a further step — so
  the path exists even though nothing is stored along it. That is what `setpath`'s
  auto-vivification consumes.
- **A step that cannot index its value raises jq's sentence.** The eight `path_*` probes in
  the corpus are the `index_*`/`iterate_*` rows above wrapped in `path(...)`, and they
  report identically because the wording comes from the same place.
- **`?` suppresses that error and nothing else.** A pruned step names no path (it never
  happened), while a step that read `null` never errored and so is untouched by `?`.

What remained after that — [#483](https://github.com/rust-works/succinctly/issues/483) and
[#530](https://github.com/rust-works/succinctly/issues/530) — was the walker's catch-all
conflating two different questions. `resolve_node` (the pre-pass that turns a computed key
or a control-flow shape into concrete `Field`/`Index`/`Slice` components before the walker
ever runs) now has arms for every shape jq treats as path-capable: `..`, `recurse(f)`,
`recurse(f; cond)`, `select(f)`, the typeof filters, `first(f)`, `if/then/else`, `//`,
`limit(n; f)`, `try/catch`, `label $x | ...`, `E as $x | body`, and `getpath([...])` with a
literal array argument. `needs_path_prepass` — the gate deciding whether that pre-pass runs
at all — was rewritten from a whitelist of "known complex shapes" (which is what let
`resolve_node` grow support for `..`/`recurse`/`select`/the typeof filters without the gate
ever routing to it) to an exclusion check: everything needs the pre-pass except the bare
primitives the walker already handles natively.

Whatever is left over — a value-producing filter that is not a path expression at all, like
`1`, `length`, `keys`, `.a + 1` or `{a:1}` — now raises `Invalid path expression with result
<v>` (`EvalError::invalid_path_expression`, the `path_non_path_*` probes), matching jq by
name rather than answering `[]`. Confirmed live: `?` does not suppress it (`path(("a")?)`
still raises in jq, because this is a statement about the filter, not a value error raised
while collecting a path), so neither does this resolver's — the one call site is
`resolve_node`'s bare-`?` arm and `Expr::Try`'s, both checking
`EvalError::is_invalid_path_expression`.

One case stays out of scope on purpose: a *multi-output* non-path leaf used bare (`range(3)`
with nothing consuming its outputs as a further computed index) still reports succinctly's
existing "Cannot use a computed index after a multi-output path component" rather than
`invalid_path_expression` or jq's own compound wording for that shape — the same
`test_unsupported_path_prefixes_report_rather_than_misfire` boundary #412 already drew,
which this fix does not move. Every `path_non_path_*` probe is single-output, matching
#530's own repro list.

A closely related gap surfaced while verifying this: `path()` used to discard outputs
already streamed before a later sibling errors (`path(.a, 1)` produced nothing at all,
where jq prints `["a"]` then raises). `resolve_node` and friends now carry that prefix
alongside a resolve-time failure, and `builtin_path` reports it as a partial result
(`QueryResult::Partial`) instead of a bare error — confirmed against jq for both the
uncaught case and `?`/`try` suppressing just the trailing error while keeping the prefix.
This does not extend to `=`/`|=`/`del()`: jq's write-side path resolution is atomic
(`(.a, 1) = 5` produces no output at all in jq), so those three still discard any partial
prefix exactly as before.

## Errors reach the CLI with the right text but the wrong exit code

`succinctly jq` prints an uncaught evaluation error to stderr with jq's wording, but exits
0 where jq exits 5. That is tracked separately as
[#355](https://github.com/rust-works/succinctly/issues/355) and is why the error cases in
the golden corpus are written as `try … catch .`: the caught form puts the message on
stdout and exits 0, so it verifies the text through the real binary without depending on
the exit-code fix.

succinctly also omits jq's `(at <stdin>:N)` input-location prefix, which is emitted by
jq's CLI rather than its evaluator and is therefore outside the message the two share.

## Provenance

| Artifact           | Path                                                                                                       |
|--------------------|------------------------------------------------------------------------------------------------------------|
| Probes             | [`tests/data/jq-error-probes.tsv`](../../../tests/data/jq-error-probes.tsv)                                 |
| Captured messages  | [`tests/data/jq-error-messages.tsv`](../../../tests/data/jq-error-messages.tsv)                             |
| Known divergences  | [`tests/data/jq-error-known-divergences.txt`](../../../tests/data/jq-error-known-divergences.txt)           |
| Harness            | [`tests/jq_error_message_tests.rs`](../../../tests/jq_error_message_tests.rs)                               |
| Sync script        | [`scripts/sync-jq-error-messages.sh`](../../../scripts/sync-jq-error-messages.sh)                           |
| Message vocabulary | [`src/jq/error.rs`](../../../src/jq/error.rs)                                                               |
| Version pin        | [`tests/data/jq-golden/JQ_VERSION`](../../../tests/data/jq-golden/JQ_VERSION)                               |
| End-to-end goldens | [`tests/data/jq-golden/cases/`](../../../tests/data/jq-golden/cases/) (the `error_msg_*` cases)             |

The captured table is committed, so `cargo test` runs hermetically with no `jq` on PATH.
The `jq-drift` CI job re-checks it against the pinned binary, in the same step group as
the golden fixtures — so a jq upgrade surfaces as churn in the table and the divergence
manifest rather than as a silent mismatch.

To move to a newer jq, bump `JQ_VERSION`, install that version, run
`./scripts/sync-jq-error-messages.sh` and `./scripts/sync-jq-golden.sh`, and review the
diff.

## Depends On

- [jq Evaluator](../../reference/jq-evaluator.md) - the evaluator raising these errors
- [jq Language Support](../../reference/jq-language.md) - feature coverage matrix

## Used By

- [jq benchmarks](../../benchmarks/jq.md) - comparison against `jq`

## Source & Docs

- [`src/jq/error.rs`](../../../src/jq/error.rs) - the message vocabulary
- [`src/jq/eval.rs`](../../../src/jq/eval.rs) - full evaluator
- [`src/jq/eval_generic.rs`](../../../src/jq/eval_generic.rs) - generic evaluator (CLI path)
- [jq manual](https://jqlang.github.io/jq/manual/) - upstream reference
