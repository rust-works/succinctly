//! #6363: yq reads the string of `from_yaml` / `from_json` through the YAML decoder, whose header
//! slurp applies to it as to a file: the line break after a leading `--- text` is lost (#6276) and a
//! run of leading `---` lines is one document (#6349). Every row was captured from yq v4.53.3
//! (`yq -n -o=json -I0 '"<text>" | from_yaml'`).

use anyhow::Result;

use crate::run_yq_stdin;

fn decode(builtin: &str, text: &str) -> Result<(String, i32)> {
    let quoted = format!("{text:?}");
    run_yq_stdin(
        &format!("{quoted} | {builtin}"),
        "",
        &["-n", "-o=json", "-I=0"],
    )
}

#[test]
fn test_from_yaml_applies_the_header_slurp_to_its_string_6363() -> Result<()> {
    for (text, expected) in [
        ("---\n---\nx", "\"x\"\n"),
        ("---\n---\n", "null\n"),
        ("---\n# c\n---\nx", "\"x\"\n"),
        ("%YAML 1.2\n---\n---\nx", "\"x\"\n"),
        ("--- x\ny", "\"xy\"\n"),
        ("--- x\ny\nz", "\"xy z\"\n"),
        ("# c\n--- x\ny", "\"xy\"\n"),
        ("  --- x\ny", "\"xy\"\n"),
        ("---\n--- x\ny", "\"xy\"\n"),
        ("--- --- x\ny\nz", "\"xyz\"\n"),
        ("--- # c\nx\ny", "\"y\"\n"),
        // Not a leading run, or a property between the markers: as before.
        ("a: 1\n---\n---\nb: 2", "{\"a\":1}\n"),
        ("---\n&a\n---\nx", "null\n"),
        ("a: 1", "{\"a\":1}\n"),
        // Only the first document is read, and a string that is no header reads as before.
        ("a: 1\n---\nb: 2", "{\"a\":1}\n"),
        ("a: &x 1\nb: *x", "{\"a\":1,\"b\":1}\n"),
        ("- &a [1]\n- *a", "[[1],[1]]\n"),
        ("x\ny", "\"x y\"\n"),
    ] {
        for builtin in ["from_yaml", "from_json"] {
            let (stdout, code) = decode(builtin, text)?;
            assert_eq!((stdout.as_str(), code), (expected, 0), "{builtin} {text:?}");
        }
    }
    Ok(())
}

#[test]
fn test_from_yaml_refuses_what_the_glue_makes_invalid_6363() -> Result<()> {
    for text in ["--- |  x\n  y", "--- a: 1\nb: 2"] {
        let (stdout, code) = decode("from_yaml", text)?;
        assert_ne!(code, 0, "{text:?} should be refused, got {stdout:?}");
    }
    Ok(())
}
