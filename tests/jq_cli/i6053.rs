//! #6053: in jq `.` followed by a string indexes by that string, and the string may hold `\(...)`
//! interpolation: `."a\(1)"` is `.["a1"]`. A string with no interpolation was already a field name.
//! The key is read from the term's input, as a bracket's is, and the step takes `?`, `=`, `|=`, `del`
//! and `path`. Every expectation is what `/usr/bin/jq` 1.7.1 printed for the same program.

use anyhow::Result;

use crate::run_jq_full;

#[test]
fn an_interpolated_string_after_a_dot_indexes_by_the_computed_key() -> Result<()> {
    let rows: &[(&str, &str)] = &[
        (r#"{"a1":5} | ."a\(1)""#, r"5"),
        (r#"{"a1":5} | ."a\(1)"?"#, r"5"),
        (r#"{"a1":{"b0":2}} | ."a\(1)"."b\(0)""#, r"2"),
        (r#"{"x":{"y":2}} | .x."\("y")""#, r"2"),
        (r#"{"k":"y","x":{"y":7}} | .x."\(.k)"?"#, r"7"),
        (r#"{"a1":1} | ."a\(1)" = 2"#, r#"{"a1":2}"#),
        (r#"{"a1":1} | ."a\(1)" |= . + 1"#, r#"{"a1":2}"#),
        (r#"{"a1":1} | del(."a\(1)")"#, r"{}"),
        (r#"{"a1":1} | path(."a\(1)")"#, r#"["a1"]"#),
        (r#"{"a1":[1,2]} | ."a\(1)"[1]"#, r"2"),
        (r#"{"a1":{"b":3}} | ."a\(1)".b"#, r"3"),
        (r#""x" | [."\(0)"?]"#, r"[]"),
        (r#"{"a":1} | [."a", ."\("a")"]"#, r"[1,1]"),
        (r#"{"a b":1} | ."a\(" ")b""#, r"1"),
        (r#"null | ."a\(1)""#, r"null"),
        (r#"{"a":1} | ."a" "#, r"1"),
    ];
    for (program, want) in rows {
        let (stdout, stderr, code) = run_jq_full(&["-nc", program], None)?;
        assert_eq!(code, 0, "`{program}` failed: {stderr}");
        assert_eq!(stdout.trim_end(), *want, "`{program}`");
    }
    Ok(())
}

#[test]
fn plain_quoted_steps_and_generator_keys_read_as_before() -> Result<()> {
    let rows: &[(&str, &str)] = &[
        (r#"{"a\tb":1} | ."a\tb""#, r"1"),
        (r#"{"é":2} | ."é""#, r"2"),
        (r#"{"":3} | ."""#, r"3"),
        (r#"{"a\"b":4} | ."a\"b""#, r"4"),
        (r#"{"a\\b":5} | ."a\\b""#, r"5"),
        (r#"{"a1":1,"a2":2} | [."a\(1,2)"]"#, r"[1,2]"),
        (r#"{"a3":5} | ."a\(1+2)" as $v | $v + 1"#, r"6"),
        (r#"{"a":{"b":1}} | [."a"."b", ."a"."b"?]"#, r"[1,1]"),
    ];
    for (program, want) in rows {
        let (stdout, stderr, code) = run_jq_full(&["-nc", program], None)?;
        assert_eq!(code, 0, "`{program}` failed: {stderr}");
        assert_eq!(stdout.trim_end(), *want, "`{program}`");
    }
    Ok(())
}
