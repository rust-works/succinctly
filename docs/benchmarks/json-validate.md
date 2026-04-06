# JSON Validation Benchmarks

[Home](../../) > [Docs](../) > [Benchmarks](./) > JSON Validation

Performance benchmarks for `succinctly json validate` - strict RFC 8259 JSON validation.

## Summary

| Metric         | Apple M4 Pro         | AMD Ryzen 9 7950X    |
|----------------|----------------------|----------------------|
| **Throughput** | 530-1800 MiB/s       | 466-1810 MiB/s       |
| **Peak**       | 1.8 GiB/s (nested)   | 1.81 GiB/s (nested)  |
| **Patterns**   | 10 (1KB-10MB each)   | 10 (1KB-10MB each)   |

**Note**: The validator is purely scalar (no SIMD). High throughput comes from efficient recursive descent parsing with good branch prediction.

## Platforms

### Apple M4 Pro (ARM)
- **CPU**: Apple M4 Pro (12 cores)
- **Build**: `cargo build --release --features cli`
- **Date**: 2026-02-04 (commit 16de8f9)

### AMD Ryzen 9 7950X (x86_64)
- **CPU**: AMD Ryzen 9 7950X 16-Core Processor
- **OS**: Ubuntu 22.04.5 LTS
- **Build**: `cargo build --release --features cli`
- **Date**: 2026-02-04 (commit a7f6f47)

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
| mixed        | 156 ns   | 507 MiB/s    |
| strings      | 570 ns   | 1.43 GiB/s   |
| nested       | 576 ns   | 1.50 GiB/s   |
| users        | 602 ns   | 844 MiB/s    |
| numbers      | 762 ns   | 1.17 GiB/s   |
| unicode      | 1.13 µs  | 885 MiB/s    |
| literals     | 1.38 µs  | 704 MiB/s    |
| arrays       | 1.86 µs  | 530 MiB/s    |
| pathological | 1.87 µs  | 521 MiB/s    |
| comprehensive| 2.00 µs  | 788 MiB/s    |

### 10KB Files

| Pattern      | Time     | Throughput   |
|--------------|----------|--------------|
| mixed        | 1.31 µs  | 652 MiB/s    |
| nested       | 5.33 µs  | 1.77 GiB/s   |
| strings      | 5.50 µs  | 1.52 GiB/s   |
| users        | 6.67 µs  | 871 MiB/s    |
| numbers      | 7.48 µs  | 1.18 GiB/s   |
| unicode      | 10.86 µs | 902 MiB/s    |
| comprehensive| 12.94 µs | 726 MiB/s    |
| literals     | 13.82 µs | 706 MiB/s    |
| pathological | 18.67 µs | 523 MiB/s    |
| arrays       | 18.72 µs | 522 MiB/s    |

### 100KB Files

| Pattern      | Time     | Throughput   |
|--------------|----------|--------------|
| mixed        | 15.5 µs  | 617 MiB/s    |
| nested       | 52.8 µs  | 1.81 GiB/s   |
| strings      | 53.8 µs  | 1.56 GiB/s   |
| users        | 66.4 µs  | 905 MiB/s    |
| numbers      | 75.4 µs  | 1.17 GiB/s   |
| unicode      | 108 µs   | 904 MiB/s    |
| comprehensive| 125 µs   | 684 MiB/s    |
| pathological | 187 µs   | 522 MiB/s    |
| arrays       | 189 µs   | 517 MiB/s    |
| literals     | 201 µs   | 485 MiB/s    |

### 1MB Files

| Pattern      | Time     | Throughput   |
|--------------|----------|--------------|
| mixed        | 167 µs   | 637 MiB/s    |
| nested       | 541 µs   | 1.81 GiB/s   |
| strings      | 550 µs   | 1.56 GiB/s   |
| users        | 693 µs   | 917 MiB/s    |
| numbers      | 776 µs   | 1.17 GiB/s   |
| unicode      | 1.10 ms  | 906 MiB/s    |
| comprehensive| 1.25 ms  | 649 MiB/s    |
| pathological | 1.91 ms  | 523 MiB/s    |
| arrays       | 1.97 ms  | 508 MiB/s    |
| literals     | 2.15 ms  | 466 MiB/s    |

### 10MB Files

| Pattern      | Time     | Throughput   |
|--------------|----------|--------------|
| mixed        | 2.08 ms  | 562 MiB/s    |
| nested       | 5.43 ms  | 1.80 GiB/s   |
| strings      | 5.51 ms  | 1.56 GiB/s   |
| users        | 7.06 ms  | 929 MiB/s    |
| numbers      | 7.75 ms  | 1.17 GiB/s   |
| unicode      | 11.05 ms | 905 MiB/s    |
| comprehensive| 12.04 ms | 665 MiB/s    |
| pathological | 18.56 ms | 539 MiB/s    |
| arrays       | 19.91 ms | 502 MiB/s    |
| literals     | 21.45 ms | 466 MiB/s    |

### Performance by Pattern Type (x86_64)

| Pattern           | Characteristics                      | Typical Throughput |
|-------------------|--------------------------------------|--------------------|
| **nested**        | Deeply nested objects/arrays         | 1.5-1.8 GiB/s      |
| **strings**       | String-heavy content                 | 1.4-1.6 GiB/s      |
| **numbers**       | Numeric arrays                       | 1.17 GiB/s         |
| **users**         | Realistic user data                  | 840-930 MiB/s      |
| **unicode**       | Unicode string content               | 885-905 MiB/s      |
| **comprehensive** | Mixed realistic content              | 650-790 MiB/s      |
| **literals**      | true/false/null heavy                | 466-706 MiB/s      |
| **mixed**         | Small mixed records                  | 510-650 MiB/s      |
| **pathological**  | Edge cases (escapes, deep nesting)   | 520-540 MiB/s      |
| **arrays**        | Large flat arrays                    | 500-530 MiB/s      |

---

## Key Findings

1. **Nested structures are fastest**: The validator excels at deeply nested JSON, achieving 1.8 GiB/s throughput. Simple structural characters (`{`, `}`, `[`, `]`) are validated quickly.

2. **String-heavy content is fast**: At 1.4-1.6 GiB/s, string validation is efficient despite checking escape sequences, surrogate pairs, and control characters.

3. **Consistent scaling**: Throughput remains stable from 1KB to 10MB files on both platforms, indicating good cache behavior and minimal overhead.

4. **Literals are slowest**: JSON with many `true`/`false`/`null` values requires keyword matching, reducing throughput to ~466-820 MiB/s.

5. **Cross-platform consistency**: ARM and x86_64 achieve comparable peak throughput (~1.8 GiB/s). The validator is purely scalar (no SIMD) - performance comes from efficient branch prediction and cache-friendly sequential access.

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
- AMD Ryzen 9 7950X: `data/bench/results/20260204_211039_a7f6f47/`

## See Also

- [jq Benchmarks](jq.md) - JSON query performance
- [Rust JSON Parsers](rust-parsers.md) - Parser comparison
- [JSON Validation Command](../guides/cli.md#json-validation) - CLI documentation
