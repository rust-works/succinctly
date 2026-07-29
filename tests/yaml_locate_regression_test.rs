//! Regression test for bug #26: YAML at_offset returns incorrect nodes
//!
//! The bug was that at_offset would return the wrong node due to incorrect
//! IB-to-BP mapping. For example, offset 12 ("age" key) was returning
//! "name" instead.
//!
//! Note: YAML documents are wrapped in a virtual root sequence, so paths
//! include `.[0]` to access the first (and typically only) document.

use succinctly::yaml::{locate_offset, locate_offset_detailed, YamlIndex};

/// Renders a byte string with any non-ASCII-printable bytes visible in
/// assertion output.
struct Text<'a>(&'a [u8]);

impl core::fmt::Debug for Text<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{:?}", String::from_utf8_lossy(self.0))
    }
}

#[test]
fn test_locate_offset_simple_mapping() {
    // The exact example from the bug report (issue #26)
    let yaml = b"name: Alice\nage: 30\nactive: true";
    //           0123456789012345678901234567890123
    //                     1111111111222222222233333

    let index = YamlIndex::build(yaml).unwrap();

    // Test key positions - these are the positions where IB bits are set
    assert_eq!(
        locate_offset(&index, yaml, 0),
        Some(".[0].name".to_string()),
        "Offset 0 ('n' in 'name') should return .[0].name"
    );
    assert_eq!(
        locate_offset(&index, yaml, 12),
        Some(".[0].age".to_string()),
        "Offset 12 ('a' in 'age') should return .[0].age"
    );
    assert_eq!(
        locate_offset(&index, yaml, 20),
        Some(".[0].active".to_string()),
        "Offset 20 ('a' in 'active') should return .[0].active"
    );

    // Test value positions - these should return the containing key's path
    assert_eq!(
        locate_offset(&index, yaml, 6),
        Some(".[0].name".to_string()),
        "Offset 6 ('A' in 'Alice') should return .[0].name"
    );
    assert_eq!(
        locate_offset(&index, yaml, 17),
        Some(".[0].age".to_string()),
        "Offset 17 ('3' in '30') should return .[0].age"
    );
    assert_eq!(
        locate_offset(&index, yaml, 28),
        Some(".[0].active".to_string()),
        "Offset 28 ('t' in 'true') should return .[0].active"
    );

    // Test positions inside values
    assert_eq!(
        locate_offset(&index, yaml, 7),
        Some(".[0].name".to_string()),
        "Offset 7 ('l' in 'Alice') should return .[0].name"
    );
    assert_eq!(
        locate_offset(&index, yaml, 18),
        Some(".[0].age".to_string()),
        "Offset 18 ('0' in '30') should return .[0].age"
    );
}

#[test]
fn test_locate_offset_detailed_byte_ranges() {
    let yaml = b"name: Alice\nage: 30";
    let index = YamlIndex::build(yaml).unwrap();

    // Test detailed info at offset 0 (name key)
    let result = locate_offset_detailed(&index, yaml, 0);
    assert!(result.is_some(), "Should get detailed result for offset 0");
    let info = result.unwrap();
    assert_eq!(
        info.expression, ".[0].name",
        "Expression should be .[0].name"
    );
    assert_eq!(
        info.byte_range,
        (0, 4),
        "Byte range should be (0, 4) for 'name'"
    );

    // Test detailed info at offset 12 (age key)
    let result = locate_offset_detailed(&index, yaml, 12);
    assert!(result.is_some(), "Should get detailed result for offset 12");
    let info = result.unwrap();
    assert_eq!(info.expression, ".[0].age", "Expression should be .[0].age");
    assert_eq!(
        info.byte_range,
        (12, 15),
        "Byte range should be (12, 15) for 'age'"
    );
}

#[test]
fn test_bug_26_regression() {
    // This is the exact bug from issue #26:
    // "When querying byte offset 12 (the 'a' in 'age'), the system incorrectly
    // returns the 'name' node instead."
    //
    // Before the fix, offset 12 returned "name" (byte_range [0, 4])
    // After the fix, offset 12 returns "age" (byte_range [12, 15])

    let yaml = b"name: Alice\nage: 30\nactive: true";
    let index = YamlIndex::build(yaml).unwrap();

    let result = locate_offset_detailed(&index, yaml, 12);
    assert!(result.is_some(), "Should find node at offset 12");
    let info = result.unwrap();

    // The critical assertion: offset 12 should NOT return "name"
    assert_ne!(
        info.byte_range,
        (0, 4),
        "Bug #26: offset 12 should NOT return 'name' (byte_range [0, 4])"
    );

    // It should return "age"
    assert_eq!(
        info.byte_range,
        (12, 15),
        "Offset 12 should return 'age' (byte_range [12, 15])"
    );
    assert_eq!(
        info.expression, ".[0].age",
        "Expression should be .[0].age, not .[0].name"
    );
}

