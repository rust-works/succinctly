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

/// Output flags on the same array: the groups print from the document, so the writer's own
/// indent, tab and key-sort handling has to match jq's.
const FLAG_INPUT: &str =
    r#"[{"b":1,"a":"x"},{"b":2,"a":"y"},{"b":1,"a":"z","c":[1,{"d":null}]},{"a":0}]"#;

/// (flags, filter, expected output, exit code), run on [`FLAG_INPUT`].
const FLAG_ROWS: &[(&[&str], &str, &str, i32)] = &[
    (&["--tab"], "group_by(.b)", "[\n\t[\n\t\t{\n\t\t\t\"a\": 0\n\t\t}\n\t],\n\t[\n\t\t{\n\t\t\t\"b\": 1,\n\t\t\t\"a\": \"x\"\n\t\t},\n\t\t{\n\t\t\t\"b\": 1,\n\t\t\t\"a\": \"z\",\n\t\t\t\"c\": [\n\t\t\t\t1,\n\t\t\t\t{\n\t\t\t\t\t\"d\": null\n\t\t\t\t}\n\t\t\t]\n\t\t}\n\t],\n\t[\n\t\t{\n\t\t\t\"b\": 2,\n\t\t\t\"a\": \"y\"\n\t\t}\n\t]\n]\n", 0),
    (&["--indent", "1"], "group_by(.b)", "[\n [\n  {\n   \"a\": 0\n  }\n ],\n [\n  {\n   \"b\": 1,\n   \"a\": \"x\"\n  },\n  {\n   \"b\": 1,\n   \"a\": \"z\",\n   \"c\": [\n    1,\n    {\n     \"d\": null\n    }\n   ]\n  }\n ],\n [\n  {\n   \"b\": 2,\n   \"a\": \"y\"\n  }\n ]\n]\n", 0),
    (&[], "group_by(.b)", "[\n  [\n    {\n      \"a\": 0\n    }\n  ],\n  [\n    {\n      \"b\": 1,\n      \"a\": \"x\"\n    },\n    {\n      \"b\": 1,\n      \"a\": \"z\",\n      \"c\": [\n        1,\n        {\n          \"d\": null\n        }\n      ]\n    }\n  ],\n  [\n    {\n      \"b\": 2,\n      \"a\": \"y\"\n    }\n  ]\n]\n", 0),
    (&["-S", "-c"], "group_by(.b)", "[[{\"a\":0}],[{\"a\":\"x\",\"b\":1},{\"a\":\"z\",\"b\":1,\"c\":[1,{\"d\":null}]}],[{\"a\":\"y\",\"b\":2}]]\n", 0),
    (&["-c"], "group_by(.b) | map(length)", "[1,2,1]\n", 0),
    (&["--indent", "3"], "[] | group_by(.a)", "[]\n", 0),
    (&["--tab"], "group_by(.b) | .[1:]", "[\n\t[\n\t\t{\n\t\t\t\"b\": 1,\n\t\t\t\"a\": \"x\"\n\t\t},\n\t\t{\n\t\t\t\"b\": 1,\n\t\t\t\"a\": \"z\",\n\t\t\t\"c\": [\n\t\t\t\t1,\n\t\t\t\t{\n\t\t\t\t\t\"d\": null\n\t\t\t\t}\n\t\t\t]\n\t\t}\n\t],\n\t[\n\t\t{\n\t\t\t\"b\": 2,\n\t\t\t\"a\": \"y\"\n\t\t}\n\t]\n]\n", 0),
];

#[test]
fn test_group_by_output_flags_answer_as_jq_5306() -> Result<()> {
    for (flags, filter, want, want_code) in FLAG_ROWS {
        let mut args: Vec<&str> = flags.to_vec();
        args.push(filter);
        let (stdout, stderr, code) = run_jq_full(&args, Some(FLAG_INPUT))?;
        assert_eq!(
            (stdout.as_str(), code),
            (*want, *want_code),
            "{flags:?} `{filter}`: {stderr}"
        );
    }
    Ok(())
}

/// The ways a consumer reads `group_by`'s groups without a decode, on four groups of which the
/// second makes `ascii_downcase` fail: `map` runs to the end whatever follows it (`first`, `limit`,
/// `.[0]` cannot skip the failure), a `break` or `halt` in its body stops where jq stops it, and
/// the point reads answer as jq does.
const READ_INPUT: &str = r#"[{"a":1,"b":"x"},{"a":2,"b":5},{"a":3,"b":"z"},{"a":1,"b":"w"}]"#;

