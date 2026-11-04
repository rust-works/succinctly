//! #6221: a repeated plain group whose body is a zero-width item that only the second engine reads
//! (`\G`, `\K`, `\Z`, a look-ahead) and then a bare option group (`(?:\G(?m))*`) was refused where jq
//! answers: the group is a sequence of two nodes to Oniguruma, a valid target, and `fancy-regex`
//! refused the zero-width body. It is rewritten as the atomic group of its body, as #6164 and #6187
//! do for an option group. An option group first (`(?:(?m)\Z)*`) encloses the rest, and a body of
//! one node (`(?:\K)*`) is refused by jq as it is here. Every row was captured from jq 1.7.1
//! (`/usr/bin/jq -n`); a row jq refuses pins that it is refused here too.

use anyhow::Result;

use crate::run_jq_full;

/// (subject, whether jq refuses the pattern, jq's `[offset, length]` list when it does not).
type Answer = (&'static str, bool, &'static str);

/// `$s | [match($p; "g") | [.offset, .length]]`, compact: whether it failed, and stdout.
fn matches(subject: &str, pattern: &str) -> Result<(bool, String)> {
    let (stdout, _stderr, code) = run_jq_full(
        &[
            "-nc",
            "--arg",
            "p",
            pattern,
            "--arg",
            "s",
            subject,
            r#"$s | [match($p; "g") | [.offset, .length]]"#,
        ],
        None,
    )?;
    Ok((code != 0, stdout.trim_end().to_string()))
}

