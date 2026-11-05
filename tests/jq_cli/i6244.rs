//! #6244: a range that ends in an escape written with digits (`\x42`, `\102`), a dash, and a class
//! escape (`\d`, `\w`, `\p{L}`) was refused as `char-class value at end of range`. The scan that reads
//! a class's dashes as Oniguruma does (#6184) took the digits of the escape for items of their own,
//! and so for something that starts a range after the range had ended. The dash after a finished range
//! is a member, and the escape after it a second one. Every row was captured from jq 1.7.1
//! (`/usr/bin/jq -c`) over `"aA1-bB _\u0001\u0002"`.

use anyhow::Result;

use crate::run_jq_full;

#[test]
fn test_a_dash_after_a_range_ending_in_a_digit_escape_is_a_member_6244() -> Result<()> {
    for (pattern, expected) in [
        (r"[\x41-\x42-\d]", r#"["A","1","-","B"]"#),
        (r"[\x41-\x41-\d]", r#"["A","1","-"]"#),
        (r"[a-\x62-\d]", r#"["a","1","-","b"]"#),
        (r"[\x41-\x42-\w]", r#"["a","A","1","-","b","B","_"]"#),
        (r"[\x41-\x42-\p{L}]", r#"["a","A","-","b","B"]"#),
        (r"[\101-\102-\d]", r#"["A","1","-","B"]"#),
        (r"[\x41-\x42-\s]", r#"["A","-","B"," "]"#),
        (
            r"[\x61-\x62-\d\x41-\x42-\d]",
            r#"["a","A","1","-","b","B"]"#,
        ),
        // The control escape's letter is part of it too (rows over `"aA1-bB _\u0001\u0002"`).
        (r"[\cA-\cB-\d]", r#"["1","-","\u0001","\u0002"]"#),
        (r"[\cA-\cB-a]", r#"["a","-","\u0001","\u0002"]"#),
        // One digit, a bare `\0`, a negated class, and a non-set item after the dash.
        (r"[\x4-\x5-\d]", r#"["1","-"]"#),
        (r"[\0-\1-\d]", r#"["1","-","\u0001"]"#),
        (r"[^\x41-\x42-\d]", r#"["a","b"," ","_","\u0001","\u0002"]"#),
        (r"[\x41-\x42-b]", r#"["A","-","b","B"]"#),
        // The letter and braced forms read as they did.
        (r"[a-b-\d]", r#"["a","1","-","b"]"#),
        (r"[\x41-\x{42}-\d]", r#"["A","1","-","B"]"#),
    ] {
        let filter = r#""aA1-bB _\u0001\u0002" | [match($p; "g") | .string]"#;
        let (stdout, stderr, code) = run_jq_full(&["-nc", "--arg", "p", pattern, filter], None)?;
        assert_eq!(
            (stdout.trim_end(), code),
            (expected, 0),
            "{pattern}: stderr {stderr}"
        );
    }
    Ok(())
}

/// A class escape after a dash that ends nothing is still a range end jq refuses, whatever the
/// escape before it is spelled.
#[test]
fn test_a_class_escape_that_ends_a_range_is_still_refused_after_a_digit_escape_6244() -> Result<()>
{
    for pattern in [
        r"[\x41-\d]",
        r"[\x41-\x42-\x43-\d]",
        r"[\101-\d]",
        r"[\1010-\d]",
    ] {
        let filter = r#""aA1-bB _" | [match($p; "g") | .string]"#;
        let (stdout, stderr, code) = run_jq_full(&["-nc", "--arg", "p", pattern, filter], None)?;
        assert_eq!(
            (stdout.trim_end(), code),
            ("", 5),
            "{pattern}: stderr {stderr}"
        );
        assert!(
            stderr.contains("char-class value at end of range"),
            "{pattern}: stderr {stderr}"
        );
    }
    Ok(())
}
