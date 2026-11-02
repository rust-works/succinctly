//! #5293: in jq mode `//` binds *looser* than the assignment operators (`parser.y` declares
//! `%right "//"` before `%nonassoc '=' SETPIPE ...`), so `.b = .a // "d"` is `(.b = .a) // "d"`
//! and an assignment's two sides stop before a `//`. Every expected output is what jq 1.7.1
//! prints. The yq order, where `//` is tighter, is pinned in `tests/yq_cli/i5293.rs`.

use anyhow::Result;

use crate::run_jq_full;

/// (input name, input, filter, expected `-c` output, exit code).
const ROWS: &[(&str, &str, &str, &str, i32)] = &[
    (
        "null",
        "{\"a\":null,\"b\":2}",
        ".b = .a // \"d\"",
        "{\"a\":null,\"b\":null}\n",
        0,
    ),
    (
        "null",
        "{\"a\":null,\"b\":2}",
        ".a = null // 7",
        "{\"a\":null,\"b\":2}\n",
        0,
    ),
    (
        "null",
        "{\"a\":null,\"b\":2}",
        ".a |= empty // 2",
        "{\"b\":2}\n",
        0,
    ),
    ("empty", "{}", ".a |= . // 5", "{\"a\":null}\n", 0),
    (
        "null",
        "{\"a\":null,\"b\":2}",
        ".b += .a // 5",
        "{\"a\":null,\"b\":2}\n",
        0,
    ),
    (
        "null",
        "{\"a\":null,\"b\":2}",
        ".b -= 1 // 5",
        "{\"a\":null,\"b\":1}\n",
        0,
    ),
    ("one", "{\"a\":1}", ".a *= 2 // 9", "{\"a\":2}\n", 0),
    ("one", "{\"a\":1}", ".a /= 0 // 9", "", 5),
    ("one", "{\"a\":1}", ".a %= 2 // 9", "{\"a\":1}\n", 0),
    (
        "null",
        "{\"a\":null,\"b\":2}",
        ".a //= 3 // 4",
        "{\"a\":3,\"b\":2}\n",
        0,
    ),
    (
        "null",
        "{\"a\":null,\"b\":2}",
        ".a //= (3 // 4)",
        "{\"a\":3,\"b\":2}\n",
        0,
    ),
    (
        "null",
        "{\"a\":null,\"b\":2}",
        ".b = .a // .b // \"z\"",
        "{\"a\":null,\"b\":null}\n",
        0,
    ),
    (
        "null",
        "{\"a\":null,\"b\":2}",
        ".b = (.a // \"d\")",
        "{\"a\":null,\"b\":\"d\"}\n",
        0,
    ),
    (
        "null",
        "{\"a\":null,\"b\":2}",
        "(.b = .a) // \"d\"",
        "{\"a\":null,\"b\":null}\n",
        0,
    ),
    (
        "null",
        "{\"a\":null,\"b\":2}",
        ".a // .b = 9",
        "{\"a\":null,\"b\":9}\n",
        0,
    ),
    (
        "null",
        "{\"a\":null,\"b\":2}",
        ".a // .c = 9",
        "{\"a\":null,\"b\":2,\"c\":9}\n",
        0,
    ),
    ("one", "{\"a\":1}", ".a // .c = 9", "1\n", 0),
    (
        "null",
        "{\"a\":null,\"b\":2}",
        ".a = 1 // 2 | .b = 3 // 4",
        "{\"a\":1,\"b\":3}\n",
        0,
    ),
    (
        "null",
        "{\"a\":null,\"b\":2}",
        ".a = 1 // 2, .b = 3 // 4",
        "{\"a\":1,\"b\":2}\n{\"a\":null,\"b\":3}\n",
        0,
    ),
    (
        "null",
        "{\"a\":null,\"b\":2}",
        ".b = .a or false // 7",
        "{\"a\":null,\"b\":false}\n",
        0,
    ),
    (
        "null",
        "{\"a\":null,\"b\":2}",
        ".b = 1 + 1 // 7",
        "{\"a\":null,\"b\":2}\n",
        0,
    ),
    (
        "null",
        "{\"a\":null,\"b\":2}",
        ".x = (1,2) // 3",
        "{\"a\":null,\"b\":2,\"x\":1}\n{\"a\":null,\"b\":2,\"x\":2}\n",
        0,
    ),
    ("null", "{\"a\":null,\"b\":2}", ".x = empty // 3", "3\n", 0),
    (
        "null",
        "{\"a\":null,\"b\":2}",
        ".x = false // 3 // 4",
        "{\"a\":null,\"b\":2,\"x\":false}\n",
        0,
    ),
    (
        "null",
        "{\"a\":null,\"b\":2}",
        "[.b = null // 8]",
        "[{\"a\":null,\"b\":null}]\n",
        0,
    ),
    (
        "null",
        "{\"a\":null,\"b\":2}",
        "[(.b = null) // 8]",
        "[{\"a\":null,\"b\":null}]\n",
        0,
    ),
    (
        "null",
        "{\"a\":null,\"b\":2}",
        "[.b = (null // 8)]",
        "[{\"a\":null,\"b\":8}]\n",
        0,
    ),
    (
        "null",
        "{\"a\":null,\"b\":2}",
        "try (.b = error(\"e\") // 1) catch \"c\"",
        "\"c\"\n",
        0,
    ),
    (
        "null",
        "{\"a\":null,\"b\":2}",
        ".b = error(\"e\") // 1",
        "",
        5,
    ),
    (
        "null",
        "{\"a\":null,\"b\":2}",
        "null as $v | .b = $v // \"d\"",
        "{\"a\":null,\"b\":null}\n",
        0,
    ),
    (
        "null",
        "{\"a\":null,\"b\":2}",
        ".a as $v | .b = $v // \"d\"",
        "{\"a\":null,\"b\":null}\n",
        0,
    ),
    (
        "null",
        "{\"a\":null,\"b\":2}",
        "reduce (null, 1) as $q ({}; .k = $q // 5)",
        "{\"k\":1}\n",
        0,
    ),
    (
        "null",
        "{\"a\":null,\"b\":2}",
        "reduce range(3) as $i ({}; .[$i|tostring] = null // $i)",
        "{\"0\":null,\"1\":null,\"2\":null}\n",
        0,
    ),
    (
        "null",
        "{\"a\":null,\"b\":2}",
        "reduce range(3) as $i ({}; .[$i|tostring] = ($i // 9))",
        "{\"0\":0,\"1\":1,\"2\":2}\n",
        0,
    ),
    (
        "null",
        "{\"a\":null,\"b\":2}",
        "reduce range(4) as $i ([]; .[$i] = false // $i)",
        "[false,false,false,false]\n",
        0,
    ),
    (
        "null",
        "{\"a\":null,\"b\":2}",
        "[foreach (1,2) as $i ({}; .k = null // $i; .k)]",
        "[null,null]\n",
        0,
    ),
    (
        "null",
        "{\"a\":null,\"b\":2}",
        "[limit(2; .b = null // (1,2,3))]",
        "[{\"a\":null,\"b\":null}]\n",
        0,
    ),
    (
        "arr",
        "{\"a\":[1,2],\"b\":[3]}",
        ".a[0] = 5 // 6",
        "{\"a\":[5,2],\"b\":[3]}\n",
        0,
    ),
    (
        "arr",
        "{\"a\":[1,2],\"b\":[3]}",
        ".a[] += 1 // 9",
        "{\"a\":[2,3],\"b\":[3]}\n",
        0,
    ),
    (
        "arr",
        "{\"a\":[1,2],\"b\":[3]}",
        ".a |= map(. // 3) // 4",
        "{\"a\":[1,2],\"b\":[3]}\n",
        0,
    ),
    (
        "arr",
        "{\"a\":[1,2],\"b\":[3]}",
        ".b[5] = 1 // 2",
        "{\"a\":[1,2],\"b\":[3,null,null,null,null,1]}\n",
        0,
    ),
    ("one", "{\"a\":1}", "first(.a = 2 // 3)", "{\"a\":2}\n", 0),
    ("one", "{\"a\":1}", "path(.a // .b)", "[\"a\"]\n", 0),
    ("one", "{\"a\":1}", "del(.a // .b)", "{}\n", 0),
    ("one", "{\"a\":1}", "(.a // .b) = 7", "{\"a\":7}\n", 0),
    ("empty", "{}", "(.a // .b) = 7", "{\"b\":7}\n", 0),
];

