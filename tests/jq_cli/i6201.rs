//! #6201: a look-behind with alternatives of different widths is refused by Oniguruma in the Perl
//! syntax jq uses (`invalid pattern in look-behind`): `(?<=a|bc)d`, `(?<!a(?:b|cd))e`,
//! `(?<=(?:a|bc){2})d`. The `regex` crate answered them. An alternation of one width (`ab|cd`), one
//! with a branch written as empty (`a|`), and a look-behind whose body repeats a varying number of
//! times (`(?<=a?)`) are accepted, as in jq. Rows are live jq 1.7.1 captures: the offsets and
//! lengths of every match in `abcd`, or `None` for an error (exit 5; the message is the engine's
//! own).

use anyhow::Result;

use crate::run_jq_full;

const TEXT: &str = r#""abcd""#;

const ROWS: &[(&str, Option<&str>)] = &[
    (r"(?<=a|bc)d", None),
    (r"(?<!a|bc)d", None),
    (r"(?<=(?:a|bc)d)e", None),
    (r"(?<!a(?:b|(?:c(?:d))))e", None),
    (r"(?<=ab|cd)e", Some("[]")),
    (r"(?<=a|b)c", Some("[[2,1]]")),
    (r"(?<!a|b)c", Some("[]")),
    (r"(?<=(?:a|b){2})d", Some("[]")),
    (r"(?<=(?:a|bc){2})d", None),
    (r"(?<=x(?:b|cd))e", None),
    (r"(?<=\b|a)b", None),
    (r"(?<=(?:.|\b))c", None),
    (r"(?<=a|)c", Some("[[2,1]]")),
    (r"(?<=(?:a|bc)?)d", Some("[[3,1]]")),
    (r"(?<=a?)c", Some("[[2,1]]")),
    (r"(?<=(?:a|bc)|)d", Some("[[3,1]]")),
    (r"(?<=a(?:b|cd))e", None),
    (r"(?<=(?i:a)|bc)d", None),
    (r"(?<=(?<=a)b|cd)e", None),
    (r"(?<=\d|ab)c", None),
    (r"(?<=[ab]|cd)e", None),
    (r"(?<=a{2}|bc)d", Some("[[3,1]]")),
    (r"(?<=(?:ab){2}|cdc)e", None),
    (r"(?<=a)b|(?<=bc)d", Some("[[1,1],[3,1]]")),
    (r"x|(?<=a|bc)d", None),
    (r"(?<=(a)|bc)d", None),
    (r"(?<=\p{L}|bc)d", None),
    (r"(?<=.|bc)d", None),
    (r"(?<=(?:a|bc)+)d", None),
    (r"(?<=(?:a|bc){1,2})d", None),
    (r"(?<=(?:a|bc){2,})d", None),
    (r"(?<=(?:a|bc)+?)d", None),
    (r"x(?<=ab(?:a|bc)+)d", None),
    (r"(?<=(?:a|bc)*)d", Some("[[3,1]]")),
    (r"(?<=(?:a|bc)?)d", Some("[[3,1]]")),
    (r"(?<=(?:(?:a|bc)+)?)d", Some("[[3,1]]")),
    (r"(?<=(?:a|bc){0,2})d", Some("[[3,1]]")),
    (r"(?<=(?:(?:a|bc)+)*)d", Some("[[3,1]]")),
];

#[test]
fn test_a_look_behind_with_alternatives_of_different_widths_is_refused_6201() -> Result<()> {
    for (pattern, want) in ROWS {
        let args = [
            "-c",
            "--arg",
            "p",
            pattern,
            r#"[match($p;"g")|[.offset,.length]]"#,
        ];
        let (stdout, stderr, code) = run_jq_full(&args, Some(TEXT))?;
        match want {
            Some(want) => assert_eq!((stdout.trim_end(), code), (*want, 0), "{pattern}: {stderr}"),
            None => {
                assert_eq!((stdout.as_str(), code), ("", 5), "{pattern}: {stderr}");
                assert!(stderr.contains("invalid regex"), "{pattern}: {stderr}");
            }
        }
    }
    Ok(())
}
