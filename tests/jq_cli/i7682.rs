//! #7682: jq compiles its patterns with Oniguruma's Perl_NG syntax, where only a `?` or a `+` right
//! after a quantifier is its lazy or possessive marker: any further quantifier is a new one on the
//! quantified atom, which the parser then reduces with the one before (`a??+` is `a*?`, so
//! `(a)??++` is the atomic `(?>(a)*?)`, which under `n` never matches). The possessive scan took
//! the last `+` of `(a)??++` for a possessive marker it could not write and refused the pattern,
//! and other runs (`a{2}?++`, `(a?){2}++`, `.?+*?+`, `(a|b)+{0}`) were refused or read as a
//! different tree. Such a run is now written as the tree Oniguruma builds, one quantifier per item.
//! The rows are a sample of a generated matrix (every pair and some triples of `?`, `*`, `+`, lazy,
//! possessive and interval quantifiers x atoms x heads and tails x flags none, `g`, `n`, `x`, `l`,
//! `gn`), each captured from jq 1.7.1 (`/usr/bin/jq`) and the same in jq 1.8.2: the first rows
//! differed before the change (51 of 61), the last ten are controls that already agreed.

use anyhow::Result;

use crate::run_jq_full;

/// Each row's matches: offset, length, text and captures, or the caught error.
const PROGRAM: &str = r#". as $r | $r.s | try ([if $r.fl == null then match($r.re) else match($r.re; $r.fl) end | [.offset, .length, .string, [.captures[] | [.offset, .length, .string, .name]]]]) catch "ERR: \(.)""#;

