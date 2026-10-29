//! #5491: an exponent past decNumber's limit (1e9 or more) prints as `DBL_MAX`'s `e+308` on
//! every input route, as it does for jq. The input bridge used to write that display text into
//! the document it re-indexes, which read back as an ordinary literal and printed `E+308`. Every
//! expected text was captured from jq 1.7.1 (`/usr/bin/jq`).

use anyhow::Result;

use crate::run_jq_stdin;

const MAX: &str = "1.7976931348623157e+308";

#[test]
fn test_stream_event_with_an_overflowing_exponent_prints_lowercase_e_5491() -> Result<()> {
    for (literal, want) in [
        ("1e9999999999", format!("[[0],{MAX}]\n[[0]]\n")),
        ("-1e9999999999", format!("[[0],-{MAX}]\n[[0]]\n")),
        // The exponent's own digit count is what matters, not the mantissa's.
        (
            "1.5e999999999999999999999999",
            format!("[[0],{MAX}]\n[[0]]\n"),
        ),
        // The last exponent decNumber keeps, and a plain overflow: both keep jq's own spelling.
        ("1e999999999", "[[0],1E+999999999]\n[[0]]\n".to_string()),
        ("1e400", "[[0],1E+400]\n[[0]]\n".to_string()),
    ] {
        let input = format!("[{literal}]");
        assert_eq!(
            run_jq_stdin(".", &input, &["--stream", "-c"])?,
            (want, 0),
            "{literal}"
        );
    }
    Ok(())
}

#[test]
fn test_stream_event_value_keeps_its_overflowing_exponent_5491() -> Result<()> {
    // The value the program sees, not only what the writer prints. The second event is `[[0]]`.
    assert_eq!(
        run_jq_stdin(".[1] | tojson", "[1e9999999999]", &["--stream"])?,
        (format!("\"{MAX}\"\n\"null\"\n"), 0)
    );
    Ok(())
}

#[test]
fn test_stream_events_read_with_inputs_agree_with_the_lazy_route_5491() -> Result<()> {
    assert_eq!(
        run_jq_stdin("[inputs]", "[1e9999999999]", &["--stream", "-c", "-n"])?,
        (format!("[[[0],{MAX}],[[0]]]\n"), 0)
    );
    Ok(())
}

#[test]
fn test_seq_slurp_with_an_overflowing_exponent_prints_lowercase_e_5491() -> Result<()> {
    // `--seq` frames each value with an RS byte (0x1e), and so does its output.
    assert_eq!(
        run_jq_stdin(
            ".",
            "\x1e[1e9999999999,-1e9999999999]\n",
            &["--seq", "-s", "-c"]
        )?,
        (format!("\x1e[[{MAX},-{MAX}]]\n"), 0)
    );
    Ok(())
}
