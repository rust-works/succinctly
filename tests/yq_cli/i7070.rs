//! #7070: yq's lexer reads the argument of `env(..)` and `strenv(..)` as the raw text up to the
//! first `)`, so `strenv("NAME")` looks up a variable called `"NAME"`, quotes included (the docs'
//! own example `yq -n 'strenv("MYENV")'` finds none and prints an empty line), `strenv( X )` one
//! called ` X `, and `strenv(a;b)` one called `a;b`. An empty argument is invalid. Every row is
//! `yq -n FILTER` with the variables below set, captured from yq v4.53.3.

use std::process::{Command, Stdio};

use anyhow::Result;

/// The variables each row runs with: the plain one, and names a lexer that took identifiers
/// could not reach.
const VARIABLES: &[(&str, &str)] = &[
    ("SUCCINCTLY_T7070", "pv"),
    ("\"SUCCINCTLY_Q7070\"", "qv"),
    (" SUCCINCTLY_T7070 ", "sp"),
    ("a;b7070", "semi"),
    ("x y7070", "spaced"),
    ("1", "one"),
];

/// (filter, yq's stdout), each with exit status 0 and nothing on stderr.
const OK_ROWS: &[(&str, &str)] = &[
    ("strenv(SUCCINCTLY_T7070)", "pv\n"),
    ("env(SUCCINCTLY_T7070)", "pv\n"),
    // A quoted name is a name with quotes in it.
    ("strenv(\"SUCCINCTLY_Q7070\")", "qv\n"),
    ("env(\"SUCCINCTLY_Q7070\")", "qv\n"),
    ("strenv(\"SUCCINCTLY_T7070\")", "\n"),
    ("strenv('SUCCINCTLY_T7070')", "\n"),
    ("strenv(\"\")", "\n"),
    // Blanks, a `;` and a number are part of the name.
    ("strenv( SUCCINCTLY_T7070 )", "sp\n"),
    ("env( SUCCINCTLY_T7070 )", "sp\n"),
    ("strenv(SUCCINCTLY_T7070 )", "\n"),
    ("strenv(a;b7070)", "semi\n"),
    ("strenv(x y7070)", "spaced\n"),
    ("env(x y7070)", "spaced\n"),
    ("strenv(1)", "one\n"),
    ("env(1)", "one\n"),
    // So is a path.
    ("strenv(.a)", "\n"),
    // The call is an ordinary value in a larger program.
    (".a = strenv(\"SUCCINCTLY_T7070\")", "a: \"\"\n"),
    ("strenv(SUCCINCTLY_T7070) + \"!\"", "pv!\n"),
    (
        "[strenv(SUCCINCTLY_T7070), strenv(SUCCINCTLY_NOPE7070)]",
        "- pv\n- \"\"\n",
    ),
];

fn run(filter: &str) -> Result<(String, String, i32)> {
    let mut command = Command::new(env!("CARGO_BIN_EXE_succinctly"));
    command.arg("yq").arg("-n").arg(filter);
    for (name, value) in VARIABLES {
        command.env(name, value);
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

#[test]
fn env_and_strenv_take_the_raw_text_up_to_the_paren_as_the_name_7070() -> Result<()> {
    for (filter, want) in OK_ROWS {
        let (out, err, code) = run(filter)?;
        assert_eq!(
            (out.as_str(), err.as_str(), code),
            (*want, "", 0),
            "{filter}"
        );
    }
    Ok(())
}

/// `env(NAME)` raises for a variable that is not set, and names it as written, quotes included.
#[test]
fn env_of_a_missing_variable_names_it_as_written_7070() -> Result<()> {
    for (filter, name) in [
        ("env(\"SUCCINCTLY_T7070\")", "\"SUCCINCTLY_T7070\""),
        ("env(SUCCINCTLY_NOPE7070)", "SUCCINCTLY_NOPE7070"),
    ] {
        let (out, err, code) = run(filter)?;
        assert_eq!((out.as_str(), code), ("", 1), "{filter}");
        assert!(
            err.starts_with(&format!(
                "Error: value for env variable '{name}' not provided in env()"
            )),
            "{filter}: {err}"
        );
    }
    Ok(())
}

/// An empty argument is no call at all to yq's lexer.
#[test]
fn an_empty_env_argument_is_refused_7070() -> Result<()> {
    for filter in ["strenv()", "env()"] {
        let (out, _, code) = run(filter)?;
        assert_eq!((out.as_str(), code), ("", 1), "{filter}");
    }
    Ok(())
}

/// jq mode keeps the identifier: `env(VAR)` is succinctly's extension there, and a second
/// argument is jq's `env/2 is not defined` (the checkpoint `env(FOO;BAR)` once pinned for yq).
#[test]
fn jq_mode_env_still_takes_an_identifier_7070() -> Result<()> {
    for filter in ["env(FOO;BAR)", "strenv(FOO;BAR)"] {
        let output = Command::new(env!("CARGO_BIN_EXE_succinctly"))
            .args(["jq", "-n", filter])
            .stdin(Stdio::null())
            .output()?;
        let stderr = String::from_utf8(output.stderr)?;
        assert_eq!(output.status.code(), Some(3), "{filter}: {stderr}");
        let name = filter.split('(').next().unwrap_or(filter);
        assert!(
            stderr.contains(&format!("{name}/2 is not defined")),
            "{filter}: {stderr}"
        );
    }
    Ok(())
}
