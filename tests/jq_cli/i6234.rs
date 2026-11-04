//! #6234: inside a character class, `\x` followed by something that is not a hex digit (or a brace)
//! is NUL in Oniguruma, and `\x` followed by one hex digit is that digit's value, as outside a class
//! (#6070). The `regex` crate refuses the bare `\x`, so the pattern was refused where jq answers. Every
//! row was captured from jq 1.7.1 (`/usr/bin/jq -n -r`) over `"ax]g\u0000\u0001A"`.

use anyhow::Result;

use crate::run_jq_full;

#[test]
fn test_a_hex_escape_that_is_not_two_digits_in_a_class_reads_as_jq_does_6234() -> Result<()> {
    for (pattern, expected, code) in [
        (r"[a\x]", "[\"a\",\"\\u0000\"]", 0),
        (r"[\x]", "[\"\\u0000\"]", 0),
        (r"[\xg]", "[\"g\",\"\\u0000\"]", 0),
        (r"[\x1]", "[\"\\u0001\"]", 0),
        (r"[\x1g]", "[\"g\",\"\\u0001\"]", 0),
        (r"[\xz1]", "[\"\\u0000\"]", 0),
        (r"[\x41]", "[\"A\"]", 0),
        (r"[\xA]", "[]", 0),
        (r"[^\x]", "[\"a\",\"x\",\"]\",\"g\",\"\\u0001\",\"A\"]", 0),
        (r"[a-\x]", "", 5),
        (r"[\x-a]", "[\"a\",\"]\",\"\\u0000\",\"\\u0001\",\"A\"]", 0),
        (r"[\x\x]", "[\"\\u0000\"]", 0),
        (r"[x\x]", "[\"x\",\"\\u0000\"]", 0),
        (r"[\xg\x1]", "[\"g\",\"\\u0000\",\"\\u0001\"]", 0),
        (r"[\x{41}]", "[\"A\"]", 0),
        (r"[\x4]", "[]", 0),
        (r"[\x0]", "[\"\\u0000\"]", 0),
        (r"[\x]a", "[]", 0),
        (r"a[\x]+", "[]", 0),
        (r"(?i)[\xg]", "[\"g\",\"\\u0000\"]", 0),
        (r"[[:alpha:]\x]", "[\"a\",\"x\",\"g\",\"\\u0000\",\"A\"]", 0),
        (r"[\w\x]", "[\"a\",\"x\",\"g\",\"\\u0000\",\"A\"]", 0),
    ] {
        let (stdout, _, status) = run_jq_full(
            &[
                "-n",
                "-r",
                "--arg",
                "p",
                pattern,
                r#""ax]g\u0000\u0001A" | [match($p; "g") | .string] | tojson"#,
            ],
            None,
        )?;
        assert_eq!((stdout.trim_end(), status), (expected, code), "{pattern}");
    }
    Ok(())
}
