//! #5306: `group_by(f)` on an array in jq mode keeps its elements as document cursors, sorted by
//! key and cut into groups, and decodes a group only when something reads it. It went through
//! the `eval.rs` bridge before, which decoded every element twice over. The expected output of
//! every row is what jq 1.7.1 prints.

use anyhow::Result;

use crate::run_jq_full;

const INPUT: &str = r#"[{"a":1,"b":"x"},{"a":2,"b":"y"},{"a":1,"b":"z"},{"a":null},{"b":3},{"a":1.0},{"a":"1"},{"a":[1]},{"a":true},{"a":false},{"a":{"x":1}},{"a":2}]"#;

/// (filter, expected `-c` output, exit code), run on [`INPUT`].
const ROWS: &[(&str, &str, i32)] = &[
    ("group_by(.a)", "[[{\"a\":null},{\"b\":3}],[{\"a\":false}],[{\"a\":true}],[{\"a\":1,\"b\":\"x\"},{\"a\":1,\"b\":\"z\"},{\"a\":1.0}],[{\"a\":2,\"b\":\"y\"},{\"a\":2}],[{\"a\":\"1\"}],[{\"a\":[1]}],[{\"a\":{\"x\":1}}]]\n", 0),
    ("group_by(.a, .b)", "[[{\"a\":null}],[{\"b\":3}],[{\"a\":false}],[{\"a\":true}],[{\"a\":1.0}],[{\"a\":1,\"b\":\"x\"}],[{\"a\":1,\"b\":\"z\"}],[{\"a\":2}],[{\"a\":2,\"b\":\"y\"}],[{\"a\":\"1\"}],[{\"a\":[1]}],[{\"a\":{\"x\":1}}]]\n", 0),
    ("group_by(.)", "[[{\"a\":null}],[{\"a\":false}],[{\"a\":true}],[{\"a\":1.0}],[{\"a\":2}],[{\"a\":\"1\"}],[{\"a\":[1]}],[{\"a\":{\"x\":1}}],[{\"a\":1,\"b\":\"x\"}],[{\"a\":1,\"b\":\"z\"}],[{\"a\":2,\"b\":\"y\"}],[{\"b\":3}]]\n", 0),
    ("group_by(empty)", "[[{\"a\":1,\"b\":\"x\"},{\"a\":2,\"b\":\"y\"},{\"a\":1,\"b\":\"z\"},{\"a\":null},{\"b\":3},{\"a\":1.0},{\"a\":\"1\"},{\"a\":[1]},{\"a\":true},{\"a\":false},{\"a\":{\"x\":1}},{\"a\":2}]]\n", 0),
    ("group_by(.a?)", "[[{\"a\":null},{\"b\":3}],[{\"a\":false}],[{\"a\":true}],[{\"a\":1,\"b\":\"x\"},{\"a\":1,\"b\":\"z\"},{\"a\":1.0}],[{\"a\":2,\"b\":\"y\"},{\"a\":2}],[{\"a\":\"1\"}],[{\"a\":[1]}],[{\"a\":{\"x\":1}}]]\n", 0),
    ("group_by(.a) | length", "8\n", 0),
    ("group_by(.a) | map(length)", "[2,1,1,3,2,1,1,1]\n", 0),
    ("group_by(.a) | .[0]", "[{\"a\":null},{\"b\":3}]\n", 0),
    ("group_by(.a) | first", "[{\"a\":null},{\"b\":3}]\n", 0),
    ("group_by(.a) | last", "[{\"a\":{\"x\":1}}]\n", 0),
    ("group_by(.a) | .[-1]", "[{\"a\":{\"x\":1}}]\n", 0),
    ("group_by(.a) | .[2:4]", "[[{\"a\":true}],[{\"a\":1,\"b\":\"x\"},{\"a\":1,\"b\":\"z\"},{\"a\":1.0}]]\n", 0),
    ("[group_by(.a)[] | length]", "[2,1,1,3,2,1,1,1]\n", 0),
    ("group_by(.a) | map(.[0].a)", "[null,false,true,1,2,\"1\",[1],{\"x\":1}]\n", 0),
    ("group_by(.a) | map(select(length > 1))", "[[{\"a\":null},{\"b\":3}],[{\"a\":1,\"b\":\"x\"},{\"a\":1,\"b\":\"z\"},{\"a\":1.0}],[{\"a\":2,\"b\":\"y\"},{\"a\":2}]]\n", 0),
    ("group_by(.a) | map(error(\"x\"))", "", 5),
    ("try group_by(error(\"boom\")) catch .", "\"boom\"\n", 0),
    ("group_by(.a) | map(group_by(.b)) | length", "8\n", 0),
    ("group_by(.a) as $g | [$g, $g] | map(length)", "[8,8]\n", 0),
    ("[limit(2; group_by(.a)[])] | map(length)", "[2,1]\n", 0),
    ("first(group_by(.a)[])", "[{\"a\":null},{\"b\":3}]\n", 0),
    ("group_by(.a) | .[100]", "null\n", 0),
    ("group_by(.a) | map(length) | add", "12\n", 0),
    ("group_by(.a) | add | length", "12\n", 0),
    ("group_by(.a) | flatten | length", "12\n", 0),
    ("group_by(.a) | tojson | length", "159\n", 0),
    ("[.[] | select(.a == 1)] | group_by(.b)", "[[{\"a\":1.0}],[{\"a\":1,\"b\":\"x\"}],[{\"a\":1,\"b\":\"z\"}]]\n", 0),
    ("map(.a) | group_by(.)", "[[null,null],[false],[true],[1,1,1.0],[2,2],[\"1\"],[[1]],[{\"x\":1}]]\n", 0),
    ("[] | group_by(.a)", "[]\n", 0),
];