/// (filter, expected `-c` output, exit code), run on [`READ_INPUT`].
const READ_ROWS: &[(&str, &str, i32)] = &[
    ("group_by(.a) | map(.[0].b | ascii_downcase) | first", "", 5),
    ("group_by(.a) | map(.[0].b | ascii_downcase) | .[0]", "", 5),
    ("first(group_by(.a) | map(.[0].b | ascii_downcase) | .[])", "", 5),
    ("[limit(1; group_by(.a) | map(.[0].b | ascii_downcase) | .[])]", "", 5),
    ("try (group_by(.a) | map(.[0].b | ascii_downcase) | first) catch \"c\"", "\"c\"\n", 0),
    ("label $out | group_by(.a) | map(break $out)", "", 0),
    ("try (label $out | group_by(.a) | map(break $out)) catch \"caught\"", "", 0),
    ("[label $out | group_by(.a) | map(if .[0].a == 2 then break $out else . end)]", "[]\n", 0),
    ("(group_by(.a) | map(error(\"x\"))), 3", "", 5),
    ("(group_by(.a) | map(halt)), 3", "", 0),
    ("1, (group_by(.a) | map(halt)), 3", "1\n", 0),
    ("group_by(.a) | .[1] | length", "1\n", 0),
    ("group_by(.a) | .[-1] | length", "1\n", 0),
    ("group_by(.a) | last | length", "1\n", 0),
    ("group_by(.a) | .[1:3] | length", "2\n", 0),
    ("group_by(.a) | .[0][0].b", "\"x\"\n", 0),
    ("group_by(.a) | .[-1][0]", "{\"a\":3,\"b\":\"z\"}\n", 0),
    ("group_by(.a) | .[1:] | map(length)", "[1,1]\n", 0),
    ("group_by(.a) | .[:1]", "[[{\"a\":1,\"b\":\"x\"},{\"a\":1,\"b\":\"w\"}]]\n", 0),
    ("group_by(.a) | .[9]", "null\n", 0),
    ("group_by(.a) | length", "3\n", 0),
    ("group_by(.a)[] | length", "2\n1\n1\n", 0),
    ("[group_by(.a)[] | .[0].a]", "[1,2,3]\n", 0),
    ("first(group_by(.a)[])", "[{\"a\":1,\"b\":\"x\"},{\"a\":1,\"b\":\"w\"}]\n", 0),
    ("group_by(.a)[1]", "[{\"a\":2,\"b\":5}]\n", 0),
    ("group_by(.a) | map(.[0].a)", "[1,2,3]\n", 0),
    ("group_by(.a) | map(.[1:])", "[[{\"a\":1,\"b\":\"w\"}],[],[]]\n", 0),
    ("group_by(.a) | map(empty)", "[]\n", 0),
    ("group_by(.a) | map(first, last)", "[{\"a\":1,\"b\":\"x\"},{\"a\":1,\"b\":\"w\"},{\"a\":2,\"b\":5},{\"a\":2,\"b\":5},{\"a\":3,\"b\":\"z\"},{\"a\":3,\"b\":\"z\"}]\n", 0),
    ("group_by(.a) | map(map(.a))", "[[1,1],[2],[3]]\n", 0),
    ("group_by(.a) | map({n: length, f: .[0].b})", "[{\"n\":2,\"f\":\"x\"},{\"n\":1,\"f\":5},{\"n\":1,\"f\":\"z\"}]\n", 0),
    ("reduce group_by(.a)[] as $g (0; . + ($g | length))", "4\n", 0),
    ("try (group_by(.a) | length) catch .", "3\n", 0),
    ("group_by(.a) as $g | $g[0] | length", "2\n", 0),
];

#[test]
fn test_group_by_groups_read_as_jq_reads_them_5306() -> Result<()> {
    for (filter, want, want_code) in READ_ROWS {
        let (stdout, stderr, code) = run_jq_full(&["-c", filter], Some(READ_INPUT))?;
        assert_eq!(
            (stdout.as_str(), code),
            (*want, *want_code),
            "`{filter}`: {stderr}"
        );
    }
    Ok(())
}

/// A decode failure outranks a key error whichever element holds it, because every element is
/// checked before any key is read; an interleaved check-then-key loop reports `boom` here.
#[test]
fn test_group_by_checks_every_element_before_reading_a_key_5306() -> Result<()> {
    let input = r#"[{"a":1},{"a":2,"s":"\ud800"}]"#;
    let (stdout, stderr, code) = run_jq_full(
        &["-c", r#"try group_by(error("boom")) catch ."#],
        Some(input),
    )?;
    assert_eq!((stdout.as_str(), code), ("", 5), "{stderr}");
    assert!(
        stderr.contains("invalid unicode escape sequence"),
        "{stderr}"
    );
    Ok(())
}

/// The elements sit one level inside the array: 256 levels of nesting in an element is one level
/// too many, as it is for every other builtin that reads the array whole.
#[test]
fn test_group_by_counts_nesting_from_the_array_5306() -> Result<()> {
    let deep = |levels: usize| format!("[{}{}]", "[".repeat(levels), "]".repeat(levels));
    let (stdout, stderr, code) = run_jq_full(&["-c", "group_by(.a?) | length"], Some(&deep(256)))?;
    assert_eq!((stdout.as_str(), code), ("", 5), "{stderr}");
    assert!(
        stderr.contains("nesting depth exceeds limit of 256"),
        "{stderr}"
    );
    let (stdout, stderr, code) = run_jq_full(&["-c", "group_by(.a?) | length"], Some(&deep(255)))?;
    assert_eq!((stdout.as_str(), code), ("1\n", 0), "{stderr}");
    Ok(())
}
