//! #6377: an explicit key (`? a`) nested in the first item of an explicit key's own block sequence was
//! `inconsistent indentation` where yq reads it. #6340's helper for an explicit key opening a compact mapping
//! reads it now (the item took the `?` for a plain scalar before), so these rows pin the key-sequence shapes that
//! issue named, apart from #6340's own. Each row is a live yq v4.53.3 capture.

use crate::run_yq_stdin;

/// (document, `-o json -I0 .` output).
const ROWS: &[(&str, &str)] = &[
    ("? - ? a\n    : b\n", "{\"\":null}\n"),
    ("?\n  - ? a\n    : b\n", "{\"\":null}\n"),
    ("?\n- ? a\n  : b\n", "{\"\":null}\n"),
    ("? - ? a\n    : b\n  - c\n: v\n", "{\"\":\"v\"}\n"),
    ("?\n  - ? a\n    : b\n  - c\n: v\n", "{\"\":\"v\"}\n"),
    ("? - ? a\n  - ? b\n    : c\n: v\n", "{\"\":\"v\"}\n"),
    ("?\n- ? a\n  : b\n: v\n", "{\"\":\"v\"}\n"),
    (
        "? - ? a\n    : b\n: v\n? w\n: z\n",
        "{\"\":\"v\",\"w\":\"z\"}\n",
    ),
    // A later item, a second nested key, and a flow key.
    ("? - x\n  - ? c\n    : d\n: v\n", "{\"\":\"v\"}\n"),
    (
        "? - ? a\n    : b\n  - ? c\n    : d\n: v\n",
        "{\"\":\"v\"}\n",
    ),
    ("? - ? [x]\n    : b\n: v\n", "{\"\":\"v\"}\n"),
    // Unchanged: a nested key with no value of its own, a compact `? ?`, and the same key in a plain sequence.
    ("? - ? a\n: v\n", "{\"\":\"v\"}\n"),
    ("? ? a\n: c\n", "{\"\":\"c\"}\n"),
    ("- - ? a\n    : b\n", "[[{\"a\":\"b\"}]]\n"),
];

#[test]
fn test_an_explicit_key_nested_in_a_key_sequence_is_read_as_yq_reads_it_6377() {
    for (doc, expected) in ROWS {
        let (stdout, code) = run_yq_stdin(".", doc, &["-o", "json", "-I0"]).unwrap();
        assert_eq!((stdout.as_str(), code), (*expected, 0), "{doc:?}");
    }
}

/// yq refuses a tab after the nested `?` (`found character that cannot start any token`), and so do we.
#[test]
fn test_a_tab_after_the_nested_question_mark_stays_refused_6377() {
    let (stdout, code) =
        run_yq_stdin(".", "? - ?\ta\n    : b\n: v\n", &["-o", "json", "-I0"]).unwrap();
    assert_eq!((stdout.as_str(), code), ("", 1));
}
