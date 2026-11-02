# Plan: yq Remaining Work

Remaining work for full yq (Mike Farah's) compatibility. Every row was re-checked against Homebrew
`yq` v4.53.3 on 2026-11-02 (#5847).

See [yq Language Reference](../reference/yq-language.md) for implemented features, and
[limitations.md](../compliance/yq/limitations.md) for recorded divergences.

## Operators

| Operator               | yq v4.53.3                          | succinctly                                     |
|------------------------|-------------------------------------|------------------------------------------------|
| `eval(expr)`           | Evaluates a string as an expression | `eval/1 is not defined`                        |
| `with_dtf(fmt; expr)`  | Sets the datetime format for `expr` | `with_dtf/2 is not defined` (#5863)            |
| `format_datetime(fmt)` | Formats a timestamp                 | `format_datetime/1 is not defined` (#5863)     |
| `tz(zone)`             | Converts a timestamp to a zone      | Raises `math function requires number` (#5863) |

## Date Arithmetic (#5863)

```bash
.time += "3h10m"   # yq adds the duration; succinctly concatenates the strings, exit 0
.time -= "24h"     # yq subtracts it; succinctly raises "cannot be subtracted"
```

`from_unix` renders in UTC where yq uses the local zone, and `now | type` is `!!float` where yq
says `!!timestamp` (#5863).

## Format Encoders

| Format   | Description          | Notes                                                                            |
|----------|----------------------|----------------------------------------------------------------------------------|
| `@xml`   | Encode as XML string | Unknown format here; XML input/output is listed as out of scope in the reference |
| `to_xml` | Same as `@xml`       | `to_xml/0 is not defined`                                                        |

## CLI Options

| Flag                              | Description                                                             |
|-----------------------------------|-------------------------------------------------------------------------|
| `--yaml-fix-merge-anchor-to-spec` | Merge `<<` per the YAML spec instead of yq's default; not accepted here |

## Removed From This List

- **Merge keys** (`<<: *alias`): implemented; reads match yq's default, and the differences in key
  lists, writes and encoders are in [limitations.md](../compliance/yq/limitations.md) (#3556).
- **Comment storage and preservation**: implemented (#710, #765, #2795); the remaining gaps are in
  [limitations.md](../compliance/yq/limitations.md).
- **`--explode-anchors`** and **`--preserve-comments`**: neither is a yq v4.53.3 flag (yq reports
  `unknown flag` for both). Anchors are expanded by the `explode(.)` operator, which is implemented,
  and comments are preserved without a flag.
