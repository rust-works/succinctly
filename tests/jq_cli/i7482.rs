//! #7482: a malformed document left with containers open at end of input, `{"a":` and 44 `[`
//! closed by one `}`, panicked with an index out of bounds in `BalancedParens::is_open` (exit 101).
//! The standard JSON index takes its parenthesis length as two bits per open, the length of a
//! balanced document, and the closes such a document lacks were the zero bits past the last one
//! written: once they crossed a word boundary the length ran past the stored words. Every row here
//! is jq 1.7.1's own output (jq 1.8.2 prints the same lines; above 255 levels it has a deeper
//! parse limit, a 1.8 change that is not this issue).

use std::io::Write;

use anyhow::Result;
use tempfile::NamedTempFile;

use crate::{run_jq_full, succinctly_bin};

fn doc(prefix: &str, depth: usize, tail: &str) -> String {
    format!("{prefix}{}{tail}\n", "[".repeat(depth))
}

fn file_of(text: &str) -> Result<NamedTempFile> {
    let mut file = NamedTempFile::new()?;
    file.write_all(text.as_bytes())?;
    Ok(file)
}

fn no_panic(what: &str, stderr: &str, code: i32) {
    assert!(
        !stderr.contains("panicked") && (0..100).contains(&code),
        "{what}: exit {code}, {stderr}"
    );
}

/// The issue's reproducer over the routes that panicked, from a file and from a pipe.
#[test]
fn a_document_open_at_end_of_input_past_a_word_reports_jqs_parse_error_7482() -> Result<()> {
    let text = doc("{\"a\":", 44, "1,\"b\":2}");
    let file = file_of(&text)?;
    let path = file.path().to_str().unwrap();
    let parse = "jq: parse error: ':' not as part of an object at line 1, column 55\n";
    for program in [&["."][..], &["-c", "."], &[".a"], &[".[]"], &[".a[0]"]] {
        let mut args = program.to_vec();
        assert_eq!(
            run_jq_full(&args, Some(&text))?,
            (String::new(), parse.to_string(), 5),
            "{program:?} on a pipe"
        );
        args.push(path);
        assert_eq!(
            run_jq_full(&args, None)?,
            (String::new(), parse.to_string(), 5),
            "{program:?} on a file"
        );
    }
    assert_eq!(
        run_jq_full(&["-s", ".", path], None)?,
        (String::new(), parse.to_string(), 5)
    );
    let raised =
        format!("jq: error (at {path}:1): ':' not as part of an object at line 1, column 55\n");
    for program in ["input", "[inputs]"] {
        assert_eq!(
            run_jq_full(&["-n", program, path], None)?,
            (String::new(), raised.clone(), 5),
            "-n {program}"
        );
    }
    // A slurp of a pipe and `--slurpfile` word the fault their own way at any depth (the
    // `--slurpfile` text is #3051's accepted divergence); here they must only not panic.
    let (_, stderr, code) = run_jq_full(&["-s", "."], Some(&text))?;
    assert_eq!(code, 5, "-s . on a pipe: {stderr}");
    let (_, stderr, code) = run_jq_full(&["-n", "--slurpfile", "x", path, "$x"], None)?;
    assert_eq!(code, 2, "--slurpfile: {stderr}");
    assert!(
        stderr.starts_with(&format!("jq: Bad JSON in --slurpfile x {path}: ")),
        "{stderr}"
    );
    Ok(())
}

/// Every depth the issue's sweep found panicking, and depths past jq's parse limit.
#[test]
fn each_depth_that_panicked_reports_jqs_parse_error_7482() -> Result<()> {
    let depths = (44..=54).chain([90, 91]).chain((107..=139).step_by(4));
    for depth in depths.chain([200, 255, 300, 3000]) {
        let text = doc("{\"a\":", depth, "1,\"b\":2}");
        let want = if depth < 255 {
            format!(
                "jq: parse error: ':' not as part of an object at line 1, column {}\n",
                depth + 11
            )
        } else {
            "jq: parse error: Exceeds depth limit for parsing at line 1, column 260\n".to_string()
        };
        assert_eq!(
            run_jq_full(&["."], Some(&text))?,
            (String::new(), want, 5),
            "depth {depth}"
        );
    }
    Ok(())
}

/// The issue's other malformed tails at depth 44, under a named and an empty key.
#[test]
fn the_other_unclosed_tails_report_jqs_parse_error_7482() -> Result<()> {
    for (prefix, base) in [("{\"a\":", 55), ("{\"\":", 54)] {
        for (tail, message, column) in [
            (
                "1,\"b\",2}",
                "Objects must consist of key:value pairs",
                base + 2,
            ),
            ("1,\"b\":2]}", "':' not as part of an object", base),
            ("1,\"b\":{}}", "':' not as part of an object", base),
            ("1,\"b\":2,\"c\":3}", "':' not as part of an object", base),
        ] {
            let text = doc(prefix, 44, tail);
            assert_eq!(
                run_jq_full(&["-c", "."], Some(&text))?,
                (
                    String::new(),
                    format!("jq: parse error: {message} at line 1, column {column}\n"),
                    5
                ),
                "{text}"
            );
        }
    }
    Ok(())
}

/// The other commands that index the same text have no jq to match; they must not panic.
#[test]
fn the_other_json_readers_do_not_panic_on_it_7482() -> Result<()> {
    let text = doc("{\"a\":", 44, "1,\"b\":2}");
    let file = file_of(&text)?;
    let path = file.path().to_str().unwrap();
    for args in [
        &["yq", "-o", "json", ".", path][..],
        &["yq", "-p", "json", ".", path],
        &["jq-locate", path, "--offset", "50"],
        &["jq-locate", path, "--line", "1", "--column", "52"],
        &["json", "validate", path],
    ] {
        let output = std::process::Command::new(succinctly_bin())
            .args(args)
            .output()?;
        let stderr = String::from_utf8_lossy(&output.stderr);
        no_panic(
            &format!("{args:?}"),
            &stderr,
            output.status.code().unwrap_or(-1),
        );
    }
    Ok(())
}
