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
