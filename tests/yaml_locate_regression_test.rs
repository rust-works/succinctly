//! Regression test for bug #26: YAML at_offset returns incorrect nodes
//!
//! The bug was that at_offset would return the wrong node due to incorrect
//! IB-to-BP mapping. For example, offset 12 ("age" key) was returning
//! "name" instead.
//!
//! Note: YAML documents are wrapped in a virtual root sequence, so paths
//! include `.[0]` to access the first (and typically only) document.

use succinctly::yaml::{locate_offset, locate_offset_detailed, YamlIndex};

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
