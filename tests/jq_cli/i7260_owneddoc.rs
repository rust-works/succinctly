//! #7260: `walk(f)` over a materialized document rewrites the owned tree in place (each object keeps
//! its map, each array its storage) instead of writing the tree out as JSON, indexing it again and
//! building a second copy; an assignment the owned route answers writes into the materialized input
//! itself instead of a copy-on-write clone of it. Neither may change an answer: every row was
//! captured from jq 1.7.1. The rows cover what the in-place rewrite has to get right: a repeated
//! key, a member deleted by an empty output, a member keeping only its first output, an array
//! element with several outputs spilling into a fresh array, an error aborting the walk, and a
//! bound copy of the input that must not see the write.

use anyhow::Result;

use crate::run_jq_full;

/// `(filter, input, stdout)`, each run with `-c`.
const ROWS: &[(&str, &str, &str)] = &[
    (
        r#"walk(if type=="string" then empty else . end)"#,
        r#"{"a":"x","b":[1,"y",{"c":"z","d":2}],"a":3}"#,
        "{\"a\":3,\"b\":[1,{\"d\":2}]}\n",
    ),
    (
        r#"walk(if type=="number" then (.,.+10) else . end)"#,
        r#"{"a":1,"b":[1,2,{"c":3}]}"#,
        "{\"a\":1,\"b\":[1,11,2,12,{\"c\":3}]}\n",
    ),
    (
        r#"walk(if type=="number" and . == 2 then (1,2,3) else . end)"#,
        r#"[1,2,3,[2],{"k":2}]"#,
        "[1,1,2,3,3,[1,2,3],{\"k\":1}]\n",
    ),
    (
        r#"walk(if type=="number" then empty else . end)"#,
        r#"[1,[2,"s",3],{"a":1,"b":"t"}]"#,
        "[[\"s\"],{\"b\":\"t\"}]\n",
    ),
    (
        r#"try walk(if . == 2 then error("boom") else . end) catch ."#,
        r#"{"a":[1,2],"b":3}"#,
        "\"boom\"\n",
    ),
    (
        r#"walk(if type=="number" then (., error("x")) else . end)"#,
        r#"{"a":1,"b":{"c":2}}"#,
        "{\"a\":1,\"b\":{\"c\":2}}\n",
    ),
    (
        "[walk(.), .] | .[0] == .[1]",
        r#"{"a":{"a":1},"b":[[],{}]}"#,
        "true\n",
    ),
    (
        r#". as $x | walk(if type=="number" then .+1 else . end) | [., $x]"#,
        r#"{"a":[1,{"b":2}]}"#,
        "[{\"a\":[2,{\"b\":3}]},{\"a\":[1,{\"b\":2}]}]\n",
    ),
    (
        "walk(.)|.users|length",
        r#"{"users":[{"id":0,"name":"A"},{"id":1,"name":"B"}]}"#,
        "2\n",
    ),
    (
        r#".users[0].name = "x" | .users"#,
        r#"{"users":[{"id":0,"name":"A"},{"id":1,"name":"B"}]}"#,
        "[{\"id\":0,\"name\":\"x\"},{\"id\":1,\"name\":\"B\"}]\n",
    ),
    (
        ".users[1].age += 1 | .users",
        r#"{"users":[{"id":0},{"id":1,"age":5}]}"#,
        "[{\"id\":0},{\"id\":1,\"age\":6}]\n",
    ),
    (
        ".users[0].name |= ascii_downcase | .users[0]",
        r#"{"users":[{"name":"ABC"}]}"#,
        "{\"name\":\"abc\"}\n",
    ),
    (
        "[., (.a.b = 1)]",
        r#"{"a":{"b":0,"c":2}}"#,
        "[{\"a\":{\"b\":0,\"c\":2}},{\"a\":{\"b\":1,\"c\":2}}]\n",
    ),
    (
        "(.a.b = 1) as $y | [$y, .]",
        r#"{"a":{"b":0}}"#,
        "[{\"a\":{\"b\":1}},{\"a\":{\"b\":0}}]\n",
    ),
    (".b[3] = 1", r#"{"b":[0]}"#, "{\"b\":[0,null,null,1]}\n"),
];

#[test]
fn owned_walk_and_assign_match_jq_7260() -> Result<()> {
    for (filter, input, expected) in ROWS {
        let (stdout, stderr, code) = run_jq_full(&["-c", filter], Some(input))?;
        assert_eq!(
            (stdout.as_str(), stderr.as_str(), code),
            (*expected, "", 0),
            "{filter} on {input}"
        );
    }
    Ok(())
}

/// A write the owned route cannot make still raises jq's error, with its text and exit code.
#[test]
fn owned_assign_error_matches_jq_7260() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(&["-c", ".a.b = 1"], Some(r#"{"a":5}"#))?;
    assert_eq!(stdout, "");
    assert_eq!(
        stderr,
        "jq: error (at <stdin>:0): Cannot index number with string \"b\"\n"
    );
    assert_eq!(code, 5);
    Ok(())
}
