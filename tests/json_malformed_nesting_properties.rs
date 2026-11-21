//! #7482: the standard JSON index over malformed nesting.
//!
//! The index takes its balanced-parentheses length as two bits per open, the length of a balanced
//! document. A document left with containers open at end of input is short of closes, and those
//! are the zero bits after the last one written; when they crossed a word boundary the length ran
//! past the stored words and `is_open` indexed out of bounds (`{"a":` and 44 `[` closed by one `}`).
//!
//! These tests build the index over bracket strings and mutations of generated documents
//! (truncations, deleted, inserted and replaced bytes, surplus and missing closers, depths 1 to a
//! few thousand), then ask every navigation question at every position and walk the values. Nothing
//! may panic; on a balanced sequence every answer must match a bit-by-bit scan, and a document that
//! was generated well-formed must read back as itself. Deterministic (a fixed seed) and sized to run
//! in a second or two in a debug build.

use succinctly::bp::BalancedParens;
use succinctly::json::light::JsonCursor;
use succinctly::json::{JsonIndex, StandardJson};

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

    fn pick<'a>(&mut self, items: &[&'a str]) -> &'a str {
        items[self.below(items.len())]
    }
}

const SCALARS: [&str; 6] = ["1", "\"b\"", "null", "true", "-2.5", "\"\""];

/// A well-formed document, compact, nested exactly `depth` deep: a spine of containers, each
/// with a few shallow siblings beside the spine's next one now and then.
fn generate(rng: &mut Rng, depth: usize, out: &mut String) {
    if depth == 0 {
        out.push_str(rng.pick(&SCALARS));
        return;
    }
    let object = rng.below(2) == 0;
    out.push(if object { '{' } else { '[' });
    let others = if rng.below(4) == 0 {
        1 + rng.below(3)
    } else {
        0
    };
    let spine = rng.below(others + 1);
    for i in 0..=others {
        if i > 0 {
            out.push(',');
        }
        if object {
            out.push_str(rng.pick(&["\"a\":", "\"\":", "\"k\\\"\":"]));
        }
        if i == spine {
            generate(rng, depth - 1, out);
        } else {
            let shallow = rng.below(3).min(depth - 1);
            generate(rng, shallow, out);
        }
    }
    out.push(if object { '}' } else { ']' });
}

const TOKENS: [&str; 14] = [
    "[", "]", "{", "}", ",", ":", "\"b\":", "\"b\"", "1", ",\"b\":2", "]}", "\"", "\\", " ",
];

fn mutate(rng: &mut Rng, text: &mut Vec<u8>) {
    for _ in 0..=rng.below(3) {
        let at = rng.below(text.len() + 1);
        match rng.below(6) {
            0 => text.truncate(at),
            1 => {
                let end = (at + 1 + rng.below(8)).min(text.len());
                text.drain(at.min(end)..end);
            }
            2 if at < text.len() => text[at] = rng.pick(&TOKENS).as_bytes()[0],
            3 => {
                // Replace every closer from `at` on with one closer: containers left open at EOF.
                let keep: Vec<u8> = text[at..]
                    .iter()
                    .copied()
                    .filter(|b| !matches!(b, b']' | b'}'))
                    .collect();
                text.truncate(at);
                text.extend_from_slice(&keep);
                text.push(if rng.below(2) == 0 { b'}' } else { b']' });
            }
            4 => {
                let n = 1 + rng.below(70);
                for _ in 0..n {
                    text.push(if rng.below(2) == 0 { b'}' } else { b']' });
                }
            }
            _ => {
                let token = rng.pick(&TOKENS).as_bytes().to_vec();
                text.splice(at..at, token);
            }
        }
    }
}

/// A string of the bracket alphabet alone: `[`, `]`, `{`, `}`, `,`, `:`, scalars.
fn bracket_string(rng: &mut Rng, len: usize) -> Vec<u8> {
    let mut text = String::new();
    while text.len() < len {
        text.push_str(rng.pick(&["[", "[", "{\"a\":", "]", "}", ",", ":", "1", "\"b\""]));
    }
    text.into_bytes()
}

/// The parentheses as the words store them, read bit by bit.
fn stored_bits(bp: &BalancedParens) -> Vec<bool> {
    let words = bp.words();
    assert!(
        bp.len() <= words.len() * 64,
        "len {} past the {} stored words",
        bp.len(),
        words.len()
    );
    (0..bp.len())
        .map(|p| (words[p / 64] >> (p % 64)) & 1 == 1)
        .collect()
}