/// `(row, jq 1.7.1's output)`: `re` the pattern, `fl` the flags (`null` for `match/1`), `s` the input.
const ROWS: &[(&str, &str)] = &[
    (r#"{"re":"(a)??++","fl":"gn","s":"aaa"}"#, "[]"),
    (
        r#"{"re":"b|.*?++|b","fl":"n","s":"ab"}"#,
        r#"[[1,1,"b",[]]]"#,
    ),
    (r#"{"re":".?+*?+","fl":"g","s":""}"#, r#"[[0,0,"",[]]]"#),
    (
        r#"{"re":"(?x).{0,2}?++{1,2}|b","fl":"g","s":"ab"}"#,
        r#"[[0,0,"",[]],[1,0,"",[]],[2,0,"",[]]]"#,
    ),
    (
        r#"{"re":"b|ab*+{2,3}?+","fl":"l","s":"ba"}"#,
        r#"[[0,1,"b",[]]]"#,
    ),
    (
        r#"{"re":"(?x)(?:a|){2}{1,}?++$","fl":"n","s":"aaa"}"#,
        r#"[[0,3,"aaa",[]]]"#,
    ),
    (r#"{"re":"(?i).{0,}{1,2}?{2}+a","fl":"g","s":"ba"}"#, "[]"),
    (r#"{"re":"a*+{2}?++a","fl":"l","s":"ab"}"#, "[]"),
    (
        r#"{"re":"((a?){0}?+*+a)","fl":null,"s":"ab"}"#,
        r#"[[0,1,"a",[[0,1,"a",null],[-1,0,null,null]]]]"#,
    ),
    (r#"{"re":"b|(a|b)+?+?{1,}+a","fl":"l","s":""}"#, "[]"),
    (r#"{"re":"b|(?:a|)?{2}+++b","fl":"g","s":"aaaaaa"}"#, "[]"),
    (r#"{"re":"(?x)(?<n>a){1,2}?++$","fl":"n","s":"aab"}"#, "[]"),
    (
        r#"{"re":"(?:a|){0,}?++$","fl":"x","s":""}"#,
        r#"[[0,0,"",[]]]"#,
    ),
    (r#"{"re":"(?:(?:a|){1}{2}+a)","fl":"l","s":"ba"}"#, "[]"),
    (
        r#"{"re":"(?x)(?<n>a)??+","fl":"gn","s":"aaaa"}"#,
        r#"[[0,1,"a",[[0,1,"a","n"]]],[1,1,"a",[[1,1,"a","n"]]],[2,1,"a",[[2,1,"a","n"]]],[3,1,"a",[[3,1,"a","n"]]]]"#,
    ),
    (
        r#"{"re":"(?i)(a){1,}?{0}?a","fl":"x","s":"ba"}"#,
        r#"[[1,1,"a",[[-1,0,null,null]]]]"#,
    ),
    (r#"{"re":"x?(?:a|){2}++?a","fl":"l","s":"aab"}"#, "[]"),
    (
        r#"{"re":"(?:[ab]{1,}?{0,}{2}+b)","fl":"g","s":"baaab"}"#,
        "[]",
    ),
    (
        r#"{"re":"b|(a){0,2}?++a","fl":"g","s":"baaab"}"#,
        r#"[[0,1,"b",[[-1,0,null,null]]],[1,1,"a",[[-1,0,null,null]]],[2,1,"a",[[-1,0,null,null]]],[3,1,"a",[[-1,0,null,null]]],[4,1,"b",[[-1,0,null,null]]]]"#,
    ),
    (
        r#"{"re":"b|(?<n>a){2,}?+?{1,}+b","fl":"gn","s":"aa"}"#,
        "[]",
    ),
    (r#"{"re":"(?x).{2}+++a$","fl":"n","s":"a"}"#, "[]"),
    (
        r#"{"re":"((a|b){1,}?{0}a)","fl":"l","s":"ab"}"#,
        r#"[[0,1,"a",[[0,1,"a",null],[-1,0,null,null]]]]"#,
    ),
    (
        r#"{"re":"(a?){2}+{0,2}{1,2}?a","fl":"n","s":"ab"}"#,
        r#"[[0,1,"a",[[-1,0,null,null]]]]"#,
    ),
    (
        r#"{"re":"(?i)(?:a|){2,}?++","fl":"gn","s":"ab"}"#,
        r#"[[0,1,"a",[]]]"#,
    ),
    (
        r#"{"re":"(?i)(a?)+{0}a","fl":"n","s":"aaaa"}"#,
        r#"[[0,1,"a",[[-1,0,null,null]]]]"#,
    ),
    (
        r#"{"re":"(?x).{1,2}?++|b","fl":null,"s":"a"}"#,
        r#"[[0,1,"a",[]]]"#,
    ),
    (
        r#"{"re":"((a){2}++++(?=a))","fl":"gn","s":"baaab"}"#,
        r#"[[1,2,"aa",[[1,2,"aa",null],[2,1,"a",null]]]]"#,
    ),
    (r#"{"re":"(a?)*+??+?a$","fl":"gn","s":"AAb"}"#, "[]"),
    (r#"{"re":"b|(a)*+{2}++","fl":"gn","s":""}"#, "[]"),
    (
        r#"{"re":"((a?){1,2}+*?+a)","fl":"gn","s":"aaaa"}"#,
        r#"[[0,1,"a",[[0,1,"a",null],[-1,0,null,null]]],[1,1,"a",[[1,1,"a",null],[-1,0,null,null]]],[2,1,"a",[[2,1,"a",null],[-1,0,null,null]]],[3,1,"a",[[3,1,"a",null],[-1,0,null,null]]]]"#,
    ),
    (
        r#"{"re":"x?(a|b)*+??+$","fl":"gn","s":"aaa"}"#,
        r#"[[0,3,"aaa",[[2,1,"a",null]]]]"#,
    ),
    (
        r#"{"re":"b|(?:a)?+??+?|b","fl":"x","s":"aaaa"}"#,
        r#"[[0,0,"",[]]]"#,
    ),
    (
        r#"{"re":"(a)??+?{1,2}+a","fl":"g","s":"aaaaaa"}"#,
        r#"[[0,1,"a",[[-1,0,null,null]]],[1,1,"a",[[-1,0,null,null]]],[2,1,"a",[[-1,0,null,null]]],[3,1,"a",[[-1,0,null,null]]],[4,1,"a",[[-1,0,null,null]]],[5,1,"a",[[-1,0,null,null]]]]"#,
    ),
    (r#"{"re":"b|ab??++a","fl":"x","s":""}"#, "[]"),
    (
        r#"{"re":"((?:a)*++?++)","fl":"gn","s":"baaab"}"#,
        r#"[[1,3,"aaa",[[1,3,"aaa",null]]]]"#,
    ),
    (r#"{"re":"(?:(a){2}?++$)","fl":"n","s":"aab"}"#, "[]"),
    (
        r#"{"re":"a{2,}?++$","fl":"l","s":"aaa"}"#,
        r#"[[1,2,"aa",[]]]"#,
    ),
    (r#"{"re":"(a|b){0,}{2}+a","fl":null,"s":"aaa"}"#, "[]"),
    (
        r#"{"re":"(?i).*?++|b","fl":null,"s":"aa"}"#,
        r#"[[0,0,"",[]]]"#,
    ),
    (r#"{"re":"((a|b){0,2}?++)","fl":"gn","s":"aaa"}"#, "[]"),
    (
        r#"{"re":"x?.*?++{2}?a$","fl":null,"s":"aaa"}"#,
        r#"[[2,1,"a",[]]]"#,
    ),
    (
        r#"{"re":"(?x)b|(?>a)\n+?+ b","fl":"n","s":"aab"}"#,
        r#"[[0,3,"aab",[]]]"#,
    ),
    (
        r#"{"re":"(?-x)\n(?:a) {0,1}+? (?:a*+)??++(?x) b","fl":"i","s":""}"#,
        "[]",
    ),
    (
        r#"{"re":"(?x)(?-x)\n(?<n>a) #c\n{1,2}+?\na b{0,1}?+ |b","fl":"i","s":"12 a"}"#,
        "[]",
    ),
    (
        r#"{"re":"b|(?x) (?: a+ b)\n{0,1}++\t(?#c)a*?+?b*+","fl":"xi","s":"12 a"}"#,
        r#"[[0,0,"",[]]]"#,
    ),
    (
        r#"{"re":"(?xi)(?>a)\t{1,2}?++ a *+ b","fl":"x","s":"12 a"}"#,
        "[]",
    ),
    (
        r#"{"re":"((?x). *??+ #c\n[ab]+?++#c\n a)","fl":"n","s":"a  a"}"#,
        "[]",
    ),
    (
        r#"{"re":"(?i). {1,2}?++(?-x) b","fl":"xi","s":"a\nb"}"#,
        "[]",
    ),
    (
        r#"{"re":"b|(?x)ab{2}?++$","fl":"xi","s":"aab"}"#,
        r#"[[2,1,"b",[]]]"#,
    ),
    (
        r#"{"re":"(?x) (?<n>a)*?(?#c)?+","fl":"gn","s":"a\nb"}"#,
        "[]",
    ),
    (
        r#"{"re":"(?:(?x) a {2}+++ a)","fl":"gxn","s":"a\nb"}"#,
        "[]",
    ),
    (
        r#"{"re":"(?x)(a){1,2}++?{2}+(?=a)","fl":"g","s":"AAb"}"#,
        "[]",
    ),
    (
        r#"{"re":"(?x)(?:a){2}?+b","fl":"gn","s":"baaab"}"#,
        r#"[[2,3,"aab",[]]]"#,
    ),
    (
        r#"{"re":"(?i)(?:a|)+?{1}$","fl":"g","s":"ab"}"#,
        r#"[[2,0,"",[]]]"#,
    ),
    (
        r#"{"re":"b|[ab]{2}+{1,}?","fl":"n","s":"baaab"}"#,
        r#"[[0,1,"b",[]]]"#,
    ),
    (
        r#"{"re":"(?x)(?<n>a){0,2}{1,2}?a$","fl":null,"s":"aa"}"#,
        r#"[[0,2,"aa",[[0,1,"a","n"]]]]"#,
    ),
    (
        r#"{"re":"(?:a|){0}+?","fl":"l","s":"ba"}"#,
        r#"[[0,0,"",[]]]"#,
    ),
    (
        r#"{"re":"(?:a|)++{2}?","fl":null,"s":"ba"}"#,
        r#"[[0,0,"",[]]]"#,
    ),
    (
        r#"{"re":"((?:a|){1,2}?*+)","fl":"g","s":"aa"}"#,
        r#"[[0,2,"aa",[[0,2,"aa",null]]],[2,0,"",[[2,0,"",null]]]]"#,
    ),
    (
        r#"{"re":"(?:[ab]+++?)","fl":"n","s":"ba"}"#,
        r#"[[0,2,"ba",[]]]"#,
    ),
    (r#"{"re":"(?x)(a?){1,}{1,2}?$","fl":"n","s":""}"#, "[]"),
];

#[test]
fn a_quantifier_after_a_quantifier_is_read_as_oniguruma_reads_it_7682() -> Result<()> {
    let input: String = ROWS.iter().map(|(row, _)| format!("{row}\n")).collect();
    let (stdout, stderr, code) = run_jq_full(&["-c", PROGRAM], Some(&input))?;
    assert_eq!((code, stderr.as_str()), (0, ""));
    let got: Vec<&str> = stdout.lines().collect();
    assert_eq!(got.len(), ROWS.len(), "{stdout}");
    for ((row, expected), got) in ROWS.iter().zip(got) {
        assert_eq!(got, *expected, "{row}");
    }
    Ok(())
}

/// The issue's first row, as written there: jq 1.7.1 and 1.8.2 both print `0`.
#[test]
fn a_possessive_plus_after_a_lazy_optional_and_a_plus_never_matches_under_n_7682() -> Result<()> {
    let out = run_jq_full(
        &["-nc", r#""aaa" | [match("(a)??++"; "gn")] | length"#],
        None,
    )?;
    assert_eq!(out, ("0\n".to_string(), String::new(), 0));
    Ok(())
}

/// The issue's second row is the `l` flag's limit, not the run's reading: `a{2,}?{2}` is
/// `(?:a{2,}?){2}` here as in jq, and `l` stays a no-op on a pattern with a lazy quantifier
/// (docs/compliance/jq/limitations.md, "Regex flags `l` and `n`"). jq 1.7.1 and 1.8.2 print `[6]`.
#[test]
fn the_l_flag_over_a_repeated_lazy_interval_is_still_a_no_op_7682() -> Result<()> {
    let out = run_jq_full(
        &["-nc", r#""aaaaaa" | [match("b|a{2,}?{2}"; "l") | .length]"#],
        None,
    )?;
    assert_eq!(out, ("[4]\n".to_string(), String::new(), 0));
    Ok(())
}
