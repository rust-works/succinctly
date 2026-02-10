//! Benchmarks for UTF-8 validation.
//!
//! These benchmarks measure the performance of UTF-8 validation across
//! different content types and sizes.
//!
//! ## Content Types
//!
//! - **ASCII**: Pure 7-bit ASCII content (fastest to validate)
//! - **Mixed UTF-8**: Realistic mix of ASCII and multi-byte characters
//! - **Multi-byte Heavy**: Predominantly 2-4 byte UTF-8 sequences
//! - **CJK Text**: Chinese/Japanese/Korean characters (3-byte sequences)
//! - **Emoji Heavy**: Heavy use of 4-byte sequences (emojis)
//!
//! ## Sizes
//!
//! Benchmarks run at multiple sizes to show scaling characteristics:
//! - 1KB, 10KB, 100KB, 1MB, 10MB

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use succinctly::text::utf8::validate_utf8;

/// Generate pure ASCII content of the specified size.
fn generate_ascii(size: usize) -> Vec<u8> {
    let pattern =
        b"The quick brown fox jumps over the lazy dog. 0123456789!@#$%^&*()_+-=[]{}|;':\",./<>?\n";
    let mut result = Vec::with_capacity(size);
    while result.len() < size {
        let remaining = size - result.len();
        let chunk = &pattern[..remaining.min(pattern.len())];
        result.extend_from_slice(chunk);
    }
    result
}

/// Generate mixed UTF-8 content (ASCII with occasional multi-byte).
/// Approximately 70% ASCII, 20% 2-byte, 8% 3-byte, 2% 4-byte.
fn generate_mixed(size: usize) -> Vec<u8> {
    let pattern = "Hello, world! Café résumé naïve über. 日本語 中文 한국어. Emoji: 🎉🚀💻. More ASCII text here.\n";
    let pattern_bytes = pattern.as_bytes();
    let mut result = Vec::with_capacity(size);
    while result.len() < size {
        let remaining = size - result.len();
        if remaining >= pattern_bytes.len() {
            result.extend_from_slice(pattern_bytes);
        } else {
            // Careful: don't split multi-byte sequences
            // Just pad with ASCII to avoid partial sequences
            result.extend(std::iter::repeat(b'A').take(remaining));
        }
    }
    result.truncate(size);
    result
}

/// Generate predominantly multi-byte content (CJK characters).
fn generate_cjk(size: usize) -> Vec<u8> {
    // Each CJK character is 3 bytes
    let cjk_chars = "日本語中文韓國語漢字假名平仮名片仮名ひらがなカタカナ한글조선어";
    let cjk_bytes = cjk_chars.as_bytes();
    let mut result = Vec::with_capacity(size);
    while result.len() < size {
        let remaining = size - result.len();
        if remaining >= cjk_bytes.len() {
            result.extend_from_slice(cjk_bytes);
        } else {
            // Pad with ASCII to avoid partial sequences
            result.extend(std::iter::repeat(b'X').take(remaining));
        }
    }
    result.truncate(size);
    result
}

/// Generate emoji-heavy content (4-byte sequences).
fn generate_emoji(size: usize) -> Vec<u8> {
    // Each emoji is 4 bytes
    let emojis = "🎉🚀💻🔥🌍😀🎯💡🌟⭐🎨🎭🎪🎢🎡🎠🎰🎲🎳🎯🎱🎾🏀🏈⚽🏐🏉🎿⛷️🏂";
    let emoji_bytes = emojis.as_bytes();
    let mut result = Vec::with_capacity(size);
    while result.len() < size {
        let remaining = size - result.len();
        if remaining >= emoji_bytes.len() {
            result.extend_from_slice(emoji_bytes);
        } else {
            // Pad with ASCII to avoid partial sequences
            result.extend(std::iter::repeat(b'E').take(remaining));
        }
    }
    result.truncate(size);
    result
}

/// Generate 2-byte character content (Latin Extended, Greek, Cyrillic).
fn generate_2byte(size: usize) -> Vec<u8> {
    // 2-byte characters: Latin Extended, Greek, Cyrillic
    let chars =
        "éèêëàâäùûüôöîïçñÉÈÊËÀÂÄÙÛÜÔÖÎÏÇÑαβγδεζηθικλμνξοπρστυφχψωАБВГДЕЖЗИЙКЛМНОПРСТУФХЦЧШЩЪЫЬЭЮЯ";
    let char_bytes = chars.as_bytes();
    let mut result = Vec::with_capacity(size);
    while result.len() < size {
        let remaining = size - result.len();
        if remaining >= char_bytes.len() {
            result.extend_from_slice(char_bytes);
        } else {
            result.extend(std::iter::repeat(b'L').take(remaining));
        }
    }
    result.truncate(size);
    result
}

/// Generate worst-case content: invalid byte at various positions.
/// This tests early-exit behavior.
fn generate_with_error_at_end(size: usize) -> Vec<u8> {
    let mut data = generate_ascii(size);
    if !data.is_empty() {
        // Put invalid byte near the end
        let pos = data.len().saturating_sub(1);
        data[pos] = 0x80; // Invalid lead byte
    }
    data
}

