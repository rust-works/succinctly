//! Micro-benchmarks for YAML to JSON transcoding performance.
//!
//! Measures the direct YAML→JSON transcoding path via `to_json_document()`.
//! This benchmarks the optimized single-pass transcoding that converts YAML
//! escape sequences directly to JSON escape sequences without intermediate
//! string allocation.
//!
//! Issue #87: Also includes micro-benchmarks for the SIMD escape scanning
//! optimization that accelerates `write_json_string`.

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use succinctly::yaml::simd::find_json_escape;
use succinctly::yaml::YamlIndex;

/// Helper to generate double-quoted YAML string with escapes
fn make_double_quoted_with_escapes(count: usize) -> Vec<u8> {
    let mut yaml = Vec::with_capacity(count * 20);
    yaml.extend_from_slice(b"value: \"");
    for i in 0..count {
        match i % 10 {
            0 => yaml.extend_from_slice(b"hello\\nworld"), // newline escape
            1 => yaml.extend_from_slice(b"tab\\there"),    // tab escape
            2 => yaml.extend_from_slice(b"quote\\\"here"), // quote escape
            3 => yaml.extend_from_slice(b"back\\\\slash"), // backslash
            4 => yaml.extend_from_slice(b"bell\\a"),       // bell
            5 => yaml.extend_from_slice(b"hex\\x41"),      // hex escape
            6 => yaml.extend_from_slice(b"unicode\\u0041"), // unicode 4-digit
            7 => yaml.extend_from_slice(b"space\\ here"),  // escaped space
            8 => yaml.extend_from_slice(b"return\\r"),     // CR
            9 => yaml.extend_from_slice(b"null\\0"),       // null
            _ => unreachable!(),
        }
        if i + 1 < count {
            yaml.push(b' ');
        }
    }
    yaml.extend_from_slice(b"\"\n");
    yaml
}

/// Helper to generate single-quoted YAML string with escapes
fn make_single_quoted_with_escapes(count: usize) -> Vec<u8> {
    let mut yaml = Vec::with_capacity(count * 20);
    yaml.extend_from_slice(b"value: '");
    for i in 0..count {
        match i % 4 {
            0 => yaml.extend_from_slice(b"it''s escaped"), // escaped single quote
            1 => yaml.extend_from_slice(b"has \"double\""), // double quote (needs JSON escape)
            2 => yaml.extend_from_slice(b"back\\slash"),   // backslash (needs JSON escape)
            3 => yaml.extend_from_slice(b"normal text"),   // plain text
            _ => unreachable!(),
        }
        if i + 1 < count {
            yaml.push(b' ');
        }
    }
    yaml.extend_from_slice(b"'\n");
    yaml
}

/// Helper to generate multiline double-quoted string with line folding
fn make_multiline_double_quoted(lines: usize) -> Vec<u8> {
    let mut yaml = Vec::with_capacity(lines * 50);
    yaml.extend_from_slice(b"value: \"line one\n");
    for i in 1..lines {
        yaml.extend_from_slice(format!("  line {} continues\n", i).as_bytes());
    }
    // Replace last newline with closing quote
    yaml.pop();
    yaml.extend_from_slice(b"\"\n");
    yaml
}

/// Helper to generate 8-digit unicode escapes (e.g., emoji)
fn make_8digit_unicode(count: usize) -> Vec<u8> {
    let mut yaml = Vec::with_capacity(count * 15);
    yaml.extend_from_slice(b"value: \"");
    for _ in 0..count {
        yaml.extend_from_slice(b"\\U0001F600"); // 😀
    }
    yaml.extend_from_slice(b"\"\n");
    yaml
}

/// Benchmark double-quoted strings with various escape sequences
fn bench_double_quoted(c: &mut Criterion) {
    let mut group = c.benchmark_group("transcode/double_quoted");

    for &count in &[10, 50, 100, 500] {
        let yaml = make_double_quoted_with_escapes(count);
        let index = YamlIndex::build(&yaml).unwrap();

        group.throughput(Throughput::Bytes(yaml.len() as u64));

        group.bench_with_input(
            BenchmarkId::from_parameter(count),
            &(&yaml, &index),
            |b, (yaml, index)| {
                b.iter(|| {
                    let cursor = index.root(black_box(*yaml));
                    let result = cursor.to_json_document();
                    black_box(result)
                })
            },
        );
    }

    group.finish();
}

/// Benchmark single-quoted strings
fn bench_single_quoted(c: &mut Criterion) {
    let mut group = c.benchmark_group("transcode/single_quoted");

    for &count in &[10, 50, 100, 500] {
        let yaml = make_single_quoted_with_escapes(count);
        let index = YamlIndex::build(&yaml).unwrap();

        group.throughput(Throughput::Bytes(yaml.len() as u64));

        group.bench_with_input(
            BenchmarkId::from_parameter(count),
            &(&yaml, &index),
            |b, (yaml, index)| {
                b.iter(|| {
                    let cursor = index.root(black_box(*yaml));
                    let result = cursor.to_json_document();
                    black_box(result)
                })
            },
        );
    }

    group.finish();
}

