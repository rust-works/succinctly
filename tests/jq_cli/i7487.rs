//! #7487: an environment entry with an empty name (`=v`) is `"": "v"` in jq's `$ENV` and `env`, which split
//! each `environ` string at its first `=`. `std::env::vars_os` skipped it, and read `=X=1` as `X`.
//! Every row was captured from jq 1.7.1 (`/usr/bin/jq`) under the same environment, and jq 1.8.2 agrees;
//! the entries are made with `Command::env("", "v")`, which Unix passes on as `=v`. Each program
//! drops the one entry an instrumented (`cargo llvm-cov`) binary adds to itself (see `OWN`).

#![cfg(unix)]

use std::process::Command;

use anyhow::Result;

use crate::succinctly_bin;

/// The whole environment as (name, value) pairs.
type Environment = &'static [(&'static str, &'static str)];

/// Drops the entries a `cargo llvm-cov` binary adds to its own environment: the profile runtime
/// calls `setenv("__LLVM_PROFILE_RT_INIT_ONCE", ...)` when it starts, after `env_clear`, so under
/// `cfg(coverage)` the child's `$ENV` holds one entry the test did not set. With no such entry
/// (jq, a plain build) `own` is the identity, so each row is still jq's output for the bare
/// program: the filtered programs were run through jq 1.7.1 and 1.8.2 under the same rows.
const OWN: &str = "def own: with_entries(select(.key | startswith(\"__LLVM_PROFILE_\") | not)); ";

/// (the whole environment, `$ENV`, `env | keys`).
const ROWS: &[(Environment, &str, &str)] = &[
    (
        &[("", "v"), ("Y", "1")],
        "{\"\":\"v\",\"Y\":\"1\"}",
        "[\"\",\"Y\"]",
    ),
    (
        &[("=X", "1"), ("Y", "1")],
        "{\"\":\"X=1\",\"Y\":\"1\"}",
        "[\"\",\"Y\"]",
    ),
    (&[("", "a=b")], "{\"\":\"a=b\"}", "[\"\"]"),
    (
        &[("", ""), ("A", "1")],
        "{\"\":\"\",\"A\":\"1\"}",
        "[\"\",\"A\"]",
    ),
    (&[("Y", "1")], "{\"Y\":\"1\"}", "[\"Y\"]"),
];

#[test]
fn an_environment_entry_with_an_empty_name_is_listed_as_jq_lists_it_7487() -> Result<()> {
    for &(environment, all, keys) in ROWS {
        for (program, want) in [
            ("$ENV | own", all),
            ("env | own | keys", keys),
            ("env == $ENV", "true"),
        ] {
            let program = format!("{OWN}{program}");
            let mut command = Command::new(succinctly_bin());
            command.env_clear().args(["jq", "-n", "-c", &program]);
            for (name, value) in environment {
                command.env(name, value);
            }
            let output = command.output()?;
            assert_eq!(
                (
                    String::from_utf8(output.stdout)?.as_str(),
                    String::from_utf8(output.stderr)?.as_str(),
                    output.status.code()
                ),
                (&format!("{want}\n")[..], "", Some(0)),
                "{environment:?}: {program}"
            );
        }
    }
    Ok(())
}