fn bench_ascii(c: &mut Criterion) {
    let mut group = c.benchmark_group("utf8_ascii");

    for size in [1024, 10 * 1024, 100 * 1024, 1024 * 1024, 10 * 1024 * 1024] {
        let data = generate_ascii(size);
        let size_name = format_size(size);

        group.throughput(Throughput::Bytes(size as u64));
        group.bench_with_input(BenchmarkId::from_parameter(&size_name), &data, |b, data| {
            b.iter(|| validate_utf8(black_box(data)));
        });
    }

    group.finish();
}

fn bench_mixed(c: &mut Criterion) {
    let mut group = c.benchmark_group("utf8_mixed");

    for size in [1024, 10 * 1024, 100 * 1024, 1024 * 1024, 10 * 1024 * 1024] {
        let data = generate_mixed(size);
        let size_name = format_size(size);

        group.throughput(Throughput::Bytes(size as u64));
        group.bench_with_input(BenchmarkId::from_parameter(&size_name), &data, |b, data| {
            b.iter(|| validate_utf8(black_box(data)));
        });
    }

    group.finish();
}

fn bench_cjk(c: &mut Criterion) {
    let mut group = c.benchmark_group("utf8_cjk");

    for size in [1024, 10 * 1024, 100 * 1024, 1024 * 1024, 10 * 1024 * 1024] {
        let data = generate_cjk(size);
        let size_name = format_size(size);

        group.throughput(Throughput::Bytes(size as u64));
        group.bench_with_input(BenchmarkId::from_parameter(&size_name), &data, |b, data| {
            b.iter(|| validate_utf8(black_box(data)));
        });
    }

    group.finish();
}

fn bench_emoji(c: &mut Criterion) {
    let mut group = c.benchmark_group("utf8_emoji");

    for size in [1024, 10 * 1024, 100 * 1024, 1024 * 1024, 10 * 1024 * 1024] {
        let data = generate_emoji(size);
        let size_name = format_size(size);

        group.throughput(Throughput::Bytes(size as u64));
        group.bench_with_input(BenchmarkId::from_parameter(&size_name), &data, |b, data| {
            b.iter(|| validate_utf8(black_box(data)));
        });
    }

    group.finish();
}

fn bench_2byte(c: &mut Criterion) {
    let mut group = c.benchmark_group("utf8_2byte");

    for size in [1024, 10 * 1024, 100 * 1024, 1024 * 1024, 10 * 1024 * 1024] {
        let data = generate_2byte(size);
        let size_name = format_size(size);

        group.throughput(Throughput::Bytes(size as u64));
        group.bench_with_input(BenchmarkId::from_parameter(&size_name), &data, |b, data| {
            b.iter(|| validate_utf8(black_box(data)));
        });
    }

    group.finish();
}

fn bench_error_at_end(c: &mut Criterion) {
    let mut group = c.benchmark_group("utf8_error_at_end");

    for size in [1024, 10 * 1024, 100 * 1024, 1024 * 1024] {
        let data = generate_with_error_at_end(size);
        let size_name = format_size(size);

        group.throughput(Throughput::Bytes(size as u64));
        group.bench_with_input(BenchmarkId::from_parameter(&size_name), &data, |b, data| {
            b.iter(|| {
                let result = validate_utf8(black_box(data));
                black_box(result)
            });
        });
    }

    group.finish();
}

/// Benchmark comparing different byte sequence lengths at same total size.
fn bench_sequence_types(c: &mut Criterion) {
    let mut group = c.benchmark_group("utf8_sequence_types_1mb");
    let size = 1024 * 1024; // 1MB

    // ASCII (1-byte)
    let ascii = generate_ascii(size);
    group.throughput(Throughput::Bytes(size as u64));
    group.bench_with_input(BenchmarkId::new("ascii_1byte", "1mb"), &ascii, |b, data| {
        b.iter(|| validate_utf8(black_box(data)));
    });

    // 2-byte sequences
    let twobyte = generate_2byte(size);
    group.bench_with_input(
        BenchmarkId::new("extended_2byte", "1mb"),
        &twobyte,
        |b, data| {
            b.iter(|| validate_utf8(black_box(data)));
        },
    );

    // 3-byte sequences (CJK)
    let cjk = generate_cjk(size);
    group.bench_with_input(BenchmarkId::new("cjk_3byte", "1mb"), &cjk, |b, data| {
        b.iter(|| validate_utf8(black_box(data)));
    });

    // 4-byte sequences (emoji)
    let emoji = generate_emoji(size);
    group.bench_with_input(BenchmarkId::new("emoji_4byte", "1mb"), &emoji, |b, data| {
        b.iter(|| validate_utf8(black_box(data)));
    });

    // Mixed
    let mixed = generate_mixed(size);
    group.bench_with_input(BenchmarkId::new("mixed", "1mb"), &mixed, |b, data| {
        b.iter(|| validate_utf8(black_box(data)));
    });

    group.finish();
}

fn format_size(bytes: usize) -> String {
    if bytes >= 1024 * 1024 {
        format!("{}mb", bytes / (1024 * 1024))
    } else if bytes >= 1024 {
        format!("{}kb", bytes / 1024)
    } else {
        format!("{}b", bytes)
    }
}

criterion_group!(
    benches,
    bench_ascii,
    bench_mixed,
    bench_cjk,
    bench_emoji,
    bench_2byte,
    bench_error_at_end,
    bench_sequence_types,
);

criterion_main!(benches);