/// Benchmark multiline strings with line folding
fn bench_multiline(c: &mut Criterion) {
    let mut group = c.benchmark_group("transcode/multiline");

    for &lines in &[5, 20, 50, 100] {
        let yaml = make_multiline_double_quoted(lines);
        let index = YamlIndex::build(&yaml).unwrap();

        group.throughput(Throughput::Bytes(yaml.len() as u64));

        group.bench_with_input(
            BenchmarkId::from_parameter(lines),
            &(&yaml, &index),
            |b, (yaml, index)| {
                b.iter(|| {
                    let cursor = index.root(black_box(*yaml));
                    let result = cursor.to_json_document();
                    black_box(result)
                })
            },
        );
    }

    group.finish();
}

/// Benchmark 8-digit unicode escapes (surrogate pairs in JSON)
fn bench_unicode_8digit(c: &mut Criterion) {
    let mut group = c.benchmark_group("transcode/unicode_8digit");

    for &count in &[10, 50, 100] {
        let yaml = make_8digit_unicode(count);
        let index = YamlIndex::build(&yaml).unwrap();

        group.throughput(Throughput::Bytes(yaml.len() as u64));

        group.bench_with_input(
            BenchmarkId::from_parameter(count),
            &(&yaml, &index),
            |b, (yaml, index)| {
                b.iter(|| {
                    let cursor = index.root(black_box(*yaml));
                    let result = cursor.to_json_document();
                    black_box(result)
                })
            },
        );
    }

    group.finish();
}

/// Realistic YAML document benchmark
fn bench_realistic_document(c: &mut Criterion) {
    let mut group = c.benchmark_group("transcode/realistic");

    // Config-like YAML with many quoted strings
    let config_yaml = br#"database:
  host: "localhost"
  port: 5432
  username: "admin"
  password: "super\"secret\\password"
  connection_string: "postgres://admin:pass@localhost:5432/db"
logging:
  level: "info"
  format: "%(asctime)s - %(name)s - %(levelname)s - %(message)s"
  file: "/var/log/app.log"
features:
  - name: "feature_one"
    enabled: true
    description: "First feature with\nnewlines"
  - name: "feature_two"
    enabled: false
    description: 'Single quoted with ''escaped'' quotes'
"#;

    let index = YamlIndex::build(config_yaml).unwrap();
    group.throughput(Throughput::Bytes(config_yaml.len() as u64));

    group.bench_function("config", |b| {
        b.iter(|| {
            let cursor = index.root(black_box(config_yaml));
            let result = cursor.to_json_document();
            black_box(result)
        })
    });

    // YAML with many escape sequences
    let escape_heavy = br#"escapes:
  newlines: "line1\nline2\nline3"
  tabs: "col1\tcol2\tcol3"
  quotes: "he said \"hello\""
  backslash: "path\\to\\file"
  mixed: "tab\there\nnewline\there\\backslash\"quote"
  unicode: "emoji\U0001F600and\u00A0nbsp"
  hex: "byte\x41value"
  all_escapes: "\0\a\b\t\n\v\f\r\e\ \_\N\L\P"
"#;

    let escape_index = YamlIndex::build(escape_heavy).unwrap();
    group.throughput(Throughput::Bytes(escape_heavy.len() as u64));

    group.bench_function("escape_heavy", |b| {
        b.iter(|| {
            let cursor = escape_index.root(black_box(escape_heavy));
            let result = cursor.to_json_document();
            black_box(result)
        })
    });

    group.finish();
}

/// Large document with many strings
fn bench_large_document(c: &mut Criterion) {
    let mut group = c.benchmark_group("transcode/large");

    // Generate large YAML with many escape sequences
    let mut large_yaml = Vec::with_capacity(100_000);
    large_yaml.extend_from_slice(b"items:\n");
    for i in 0..500 {
        large_yaml.extend_from_slice(
            format!(
                "  - id: {}\n    name: \"Item {} with escape\\nand tab\\t\"\n    desc: 'Single ''quoted'' text'\n",
                i, i
            )
            .as_bytes(),
        );
    }

    let index = YamlIndex::build(&large_yaml).unwrap();
    group.throughput(Throughput::Bytes(large_yaml.len() as u64));

    group.bench_function("500_items", |b| {
        b.iter(|| {
            let cursor = index.root(black_box(&large_yaml));
            let result = cursor.to_json_document();
            black_box(result)
        })
    });

    group.finish();
}

// ============================================================================
// Issue #87: SIMD Escape Scanning Micro-benchmarks
// ============================================================================

/// Scalar reference implementation for comparison
fn find_json_escape_scalar(bytes: &[u8], start: usize) -> usize {
    for (i, &b) in bytes[start..].iter().enumerate() {
        if b == b'"' || b == b'\\' || b < 0x20 {
            return start + i;
        }
    }
    bytes.len()
}

