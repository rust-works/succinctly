//! #5791: `map_values(f)` reads only the first output of `f`, as jq's `.[] |= f` does, so a control
//! `f` raises after producing a value (an error, here) never happens. The evaluator ran all of `f`
//! and surfaced the error in place of the first output. Each row reads a file and then the same
//! text from stdin; the answers and exit statuses are captured from jq 1.7.1 (`/usr/bin/jq`).

use anyhow::Result;

use crate::run_jq_full;

/// `(input, filter, stdout, exit status)`.
const ROWS: &[(&str, &str, &str, i32)] = &[
    ("[1]", "map_values((1, error(\"x\")))", "[1]\n", 0),
    (
        "{\"a\":1,\"b\":2}",
        "map_values((1, error(\"x\")))",
        "{\"a\":1,\"b\":1}\n",
        0,
    ),
    (
        "[1,2]",
        "map_values((.+10, error(\"x\")), 5)",
        "[11,12]\n",
        0,
    ),
    ("false", "[.] | map_values((., .[\"a\"]))", "[false]\n", 0),
    ("[[1]]", "map_values((.[0], error(\"x\")))", "[1]\n", 0),
    // Nothing is produced before the error, or the error is the only way out of `f`: jq raises.
    ("[1]", "map_values((7, 1) | error(\"x\"))", "", 5),
    ("[1]", "map_values(empty, error(\"x\"))", "", 5),
    ("[1]", "map_values((error(\"x\"), 1))", "", 5),
    ("{\"a\":1}", "map_values((error(\"x\"), 1))", "", 5),
    // Nothing before the control: it is raised or caught as ever.
    ("[1]", "label $x | map_values(empty, break $x)", "", 0),
    ("[1]", "map_values(empty, halt_error)", "", 5),
    // The value is kept and a `break` after it is never reached.
    ("[3]", "label $x | map_values((3, break $x))", "[3]\n", 0),
];

#[test]
fn map_values_reads_the_first_output_of_f_only_5791() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("in.json");
    for (input, filter, stdout, code) in ROWS {
        std::fs::write(&path, format!("{input}\n"))?;
        let (out, err, status) = run_jq_full(&["-c", filter, path.to_str().unwrap()], None)?;
        assert_eq!(
            (out.as_str(), status),
            (*stdout, *code),
            "{filter} over {input} (file): {err}"
        );
        let (out, err, status) = run_jq_full(&["-c", filter], Some(&format!("{input}\n")))?;
        assert_eq!(
            (out.as_str(), status),
            (*stdout, *code),
            "{filter} over {input} (stdin): {err}"
        );
    }
    Ok(())
}

/// The same rule for a body that reads the member's position (`key`, a succinctly extension in jq
/// mode with no jq to capture it from), which runs the body once per member on a separate route.
#[test]
fn map_values_reads_the_first_output_of_a_positioned_body_5791() -> Result<()> {
    for (input, filter, stdout) in [
        (
            "{\"a\":1}",
            "map_values((key, error(\"x\")))",
            "{\"a\":\"a\"}\n",
        ),
        ("[5]", "map_values((key, error(\"x\")))", "[0]\n"),
    ] {
        let (out, err, status) = run_jq_full(&["-c", filter], Some(&format!("{input}\n")))?;
        assert_eq!(
            (out.as_str(), status),
            (stdout, 0),
            "{filter} over {input}: {err}"
        );
    }
    Ok(())
}
