//! #7306: `min_by(f)`/`max_by(f)` over a document array take one walk that keeps only the winner
//! and its key, and `add` folds without collecting the array's cursors first. Each row is
//! `(document, filter, stdout, stderr, exit status)` as jq 1.7.1 (`/usr/bin/jq`) answers: ties (the
//! first of equal minima and the last of equal maxima win, also between equal spellings), a winner
//! that is overtaken later (its key must move with it), NaN keys (#6008), the empty array, a key
//! filter with several outputs, none, or an error on the first element that raises, keys of mixed types, a non-array input, the same over a computed array
//! (which the full evaluator answers), and `add` over every kind of element it concatenates or
//! refuses.

use anyhow::Result;

use crate::run_jq_full;

const ROWS: &[(&str, &str, &str, &str, i32)] = &[
    (
        "[{\"a\":1,\"i\":0},{\"a\":1,\"i\":1},{\"a\":0,\"i\":2},{\"a\":1,\"i\":3}]",
        "min_by(.a)",
        "{\"a\":0,\"i\":2}\n",
        "",
        0,
    ),
    (
        "[{\"a\":1,\"i\":0},{\"a\":1,\"i\":1},{\"a\":0,\"i\":2},{\"a\":1,\"i\":3}]",
        "max_by(.a)",
        "{\"a\":1,\"i\":3}\n",
        "",
        0,
    ),
    ("[1,1.0,1.00]", "min_by(.)", "1\n", "", 0),
    ("[1,1.0,1.00]", "max_by(.)", "1.00\n", "", 0),
    (
        "[123456789012345679,123456789012345678.9]",
        "[min_by(.), max_by(.)]",
        "[123456789012345678.9,123456789012345679]\n",
        "",
        0,
    ),
    ("[3,1,2]", "min_by(.)", "1\n", "", 0),
    ("[1,3,2]", "max_by(.)", "3\n", "", 0),
    ("[3,1,2]", "[.[]] | min_by(.)", "1\n", "", 0),
    ("[1,3,2]", "[.[]] | max_by(.)", "3\n", "", 0),
    (
        "[{\"a\":3},{\"a\":1},{\"a\":2}]",
        "[min_by(.a), max_by(.a)]",
        "[{\"a\":1},{\"a\":3}]\n",
        "",
        0,
    ),
    (
        "[1,2,3]",
        "min_by(if . == 2 then nan else . end)",
        "2\n",
        "",
        0,
    ),
    (
        "[1,2,3]",
        "max_by(if . == 2 then nan else . end)",
        "3\n",
        "",
        0,
    ),
    ("[1,2,3]", "[min_by(nan), max_by(nan)]", "[3,1]\n", "", 0),
    ("[]", "[min_by(.), max_by(.)]", "[null,null]\n", "", 0),
    ("[7]", "[min_by(.), max_by(.)]", "[7,7]\n", "", 0),
    (
        "[{\"a\":1,\"b\":5},{\"a\":1,\"b\":3},{\"a\":0,\"b\":9}]",
        "max_by(.a, .b)",
        "{\"a\":1,\"b\":5}\n",
        "",
        0,
    ),
    (
        "[{\"a\":1,\"b\":5},{\"a\":1,\"b\":3},{\"a\":0,\"b\":9}]",
        "min_by(.a, .b)",
        "{\"a\":0,\"b\":9}\n",
        "",
        0,
    ),
    (
        "[1,2,3]",
        "[min_by(empty), max_by(empty)]",
        "[1,3]\n",
        "",
        0,
    ),
    (
        "[null,false,true,0,\"a\",[],{}]",
        "[min_by(.), max_by(.)]",
        "[null,{}]\n",
        "",
        0,
    ),
    (
        "[{\"a\":1},{\"b\":2}]",
        "[min_by(.a), max_by(.a)]",
        "[{\"b\":2},{\"a\":1}]\n",
        "",
        0,
    ),
    (
        "[{\"k\":[2,1]},{\"k\":[2]},{\"k\":{\"x\":1}},{\"k\":\"s\"}]",
        "[min_by(.k), max_by(.k)]",
        "[{\"k\":\"s\"},{\"k\":{\"x\":1}}]\n",
        "",
        0,
    ),
    ("[[3,1],[2]]", "[.[] | max_by(.)]", "[3,2]\n", "", 0),
    (
        "[1,2]",
        "max_by(error)",
        "",
        "jq: error (at <stdin>:0) (not a string): 1\n",
        5,
    ),
    (
        "[1,2,3]",
        "max_by(if . == 2 then error(\"x\") else . end)",
        "",
        "jq: error (at <stdin>:0): x\n",
        5,
    ),
    (
        "[{\"a\":1},2,{\"a\":3}]",
        "min_by(.a)",
        "",
        "jq: error (at <stdin>:0): Cannot index number with string \"a\"\n",
        5,
    ),
    ("[1]", "try max_by(error(\"x\")) catch .", "\"x\"\n", "", 0),
    (
        "{\"a\":1}",
        "max_by(.)",
        "",
        "jq: error (at <stdin>:0): object ({\"a\":1}) and array ([[1]]) cannot be iterated over\n",
        5,
    ),
    (
        "\"abc\"",
        "min_by(.)",
        "",
        "jq: error (at <stdin>:0): Cannot iterate over string (\"abc\")\n",
        5,
    ),
    (
        "null",
        "max_by(.)",
        "",
        "jq: error (at <stdin>:0): Cannot iterate over null (null)\n",
        5,
    ),
    ("{\"a\":1}", "[max_by(.)?]", "[]\n", "", 0),
    (
        "[1,2]",
        "reduce .[] as $x (null; [$x, .] | max_by(.))",
        "2\n",
        "",
        0,
    ),
    (
        "[1,2]",
        "path(max_by(.))",
        "",
        "jq: error (at <stdin>:0): Invalid path expression with result 2\n",
        5,
    ),
    (
        "[{\"a\":1,\"i\":0},{\"a\":1,\"i\":1},{\"a\":0,\"i\":2},{\"a\":1,\"i\":3}]",
        "[.[]] | [min_by(.a), max_by(.a)]",
        "[{\"a\":0,\"i\":2},{\"a\":1,\"i\":3}]\n",
        "",
        0,
    ),
    (
        "[{\"a\":1,\"i\":0},{\"a\":1,\"i\":1},{\"a\":0,\"i\":2},{\"a\":1,\"i\":3}]",
        "map(.) | [min_by(.a), max_by(.a)]",
        "[{\"a\":0,\"i\":2},{\"a\":1,\"i\":3}]\n",
        "",
        0,
    ),
    (
        "[3,1,1.0,2]",
        ".[1:] | [min_by(.), max_by(.)]",
        "[1,2]\n",
        "",
        0,
    ),
    (
        "[1,2,3]",
        "[.[]] | [min_by(if . == 2 then nan else . end), max_by(if . == 2 then nan else . end)]",
        "[2,3]\n",
        "",
        0,
    ),
    (
        "[1,2,3]",
        "[.[]] | [min_by(empty), max_by(empty)]",
        "[1,3]\n",
        "",
        0,
    ),
    (
        "[{\"a\":1,\"b\":5},{\"a\":1,\"b\":3},{\"a\":0,\"b\":9}]",
        "[.[]] | [min_by(.a, .b), max_by(.a, .b)]",
        "[{\"a\":0,\"b\":9},{\"a\":1,\"b\":5}]\n",
        "",
        0,
    ),
    (
        "[1,2]",
        "[.[]] | max_by(error)",
        "",
        "jq: error (at <stdin>:0) (not a string): 1\n",
        5,
    ),
    (
        "[{\"a\":1},2,{\"a\":3}]",
        "[.[]] | min_by(.a)",
        "",
        "jq: error (at <stdin>:0): Cannot index number with string \"a\"\n",
        5,
    ),
    (
        "[]",
        "[.[]] | [min_by(.), max_by(.)]",
        "[null,null]\n",
        "",
        0,
    ),
    ("[1,2.5,-0]", "add", "3.5\n", "", 0),
    ("[null,1]", "add", "1\n", "", 0),
    ("[]", "add", "null\n", "", 0),
    ("[1,null,2]", "add", "3\n", "", 0),
    ("[\"a\",null,\"b\"]", "add", "\"ab\"\n", "", 0),
    ("[[1],[2]]", "add", "[1,2]\n", "", 0),
    ("[{\"a\":1},{\"b\":2}]", "add", "{\"a\":1,\"b\":2}\n", "", 0),
    (
        "[1,\"a\"]",
        "add",
        "",
        "jq: error (at <stdin>:0): number (1) and string (\"a\") cannot be added\n",
        5,
    ),
    (
        "[1,{\"a\":1},\"x\"]",
        "add",
        "",
        "jq: error (at <stdin>:0): number (1) and object ({\"a\":1}) cannot be added\n",
        5,
    ),
    ("{\"a\":1,\"b\":2}", "add", "3\n", "", 0),
    (
        "\"ab\"",
        "add",
        "",
        "jq: error (at <stdin>:0): Cannot iterate over string (\"ab\")\n",
        5,
    ),
];

#[test]
fn test_keyed_extremum_and_add_walk_answer_as_jq_7306() -> Result<()> {
    for (document, filter, stdout, stderr, code) in ROWS {
        let (out, err, status) = run_jq_full(&["-c", filter], Some(document))?;
        assert_eq!(
            (out.as_str(), err.as_str(), status),
            (*stdout, *stderr, *code),
            "{filter} over {document:?}"
        );
    }
    Ok(())
}
