# DSV Input Performance Benchmarks

Benchmarks for `succinctly jq --input-dsv` parsing DSV (CSV/TSV) files and converting them to JSON.

## Methodology

Benchmarks measure:
- **Wall time**: Total elapsed time
- **Peak memory**: Maximum resident set size (RSS)
- **Throughput**: Input bytes processed per second

Run with:
```bash
./target/release/succinctly dev bench dsv
```

---

# Apple M1 Max (ARM, aarch64)

**CPU**: Apple M1 Max
**OS**: macOS
**Delimiter**: `,` (comma-separated values)
**succinctly**: Built with `--release --features cli`

## Pattern: tabular

Standard CSV with uniform column widths.

| Size      | Time     | Throughput   | Memory   |
|-----------|----------|--------------|----------|
| **100mb** |    9.77s |   10.2 MiB/s |    2 GB  |
| **10mb**  |    1.02s |    9.8 MiB/s |  178 MB  |
| **1mb**   |  113.0ms |    8.8 MiB/s |   24 MB  |
| **100kb** |   17.3ms |    5.6 MiB/s |    9 MB  |
| **10kb**  |    6.5ms |    1.5 MiB/s |    7 MB  |
| **1kb**   |    5.7ms |    0.2 MiB/s |    7 MB  |

## Pattern: users

Realistic user records with names, emails, addresses.

| Size      | Time     | Throughput   | Memory   |
|-----------|----------|--------------|----------|
| **100mb** |   10.31s |    9.7 MiB/s |    2 GB  |
| **10mb**  |    1.04s |    9.6 MiB/s |  231 MB  |
| **1mb**   |  113.9ms |    8.8 MiB/s |   30 MB  |
| **100kb** |   16.7ms |    5.9 MiB/s |    9 MB  |
| **10kb**  |    6.9ms |    1.4 MiB/s |    7 MB  |
| **1kb**   |    5.6ms |    0.2 MiB/s |    7 MB  |

## Pattern: numeric

Numeric-heavy CSV (IDs, counts, measurements).

| Size      | Time     | Throughput   | Memory   |
|-----------|----------|--------------|----------|
| **100mb** |   10.22s |    9.8 MiB/s |    2 GB  |
| **10mb**  |    1.03s |    9.7 MiB/s |  164 MB  |
| **1mb**   |  110.2ms |    9.1 MiB/s |   23 MB  |
| **100kb** |   16.5ms |    5.9 MiB/s |    9 MB  |
| **10kb**  |    6.1ms |    1.6 MiB/s |    7 MB  |
| **1kb**   |    5.7ms |    0.2 MiB/s |    7 MB  |

## Pattern: strings

String-heavy CSV with few quoted fields.

| Size      | Time     | Throughput   | Memory   |
|-----------|----------|--------------|----------|
| **100mb** |    3.94s |   25.4 MiB/s |  610 MB  |
| **10mb**  |  396.9ms |   25.2 MiB/s |   68 MB  |
| **1mb**   |   46.2ms |   21.6 MiB/s |   13 MB  |
| **100kb** |    9.6ms |   10.2 MiB/s |    8 MB  |
| **10kb**  |    6.0ms |    1.7 MiB/s |    7 MB  |
| **1kb**   |    5.5ms |    0.2 MiB/s |    7 MB  |

## Pattern: quoted

CSV with many quoted fields containing special characters.

| Size      | Time     | Throughput   | Memory   |
|-----------|----------|--------------|----------|
| **100mb** |    6.70s |   14.9 MiB/s |  861 MB  |
| **10mb**  |  693.9ms |   14.4 MiB/s |   95 MB  |
| **1mb**   |   76.6ms |   13.1 MiB/s |   17 MB  |
| **100kb** |   13.8ms |    7.1 MiB/s |    8 MB  |
| **10kb**  |    7.0ms |    1.4 MiB/s |    7 MB  |
| **1kb**   |    6.1ms |    0.2 MiB/s |    7 MB  |

## Pattern: multiline

CSV with multiline fields (newlines inside quotes).

| Size      | Time     | Throughput   | Memory   |
|-----------|----------|--------------|----------|
| **100mb** |    5.40s |   18.5 MiB/s |  743 MB  |
| **10mb**  |  558.1ms |   17.9 MiB/s |   84 MB  |
| **1mb**   |   65.4ms |   15.3 MiB/s |   15 MB  |
| **100kb** |   12.2ms |    8.0 MiB/s |    8 MB  |
| **10kb**  |    7.0ms |    1.4 MiB/s |    7 MB  |
| **1kb**   |    6.7ms |    0.2 MiB/s |    7 MB  |

## Pattern: wide

Wide tables (many columns per row).

