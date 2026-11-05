//! #6326: jq 1.7.1 searches again one *byte* after an empty match, so a multibyte character gives
//! an empty match at each of its continuation bytes, reported where the next character starts
//! (`"ß" | [match("";"g") | .offset]` is `[0,1,1]`). It holds for a pattern whose empty match does
//! not depend on where it stands (no assertion); one with `\b`, `^`, `$` or a look-around is
//! recorded in `docs/compliance/jq/limitations.md`. Every row was captured from jq 1.7.1
//! (`/usr/bin/jq -nc`).

use anyhow::Result;

use crate::run_jq_full;

fn assert_rows(rows: &[(&str, &str)]) -> Result<()> {
    for (filter, expected) in rows {
        let (stdout, stderr, code) = run_jq_full(&["-nc", filter], None)?;
        assert_eq!(
            (stdout.trim_end(), code),
            (*expected, 0),
            "{filter}: stderr {stderr}"
        );
    }
    Ok(())
}

#[test]
fn test_a_multibyte_character_gives_an_empty_match_per_continuation_byte_6326() -> Result<()> {
    assert_rows(&[
        (r#""ß" | [match("";"g") | .offset]"#, r"[0,1,1]"),
        (r#""aß" | [match("";"g") | .offset]"#, r"[0,1,2,2]"),
        (r#""ßa" | [match("x*";"g") | .offset]"#, r"[0,1,1,2]"),
        (
            r#""日本" | [match("x*";"g") | .offset]"#,
            r"[0,1,1,1,2,2,2]",
        ),
        (r#""😀" | [match("x*";"g") | .offset]"#, r"[0,1,1,1,1]"),
        (r#""a😀b" | [match("";"g") | .offset]"#, r"[0,1,2,2,2,2,3]"),
        (r#""éaé" | [match("";"g") | .offset]"#, r"[0,1,1,2,3,3]"),
        (r#""ß" | gsub("";"X")"#, r#""XßXX""#),
        (r#""日a" | gsub("";"-")"#, r#""-日---a-""#),
        (r#""ß" | [splits("")]"#, r#"["","ß","",""]"#),
        (r#""é" | [scan("")]"#, r#"["","",""]"#),
        (
            r#""éa" | [match("a*";"g") | [.offset,.length]]"#,
            r"[[0,0],[1,0],[1,1],[2,0]]",
        ),
        (
            r#""éé" | [match("(x)?";"g") | [.offset,(.captures|map(.string))]]"#,
            r#"[[0,[""]],[1,[""]],[1,[""]],[2,[""]],[2,[""]]]"#,
        ),
        (
            r#""é" | [match("(?<n>x*)";"g") | .captures[0] | [.name,.offset,.string]]"#,
            r#"[["n",0,""],["n",1,""],["n",1,""]]"#,
        ),
        (r#""ß" | [match("";"gl") | .offset]"#, r"[0,1,1]"),
        (r#""ß" | [match("x*";"gn") | .offset]"#, r"[]"),
        (r#""é" | sub("";"X")"#, r#""Xé""#),
        (r#""é" | [match("";"") | .offset]"#, r"[0]"),
        (r#""é" | test("")"#, r"true"),
        (r#""abc" | [match("";"g") | .offset]"#, r"[0,1,2,3]"),
        (r#""日本" | gsub("\\s*";"_")"#, r#""_日___本___""#),
    ])
}

/// ASCII text and a pattern that never matches empty are untouched, and so is yq, whose Go engine
/// steps by character.
#[test]
fn test_text_without_a_multibyte_empty_match_is_untouched_6326() -> Result<()> {
    assert_rows(&[
        (r#""abc" | gsub("";"-")"#, r#""-a-b-c-""#),
        (r#""éa" | [match("a";"g") | .offset]"#, "[1]"),
        (r#""日本" | [match("本";"g") | .offset]"#, "[1]"),
    ])
}
