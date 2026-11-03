//! #6068: three constructs the `regex` crate reads one way and Oniguruma (jq's engine) another, accepted
//! here with the crate's meaning and no error:
//!
//! - `\u` and `\U` are not escapes in jq; they are the letter (`"u00e9" | test("\\u00e9")`);
//! - a `{` that does not open `{n}`, `{n,}`, `{n,m}` or `{,m}` is a literal brace (`a{ 1}`, `a{1 }`);
//! - `\p{..}` and `\P{..}` outside a class are not case-folded under `i` (`"a" | test("\\p{Lu}"; "i")`
//!   is `false`), and a `\p` or `\P` with no brace is the letter (`"pL" | test("\\pL")`).
//!
//! Every row's answer is what `/usr/bin/jq` 1.7.1 printed for `[match($p; $f) | [offset, length, string,
//! captures]]`, or `"ERR"` where it raised.

use anyhow::Result;

use crate::run_jq_full;

/// `(text, pattern, flags, jq's answer)`.
const ROWS: &[(&str, &str, &str, &str)] = &[
    (r"u00e9", r"\u00e9", r"", r#"[[0,5,"u00e9",[]]]"#),
    (r"é", r"\u00e9", r"", r"[]"),
    (
        r"U0001F600",
        r"\U0001F600",
        r"",
        r#"[[0,9,"U0001F600",[]]]"#,
    ),
    (r"😀", r"\U0001F600", r"", r"[]"),
    (r"u", r"\u", r"", r#"[[0,1,"u",[]]]"#),
    (r"uZZ", r"\uZZ", r"", r#"[[0,3,"uZZ",[]]]"#),
    (r"U", r"\U", r"", r#"[[0,1,"U",[]]]"#),
    (r"u", r"[\u]", r"", r#"[[0,1,"u",[]]]"#),
    (r"é", r"[\u00e9]", r"", r"[]"),
    (r"u0", r"[\u0]+", r"", r#"[[0,2,"u0",[]]]"#),
    (r"a\u", r"\\u", r"", r#"[[1,2,"\\u",[]]]"#),
    (r"\u00e9", r"\\u00e9", r"", r#"[[0,6,"\\u00e9",[]]]"#),
    (r"é", r"\x{e9}", r"", r#"[[0,1,"é",[]]]"#),
    (r"a", r"(?i)\u", r"", r"[]"),
    (r"U", r"\u", r"i", r#"[[0,1,"U",[]]]"#),
    (r"a{ 1}", r"a{ 1}", r"", r#"[[0,5,"a{ 1}",[]]]"#),
    (r"aa", r"a{ 1}", r"", r"[]"),
    (r"a{1 }", r"a{1 }", r"", r#"[[0,5,"a{1 }",[]]]"#),
    (r"a{ 1,2}", r"a{ 1,2}", r"", r#"[[0,7,"a{ 1,2}",[]]]"#),
    (r"a{1, 2}", r"a{1, 2}", r"", r#"[[0,7,"a{1, 2}",[]]]"#),
    (r"a{1 ,2}", r"a{1 ,2}", r"", r#"[[0,7,"a{1 ,2}",[]]]"#),
    (r"aa", r"a{1,2}", r"", r#"[[0,2,"aa",[]]]"#),
    (r"aaa", r"a{2,}", r"", r#"[[0,3,"aaa",[]]]"#),
    (r"aa", r"a{2}", r"", r#"[[0,2,"aa",[]]]"#),
    (r"a{,2}", r"a{,2}", r"", r#"[[0,5,"a{,2}",[]]]"#),
    (r"aa", r"a{,2}", r"", r"[]"),
    (r"a{", r"a{", r"", r#"[[0,2,"a{",[]]]"#),
    (r"a{1", r"a{1", r"", r#"[[0,3,"a{1",[]]]"#),
    (r"a{x}", r"a{x}", r"", r#"[[0,4,"a{x}",[]]]"#),
    (r"a{}", r"a{}", r"", r#"[[0,3,"a{}",[]]]"#),
    (r"a{,}", r"a{,}", r"", r#"[[0,4,"a{,}",[]]]"#),
    (r"{a}", r"{a}", r"", r#"[[0,3,"{a}",[]]]"#),
    (r"a}", r"a}", r"", r#"[[0,2,"a}",[]]]"#),
    (r"ab{", r"b{", r"", r#"[[1,2,"b{",[]]]"#),
    (r"aa", r"(a){2}", r"", r#"[[0,2,"aa",["a"]]]"#),
    (r"aa", r"(?:a){ 2}", r"", r"[]"),
    (r"a{ 2}", r"(?:a){ 2}", r"", r#"[[0,5,"a{ 2}",[]]]"#),
    (r"a{ 1}", r"a{ 1}", r"x", r"[]"),
    (r"aa", r"a{1}", r"x", r#"[[0,1,"a",[]]]"#),
    (r"a{", r"[{]", r"", r#"[[1,1,"{",[]]]"#),
    (r"a{ 1}", r"[a{ 1}]+", r"", r#"[[0,5,"a{ 1}",[]]]"#),
    (r"a", r"\p{Lu}", r"i", r"[]"),
    (r"é", r"\p{Lu}", r"i", r"[]"),
    (r"a", r"\P{Lu}", r"i", r#"[[0,1,"a",[]]]"#),
    (r"A", r"\p{Lu}", r"i", r#"[[0,1,"A",[]]]"#),
    (r"A", r"\p{Lu}", r"", r#"[[0,1,"A",[]]]"#),
    (r"a", r"\p{Lower}", r"i", r#"[[0,1,"a",[]]]"#),
    (r"A", r"\p{Lower}", r"i", r"[]"),
    (r"a", r"\p{Upper}", r"i", r"[]"),
    (r"a", r"(?i)\p{Lu}", r"", r"[]"),
    (r"a", r"(?i:\p{Lu})", r"", r"[]"),
    (r"a", r"[\p{Lu}]", r"i", r#"[[0,1,"a",[]]]"#),
    (r"A", r"[\p{Lu}]", r"i", r#"[[0,1,"A",[]]]"#),
    (r"a", r"\p{^Lu}", r"i", r#"[[0,1,"a",[]]]"#),
    (r"a", r"\p{Lt}", r"i", r"[]"),
    (r"ǅ", r"\p{Lt}", r"i", r#"[[0,1,"ǅ",[]]]"#),
    (r"a", r"\p{Lu}+", r"i", r"[]"),
    (r"aA", r"\p{Lu}+", r"i", r#"[[1,1,"A",[]]]"#),
    (r"aB", r"\p{Lu}\p{Lu}", r"i", r"[]"),
    (r"a", r"(\p{Lu})", r"i", r"[]"),
    (r"a", r"\p{Alpha}", r"i", r#"[[0,1,"a",[]]]"#),
    (r"a", r"\p{L}", r"i", r#"[[0,1,"a",[]]]"#),
    (r"a", r"\pL", r"i", r"[]"),
    (r"ab", r"\p{Ll}{2}", r"i", r#"[[0,2,"ab",[]]]"#),
    (r"a", r"\p{Lu}", r"l", r"[]"),
    (r"a", r"\p{Lu}", r"n", r"[]"),
    (r"a", r"\p{Lu}", r"gi", r"[]"),
    (r"PL", r"\PL", r"", r#"[[0,2,"PL",[]]]"#),
    (r"a", r"\PL", r"", r"[]"),
    (r"pLu", r"\pLu", r"", r#"[[0,3,"pLu",[]]]"#),
    (r"p", r"[\pL]", r"", r#"[[0,1,"p",[]]]"#),
    (r"L", r"[\pL]", r"", r#"[[0,1,"L",[]]]"#),
    (r"a", r"[\pL]", r"", r"[]"),
    (r"P", r"[\PL]", r"", r#"[[0,1,"P",[]]]"#),
    (r"a", r"\p{L}", r"", r#"[[0,1,"a",[]]]"#),
    (r"p", r"\p", r"", r#"[[0,1,"p",[]]]"#),
    (r"p1", r"\p1", r"", r#"[[0,2,"p1",[]]]"#),
    (r"p-", r"\p-", r"", r#"[[0,2,"p-",[]]]"#),
    (r"a", r"[\p{L}]", r"", r#"[[0,1,"a",[]]]"#),
    (r"1", r"\p{^L}", r"", r#"[[0,1,"1",[]]]"#),
    (r"x", r"\p{Lu", r"", r#""ERR""#),
    (r"pL", r"\pL", r"i", r#"[[0,2,"pL",[]]]"#),
    (r"PL", r"\pL", r"i", r#"[[0,2,"PL",[]]]"#),
    (r"a", r"\p{L}\pL", r"", r"[]"),
];

#[test]
fn oniguruma_readings_of_u_braces_and_properties() -> Result<()> {
    let program = r#"$s | try ([match($p; $f) | [.offset, .length, .string, (.captures | map(.string))]]) catch "ERR""#;
    for (text, pattern, flags, want) in ROWS {
        let (stdout, stderr, code) = run_jq_full(
            &[
                "-nc", "--arg", "s", text, "--arg", "p", pattern, "--arg", "f", flags, program,
            ],
            None,
        )?;
        assert_eq!(code, 0, "{text:?} ~ {pattern:?} [{flags}]: {stderr}");
        assert_eq!(stdout.trim(), *want, "{text:?} ~ {pattern:?} [{flags}]");
    }
    Ok(())
}

/// The three constructs also hold on the `fancy-regex` engine (lookaround, a backreference and the `l`
/// flag take it), where the same rewritten pattern is compiled.
#[test]
fn the_same_readings_hold_on_the_backtracking_engine() -> Result<()> {
    let program = r#"$s | try ([match($p; $f) | [.offset, .length, .string]]) catch "ERR""#;
    for (text, pattern, flags, want) in [
        ("ua", r"(?=\u)u", "", r#"[[0,1,"u"]]"#),
        ("a{ 1}", r"(?<=a)\{ 1}", "", r#"[[1,4,"{ 1}"]]"#),
        ("aa", r"(?<=a)a{ 1}", "", "[]"),
        ("aA", r"(?<=a)\p{Lu}", "i", r#"[[1,1,"A"]]"#),
        ("aa", r"(?<=a)\p{Lu}", "i", "[]"),
        ("a{ 1}a{ 1}", r"(a{ 1})\1", "", r#"[[0,10,"a{ 1}a{ 1}"]]"#),
        ("aAa", r"\p{Lu}", "il", r#"[[1,1,"A"]]"#),
        ("a", r"\p{Lu}", "il", "[]"),
    ] {
        let (stdout, stderr, code) = run_jq_full(
            &[
                "-nc", "--arg", "s", text, "--arg", "p", pattern, "--arg", "f", flags, program,
            ],
            None,
        )?;
        assert_eq!(code, 0, "{text:?} ~ {pattern:?} [{flags}]: {stderr}");
        assert_eq!(stdout.trim(), want, "{text:?} ~ {pattern:?} [{flags}]");
    }
    Ok(())
}
