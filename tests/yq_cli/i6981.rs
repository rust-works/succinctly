//! #6981: yq v4.53.3 has no `/=`, `%=` or `//=` operator (it reads `/`, `%` or `//` with a missing
//! operand and stops: `'/' expects 2 args but there is 1`), so `succinctly yq` refuses them unless
//! `--jq-extensions` is passed, as it does a jq-only builtin. `+=`, `-=`, `*=` and `|=` are yq's
//! own and need no flag; `succinctly jq` always accepts all of them.
//!
//! The refusals are captured from yq v4.53.3 (`a: 4`); the answers with the flag are this tool's
//! own, since there is no yq answer to match.

use std::io::Write;

use anyhow::Result;
use tempfile::NamedTempFile;

use crate::{run_jq_stdin, run_yq_files, run_yq_stdin, run_yq_stdin_with_stderr};

const GATED: &[(&str, &str)] = &[(".a /= 2", "/="), (".a %= 2", "%="), (".a //= 3", "//=")];

#[test]
fn the_compound_assignments_yq_lacks_are_refused_without_the_flag_6981() -> Result<()> {
    for (filter, op) in GATED {
        let (out, err, code) = run_yq_stdin_with_stderr(filter, "a: 4\n", &["-o=json", "-I=0"])?;
        assert_ne!(code, 0, "{filter}: stdout={out:?}");
        assert!(out.is_empty(), "{filter}: nothing is written: {out:?}");
        assert!(
            err.contains(&format!("\"{op}\" is not part of yq's syntax"))
                && err.contains("--jq-extensions"),
            "{filter}: {err:?}"
        );
    }
    Ok(())
}

/// The gate sits in the assignment parser, so it has to hold wherever an assignment can: after a
/// pipe, in a parenthesised or comma target, under `with`, nested, behind a path, and on the
/// `--eval-all`, `-P` and `--inplace` routes. yq refuses every one of these filters.
#[test]
fn the_refusal_holds_in_every_position_an_assignment_can_be_in_6981() -> Result<()> {
    for filter in [
        ".a /= 2 | .b",
        ".b = 1 | .a %= 2",
        "(.a, .b) //= 3",
        "(.a /= 2)",
        "with(.a; . /= 2)",
        ".a.b //= 3",
        ".[\"a\"] %= 3",
        "[.a] | .[0] /= 2",
        ".c = (.a //= 3)",
    ] {
        for args in [&["-o=json", "-I=0"][..], &["-P"], &["--eval-all"]] {
            let (out, err, code) = run_yq_stdin_with_stderr(filter, "a: 4\n", args)?;
            assert_ne!(code, 0, "{filter} {args:?}: stdout={out:?}");
            assert!(
                err.contains("is not part of yq's syntax") && err.contains("--jq-extensions"),
                "{filter} {args:?}: {err:?}"
            );
        }
    }
    Ok(())
}

/// `--inplace` refuses before it touches the file.
#[test]
fn inplace_refuses_before_writing_6981() -> Result<()> {
    let mut file = NamedTempFile::new()?;
    writeln!(file, "a: 4")?;
    let (_, stderr, code) = run_yq_files(".a /= 2", &[file.path()], &["-i"])?;
    assert_ne!(code, 0);
    assert!(stderr.contains("--jq-extensions"), "{stderr:?}");
    assert_eq!(std::fs::read(file.path())?, b"a: 4\n");
    // With the flag the write goes through.
    let (_, stderr, code) = run_yq_files(".a %= 3", &[file.path()], &["-i", "--jq-extensions"])?;
    assert_eq!(code, 0, "{stderr}");
    assert_eq!(std::fs::read(file.path())?, b"a: 1\n");
    Ok(())
}

#[test]
fn the_flag_opts_back_into_them_6981() -> Result<()> {
    for (filter, want) in [
        (".a /= 2", "{\"a\":2.0}\n"),
        (".a %= 3", "{\"a\":1}\n"),
        (".a //= 3", "{\"a\":4}\n"),
        (".b //= 3", "{\"a\":4,\"b\":3}\n"),
    ] {
        let (out, code) = run_yq_stdin(filter, "a: 4\n", &["-o=json", "-I=0", "--jq-extensions"])?;
        assert_eq!((out.as_str(), code), (want, 0), "{filter}");
    }
    Ok(())
}

#[test]
fn yqs_own_operators_and_the_arithmetic_forms_need_no_flag_6981() -> Result<()> {
    for (filter, want) in [
        (".a += 1", "{\"a\":5}\n"),
        (".a -= 1", "{\"a\":3}\n"),
        (".a *= 2", "{\"a\":8}\n"),
        (".a = .a / 2", "{\"a\":2.0}\n"),
        (".a = .a % 3", "{\"a\":1}\n"),
        (".b = .b // 3", "{\"a\":4,\"b\":3}\n"),
    ] {
        let (out, code) = run_yq_stdin(filter, "a: 4\n", &["-o=json", "-I=0"])?;
        assert_eq!((out.as_str(), code), (want, 0), "{filter}");
    }
    Ok(())
}

#[test]
fn jq_mode_always_accepts_them_6981() -> Result<()> {
    for (filter, want) in [
        (".a /= 2", "{\"a\":2}\n"),
        (".a %= 3", "{\"a\":1}\n"),
        (".b //= 3", "{\"a\":4,\"b\":3}\n"),
    ] {
        let (out, code) = run_jq_stdin(filter, "{\"a\":4}", &["-c"])?;
        assert_eq!((out.as_str(), code), (want, 0), "{filter}");
    }
    Ok(())
}
