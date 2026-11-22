//! #7716: a malformed object key (`{1+2:3}`, `{.a:1}`) is two compile errors in jq: bison's syntax
//! error, then `May need parentheses around object key expression` from the grammar's recovery
//! rule (`MkDictPair: error ':' ExpD`, and `ObjPat: error ':' Pattern` for a destructuring
//! pattern). `succinctly jq` reported the first alone. It now follows jq's recovery: the tokens
//! up to the first `:`, `,` or (for the first entry) `}` are dropped, the pair's value is read,
//! and a syntax error within three tokens of the recovery is not reported, as bison does.
//!
//! Each row is the errors jq 1.7.1 (`/usr/bin/jq`, macOS) prints, in order, as a syntax error
//! (`S`, whose wording is succinctly's own: see `docs/compliance/jq/limitations.md`) or the hint
//! (`H`), with the line each names; the trailer counts them. jq 1.8.2 prints the same for every
//! row but the ones `DIFFERS_IN_1_8_2` lists, where its grammar reads more (its `ExpD` takes any
//! operator), and 1.7.1 stays the reference.

use anyhow::Result;

use Kind::{H, S};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    S,
    H,
}

const HINT: &str = "May need parentheses around object key expression";

/// (program, the errors jq 1.7.1 prints for it).
const ROWS: &[(&str, &[(Kind, usize)])] = &[
    ("{1+2:3}", &[(S, 1), (H, 1)]),
    ("{1:3}", &[(S, 1), (H, 1)]),
    ("{\"a\"+\"b\":1}", &[(S, 1), (H, 1)]),
    ("{.a:1}", &[(S, 1), (H, 1)]),
    ("{a.b:1}", &[(S, 1), (H, 1)]),
    ("{1:3, 2:4}", &[(S, 1), (H, 1), (S, 1), (H, 1)]),
    ("{1:3, a:4}", &[(S, 1), (H, 1)]),
    ("{a:1, 1:3}", &[(S, 1), (H, 1)]),
    ("{1, a:2}", &[(S, 1)]),
    ("{1}", &[(S, 1)]),
    ("{1+2}", &[(S, 1)]),
    ("{1:}", &[(S, 1)]),
    ("{1:2 3}", &[(S, 1), (H, 1)]),
    ("{1: 2+3 4}", &[(S, 1), (H, 1)]),
    ("{1:2|3}", &[(S, 1), (H, 1)]),
    ("{1:-2}", &[(S, 1), (H, 1)]),
    ("{1 2 3:4}", &[(S, 1), (H, 1)]),
    ("{1:(2)}", &[(S, 1), (H, 1)]),
    ("{[1]:2}", &[(S, 1), (H, 1)]),
    ("{1:{2:3}}", &[(S, 1), (H, 1), (H, 1)]),
    ("{-1:2}", &[(S, 1), (H, 1)]),
    ("{@base64:1}", &[(S, 1), (H, 1)]),
    ("{\"a\" \"b\":1}", &[(S, 1), (H, 1)]),
    ("{a b:1}", &[(S, 1), (H, 1)]),
    ("{$x.y:1}", &[(S, 1), (H, 1)]),
    ("{.:1}", &[(S, 1), (H, 1)]),
    ("{.[0]:1}", &[(S, 1), (H, 1)]),
    ("{..:1}", &[(S, 1), (H, 1)]),
    ("{1:2}|{3:4}", &[(S, 1), (H, 1), (S, 1), (H, 1)]),
    ("[{1:2}, {3:4}]", &[(S, 1), (H, 1), (S, 1), (H, 1)]),
    ("{1:2", &[(S, 1), (H, 1)]),
    ("{1+2:3} | {4:5}", &[(S, 1), (H, 1), (S, 1), (H, 1)]),
    (". as {1:$a} | $a", &[(S, 1), (H, 1)]),
    (". as {a.b:$a} | $a", &[(S, 1), (H, 1)]),
    ("{if:1, 1:2}", &[(S, 1), (H, 1)]),
    ("{(1):2, 3:4}", &[(S, 1), (S, 1), (H, 1)]),
    // A constant non-string key's own error (`S` here too) is the pair's action: after the
    // errors in its value, and not raised for a value that fails.
    ("{(1):{,}}", &[(S, 1), (S, 1)]),
    ("{(1):}", &[(S, 1)]),
    ("{(1):2 3}", &[(S, 1), (S, 1)]),
    ("{1:2,}", &[(S, 1), (H, 1)]),
    ("{1:.a.b}", &[(S, 1), (H, 1)]),
    ("{\"\\(1)\"+\"x\":1}", &[(S, 1), (H, 1)]),
    ("{$__loc__: 1}", &[(S, 1), (H, 1)]),
    ("{1, 2:3}", &[(S, 1), (H, 1)]),
    ("{1, 2:3, 4:5}", &[(S, 1), (H, 1), (S, 1), (H, 1)]),
    ("{a:1, 2}", &[(S, 1)]),
    ("{a:1, 2, b:3}", &[(S, 1)]),
    ("{1} | {2:3}", &[(S, 1), (S, 1), (H, 1)]),
    ("{1} | 2 | {3:4}", &[(S, 1), (S, 1), (H, 1)]),
    ("{1:2} 3", &[(S, 1), (H, 1), (S, 1)]),
    ("{a: 1 + 2: 3}", &[(S, 1), (H, 1)]),
    ("{a: .b c: 1}", &[(S, 1), (H, 1)]),
    ("{(1)}", &[(S, 1)]),
    ("{1:2, }", &[(S, 1), (H, 1)]),
    ("{1:2,, a:3}", &[(S, 1), (H, 1), (S, 1)]),
    ("{\"a\":1 \"b\":2}", &[(S, 1), (H, 1)]),
    ("{a:1 b:2}", &[(S, 1), (H, 1)]),
    ("{1:{2:{3:4}}}", &[(S, 1), (H, 1), (H, 1), (H, 1)]),
    ("{a:{2:3}}", &[(S, 1), (H, 1)]),
    ("{1: {a:{2:3}}}", &[(S, 1), (S, 1), (H, 1), (H, 1)]),
    ("{1 \"x\\(1)\":2}", &[(S, 1), (H, 1)]),
    ("{1:2 | 3:4}", &[(S, 1), (H, 1), (S, 1), (H, 1)]),
    ("{1:2,3}", &[(S, 1), (H, 1), (S, 1)]),
    (". as {1:$a, 2:$b} | $a", &[(S, 1), (H, 1), (S, 1), (H, 1)]),
    (". as {1:$a} ?// [$a] | $a", &[(S, 1), (H, 1)]),
    ("{1:2} as $x | $x", &[(S, 1), (H, 1)]),
    ("{a:1} | {2+3:4} | {5:6}", &[(S, 1), (H, 1), (S, 1), (H, 1)]),
    ("{\n1+2:3}", &[(S, 2), (H, 2)]),
    ("{a:1,\n 1:2}", &[(S, 2), (H, 2)]),
    ("{\"a\"\n+\"b\":1}", &[(S, 2), (H, 1)]),
    ("{1 # c\n:2}", &[(S, 1), (H, 1)]),
    (". as {\n1:$a} | $a", &[(S, 2), (H, 2)]),
    ("{a:1, 2} | {3:4}", &[(S, 1), (H, 1)]),
    ("{1:2,3} | {4:5}", &[(S, 1), (H, 1), (S, 1), (H, 1)]),
    ("{a:\n.b c:1}", &[(S, 2), (H, 1)]),
];

