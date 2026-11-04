//! #6212: an inline option switch inside a capture group, plain or named, lasts past the group's `)`
//! on the second regex engine (any pattern with a look-around or a back-reference); Oniguruma ends
//! it there, as it does for any group. Every expected answer was captured from jq 1.7.1
//! (`/usr/bin/jq -n`), and the rows where jq refuses the pattern pin that it is refused here too.

use anyhow::Result;

use crate::run_jq_full;

/// `$s | [match($p; "g") | [.offset, .length, .string]]`, compact; the exit code and stdout.
fn matches(subject: &str, pattern: &str) -> Result<(i32, String)> {
    let (stdout, _stderr, code) = run_jq_full(
        &[
            "-nc",
            "--arg",
            "p",
            pattern,
            "--arg",
            "s",
            subject,
            r#"$s | [match($p; "g") | [.offset, .length, .string]]"#,
        ],
        None,
    )?;
    Ok((code, stdout.trim_end().to_string()))
}

/// (subject, exit code, stdout) for one subject.
type Answer = (&'static str, i32, &'static str);

/// (pattern, the answer for each subject).
const ROWS: &[(&str, &[Answer])] = &[
    (
        r"(?i)(x(?-i))a(?=)",
        &[
            ("xA", 0, r#"[[0,2,"xA"]]"#),
            ("xa", 0, r#"[[0,2,"xa"]]"#),
            ("aA", 0, r"[]"),
            ("xX", 0, r"[]"),
        ],
    ),
    (
        r"(?i)((?-i)x)a(?=)",
        &[
            ("xA", 0, r#"[[0,2,"xA"]]"#),
            ("xa", 0, r#"[[0,2,"xa"]]"#),
            ("aA", 0, r"[]"),
            ("xX", 0, r"[]"),
        ],
    ),
    (
        r"(?i)((?-i))a(?=)",
        &[
            ("xA", 0, r#"[[1,1,"A"]]"#),
            ("xa", 0, r#"[[1,1,"a"]]"#),
            ("aA", 0, r#"[[0,1,"a"],[1,1,"A"]]"#),
            ("xX", 0, r"[]"),
        ],
    ),
    (
        r"(?i)(?<n>x(?-i))a(?=)",
        &[
            ("xA", 0, r#"[[0,2,"xA"]]"#),
            ("xa", 0, r#"[[0,2,"xa"]]"#),
            ("aA", 0, r"[]"),
            ("xX", 0, r"[]"),
        ],
    ),
    (
        r"(?i)(?'n'x(?-i))a(?=)",
        &[
            ("xA", 0, r#"[[0,2,"xA"]]"#),
            ("xa", 0, r#"[[0,2,"xa"]]"#),
            ("aA", 0, r"[]"),
            ("xX", 0, r"[]"),
        ],
    ),
    (
        r"(?-i)((?i))a\1",
        &[
            ("xA", 0, r"[]"),
            ("xa", 0, r#"[[1,1,"a"]]"#),
            ("aA", 0, r#"[[0,1,"a"]]"#),
            ("xX", 0, r"[]"),
        ],
    ),
    (
        r"(?i)(x(?-i)|y)a(?=)",
        &[
            ("xA", 0, r#"[[0,2,"xA"]]"#),
            ("xa", 0, r#"[[0,2,"xa"]]"#),
            ("aA", 0, r"[]"),
            ("xX", 0, r"[]"),
        ],
    ),
    (
        r"(?i)((?-i)|y)a(?=)",
        &[
            ("xA", 0, r#"[[1,1,"A"]]"#),
            ("xa", 0, r#"[[1,1,"a"]]"#),
            ("aA", 0, r#"[[0,1,"a"],[1,1,"A"]]"#),
            ("xX", 0, r"[]"),
        ],
    ),
    (
        r"(?i)(x(?-i))\1",
        &[
            ("xA", 0, r"[]"),
            ("xa", 0, r"[]"),
            ("aA", 0, r"[]"),
            ("xX", 0, r#"[[0,2,"xX"]]"#),
        ],
    ),
    (
        r"(?i)(?:x(?-i))a(?=)",
        &[
            ("xA", 0, r#"[[0,2,"xA"]]"#),
            ("xa", 0, r#"[[0,2,"xa"]]"#),
            ("aA", 0, r"[]"),
            ("xX", 0, r"[]"),
        ],
    ),
    (
        r"(?i)(x)((?-i)y)a(?=)",
        &[
            ("xA", 0, r"[]"),
            ("xa", 0, r"[]"),
            ("aA", 0, r"[]"),
            ("xX", 0, r"[]"),
        ],
    ),
    (
        r"(?i)(x((?-i)))a(?=)",
        &[
            ("xA", 0, r#"[[0,2,"xA"]]"#),
            ("xa", 0, r#"[[0,2,"xa"]]"#),
            ("aA", 0, r"[]"),
            ("xX", 0, r"[]"),
        ],
    ),
    (
        r"(?i)(x(?:(?-i)y))a(?=)",
        &[
            ("xA", 0, r"[]"),
            ("xa", 0, r"[]"),
            ("aA", 0, r"[]"),
            ("xX", 0, r"[]"),
        ],
    ),
    (
        r"(?i)(x(?-i)y)a(?=)",
        &[
            ("xA", 0, r"[]"),
            ("xa", 0, r"[]"),
            ("aA", 0, r"[]"),
            ("xX", 0, r"[]"),
        ],
    ),
    (
        r"(?i)(?<=(x))a",
        &[
            ("xA", 0, r#"[[1,1,"A"]]"#),
            ("xa", 0, r#"[[1,1,"a"]]"#),
            ("aA", 0, r"[]"),
            ("xX", 0, r"[]"),
        ],
    ),
    (
        r"(?x)(x (?-x) y)a(?=)",
        &[
            ("xA", 0, r"[]"),
            ("xa", 0, r"[]"),
            ("aA", 0, r"[]"),
            ("xX", 0, r"[]"),
        ],
    ),
    (
        r"(?i)(x(?-i))A(?=)",
        &[
            ("xA", 0, r#"[[0,2,"xA"]]"#),
            ("xa", 0, r#"[[0,2,"xa"]]"#),
            ("aA", 0, r"[]"),
            ("xX", 0, r"[]"),
        ],
    ),
    (
        r"(?i)(?<n>(?-i)x)\k<n>a",
        &[
            ("xA", 0, r"[]"),
            ("xa", 0, r"[]"),
            ("aA", 0, r"[]"),
            ("xX", 0, r"[]"),
        ],
    ),
    (
        r"(?<=(a(?i)))b",
        &[
            ("abc", 0, r#"[[1,1,"b"]]"#),
            ("aBc ab c", 0, r#"[[5,1,"b"]]"#),
            ("aa", 0, r"[]"),
            ("aBaB", 0, r"[]"),
        ],
    ),
    (
        r"(?<!(a(?i)))b",
        &[
            ("abc", 5, r""),
            ("aBc ab c", 5, r""),
            ("aa", 5, r""),
            ("aBaB", 5, r""),
        ],
    ),
    (
        r"(?x)(a (?-x) b)c",
        &[
            ("abc", 0, r"[]"),
            ("aBc ab c", 0, r"[]"),
            ("aa", 0, r"[]"),
            ("aBaB", 0, r"[]"),
        ],
    ),
    (
        r"(?x)(a(?-x) b)\1",
        &[
            ("abc", 0, r"[]"),
            ("aBc ab c", 0, r"[]"),
            ("aa", 0, r"[]"),
            ("aBaB", 0, r"[]"),
        ],
    ),
    (
        r"(a(?i))\1",
        &[
            ("abc", 0, r"[]"),
            ("aBc ab c", 0, r"[]"),
            ("aa", 0, r#"[[0,2,"aa"]]"#),
            ("aBaB", 0, r"[]"),
        ],
    ),
    (
        r"((a)(?i)b)\2",
        &[
            ("abc", 0, r"[]"),
            ("aBc ab c", 0, r"[]"),
            ("aa", 0, r"[]"),
            ("aBaB", 0, r#"[[0,3,"aBa"]]"#),
        ],
    ),
    (
        r"(?x)(a(?i) b)\1",
        &[
            ("abc", 0, r"[]"),
            ("aBc ab c", 0, r"[]"),
            ("aa", 0, r"[]"),
            ("aBaB", 0, r#"[[0,4,"aBaB"]]"#),
        ],
    ),
    (
        r"(a(?#(?i))b)\1",
        &[
            ("abc", 5, r""),
            ("aBc ab c", 5, r""),
            ("aa", 5, r""),
            ("aBaB", 5, r""),
        ],
    ),
    (
        r"(?i)(a(?-i)b)(?<=a)B",
        &[
            ("abc", 0, r"[]"),
            ("aBc ab c", 0, r"[]"),
            ("aa", 0, r"[]"),
            ("aBaB", 0, r"[]"),
        ],
    ),
    (
        r"(?<>a)",
        &[
            ("abc", 5, r""),
            ("aBc ab c", 5, r""),
            ("aa", 5, r""),
            ("aBaB", 5, r""),
        ],
    ),
];

#[test]
fn test_an_option_switch_in_a_capture_group_ends_at_its_paren_6212() -> Result<()> {
    for &(pattern, expected) in ROWS {
        for &(subject, code, stdout) in expected {
            let got = matches(subject, pattern)?;
            assert_eq!(got, (code, stdout.to_string()), "{subject} {pattern}");
        }
    }
    Ok(())
}
