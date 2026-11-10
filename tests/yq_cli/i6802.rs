//! #6802: yq's `ireduce` folds a *list* of accumulators (`reduceOperator`): every output of UPDATE is
//! an accumulator of the next step, a union in UPDATE is branch-major over the list, an UPDATE with
//! no output drops its accumulator, and several INIT outputs all fold the one source (jq's later
//! forks read a `null` source). Every row is `yq -o=json -I0 FILTER` on [`DOC`], captured from yq
//! v4.53.3. `docs/compliance/yq/limitations.md` records the absent-key read that still differs.

use anyhow::Result;

use crate::{run_yq_stdin, run_yq_stdin_with_stderr};

const DOC: &str = "l: [1, 2]\nm: {a: 1, b: 2}\ne: []\n";

/// (filter, stdout), each the same as yq v4.53.3's with exit code 0.
const ROWS: &[(&str, &str)] = &[
    (".l[] as $i ireduce((0, 100); . + $i)", "3\n103\n"),
    (".l[] as $i ireduce(0; select(false))", ""),
    (".l[] as $i ireduce(0; select(. < 1))", "0\n"),
    (".l[] as $i ireduce(0; (. + $i, . * 10))", "3\n2\n10\n0\n"),
    (
        ".l[] as $i ireduce((1, 2); (. + $i, . * 10))",
        "4\n5\n12\n22\n20\n30\n100\n200\n",
    ),
    ("[.l[] as $i ireduce(0; (. + $i, . * 10))]", "[3,2,10,0]\n"),
    (
        ".l[] as $i ireduce(0; ((. + $i), (. + $i)))",
        "3\n3\n3\n3\n",
    ),
    (".l[] as $i ireduce(0; . + $i)", "3\n"),
    (".e[] as $i ireduce(5; . + $i)", "5\n"),
    (".m[] as $i ireduce(0; . + $i)", "3\n"),
    (".l[] as $i ireduce([]; . + [$i])", "[1,2]\n"),
    (
        ".l[] as $i ireduce((0, 10); (. + $i, . - $i))",
        "3\n13\n1\n11\n-1\n9\n-3\n7\n",
    ),
    (".l[] as $i ireduce(0; (., .))", "0\n"),
    (".l[] as $i ireduce({}; .a += $i)", "{\"a\":3}\n"),
    (
        ".l[] as $i ireduce((1, 2, 3); select(. != 2) | . + $i)",
        "6\n",
    ),
    (
        ".l[] as $i ireduce(0; (. + 1) , (. + 2) | . * 2)",
        "2\n5\n6\n12\n",
    ),
    (".l[] as $i ireduce(0; [.] | .[0] + $i)", "3\n"),
    (".l[] as $i ireduce(0; . + $i) | . + 1", "4\n"),
];

#[test]
fn every_row_matches_yq_6802() -> Result<()> {
    for (filter, expected) in ROWS {
        let (stdout, code) = run_yq_stdin(filter, DOC, &["-o", "json", "-I0"])?;
        assert_eq!(code, 0, "{filter}");
        assert_eq!(&stdout, expected, "{filter}");
    }
    Ok(())
}

/// Under `--eval-all` the accumulators fold the document list: `0` is one seed per document and each
/// seed's UPDATE fans out over the list of accumulators.
#[test]
fn the_accumulator_list_folds_the_document_list_under_eval_all_6802() -> Result<()> {
    let (stdout, stderr, code) = run_yq_stdin_with_stderr(
        ". as $i ireduce (0; (. + $i.a, . * 10))",
        "a: 1\n---\na: 2\n",
        &["--eval-all", "-o", "json", "-I0"],
    )?;
    assert_eq!(code, 0, "{stderr}");
    assert_eq!(stdout, "3\n3\n2\n2\n10\n10\n0\n0\n");
    Ok(())
}

/// An error in UPDATE ends the run with no output.
#[test]
fn an_error_in_the_update_prints_nothing_6802() -> Result<()> {
    let (stdout, code) = run_yq_stdin(
        ".l[] as $i ireduce (0; error(\"x\"))",
        DOC,
        &["-o", "json", "-I0"],
    )?;
    assert_ne!(code, 0);
    assert_eq!(stdout, "");
    Ok(())
}

/// A source of three elements: the list a step leaves is the one the next step starts from.
const THREE: &[(&str, &str)] = &[
    (
        ".n[] as $i ireduce(0; (. + $i, . * 10))",
        "6\n5\n13\n3\n30\n20\n100\n0\n",
    ),
    (
        ".n[] as $i ireduce((1, 2); (. + $i, . - 1))",
        "7\n8\n5\n6\n4\n5\n2\n3\n3\n4\n1\n2\n0\n1\n-2\n-1\n",
    ),
    (
        ".n[] as $i ireduce(0; ((. + $i), (. * 2)) | . + 1)",
        "9\n8\n9\n7\n11\n9\n11\n7\n",
    ),
    (".n[] as $i ireduce(0; select(. < 4) | . + $i)", "6\n"),
    (
        ".n[] as $i ireduce([]; . + [$i], . + [$i * 10])",
        "[1,2,3]\n[10,2,3]\n[1,20,3]\n[10,20,3]\n[1,2,30]\n[10,2,30]\n[1,20,30]\n[10,20,30]\n",
    ),
];

#[test]
fn the_list_carries_from_step_to_step_6802() -> Result<()> {
    for (filter, expected) in THREE {
        let (stdout, code) = run_yq_stdin(filter, "n: [1, 2, 3]\n", &["-o", "json", "-I0"])?;
        assert_eq!(code, 0, "{filter}");
        assert_eq!(&stdout, expected, "{filter}");
    }
    Ok(())
}