/// (input, filter, expected `-c` output, exit code).
const OTHER_INPUTS: &[(&str, &str, &str, i32)] = &[
    ("5", "group_by(.a)", "", 5),
    ("null", "group_by(.a)", "", 5),
    ("{\"a\":3,\"b\":1}", "group_by(.)", "", 5),
    ("\"s\"", "group_by(.)", "", 5),
    (
        "[[3,1],[1,2],[3,0],[1,2]]",
        "group_by(.[0])",
        "[[[1,2],[1,2]],[[3,1],[3,0]]]\n",
        0,
    ),
];

#[test]
fn test_group_by_array_answers_as_jq_5306() -> Result<()> {
    for (filter, want, want_code) in ROWS {
        let (stdout, stderr, code) = run_jq_full(&["-c", filter], Some(INPUT))?;
        assert_eq!(
            (stdout.as_str(), code),
            (*want, *want_code),
            "`{filter}`: {stderr}"
        );
    }
    for (input, filter, want, want_code) in OTHER_INPUTS {
        let (stdout, stderr, code) = run_jq_full(&["-c", filter], Some(input))?;
        assert_eq!(
            (stdout.as_str(), code),
            (*want, *want_code),
            "{input} | `{filter}`: {stderr}"
        );
    }
    Ok(())
}

/// An element that cannot be decoded raises its decode error before any key is read, so a key
/// that would raise (or that `try` would catch) never gets the chance.
#[test]
fn test_group_by_undecodable_element_wins_over_a_key_error_5306() -> Result<()> {
    let input = r#"[{"a":2,"s":"\ud800"},{"a":1,"s":"ok"}]"#;
    for filter in [
        "group_by(.a)",
        "try group_by(error(\"boom\")) catch .",
        "group_by(.[0])",
    ] {
        let (stdout, stderr, code) = run_jq_full(&["-c", filter], Some(input))?;
        assert_eq!((stdout.as_str(), code), ("", 5), "`{filter}`: {stderr}");
        assert!(
            stderr.contains("invalid unicode escape sequence"),
            "`{filter}`: {stderr}"
        );
    }
    Ok(())
}
