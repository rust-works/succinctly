//! #6111: `strftime(f)` and `strflocaltime(f)` with a format that is not a string raise the error
//! jq 1.7.1 does for the input first (`f_strftime` tests the input's kind, then the format). The
//! wording of each case was captured from `/usr/bin/jq` 1.7.1.

use anyhow::Result;

use crate::run_jq_stdin;

fn caught(filter: &str, input: &str) -> Result<String> {
    let (out, code) = run_jq_stdin(&format!("try ({filter}) catch ."), input, &["-c"])?;
    assert_eq!(code, 0, "{filter} << {input}");
    Ok(out.trim_end().to_string())
}

#[test]
fn a_non_datetime_input_is_reported_before_the_format() -> Result<()> {
    for name in ["strftime", "strflocaltime"] {
        for input in ["null", r#""x""#, "{}", "true"] {
            for fmt in ["1", "null", "[]"] {
                assert_eq!(
                    caught(&format!("{name}({fmt})"), input)?,
                    format!(r#""{name}/1 requires parsed datetime inputs""#),
                    "{name}({fmt}) << {input}"
                );
            }
        }
    }
    Ok(())
}

#[test]
fn an_array_input_reaches_the_format_check() -> Result<()> {
    for name in ["strftime", "strflocaltime"] {
        for input in ["[]", "[1]", "[2000,0,1,0,0,0,0,0]"] {
            for fmt in ["1", "null", "{}"] {
                assert_eq!(
                    caught(&format!("{name}({fmt})"), input)?,
                    format!(r#""{name}/1 requires a string format""#),
                    "{name}({fmt}) << {input}"
                );
            }
        }
    }
    Ok(())
}

#[test]
fn a_number_that_does_not_convert_reports_the_conversion() -> Result<()> {
    for input in ["1e1000", "1e18", "-1e18"] {
        assert_eq!(
            caught("strftime(1)", input)?,
            r#""error converting number of seconds since epoch to datetime""#,
            "strftime(1) << {input}"
        );
    }
    Ok(())
}

/// jq aborts on an assertion here (#3046), so there is no wording to match: this build raises.
#[test]
fn a_number_that_converts_keeps_the_format_error() -> Result<()> {
    assert_eq!(
        caught("strftime(1)", "0")?,
        r#""expected string, got strftime format""#
    );
    assert_eq!(
        caught("strflocaltime([])", "0")?,
        r#""expected string, got strflocaltime format""#
    );
    Ok(())
}

/// jq has no wording for a number that fails `localtime` either (it aborts on `jv_array_get`), so
/// the local-zone builtin raises the same conversion error `strftime` does.
#[test]
fn strflocaltime_reports_the_conversion_of_a_number_that_fails() -> Result<()> {
    for fmt in ["1", r#""%Y""#] {
        assert_eq!(
            caught(&format!("strflocaltime({fmt})"), "1e1000")?,
            r#""error converting number of seconds since epoch to datetime""#,
        );
    }
    Ok(())
}

/// Each format of a fan-out is tested on its own, and `?` and `try` swallow the fault.
#[test]
fn a_fan_out_of_formats_and_the_optional_forms() -> Result<()> {
    assert_eq!(
        caught("[strftime(1, null)?]", "[]")?,
        "[]",
        "`?` swallows the first fault"
    );
    let (out, code) = run_jq_stdin("[.[] | try strftime(1) catch .]", "[null,[],{}]", &["-c"])?;
    assert_eq!(code, 0);
    assert_eq!(
        out.trim_end(),
        r#"["strftime/1 requires parsed datetime inputs","strftime/1 requires a string format","strftime/1 requires parsed datetime inputs"]"#
    );
    Ok(())
}