/// A list that doubles every step is refused at the step budget, where yq grows it until it dies.
#[test]
fn a_list_that_doubles_every_step_is_refused_6802() -> Result<()> {
    let input = "n: [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20]\n";
    let (stdout, code) = run_yq_stdin(
        ".n[] as $i ireduce (0; (. + 1, . + 2)) | length",
        input,
        &["-o", "json", "-I0"],
    )?;
    assert_ne!(code, 0);
    assert_eq!(stdout, "");
    Ok(())
}

/// An INIT that raises ends the run, and an INIT with no output prints nothing and never reads the
/// source: both as yq's.
#[test]
fn an_init_that_raises_or_answers_nothing_6802() -> Result<()> {
    let (stdout, code) = run_yq_stdin(
        ".l[] as $i ireduce (error(\"i\"); . + $i)",
        DOC,
        &["-o", "json", "-I0"],
    )?;
    assert_ne!(code, 0);
    assert_eq!(stdout, "");
    let (stdout, code) = run_yq_stdin(
        ".l[] as $i ireduce (select(false); . + $i)",
        DOC,
        &["-o", "json", "-I0"],
    )?;
    assert_eq!(code, 0);
    assert_eq!(stdout, "");
    Ok(())
}

/// A destructuring pattern the element does not fit raises (succinctly's own `--jq-extensions`
/// syntax; yq has none).
#[test]
fn a_pattern_the_element_does_not_fit_raises_6802() -> Result<()> {
    let (stdout, code) = run_yq_stdin(
        ".l[] as [$a] ireduce (0; . + 1)",
        DOC,
        &["--jq-extensions", "-o", "json", "-I0"],
    )?;
    assert_ne!(code, 0);
    assert_eq!(stdout, "");
    Ok(())
}

/// The right operand of `. + X` is read-only in yq, so a key of the loop variable that is not there
/// is no output (`. + [$i.a]` adds `[]`, not `[null]`) -- while the same read as a whole UPDATE, or
/// as the value of a write, is a `null` node. Every row is `yq -o=json -I0` on `b: 1`.
const ABSENT_ROWS: &[(&str, &str)] = &[
    (". as $i ireduce ([]; . + [$i.a])", "[]\n"),
    (". as $i ireduce ([]; . + [$i.b])", "[1]\n"),
    (". as $i ireduce ([]; . + [$i.a.c])", "[]\n"),
    (". as $i ireduce ([]; . + [$i | .a])", "[]\n"),
    (". as $i ireduce ([]; . + [$i.a, 1])", "[1]\n"),
    (". as $i ireduce ({}; . + {\"k\": $i.a})", "{}\n"),
    (". as $i ireduce ({}; . + {\"k\": $i.b})", "{\"k\":1}\n"),
    (". as $i ireduce (0; . + $i.a)", "0\n"),
    (". as $i ireduce (0; . + $i.b)", "1\n"),
    (". as $i ireduce (0; . + 1 + $i.a)", "1\n"),
    (". as $i ireduce (\"s\"; . + \"x\" + $i.b)", "\"sx1\"\n"),
    (". as $i ireduce ([]; . + [$i.a // 7])", "[7]\n"),
    (". as $i ireduce (0; $i.a // 5)", "5\n"),
    // The same read as the whole UPDATE or as a write's value is a `null` node.
    (". as $i ireduce ([]; [$i.a])", "[null]\n"),
    (". as $i ireduce ({}; .x = $i.a)", "{\"x\":null}\n"),
    // A plain pipe took the same shortcut.
    ("{\"b\":1} as $i | [] | . + [$i.a]", "[]\n"),
];

#[test]
fn an_absent_key_of_the_loop_variable_is_no_output_in_an_arithmetic_operand_6802() -> Result<()> {
    for (filter, expected) in ABSENT_ROWS {
        let (stdout, code) = run_yq_stdin(filter, "b: 1\n", &["-o", "json", "-I0"])?;
        assert_eq!(code, 0, "{filter}");
        assert_eq!(&stdout, expected, "{filter}");
    }
    Ok(())
}

/// Under `--eval-all` the loop variable is each document in turn and `.a` is absent from two of
/// the three.
#[test]
fn an_absent_key_of_the_loop_variable_under_eval_all_6802() -> Result<()> {
    for (filter, expected) in [
        (". as $i ireduce ([]; . + [$i.a])", "[3]\n"),
        (". as $i ireduce ([]; . + [$i.b])", "[1,2]\n"),
        (". as $i ireduce ({}; . + {\"k\": $i.a})", "{\"k\":3}\n"),
        (". as $i ireduce (0; . + $i.a)", "3\n3\n3\n"),
    ] {
        let (stdout, stderr, code) = run_yq_stdin_with_stderr(
            filter,
            "b: 1\n---\nb: 2\n---\na: 3\n",
            &["--eval-all", "-o", "json", "-I0"],
        )?;
        assert_eq!(code, 0, "{filter}: {stderr}");
        assert_eq!(stdout, expected, "{filter}");
    }
    Ok(())
}

/// jq's `reduce` has no read-only operand: an absent key is `null`.
#[test]
fn jq_mode_keeps_the_null_6802() -> Result<()> {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_succinctly"))
        .args(["jq", "-c", "reduce . as $i ([]; . + [$i.a])"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .and_then(|mut child| {
            use std::io::Write;
            child.stdin.take().expect("piped").write_all(b"{\"b\":1}")?;
            child.wait_with_output()
        })?;
    assert_eq!(String::from_utf8_lossy(&output.stdout), "[null]\n");
    Ok(())
}
