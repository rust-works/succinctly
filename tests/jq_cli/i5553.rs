//! #5553: under `-C`, jq colors the line `debug`/`debug(msg)` write to stderr as it colors any
//! value it prints (compact: it drops the indent flags and keeps color, sorting and ASCII). The
//! line was written uncolored. Every expected byte string was captured from jq 1.7.1
//! (`/usr/bin/jq`).

use anyhow::Result;

use crate::run_jq_full;

/// The `["DEBUG:","m"]` line, colored as jq 1.7.1 does.
const COLORED_MSG: &str = "\x1b[1;39m[\x1b[0;32m\"DEBUG:\"\x1b[0m\x1b[1;39m,\x1b[0;32m\"m\"\x1b[0m\x1b[1;39m\x1b[1;39m]\x1b[0m\n";

fn debug_stderr(args: &[&str], filter: &str) -> Result<String> {
    let mut all = vec!["-n"];
    all.extend_from_slice(args);
    all.push(filter);
    let (_, stderr, code) = run_jq_full(&all, None)?;
    assert_eq!(code, 0, "{filter}: {stderr}");
    Ok(stderr)
}

#[test]
fn test_debug_line_is_colored_under_dash_c_5553() -> Result<()> {
    for args in [&["-C"][..], &["-C", "-c"], &["-C", "--tab"]] {
        assert_eq!(
            debug_stderr(args, r#""x" | debug("m") | empty"#)?,
            COLORED_MSG
        );
    }
    assert_eq!(
        debug_stderr(&["-C"], "[1,\"x\"] | debug | empty")?,
        "\x1b[1;39m[\x1b[0;32m\"DEBUG:\"\x1b[0m\x1b[1;39m,\x1b[1;39m[\x1b[0;39m1\x1b[0m\x1b[1;39m,\x1b[0;32m\"x\"\x1b[0m\x1b[1;39m\x1b[1;39m]\x1b[0m\x1b[1;39m\x1b[1;39m]\x1b[0m\n"
    );
    // One line per output of `msg`, each colored.
    assert_eq!(
        debug_stderr(&["-C"], "1 | debug(1, 2) | empty")?,
        "\x1b[1;39m[\x1b[0;32m\"DEBUG:\"\x1b[0m\x1b[1;39m,\x1b[0;39m1\x1b[0m\x1b[1;39m\x1b[1;39m]\x1b[0m\n\
         \x1b[1;39m[\x1b[0;32m\"DEBUG:\"\x1b[0m\x1b[1;39m,\x1b[0;39m2\x1b[0m\x1b[1;39m\x1b[1;39m]\x1b[0m\n"
    );
    Ok(())
}

/// The line is the compact colored rendering of `["DEBUG:", v]` that stdout would print under
/// `-C -c`, keys, a NaN and the non-ASCII `-a` escape included; `-S` and `-a` apply to it.
#[test]
fn test_colored_debug_line_is_the_colored_stdout_rendering_5553() -> Result<()> {
    let value = r#"{"b":[1,null,{"z":true,"a":"é"}],"a":nan}"#;
    for args in [
        &["-C"][..],
        &["-C", "-S"],
        &["-C", "-a"],
        &["-C", "-S", "-a"],
    ] {
        let stderr = debug_stderr(args, &format!("{value} | debug | empty"))?;
        let mut stdout_args = vec!["-n", "-c"];
        stdout_args.extend_from_slice(args);
        let program = format!(r#"["DEBUG:", {value}]"#);
        stdout_args.push(&program);
        let (stdout, _, code) = run_jq_full(&stdout_args, None)?;
        assert_eq!(code, 0);
        assert_eq!(stderr, stdout, "{args:?}");
    }
    Ok(())
}

/// A value past jq's print depth is replaced by `<skipped: too deep>` in the replaced value's
/// color, on the debug line as on stdout. 256 arrays around the value put it one level past the
/// limit once the `["DEBUG:", ...]` wrapper is counted, so the skipped value is the leaf itself.
#[test]
fn test_colored_debug_line_marks_a_too_deep_value_5553() -> Result<()> {
    for (init, color) in [
        ("0", "0;39"),
        ("\"s\"", "0;32"),
        ("null", "0;90"),
        ("true", "0;39"),
        ("false", "0;39"),
        ("[]", "1;39"),
        ("{}", "1;39"),
    ] {
        let nested = format!("reduce range(256) as $i ({init}; [.])");
        let stderr = debug_stderr(&["-C"], &format!("{nested} | debug | empty"))?;
        let (stdout, _, _) = run_jq_full(
            &["-n", "-C", "-c", &format!("[\"DEBUG:\", {nested}]")],
            None,
        )?;
        assert_eq!(stderr, stdout, "{init}");
        assert!(
            stderr.contains(&format!("\x1b[{color}m<skipped: too deep>\x1b[0m")),
            "{init}: {stderr}"
        );
    }
    Ok(())
}

/// What stays as it was: no `-C`, `-M`, and `stderr`/an uncaught `error`, which jq does not color.
#[test]
fn test_uncolored_debug_line_is_unchanged_5553() -> Result<()> {
    for args in [&[][..], &["-M"], &["-c"]] {
        assert_eq!(
            debug_stderr(args, r#""x" | debug("m") | empty"#)?,
            "[\"DEBUG:\",\"m\"]\n"
        );
    }
    assert_eq!(debug_stderr(&["-C"], r#""x" | stderr | empty"#)?, "x");
    let (_, stderr, code) = run_jq_full(&["-n", "-C", r#"{"a":1} | error"#], None)?;
    assert_eq!(code, 5);
    assert_eq!(
        stderr,
        "jq: error (at <unknown>) (not a string): {\"a\":1}\n"
    );
    Ok(())
}
