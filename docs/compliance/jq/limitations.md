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

Measured against jq-1.7.1 over the 102 probes in
[`tests/data/jq-error-probes.tsv`](../../../tests/data/jq-error-probes.tsv), through
**both** evaluators — the full one (`src/jq/eval.rs`) and the generic one
(`src/jq/eval_generic.rs`, which the CLI uses):

| Dimension                                   | Result             | Meaning                                            |
|---------------------------------------------|--------------------|----------------------------------------------------|
| **Message text** (both evaluators, verbatim) | **98/102 = 96.1%** | Byte-identical to jq                               |
| **Wording divergences**                      | **0**              | Every probe that errors in both errors identically |
| **Behaviour / parser gaps**                  | **4**              | succinctly does not raise the error at all         |

The four non-passing probes are enumerated individually, with a category, reason and issue
link, in
[`tests/data/jq-error-known-divergences.txt`](../../../tests/data/jq-error-known-divergences.txt).
That file is the machine-readable source of truth; the test asserts it matches reality
exactly in both directions — a newly diverging probe and a newly matching one both break
the build — so it cannot silently drift from this page.

Crucially, **none of the four is a wording bug**. In each case succinctly returns a value
or fails to compile the filter, so there is no message to compare; the wording is already
correct in `src/jq/error.rs` and will be reached once the underlying bug is fixed.

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
| `<v> cannot be parsed as a number`                       | `cannot_parse_as_number`                      |
| `<v> only strings can be parsed`                         | `only_strings_can_be_parsed`                  |
| `<v> only strings have UTF-8 byte length`                | `no_utf8_byte_length`                         |
| `Cannot check whether <t> has a <key-type> key`          | `cannot_check_has`                            |
| `Invalid numeric literal at EOF at line 1, column <n> …` | `invalid_numeric_literal`                     |

`EvalError::type_error` ("expected X, got Y") survives for the raise sites jq has no
counterpart for — succinctly extensions (`at_offset`, `@dsv`, `pick`/`omit`, module
loading) and builtins jq does not define. Anything jq also reports should use a named
constructor instead.

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
classes; the hand-rolled `parse_json_string` in `src/jq/eval.rs` does not carry offsets.
The shapes a filter is likely to branch on (`Invalid numeric literal`, `cannot be parsed
as a number`) are exact, so this is left as a deliberate approximation.

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

The four probes on record are not wording problems — succinctly never raises the error, so
there is nothing to word. Each has its own issue and is listed in
[`tests/data/jq-error-known-divergences.txt`](../../../tests/data/jq-error-known-divergences.txt).

| Probe(s)                                                          | Divergence                                                       | Issue |
|-------------------------------------------------------------------|------------------------------------------------------------------|-------|
| `setpath_on_number`                                               | Builds a container on a scalar instead of refusing to index it   | [#359](https://github.com/rust-works/succinctly/issues/359) |
| `index_null_key_on_object`, `index_bool_key_on_object`, `index_object_key_on_object` | `.[null]`, `.[true]`, `.[{}]` are rejected by the parser | [#360](https://github.com/rust-works/succinctly/issues/360) |

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