/// (pattern, the answer for each subject).
const ROWS: &[(&str, &[Answer])] = &[
    (
        "(?:\\G(?m))*?",
        &[
            ("cab", false, "[[0,0],[1,0],[2,0],[3,0]]"),
            ("ab c", false, "[[0,0],[1,0],[2,0],[3,0],[4,0]]"),
        ],
    ),
    (
        "(?:\\G(?m))*?b",
        &[("cab", false, "[[2,1]]"), ("ab c", false, "[[1,1]]")],
    ),
    ("(?:(?m)\\G)?", &[("cab", true, ""), ("ab c", true, "")]),
    ("(?:(?m)\\G)*?b", &[("cab", true, ""), ("ab c", true, "")]),
    (
        "(?:\\G(?:(?i)))*+",
        &[
            ("cab", false, "[[0,0],[1,0],[2,0],[3,0]]"),
            ("ab c", false, "[[0,0],[1,0],[2,0],[3,0],[4,0]]"),
        ],
    ),
    (
        "(?:\\G(?:(?i)))*?b",
        &[("cab", false, "[[2,1]]"), ("ab c", false, "[[1,1]]")],
    ),
    (
        "(?:\\G(?m)(?m))*?",
        &[
            ("cab", false, "[[0,0],[1,0],[2,0],[3,0]]"),
            ("ab c", false, "[[0,0],[1,0],[2,0],[3,0],[4,0]]"),
        ],
    ),
    (
        "(?:\\G(?m)(?m))*b",
        &[("cab", false, "[[2,1]]"), ("ab c", false, "[[1,1]]")],
    ),
    (
        "(?:(?i)\\G(?i))?",
        &[
            ("cab", false, "[[0,0],[1,0],[2,0],[3,0]]"),
            ("ab c", false, "[[0,0],[1,0],[2,0],[3,0],[4,0]]"),
        ],
    ),
    (
        "(?:(?i)\\G(?i))*?b",
        &[("cab", false, "[[2,1]]"), ("ab c", false, "[[1,1]]")],
    ),
    (
        "(?:\\G(?m)b)+",
        &[("cab", false, "[]"), ("ab c", false, "[]")],
    ),
    (
        "(?:\\G(?m)b)*+b",
        &[("cab", false, "[[2,1]]"), ("ab c", false, "[[1,1]]")],
    ),
    (
        "(?:\\K(?i))*?",
        &[
            ("cab", false, "[[0,0],[1,0],[2,0],[3,0]]"),
            ("ab c", false, "[[0,0],[1,0],[2,0],[3,0],[4,0]]"),
        ],
    ),
    (
        "(?:\\K(?i))*?b",
        &[("cab", false, "[[2,1]]"), ("ab c", false, "[[1,1]]")],
    ),
    ("(?:(?i)\\K){2}", &[("cab", true, ""), ("ab c", true, "")]),
    ("(?:(?i)\\K)*+b", &[("cab", true, ""), ("ab c", true, "")]),
    (
        "(?:\\K(?:(?m)))+",
        &[
            ("cab", false, "[[0,0],[1,0],[2,0],[3,0]]"),
            ("ab c", false, "[[0,0],[1,0],[2,0],[3,0],[4,0]]"),
        ],
    ),
    (
        "(?:\\K(?:(?m)))*+b",
        &[("cab", false, "[[2,1]]"), ("ab c", false, "[[1,1]]")],
    ),
    (
        "(?:\\K(?m)(?m))*?",
        &[
            ("cab", false, "[[0,0],[1,0],[2,0],[3,0]]"),
            ("ab c", false, "[[0,0],[1,0],[2,0],[3,0],[4,0]]"),
        ],
    ),
    (
        "(?:\\K(?m)(?m)){2}b",
        &[("cab", false, "[[2,1]]"), ("ab c", false, "[[1,1]]")],
    ),
    (
        "(?:(?-i)\\K(?-i))*",
        &[
            ("cab", false, "[[0,0],[1,0],[2,0],[3,0]]"),
            ("ab c", false, "[[0,0],[1,0],[2,0],[3,0],[4,0]]"),
        ],
    ),
    (
        "(?:(?-i)\\K(?-i))*+b",
        &[("cab", false, "[[2,1]]"), ("ab c", false, "[[1,1]]")],
    ),
    (
        "(?:\\K(?m)b)+",
        &[("cab", false, "[[2,1]]"), ("ab c", false, "[[1,1]]")],
    ),
    (
        "(?:\\K(?m)b)*?b",
        &[("cab", false, "[[2,1]]"), ("ab c", false, "[[1,1]]")],
    ),
    (
        "(?:\\Z(?m))?",
        &[
            ("cab", false, "[[0,0],[1,0],[2,0],[3,0]]"),
            ("ab c", false, "[[0,0],[1,0],[2,0],[3,0],[4,0]]"),
        ],
    ),
    (
        "(?:\\Z(?m))*b",
        &[("cab", false, "[[2,1]]"), ("ab c", false, "[[1,1]]")],
    ),
    ("(?:(?i)\\Z){2}", &[("cab", true, ""), ("ab c", true, "")]),
    ("(?:(?i)\\Z)*?b", &[("cab", true, ""), ("ab c", true, "")]),
    (
        "(?:\\Z(?:(?-i))){2}",
        &[("cab", false, "[[3,0]]"), ("ab c", false, "[[4,0]]")],
    ),
    (
        "(?:\\Z(?:(?-i)))*+b",
        &[("cab", false, "[[2,1]]"), ("ab c", false, "[[1,1]]")],
    ),
    (
        "(?:\\Z(?i)(?i)){2}",
        &[("cab", false, "[[3,0]]"), ("ab c", false, "[[4,0]]")],
    ),
    (
        "(?:\\Z(?i)(?i))*+b",
        &[("cab", false, "[[2,1]]"), ("ab c", false, "[[1,1]]")],
    ),
    (
        "(?:(?-i)\\Z(?-i)){2}",
        &[("cab", false, "[[3,0]]"), ("ab c", false, "[[4,0]]")],
    ),
    (
        "(?:(?-i)\\Z(?-i))+b",
        &[("cab", false, "[]"), ("ab c", false, "[]")],
    ),
    (
        "(?:\\Z(?i)b)*",
        &[
            ("cab", false, "[[0,0],[1,0],[2,0],[3,0]]"),
            ("ab c", false, "[[0,0],[1,0],[2,0],[3,0],[4,0]]"),
        ],
    ),
    (
        "(?:\\Z(?i)b)*b",
        &[("cab", false, "[[2,1]]"), ("ab c", false, "[[1,1]]")],
    ),
    (
        "(?:(?=a)(?m)){2}",
        &[("cab", false, "[[1,0]]"), ("ab c", false, "[[0,0]]")],
    ),
    (
        "(?:(?=a)(?m))+b",
        &[("cab", false, "[]"), ("ab c", false, "[]")],
    ),
    ("(?:(?i)(?=a))*+", &[("cab", true, ""), ("ab c", true, "")]),
    (
        "(?:(?i)(?=a)){2}b",
        &[("cab", true, ""), ("ab c", true, "")],
    ),
    (
        "(?:(?=a)(?:(?-i)))?",
        &[
            ("cab", false, "[[0,0],[1,0],[2,0],[3,0]]"),
            ("ab c", false, "[[0,0],[1,0],[2,0],[3,0],[4,0]]"),
        ],
    ),
    (
        "(?:(?=a)(?:(?-i))){2}b",
        &[("cab", false, "[]"), ("ab c", false, "[]")],
    ),
    (
        "(?:(?=a)(?-i)(?-i)){2}",
        &[("cab", false, "[[1,0]]"), ("ab c", false, "[[0,0]]")],
    ),
    (
        "(?:(?=a)(?-i)(?-i))*?b",
        &[("cab", false, "[[2,1]]"), ("ab c", false, "[[1,1]]")],
    ),
    (
        "(?:(?i)(?=a)(?i))*?",
        &[
            ("cab", false, "[[0,0],[1,0],[2,0],[3,0]]"),
            ("ab c", false, "[[0,0],[1,0],[2,0],[3,0],[4,0]]"),
        ],
    ),
    (
        "(?:(?i)(?=a)(?i))*?b",
        &[("cab", false, "[[2,1]]"), ("ab c", false, "[[1,1]]")],
    ),
    (
        "(?:(?=a)(?i)b)*?",
        &[
            ("cab", false, "[[0,0],[1,0],[2,0],[3,0]]"),
            ("ab c", false, "[[0,0],[1,0],[2,0],[3,0],[4,0]]"),
        ],
    ),
    (
        "(?:(?=a)(?i)b)+b",
        &[("cab", false, "[]"), ("ab c", false, "[]")],
    ),
    (
        "(?:(?<=c)(?i))*+",
        &[
            ("cab", false, "[[0,0],[1,0],[2,0],[3,0]]"),
            ("ab c", false, "[[0,0],[1,0],[2,0],[3,0],[4,0]]"),
        ],
    ),
    (
        "(?:(?<=c)(?i))*b",
        &[("cab", false, "[[2,1]]"), ("ab c", false, "[[1,1]]")],
    ),
    ("(?:(?i)(?<=c))*?", &[("cab", true, ""), ("ab c", true, "")]),
    (
        "(?:(?i)(?<=c))*+b",
        &[("cab", true, ""), ("ab c", true, "")],
    ),
    (
        "(?:(?<=c)(?:(?-i)))+",
        &[("cab", false, "[[1,0]]"), ("ab c", false, "[[4,0]]")],
    ),
    (
        "(?:(?<=c)(?:(?-i)))*+b",
        &[("cab", false, "[[2,1]]"), ("ab c", false, "[[1,1]]")],
    ),
    (
        "(?:(?<=c)(?i)(?i))*?",
        &[
            ("cab", false, "[[0,0],[1,0],[2,0],[3,0]]"),
            ("ab c", false, "[[0,0],[1,0],[2,0],[3,0],[4,0]]"),
        ],
    ),
    (
        "(?:(?<=c)(?i)(?i))*?b",
        &[("cab", false, "[[2,1]]"), ("ab c", false, "[[1,1]]")],
    ),
    (
        "(?:(?-i)(?<=c)(?-i))*",
        &[
            ("cab", false, "[[0,0],[1,0],[2,0],[3,0]]"),
            ("ab c", false, "[[0,0],[1,0],[2,0],[3,0],[4,0]]"),
        ],
    ),
    (
        "(?:(?-i)(?<=c)(?-i))*+b",
        &[("cab", false, "[[2,1]]"), ("ab c", false, "[[1,1]]")],
    ),
    (
        "(?:(?<=c)(?-i)b)+",
        &[("cab", false, "[]"), ("ab c", false, "[]")],
    ),
    (
        "(?:(?<=c)(?-i)b)*+b",
        &[("cab", false, "[[2,1]]"), ("ab c", false, "[[1,1]]")],
    ),
    (
        "(?:\\b(?-i))?",
        &[
            ("cab", false, "[[0,0],[1,0],[2,0],[3,0]]"),
            ("ab c", false, "[[0,0],[1,0],[2,0],[3,0],[4,0]]"),
        ],
    ),
    (
        "(?:\\b(?-i))?b",
        &[("cab", false, "[[2,1]]"), ("ab c", false, "[[1,1]]")],
    ),
    ("(?:(?m)\\b)*", &[("cab", true, ""), ("ab c", true, "")]),
    ("(?:(?m)\\b){2}b", &[("cab", true, ""), ("ab c", true, "")]),
    (
        "(?:\\b(?:(?-i))){2}",
        &[
            ("cab", false, "[[0,0],[3,0]]"),
            ("ab c", false, "[[0,0],[2,0],[3,0],[4,0]]"),
        ],
    ),
    (
        "(?:\\b(?:(?-i)))*b",
        &[("cab", false, "[[2,1]]"), ("ab c", false, "[[1,1]]")],
    ),
    (
        "(?:\\b(?i)(?i))*",
        &[
            ("cab", false, "[[0,0],[1,0],[2,0],[3,0]]"),
            ("ab c", false, "[[0,0],[1,0],[2,0],[3,0],[4,0]]"),
        ],
    ),
    (
        "(?:\\b(?i)(?i)){2}b",
        &[("cab", false, "[]"), ("ab c", false, "[]")],
    ),
    (
        "(?:(?m)\\b(?m))*",
        &[
            ("cab", false, "[[0,0],[1,0],[2,0],[3,0]]"),
            ("ab c", false, "[[0,0],[1,0],[2,0],[3,0],[4,0]]"),
        ],
    ),
    (
        "(?:(?m)\\b(?m))?b",
        &[("cab", false, "[[2,1]]"), ("ab c", false, "[[1,1]]")],
    ),
    (
        "(?:\\b(?i)b){2}",
        &[("cab", false, "[]"), ("ab c", false, "[]")],
    ),
    (
        "(?:\\b(?i)b)*b",
        &[("cab", false, "[[2,1]]"), ("ab c", false, "[[1,1]]")],
    ),
    (
        "(?:^(?m))*?",
        &[
            ("cab", false, "[[0,0],[1,0],[2,0],[3,0]]"),
            ("ab c", false, "[[0,0],[1,0],[2,0],[3,0],[4,0]]"),
        ],
    ),
    (
        "(?:^(?m))*?b",
        &[("cab", false, "[[2,1]]"), ("ab c", false, "[[1,1]]")],
    ),
    ("(?:(?m)^){2}", &[("cab", true, ""), ("ab c", true, "")]),
    ("(?:(?m)^)*+b", &[("cab", true, ""), ("ab c", true, "")]),
    (
        "(?:^(?:(?-i)))?",
        &[
            ("cab", false, "[[0,0],[1,0],[2,0],[3,0]]"),
            ("ab c", false, "[[0,0],[1,0],[2,0],[3,0],[4,0]]"),
        ],
    ),
    (
        "(?:^(?:(?-i)))*?b",
        &[("cab", false, "[[2,1]]"), ("ab c", false, "[[1,1]]")],
    ),
    (
        "(?:^(?i)(?i))*?",
        &[
            ("cab", false, "[[0,0],[1,0],[2,0],[3,0]]"),
            ("ab c", false, "[[0,0],[1,0],[2,0],[3,0],[4,0]]"),
        ],
    ),
    (
        "(?:^(?i)(?i))+b",
        &[("cab", false, "[]"), ("ab c", false, "[]")],
    ),
    (
        "(?:(?m)^(?m))?",
        &[
            ("cab", false, "[[0,0],[1,0],[2,0],[3,0]]"),
            ("ab c", false, "[[0,0],[1,0],[2,0],[3,0],[4,0]]"),
        ],
    ),
    (
        "(?:(?m)^(?m))*b",
        &[("cab", false, "[[2,1]]"), ("ab c", false, "[[1,1]]")],
    ),
    (
        "(?:^(?m)b)*",
        &[
            ("cab", false, "[[0,0],[1,0],[2,0],[3,0]]"),
            ("ab c", false, "[[0,0],[1,0],[2,0],[3,0],[4,0]]"),
        ],
    ),
    (
        "(?:^(?m)b)*?b",
        &[("cab", false, "[[2,1]]"), ("ab c", false, "[[1,1]]")],
    ),
    (
        "(?:\\G(?m))*",
        &[
            ("cab", false, "[[0,0],[1,0],[2,0],[3,0]]"),
            ("ab c", false, "[[0,0],[1,0],[2,0],[3,0],[4,0]]"),
        ],
    ),
    (
        "(?:\\K(?m))*",
        &[
            ("cab", false, "[[0,0],[1,0],[2,0],[3,0]]"),
            ("ab c", false, "[[0,0],[1,0],[2,0],[3,0],[4,0]]"),
        ],
    ),
    (
        "(?:\\Z(?m))*",
        &[
            ("cab", false, "[[0,0],[1,0],[2,0],[3,0]]"),
            ("ab c", false, "[[0,0],[1,0],[2,0],[3,0],[4,0]]"),
        ],
    ),
    (
        "(?:(?=a)(?i))*",
        &[
            ("cab", false, "[[0,0],[1,0],[2,0],[3,0]]"),
            ("ab c", false, "[[0,0],[1,0],[2,0],[3,0],[4,0]]"),
        ],
    ),
    (
        "(?:\\G(?m)\\G)*",
        &[
            ("cab", false, "[[0,0],[1,0],[2,0],[3,0]]"),
            ("ab c", false, "[[0,0],[1,0],[2,0],[3,0],[4,0]]"),
        ],
    ),
    (
        "(?:\\b(?i))*",
        &[
            ("cab", false, "[[0,0],[1,0],[2,0],[3,0]]"),
            ("ab c", false, "[[0,0],[1,0],[2,0],[3,0],[4,0]]"),
        ],
    ),
    ("(?:(?m)\\Z)*", &[("cab", true, ""), ("ab c", true, "")]),
    (
        "(?:\\G(?:(?m)))*",
        &[
            ("cab", false, "[[0,0],[1,0],[2,0],[3,0]]"),
            ("ab c", false, "[[0,0],[1,0],[2,0],[3,0],[4,0]]"),
        ],
    ),
    ("(?:\\K)*", &[("cab", true, ""), ("ab c", true, "")]),
    (
        "(?:\\G\\K)*",
        &[
            ("cab", false, "[[0,0],[1,0],[2,0],[3,0]]"),
            ("ab c", false, "[[0,0],[1,0],[2,0],[3,0],[4,0]]"),
        ],
    ),
];

