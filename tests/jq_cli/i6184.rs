//! #6184: a dash in a character class is read as Oniguruma reads it. The `regex` crate takes `--` for
//! set difference and a `]` that opens a class for a literal no range can start, so `[+--]` (the
//! range `+` to `-`), `[--{]` and `[]-b{]` came out as other sets and the reversed `[?--]` and
//! `[]-(]` (errors in jq) were accepted. A range that starts or ends on a set (`[[:alpha:]-a]`,
//! `[a-\\d]`) is an error in jq and now here. Rows are live jq 1.7.1 captures: the offsets and
//! lengths of every match of the class in one text, or `None` for the error (exit 5; the message
//! is the engine's own).

use anyhow::Result;

use crate::run_jq_full;

const TEXT: &str = r#""aab-?]^(+{,xAz""#;

const ROWS: &[(&str, Option<&str>)] = &[
    (r"[+--]", Some("[[3,1],[8,1],[10,1]]")),
    (
        r"[--{]",
        Some("[[0,1],[1,1],[2,1],[3,1],[4,1],[5,1],[6,1],[9,1],[11,1],[12,1],[13,1]]"),
    ),
    (r"[?--]", None),
    (r"[(?--)]", None),
    (r"[]-b{]", Some("[[0,1],[1,1],[2,1],[5,1],[6,1],[9,1]]")),
    (r"[]-(]", None),
    (r"[]-^?]", Some("[[4,1],[5,1],[6,1]]")),
    (r"[a--]", None),
    (r"[--]", Some("[[3,1]]")),
    (r"[---]", Some("[[3,1]]")),
    (
        r"[a-c--e]",
        Some("[[0,1],[1,1],[2,1],[3,1],[4,1],[5,1],[6,1],[12,1]]"),
    ),
    (
        r"[\--{]",
        Some("[[0,1],[1,1],[2,1],[3,1],[4,1],[5,1],[6,1],[9,1],[11,1],[12,1],[13,1]]"),
    ),
    (
        r"[^]-b]",
        Some("[[3,1],[4,1],[7,1],[8,1],[9,1],[10,1],[11,1],[12,1],[13,1]]"),
    ),
    (
        r"[^--a]",
        Some("[[2,1],[7,1],[8,1],[9,1],[10,1],[11,1],[13,1]]"),
    ),
    (r"[a-]", Some("[[0,1],[1,1],[3,1]]")),
    (r"[-a]", Some("[[0,1],[1,1],[3,1]]")),
    (
        r"[a\--z]",
        Some("[[0,1],[1,1],[2,1],[3,1],[4,1],[5,1],[6,1],[11,1],[12,1],[13,1]]"),
    ),
    (r"[+\--]", Some("[[3,1],[8,1]]")),
    (r"[]-\]]", Some("[[5,1]]")),
    (r"[[:alpha:]--]", None),
    (r"[--+]", None),
    (r"[a-c-e]", Some("[[0,1],[1,1],[2,1],[3,1]]")),
    (r"[,--]", Some("[[3,1],[10,1]]")),
    (r"[x\-\--]", Some("[[3,1],[11,1]]")),
    (r"[]-]", Some("[[3,1],[5,1]]")),
    (
        r"[^]-]",
        Some("[[0,1],[1,1],[2,1],[4,1],[6,1],[7,1],[8,1],[9,1],[10,1],[11,1],[12,1],[13,1]]"),
    ),
    (r"[[:alpha:]-a]", None),
    (r"[\d--a]", None),
    (r"[a-[:alpha:]]", None),
    (r"[+-[:alpha:]]", None),
    (
        r"[\w-]",
        Some("[[0,1],[1,1],[2,1],[3,1],[11,1],[12,1],[13,1]]"),
    ),
    (
        r"[a-z&&--]",
        Some("[[0,1],[1,1],[2,1],[3,1],[7,1],[8,1],[10,1],[11,1],[13,1]]"),
    ),
    (r"[~~--a]", None),
    (r"(?:[a--])", None),
    (r"x[+--]y?", Some("[]")),
];

#[test]
fn test_a_dash_in_a_class_is_read_as_oniguruma_reads_it_6184() -> Result<()> {
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

/// The same classes under the flags and inside groups and repeats.
#[test]
fn test_a_class_dash_under_flags_and_groups_6184() -> Result<()> {
    for (program, want) in [
        (r#"[match("[+--]";"gi")|.offset]"#, "[3,8,10]"),
        (r#"[match("(?:[+--])+";"g")|.offset]"#, "[3,8,10]"),
        (r#"[match("x|[]-b{]";"g")|.offset]"#, "[0,1,2,5,6,9,11]"),
        (r#"[match("[!--]";"gi")|.offset]"#, "[3,7,8,10]"),
    ] {
        let (stdout, stderr, code) = run_jq_full(&["-c", program], Some(TEXT))?;
        assert_eq!((stdout.trim_end(), code), (want, 0), "{program}: {stderr}");
    }
    Ok(())
}
