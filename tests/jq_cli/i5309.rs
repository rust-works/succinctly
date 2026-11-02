//! #5309: the regex builtins read their input before their pattern and flags, `nth` words a
//! non-number index the way jq's `if $n < 0 ... limit($n + 1; ...)` does, and `delpaths` names the
//! first non-array in value order. The messages themselves are pinned over both evaluators by the
//! probes in `tests/data/jq-error-probes.tsv`; these pin what a probe cannot hold: the catchable and
//! optional forms, multi-output arguments, arguments that produce nothing, and path mode. Every
//! expected value was captured live against jq 1.7.1 (`/usr/bin/jq`).

use anyhow::Result;

use crate::run_jq_full;

fn run(filter: &str) -> Result<(String, String, i32)> {
    run_jq_full(&["-nc", filter], None)
}

const NOT_A_STRING: &str = "null (null) cannot be matched, as it is not a string";

#[test]
fn test_the_input_error_is_catchable_and_optional_5309() -> Result<()> {
    for (filter, want) in [
        (
            "null | try match(1;2) catch .",
            format!("\"{NOT_A_STRING}\"\n"),
        ),
        // A multi-output pattern or flags still reports the input once.
        (
            "null | try match((1,2);3) catch .",
            format!("\"{NOT_A_STRING}\"\n"),
        ),
        (
            "[null | try match(\"a\";(1,2)) catch .]",
            format!("[\"{NOT_A_STRING}\"]\n"),
        ),
        (
            "[null | try match((1,\"a\");(2,3)) catch .]",
            format!("[\"{NOT_A_STRING}\"]\n"),
        ),
        (
            "try (null | gsub((1,2);\"x\";\"g\")) catch .",
            format!("\"{NOT_A_STRING}\"\n"),
        ),
        ("null | [match(1;2)?]", "[]\n".to_string()),
        (
            "null | [try test(1;2) catch \"x\"]",
            "[\"x\"]\n".to_string(),
        ),
        // An argument that produces nothing never reaches the call that reads the input.
        ("null | [match(empty;2)?]", "[]\n".to_string()),
        ("null | try match(empty;2) catch .", String::new()),
        ("null | [scan(empty;\"g\")]", "[]\n".to_string()),
        ("null | try scan(1;empty) catch .", String::new()),
    ] {
        let (out, err, code) = run(filter)?;
        assert_eq!((out.as_str(), code), (want.as_str(), 0), "{filter}: {err}");
    }
    Ok(())
}

#[test]
fn test_nth_and_delpaths_in_path_and_try_positions_5309() -> Result<()> {
    for (filter, want, want_code) in [
        (
            "[\"a\"] | try nth(\"a\";.[]) catch .",
            "\"string (\\\"a\\\") and number (1) cannot be added\"\n",
            0,
        ),
        (
            "null | try delpaths([[0],{}]) catch .",
            "\"Path must be specified as array, not object\"\n",
            0,
        ),
        // The kind named is the lowest in value order, whatever order the entries are written in.
        (
            "[[1,\"a\",null],[null,\"a\",1],[\"a\",1,true]] | map(. as $p | try delpaths($p) catch .)",
            "[\"Path must be specified as array, not null\",\"Path must be specified as array, not null\",\"Path must be specified as array, not boolean\"]\n",
            0,
        ),
    ] {
        let (out, err, code) = run(filter)?;
        assert_eq!((out.as_str(), code), (want, want_code), "{filter}: {err}");
    }
    Ok(())
}

/// `nth` in a path position follows the same index rules; the holdout build refuses a path
/// expression that does more than navigate, so it cannot run these programs.
#[test]
fn test_nth_in_a_path_position_5309() -> Result<()> {
    for (filter, want, want_code) in [
        ("[path(null|nth(1;.))]", "[]\n", 0),
        // A negative-branch index (`null` sorts below every number) raises, even in a path.
        ("[path(nth(null;1))]", "", 5),
    ] {
        let (out, err, code) = run(filter)?;
        assert_eq!((out.as_str(), code), (want, want_code), "{filter}: {err}");
    }
    Ok(())
}

/// `fromjson` and `tonumber` word a refusal as jq's push parser does: the first fault, where it
/// was met. The one-line probes in `tests/data/jq-error-probes.tsv` hold the common shapes; these
/// are the ones a one-line table cannot: a message spanning a newline, and a NUL, which jq's
/// `fromjson` parses past (it takes a length) and its `tonumber` stops at (it takes a C string),
/// and which both quote only up to.
#[test]
fn test_json_text_refusals_name_the_first_fault_5309() -> Result<()> {
    for (filter, want) in [
        // The line and column are the parser's, so a newline restarts the column.
        (
            "\"a\\n,b\" | try fromjson catch .",
            "\"Invalid numeric literal at line 2, column 0 (while parsing 'a\\n,b')\"\n",
        ),
        // `fromjson` reads past the NUL, so the fault is later than the quotation shows.
        (
            "\"a\\u0000b c\" | try fromjson catch .",
            "\"Invalid numeric literal at line 1, column 4 (while parsing 'a')\"\n",
        ),
        // `tonumber` stops at the NUL, so it sees (and quotes) only `x`.
        (
            "\"x\\u0000 y\" | try tonumber catch .",
            "\"Invalid numeric literal at EOF at line 1, column 1 (while parsing 'x')\"\n",
        ),
        ("\"1\\u00002\" | tonumber", "1\n"),
        // A NUL inside a string is a control character.
        (
            "\"\\\"a\\u0000\\\"\" | try fromjson catch .",
            "\"Invalid string: control characters from U+0000 through U+001F must be escaped at line 1, column 4 (while parsing '\\\"a')\"\n",
        ),
        // jq's parser skips only space, tab, CR and LF: a vertical tab or a no-break space in
        // front of a number is a bad literal for `tonumber` as it is for `fromjson`.
        (
            "\"\\u000b1\" | try tonumber catch .",
            "\"Invalid numeric literal at EOF at line 1, column 2 (while parsing '\\u000b1')\"\n",
        ),
        ("\" 1\\t\\r\\n\" | tonumber", "1\n"),
        // Whitespace round a lone value is fine; a second value is the fault.
        ("\" 1 \" | fromjson", "1\n"),
        ("\" 1 2 \" | try fromjson catch .", "\"Unexpected extra JSON values (while parsing ' 1 2 ')\"\n"),
        // `tonumber` on valid JSON that is not a number is named as the string it is.
        (
            "\"[1]\" | try tonumber catch .",
            "\"string (\\\"[1]\\\") cannot be parsed as a number\"\n",
        ),
        (
            "\"[1,\" | try tonumber catch .",
            "\"Unfinished JSON term at EOF at line 1, column 3 (while parsing '[1,')\"\n",
        ),
    ] {
        let (out, err, code) = run(filter)?;
        assert_eq!((out.as_str(), code), (want, 0), "{filter}: {err}");
    }
    Ok(())
}
