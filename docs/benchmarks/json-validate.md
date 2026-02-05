# JSON Validation Benchmarks

[Home](/) > [Docs](../) > [Benchmarks](./) > JSON Validation

Performance benchmarks for `succinctly json validate` - strict RFC 8259 JSON validation.

## Summary

| Metric         | Apple M4 Pro         | AMD Ryzen 9 7950X (SIMD) |
|----------------|----------------------|--------------------------|
| **Throughput** | 530-1800 MiB/s       | 400 MiB/s - 15.9 GiB/s   |
| **Peak**       | 1.8 GiB/s (nested)   | 15.9 GiB/s (nested)      |
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
- **Date**: 2026-02-05 (commit 06bcba2)

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
| nested       | 235 ns   | 3.69 GiB/s   |
| mixed        | 269 ns   | 293 MiB/s    |
| strings      | 504 ns   | 1.62 GiB/s   |
| unicode      | 680 ns   | 1.43 GiB/s   |
| numbers      | 768 ns   | 1.16 GiB/s   |
| users        | 922 ns   | 551 MiB/s    |
| literals     | 1.35 µs  | 720 MiB/s    |
| arrays       | 1.91 µs  | 515 MiB/s    |
| comprehensive| 2.09 µs  | 756 MiB/s    |
| pathological | 2.45 µs  | 397 MiB/s    |

### 10KB Files

| Pattern      | Time     | Throughput   |
|--------------|----------|--------------|
| nested       | 792 ns   | 11.93 GiB/s  |
| strings      | 1.35 µs  | 6.19 GiB/s   |
| mixed        | 1.49 µs  | 572 MiB/s    |
| unicode      | 4.84 µs  | 1.97 GiB/s   |
| numbers      | 7.47 µs  | 1.18 GiB/s   |
| users        | 8.12 µs  | 715 MiB/s    |
| comprehensive| 12.31 µs | 763 MiB/s    |
| literals     | 13.48 µs | 724 MiB/s    |
| arrays       | 17.55 µs | 557 MiB/s    |
| pathological | 24.23 µs | 404 MiB/s    |

### 100KB Files

| Pattern      | Time     | Throughput   |
|--------------|----------|--------------|
| nested       | 6.17 µs  | 15.44 GiB/s  |
| strings      | 9.80 µs  | 8.55 GiB/s   |
| mixed        | 16.81 µs | 569 MiB/s    |
| unicode      | 46.78 µs | 2.04 GiB/s   |
| numbers      | 75.02 µs | 1.18 GiB/s   |
| users        | 78.32 µs | 767 MiB/s    |
| comprehensive| 118.48 µs| 720 MiB/s    |
| arrays       | 186.17 µs| 525 MiB/s    |
| literals     | 192.40 µs| 508 MiB/s    |
| pathological | 241.99 µs| 404 MiB/s    |

### 1MB Files

| Pattern      | Time     | Throughput   |
|--------------|----------|--------------|
| nested       | 61.5 µs  | 15.87 GiB/s  |
| strings      | 96.9 µs  | 8.85 GiB/s   |
| mixed        | 178 µs   | 598 MiB/s    |
| unicode      | 478 µs   | 2.04 GiB/s   |
| numbers      | 771 µs   | 1.17 GiB/s   |
| users        | 803 µs   | 792 MiB/s    |
| comprehensive| 1.14 ms  | 708 MiB/s    |
| arrays       | 1.91 ms  | 524 MiB/s    |
| literals     | 2.04 ms  | 491 MiB/s    |
| pathological | 2.47 ms  | 405 MiB/s    |

### 10MB Files

| Pattern      | Time     | Throughput   |
|--------------|----------|--------------|
| nested       | 615 µs   | 15.86 GiB/s  |
| strings      | 969 µs   | 8.85 GiB/s   |
| mixed        | 2.18 ms  | 538 MiB/s    |
| unicode      | 4.78 ms  | 2.04 GiB/s   |
| numbers      | 7.70 ms  | 1.17 GiB/s   |
| users        | 8.10 ms  | 810 MiB/s    |
| comprehensive| 11.14 ms | 718 MiB/s    |
| arrays       | 19.08 ms | 524 MiB/s    |
| literals     | 20.54 ms | 487 MiB/s    |
| pathological | 24.77 ms | 404 MiB/s    |

### Performance by Pattern Type (x86_64 with AVX2 SIMD)

| Pattern           | Characteristics                      | Typical Throughput |
|-------------------|--------------------------------------|--------------------|
| **nested**        | Deeply nested objects/arrays         | 11.9-15.9 GiB/s    |
| **strings**       | String-heavy content                 | 1.6-8.9 GiB/s      |
| **unicode**       | Unicode string content               | 1.4-2.0 GiB/s      |
| **numbers**       | Numeric arrays                       | 1.16-1.18 GiB/s    |
| **users**         | Realistic user data                  | 550-810 MiB/s      |
| **comprehensive** | Mixed realistic content              | 708-763 MiB/s      |
| **literals**      | true/false/null heavy                | 487-724 MiB/s      |
| **mixed**         | Small mixed records                  | 293-598 MiB/s      |
| **arrays**        | Large flat arrays                    | 515-557 MiB/s      |
| **pathological**  | Edge cases (escapes, deep nesting)   | 397-405 MiB/s      |

---

## Key Findings

1. **SIMD provides 8-9x speedup on nested structures**: With AVX2 SIMD on x86_64, nested JSON achieves 15.9 GiB/s throughput (vs 1.8 GiB/s scalar). SIMD scans 32 bytes at once for structural characters.

2. **String-heavy content benefits from SIMD scanning**: AVX2 string scanning with Keiser-Lemire UTF-8 validation achieves 8.9 GiB/s on pure strings (vs 1.5 GiB/s scalar), a 5.9x improvement.

3. **Unicode benefits from SIMD**: Unicode patterns see 2.0 GiB/s with SIMD (vs 900 MiB/s scalar), a 2.2x improvement due to faster UTF-8 validation.

4. **Consistent scaling**: Throughput remains stable from 1KB to 10MB files, indicating good cache behavior and minimal overhead.

5. **Literals are slowest**: JSON with many `true`/`false`/`null` values requires keyword matching, limiting SIMD benefits (~500-720 MiB/s).

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
- AMD Ryzen 9 7950X (SIMD): `data/bench/results/20260205_130353_06bcba2/`

## See Also

- [jq Benchmarks](jq.md) - JSON query performance
- [Rust JSON Parsers](rust-parsers.md) - Parser comparison
- [JSON Validation Command](../guides/cli.md#json-validation) - CLI documentation
