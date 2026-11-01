//! #5721: a NUL ends a JSON input (a lone NUL is the empty document), and an explicit `-p=json`
//! makes `-o auto` YAML. Expected output is yq v4.53.3's.

use anyhow::Result;

use crate::{run_yq_stdin, run_yq_stdin_bytes_with_stderr};

fn json_nul(input: &[u8], extra: &[&str]) -> Result<(String, i32)> {
    let mut args = vec!["-p=json", "-o=json", "-I0"];
    args.extend_from_slice(extra);
    let (out, _, code) = run_yq_stdin_bytes_with_stderr(".", input, &args)?;
    Ok((out, code))
}

#[test]
fn a_nul_ends_a_json_input() -> Result<()> {
    for (input, expected) in [
        (&b"\0"[..], "null\n"),
        (b"\0\n", "null\n"),
        (b" \0 ", "null\n"),
        (b"\0\0", "null\n"),
        (b"\0 1", "null\n"),
        (b"\0{\"a\":1}", "null\n"),
        (b"1\0", "1\n"),
        (b"1\0\n2", "1\n"),
        (b"{\"a\":1}\0", "{\"a\":1}\n"),
    ] {
        assert_eq!(
            json_nul(input, &[])?,
            (expected.to_string(), 0),
            "{input:?}"
        );
    }
    Ok(())
}

#[test]
fn a_lone_nul_is_the_empty_document_for_yaml_output() -> Result<()> {
    let (out, _, code) = run_yq_stdin_bytes_with_stderr(".", b"\0", &["-p=json"])?;
    assert_eq!((out.as_str(), code), ("\n", 0));
    Ok(())
}

#[test]
fn explicit_json_input_makes_output_auto_yaml() -> Result<()> {
    let (out, code) = run_yq_stdin(".", "{\"a\":1}", &["-p=json", "-o", "auto"])?;
    assert_eq!((out.as_str(), code), ("a: 1\n", 0));
    let (out, code) = run_yq_stdin(".", "{\"a\":1}", &["-p=json", "-o", "auto", "-I0"])?;
    assert_eq!((out.as_str(), code), ("a: 1\n", 0));
    Ok(())
}
