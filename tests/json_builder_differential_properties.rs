#![allow(unsafe_code)] // the SVE2 builders are `unsafe fn`, called only after `has_sve2()`
//! #7539: every JSON index builder this architecture compiles produces the same index.
//!
//! The Standard Cursor has one definition, the four-state machine in `json::standard` (see
//! `docs/parsing/json.md`); the portable PFSM builder is its table-driven form and the builder the
//! index uses where no SIMD backend exists, and every SIMD backend must reproduce it byte for byte
//! on any input, malformed input included (#186). NEON and SVE2 did not: after a bare value token
//! they took a `"` as the start of a new string, where the state machine ends the value and opens
//! nothing (`""""""a"`). The x86 builders share one table-driven kernel derived from the state
//! machine, so on x86_64 this file checks SSE2 and, where detected, AVX2.
//!
//! Every builder available here (scalar, portable PFSM, the dispatcher, and each SIMD level) is
//! run over every one-to-three-byte combination of the bytes the state machines distinguish,
//! placed across the 16- and 32-byte chunk boundaries, over random byte soups, and over generated
//! documents and mutations of them (keys and strings with escapes, truncations, inserted, deleted
//! and replaced tokens). The interest bits, parentheses bits and final state must equal the
//! portable builder's. The Simple Cursor's builders are checked the same way. Deterministic (a
//! fixed seed) and sized to run in a few seconds in a debug build.

use succinctly::json;

/// splitmix64: deterministic, no dependency on the `rand` API.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }

    fn pick<T: Copy>(&mut self, items: &[T]) -> T {
        items[self.below(items.len())]
    }
}

/// One byte of each class the Standard and Simple state machines tell apart, plus the near
/// misses a nibble classifier can confuse with them (`*` `<` `@` `_` `^` `|`, #186).
const ALPHABET: &[u8] = b"{}[],:\"\\ \n\ta1Z-+.e_@*<^|~\x00\x7f\xc3\xa9";

