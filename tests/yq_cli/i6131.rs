//! #6131: yq's `@csv` and `@tsv` are Go's `encoding/csv` over the node, not jq's encoders: a field is
//! quoted only where it needs it, a scalar input is its own text, an array of arrays is one record
//! each, and an array of flat objects is a header record of the first object's keys then one record
//! each. Every expected output is what yq v4.53.3 prints (`-o=json -I=0`).

use anyhow::Result;

use crate::{run_yq_stdin, run_yq_stdin_with_stderr};

/// (document, filter, expected stdout).
const ROWS: &[(&str, &str, &str)] = &[
    ("[{a: 1}, {a: 3, b: [4]}]\n", "@csv", "\"a\\n1\\n3\"\n"),
    ("[[\"a\"], [\"\"]]\n", "@csv", "\"a\"\n"),
    ("[\"\\tx\", \"y\"]\n", "@tsv", "\"\\\"\\tx\\\"\\ty\"\n"),
    ("[1, a, b]\n", "@csv", "\"1,a,b\"\n"),
    ("[1, a, b]\n", "@tsv", "\"1\\ta\\tb\"\n"),
    (
        "[a b, \"x,y\", \"q\\\"r\"]\n",
        "@csv",
        "\"a b,\\\"x,y\\\",\\\"q\\\"\\\"r\\\"\"\n",
    ),
    (
        "[a b, \"x,y\", \"q\\\"r\"]\n",
        "@tsv",
        "\"a b\\tx,y\\t\\\"q\\\"\\\"r\\\"\"\n",
    ),
    (
        "[\"lead sp\", \" x\", \"x \"]\n",
        "@csv",
        "\"lead sp,\\\" x\\\",x \"\n",
    ),
    (
        "[\"lead sp\", \" x\", \"x \"]\n",
        "@tsv",
        "\"lead sp\\t\\\" x\\\"\\tx \"\n",
    ),
    (
        "[true, 1.50, 0x1f, 1_0]\n",
        "@csv",
        "\"true,1.50,0x1f,1_0\"\n",
    ),
    (
        "[true, 1.50, 0x1f, 1_0]\n",
        "@tsv",
        "\"true\\t1.50\\t0x1f\\t1_0\"\n",
    ),
    ("[[1,2],[3]]\n", "@csv", "\"1,2\\n3\"\n"),
    ("[[1,2],[3]]\n", "@tsv", "\"1\\t2\\n3\"\n"),
    (
        "[{a: 1, b: 2}, {b: 5, a: 6}]\n",
        "@csv",
        "\"a,b\\n1,2\\n6,5\"\n",
    ),
    (
        "[{a: 1, b: 2}, {b: 5, a: 6}]\n",
        "@tsv",
        "\"a\\tb\\n1\\t2\\n6\\t5\"\n",
    ),
    ("[{a: 1, b: 2}, {a: 3}]\n", "@csv", "\"a,b\\n1,2\\n3,\"\n"),
    (
        "[{a: 1, b: 2}, {a: 3}]\n",
        "@tsv",
        "\"a\\tb\\n1\\t2\\n3\\t\"\n",
    ),
    ("[{a: 1}, {a: 3, b: 4}]\n", "@csv", "\"a\\n1\\n3\"\n"),
    ("[{a: 1}, {a: 3, b: 4}]\n", "@tsv", "\"a\\n1\\n3\"\n"),
    (
        "[{\"x,y\": \"q\\\"r\", b: 2}]\n",
        "@csv",
        "\"\\\"x,y\\\",b\\n\\\"q\\\"\\\"r\\\",2\"\n",
    ),
    (
        "[{\"x,y\": \"q\\\"r\", b: 2}]\n",
        "@tsv",
        "\"x,y\\tb\\n\\\"q\\\"\\\"r\\\"\\t2\"\n",
    ),
    ("[]\n", "@csv", "\"\"\n"),
    ("[]\n", "@tsv", "\"\"\n"),
    ("[[]]\n", "@csv", "\"\"\n"),
    ("[[]]\n", "@tsv", "\"\"\n"),
    ("[{}]\n", "@csv", "\"\"\n"),
    ("[{}]\n", "@tsv", "\"\"\n"),
    ("[[], [1]]\n", "@csv", "\"\\n1\"\n"),
    ("[[], [1]]\n", "@tsv", "\"\\n1\"\n"),
    ("[[1], []]\n", "@csv", "\"1\"\n"),
    ("[[1], []]\n", "@tsv", "\"1\"\n"),
    ("[[\"\", \"a\"]]\n", "@csv", "\",a\"\n"),
    ("[[\"\", \"a\"]]\n", "@tsv", "\"\\ta\"\n"),
    ("[\"a\\tb\"]\n", "@csv", "\"a\\tb\"\n"),
    ("[\"a\\tb\"]\n", "@tsv", "\"\\\"a\\tb\\\"\"\n"),
    ("[\"a\\nb\"]\n", "@csv", "\"\\\"a\\nb\\\"\"\n"),
    ("[\"a\\nb\"]\n", "@tsv", "\"\\\"a\\nb\\\"\"\n"),
    ("[\"#x\", \"é\", \"ü,\"]\n", "@csv", "\"#x,é,\\\"ü,\\\"\"\n"),
    ("[\"#x\", \"é\", \"ü,\"]\n", "@tsv", "\"#x\\té\\tü,\"\n"),
    ("abc\n", "@csv", "\"abc\"\n"),
    ("abc\n", "@tsv", "\"abc\"\n"),
    ("5\n", "@csv", "\"5\"\n"),
    ("5\n", "@tsv", "\"5\"\n"),
    ("\"a,b\"\n", "@csv", "\"a,b\"\n"),
    ("\"a,b\"\n", "@tsv", "\"a,b\"\n"),
    (
        "[[a, \" b\"], [\"c\\nd\", \"e\"]]\n",
        "@csv",
        "\"a,\\\" b\\\"\\n\\\"c\\nd\\\",e\"\n",
    ),
    (
        "[[a, \" b\"], [\"c\\nd\", \"e\"]]\n",
        "@tsv",
        "\"a\\t\\\" b\\\"\\n\\\"c\\nd\\\"\\te\"\n",
    ),
    ("[\"\\\\.\"]\n", "@csv", "\"\\\"\\\\.\\\"\"\n"),
    ("[\"\\\\.\"]\n", "@tsv", "\"\\\"\\\\.\\\"\"\n"),
];