#[test]
fn test_locate_offset_block_sequence() {
    // Block sequences exercise `is_seq_item`: `path_to_bp` walks up through the
    // sequence-item wrapper nodes (which are derived from the leading `- ` in the
    // text, not a bitvector) and skips them, emitting `[n]` index components.
    //
    //   fruits:\n  - apple\n  - banana\ntop:\n  - - 1\n    - 2\n
    //   0000000000 111111111 12222222222 2333 33333334 44444444 4
    //   0123456789 012345678 90123456789 0123 45678901 23456789 0
    let yaml = b"fruits:\n  - apple\n  - banana\ntop:\n  - - 1\n    - 2\n";
    let index = YamlIndex::build(yaml).unwrap();

    // Offsets inside the two `fruits` items resolve to indexed paths.
    assert_eq!(
        locate_offset(&index, yaml, 12), // 'a' in "apple"
        Some(".[0].fruits[0]".to_string()),
    );
    assert_eq!(
        locate_offset(&index, yaml, 22), // 'b' in "banana"
        Some(".[0].fruits[1]".to_string()),
    );

    // Nested block sequence: walking up passes through two seq-item wrappers.
    assert_eq!(
        locate_offset(&index, yaml, 40), // '1'
        Some(".[0].top[0][0]".to_string()),
    );
    assert_eq!(
        locate_offset(&index, yaml, 48), // '2'
        Some(".[0].top[0][1]".to_string()),
    );
}

/// A block-context plain scalar containing `,`, `]`, or `}` is not truncated
/// by locate (#434). YAML's `ns-plain-char` reserves these bytes as flow
/// indicators only inside a flow collection - in block context they are
/// ordinary scalar content (e.g. `note: hello, world`), and `syq` already
/// printed the value in full. `find_scalar_end` (`YamlCursor::raw_bytes`'s
/// re-derivation of a scalar's extent from text, used by `at_offset` and
/// `syq-locate`) broke on these bytes unconditionally regardless of context,
/// despite its own comment calling them "flow context delimiters" - the same
/// eval/locate divergence shape as #370, just reached through a flow
/// indicator instead of a missing tab.
#[test]
fn the_located_byte_range_is_not_truncated_by_a_flow_indicator_in_block_context() {
    // (document, offset to locate, the bytes the reported range must cover)
    for (yaml, offset, expected) in [
        (&b"note: hello, world\n"[..], 6, &b"hello, world"[..]),
        (b"item: use a[0] here\n", 6, b"use a[0] here"),
        (b"note: use {key} here\n", 6, b"use {key} here"),
    ] {
        let index = YamlIndex::build(yaml).expect("parses");
        let found = locate_offset_detailed(&index, yaml, offset)
            .unwrap_or_else(|| panic!("offset {offset} of {:?} located nothing", Text(yaml)));
        let (start, end) = found.byte_range;
        assert_eq!(
            &yaml[start..end],
            expected,
            "offset {offset} of {:?}: range {:?} covers {:?}",
            Text(yaml),
            found.byte_range,
            Text(&yaml[start..end])
        );
    }
}

/// The flow-context control: inside an actual flow collection, `,` and `]`
/// still correctly terminate the element they follow - the fix must not make
/// `find_scalar_end` blind to flow indicators altogether, only context-aware.
#[test]
fn the_located_byte_range_still_stops_at_a_flow_indicator_in_flow_context() {
    let yaml = b"[a, b]\n";
    let index = YamlIndex::build(yaml).expect("parses");
    let found = locate_offset_detailed(&index, yaml, 1)
        .unwrap_or_else(|| panic!("offset 1 of {:?} located nothing", Text(yaml)));
    let (start, end) = found.byte_range;
    assert_eq!(&yaml[start..end], b"a", "the first flow sequence element");
}
