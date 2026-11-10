//! #6877: a `sort`, `sort_by`, `reverse`, `unique` or `unique_by` read of a flow-style sequence
//! prints it in flow style with its line comment, quoting and anchor, as yq does when it reorders
//! the node in place: the read is answered through its write form, `(NAV |= f) | NAV`. Every row
//! is captured from yq v4.53.3 (default YAML output). The block-style sequence, a reorder that is
//! only part of the read, and the write form are the controls that were right before.

use anyhow::Result;

use crate::run_yq_stdin;

const DOC: &str = "# head\na: [c, \"b\", 'a']\nb: []\nc: [{n: 2}, {n: 1}]\nd: [[3, 1], [2]]\ne: [1, a, 2.5, true, null]\nf:\n  - 3\n  - 1\ng: [3, 1] # tail\nh: &x [2, 1]\nj: [ 3 ,1 ,2 ]\nk: {l: [9, 8]}\n";

/// `(filter, output)`.
const ROWS: &[(&str, &str)] = &[
    (".a | sort", "['a', \"b\", c]\n"),
    (".a | reverse", "['a', \"b\", c]\n"),
    (".a | unique", "[c, \"b\", 'a']\n"),
    (".a | sort_by(length)", "[c, \"b\", 'a']\n"),
    (".a | (sort)", "['a', \"b\", c]\n"),
    (".b | sort", "[]\n"),
    (".c | sort_by(.n)", "[{n: 1}, {n: 2}]\n"),
    (".c | unique_by(.n)", "[{n: 2}, {n: 1}]\n"),
    (".d | sort", "[[3, 1], [2]]\n"),
    (".d | .[0] | sort", "[1, 3]\n"),
    (".d[0] | reverse", "[1, 3]\n"),
    (".e | sort", "[null, true, 1, 2.5, a]\n"),
    (".e | unique", "[1, a, 2.5, true, null]\n"),
    (".g | sort", "[1, 3] # tail\n"),
    (".g | reverse", "[1, 3] # tail\n"),
    (".h | sort", "&x [1, 2]\n"),
    (".j | sort", "[1, 2, 3]\n"),
    (".k.l | sort", "[8, 9]\n"),
    (".k | .l | reverse", "[8, 9]\n"),
    (".f | sort", "- 1\n- 3\n"),
    (".f | reverse", "- 1\n- 3\n"),
    (".a | sort | length", "3\n"),
    (".a | sort | .[0]", "a\n"),
    (".g |= sort", "# head\na: [c, \"b\", 'a']\nb: []\nc: [{n: 2}, {n: 1}]\nd: [[3, 1], [2]]\ne: [1, a, 2.5, true, null]\nf:\n  - 3\n  - 1\ng: [1, 3] # tail\nh: &x [2, 1]\nj: [3, 1, 2]\nk: {l: [9, 8]}\n"),
    ("(.g | sort) as $x | $x | length", "2\n"),
];

#[test]
fn test_a_reorder_read_of_a_flow_sequence_keeps_its_presentation_6877() -> Result<()> {
    for (filter, want) in ROWS {
        let (stdout, code) = run_yq_stdin(filter, DOC, &[])?;
        assert_eq!(code, 0, "exit code of {filter:?}");
        assert_eq!(&stdout, want, "{filter:?}");
    }
    Ok(())
}

/// `(document, filter, extra arguments, output)`: more documents, spellings and flags around the
/// same read; the block-style row and the other output formats are the controls that were right before.
const MORE_ROWS: &[(&str, &str, &[&str], &str)] = &[
    (
        "l:\n  - b\n  - a\n---\nl: [d, c]\n",
        ".l | sort",
        &[],
        "- a\n- b\n---\n[c, d]\n",
    ),
    ("l: [b, a]\n", ". | .l | sort", &[], "[a, b]\n"),
    ("[b, a] # c\n", ". | sort", &[], "[a, b] # c\n"),
    ("[b, a] # c\n", "sort", &[], "[a, b] # c\n"),
    ("l: [b, a]\n", "(.l | sort)", &[], "[a, b]\n"),
    ("l: [b, a]\n", ".l | sort", &["-N"], "[a, b]\n"),
    ("l: [b, a]\n", ".l | sort", &["-I=4"], "[a, b]\n"),
    ("l: [b, a]\n", ".l | sort", &["-r"], "[a, b]\n"),
    ("l: [b, a]\n", ".l | sort", &["-P"], "- a\n- b\n"),
    (
        "l: [b, a]\n",
        ".l | sort",
        &["-o=json", "-I=0"],
        "[\"a\",\"b\"]\n",
    ),
    ("l:\n  - b\n  - a\n", ".l | sort", &[], "- a\n- b\n"),
];

#[test]
fn test_a_reorder_read_of_a_flow_sequence_in_other_settings_6877() -> Result<()> {
    for (doc, filter, args, want) in MORE_ROWS {
        let (stdout, code) = run_yq_stdin(filter, doc, args)?;
        assert_eq!(code, 0, "exit code of {filter:?} {args:?}");
        assert_eq!(&stdout, want, "{filter:?} {args:?} over {doc:?}");
    }
    Ok(())
}