/// (document, filter, yq's error line).
const ERROR_ROWS: &[(&str, &str, &str)] = &[
    ("[[1,2], 3]\n", "@csv", "Error: csv encoding only works for arrays of scalars (string/numbers/booleans), child[1] is a !!int"),
    ("[[1,2], 3]\n", "@tsv", "Error: csv encoding only works for arrays of scalars (string/numbers/booleans), child[1] is a !!int"),
    ("[1, {k: v}]\n", "@csv", "Error: csv encoding only works for arrays of scalars (string/numbers/booleans), child[1] is a !!map"),
    ("[1, {k: v}]\n", "@tsv", "Error: csv encoding only works for arrays of scalars (string/numbers/booleans), child[1] is a !!map"),
    ("[[1,[2]]]\n", "@csv", "Error: csv encoding only works for arrays of scalars (string/numbers/booleans), child[1] is a !!seq"),
    ("[[1,[2]]]\n", "@tsv", "Error: csv encoding only works for arrays of scalars (string/numbers/booleans), child[1] is a !!seq"),
    ("[{k: v}, 1]\n", "@csv", "Error: csv object encoding only works for arrays of flat objects (string key => string/numbers/boolean value), child[1] is a !!int"),
    ("[{k: v}, 1]\n", "@tsv", "Error: csv object encoding only works for arrays of flat objects (string key => string/numbers/boolean value), child[1] is a !!int"),
    ("[{a: [1], b: 2}]\n", "@csv", "Error: csv encoding only works for arrays of scalars (string/numbers/booleans), child[0] is a !!seq"),
    ("[{a: [1], b: 2}]\n", "@tsv", "Error: csv encoding only works for arrays of scalars (string/numbers/booleans), child[0] is a !!seq"),
    ("[{b: 2, a: [1]}]\n", "@csv", "Error: csv encoding only works for arrays of scalars (string/numbers/booleans), child[1] is a !!seq"),
    ("[{b: 2, a: [1]}]\n", "@tsv", "Error: csv encoding only works for arrays of scalars (string/numbers/booleans), child[1] is a !!seq"),
    ("{a: 1}\n", "@csv", "Error: csv encoding only works for arrays, got: !!map"),
    ("{a: 1}\n", "@tsv", "Error: csv encoding only works for arrays, got: !!map"),
];

#[test]
fn csv_and_tsv_encode_as_go_encoding_csv_does_6131() -> Result<()> {
    for &(document, filter, expected) in ROWS {
        let (output, code) = run_yq_stdin(filter, document, &["-o=json", "-I=0"])?;
        assert_eq!(code, 0, "{filter} over {document:?}");
        assert_eq!(output, expected, "{filter} over {document:?}");
    }
    Ok(())
}

#[test]
fn csv_and_tsv_refuse_what_go_encoding_csv_does_6131() -> Result<()> {
    for &(document, filter, expected) in ERROR_ROWS {
        let (_, stderr, code) = run_yq_stdin_with_stderr(filter, document, &["-o=json", "-I=0"])?;
        assert_eq!(code, 1, "{filter} over {document:?}");
        assert!(
            stderr.contains(expected),
            "{filter} over {document:?}: {stderr}"
        );
    }
    Ok(())
}

/// The one difference left: a `null` is spelled `null`, where yq keeps the source spelling
/// (`~` here), which a value does not hold (`docs/compliance/yq/limitations.md`, #6131).
#[test]
fn a_tilde_null_is_spelled_null_6131() -> Result<()> {
    let (output, code) = run_yq_stdin("@csv", "[~, null, a]\n", &["-o=json", "-I=0"])?;
    assert_eq!((output.as_str(), code), ("\"null,null,a\"\n", 0));
    Ok(())
}

/// jq's encoders are unchanged.
#[test]
fn jq_mode_csv_still_quotes_every_string_6131() -> Result<()> {
    let (output, code) = crate::run_jq_stdin("@csv", "[1, \"a\"]", &[])?;
    assert_eq!((output.as_str(), code), ("\"1,\\\"a\\\"\"\n", 0));
    Ok(())
}
