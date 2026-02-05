# JSON Validation Benchmarks

[Home](/) > [Docs](../) > [Benchmarks](./) > JSON Validation

Performance benchmarks for `succinctly json validate` - strict RFC 8259 JSON validation.

## Summary

| Metric         | Apple M4 Pro         | AMD Ryzen 9 7950X (SIMD) |
|----------------|----------------------|--------------------------|
| **Throughput** | 530-1800 MiB/s       | 400 MiB/s - 17.3 GiB/s   |
| **Peak**       | 1.8 GiB/s (nested)   | 17.3 GiB/s (nested)      |
| **Patterns**   | 10 (1KB-10MB each)   | 10 (1KB-10MB each)       |

**Note**: x86_64 uses AVX2 SIMD for string scanning with Keiser-Lemire UTF-8 validation. ARM results are scalar. SIMD provides 2-9x improvement on string-heavy patterns.

## Platforms

### Apple M4 Pro (ARM)
- **CPU**: Apple M4 Pro (12 cores)
- **Build**: `cargo build --release --features cli`
- **Date**: 2026-02-04 (commit 16de8f9)

### AMD Ryzen 9 7950X (x86_64) - SIMD Enabled
- **CPU**: AMD Ryzen 9 7950X 16-Core Processor
- **OS**: Ubuntu 22.04.5 LTS
- **Build**: `cargo build --release --features cli`
- **SIMD**: AVX2 string scanning with Keiser-Lemire UTF-8 validation
- **Date**: 2026-02-05 (commit df77c84)

---

## Apple M4 Pro (ARM) Results

### 1KB Files

| Pattern      | Time     | Throughput   |
|--------------|----------|--------------|
| mixed        | 124 ns   | 637 MiB/s    |
| users        | 517 ns   | 984 MiB/s    |
| strings      | 534 ns   | 1.52 GiB/s   |
| nested       | 568 ns   | 1.52 GiB/s   |
| numbers      | 683 ns   | 1.31 GiB/s   |
| literals     | 1.18 µs  | 824 MiB/s    |
| pathological | 1.32 µs  | 736 MiB/s    |
| comprehensive| 1.59 µs  | 994 MiB/s    |
| arrays       | 1.65 µs  | 597 MiB/s    |
| unicode      | 1.01 µs  | 991 MiB/s    |

### 10KB Files

| Pattern      | Time     | Throughput   |
|--------------|----------|--------------|
| mixed        | 1.11 µs  | 769 MiB/s    |
| nested       | 5.43 µs  | 1.74 GiB/s   |
| strings      | 5.18 µs  | 1.62 GiB/s   |
| users        | 5.77 µs  | 1006 MiB/s   |
| numbers      | 6.74 µs  | 1.31 GiB/s   |
| comprehensive| 11.31 µs | 831 MiB/s    |
| pathological | 13.05 µs | 749 MiB/s    |
| literals     | 15.65 µs | 623 MiB/s    |
| arrays       | 16.96 µs | 576 MiB/s    |
| unicode      | 9.75 µs  | 1004 MiB/s   |

### 100KB Files

| Pattern      | Time     | Throughput   |
|--------------|----------|--------------|
| mixed        | 13.4 µs  | 714 MiB/s    |
| nested       | 52.9 µs  | 1.80 GiB/s   |
| strings      | 52.3 µs  | 1.60 GiB/s   |
| users        | 59.2 µs  | 1016 MiB/s   |
| numbers      | 67.6 µs  | 1.31 GiB/s   |
| comprehensive| 109.5 µs | 780 MiB/s    |
| pathological | 134.2 µs | 728 MiB/s    |
| literals     | 179.7 µs | 543 MiB/s    |
| arrays       | 172.7 µs | 566 MiB/s    |
| unicode      | 98.1 µs  | 996 MiB/s    |

### 1MB Files

| Pattern      | Time     | Throughput   |
|--------------|----------|--------------|
| mixed        | 164 µs   | 649 MiB/s    |
| nested       | 540 µs   | 1.81 GiB/s   |
| strings      | 528 µs   | 1.62 GiB/s   |
| users        | 618 µs   | 1.01 GiB/s   |
| numbers      | 685 µs   | 1.32 GiB/s   |
| comprehensive| 1.07 ms  | 757 MiB/s    |
| pathological | 1.35 ms  | 741 MiB/s    |
| literals     | 1.89 ms  | 528 MiB/s    |
| arrays       | 1.77 ms  | 565 MiB/s    |
| unicode      | 1.01 ms  | 989 MiB/s    |

