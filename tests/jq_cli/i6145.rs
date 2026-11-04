//! #6145: under `-s` the whole stream was read to build the one value, so a program's `input` past
//! it reads off the end of a stream jq has already run off, and `input_line_number` answers 0 from
//! then on. Without `-s` the first read past the end keeps the last line (#4303). Every row was
//! captured from jq 1.7.1 (`/usr/bin/jq`).

use anyhow::Result;

use crate::run_jq_stdin;

const PAST_END_TWICE: &str = "[input_line_number, (try input catch .), input_line_number, (try input catch .), input_line_number]";

#[test]
fn a_read_past_a_slurped_value_resets_input_line_number_6145() -> Result<()> {
    for (flags, input, expected) in [
        (&["-c", "-s"][..], "1\n2\n3\n", r#"[3,"break",0,"break",0]"#),
        (&["-c", "-s"], "1\n2\n3", r#"[2,"break",0,"break",0]"#),
        (&["-c", "-s"], "\n", r#"[1,"break",0,"break",0]"#),
        (&["-c", "-s"], "1\n\n\n", r#"[3,"break",0,"break",0]"#),
        (&["-c", "-s"], "[1,\n2]\n", r#"[2,"break",0,"break",0]"#),
        (&["-c", "-s"], "1 2 3", r#"[0,"break",0,"break",0]"#),
        (
            &["-c", "-s", "--seq"],
            "\x1e1\n\x1e2\n\x1e3\n",
            "\x1e[3,\"break\",0,\"break\",0]",
        ),
        (&["-c", "-s", "-R"], "a\nb\n", r#"[2,"break",0,"break",0]"#),
        (&["-c", "-s"], "", r#"[0,"break",0,"break",0]"#),
        (
            &["-c", "-s", "--stream"],
            "[1]\n2\n",
            r#"[2,"break",0,"break",0]"#,
        ),
    ] {
        let (out, code) = run_jq_stdin(PAST_END_TWICE, input, flags)?;
        assert_eq!(code, 0, "{flags:?} {input:?}");
        assert_eq!(out.trim_end(), expected, "{flags:?} {input:?}");
    }
    Ok(())
}

/// `-n` reads the slurped array with `input` or `inputs`, so the line is the stream's end until the
/// read after it.
#[test]
fn under_null_input_the_read_after_the_slurp_resets_it_6145() -> Result<()> {
    for (filter, input, expected) in [
        ("[inputs] | [input_line_number]", "1\n2\n3\n", "[0]"),
        ("[inputs] | [input_line_number]", "[1,\n2]\n", "[0]"),
        (
            "[input_line_number, input, input_line_number]",
            "[1,\n2]\n",
            "[0,[[1,2]],2]",
        ),
    ] {
        let (out, code) = run_jq_stdin(filter, input, &["-c", "-n", "-s"])?;
        assert_eq!(code, 0, "{filter} {input:?}");
        assert_eq!(out.trim_end(), expected, "{filter} {input:?}");
    }
    Ok(())
}
