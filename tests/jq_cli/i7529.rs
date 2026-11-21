//! #7529: `toboolean`, `trim`, `ltrim` and `rtrim` are jq 1.8 builtins (jq 1.7.1 has none of them), so jq 1.8.2
//! (`/opt/homebrew/bin/jq`) is the only reference; every expected line below was captured from it.
//!
//! - `toboolean`'s refusal names the value with JSON escapes (`\u0001`, `\u007f`, `\u0000`), not Rust's
//!   (`\u{1}`, `\0`), and names it for every type (`null (null)`, `number (0)`, `array ([1])`).
//! - `trim`, `ltrim` and `rtrim` all say `trim input must be a string`.

use anyhow::Result;

use crate::run_jq_full;

#[test]
fn toboolean_refusal_names_the_value_as_jq_1_8_does_7529() -> Result<()> {
    let program = r#"[null,0,"tru",1.5,[],{},[1],{"a":1},true,false,"true","false","\u0001","a\nb","é\t","\u007f","x\u0000","\u001f","\"q\\","\u2028","\u0000"] | .[] | try toboolean catch ."#;
    let (out, err, code) = run_jq_full(&["-nc", program], None)?;
    assert_eq!((err.as_str(), code), ("", 0));
    let want = [
        r#""null (null) cannot be parsed as a boolean""#,
        r#""number (0) cannot be parsed as a boolean""#,
        r#""string (\"tru\") cannot be parsed as a boolean""#,
        r#""number (1.5) cannot be parsed as a boolean""#,
        r#""array ([]) cannot be parsed as a boolean""#,
        r#""object ({}) cannot be parsed as a boolean""#,
        r#""array ([1]) cannot be parsed as a boolean""#,
        r#""object ({\"a\":1}) cannot be parsed as a boolean""#,
        "true",
        "false",
        "true",
        "false",
        r#""string (\"\\u0001\") cannot be parsed as a boolean""#,
        r#""string (\"a\\nb\") cannot be parsed as a boolean""#,
        r#""string (\"é\\t\") cannot be parsed as a boolean""#,
        r#""string (\"\\u007f\") cannot be parsed as a boolean""#,
        r#""string (\"x\\u0000\") cannot be parsed as a boolean""#,
        r#""string (\"\\u001f\") cannot be parsed as a boolean""#,
        r#""string (\"\\\"q\\\\\") cannot be parsed as a boolean""#,
        "\"string (\\\"\u{2028}\\\") cannot be parsed as a boolean\"",
        r#""string (\"\\u0000\") cannot be parsed as a boolean""#,
    ]
    .join("\n")
        + "\n";
    assert_eq!(out, want);
    Ok(())
}

#[test]
fn an_uncaught_toboolean_refusal_names_the_value_7529() -> Result<()> {
    let (out, err, code) = run_jq_full(&["-n", "[1] | toboolean"], None)?;
    assert_eq!(
        (out.as_str(), err.as_str(), code),
        (
            "",
            "jq: error (at <unknown>): array ([1]) cannot be parsed as a boolean\n",
            5
        )
    );
    // `?` still suppresses it.
    let (out, _, code) = run_jq_full(&["-nc", r#"[1,null,"x"] | map(toboolean?)"#], None)?;
    assert_eq!((out.as_str(), code), ("[]\n", 0));
    Ok(())
}

#[test]
fn trim_ltrim_and_rtrim_share_one_message_7529() -> Result<()> {
    let program =
        r"123, null, [1], {}, true | (try trim catch .), (try ltrim catch .), (try rtrim catch .)";
    let (out, err, code) = run_jq_full(&["-nc", program], None)?;
    assert_eq!((err.as_str(), code), ("", 0));
    assert_eq!(out, "\"trim input must be a string\"\n".repeat(15));
    for name in ["trim", "ltrim", "rtrim"] {
        let (out, err, code) = run_jq_full(&["-n", &format!("123 | {name}")], None)?;
        assert_eq!(
            (out.as_str(), err.as_str(), code),
            (
                "",
                "jq: error (at <unknown>): trim input must be a string\n",
                5
            ),
            "{name}"
        );
    }
    Ok(())
}
