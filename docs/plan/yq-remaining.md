# Plan: yq Remaining Work

Remaining work for full yq (Mike Farah's) compatibility.

See [yq Language Reference](../reference/yq-language.md) for implemented features.

## Operators (Low Priority)

| Operator       | Description                              | Notes                    |
|----------------|------------------------------------------|--------------------------|
| `eval(expr)`   | Evaluate string as expression            | Security implications    |
| `with_dtf(fmt)`| Set datetime format context              | Rarely used              |

## Format Encoders (Low Priority)

| Format   | Description              | Notes                          |
|----------|--------------------------|--------------------------------|
| `@xml`   | Encode as XML string     | Requires attribute conventions |
| `to_xml` | Same as @xml             |                                |

## CLI Options (Low Priority)

| Flag                   | Description                              |
|------------------------|------------------------------------------|
| `--explode-anchors`    | Expand anchor/alias references inline    |
| `--preserve-comments`  | Preserve comments in YAML output — partial (#710, #765): always-on for trailing same-line comments (value-scoped and key-scoped) on cursor-preserving paths (identity, navigation, filters, sort-keys); not yet for assignment |

## YAML Features (Low Priority)

| Feature          | Description                              | Notes                    |
|------------------|------------------------------------------|--------------------------|
| Merge keys       | `<<: *alias` syntax                      | Complex semantics        |
| Comment storage  | Store and query trailing same-line comments | Implemented (#710) — `bp_to_line_comment` in the YAML semi-index, `line_comment` builtin. Key-scoped comments on a deferred value's key line also preserved on identity/output (#765), but not exposed via any getter (matches real `yq`). Standalone `head_comment`/`foot_comment` and assignment-path preservation remain open |

## Date Arithmetic (Low Priority)

```bash
.time += "3h10m"   # Add duration
.time -= "24h"     # Subtract duration
```
