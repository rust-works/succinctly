//! #6051: a `-0` expression literal is the integer zero in yq. It keeps its text for `tostring`
//! and YAML output (`-0`), but yq's JSON encoder prints `0`, and so does `path` through a
//! variable, which printed `[-0]` here. A YAML document's `-0` is a float and still prints
//! `-0.0`. Every row is from yq v4.53.3.

use anyhow::Result;

use crate::run_yq_stdin;

fn json(filter: &str, input: &str) -> Result<String> {
    let (stdout, code) = run_yq_stdin(filter, input, &["-o=json", "-I0"])?;
    assert_eq!(code, 0, "`{filter}` failed");
    Ok(stdout)
}

#[test]
fn a_negative_zero_literal_prints_zero_in_json() -> Result<()> {
    for (filter, want) in [
        ("(-0)", "0\n"),
        ("[-0]", "[0]\n"),
        (r#"{"a": -0}"#, "{\"a\":0}\n"),
        ("(-0) | to_json(0)", "\"0\"\n"),
        ("(-0) | @json", "\"0\"\n"),
        (".a |= -0", "{\"a\":0}\n"),
    ] {
        assert_eq!(json(filter, "a: 1\n")?, want, "`{filter}`");
    }
    Ok(())
}

#[test]
fn a_negative_zero_variable_index_has_an_integer_path() -> Result<()> {
    let got = json(". as $d | -0 | . as $i | $d | .[$i] | path", "[1, 2, 3]\n")?;
    assert_eq!(got, "[0]\n");
    Ok(())
}

#[test]
fn the_text_of_a_negative_zero_literal_is_kept_outside_json() -> Result<()> {
    let (stdout, _) = run_yq_stdin("(-0) | tostring", "a: 1\n", &["-I0"])?;
    assert_eq!(stdout, "-0\n");
    let (stdout, _) = run_yq_stdin("(-0)", "a: 1\n", &["-I0"])?;
    assert_eq!(stdout, "-0\n");
    Ok(())
}

#[test]
fn a_negative_zero_in_a_document_is_still_a_float() -> Result<()> {
    assert_eq!(json(".", "a: -0\n")?, "{\"a\":-0.0}\n");
    assert_eq!(json(".a", "a: -0\n")?, "-0.0\n");
    Ok(())
}

#[test]
fn a_negative_zero_read_from_json_prints_zero() -> Result<()> {
    let (stdout, code) = run_yq_stdin(".", "[-0, 1]\n", &["-p=json", "-o=json", "-I0"])?;
    assert_eq!(code, 0);
    assert_eq!(stdout, "[0,1]\n");
    Ok(())
}