### 10MB Files

| Pattern      | Time     | Throughput   |
|--------------|----------|--------------|
| mixed        | 1.93 ms  | 606 MiB/s    |
| nested       | 5.44 ms  | 1.80 GiB/s   |
| strings      | 5.42 ms  | 1.58 GiB/s   |
| users        | 6.25 ms  | 1.03 GiB/s   |
| numbers      | 6.88 ms  | 1.31 GiB/s   |
| comprehensive| 10.45 ms | 766 MiB/s    |
| pathological | 13.56 ms | 738 MiB/s    |
| literals     | 18.80 ms | 532 MiB/s    |
| arrays       | 17.77 ms | 563 MiB/s    |
| unicode      | 10.12 ms | 989 MiB/s    |

## Performance by Pattern Type

| Pattern           | Characteristics                      | Typical Throughput |
|-------------------|--------------------------------------|--------------------|
| **nested**        | Deeply nested objects/arrays         | 1.7-1.8 GiB/s      |
| **strings**       | String-heavy content                 | 1.5-1.6 GiB/s      |
| **numbers**       | Numeric arrays                       | 1.3 GiB/s          |
| **users**         | Realistic user data                  | 1.0 GiB/s          |
| **unicode**       | Unicode string content               | 990 MiB/s          |
| **comprehensive** | Mixed realistic content              | 760-830 MiB/s      |
| **pathological**  | Edge cases (escapes, deep nesting)   | 730-750 MiB/s      |
| **mixed**         | Small mixed records                  | 600-770 MiB/s      |
| **arrays**        | Large flat arrays                    | 560-600 MiB/s      |
| **literals**      | true/false/null heavy                | 530-820 MiB/s      |

---

## AMD Ryzen 9 7950X (x86_64) Results

### 1KB Files

| Pattern      | Time     | Throughput   |
|--------------|----------|--------------|
| nested       | 226 ns   | 3.82 GiB/s   |
| mixed        | 248 ns   | 319 MiB/s    |
| strings      | 466 ns   | 1.75 GiB/s   |
| unicode      | 686 ns   | 1.42 GiB/s   |
| numbers      | 723 ns   | 1.24 GiB/s   |
| users        | 878 ns   | 579 MiB/s    |
| literals     | 1.32 µs  | 734 MiB/s    |
| arrays       | 1.71 µs  | 575 MiB/s    |
| comprehensive| 1.96 µs  | 803 MiB/s    |
| pathological | 2.34 µs  | 416 MiB/s    |

### 10KB Files

| Pattern      | Time     | Throughput   |
|--------------|----------|--------------|
| nested       | 737 ns   | 12.83 GiB/s  |
| strings      | 1.25 µs  | 6.70 GiB/s   |
| mixed        | 1.48 µs  | 576 MiB/s    |
| unicode      | 5.07 µs  | 1.88 GiB/s   |
| numbers      | 7.04 µs  | 1.26 GiB/s   |
| users        | 7.78 µs  | 746 MiB/s    |
| comprehensive| 12.00 µs | 783 MiB/s    |
| literals     | 13.15 µs | 742 MiB/s    |
| arrays       | 17.25 µs | 566 MiB/s    |
| pathological | 23.35 µs | 419 MiB/s    |

### 100KB Files

| Pattern      | Time     | Throughput   |
|--------------|----------|--------------|
| nested       | 5.67 µs  | 16.81 GiB/s  |
| strings      | 9.04 µs  | 9.27 GiB/s   |
| mixed        | 15.96 µs | 600 MiB/s    |
| unicode      | 49.32 µs | 1.93 GiB/s   |
| numbers      | 69.64 µs | 1.27 GiB/s   |
| users        | 76.34 µs | 787 MiB/s    |
| comprehensive| 114 µs   | 748 MiB/s    |
| arrays       | 172 µs   | 567 MiB/s    |
| literals     | 191 µs   | 511 MiB/s    |
| pathological | 233 µs   | 420 MiB/s    |

### 1MB Files