#[test]
fn test_a_repeated_two_node_zero_width_group_is_a_valid_target_6221() -> Result<()> {
    for &(pattern, expected) in ROWS {
        for &(subject, refused, stdout) in expected {
            let (failed, got) = matches(subject, pattern)?;
            assert_eq!(failed, refused, "{subject} {pattern:?}");
            if !refused {
                assert_eq!(got, stdout, "{subject} {pattern:?}");
            }
        }
    }
    Ok(())
}

/// Under the `x` flag, inline or as a flag, a blank or a `#` comment is no node: `(?:\G )*` is a body
/// of one, which jq refuses, so the rewrite is not offered a body that holds either.
#[test]
fn test_a_blank_in_the_body_is_no_node_under_the_x_flag_6221() -> Result<()> {
    for (pattern, flags) in [
        (r"(?x)(?:\G )*", "g"),
        (r"(?:\G )*", "gx"),
        ("(?:\\G #c\n)*", "gx"),
        (r"(?x)(?:\K  )+b", "g"),
    ] {
        let (stdout, stderr, code) = run_jq_full(
            &[
                "-nc",
                "--arg",
                "p",
                pattern,
                "--arg",
                "f",
                flags,
                r#""cab" | [match($p; $f) | .string]"#,
            ],
            None,
        )?;
        assert_eq!(
            code, 5,
            "{pattern:?} {flags}: stdout={stdout:?} stderr={stderr:?}"
        );
    }
    Ok(())
}
