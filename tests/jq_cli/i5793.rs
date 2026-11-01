//! #5793: an index or slice key that is a `map(f)` over a document value, with an `f` that raises,
//! raised nothing on the document route: no output, no diagnostic, exit 0, and a surrounding `try`
//! never saw the error. The key was read as an empty list. Each row reads a file or stdin (the
//! route that streams from the document); the answers, exit status and the number of errors
//! reported (jq: one) are captured from jq 1.7.1 (`/usr/bin/jq`).

use anyhow::Result;

use crate::run_jq_full;

const DOC: &str = "{\"a\":[1],\"b\":[1,2,3]}";

/// `(filter, stdout, exit status, errors reported)` over [`DOC`].
const ROWS: &[(&str, &str, i32, usize)] = &[
    (".[(.a|map(error(\"x\")))]", "", 5, 1),
    (".[(.a|map(error(\"x\"))):]", "", 5, 1),
    (".[(.a|map(error(\"x\"))):.]", "", 5, 1),
    (".[.a|map(error(\"x\"))]", "", 5, 1),
    (".[0:(.a|map(error(\"x\")))]", "", 5, 1),
    (".b[(.a|map(error(\"x\"))):]", "", 5, 1),
    (".b[:(.a|map(error(\"x\")))]", "", 5, 1),
    ("[.[(.a|map(error(\"x\")))]]", "", 5, 1),
    (".[(.a|map(error(\"x\")))] as $x | 1", "", 5, 1),
    // `?` suppresses the final index step, never an error computing the key.
    (".[(.a|map(error(\"x\")))]?", "", 5, 1),
    ("[.[(.a|map(error(\"x\")))]?]", "", 5, 1),
    // The error is the key's, and `try` sees it.
    ("try .[(.a|map(error(\"x\")))] catch \"c\"", "\"c\"\n", 0, 0),
    // A key before the failing one is indexed first.
    (".[(\"a\",(.a|map(error(\"x\"))))]", "[1]\n", 5, 1),
    (
        ".[(.b|map(if .==2 then error(\"x\") else . end))]",
        "",
        5,
        1,
    ),
    // A bound before the failing one is applied first, and the other control flows pass through.
    (".b[(0,(.a|map(error(\"x\")))):]", "[1,2,3]\n", 5, 1),
    (".b[:(0,(.a|map(error(\"x\"))))]", "[]\n", 5, 1),
    ("label $l | 1, .[(.a|map(break $l))], 2", "1\n", 0, 0),
    ("first(.[(.a|map(error(\"x\")))])", "", 5, 1),
    ("limit(1; .[(.a|map(error(\"x\")))])", "", 5, 1),
    (".[(.a|map(error(\"x\")))] as [$x] ?// $x | $x", "", 5, 1),
    // The target side already raised.
    ("(.a|map(error(\"x\")))[0]", "", 5, 1),
    ("(.a|map(error(\"x\")))[1:]", "", 5, 1),
    // A key that does not fail is unchanged.
    (".b[(.a|map(.))]", "[0]\n", 0, 0),
    (".[(.a|map(\"a\")|.[0])]", "[1]\n", 0, 0),
];

#[test]
fn a_failing_map_as_an_index_or_slice_key_raises_5793() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("in.json");
    std::fs::write(&path, format!("{DOC}\n"))?;
    for (filter, stdout, code, errors) in ROWS {
        let (out, err, status) = run_jq_full(&["-c", filter, path.to_str().unwrap()], None)?;
        assert_eq!(
            (out.as_str(), status, err.matches("jq: error").count()),
            (*stdout, *code, *errors),
            "{filter} over a file: {err}"
        );
        let (out, err, status) = run_jq_full(&["-c", filter], Some(DOC))?;
        assert_eq!(
            (out.as_str(), status, err.matches("jq: error").count()),
            (*stdout, *code, *errors),
            "{filter} over stdin: {err}"
        );
    }
    Ok(())
}
