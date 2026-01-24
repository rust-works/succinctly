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
| `--preserve-comments`  | Preserve comments in YAML output         |

## YAML Features (Low Priority)

| Feature          | Description                              | Notes                    |
|------------------|------------------------------------------|--------------------------|
| Merge keys       | `<<: *alias` syntax                      | Complex semantics        |
| Comment storage  | Store and query comments                 | Requires index changes   |

## Date Arithmetic (Low Priority)

```bash
.time += "3h10m"   # Add duration
.time -= "24h"     # Subtract duration
```