| Size      | Time     | Throughput   | Memory   |
|-----------|----------|--------------|----------|
| **100mb** |   15.34s |    6.5 MiB/s |    3 GB  |
| **10mb**  |    1.52s |    6.6 MiB/s |  350 MB  |
| **1mb**   |  157.4ms |    6.4 MiB/s |   42 MB  |
| **100kb** |   20.3ms |    4.8 MiB/s |   11 MB  |
| **10kb**  |    6.7ms |    1.5 MiB/s |    7 MB  |
| **1kb**   |    6.0ms |    0.2 MiB/s |    7 MB  |

## Pattern: long

Tall tables (many rows, few columns).

| Size      | Time     | Throughput   | Memory   |
|-----------|----------|--------------|----------|
| **100mb** |   16.28s |    6.1 MiB/s |    4 GB  |
| **10mb**  |    1.71s |    5.9 MiB/s |  394 MB  |
| **1mb**   |  190.8ms |    5.2 MiB/s |   49 MB  |
| **100kb** |   25.3ms |    3.9 MiB/s |   11 MB  |
| **10kb**  |    8.1ms |    1.2 MiB/s |    8 MB  |
| **1kb**   |    6.0ms |    0.2 MiB/s |    7 MB  |

## Pattern: mixed

Mixed content with various field types and quoting.

| Size      | Time     | Throughput   | Memory   |
|-----------|----------|--------------|----------|
| **100mb** |   10.05s |    9.9 MiB/s |    2 GB  |
| **10mb**  |    1.03s |    9.7 MiB/s |  187 MB  |
| **1mb**   |  105.8ms |    9.4 MiB/s |   24 MB  |
| **100kb** |   16.7ms |    5.8 MiB/s |    9 MB  |
| **10kb**  |    6.9ms |    1.4 MiB/s |    7 MB  |
| **1kb**   |    5.8ms |    0.2 MiB/s |    7 MB  |

## Pattern: pathological

Worst-case CSV patterns (heavy quoting, escapes, edge cases).

| Size      | Time     | Throughput   | Memory   |
|-----------|----------|--------------|----------|
| **100mb** |    7.45s |   13.4 MiB/s |  911 MB  |
| **10mb**  |  759.8ms |   13.2 MiB/s |  109 MB  |
| **1mb**   |   80.6ms |   12.4 MiB/s |   18 MB  |
| **100kb** |   13.9ms |    7.0 MiB/s |    8 MB  |
| **10kb**  |    6.4ms |    1.5 MiB/s |    7 MB  |
| **1kb**   |    5.6ms |    0.2 MiB/s |    7 MB  |

---

## Summary

### Throughput by Pattern (100MB files)

| Pattern        | Throughput   | Notes                                     |
|----------------|--------------|-------------------------------------------|
| **strings**    |   25.4 MiB/s | Fastest - few quotes, simple fields       |
| **multiline**  |   18.5 MiB/s | Fast despite newlines in fields           |
| **quoted**     |   14.9 MiB/s | Quote handling overhead                   |
| **pathological**|  13.4 MiB/s | Complex edge cases                        |
| **tabular**    |   10.2 MiB/s | Uniform column widths                     |
| **mixed**      |    9.9 MiB/s | Varied content types                      |
| **numeric**    |    9.8 MiB/s | Number-heavy content                      |
| **users**      |    9.7 MiB/s | Realistic user records                    |
| **wide**       |    6.5 MiB/s | Many columns increases overhead           |
| **long**       |    6.1 MiB/s | Many rows increases per-row overhead      |

### Pattern Descriptions

| Pattern         | Description                                           |
|-----------------|-------------------------------------------------------|
| **tabular**     | Standard CSV with uniform column widths               |
| **users**       | Realistic user records (names, emails, addresses)     |
| **numeric**     | Numeric-heavy (IDs, counts, measurements)             |
| **strings**     | String-heavy with minimal quoting                     |
| **quoted**      | Many quoted fields with special characters            |
| **multiline**   | Fields containing newlines (inside quotes)            |
| **wide**        | Wide tables (100+ columns per row)                    |
| **long**        | Tall tables (many rows, 3-5 columns)                  |
| **mixed**       | Mixed content with various field types                |
| **pathological**| Worst-case patterns (heavy escaping, edge cases)      |

### Key Observations

1. **String-heavy data is fastest** (25.4 MiB/s) - Simple fields without quoting have minimal parsing overhead
2. **Quote handling adds overhead** - Quoted patterns run at ~60% the speed of unquoted
3. **Table shape matters**:
   - Wide tables (many columns): 6.5 MiB/s - More field boundaries to track
   - Long tables (many rows): 6.1 MiB/s - More row boundaries to process
4. **Memory scales with file size** - Approximately 20x file size for large files (2GB for 100MB input)

## Reproducing Benchmarks

```bash
# Build release binary
cargo build --release --features cli

# Generate DSV benchmark data
./target/release/succinctly dsv generate-suite

# Run DSV input benchmarks
./target/release/succinctly dev bench dsv
```
