use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};

/// Scalar version: parse anchor name byte-by-byte
fn parse_anchor_name_scalar(input: &[u8], start: usize) -> usize {
    let mut pos = start;
    while pos < input.len() {
        let b = input[pos];
        match b {
            // Stop at flow indicators, whitespace, and newlines
            b' ' | b'\t' | b'\n' | b'\r' | b'[' | b']' | b'{' | b'}' | b',' => break,
            // Colon is allowed in anchor names if not followed by whitespace
            b':' => {
                if pos + 1 < input.len() {
                    let next = input[pos + 1];
                    if next == b' ' || next == b'\t' || next == b'\n' || next == b'\r' {
                        break;
                    }
                }
                pos += 1;
            }
            _ => pos += 1,
        }
    }
    pos
}

/// SIMD version: scan for terminator characters in parallel
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn parse_anchor_name_simd(input: &[u8], start: usize) -> usize {
    use std::arch::x86_64::*;

    let len = input.len();
    if start >= len {
        return start;
    }

    let mut pos = start;
    let end = len;

    // Process 32 bytes at a time with AVX2
    while pos + 32 <= end {
        let chunk = _mm256_loadu_si256(input.as_ptr().add(pos) as *const __m256i);

        // Check for whitespace characters
        let space = _mm256_cmpeq_epi8(chunk, _mm256_set1_epi8(b' ' as i8));
        let tab = _mm256_cmpeq_epi8(chunk, _mm256_set1_epi8(b'\t' as i8));
        let newline = _mm256_cmpeq_epi8(chunk, _mm256_set1_epi8(b'\n' as i8));
        let cr = _mm256_cmpeq_epi8(chunk, _mm256_set1_epi8(b'\r' as i8));

        // Check for flow indicators
        let lbracket = _mm256_cmpeq_epi8(chunk, _mm256_set1_epi8(b'[' as i8));
        let rbracket = _mm256_cmpeq_epi8(chunk, _mm256_set1_epi8(b']' as i8));
        let lbrace = _mm256_cmpeq_epi8(chunk, _mm256_set1_epi8(b'{' as i8));
        let rbrace = _mm256_cmpeq_epi8(chunk, _mm256_set1_epi8(b'}' as i8));
        let comma = _mm256_cmpeq_epi8(chunk, _mm256_set1_epi8(b',' as i8));
        let colon = _mm256_cmpeq_epi8(chunk, _mm256_set1_epi8(b':' as i8));

        // Combine all terminator checks
        let ws = _mm256_or_si256(space, tab);
        let ws = _mm256_or_si256(ws, newline);
        let ws = _mm256_or_si256(ws, cr);

        let flow = _mm256_or_si256(lbracket, rbracket);
        let flow = _mm256_or_si256(flow, lbrace);
        let flow = _mm256_or_si256(flow, rbrace);
        let flow = _mm256_or_si256(flow, comma);

        let terminators = _mm256_or_si256(ws, flow);
        let terminators = _mm256_or_si256(terminators, colon);

        let mask = _mm256_movemask_epi8(terminators);

        if mask != 0 {
            // Found terminator - find first position
            let offset = mask.trailing_zeros() as usize;
            return pos + offset;
        }

        pos += 32;
    }

    // Handle remaining bytes with scalar
    while pos < end {
        let b = input[pos];
        match b {
            b' ' | b'\t' | b'\n' | b'\r' | b'[' | b']' | b'{' | b'}' | b',' => break,
            b':' => {
                if pos + 1 < end {
                    let next = input[pos + 1];
                    if next == b' ' || next == b'\t' || next == b'\n' || next == b'\r' {
                        break;
                    }
                }
                pos += 1;
            }
            _ => pos += 1,
        }
    }

    pos
}

fn bench_anchor_name_parsing(c: &mut Criterion) {
    let mut group = c.benchmark_group("anchor_name");

    // Test different anchor name lengths
    let sizes = vec![
        ("short_4", "test"),
        ("medium_16", "my_anchor_name_1"),
        ("long_32", "very_long_anchor_name_with_nums"),
        (
            "very_long_64",
            "extremely_long_anchor_name_that_goes_on_and_on_with_underscores_",
        ),
    ];

    for (name, anchor) in sizes {
        let input = format!("&{} value", anchor);
        let bytes = input.as_bytes();
        let len = anchor.len();

        group.throughput(Throughput::Bytes(len as u64));

        group.bench_with_input(BenchmarkId::new("scalar", name), &bytes, |b, input| {
            b.iter(|| parse_anchor_name_scalar(black_box(input), black_box(1)))
        });

        #[cfg(target_arch = "x86_64")]
        if is_x86_feature_detected!("avx2") {
            group.bench_with_input(BenchmarkId::new("simd", name), &bytes, |b, input| {
                b.iter(|| unsafe { parse_anchor_name_simd(black_box(input), black_box(1)) })
            });
        }
    }

    group.finish();
}

fn bench_alias_scanning(c: &mut Criterion) {
    let mut group = c.benchmark_group("alias_scan");

    // Test scanning for & and * characters in documents
    let sizes = vec![
        ("small_100", 100),
        ("medium_1kb", 1024),
        ("large_10kb", 10240),
    ];

    for (name, size) in sizes {
        // Create document with occasional anchors/aliases
        let mut doc = String::new();
        for i in 0..size / 20 {
            if i % 10 == 0 {
                doc.push_str(&format!("key{}: &anchor{} value\n", i, i));
            } else if i % 10 == 5 {
                doc.push_str(&format!("ref{}: *anchor{}\n", i, i / 10));
            } else {
                doc.push_str(&format!("key{}: value\n", i));
            }
        }
        let bytes = doc.as_bytes();

        group.throughput(Throughput::Bytes(bytes.len() as u64));

        group.bench_with_input(
            BenchmarkId::new("count_anchors_scalar", name),
            &bytes,
            |b, input| {
                b.iter(|| {
                    let input = black_box(input);
                    let mut count = 0;
                    for &byte in input.iter() {
                        if byte == b'&' || byte == b'*' {
                            count += 1;
                        }
                    }
                    count
                })
            },
        );

        #[cfg(target_arch = "x86_64")]
        if is_x86_feature_detected!("avx2") {
            group.bench_with_input(
                BenchmarkId::new("count_anchors_simd", name),
                &bytes,
                |b, input| {
                    b.iter(|| unsafe {
                        use std::arch::x86_64::*;
                        let input = black_box(input);
                        let mut count = 0;
                        let mut pos = 0;

                        while pos + 32 <= input.len() {
                            let chunk =
                                _mm256_loadu_si256(input.as_ptr().add(pos) as *const __m256i);
                            let anchor = _mm256_cmpeq_epi8(chunk, _mm256_set1_epi8(b'&' as i8));
                            let alias = _mm256_cmpeq_epi8(chunk, _mm256_set1_epi8(b'*' as i8));
                            let matches = _mm256_or_si256(anchor, alias);
                            let mask = _mm256_movemask_epi8(matches);
                            count += mask.count_ones();
                            pos += 32;
                        }

                        while pos < input.len() {
                            if input[pos] == b'&' || input[pos] == b'*' {
                                count += 1;
                            }
                            pos += 1;
                        }

                        count
                    })
                },
            );
        }
    }

    group.finish();
}

criterion_group!(benches, bench_anchor_name_parsing, bench_alias_scanning);
criterion_main!(benches);