type Standard = (
    &'static str,
    Box<dyn Fn(&[u8]) -> json::standard::SemiIndex>,
);
type Simple = (&'static str, Box<dyn Fn(&[u8]) -> json::simple::SemiIndex>);

/// Every Standard Cursor builder compiled for this target, the reference (portable PFSM) first.
fn standard_builders() -> Vec<Standard> {
    let mut builders: Vec<Standard> = vec![
        ("portable", Box::new(json::standard::build_semi_index)),
        ("scalar", Box::new(json::standard::build_semi_index_scalar)),
    ];
    #[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
    builders.push(("dispatch", Box::new(json::simd::build_semi_index_standard)));
    #[cfg(target_arch = "x86_64")]
    {
        builders.push(("sse2", Box::new(json::simd::x86::build_semi_index_standard)));
        #[cfg(feature = "std")]
        if is_x86_feature_detected!("avx2") {
            builders.push((
                "avx2",
                Box::new(json::simd::avx2::build_semi_index_standard),
            ));
        }
    }
    #[cfg(target_arch = "aarch64")]
    {
        builders.push((
            "neon",
            Box::new(json::simd::neon::build_semi_index_standard),
        ));
        #[cfg(feature = "std")]
        if json::simd::sve2::has_sve2() {
            builders.push((
                "sve2",
                // SAFETY: SVE2 was detected just above.
                Box::new(|j: &[u8]| unsafe { json::simd::sve2::build_semi_index_standard(j) }),
            ));
        }
    }
    builders
}

/// Every Simple Cursor builder compiled for this target, the reference (portable) first.
fn simple_builders() -> Vec<Simple> {
    let mut builders: Vec<Simple> = vec![("portable", Box::new(json::simple::build_semi_index))];
    #[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
    builders.push(("dispatch", Box::new(json::simd::build_semi_index_simple)));
    #[cfg(target_arch = "x86_64")]
    {
        builders.push(("sse2", Box::new(json::simd::x86::build_semi_index_simple)));
        #[cfg(feature = "std")]
        if is_x86_feature_detected!("avx2") {
            builders.push(("avx2", Box::new(json::simd::avx2::build_semi_index_simple)));
        }
    }
    #[cfg(target_arch = "aarch64")]
    {
        builders.push(("neon", Box::new(json::simd::neon::build_semi_index_simple)));
        #[cfg(feature = "std")]
        if json::simd::sve2::has_sve2() {
            builders.push((
                "sve2",
                // SAFETY: SVE2 was detected just above.
                Box::new(|j: &[u8]| unsafe { json::simd::sve2::build_semi_index_simple(j) }),
            ));
        }
    }
    builders
}

/// The first builder that disagrees with the reference on `text`, as a report.
fn standard_disagreement(builders: &[Standard], text: &[u8]) -> Option<String> {
    let want = (builders[0].1)(text);
    builders[1..].iter().find_map(|(name, build)| {
        let got = build(text);
        (got.ib != want.ib || got.bp != want.bp || got.state != want.state).then(|| {
            format!(
                "{name} vs {}: ib {:x?} vs {:x?}, bp {:x?} vs {:x?}, state {:?} vs {:?}",
                builders[0].0, got.ib, want.ib, got.bp, want.bp, got.state, want.state
            )
        })
    })
}

fn simple_disagreement(builders: &[Simple], text: &[u8]) -> Option<String> {
    let want = (builders[0].1)(text);
    builders[1..].iter().find_map(|(name, build)| {
        let got = build(text);
        (got.ib != want.ib || got.bp != want.bp || got.state != want.state).then(|| {
            format!(
                "{name} vs {}: ib {:x?} vs {:x?}, bp {:x?} vs {:x?}, state {:?} vs {:?}",
                builders[0].0, got.ib, want.ib, got.bp, want.bp, got.state, want.state
            )
        })
    })
}

/// Deletes bytes from a failing input while it still fails, so the report names a short case.
fn shrink(text: &[u8], fails: impl Fn(&[u8]) -> bool) -> Vec<u8> {
    let mut text = text.to_vec();
    let mut i = 0;
    while i < text.len() {
        let mut shorter = text.clone();
        shorter.remove(i);
        if fails(&shorter) {
            text = shorter;
        } else {
            i += 1;
        }
    }
    text
}

/// Panics, naming the shortest failing input found, if any builder disagrees on any text.
fn assert_all_agree(texts: impl IntoIterator<Item = Vec<u8>>) {
    let standard = standard_builders();
    let simple = simple_builders();
    let mut checked = 0usize;
    for text in texts {
        checked += 1;
        if standard_disagreement(&standard, &text).is_some() {
            let small = shrink(&text, |t| standard_disagreement(&standard, t).is_some());
            panic!(
                "standard cursor builders disagree on {:?} (shrunk from {:?}): {}",
                String::from_utf8_lossy(&small),
                String::from_utf8_lossy(&text),
                standard_disagreement(&standard, &small).unwrap()
            );
        }
        if simple_disagreement(&simple, &text).is_some() {
            let small = shrink(&text, |t| simple_disagreement(&simple, t).is_some());
            panic!(
                "simple cursor builders disagree on {:?} (shrunk from {:?}): {}",
                String::from_utf8_lossy(&small),
                String::from_utf8_lossy(&text),
                simple_disagreement(&simple, &small).unwrap()
            );
        }
    }
    assert!(checked > 0, "no inputs were generated");
}

/// The case #7539 was found on, with the index the state machine gives it: three empty
/// strings, the value `a`, and a `"` that ends the value without opening a string.
#[test]
fn a_quote_glued_to_a_value_ends_it_and_opens_nothing() {
    let text = b"\"\"\"\"\"\"a\"";
    for (name, build) in standard_builders() {
        let semi = build(text);
        assert_eq!(semi.ib, [0x55], "{name}: interest bits");
        assert_eq!(semi.bp, [0x55], "{name}: parentheses bits");
        assert_eq!(
            semi.state,
            json::standard::State::InJson,
            "{name}: final state"
        );
    }
    for glued in [
        &br#"[1"a"]"#[..],
        br#"{"a":1"b"}"#,
        b"true\"x",
        b"-2.5e3\"\\\"",
    ] {
        assert_all_agree([glued.to_vec()]);
    }
}

/// The bytes whose transitions differ between states; [`ALPHABET`] adds the near misses.
const CORE: &[u8] = b"{}[],:\"\\ a1-.\xc3";

/// Every byte value, every string of one or two alphabet bytes, and of three core bytes, starting
/// at offsets on both sides of the 16- and 32-byte chunk boundaries (the leading spaces keep the
/// state machine in `InJson`), on its own, after a value token, inside a string and after an
/// escape, so every transition out of every state is taken at every chunk position.
#[test]
fn every_short_combination_agrees_at_every_chunk_position() {
    let mut words: Vec<Vec<u8>> = (0..=255u8).map(|a| vec![a]).collect();
    for &a in ALPHABET {
        for &b in ALPHABET {
            words.push(vec![a, b]);
        }
    }
    for &a in CORE {
        for &b in CORE {
            for &c in CORE {
                words.push(vec![a, b, c]);
            }
        }
    }
    let leads: &[&[u8]] = &[b"", b"7", b"\"", b"\"\\"];
    let texts = words.iter().flat_map(|word| {
        let pads: &[usize] = if word.len() < 3 {
            &[0, 13, 15, 16, 30, 31, 47]
        } else {
            &[0, 14, 30]
        };
        pads.iter().flat_map(move |&pad| {
            leads.iter().map(move |lead| {
                let mut text = vec![b' '; pad];
                text.extend_from_slice(lead);
                text.extend_from_slice(word);
                text
            })
        })
    });
    assert_all_agree(texts);
}

/// Byte soups over the alphabet, of every length up to a few chunks.
#[test]
fn random_byte_soups_agree() {
    let mut rng = Rng(0x7539_0001);
    let texts = (0..4000).map(|_| {
        let len = rng.below(150);
        (0..len).map(|_| rng.pick(ALPHABET)).collect::<Vec<u8>>()
    });
    assert_all_agree(texts.collect::<Vec<_>>());
}

const STRINGS: &[&str] = &[
    r#""""#,
    r#""a""#,
    r#""k\"""#,
    r#""\\""#,
    r#""a\\b\"c""#,
    r#""é""#,
    r#""é""#,
    r#""x y""#,
    r#""\\\\\"""#,
];
const SCALARS: &[&str] = &["1", "-2.5e+3", "0", "null", "true", "false"];
const SPACE: &[&str] = &["", "", "", " ", "\n", " \t "];

/// A well-formed document of up to `depth` levels, with escapes in keys and strings.
fn generate(rng: &mut Rng, depth: usize, out: &mut String) {
    let ws = |rng: &mut Rng, out: &mut String| out.push_str(rng.pick(SPACE));
    match if depth == 0 {
        rng.below(2)
    } else {
        rng.below(4)
    } {
        0 => out.push_str(rng.pick(SCALARS)),
        1 => out.push_str(rng.pick(STRINGS)),
        kind => {
            let object = kind == 2;
            out.push(if object { '{' } else { '[' });
            for i in 0..rng.below(4) {
                if i > 0 {
                    out.push(',');
                }
                ws(rng, out);
                if object {
                    out.push_str(rng.pick(STRINGS));
                    ws(rng, out);
                    out.push(':');
                    ws(rng, out);
                }
                generate(rng, depth - 1, out);
                ws(rng, out);
            }
            out.push(if object { '}' } else { ']' });
        }
    }
}

const TOKENS: &[&str] = &[
    "\"",
    "\\",
    "\\\"",
    "a",
    "1",
    "-",
    "[",
    "]",
    "{",
    "}",
    ",",
    ":",
    " ",
    "\"k\\\"\":",
    "e+",
];

fn mutate(rng: &mut Rng, text: &mut Vec<u8>) {
    for _ in 0..=rng.below(3) {
        let at = rng.below(text.len() + 1);
        match rng.below(5) {
            0 => text.truncate(at),
            1 if at < text.len() => {
                text.remove(at);
            }
            2 if at < text.len() => text[at] = rng.pick(ALPHABET),
            _ => {
                let token = rng.pick(TOKENS).as_bytes();
                text.splice(at..at, token.iter().copied());
            }
        }
    }
}

/// Generated documents, each as written and after a few mutations.
#[test]
fn generated_and_mutated_documents_agree() {
    let mut rng = Rng(0x7539_0002);
    let mut texts = Vec::new();
    for round in 0..1500 {
        let mut doc = String::new();
        let depth = if round % 50 == 0 { 8 } else { 1 + rng.below(4) };
        generate(&mut rng, depth, &mut doc);
        let doc = doc.into_bytes();
        texts.push(doc.clone());
        for _ in 0..3 {
            let mut mutated = doc.clone();
            mutate(&mut rng, &mut mutated);
            texts.push(mutated);
        }
    }
    assert_all_agree(texts);
}