/// Benchmark escape scanning with no escape characters (worst case for SIMD - scans entire string)
fn bench_escape_scan_no_escapes(c: &mut Criterion) {
    let mut group = c.benchmark_group("escape_scan/no_escapes");

    for &size in &[16, 32, 64, 128, 256, 512, 1024] {
        // String with no characters needing escape
        let data = vec![b'x'; size];
        group.throughput(Throughput::Bytes(size as u64));

        group.bench_with_input(BenchmarkId::new("simd", size), &data, |b, data| {
            b.iter(|| find_json_escape(black_box(data), 0))
        });

        group.bench_with_input(BenchmarkId::new("scalar", size), &data, |b, data| {
            b.iter(|| find_json_escape_scalar(black_box(data), 0))
        });
    }

    group.finish();
}

/// Benchmark escape scanning with early escape (best case - quick exit)
fn bench_escape_scan_early_escape(c: &mut Criterion) {
    let mut group = c.benchmark_group("escape_scan/early_escape");

    for &size in &[16, 32, 64, 128, 256, 512, 1024] {
        // Escape at position 8 (within first SIMD chunk)
        let mut data = vec![b'x'; size];
        data[8] = b'"';
        group.throughput(Throughput::Bytes(size as u64));

        group.bench_with_input(BenchmarkId::new("simd", size), &data, |b, data| {
            b.iter(|| find_json_escape(black_box(data), 0))
        });

        group.bench_with_input(BenchmarkId::new("scalar", size), &data, |b, data| {
            b.iter(|| find_json_escape_scalar(black_box(data), 0))
        });
    }

    group.finish();
}

/// Benchmark escape scanning with escape at middle
fn bench_escape_scan_mid_escape(c: &mut Criterion) {
    let mut group = c.benchmark_group("escape_scan/mid_escape");

    for &size in &[32, 64, 128, 256, 512, 1024] {
        // Escape at middle of string
        let mut data = vec![b'x'; size];
        data[size / 2] = b'\\';
        group.throughput(Throughput::Bytes(size as u64));

        group.bench_with_input(BenchmarkId::new("simd", size), &data, |b, data| {
            b.iter(|| find_json_escape(black_box(data), 0))
        });

        group.bench_with_input(BenchmarkId::new("scalar", size), &data, |b, data| {
            b.iter(|| find_json_escape_scalar(black_box(data), 0))
        });
    }

    group.finish();
}

/// Benchmark escape scanning with control characters
fn bench_escape_scan_control_chars(c: &mut Criterion) {
    let mut group = c.benchmark_group("escape_scan/control_chars");

    for &size in &[32, 64, 128, 256, 512, 1024] {
        // Control char (tab) at position 50
        let mut data = vec![b'x'; size];
        let escape_pos = (size / 2).min(50);
        data[escape_pos] = b'\t';
        group.throughput(Throughput::Bytes(size as u64));

        group.bench_with_input(BenchmarkId::new("simd", size), &data, |b, data| {
            b.iter(|| find_json_escape(black_box(data), 0))
        });

        group.bench_with_input(BenchmarkId::new("scalar", size), &data, |b, data| {
            b.iter(|| find_json_escape_scalar(black_box(data), 0))
        });
    }

    group.finish();
}

/// Benchmark with realistic JSON string content (mixed alphanumeric with occasional escapes)
fn bench_escape_scan_realistic(c: &mut Criterion) {
    let mut group = c.benchmark_group("escape_scan/realistic");

    // Simulate typical JSON string: mostly safe chars with occasional escapes
    // Pattern: 20 safe chars, then an escape, repeated
    fn make_realistic_string(len: usize) -> Vec<u8> {
        let mut data = Vec::with_capacity(len);
        let pattern: &[u8] = b"hello world 12345!?";
        for i in 0..len {
            if i > 0 && i % 20 == 0 {
                data.push(b'"'); // Escape every 20 chars
            } else {
                data.push(pattern[i % pattern.len()]);
            }
        }
        data
    }

    for &size in &[64, 128, 256, 512, 1024, 2048] {
        let data = make_realistic_string(size);
        group.throughput(Throughput::Bytes(size as u64));

        group.bench_with_input(BenchmarkId::new("simd", size), &data, |b, data| {
            b.iter(|| {
                let mut pos = 0;
                while pos < data.len() {
                    let escape_pos = find_json_escape(black_box(data), pos);
                    if escape_pos < data.len() {
                        pos = escape_pos + 1;
                    } else {
                        break;
                    }
                }
            })
        });

        group.bench_with_input(BenchmarkId::new("scalar", size), &data, |b, data| {
            b.iter(|| {
                let mut pos = 0;
                while pos < data.len() {
                    let escape_pos = find_json_escape_scalar(black_box(data), pos);
                    if escape_pos < data.len() {
                        pos = escape_pos + 1;
                    } else {
                        break;
                    }
                }
            })
        });
    }

    group.finish();
}

criterion_group!(
    benches,
    bench_double_quoted,
    bench_single_quoted,
    bench_multiline,
    bench_unicode_8digit,
    bench_realistic_document,
    bench_large_document,
);

criterion_group!(
    escape_scan_benches,
    bench_escape_scan_no_escapes,
    bench_escape_scan_early_escape,
    bench_escape_scan_mid_escape,
    bench_escape_scan_control_chars,
    bench_escape_scan_realistic,
);

criterion_main!(benches, escape_scan_benches);
