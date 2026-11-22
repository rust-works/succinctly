//! #7448 follow-up: what a field lookup reads of a document that has a member with no value
//! outside the object it looks in, recorded in `docs/compliance/jq/limitations.md` ("A field
//! lookup reads the members of its own object, not the values it steps over").
//!
//! The lookup steps over each member's value by the index's bracket structure, so a member with
//! no value one level down is not seen, nor is an array's element when the lookup fails on the
//! array's type, nor a `,`/`:` gap of a member the lookup steps over. jq 1.7.1 and 1.8.2 refuse
//! every document below at parse time, exit 5; the comment on each row gives jq's line. The rows
//! that read the faulty text are jq's answer, captured from `/usr/bin/jq` (1.8.2 agrees).

use anyhow::Result;

use crate::{run_jq_full, spawn_jq};

/// (document, `-c` filter, stdout, stderr, exit status): succinctly's answer, pinned.
const ROWS: &[(&str, &str, &str, &str, i32)] = &[
    // jq: `jq: parse error: Unmatched '}' at line 1, column 11`, exit 5, for every filter.
    (r#"{"x":{"a":},"y":1}"#, ".a", "null\n", "", 0),
    (r#"{"x":{"a":},"y":1}"#, ".y", "1\n", "", 0),
    (r#"{"x":{"a":},"y":1}"#, "keys", "[\"x\",\"y\"]\n", "", 0),
    (r#"{"a":1,"x":{"a":}}"#, ".a", "1\n", "", 0),
    // Read the inner object: jq's answer.
    (
        r#"{"x":{"a":},"y":1}"#,
        ".x.a",
        "",
        "jq: parse error: Unmatched '}' at line 1, column 11\n",
        5,
    ),
    (
        r#"{"x":{"a":},"y":1}"#,
        ".",
        "",
        "jq: parse error: Unmatched '}' at line 1, column 11\n",
        5,
    ),
    // jq: `jq: parse error: Unmatched '}' at line 1, column 7`, exit 5.
    (
        r#"[{"a":}]"#,
        ".a",
        "",
        "jq: error (at <stdin>:0): Cannot index array with string \"a\"\n",
        5,
    ),
    (
        r#"[{"a":}]"#,
        ".[0]",
        "",
        "jq: parse error: Unmatched '}' at line 1, column 7\n",
        5,
    ),
    // jq: `jq: parse error: Expected value before ',' at line 1, column 4`, exit 5.
    (
        "[1,,3]",
        ".a",
        "",
        "jq: error (at <stdin>:0): Cannot index array with string \"a\"\n",
        5,
    ),
    // jq: `jq: parse error: Expected separator between values at line 1, column 10`, exit 5.
    (r#"{"a":1 "b":2}"#, ".a", "1\n", "", 0),
    // jq: `jq: parse error: Objects must consist of key:value pairs at line 1, column 5`, exit 5.
    (r#"{"a",1}"#, ".b", "null\n", "", 0),
    // jq: `jq: parse error: Expected separator between values at line 1, column 7`, exit 5.
    (r#"{"a" 1}"#, ".b", "null\n", "", 0),
    // The lookup's own level: jq's answer since #7448.
    (
        r#"{"a":1,"b":}"#,
        ".a",
        "",
        "jq: parse error: Unmatched '}' at line 1, column 12\n",
        5,
    ),
    (
        r#"{"b":,"a":1}"#,
        ".b",
        "",
        "jq: parse error: Expected value before ',' at line 1, column 6\n",
        5,
    ),
];

#[test]
fn test_a_lookup_reads_its_own_objects_members_not_the_values_it_steps_over_7448() -> Result<()> {
    let mut failures = Vec::new();
    for &(doc, filter, stdout, stderr, status) in ROWS {
        let want = (stdout.to_string(), stderr.to_string(), status);
        let got = run_jq_full(&["-c", filter], Some(doc))?;
        if got != want {
            failures.push(format!("stdin {doc:?} | {filter}: {got:?}, want {want:?}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    Ok(())
}

/// The same rows from a named file, and the routes that read the document whole there: `-n input`
/// and `--stream` raise as jq 1.7.1 does.
#[test]
fn test_a_nested_member_without_a_value_from_a_file_7448() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("in.json");
    std::fs::write(&path, r#"{"x":{"a":},"y":1}"#)?;
    let name = path.to_string_lossy().into_owned();
    let rows: &[(&[&str], &str, &str, i32)] = &[
        (&["-c", ".a"], "null\n", "", 0),
        (&["-c", ".y"], "1\n", "", 0),
        (
            &["-c", "."],
            "",
            "jq: parse error: Unmatched '}' at line 1, column 11\n",
            5,
        ),
        (
            &["-c", "-n", "input | .a"],
            "",
            "jq: error (at <f>:0): Unmatched '}' at line 1, column 11\n",
            5,
        ),
        (
            &["-c", "--stream", "."],
            "",
            "jq: parse error: Missing value in key:value pair at line 1, column 11\n",
            5,
        ),
    ];
    let mut failures = Vec::new();
    for &(args, stdout, stderr, status) in rows {
        let mut argv = args.to_vec();
        argv.push(&name);
        let (out, code) = spawn_jq(&argv, None)?;
        let got = (
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).replace(&name, "<f>"),
            code,
        );
        let want = (stdout.to_string(), stderr.to_string(), status);
        if got != want {
            failures.push(format!("{args:?}: {got:?}, want {want:?}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    Ok(())
}
