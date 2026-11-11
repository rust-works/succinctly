//! #6981: yq v4.53.3 has no `/=`, `%=` or `//=` operator (it reads `/`, `%` or `//` with a missing
//! operand and stops: `'/' expects 2 args but there is 1`), so `succinctly yq` refuses them unless
//! `--jq-extensions` is passed, as it does a jq-only builtin. `+=`, `-=`, `*=` and `|=` are yq's
//! own and need no flag; `succinctly jq` always accepts all of them.
//!
//! The refusals are captured from yq v4.53.3 (`a: 4`); the answers with the flag are this tool's
//! own, since there is no yq answer to match.

use anyhow::Result;

use crate::{run_jq_stdin, run_yq_stdin, run_yq_stdin_with_stderr};

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
