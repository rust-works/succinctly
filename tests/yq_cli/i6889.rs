//! #6889: a block mapping has no style of its own, so `style` is `""` for one whose first key is
//! written in quotes: its text begins where that key does, and the key's quote is not the mapping's.
//! A quoted scalar value, a flow collection and a mapping inside a sequence item answer as before.
//! Every row is captured from yq v4.53.3: the source document, the filter and the whole output
//! (`-o=json -I0`).

use anyhow::Result;

use crate::run_yq_stdin;

const ROWS: &[(&str, &str, &str)] = &[
    ("a:\n  'k': 1\n", ".a | style", "\"\"\n"),
    ("a:\n  \"k\": 1\n", ".a | style", "\"\"\n"),
    ("a:\n  'k': 1\n  'j': 2\n", ".a | style", "\"\"\n"),
    ("a:\n  \"x y\": [1]\n", ".a | style", "\"\"\n"),
    ("a:\n  &x 'k': 1\n", ".a | style", "\"\"\n"),
    ("a:\n  !foo 'k': 1\n", ".a | style", "\"\"\n"),
    ("- 'k': 1\n", ".[0] | style", "\"\"\n"),
    ("a:\n  'k': 1\n", "[.. | style]", "[\"\",\"\",\"\"]\n"),
    ("'a':\n  'b': 1\n", "[.. | style]", "[\"\",\"\",\"\"]\n"),
    ("a:\n  \"k\":\n    'j': 1\n", "[.. | style]", "[\"\",\"\",\"\",\"\"]\n"),
    ("a:\n  'k': \"v\"\n", "[.. | style]", "[\"\",\"\",\"double\"]\n"),
    ("a:\n  'k': 1\n", ".a.k | style", "\"\"\n"),
    ("'a': 1\n", ". | style", "\"\"\n"),
    ("a:\n  - 'k': 1\n", ".a | style", "\"\"\n"),
    ("a:\n  - 'x'\n  - \"y\"\n", "[.a[] | style]", "[\"single\",\"double\"]\n"),
    ("a: ['k']\n", ".a | style", "\"flow\"\n"),
    ("a: {'k': 1}\n", ".a | style", "\"flow\"\n"),
    ("a: \"x\"\n", ".a | style", "\"double\"\n"),
    ("a: 'x'\n", ".a | style", "\"single\"\n"),
    ("a:\n  ? 'k'\n  : 1\n", ".a | style", "\"\"\n"),
    ("- generic [ref=e5]:\n  - link \"Login bug\" [ref=e16] [cursor=pointer]:\n    - /url: /wiki\n  - 'row \"x\"': 1\n", "[.. | select(style == \"single\")] | length", "0\n"),
    ("a:\n  \"k\": 1\n  \"j\": \"v\"\n", "[.. | select(style == \"double\")] | length", "1\n"),
];

#[test]
fn a_block_mapping_with_a_quoted_first_key_has_no_style_6889() -> Result<()> {
    for (input, filter, want) in ROWS {
        let (out, code) = run_yq_stdin(filter, input, &["-o=json", "-I0"])?;
        assert_eq!(code, 0, "{filter} over {input:?}");
        assert_eq!(out, *want, "{filter} over {input:?}");
    }
    Ok(())
}
