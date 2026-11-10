//! #6847: a compact mapping that opens on a `?` or `:` line and leaves its key's value to the next
//! line takes a block sequence written at the key's own column as that value (`? a` / `: k:` /
//! `  - 1` is `{"a":{"k":[1]}}`), as it does under a plain key or a dash. The check that refuses a
//! sequence after a completed `key: value` entry (#4530) read the previous line's key without
//! walking past the `?` or `:`, took its column for the marker's, and refused the valid document.
//! Every answer was captured from yq v4.53.3 (`-o=json -I=0`); `None` is a document yq refuses, and
//! the last rows are the refusals that have to stand.

use anyhow::Result;

use crate::run_yq_stdin;

/// (document, yq's answer or `None` for a refusal).
const ROWS: &[(&str, Option<&str>)] = &[
    ("? k:\n  - 1\n", Some("{\"\":null}")),
    ("? k:\n  - 1\n: v\n", Some("{\"\":\"v\"}")),
    ("? a\n: k:\n  - 1\n", Some("{\"a\":{\"k\":[1]}}")),
    (
        "? a\n: k:\n  - 1\n  m: 2\n",
        Some("{\"a\":{\"k\":[1],\"m\":2}}"),
    ),
    ("? a\n: k: &x\n  - 1\n", Some("{\"a\":{\"k\":[1]}}")),
    ("? a\n: - k:\n    - 1\n", Some("{\"a\":[{\"k\":[1]}]}")),
    ("- ? k:\n    - 1\n", Some("[{\"\":null}]")),
    ("a:\n  ? k:\n    - 1\n", Some("{\"a\":{\"\":null}}")),
    ("? k:\n  - 1\n  - 2\n  - 3\n", Some("{\"\":null}")),
    ("? k: &x\n  - 1\n", Some("{\"\":null}")),
    ("? k: # c\n  - 1\n: v\n", Some("{\"\":\"v\"}")),
    ("? - a\n  - b\n: k:\n  - 1\n", Some("{\"\":{\"k\":[1]}}")),
    ("? k:\n  -\n  - 1\n", Some("{\"\":null}")),
    ("? k:\n  - 1\n? j:\n  - 2\n", Some("{\"\":null,\"\":null}")),
    ("- ? k:\n    - 1\n  : v\n", Some("[{\"\":\"v\"}]")),
    (
        "? a\n: k: v\n  m:\n  - 1\n",
        Some("{\"a\":{\"k\":\"v\",\"m\":[1]}}"),
    ),
    ("? a\n:\n  k:\n  - 1\n", Some("{\"a\":{\"k\":[1]}}")),
    // a bare `?` or `:` that leaves its node to the lines below (test suite 6PBE)
    (
        "---\n?\n- a\n- b\n:\n- c\n- d\n",
        Some("{\"\":[\"c\",\"d\"]}"),
    ),
    ("?\n- a\n:\n- c\n", Some("{\"\":[\"c\"]}")),
    ("? a\n: &x\n- c\n", Some("{\"a\":[\"c\"]}")),
    ("?\n- a\n: k:\n  - 1\n", Some("{\"\":{\"k\":[1]}}")),
    ("? k:\n:\n- 1\n", Some("{\"\":[1]}")),
    ("- ?\n  - a\n  :\n  - c\n", Some("[{\"\":[\"c\"]}]")),
    ("? k: 1\n  - 2\n", None),
    ("? k: x\n- 1\n", None),
    ("? a\n: k: 1\n  - 2\n", None),
    ("? k:\n  m: 2\n  - 1\n", None),
    ("? a\n: k:\n  m: 1\n  - 2\n", None),
    ("? k:\n- 1\n", None),
    ("? a\n: - k: 1\n    - 2\n", None),
    ("- ? k: 1\n    - 2\n", None),
];

#[test]
fn a_zero_indented_sequence_is_the_value_of_a_compact_mapping_after_an_explicit_marker_6847(
) -> Result<()> {
    for &(input, expected) in ROWS {
        let (out, code) = run_yq_stdin(".", input, &["-o=json", "-I=0"])?;
        match expected {
            Some(want) => {
                assert_eq!(code, 0, "{input:?}: {out}");
                assert_eq!(out.trim_end(), want, "{input:?}");
            }
            None => {
                assert_ne!(code, 0, "{input:?}: {out}");
                assert_eq!(out.trim_end(), "", "{input:?}");
            }
        }
    }
    Ok(())
}