/// Every navigation question at every position, against a matching of `bits` with a stack when
/// they are balanced (one pass, so a document thousands deep stays quick).
fn check_navigation(bp: &BalancedParens, bits: &[bool], what: &str) {
    let n = bits.len();
    let mut excess = Vec::with_capacity(n);
    let mut mate = vec![None; n];
    let mut parent = vec![None; n];
    let mut stack: Vec<usize> = Vec::new();
    let mut balanced = true;
    let mut e = 0i64;
    for (p, &open) in bits.iter().enumerate() {
        e += if open { 1 } else { -1 };
        excess.push(e);
        if open {
            parent[p] = stack.last().copied();
            stack.push(p);
        } else if let Some(q) = stack.pop() {
            mate[p] = Some(q);
            mate[q] = Some(p);
        } else {
            balanced = false;
        }
    }
    balanced &= stack.is_empty();
    let mut ones = 0;
    for p in 0..n + 2 {
        // Past the end too: every query must answer, not index.
        let close = bp.find_close(p);
        let open = bp.find_open(p);
        let enclose = bp.enclose(p);
        let sibling = bp.next_sibling(p);
        let child = bp.first_child(p);
        let _ = (
            bp.prev_sibling(p),
            bp.parent(p),
            bp.depth(p),
            bp.subtree_size(p),
        );
        let _ = (bp.is_open(p), bp.is_close(p), bp.excess(p), bp.rank0(p));
        assert_eq!(bp.rank1(p), ones, "{what}: rank1({p})");
        if p >= n {
            assert_eq!(
                (close, sibling, child),
                (None, None, None),
                "{what}: past end {p}"
            );
            continue;
        }
        if bits[p] {
            ones += 1;
        }
        assert_eq!(bp.is_open(p), bits[p], "{what}: is_open({p})");
        assert_eq!(i64::from(bp.excess(p)), excess[p], "{what}: excess({p})");
        if !balanced {
            continue;
        }
        if bits[p] {
            assert_eq!((close, open), (mate[p], None), "{what}: find_close({p})");
            assert_eq!(enclose, parent[p], "{what}: enclose({p})");
            let want_sibling = mate[p].and_then(|c| (c + 1 < n && bits[c + 1]).then_some(c + 1));
            assert_eq!(sibling, want_sibling, "{what}: next_sibling({p})");
            assert_eq!(
                child,
                (p + 1 < n && bits[p + 1]).then_some(p + 1),
                "{what}: first_child({p})"
            );
        } else {
            assert_eq!((close, open), (None, mate[p]), "{what}: find_open({p})");
        }
    }
}

