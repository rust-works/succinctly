//! #6220: `\X`, Oniguruma's extended grapheme cluster, is an error in the `regex` crate. It is
//! rewritten to the UAX #29 rules over the crate's `Grapheme_Cluster_Break` and
//! `Extended_Pictographic` properties, as an atomic group (Oniguruma never gives a cluster back in
//! part). Every row was captured from jq 1.7.1 (`/usr/bin/jq -nc`).

use anyhow::Result;

use crate::run_jq_full;

#[test]
fn x_matches_an_extended_grapheme_cluster_6220() -> Result<()> {
    for (filter, expected) in [
        (
            r#""e\u0301a" | [match("\\X";"g")|[.offset,.length]]"#,
            r"[[0,2],[2,1]]",
        ),
        (
            r#""a\r\nb\n\rc" | [match("\\X";"g")|[.offset,.length]]"#,
            r"[[0,1],[1,2],[3,1],[4,1],[5,1],[6,1]]",
        ),
        (
            r#""\ud83c\udde6\ud83c\udde7\ud83c\udde8 x" | [match("\\X";"g")|[.offset,.length]]"#,
            r"[[0,2],[2,1],[3,1],[4,1]]",
        ),
        (
            r#""\u1100\u1161\u11a8\uac00\uac01\u11a8\u1100\u1100x" | [match("\\X";"g")|[.offset,.length]]"#,
            r"[[0,3],[3,1],[4,2],[6,2],[8,1]]",
        ),
        (
            r#""\ud83d\udc68\u200d\ud83d\udc69\u200d\ud83d\udc66!\u2764\ufe0f" | [match("\\X";"g")|[.offset,.length]]"#,
            r"[[0,5],[5,1],[6,2]]",
        ),
        (
            r#""\ud83d\udc4d\ud83c\udffd\ud83d\udc4d" | [match("\\X";"g")|[.offset,.length]]"#,
            r"[[0,2],[2,1]]",
        ),
        (
            r#""\u0600a\u0600\n" | [match("\\X";"g")|[.offset,.length]]"#,
            r"[[0,2],[2,1],[3,1]]",
        ),
        (
            r#""\u0915\u093f\u0e01\u0e33\u1100" | [match("\\X";"g")|[.offset,.length]]"#,
            r"[[0,2],[2,2],[4,1]]",
        ),
        (
            r#""\u0001\u200ba" | [match("\\X";"g")|[.offset,.length]]"#,
            r"[[0,1],[1,1],[2,1]]",
        ),
        (r#""a\r\nb\n\rc" | [match("\\X\n";"g")|.length]"#, r"[2]"),
        (
            r#""a\r\nb\n\rc" | [match("(?m)\\X$";"g")|[.offset,.length]]"#,
            r"[[3,1],[6,1]]",
        ),
        (r#""a\r\nb\n\rc" | [match("\\X+";"g")|.length]"#, r"[7]"),
        (
            r#""\ud83d\udc68\u200d\ud83d\udc69\u200d\ud83d\udc66!\u2764\ufe0f" | [match("(\\X)(\\X)";"g")|[.offset,.length]]"#,
            r"[[0,6]]",
        ),
        (r#""e\u0301a" | [match("(?i)\\X{2}";"g")|.length]"#, r"[3]"),
        (
            r#""\u1100\u1161\u11a8\uac00\uac01\u11a8\u1100\u1100x" | [match("(?<=a)\\X|\\X$";"g")|.length]"#,
            r"[1]",
        ),
        (
            r#""\ud83c\udde6\ud83c\udde7\ud83c\udde8 x" | [match("[\\X]";"g")|.length]"#,
            r"[]",
        ),
    ] {
        let (stdout, stderr, code) = run_jq_full(&["-nc", filter], None)?;
        assert_eq!(stdout.trim_end(), expected, "{filter}: stderr={stderr:?}");
        assert_eq!(code, 0, "{filter}");
    }
    Ok(())
}

/// `\X` has no fixed width, so jq refuses it in a look-behind.
#[test]
fn x_in_a_look_behind_is_refused_6220() -> Result<()> {
    for pattern in [r"(?<=\X)b", r"(?<!\X)b", r"(?<=\X\X)b", r"(?<=a(?:\X))b"] {
        let filter = format!(r#""ab" | test({})"#, serde_json::to_string(pattern)?);
        let (stdout, stderr, code) = run_jq_full(&["-nc", &filter], None)?;
        assert_eq!(stdout.trim_end(), "", "{pattern}");
        assert_ne!(code, 0, "{pattern}");
        assert!(stderr.contains("look-behind"), "{pattern}: {stderr:?}");
    }
    Ok(())
}
