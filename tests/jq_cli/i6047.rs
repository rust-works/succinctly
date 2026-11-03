//! #6047: with `-a` and `-r`, `-j` or `--raw-output0`, jq writes a top-level string quoted and escaped
//! and drops `-C` for it; a string inside a container and a non-string stay colored. Every expectation
//! was captured from jq 1.7.1 (`/usr/bin/jq`, macOS).

use anyhow::Result;

use crate::{run_jq_full, run_jq_null, run_jq_stdin};

const GREEN: &str = "\u{1b}[0;32m";
const RESET: &str = "\u{1b}[0m";

fn colored(text: &str) -> String {
    format!("{GREEN}{text}{RESET}")
}

#[test]
fn a_top_level_string_is_quoted_and_uncolored_under_a_raw_mode() -> Result<()> {
    for (mode, end) in [("-r", "\n"), ("-j", ""), ("--raw-output0", "\0")] {
        for (input, want) in [(r#""é""#, r#""\u00e9""#), (r#""x""#, r#""x""#)] {
            let (out, code) = run_jq_stdin(".", input, &["-C", "-a", mode])?;
            assert_eq!(code, 0, "{mode} {input}");
            assert_eq!(out, format!("{want}{end}"), "{mode} {input}");
        }
    }
    Ok(())
}

#[test]
fn a_string_built_by_the_program_is_uncolored_too() -> Result<()> {
    let (out, code) = run_jq_null(r#""é""#, &["-C", "-a", "-j"])?;
    assert_eq!(code, 0);
    assert_eq!(out, "\"\\u00e9\"");
    Ok(())
}

#[test]
fn a_string_read_off_a_document_is_uncolored_too() -> Result<()> {
    let (out, code) = run_jq_stdin(".k", r#"{"k":"é"}"#, &["-C", "-a", "-r"])?;
    assert_eq!(code, 0);
    assert_eq!(out, "\"\\u00e9\"\n");
    Ok(())
}

#[test]
fn a_string_in_a_container_and_a_non_string_stay_colored() -> Result<()> {
    let (out, code) = run_jq_stdin(".a", r#"{"a":["é"]}"#, &["-C", "-a", "-r", "-c"])?;
    assert_eq!(code, 0);
    assert!(
        out.contains(&colored(r#""\u00e9""#)),
        "a string in an array keeps its color: {out:?}"
    );
    let (out, code) = run_jq_stdin(".n", r#"{"n":1}"#, &["-C", "-a", "-r"])?;
    assert_eq!(code, 0);
    assert_eq!(out, "\u{1b}[0;39m1\u{1b}[0m\n");
    Ok(())
}

#[test]
fn the_neighbouring_flag_pairs_are_unchanged() -> Result<()> {
    // -C -a alone and -C -r alone: colored / raw respectively.
    let (out, _) = run_jq_stdin(".", r#""é""#, &["-C", "-a"])?;
    assert_eq!(out, format!("{}\n", colored(r#""\u00e9""#)));
    let (out, _) = run_jq_stdin(".", r#""é""#, &["-C", "-r"])?;
    assert_eq!(out, "é\n");
    // -a -r without -C: quoted and escaped, as before (#2662).
    let (out, _) = run_jq_stdin(".", r#""é""#, &["-a", "-r"])?;
    assert_eq!(out, "\"\\u00e9\"\n");
    Ok(())
}

#[test]
fn a_string_read_from_a_file_is_uncolored_too() -> Result<()> {
    let mut file = tempfile::NamedTempFile::new()?;
    std::io::Write::write_all(&mut file, "\"é\"\n".as_bytes())?;
    let path = file.path().to_string_lossy().into_owned();
    for (mode, end) in [("-r", "\n"), ("-j", ""), ("--raw-output0", "\0")] {
        let (out, _, code) = run_jq_full(&["-C", "-a", mode, ".", &path], None)?;
        assert_eq!(code, 0, "{mode}");
        assert_eq!(out, format!("\"\\u00e9\"{end}"), "{mode}");
    }
    Ok(())
}

#[test]
fn seq_and_a_nul_agree_with_jq() -> Result<()> {
    // `-a` escapes the NUL, so `--raw-output0` has nothing to refuse and writes one record.
    let (out, code) = run_jq_null(r#""é\u0000""#, &["-C", "-a", "--raw-output0"])?;
    assert_eq!(code, 0);
    assert_eq!(out, "\"\\u00e9\\u0000\"\0");
    // Without `-a` the NUL is refused, as before.
    let (_, code) = run_jq_null(r#""é\u0000""#, &["-C", "--raw-output0"])?;
    assert_eq!(code, 5);
    // `--seq` writes no RS before a raw-shaped string, colored or not.
    let (out, code) = run_jq_null(r#""é""#, &["--seq", "-C", "-a", "-r"])?;
    assert_eq!(code, 0);
    assert_eq!(out, "\"\\u00e9\"\n");
    Ok(())
}