| Pattern      | Time     | Throughput   |
|--------------|----------|--------------|
| nested       | 57.1 µs  | 17.09 GiB/s  |
| strings      | 89.7 µs  | 9.56 GiB/s   |
| mixed        | 173 µs   | 615 MiB/s    |
| unicode      | 510 µs   | 1.91 GiB/s   |
| numbers      | 717 µs   | 1.26 GiB/s   |
| users        | 783 µs   | 813 MiB/s    |
| comprehensive| 1.11 ms  | 730 MiB/s    |
| arrays       | 1.82 ms  | 550 MiB/s    |
| literals     | 2.00 ms  | 499 MiB/s    |
| pathological | 2.39 ms  | 419 MiB/s    |

### 10MB Files

| Pattern      | Time     | Throughput   |
|--------------|----------|--------------|
| nested       | 565 µs   | 17.27 GiB/s  |
| strings      | 895 µs   | 9.59 GiB/s   |
| mixed        | 2.04 ms  | 575 MiB/s    |
| unicode      | 5.06 ms  | 1.93 GiB/s   |
| numbers      | 7.15 ms  | 1.26 GiB/s   |
| users        | 7.88 ms  | 833 MiB/s    |
| comprehensive| 10.82 ms | 740 MiB/s    |
| arrays       | 18.0 ms  | 556 MiB/s    |
| literals     | 19.9 ms  | 502 MiB/s    |
| pathological | 24.2 ms  | 413 MiB/s    |

### Performance by Pattern Type (x86_64 with AVX2 SIMD)

| Pattern           | Characteristics                      | Typical Throughput |
|-------------------|--------------------------------------|--------------------|
| **nested**        | Deeply nested objects/arrays         | 12.8-17.3 GiB/s    |
| **strings**       | String-heavy content                 | 1.8-9.6 GiB/s      |
| **unicode**       | Unicode string content               | 1.4-1.9 GiB/s      |
| **numbers**       | Numeric arrays                       | 1.24-1.27 GiB/s    |
| **users**         | Realistic user data                  | 579-833 MiB/s      |
| **comprehensive** | Mixed realistic content              | 730-803 MiB/s      |
| **literals**      | true/false/null heavy                | 499-742 MiB/s      |
| **mixed**         | Small mixed records                  | 319-615 MiB/s      |
| **arrays**        | Large flat arrays                    | 550-575 MiB/s      |
| **pathological**  | Edge cases (escapes, deep nesting)   | 413-420 MiB/s      |

---

## Key Findings

1. **SIMD provides 9-10x speedup on nested structures**: With AVX2 SIMD on x86_64, nested JSON achieves 17.3 GiB/s throughput (vs 1.8 GiB/s scalar). SIMD scans 32 bytes at once for structural characters.

2. **String-heavy content benefits from SIMD scanning**: AVX2 string scanning with Keiser-Lemire UTF-8 validation achieves 9.6 GiB/s on pure strings (vs 1.5 GiB/s scalar), a 6.4x improvement.

3. **Unicode benefits from SIMD**: Unicode patterns see 1.9 GiB/s with SIMD (vs 990 MiB/s scalar), a 1.9x improvement due to faster UTF-8 validation.

4. **Consistent scaling**: Throughput remains stable from 1KB to 10MB files, indicating good cache behavior and minimal overhead.

5. **Literals are slowest**: JSON with many `true`/`false`/`null` values requires keyword matching, limiting SIMD benefits (~500-740 MiB/s).

6. **ARM remains scalar**: Apple M4 Pro achieves 1.8 GiB/s peak using scalar code. ARM NEON SIMD not yet implemented for validation.

## Running Benchmarks

```bash
# Build with benchmark runner
cargo build --release --features bench-runner

# Run JSON validation benchmarks
./target/release/succinctly bench run json_validate_bench

# Run with Criterion directly
cargo bench --bench json_validate_bench
```

## Benchmark Data Location

Raw benchmark results:
- Apple M4 Pro: `data/bench/results/20260204_194447_16de8f9/`
- AMD Ryzen 9 7950X (SIMD): `data/bench/results/20260205_235444_df77c84/`

## See Also

- [jq Benchmarks](jq.md) - JSON query performance
- [Rust JSON Parsers](rust-parsers.md) - Parser comparison
- [JSON Validation Command](../guides/cli.md#json-validation) - CLI documentation
