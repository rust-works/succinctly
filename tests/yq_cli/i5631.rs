//! #5631: yq v4.53.3's ordering comparisons (`<`, `<=`, `>`, `>=`) are not jq's total order. A
//! container on the left raises `arrays`/`maps not yet supported for comparison`; a scalar against
//! a container on the right raises `<right tag> () cannot be subtracted from <left tag>`; a `null`
//! on either side answers `false`; two numbers or two strings compare; anything else (a bool, or
//! two kinds that do not compare) raises `<left tag> not yet supported for comparison`. `==` and
//! `!=` never raise, and jq mode keeps its total order. Every row was captured from yq v4.53.3.

use std::io::Write;
use std::process::{Command, Stdio};

use anyhow::Result;

/// `(program, Ok(stdout) | Err(stderr))` for `yq -n`.
const ROWS: &[(&str, Result<&str, &str>)] = &[
    // The two kinds that do compare.
    ("2 > 1", Ok("true\n")),
    ("1 <= 1.5", Ok("true\n")),
    ("1.5 >= 2", Ok("false\n")),
    ("\"a\" < \"b\"", Ok("true\n")),
    ("\"1\" >= \"1\"", Ok("true\n")),
    // `null` on either side answers false, except both-null with `<=`/`>=`.
    ("null < 1", Ok("false\n")),
    ("1 < null", Ok("false\n")),
    ("null <= null", Ok("true\n")),
    ("true > null", Ok("false\n")),
    ("\"a\" >= null", Ok("false\n")),
    // A bool, or two kinds that do not compare, names the left operand's tag.
    ("2 > true", Err("!!int not yet supported for comparison")),
    (
        "true > false",
        Err("!!bool not yet supported for comparison"),
    ),
    ("false <= 1", Err("!!bool not yet supported for comparison")),
    ("1 < \"a\"", Err("!!int not yet supported for comparison")),
    ("\"a\" < 1", Err("!!str not yet supported for comparison")),
    (
        "1.5 >= \"1\"",
        Err("!!float not yet supported for comparison"),
    ),
    (
        "\"a\" <= true",
        Err("!!str not yet supported for comparison"),
    ),
    // A container on the left, whatever is on the right (a `null` included).
    ("[] < 1", Err("arrays not yet supported for comparison")),
    ("[1] >= [1]", Err("arrays not yet supported for comparison")),
    ("[] < null", Err("arrays not yet supported for comparison")),
    ("{} > 1", Err("maps not yet supported for comparison")),
    (
        "{\"a\":1} <= {\"a\":1}",
        Err("maps not yet supported for comparison"),
    ),
    ("{} >= null", Err("maps not yet supported for comparison")),
    ("[] < {}", Err("arrays not yet supported for comparison")),
    // A scalar on the left against a container on the right.
    ("1 < []", Err("!!seq () cannot be subtracted from !!int")),
    ("1 < [1]", Err("!!seq () cannot be subtracted from !!int")),
    (
        "\"a\" > {\"a\":1}",
        Err("!!map () cannot be subtracted from !!str"),
    ),
    (
        "true >= []",
        Err("!!seq () cannot be subtracted from !!bool"),
    ),
    (
        "null < []",
        Err("!!seq () cannot be subtracted from !!null"),
    ),
    (
        "null >= {}",
        Err("!!map () cannot be subtracted from !!null"),
    ),
    // Equality never raises.
    ("2 == true", Ok("false\n")),
    ("[] != 1", Ok("true\n")),
    ("{} == {}", Ok("false\n")),
];

fn run(args: &[&str], stdin: &str) -> Result<(String, String, i32)> {
    let mut child = Command::new(env!("CARGO_BIN_EXE_succinctly"))
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    child.stdin.take().unwrap().write_all(stdin.as_bytes())?;
    let output = child.wait_with_output()?;
    Ok((
        String::from_utf8(output.stdout)?,
        String::from_utf8(output.stderr)?,
        output.status.code().unwrap_or(-1),
    ))
}

#[test]
fn ordering_comparison_matches_yq_for_every_pairing_5631() -> Result<()> {
    for (program, expected) in ROWS {
        let (stdout, stderr, code) = run(&["yq", "-n", program], "")?;
        match expected {
            Ok(want) => {
                assert_eq!(&stdout, want, "{program}: stderr {stderr:?}");
                assert_eq!(code, 0, "{program}");
            }
            Err(message) => {
                assert_eq!(stdout, "", "{program}");
                assert_eq!(stderr, format!("Error: {message}\n"), "{program}");
                assert_eq!(code, 1, "{program}");
            }
        }
    }
    Ok(())
}

/// The same refusal reaches every route a comparison is evaluated on: the plain fan-out, a
/// `select` condition, a map, an update, a delete and a path-selecting assignment.
#[test]
fn ordering_comparison_raises_on_every_evaluation_route_5631() -> Result<()> {
    let input = "a: [1, 5, x, 9]\nm: {p: 1, q: foo}\n";
    for program in [
        ".a[] | . > 3",
        ".a[] | select(. > 3)",
        "[.a[] | select(. < 5)]",
        ".a | map(. > 3)",
        ".a[] |= (. > 2)",
        "del(.a[] | select(. > 3))",
        "(.a[] | select(. > 3)) = 0",
        ".m | with_entries(select(.value > 1))",
        ".m | map_values(. > 0)",
        ".a | sort_by(. > 0)",
        ".a | group_by(. > 3)",
        ".a | map(. > 3 and . < 9)",
    ] {
        let (stdout, stderr, code) = run(&["yq", program], input)?;
        assert_eq!(stdout, "", "{program}");
        assert_eq!(
            stderr, "Error: !!str not yet supported for comparison\n",
            "{program}"
        );
        assert_eq!(code, 1, "{program}");
    }
    for program in [".. | select(. > 1)", "del(.m | select(. > 1))", ". < 1"] {
        let (stdout, stderr, code) = run(&["yq", program], input)?;
        assert_eq!(stdout, "", "{program}");
        assert_eq!(
            stderr, "Error: maps not yet supported for comparison\n",
            "{program}"
        );
        assert_eq!(code, 1, "{program}");
    }
    Ok(())
}

/// An absent path is `null` and answers `false`; a comparison of two numbers still filters.
#[test]
fn ordering_comparison_still_filters_numbers_and_absent_paths_5631() -> Result<()> {
    let (stdout, _, code) = run(&["yq", ".a[] | select(. > 3)"], "a: [1, 5, 9]\n")?;
    assert_eq!((stdout.as_str(), code), ("5\n9\n", 0));
    let (stdout, _, code) = run(&["yq", ".zzz < 1"], "a: 1\n")?;
    assert_eq!((stdout.as_str(), code), ("false\n", 0));
    Ok(())
}

/// jq mode keeps its total order (`null < bool < number < string < array < object`).
#[test]
fn jq_mode_ordering_is_still_total_5631() -> Result<()> {
    for (program, want) in [
        ("2 > true", "true\n"),
        ("\"a\" < 1", "false\n"),
        ("[] < 1", "false\n"),
        ("{} > []", "true\n"),
        ("null < false", "true\n"),
    ] {
        let (stdout, _, code) = run(&["jq", "-n", program], "")?;
        assert_eq!((stdout.as_str(), code), (want, 0), "{program}");
    }
    Ok(())
}