/// Where this does not follow jq 1.7.1 (recorded in `docs/compliance/jq/limitations.md`): jq
/// recovers from an error inside a bracket of a pair's value at that bracket (`'[' error ']'`) and
/// reads on (`S, S, H`); here the value's error ends the parse.
const NOT_FOLLOWED: &[(&str, &[(Kind, usize)])] = &[("{1:[2:3]}", &[(S, 1), (S, 1)])];

/// The rows where jq 1.8.2 prints other errors than 1.7.1 (a hint more, an error more, or another
/// wording of the syntax error).
const DIFFERS_IN_1_8_2: &[&str] = &[
    "{1, a:2}",
    "{a:1, 2, b:3}",
    "{1: 2+3 4}",
    "{1:2,, a:3}",
    "{a: 1 + 2: 3}",
    "{1:[2:3]}",
];

/// The errors a run printed, in order, its trailer and its exit status.
type Report = (Vec<(Kind, usize)>, String, Option<i32>);

fn errors_of(program: &str) -> Result<Report> {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_succinctly"))
        .args(["jq", "-n", "-c", program])
        .env("LC_ALL", "C")
        .env_remove("JQ_LIBRARY_PATH")
        .output()?;
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    let mut errors = Vec::new();
    let mut trailer = String::new();
    for line in stderr.lines() {
        if let Some(rest) = line.strip_prefix("jq: error: ") {
            let (message, place) = rest
                .rsplit_once(" at <top-level>, line ")
                .unwrap_or_else(|| panic!("{program:?}: {line:?}"));
            let line_number = place.trim_end_matches(':').parse()?;
            errors.push((if message == HINT { H } else { S }, line_number));
        } else if line.starts_with("jq: ") {
            trailer = line.to_string();
        }
    }
    Ok((errors, trailer, output.status.code()))
}

fn trailer(count: usize) -> String {
    if count == 1 {
        "jq: 1 compile error".to_string()
    } else {
        format!("jq: {count} compile errors")
    }
}

#[test]
fn a_malformed_object_key_is_reported_as_jq_reports_it_7716() -> Result<()> {
    for program in DIFFERS_IN_1_8_2 {
        assert!(
            ROWS.iter()
                .chain(NOT_FOLLOWED)
                .any(|(row, _)| row == program),
            "{program:?}"
        );
    }
    for &(program, want) in ROWS.iter().chain(NOT_FOLLOWED) {
        assert_eq!(
            errors_of(program)?,
            (want.to_vec(), trailer(want.len()), Some(3)),
            "{program:?}"
        );
    }
    Ok(())
}

/// A recovery never goes past the parser running out of room: bison's `memory exhausted` ends the
/// parse and is always reported, so twenty thousand nested malformed keys are the first syntax
/// error and that, as in jq 1.7.1 (which runs out sooner, at a few thousand levels).
#[test]
fn running_out_of_room_ends_the_recovery_7716() -> Result<()> {
    let depth = 20_000;
    let program = format!("{}1{}", "{1:".repeat(depth), "}".repeat(depth));
    let (errors, trailer, status) = errors_of(&program)?;
    assert_eq!(
        (errors, trailer.as_str(), status),
        (vec![(S, 1), (S, 1)], "jq: 2 compile errors", Some(3))
    );
    Ok(())
}
