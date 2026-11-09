//! #6566: `strenv(NAME)` and `env(NAME)` reported a variable whose value is not valid UTF-8 as
//! "not provided", as if it were unset. yq reads its bytes (`strenv(BADVAR)` prints the two bytes
//! `\xff\xfe`, captured from yq v4.53.3); an `OwnedValue::String` cannot hold them, so
//! `succinctly yq` refuses it, and names the cause. The refusal is a recorded divergence. An unset
//! variable is still "not provided" for `env` (#6771: `strenv` answers `""`, as yq does), and the exit status is 1 either way.

use std::ffi::OsStr;
use std::process::{Command, Stdio};

use anyhow::Result;

/// `succinctly yq -n FILTER` with `NAME` set to `value`, as `(stdout, stderr, status)`.
fn run_with_env(filter: &str, value: Option<&OsStr>) -> Result<(String, String, i32)> {
    let mut command = Command::new(env!("CARGO_BIN_EXE_succinctly"));
    command
        .args(["yq", "-n", filter])
        .env_remove("SUCCINCTLY_T6566");
    if let Some(value) = value {
        command.env("SUCCINCTLY_T6566", value);
    }
    let output = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()?;
    Ok((
        String::from_utf8(output.stdout)?,
        String::from_utf8(output.stderr)?,
        output.status.code().unwrap_or(-1),
    ))
}

#[cfg(unix)]
#[test]
fn yq_names_a_non_utf8_environment_value_instead_of_calling_it_unset_6566() -> Result<()> {
    use std::os::unix::ffi::OsStrExt;
    let bad = OsStr::from_bytes(&[0xff, 0xfe]);
    for (filter, builtin) in [
        ("strenv(SUCCINCTLY_T6566)", "strenv()"),
        ("env(SUCCINCTLY_T6566)", "env()"),
    ] {
        let (stdout, stderr, status) = run_with_env(filter, Some(bad))?;
        assert_eq!((stdout.as_str(), status), ("", 1), "{filter}");
        assert!(
            stderr.contains(&format!(
                "value for env variable 'SUCCINCTLY_T6566' is not valid UTF-8 in {builtin}"
            )),
            "{filter}: {stderr}"
        );
        assert!(!stderr.contains("not provided"), "{filter}: {stderr}");
    }
    Ok(())
}

#[test]
fn yq_still_calls_an_unset_environment_variable_not_provided_6566() -> Result<()> {
    // #6771: `strenv` of an unset variable is the empty string in yq, which never checks that it
    // is set; only `env` raises.
    let (stdout, _stderr, status) = run_with_env("strenv(SUCCINCTLY_T6566)", None)?;
    assert_eq!((stdout.as_str(), status), ("\n", 0));
    let (stdout, stderr, status) = run_with_env("env(SUCCINCTLY_T6566)", None)?;
    assert_eq!((stdout.as_str(), status), ("", 1));
    assert!(
        stderr.contains("value for env variable 'SUCCINCTLY_T6566' not provided in env()"),
        "{stderr}"
    );
    let (stdout, _stderr, status) =
        run_with_env("strenv(SUCCINCTLY_T6566)", Some(OsStr::new("ok")))?;
    assert_eq!((stdout.as_str(), status), ("ok\n", 0));
    Ok(())
}
