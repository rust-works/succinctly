//! #6244: a range that ends in an escape written with digits (`\x42`, `\102`), a dash, and a class
//! escape (`\d`, `\w`, `\p{L}`) was refused as `char-class value at end of range`. The scan that reads
//! a class's dashes as Oniguruma does (#6184) took the digits of the escape for items of their own,
//! and so for something that starts a range after the range had ended. The dash after a finished range
//! is a member, and the escape after it a second one. Every row was captured from jq 1.7.1
//! (`/usr/bin/jq -c`) over `"aA1-bB _"`.

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
        // The letter and braced forms read as they did.
        (r"[a-b-\d]", r#"["a","1","-","b"]"#),
        (r"[\x41-\x{42}-\d]", r#"["A","1","-","B"]"#),
    ] {
        let filter = r#""aA1-bB _" | [match($p; "g") | .string]"#;
        let (stdout, stderr, code) = run_jq_full(&["-nc", "--arg", "p", pattern, filter], None)?;
        assert_eq!(
            (stdout.trim_end(), code),
            (expected, 0),
            "{pattern}: stderr {stderr}"
        );
    }
    Ok(())
}
