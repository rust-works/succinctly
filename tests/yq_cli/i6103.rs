//! #6103: yq's `unique` keys an element on its text, so `~`, `null`, `Null` and an empty value are
//! four elements, and a container holding one is keyed by those spellings as well. Every expected
//! output is what yq v4.53.3 prints.

use anyhow::Result;

use crate::run_yq_stdin;

/// (document, filter, expected stdout).
const ROWS: &[(&str, &str, &str)] = &[
    (
        "- ~\n- null\n- ~\n- 1\n- null\n",
        "unique",
        "- ~\n- null\n- 1\n",
    ),
    (
        "- ~\n- null\n- ~\n- 1\n- null\n- Null\n- NULL\n-\n- \"\"\n- \"null\"\n- \"~\"\n",
        "unique",
        "- ~\n- null\n- 1\n- Null\n- NULL\n-\n",
    ),
    (
        "- ~\n- null\n- ~\n- 1\n",
        "unique_by(.)",
        "- ~\n- null\n- 1\n",
    ),
    ("- ~\n- null\n", "unique | length", "2\n"),
    ("- ~\n- ~\n", "unique", "- ~\n"),
    ("- null\n- null\n", "unique", "- null\n"),
    ("- [~]\n- [null]\n- [~]\n", "unique", "- [~]\n- [null]\n"),
    (
        "- {a: ~}\n- {a: null}\n- {a: ~}\n- [Null, ~]\n- [~, Null]\n- [Null, ~]\n",
        "unique",
        "- {a: ~}\n- {a: null}\n- [Null, ~]\n- [~, Null]\n",
    ),
    (
        "- {a: ~, b: [null, ~]}\n- {a: ~, b: [~, null]}\n- {a: ~, b: [null, ~]}\n",
        "unique",
        "- {a: ~, b: [null, ~]}\n- {a: ~, b: [~, null]}\n",
    ),
    (
        "- [~, \"null\"]\n- [null, \"null\"]\n",
        "unique",
        "- [~, \"null\"]\n- [null, \"null\"]\n",
    ),
    (
        "- [[~], [null]]\n- [[null], [~]]\n- [[~], [null]]\n",
        "unique",
        "- [[~], [null]]\n- [[null], [~]]\n",
    ),
    ("- a\n- ~\n- b\n- null\n", "unique | length", "4\n"),
    ("- [1]\n- [1]\n", "unique", "- [1]\n"),
    ("- \"\"\n-\n", "unique", "- \"\"\n"),
    ("-\n- \"\"\n", "unique", "-\n"),
    ("- [&a ~]\n- [null]\n", "unique | length", "2\n"),
];

#[test]
fn test_unique_keeps_null_spellings_apart_as_yq_does_6103() -> Result<()> {
    for &(document, filter, expected) in ROWS {
        let (output, code) = run_yq_stdin(filter, document, &[])?;
        assert_eq!(code, 0, "{filter} on {document:?}");
        assert_eq!(output, expected, "{filter} on {document:?}");
    }
    Ok(())
}

/// JSON has no plain scalars, so a `null` has one spelling and `unique` keeps one of them, as yq does.
#[test]
fn test_unique_of_json_nulls_keeps_one_6103() -> Result<()> {
    let (output, code) = run_yq_stdin("unique", "[null, null]", &["-p=json", "-o=json", "-I=0"])?;
    assert_eq!(code, 0);
    assert_eq!(output, "[null]\n");
    Ok(())
}

/// Where the key is not a node of the document, `~` and `null` are still one key here and two in
/// yq (the first two columns are this tool's answer, pinned so the limitations page stays true;
/// the last is yq v4.53.3's).
const GAPS: &[(&str, &str, &str, &str)] = &[
    (
        "- a: ~\n- a: null\n",
        "unique_by(.a) | length",
        "1\n",
        "2\n",
    ),
    ("- ~\n- null\n", "group_by(.) | length", "1\n", "2\n"),
    ("- ~\n- null\n", "map(.) | unique | length", "1\n", "2\n"),
    ("- ~\n- null\n", ". + [] | unique | length", "1\n", "2\n"),
    ("- &a ~\n- *a\n- null\n", "unique | length", "1\n", "3\n"),
];

#[test]
fn test_unique_without_a_document_node_still_merges_null_spellings_6103() -> Result<()> {
    for &(document, filter, here, _yq) in GAPS {
        let (output, code) = run_yq_stdin(filter, document, &[])?;
        assert_eq!(code, 0, "{filter} on {document:?}");
        assert_eq!(output, here, "{filter} on {document:?}");
    }
    Ok(())
}
