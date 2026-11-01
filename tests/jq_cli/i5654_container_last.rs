//! #5654: junk after a container that is its parent's last child (`[[1]@]`, `{"a":[1]@}`, `[{}@]`),
//! and a closer of the wrong kind there (`[[1]},2]`), were read as whitespace: only a last child
//! that was a scalar had the gap before the closing bracket checked. jq 1.7.1 refuses each as a
//! parse error, whatever the filter. Documents and messages were captured from jq 1.7.1
//! (`/usr/bin/jq`). `map(f)` is `i5654_lazy_map.rs`; a consumer that stops early still skips the check.

use anyhow::Result;

use crate::run_jq_full;

/// `(document, stderr)`; every row exits 5 with nothing on stdout.
const REFUSED: &[(&str, &str)] = &[
    (
        "[[1]@]",
        "jq: parse error: Invalid numeric literal at line 1, column 6\n",
    ),
    (
        "[1,[2]\x0c]",
        "jq: parse error: Invalid numeric literal at line 1, column 8\n",
    ),
    (
        "[{}@]",
        "jq: parse error: Invalid numeric literal at line 1, column 5\n",
    ),
    (
        "[[]@]",
        "jq: parse error: Invalid numeric literal at line 1, column 5\n",
    ),
    (
        "[1,{}@]",
        "jq: parse error: Invalid numeric literal at line 1, column 7\n",
    ),
    (
        "{\"a\":[1]@}",
        "jq: parse error: Invalid numeric literal at line 1, column 10\n",
    ),
    (
        "{\"a\":{\"b\":1}@}",
        "jq: parse error: Invalid numeric literal at line 1, column 14\n",
    ),
    (
        "{\"a\":{}@}",
        "jq: parse error: Invalid numeric literal at line 1, column 9\n",
    ),
    (
        "{\"a\":1,\"b\":[]@}",
        "jq: parse error: Invalid numeric literal at line 1, column 15\n",
    ),
    (
        "[[1]},2]",
        "jq: parse error: Objects must consist of key:value pairs at line 1, column 5\n",
    ),
    (
        "[[1],]",
        "jq: parse error: Expected another array element at line 1, column 6\n",
    ),
    (
        "{\"a\":[1],}",
        "jq: parse error: Expected another key-value pair at line 1, column 10\n",
    ),
];

/// Junk below the container-last child of the root: only a walk of the whole document reaches it,
/// so `.length` and `keys`, which read the root's own frame, do not.
const DEEP: &[(&str, &str)] = &[
    (
        "[{\"a\":1},{\"b\":[2]#}]",
        "jq: parse error: Invalid numeric literal at line 1, column 19\n",
    ),
    (
        "[{\"a\":[1]@}]",
        "jq: parse error: Invalid numeric literal at line 1, column 11\n",
    ),
    (
        "[[[1]@]]",
        "jq: parse error: Invalid numeric literal at line 1, column 7\n",
    ),
];

/// Whitespace before the closing bracket of a container-last child is still whitespace.
const ACCEPTED: &[(&str, &str)] = &[
    ("[[1] ]", "[[1]]\n"),
    ("[{} ]", "[{}]\n"),
    ("[[1] , [2] \n]", "[[1],[2]]\n"),
    (
        "{\"a\":[1] , \"b\":{\"c\":[2]} \t}",
        "{\"a\":[1],\"b\":{\"c\":[2]}}\n",
    ),
    ("[[],{}]", "[[],{}]\n"),
    ("[[[]] ]", "[[[]]]\n"),
    ("{\"a\":{}\n}", "{\"a\":{}}\n"),
];

#[test]
fn test_junk_after_a_container_last_child_is_refused_5654() -> Result<()> {
    for (document, stderr) in REFUSED {
        for filter in [".", "length", "tojson", "keys"] {
            let (out, err, status) = run_jq_full(&["-c", filter], Some(document))?;
            assert_eq!(
                (out.as_str(), err.as_str(), status),
                ("", *stderr, 5),
                "{filter} over {document:?}"
            );
        }
    }
    Ok(())
}

#[test]
fn test_junk_below_a_container_last_child_is_refused_by_a_whole_walk_5654() -> Result<()> {
    for (document, stderr) in DEEP {
        for filter in [".", "tojson"] {
            let (out, err, status) = run_jq_full(&["-c", filter], Some(document))?;
            assert_eq!(
                (out.as_str(), err.as_str(), status),
                ("", *stderr, 5),
                "{filter} over {document:?}"
            );
        }
    }
    Ok(())
}

#[test]
fn test_whitespace_after_a_container_last_child_is_accepted_5654() -> Result<()> {
    for (document, stdout) in ACCEPTED {
        let (out, err, status) = run_jq_full(&["-c", "."], Some(document))?;
        assert_eq!(
            (out.as_str(), err.as_str(), status),
            (*stdout, "", 0),
            "{document:?}"
        );
    }
    Ok(())
}
