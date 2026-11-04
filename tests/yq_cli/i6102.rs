//! #6102: `pivot` pads a row that is too short with an empty node, which yq prints as nothing
//! (`-` with no value, `key:` bare), and moves every other cell with its comment, quoting, anchor
//! and tag; `succinctly yq` padded with a `null` that printed as `null` and rebuilt the cells plain.
//! Every expected output is what yq v4.53.3 prints.

use anyhow::Result;

use crate::run_yq_stdin;

/// (document, filter, expected stdout).
const ROWS: &[(&str, &str, &str)] = &[
    (
        "- [foo, bar]\n- [sis]\n",
        "pivot",
        "- - foo\n  - sis\n- - bar\n  -\n",
    ),
    (
        "- {a: 1, b: ~}\n- {a: 2, c: 3}\n",
        "pivot",
        "a:\n  - 1\n  - 2\nb:\n  - ~\n  -\nc:\n  -\n  - 3\n",
    ),
    (
        "- {a: 1, b: 2}\n- {a: 3}\n",
        "pivot",
        "a:\n  - 1\n  - 3\nb:\n  - 2\n  -\n",
    ),
    // A cell that is in the input keeps its spelling, comment, quoting, anchor, alias and tag.
    (
        "- [~, bar]\n- [sis]\n",
        "pivot",
        "- - ~\n  - sis\n- - bar\n  -\n",
    ),
    (
        "- - a # ca\n  - b # cb\n- - c # cc\n",
        "pivot",
        "- - a # ca\n  - c # cc\n- - b # cb\n  -\n",
    ),
    (
        "- - !!str a\n  - \"b\"\n- - &x c\n  - *x\n",
        "pivot",
        "- - !!str a\n  - &x c\n- - \"b\"\n  - *x\n",
    ),
    (
        "- - a\n  # standalone\n  - b\n- - c\n",
        "pivot",
        "- - a\n  - c\n- # standalone\n  - b\n  -\n",
    ),
    (
        "- {\"a\": 1, b: 2}\n- {&k a: 3, c: 4 # cc\n  }\n",
        "pivot",
        "a:\n  - 1\n  - 3\nb:\n  - 2\n  -\nc:\n  -\n  - 4 # cc\n",
    ),
    // The mapping's keys are new strings, so one that reads as another type stays quoted.
    (
        "- {\"1\": a}\n- {2: b}\n",
        "pivot",
        "\"1\":\n  - a\n  -\n\"2\":\n  -\n  - b\n",
    ),
    (
        "- {true: a}\n- {~: b, null: c, x y: d}\n",
        "pivot",
        "\"true\":\n  - a\n  -\n\"~\":\n  -\n  - b\n\"null\":\n  -\n  - c\nx y:\n  -\n  - d\n",
    ),
    // A pivot with no short row is as it was.
    (
        "- [a, b]\n- [c, d]\n",
        "pivot",
        "- - a\n  - c\n- - b\n  - d\n",
    ),
    ("- [1, 2]\n- [3]\n", "pivot", "- - 1\n  - 3\n- - 2\n  -\n"),
    ("- [a, b]\n- []\n", "pivot", "- - a\n  -\n- - b\n  -\n"),
    ("- {}\n- {a: 1}\n", "pivot", "a:\n  -\n  - 1\n"),
    // The padding is still `!!null` to the next stage.
    ("- [foo, bar]\n- [sis]\n", "pivot | .[1]", "- bar\n-\n"),
    (
        "- [foo, bar]\n- [sis]\n",
        "pivot | .[1][1] | tag",
        "!!null\n",
    ),
    (
        "- [foo, bar]\n- [sis]\n",
        "pivot | .[1][1] = \"x\"",
        "- - foo\n  - sis\n- - bar\n  - x\n",
    ),
];

#[test]
fn test_pivot_pads_a_short_row_with_an_empty_node_as_yq_does_6102() -> Result<()> {
    for &(document, filter, expected) in ROWS {
        let (output, code) = run_yq_stdin(filter, document, &[])?;
        assert_eq!(code, 0, "{filter} on {document:?}");
        assert_eq!(output, expected, "{filter} on {document:?}");
    }
    Ok(())
}

/// The padding reaches a data format as `null`, as in yq, and `-P` keeps the empty node.
#[test]
fn test_pivot_padding_is_null_in_json_and_empty_under_p_6102() -> Result<()> {
    let (output, code) = run_yq_stdin("pivot", "- {a: 1, b: 2}\n- {a: 3}\n", &["-o=json", "-I=0"])?;
    assert_eq!(code, 0);
    assert_eq!(output, "{\"a\":[1,3],\"b\":[2,null]}\n");
    let (output, code) = run_yq_stdin("pivot", "- [foo, bar]\n- [sis]\n", &["-P"])?;
    assert_eq!(code, 0);
    assert_eq!(output, "- - foo\n  - sis\n- - bar\n  -\n");
    Ok(())
}

/// Where the `pivot` is not the whole filter, the padding is a `null` that prints as `null`: the
/// presentation of a built value is read only from a filter that reshapes the document and nothing
/// else (the first two columns are this tool's answer, pinned so the limitations page stays true;
/// the last is yq v4.53.3's).
const GAPS: &[(&str, &str, &str, &str)] = &[
    (
        "- [foo, bar]\n- [sis]\n",
        "[pivot]",
        "- - - foo\n    - sis\n  - - bar\n    - null\n",
        "- - - foo\n    - sis\n  - - bar\n    -\n",
    ),
    ("- [foo, bar]\n- [sis]\n", "pivot | .[1][1]", "null\n", "\n"),
];

#[test]
fn test_pivot_inside_another_filter_still_pads_with_null_6102() -> Result<()> {
    for &(document, filter, ours, _yq) in GAPS {
        let (output, code) = run_yq_stdin(filter, document, &[])?;
        assert_eq!(code, 0, "{filter} on {document:?}");
        assert_eq!(output, ours, "{filter} on {document:?}");
    }
    Ok(())
}
