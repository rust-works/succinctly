# yq vs succinctly YAML Comparison Benchmarks

Benchmarks comparing `succinctly yq .` (identity filter) vs `yq .` (Mike Farah's yq v4.48.1) for YAML formatting/printing.

## Platforms

### Platform 1: ARM (AWS Graviton 4 - Neoverse-V2)

**CPU**: ARM Neoverse-V2 (AWS Graviton 4)
**OS**: Linux 6.14.0-1018-aws
**yq version**: v4.48.1 (https://github.com/mikefarah/yq/)
**succinctly**: Built with `--release --features cli`
**SIMD**: NEON (128-bit), SVE2 (128-bit vectors), SVEBITPERM (BDEP/BEXT)

### Platform 2: ARM (AWS Graviton 3 - Neoverse-V1)

**CPU**: ARM Neoverse-V1 (4 cores)
**OS**: Linux 6.14.0-1018-aws
**yq version**: Not installed (succinctly-only benchmarks)
**succinctly**: Built with `--release --features cli`
**SIMD**: NEON (128-bit), SVE (256-bit vectors)

### Platform 4: ARM (Apple Silicon)

**CPU**: Apple M1 Max
**OS**: macOS Darwin 25.1.0
**yq version**: v4.48.1 (https://github.com/mikefarah/yq/)
**succinctly**: Built with `--release --features cli`
**SIMD**: ARM NEON (16 bytes/iteration for string scanning)

### Platform 5: x86_64 (AMD Zen 4)

**CPU**: AMD Ryzen 9 7950X (Zen 4)
**OS**: Linux 6.6.87.2-microsoft-standard-WSL2 (WSL2)
**succinctly**: Built with `--release --features cli` and `-C target-cpu=native`
**SIMD**: SSE2/AVX2 (16/32 bytes/iteration for string scanning)
**Optimizations**: P0+ SIMD optimizations (multi-character classification + hybrid scalar/SIMD integration)

## Methodology

Benchmarks measure wall time for the identity filter (`.`) which parses and re-emits the YAML document. This exercises the full parsing pipeline.

Run with:
```bash
cargo bench --bench yq_comparison
```

---

## Summary Results

### ARM (Neoverse-V2 / Graviton 4) - yq Identity Comparison

| Size      | succinctly             | yq                     | Speedup    |
|-----------|------------------------|------------------------|------------|
| **10KB**  | 0.99 ms (9.9 MiB/s)    | 4.46 ms (2.2 MiB/s)    | **4.5x**   |
| **100KB** | 2.20 ms (41.8 MiB/s)   | 20.1 ms (4.6 MiB/s)    | **9.1x**   |
| **1MB**   | 13.6 ms (68.0 MiB/s)   | 154 ms (6.0 MiB/s)     | **11.4x**  |

**Key metrics:**
- ✅ NEON SIMD optimizations active (block scalar + anchor parsing)
- ✅ SVE2 with SVEBITPERM support (BDEP/BEXT for DSV)
- ✅ **11.4x faster** than yq on 1MB files
- ✅ 68 MiB/s throughput vs yq's 6 MiB/s
- ✅ ~20% faster than Neoverse-V1 due to improved microarchitecture

### ARM (Neoverse-V1 / Graviton 3) - succinctly yq Performance

Note: System `yq` was not installed for comparison. Results show succinctly performance only.

| Size      | succinctly Time | Throughput     |
|-----------|-----------------|----------------|
| **1KB**   | 1.11 ms         | 1.09 MiB/s     |
| **10KB**  | 1.27 ms         | 7.71 MiB/s     |
| **100KB** | 2.87 ms         | 32.0 MiB/s     |
| **1MB**   | 17.71 ms        | 52.1 MiB/s     |

**Key metrics:**
- ✅ NEON SIMD optimizations active
- ✅ 52 MiB/s throughput on 1MB files
- ✅ Scales well: 1MB throughput 47x higher than 1KB
- ✅ SVE-capable CPU (256-bit vectors)

### ARM (Apple M1 Max) - yq Identity Comparison (comprehensive pattern)

| Size      | succinctly   | yq           | Speedup       |
|-----------|--------------|--------------|---------------|
| **10KB**  |   4.2 ms     |   8.4 ms     | **2.0x**      |
| **100KB** |   5.5 ms     |  20.6 ms     | **3.8x**      |
| **1MB**   |  15.3 ms     | 120.7 ms     | **7.9x**      |

#### Throughput Comparison (ARM)

| Size      | succinctly      | yq             | Ratio         |
|-----------|-----------------|----------------|---------------|
| **10KB**  |   2.3 MiB/s     |   1.2 MiB/s    | **2.0x**      |
| **100KB** |  16.8 MiB/s     |   4.5 MiB/s    | **3.8x**      |
| **1MB**   |  60.2 MiB/s     |   7.6 MiB/s    | **7.9x**      |

### x86_64 (AMD Ryzen 9 7950X) - yq Identity Comparison

| Size      | succinctly             | yq                     | Speedup    |
|-----------|------------------------|------------------------|------------|
| **10KB**  | 1.63 ms (6.0 MiB/s)    | 64.6 ms (155 KiB/s)    | **40x**    |
| **100KB** | 2.78 ms (33.1 MiB/s)   | 79.6 ms (1.2 MiB/s)    | **29x**    |
| **1MB**   | 13.2 ms (69.7 MiB/s)   | 210.5 ms (4.4 MiB/s)   | **16x**    |

### x86_64 (AMD Ryzen 9 7950X) - Key Achievements

**Key Achievements:**
- ✅ **Full yq CLI compatibility** - quoted strings preserved as strings
- ✅ **40x faster than yq** on 10KB files (end-to-end CLI comparison)
- ✅ **16x faster than yq** on 1MB files
- ✅ **Comprehensive test suite** - 32 tests including 8 direct byte-for-byte comparisons

**See also:** [docs/parsing/yaml.md](../parsing/yaml.md) for optimization details.

---

## Detailed Results by Pattern (ARM - Apple M1 Max)

### Pattern: comprehensive

Mixed YAML content with various features.

| Size      | succinctly             | yq                     | Speedup    |
|-----------|------------------------|------------------------|------------|
| **1KB**   |   3.5 ms (350 KiB/s)   |   6.3 ms (197 KiB/s)   | **1.8x**   |
| **10KB**  |   3.9 ms (2.5 MiB/s)   |   7.5 ms (1.3 MiB/s)   | **1.9x**   |
| **100KB** |   4.7 ms (19.5 MiB/s)  |  19.5 ms (4.7 MiB/s)   | **4.1x**   |
| **1MB**   |  13.3 ms (69.2 MiB/s)  | 115.7 ms (8.0 MiB/s)   | **8.7x**   |

### Pattern: users

Realistic user record arrays (common in config files).

| Size      | succinctly             | yq                     | Speedup    |
|-----------|------------------------|------------------------|------------|
| **1KB**   |   3.5 ms (303 KiB/s)   |   6.6 ms (163 KiB/s)   | **1.9x**   |
| **10KB**  |   4.2 ms (2.3 MiB/s)   |   7.5 ms (1.3 MiB/s)   | **1.8x**   |
| **100KB** |   9.6 ms (10.2 MiB/s)  |  21.0 ms (4.6 MiB/s)   | **2.2x**   |
| **1MB**   |  66.0 ms (15.2 MiB/s)  | 141.4 ms (7.1 MiB/s)   | **2.1x**   |

### Pattern: nested

Deeply nested mapping structures.

| Size      | succinctly             | yq                     | Speedup    |
|-----------|------------------------|------------------------|------------|
| **1KB**   |   3.6 ms (282 KiB/s)   |   6.3 ms (160 KiB/s)   | **1.8x**   |
| **10KB**  |   3.9 ms (1.8 MiB/s)   |   7.2 ms (1.0 MiB/s)   | **1.8x**   |
| **100KB** |   6.0 ms (8.3 MiB/s)   |  14.4 ms (3.5 MiB/s)   | **2.4x**   |
| **1MB**   |  28.8 ms (21.9 MiB/s)  |  92.1 ms (6.8 MiB/s)   | **3.2x**   |

### Pattern: sequences

Sequence-heavy YAML content.

| Size      | succinctly             | yq                     | Speedup    |
|-----------|------------------------|------------------------|------------|
| **1KB**   |   3.6 ms (284 KiB/s)   |   6.3 ms (162 KiB/s)   | **1.8x**   |
| **10KB**  |   4.2 ms (2.3 MiB/s)   |   7.5 ms (1.3 MiB/s)   | **1.8x**   |
| **100KB** |   9.1 ms (10.7 MiB/s)  |  20.6 ms (4.7 MiB/s)   | **2.3x**   |
| **1MB**   |  56.7 ms (17.6 MiB/s)  | 137.8 ms (7.3 MiB/s)   | **2.4x**   |

### Pattern: strings

String-heavy YAML with quoted variants.

| Size      | succinctly             | yq                     | Speedup    |
|-----------|------------------------|------------------------|------------|
| **1KB**   |   3.5 ms (287 KiB/s)   |   6.2 ms (162 KiB/s)   | **1.8x**   |
| **10KB**  |   4.1 ms (2.4 MiB/s)   |   6.6 ms (1.5 MiB/s)   | **1.6x**   |
| **100KB** |   8.2 ms (11.9 MiB/s)  |  13.7 ms (7.1 MiB/s)   | **1.7x**   |
| **1MB**   |  49.1 ms (20.4 MiB/s)  |  79.2 ms (12.6 MiB/s)  | **1.6x**   |

---

## Pattern Descriptions

| Pattern           | Description                                    |
|-------------------|------------------------------------------------|
| **comprehensive** | Mixed content with various YAML features       |
| **users**         | Realistic user record objects (config-like)    |
| **nested**        | Deeply nested mappings (tests tree navigation) |
| **sequences**     | Sequence-heavy content (arrays)                |
| **strings**       | String-heavy with quoted variants              |

---

## Key Findings

### Speed

- **1.6-3.2x faster** across all patterns and sizes
- **Best performance on nested structures**: 3.2x speedup on 1MB deeply nested files
- **Nested structures**: Efficient BP tree navigation provides consistent speedups
- **Larger files benefit more**: Speedup increases with file size (amortizes index construction)

### Why succinctly is faster

1. **Semi-index architecture**: YAML structure is pre-indexed using balanced parentheses, enabling O(1) navigation
2. **Direct YAML→JSON transcoding**: Escape sequences are translated directly without intermediate string allocation
3. **SIMD string scanning**: ARM NEON accelerates quoted string parsing (6-9% faster than scalar)
4. **SIMD indentation counting**: Vectorized space counting for block-style parsing (10-24% faster on small files)
5. **Streaming output**: For identity queries, outputs directly from source without building intermediate structures
6. **Lazy evaluation**: Only materializes values that are actually accessed

### SIMD Optimizations

The YAML parser uses platform-specific SIMD for hot paths:

| Platform | Instruction Set | Width            | Operations                                    | Status |
|----------|-----------------|------------------|-----------------------------------------------|--------|
| ARM64    | NEON            | 16 bytes/iter    | String scanning, indentation count            | ✓ |
| x86_64   | SSE2/AVX2       | 16-32 bytes/iter | String scanning, indentation, multi-char classification | ✓ P0 |

#### ARM64 (NEON) Results

**String scanning** (double-quoted strings, 1000 entries):
- **Scalar**: 67.1µs @ 688 MiB/s
- **SIMD**:   63.0µs @ 738 MiB/s (+7.3% throughput)

**Indentation scanning** (end-to-end improvement on yq identity):
- **1KB files**: 14-24% faster
- **10KB files**: 10-18% faster
- **100KB+ files**: 5-8% faster (other costs dominate)

#### x86_64 (AVX2) P2 Optimized Results (2026-01-17)

**P2 Integration of `classify_yaml_chars`:**
- Uses SIMD to scan 32 bytes at once for unquoted value/key terminators
- Conditional activation: only uses SIMD for values ≥32 bytes remaining
- Avoids overhead for typical short YAML values (<32 bytes)

**Unquoted value/key parsing improvements:**
- simple_kv/1000: **-12% faster** (41.9µs → 36.8µs)
- simple_kv/10000: **-11% faster** (418µs → 371µs)
- large/100kb: **-11% faster** (222µs → 198µs)
- large/1mb: **-17% faster** (2.18ms → 1.82ms)

**End-to-end yq comparison (x86_64):**
- 10KB files: **40x faster** than yq (1.6ms vs 64.6ms)
- 100KB files: **29x faster** than yq (2.8ms vs 79.6ms)
- 1MB files: **16x faster** than yq (13.2ms vs 210.5ms)

**Optimizations (cumulative):**
- **P0**: Multi-character classification infrastructure (8 types in parallel)
- **P0+**: Hybrid scalar/SIMD space skipping (fast path for 0-8 spaces, SIMD for longer runs)
- **P2**: Conditional SIMD for unquoted value/key scanning (32-byte chunks via `classify_yaml_chars`)
- **P2.7**: Block scalar SIMD - AVX2 newline scanning + indentation checking (19-25% improvement)
- **P4**: Anchor/Alias SIMD - AVX2 scans for anchor name terminators (6-17% improvement on anchor-heavy workloads)
- **P9**: Direct YAML-to-JSON streaming - eliminated intermediate DOM for identity queries (8-22% improvement, 2.3x on yq benchmarks)

**Rejected Optimizations:**
- **P1 (YFSM)**: Table-driven state machine for string parsing tested but showed only 0-2% improvement vs expected 15-25%. YAML strings are too simple compared to JSON (where PFSM succeeded with 33-77% gains). P0+ SIMD already optimal. See [docs/parsing/yaml.md](../parsing/yaml.md#p1-yfsm-yaml-finite-state-machine---rejected-) for full analysis.
- **P2.6, P2.8, P3, P5, P6, P7, P8**: Various optimizations rejected due to micro-benchmark/real-world mismatches, grammar incompatibilities, or memory bottlenecks. See [docs/parsing/yaml.md](../parsing/yaml.md) for detailed analyses.

### Trade-offs

- **Small files (<1KB)**: Process startup dominates; speedup is modest (~1.7x)
- **Large files (1MB+)**: Index construction amortizes; best speedups (~2x)
- **Memory**: succinctly uses more memory for very small files due to index overhead, but less for large files

---

## Selection Benchmarks (Lazy Evaluation)

The `yq_select` benchmark demonstrates lazy evaluation benefits when selecting partial data (~5% of input).

### Query Types

**5% Slice Selection** - Extract first ~5% of array elements:
- `.users[:4]` (4 of 75 users from 10KB)
- `.users[:37]` (37 of 742 users from 100KB)
- `.users[:377]` (377 of 7540 users from 1MB)

**Single Field Extraction** - Navigate to specific elements:
- `.users[0].name` (first user)
- `.users[3770]` (middle of 1MB file)
- `.users[7000]` (near end of 1MB file)

### Why Selection is Faster

With lazy evaluation, succinctly only materializes the requested subset:
1. **Semi-index navigation**: O(1) skip to array element N using BP tree
2. **Partial parsing**: Only parses values that are output
3. **No DOM construction**: Streams directly from source bytes

Traditional parsers (like yq) must parse the entire document before selection.

Run with:
```bash
cargo bench --bench yq_select
```

---

## Compatibility with yq

### Drop-in Replacement Status

**`succinctly yq` is now yq-compatible** for YAML scalar type preservation. Quoted strings are preserved as strings, and unquoted scalars undergo YAML type detection.

### Type Preservation Behavior

`succinctly yq` correctly implements YAML 1.2 type preservation:

**Example**:
```yaml
# Source YAML
version: "1.0"   # Quoted string
id: "001"        # Quoted string
number: 123      # Unquoted number
code: "007"      # Quoted string
```

**Output (both `yq` and `succinctly yq`):**
```json
{
  "version": "1.0",
  "id": "001",
  "number": 123,
  "code": "007"
}
```

**Behavior**:
- **Quoted strings** (`"1.0"`, `"001"`) are always output as JSON strings
- **Unquoted scalars** (`123`, `true`, `null`) undergo type detection per YAML 1.2 spec
- **Preserves semantic meaning** of the original YAML document

### Compatible Query Patterns

✓ **All standard yq query patterns produce identical output**:

| Pattern | Example | Notes |
|---------|---------|-------|
| Identity | `.` | Full document output |
| Field selection | `.users[].name` | String and number fields |
| Filtering | `.users[] \| select(.age > 30)` | Comparison operations |
| Boolean selection | `.users[] \| select(.active)` | Boolean values |
| Array indexing | `.users[0]` | Array element access |
| Nested paths | `.config.version` | Preserves quoted strings |
| Number fields | `.scores.total` | Unquoted numbers |
| Quoted numbers | `.id` | Preserves `"001"` as string |

### Migration from yq

`succinctly yq` can be used as a drop-in replacement for `yq` in most scenarios:

```bash
# These produce identical output:
yq -o json '.' file.yaml
succinctly yq -o json '.' file.yaml

# Version strings preserved correctly
$ echo 'version: "1.0"' | succinctly yq -o json '.'
{"version":"1.0"}

# Leading-zero IDs preserved
$ echo 'id: "001"' | succinctly yq -o json '.'
{"id":"001"}
```

### Performance Benefits

`succinctly yq` offers significant performance advantages over `yq`:
- **7.9x faster** on 1MB files (Apple M1 Max)
- **3.8x faster** on 100KB files (Apple M1 Max)
- **16x faster** on 1MB files (AMD Ryzen 9 7950X)
- **40x faster** on 10KB files (AMD Ryzen 9 7950X)
- **Lower memory usage** on large files
- **Streaming architecture** for better scalability
- **Full yq compatibility** for type preservation
---

## Reproducing Benchmarks

```bash
# Build release binary
cargo build --release --features cli

# Generate benchmark files
cargo run --release --features cli -- yaml generate-suite

# Run identity comparison benchmarks
cargo bench --bench yq_comparison

# Run selection/lazy evaluation benchmarks
cargo bench --bench yq_select

# Or use the CLI for manual testing
./target/release/succinctly yq -o json -I 0 . input.yaml
time yq -o json -I 0 . input.yaml
```
