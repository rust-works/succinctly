//! Micro-benchmarks comparing old (as_str + JSON escape) vs new (direct transcode) paths
//!
//! The "old way" decodes YAML to a Rust String via as_str(), then JSON-escapes it.
//! The "new way" transcodes YAML escapes directly to JSON escapes in one pass via to_json_document().
#![allow(clippy::collapsible_match)]

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use succinctly::yaml::YamlIndex;

/// JSON-escape a string (the "old way" second step)
fn json_escape_string(s: &str) -> String {
    let mut output = String::with_capacity(s.len() + 2);
    output.push('"');
    for c in s.chars() {
        match c {
            '"' => output.push_str("\\\""),
            '\\' => output.push_str("\\\\"),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            c if c < '\x20' => {
                output.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => output.push(c),
        }
    }
    output.push('"');
    output
}

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

/// Compare old vs new path for double-quoted strings with escapes
fn bench_double_quoted_comparison(c: &mut Criterion) {
    let mut group = c.benchmark_group("transcode/double_quoted");

    for &count in &[10, 50, 100, 500] {
        let yaml = make_double_quoted_with_escapes(count);
        let index = YamlIndex::build(&yaml).unwrap();

        group.throughput(Throughput::Bytes(yaml.len() as u64));

        // Old way: Navigate to string, call as_str() to decode, then JSON-escape
        group.bench_with_input(
            BenchmarkId::new("old_as_str", count),
            &(&yaml, &index),
            |b, (yaml, index)| {
                b.iter(|| {
                    let cursor = index.root(yaml);
                    // Root is a sequence containing one document
                    // Document is a mapping with key "value"
                    if let succinctly::yaml::YamlValue::Sequence(elements) = cursor.value() {
                        if let Some((doc, _)) = elements.uncons() {
                            if let succinctly::yaml::YamlValue::Mapping(fields) = doc {
                                if let Some(value) = fields.find("value") {
                                    if let succinctly::yaml::YamlValue::String(s) = value {
                                        // Old path: decode to Rust string, then JSON-escape
                                        let decoded = s.as_str().unwrap();
                                        let json = json_escape_string(&decoded);
                                        return black_box(json);
                                    }
                                }
                            }
                        }
                    }
                    black_box(String::new())
                })
            },
        );

        // New way: direct transcode via to_json_document
        group.bench_with_input(
            BenchmarkId::new("new_transcode", count),
            &(&yaml, &index),
            |b, (yaml, index)| {
                b.iter(|| {
                    let cursor = index.root(black_box(yaml));
                    let result = cursor.to_json_document();
                    black_box(result)
                })
            },
        );
    }

    group.finish();
}

/// Compare old vs new path for single-quoted strings
fn bench_single_quoted_comparison(c: &mut Criterion) {
    let mut group = c.benchmark_group("transcode/single_quoted");

    for &count in &[10, 50, 100, 500] {
        let yaml = make_single_quoted_with_escapes(count);
        let index = YamlIndex::build(&yaml).unwrap();

        group.throughput(Throughput::Bytes(yaml.len() as u64));

        // Old way
        group.bench_with_input(
            BenchmarkId::new("old_as_str", count),
            &(&yaml, &index),
            |b, (yaml, index)| {
                b.iter(|| {
                    let cursor = index.root(yaml);
                    if let succinctly::yaml::YamlValue::Sequence(elements) = cursor.value() {
                        if let Some((doc, _)) = elements.uncons() {
                            if let succinctly::yaml::YamlValue::Mapping(fields) = doc {
                                if let Some(value) = fields.find("value") {
                                    if let succinctly::yaml::YamlValue::String(s) = value {
                                        let decoded = s.as_str().unwrap();
                                        let json = json_escape_string(&decoded);
                                        return black_box(json);
                                    }
                                }
                            }
                        }
                    }
                    black_box(String::new())
                })
            },
        );

        // New way
        group.bench_with_input(
            BenchmarkId::new("new_transcode", count),
            &(&yaml, &index),
            |b, (yaml, index)| {
                b.iter(|| {
                    let cursor = index.root(black_box(yaml));
                    let result = cursor.to_json_document();
                    black_box(result)
                })
            },
        );
    }

    group.finish();
}

/// Compare old vs new path for multiline strings (line folding)
fn bench_multiline_comparison(c: &mut Criterion) {
    let mut group = c.benchmark_group("transcode/multiline");

    for &lines in &[5, 20, 50, 100] {
        let yaml = make_multiline_double_quoted(lines);
        let index = YamlIndex::build(&yaml).unwrap();

        group.throughput(Throughput::Bytes(yaml.len() as u64));

        // Old way
        group.bench_with_input(
            BenchmarkId::new("old_as_str", lines),
            &(&yaml, &index),
            |b, (yaml, index)| {
                b.iter(|| {
                    let cursor = index.root(yaml);
                    if let succinctly::yaml::YamlValue::Sequence(elements) = cursor.value() {
                        if let Some((doc, _)) = elements.uncons() {
                            if let succinctly::yaml::YamlValue::Mapping(fields) = doc {
                                if let Some(value) = fields.find("value") {
                                    if let succinctly::yaml::YamlValue::String(s) = value {
                                        let decoded = s.as_str().unwrap();
                                        let json = json_escape_string(&decoded);
                                        return black_box(json);
                                    }
                                }
                            }
                        }
                    }
                    black_box(String::new())
                })
            },
        );

        // New way
        group.bench_with_input(
            BenchmarkId::new("new_transcode", lines),
            &(&yaml, &index),
            |b, (yaml, index)| {
                b.iter(|| {
                    let cursor = index.root(black_box(yaml));
                    let result = cursor.to_json_document();
                    black_box(result)
                })
            },
        );
    }

    group.finish();
}

/// Compare old vs new path for 8-digit unicode (surrogate pairs)
fn bench_unicode_comparison(c: &mut Criterion) {
    let mut group = c.benchmark_group("transcode/unicode_8digit");

    for &count in &[10, 50, 100] {
        let yaml = make_8digit_unicode(count);
        let index = YamlIndex::build(&yaml).unwrap();

        group.throughput(Throughput::Bytes(yaml.len() as u64));

        // Old way
        group.bench_with_input(
            BenchmarkId::new("old_as_str", count),
            &(&yaml, &index),
            |b, (yaml, index)| {
                b.iter(|| {
                    let cursor = index.root(yaml);
                    if let succinctly::yaml::YamlValue::Sequence(elements) = cursor.value() {
                        if let Some((doc, _)) = elements.uncons() {
                            if let succinctly::yaml::YamlValue::Mapping(fields) = doc {
                                if let Some(value) = fields.find("value") {
                                    if let succinctly::yaml::YamlValue::String(s) = value {
                                        let decoded = s.as_str().unwrap();
                                        let json = json_escape_string(&decoded);
                                        return black_box(json);
                                    }
                                }
                            }
                        }
                    }
                    black_box(String::new())
                })
            },
        );

        // New way
        group.bench_with_input(
            BenchmarkId::new("new_transcode", count),
            &(&yaml, &index),
            |b, (yaml, index)| {
                b.iter(|| {
                    let cursor = index.root(black_box(yaml));
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

    // Old way - extract each string field individually and JSON-escape
    group.bench_function("config_old_as_str", |b| {
        b.iter(|| {
            let mut result = String::new();
            let cursor = index.root(config_yaml);
            if let succinctly::yaml::YamlValue::Sequence(elements) = cursor.value() {
                if let Some((doc, _)) = elements.uncons() {
                    if let succinctly::yaml::YamlValue::Mapping(fields) = doc {
                        // Extract database strings
                        if let Some(db) = fields.find("database") {
                            if let succinctly::yaml::YamlValue::Mapping(db_fields) = db {
                                for key in &["host", "username", "password", "connection_string"] {
                                    if let Some(value) = db_fields.find(key) {
                                        if let succinctly::yaml::YamlValue::String(s) = value {
                                            if let Ok(decoded) = s.as_str() {
                                                result.push_str(&json_escape_string(&decoded));
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        // Extract logging strings
                        if let Some(log) = fields.find("logging") {
                            if let succinctly::yaml::YamlValue::Mapping(log_fields) = log {
                                for key in &["level", "format", "file"] {
                                    if let Some(value) = log_fields.find(key) {
                                        if let succinctly::yaml::YamlValue::String(s) = value {
                                            if let Ok(decoded) = s.as_str() {
                                                result.push_str(&json_escape_string(&decoded));
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            black_box(result)
        })
    });

    // New way - full document conversion
    group.bench_function("config_new_transcode", |b| {
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

    group.bench_function("escape_heavy_old", |b| {
        b.iter(|| {
            let mut result = String::new();
            let cursor = escape_index.root(escape_heavy);
            if let succinctly::yaml::YamlValue::Sequence(elements) = cursor.value() {
                if let Some((doc, _)) = elements.uncons() {
                    if let succinctly::yaml::YamlValue::Mapping(fields) = doc {
                        if let Some(escapes) = fields.find("escapes") {
                            if let succinctly::yaml::YamlValue::Mapping(esc_fields) = escapes {
                                for key in &[
                                    "newlines",
                                    "tabs",
                                    "quotes",
                                    "backslash",
                                    "mixed",
                                    "unicode",
                                    "hex",
                                    "all_escapes",
                                ] {
                                    if let Some(value) = esc_fields.find(key) {
                                        if let succinctly::yaml::YamlValue::String(s) = value {
                                            if let Ok(decoded) = s.as_str() {
                                                result.push_str(&json_escape_string(&decoded));
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            black_box(result)
        })
    });

    group.bench_function("escape_heavy_new", |b| {
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

    // Old way - extract strings from first 10 items
    group.bench_function("500_items_old_sample", |b| {
        b.iter(|| {
            let mut result = String::new();
            let cursor = index.root(&large_yaml);
            if let succinctly::yaml::YamlValue::Sequence(elements) = cursor.value() {
                if let Some((doc, _)) = elements.uncons() {
                    if let succinctly::yaml::YamlValue::Mapping(fields) = doc {
                        if let Some(items) = fields.find("items") {
                            if let succinctly::yaml::YamlValue::Sequence(item_elements) = items {
                                // Extract from first 10 items
                                let mut remaining = item_elements;
                                for _ in 0..10 {
                                    if let Some((item, rest)) = remaining.uncons() {
                                        if let succinctly::yaml::YamlValue::Mapping(item_fields) =
                                            item
                                        {
                                            if let Some(name) = item_fields.find("name") {
                                                if let succinctly::yaml::YamlValue::String(s) = name
                                                {
                                                    if let Ok(decoded) = s.as_str() {
                                                        result.push_str(&json_escape_string(
                                                            &decoded,
                                                        ));
                                                    }
                                                }
                                            }
                                            if let Some(desc) = item_fields.find("desc") {
                                                if let succinctly::yaml::YamlValue::String(s) = desc
                                                {
                                                    if let Ok(decoded) = s.as_str() {
                                                        result.push_str(&json_escape_string(
                                                            &decoded,
                                                        ));
                                                    }
                                                }
                                            }
                                        }
                                        remaining = rest;
                                    } else {
                                        break;
                                    }
                                }
                            }
                        }
                    }
                }
            }
            black_box(result)
        })
    });

    // New way - full document conversion
    group.bench_function("500_items_new_full", |b| {
        b.iter(|| {
            let cursor = index.root(black_box(&large_yaml));
            let result = cursor.to_json_document();
            black_box(result)
        })
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_double_quoted_comparison,
    bench_single_quoted_comparison,
    bench_multiline_comparison,
    bench_unicode_comparison,
    bench_realistic_document,
    bench_large_document,
);
criterion_main!(benches);
