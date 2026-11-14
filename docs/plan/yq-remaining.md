# Plan: yq Remaining Work

Remaining work for full yq (Mike Farah's) compatibility. Every row was re-checked against Homebrew
`yq` v4.53.3 on 2026-11-02 (#5847).

See [yq Language Reference](../reference/yq-language.md) for implemented features, and
[limitations.md](../compliance/yq/limitations.md) for recorded divergences.

## Operators

| Operator     | yq v4.53.3                          | succinctly              |
|--------------|-------------------------------------|-------------------------|
| `eval(expr)` | Evaluates a string as an expression | `eval/1 is not defined` |

## Format Encoders

| Format   | Description          | Notes                                                                            |
|----------|----------------------|----------------------------------------------------------------------------------|
| `@xml`   | Encode as XML string | Unknown format here; XML input/output is listed as out of scope in the reference |
| `to_xml` | Same as `@xml`       | `to_xml/0 is not defined`                                                        |

## Removed From This List

- **Datetime operators** (`with_dtf`, `format_datetime`, `tz`, and `+`/`-` of a duration to a
  timestamp, `.time += "3h10m"`): implemented, with `now` and `from_unix` in the local zone (#5863).
  `now | type` is `!!str` here, and the other differences are in
  [limitations.md](../compliance/yq/limitations.md).

- **Merge keys** (`<<: *alias`): implemented; reads match yq's default, and the differences in key
  lists, writes and encoders are in [limitations.md](../compliance/yq/limitations.md) (#3556).
- **`--yaml-fix-merge-anchor-to-spec`**: implemented for value reads, iteration, the JSON rendering and
  `explode` (#7025); a malformed `<<` and the write routes differ, see
  [limitations.md](../compliance/yq/limitations.md).
- **Comment storage and preservation**: implemented (#710, #765, #2795); the remaining gaps are in
  [limitations.md](../compliance/yq/limitations.md).
- **`--explode-anchors`** and **`--preserve-comments`**: neither is a yq v4.53.3 flag (yq reports
  `unknown flag` for both). Anchors are expanded by the `explode(.)` operator, which is implemented,
  and comments are preserved without a flag.