enum Frame<'a> {
    Elements(succinctly::json::light::JsonElements<'a>, bool),
    Fields(succinctly::json::light::JsonFields<'a>, bool),
}

/// Writes a value compactly, raw scalars as spelt, without recursion (documents nest thousands deep).
fn walk(value: StandardJson<'_>, out: &mut Vec<u8>, budget: &mut usize) {
    let mut stack: Vec<Frame<'_>> = Vec::new();
    let mut next = Some(value);
    loop {
        if let Some(value) = next.take() {
            *budget = budget.saturating_sub(1);
            match value {
                StandardJson::Array(elements) => {
                    out.push(b'[');
                    stack.push(Frame::Elements(elements, true));
                }
                StandardJson::Object(fields) => {
                    out.push(b'{');
                    stack.push(Frame::Fields(fields, true));
                }
                StandardJson::String(s) => {
                    out.extend_from_slice(s.raw_bytes());
                    let _ = s.as_str();
                }
                StandardJson::Number(n) => {
                    out.extend_from_slice(n.raw_bytes());
                    let _ = (n.as_i64(), n.as_f64());
                }
                StandardJson::Bool(b) => out.extend_from_slice(if b { b"true" } else { b"false" }),
                StandardJson::Null => out.extend_from_slice(b"null"),
                StandardJson::Error(e) => {
                    out.extend_from_slice(b"<error:");
                    out.extend_from_slice(e.as_bytes());
                    out.push(b'>');
                }
            }
        }
        if *budget == 0 {
            return;
        }
        match stack.last_mut() {
            None => return,
            Some(Frame::Elements(elements, first)) => match elements.next() {
                Some(element) => {
                    if !core::mem::replace(first, false) {
                        out.push(b',');
                    }
                    next = Some(element);
                }
                None => {
                    stack.pop();
                    out.push(b']');
                }
            },
            Some(Frame::Fields(fields, first)) => match fields.next() {
                Some(field) => {
                    if !core::mem::replace(first, false) {
                        out.push(b',');
                    }
                    match field.key() {
                        StandardJson::String(s) => out.extend_from_slice(s.raw_bytes()),
                        _ => out.extend_from_slice(b"<key>"),
                    }
                    out.push(b':');
                    next = Some(field.value());
                }
                None => {
                    stack.pop();
                    out.push(b'}');
                }
            },
        }
    }
}

/// Builds the index over `text` and checks it; `well_formed` texts must also read back as written.
fn check(text: &[u8], well_formed: bool) {
    let what = String::from_utf8_lossy(&text[..text.len().min(120)]).into_owned();
    let index = JsonIndex::build(text);
    let bp = index.bp();

    // The length is two bits per open; the stored bits are those of the builder the index
    // runs, with any closes the text lacks supplied as zeros.
    let reference = succinctly::json::simd::build_semi_index_standard(text).bp;
    let opens: usize = reference.iter().map(|w| w.count_ones() as usize).sum();
    assert_eq!(bp.len(), 2 * opens, "{what}: len");
    let bits = stored_bits(bp);
    for (p, &bit) in bits.iter().enumerate() {
        let want = reference
            .get(p / 64)
            .is_some_and(|w| (w >> (p % 64)) & 1 == 1);
        assert_eq!(bit, want, "{what}: bit {p}");
    }
    check_navigation(bp, &bits, &what);

    let mut out = Vec::new();
    let mut budget = 50_000;
    walk(index.root(text).value(), &mut out, &mut budget);
    if well_formed {
        assert_eq!(
            String::from_utf8_lossy(&out),
            String::from_utf8_lossy(text),
            "walk"
        );
    }
    // Cursors made at opens through the sequence, and at offsets through the text.
    let opens_at: Vec<usize> = (0..bp.len()).filter(|&p| bits[p]).collect();
    for &p in opens_at.iter().step_by(1 + opens_at.len() / 64) {
        let cursor = JsonCursor::from_bp_position(&index, text, p);
        let _ = (
            cursor.first_child(),
            cursor.next_sibling(),
            cursor.parent(),
            cursor.text_range(),
        );
        let mut out = Vec::new();
        let mut budget = 16;
        walk(cursor.value(), &mut out, &mut budget);
    }
    let root = index.root(text);
    for offset in (0..=text.len()).step_by(1 + text.len() / 16) {
        let _ = root.cursor_at_offset(offset);
    }
}

#[test]
fn the_issue_reproducer_has_its_closes_supplied_7482() {
    let mut text = b"{\"a\":".to_vec();
    text.extend(std::iter::repeat(b'[').take(44));
    text.extend_from_slice(b"1,\"b\":2}\n");
    check(&text, false);
    let index = JsonIndex::build(&text);
    // 1 object + 1 key + 44 arrays + 1, "b", 2: 49 opens, 98 bits, two words.
    assert_eq!(index.bp().len(), 98);
    assert_eq!(index.bp().words().len(), 2);
}

#[test]
fn deep_keyed_arrays_with_one_closer_at_every_depth_7482() {
    for depth in 0..270 {
        for tail in ["1,\"b\":2}", "1,\"b\",2]", ""] {
            let mut text = b"{\"a\":".to_vec();
            text.extend(std::iter::repeat(b'[').take(depth));
            text.extend_from_slice(tail.as_bytes());
            check(&text, false);
        }
    }
}

#[test]
fn mutated_documents_never_panic_and_balanced_ones_navigate_as_a_scan_does_7482() {
    let mut rng = Rng(7482);
    for case in 0..250 {
        let depth = match case % 10 {
            0 if case % 30 == 0 => 200 + rng.below(1_800),
            1..=3 => 30 + rng.below(170),
            _ => 1 + rng.below(30),
        };
        let mut doc = String::new();
        generate(&mut rng, depth, &mut doc);
        check(doc.as_bytes(), true);
        let mut text = doc.into_bytes();
        mutate(&mut rng, &mut text);
        check(&text, false);
    }
}

#[test]
fn bracket_strings_never_panic_7482() {
    let mut rng = Rng(0x7482);
    for _ in 0..400 {
        let len = 1 + rng.below(400);
        let text = bracket_string(&mut rng, len);
        check(&text, false);
    }
}