#[test]
#[allow(clippy::literal_string_with_formatting_args)] // jq object patterns read like format arguments
fn test_alternative_binds_looser_than_assignment_in_jq_mode_5293() -> Result<()> {
    for (name, input, filter, want, want_code) in ROWS {
        let (stdout, stderr, code) =
            run_jq_full(&["-n", "-c", &format!("{input} | {filter}")], None)?;
        assert_eq!(
            (stdout.as_str(), code),
            (*want, *want_code),
            "{name}: `{filter}`: {stderr}"
        );
    }
    Ok(())
}

/// The same rows whose target does more than navigate (`.a // 1 %= 2` is `.a // (1 %= 2)`, whose
/// left side is a path expression that reads).
/// (input name, input, filter, expected `-c` output, exit code).
const DYNAMIC_ROWS: &[(&str, &str, &str, &str, i32)] = &[
    (
        "one",
        "{\"a\":1}",
        ".a = 1 as $x | .a // 1 %= 2",
        "{\"a\":1}\n",
        0,
    ),
    (
        "one",
        "{\"a\":1}",
        ".a = 1 as $x | (.a // 1) %= 2",
        "{\"a\":{\"a\":1}}\n",
        0,
    ),
];

#[test]
fn test_alternative_in_a_binding_body_after_assignment_5293() -> Result<()> {
    for (name, input, filter, want, want_code) in DYNAMIC_ROWS {
        let (stdout, stderr, code) =
            run_jq_full(&["-n", "-c", &format!("{input} | {filter}")], None)?;
        assert_eq!(
            (stdout.as_str(), code),
            (*want, *want_code),
            "{name}: `{filter}`: {stderr}"
        );
    }
    Ok(())
}
