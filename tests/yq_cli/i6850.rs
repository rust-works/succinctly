//! #6850: `succinctly yq -o props|shell|csv|tsv|base64|uri`, the first slice of yq's output formats
//! (each one is the results of the filter through that format's encoder, printed raw). Every answer
//! was captured from yq v4.53.3; a row is kept only when both agree on stdout and the exit status.
//! Left out because they still differ: a comment of a value the filter builds (#6922 writes the ones
//! read from the document), a null written `~` (yq echoes the spelling, #2802), an encoder failing after the earlier results of a
//! top-level comma were produced (yq printed them; #6923 keeps them for every other shape) and an
//! accented `shell` key (yq decomposes it first). See `docs/compliance/yq/limitations.md`.

use anyhow::Result;

use crate::run_yq_stdin;

/// (format, input, filter, yq's stdout, yq's exit status).
const ROWS: &[(&str, &str, &str, &str, i32)] = &[
    (
        "props",
        "a: 1\nb: [x, y]\nc: {d: e}\n",
        ".",
        "a = 1\nb.0 = x\nb.1 = y\nc.d = e\n",
        0,
    ),
    ("props", "hello\n", ".", "hello\n", 0),
    (
        "props",
        "a: \"x y\"\nb: \"multi\\nline\"\nc: \"\"\n",
        ".",
        "a = x y\nb = multi\\nline\nc = \n",
        0,
    ),
    ("props", "---\na: 1\n---\nb: 2\n", ".", "a = 1\nb = 2\n", 0),
    ("props", "[a, b]\n", ".[]", "a\nb\n", 0),
    ("props", "{}\n", ".", "", 0),
    (
        "shell",
        "a: 1\nb: [x, y]\nc: {d: e}\n",
        ".",
        "a=1\nb_0=x\nb_1=y\nc_d=e\n",
        0,
    ),
    ("shell", "hello world\n", ".", "value='hello world'\n", 0),
    (
        "shell",
        "a-b: 1\nc.d: 2\ne f: 3\n1x: 4\n\"\": 5\n",
        ".",
        "a_b=1\nc_d=2\ne_f=3\n_1x=4\n_=5\n",
        0,
    ),
    (
        "shell",
        "a: \"it's\"\nb: \"\"\nc: \"a/b.c\"\nd: \"$HOME\"\n",
        ".",
        "a='it'\"'\"'s'\nb=\nc='a/b.c'\nd='$HOME'\n",
        0,
    ),
    ("shell", "[a, b]\n", ".", "_0=a\n_1=b\n", 0),
    (
        "csv",
        "[a, \"b,c\", \"q\\\"r\", 1, true]\n",
        ".",
        "a,\"b,c\",\"q\"\"r\",1,true\n",
        0,
    ),
    ("csv", "[[a, b], [1, 2]]\n", ".", "a,b\n1,2\n", 0),
    ("csv", "[{a: 1, b: 2}, {a: 3}]\n", ".", "a,b\n1,2\n3,\n", 0),
    ("csv", "[]\n", ".", "", 0),
    ("csv", "hello\n", ".", "hello\n", 0),
    (
        "shell",
        "x: {\"日本\": 1, \"a日b\": 2}\n",
        ".",
        "x_=1\nx_ab=2\n",
        0,
    ),
    ("csv", "[[], []]\n", ".", "\n\n", 0),
    ("csv", "---\n[a, b]\n---\n[c]\n", ".", "a,b\nc\n", 0),
    ("tsv", "[a, \"b\\tc\", 1]\n", ".", "a\t\"b\tc\"\t1\n", 0),
    ("tsv", "[[a, b], [1, 2]]\n", ".", "a\tb\n1\t2\n", 0),
    ("base64", "hello\n", ".", "aGVsbG8=", 0),
    ("base64", "---\na\n---\nb\n", ".", "YQ==Yg==", 0),
    ("base64", "\"x y\"\n", ".", "eCB5", 0),
    ("uri", "\"a b/c?d=é\"\n", ".", "a+b%2Fc%3Fd%3D%C3%A9", 0),
    ("uri", "---\na b\n---\nc d\n", ".", "a+bc+d", 0),
    ("base64", "a: 1\n", ".", "", 1),
    ("base64", "3\n", ".", "", 1),
    ("uri", "[a]\n", ".", "", 1),
    ("csv", "a: 1\n", ".", "", 1),
    ("tsv", "a: 1\n", ".", "", 1),
    ("csv", "[a, [b]]\n", ".", "", 1),
];

#[test]
fn output_format_rows_match_yq_6850() -> Result<()> {
    for (format, input, filter, expected, code) in ROWS {
        let flag = format!("-o={format}");
        let (stdout, status) = run_yq_stdin(filter, input, &[flag.as_str()])?;
        assert_eq!(
            (stdout.as_str(), status),
            (*expected, *code),
            "-o={format} {filter:?} over {input:?}"
        );
    }
    Ok(())
}

#[test]
fn output_format_short_names_6850() -> Result<()> {
    let input = "a: 1\nb: [x]\n";
    for (short, long) in [("p", "props"), ("s", "shell"), ("c", "csv"), ("t", "tsv")] {
        let one = run_yq_stdin(".b", input, &[format!("-o={short}").as_str()])?;
        let other = run_yq_stdin(".b", input, &[format!("-o={long}").as_str()])?;
        assert_eq!(one, other, "-o={short} is -o={long}");
    }
    Ok(())
}

#[test]
fn shell_key_separator_6850() -> Result<()> {
    let (stdout, status) = run_yq_stdin(
        ".",
        "a: {b: [1]}\n",
        &["-o=shell", "--shell-key-separator=:"],
    )?;
    assert_eq!((stdout.as_str(), status), ("a:b:0=1\n", 0));
    Ok(())
}

#[test]
fn nul_output_trims_the_encoders_last_line_end_6850() -> Result<()> {
    let (stdout, status) = run_yq_stdin(".", "a: 1\nb: 2\n", &["-o=props", "-0"])?;
    assert_eq!((stdout.as_str(), status), ("a = 1\nb = 2\0", 0));
    Ok(())
}
