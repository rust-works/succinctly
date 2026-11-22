//! #7654: Oniguruma ends a quantifier at a blank or a `#` comment under the `x` flag, and at a
//! `(?#..)` comment, so a `?` or `+` after one is a new quantifier on the quantified atom, not the
//! lazy or possessive marker of the one before: `a{1,2} ?` under `x` is `(?:a{1,2})?` and
//! `(a)? +` is `(?:(a)?)+`. The `regex` crate drops the blank and read the lazy `a{1,2}?` and the
//! possessive `(a)?+`, so `"" | [match("a{1,2} ?$"; "gx")] | length` was `0` where jq answers `1`.
//! `{,n}`, with no lower count, is literal text to jq, and a quantifier after it repeats its `}`.
//! The rows are a sample of a generated matrix (atom x quantifier x blank, `#` and `(?#..)` gaps x
//! a following `?`, `+` or other quantifier x the flag argument and inline `(?x)`/`(?x:...)` x
//! `n`), each captured from jq 1.7.1 (`/usr/bin/jq`) and the same in jq 1.8.2; the last rows are
//! controls that answered as jq before the change and must not move.

use anyhow::Result;

use crate::run_jq_full;

/// Each row's matches: offset, length, text and captures, or the caught error.
const PROGRAM: &str = r#". as $r | $r.s | try ([if $r.fl == null then match($r.re) else match($r.re; $r.fl) end | [.offset, .length, .string, [.captures[] | [.offset, .length, .string, .name]]]]) catch "ERR: \(.)""#;

/// `(row, jq 1.7.1's output)`: `re` the pattern, `fl` the flags (`null` for `match/1`), `s` the input.
const ROWS: &[(&str, &str)] = &[
    (r#"{"re":"a{1,2} ?$","fl":"gx","s":""}"#, r#"[[0,0,"",[]]]"#),
    (
        r#"{"re":"(a)? +","fl":"xn","s":"aaa"}"#,
        r#"[[0,3,"aaa",[[2,1,"a",null]]]]"#,
    ),
    (r#"{"re":"a{2} ?","fl":"x","s":"a"}"#, r#"[[0,0,"",[]]]"#),
    (
        r#"{"re":"a* ?+b","fl":"x","s":"aab"}"#,
        r#"[[0,3,"aab",[]]]"#,
    ),
    (
        r#"{"re":"a{1,2}(?#c)?$","fl":"g","s":""}"#,
        r#"[[0,0,"",[]]]"#,
    ),
    (
        r#"{"re":"(?x)a{1,2} ?$","fl":"g","s":""}"#,
        r#"[[0,0,"",[]]]"#,
    ),
    (
        r#"{"re":"(?x:a{1,2} ?)$","fl":"g","s":""}"#,
        r#"[[0,0,"",[]]]"#,
    ),
    (
        r#"{"re":"a{1,2} #c\n?$","fl":"gx","s":""}"#,
        r#"[[0,0,"",[]]]"#,
    ),
    (
        r#"{"re":"a{1,2} ?","fl":"xn","s":"aa"}"#,
        r#"[[0,2,"aa",[]]]"#,
    ),
    (
        r#"{"re":"(a) ? +","fl":"xn","s":"aaa"}"#,
        r#"[[0,3,"aaa",[[2,1,"a",null]]]]"#,
    ),
    (
        r#"{"re":"(?x) (?>a){2}?(?#c)(?#d)+)","fl":"gxn","s":"aa"}"#,
        r#""ERR: Regex failure: unmatched close parenthesis""#,
    ),
    (
        r#"{"re":"(?x) (?:a) +++\t? a","fl":"xn","s":"a"}"#,
        r#"[[0,1,"a",[]]]"#,
    ),
    (
        r#"{"re":"(?:(?x) (a?) {0,1} (?#c) ?+|b)","fl":"xi","s":"AAB"}"#,
        r#"[[0,1,"A",[[0,1,"A",null]]]]"#,
    ),
    (
        r#"{"re":"\\w {0,1}(?#c)(?#d)?+|b","fl":"x","s":"aaa"}"#,
        r#"[[0,1,"a",[]]]"#,
    ),
    (
        r#"{"re":"(?x)b|a*+ + ?a","fl":"n","s":"aab"}"#,
        r#"[[0,1,"a",[]]]"#,
    ),
    (
        r#"{"re":"((?x)[ab]{2}+  ? +a)","fl":"xi","s":"a\nb"}"#,
        r#"[[0,1,"a",[[0,1,"a",null]]]]"#,
    ),
    (
        r#"{"re":"(?x) [ab]?(?#c)(?#d)?+|b","fl":"g","s":"aaab b"}"#,
        r#"[[0,1,"a",[]],[1,1,"a",[]],[2,1,"a",[]],[3,1,"b",[]],[4,0,"",[]],[5,1,"b",[]],[6,0,"",[]]]"#,
    ),
    (
        r#"{"re":"(?x)(?>a){1,2}?? +  ?a","fl":"gn","s":"aaab b"}"#,
        r#"[[0,3,"aaa",[]]]"#,
    ),
    (
        r#"{"re":"(?i)(?:a|b){1,2}\n?a","fl":"gx","s":"a"}"#,
        r#"[[0,1,"a",[]]]"#,
    ),
    (
        r#"{"re":"(?x) ab *? (?#c) ++)","fl":"xi","s":"b"}"#,
        r#""ERR: Regex failure: unmatched close parenthesis""#,
    ),
    (
        r#"{"re":"(?x:(?:a) ++ ? + a)","fl":"xi","s":"aab"}"#,
        r#"[[0,1,"a",[]]]"#,
    ),
    (
        r#"{"re":"( a ){2}+ ??\ta","fl":"xn","s":"aaab b"}"#,
        r#"[[0,1,"a",[[-1,0,null,null]]]]"#,
    ),
    (
        r#"{"re":"\\w ??(?#c)(?#d)? +(?#c)(?#d)?)","fl":"x","s":""}"#,
        r#""ERR: Regex failure: unmatched close parenthesis""#,
    ),
    (
        r#"{"re":"(?xi)\\w {2,}\n?|b","fl":"xi","s":"aaa"}"#,
        r#"[[0,3,"aaa",[]]]"#,
    ),
    (
        r#"{"re":"(?x) (?x:a ) {2}\t??\t?","fl":"x","s":"aaaaa"}"#,
        r#"[[0,0,"",[]]]"#,
    ),
    (
        r#"{"re":"[ab] {2}?(?#c)(?#d)?)a","fl":"gx","s":"aab"}"#,
        r#""ERR: Regex failure: unmatched close parenthesis""#,
    ),
    (
        r#"{"re":"(?x)b|(?<n>a) {2}(?#c)(?#d)? +|b","fl":"xn","s":"aaaaa"}"#,
        r#"[[0,4,"aaaa",[[3,1,"a","n"]]]]"#,
    ),
    (
        r#"{"re":"(?x)(?-x)(a?) {2}?(?#c)?)  ?)","fl":"gx","s":"aaa"}"#,
        r#""ERR: Regex failure: unmatched close parenthesis""#,
    ),
    (
        r#"{"re":"a(?x)\\  {2}?\n{2}(?#c)(?#d)+|b","fl":"gx","s":"a\nb"}"#,
        r#"[[2,1,"b",[]]]"#,
    ),
    (
        r#"{"re":"((?x)\\ {1,2} #c\n+ ?+$)","fl":"g","s":"AAB"}"#,
        r#"[[3,0,"",[[3,0,"",null]]]]"#,
    ),
    (
        r#"{"re":"(?x) ( a ) {0,1} #c\n?+ b","fl":"n","s":"aab"}"#,
        r#"[[1,2,"ab",[[1,1,"a",null]]]]"#,
    ),
    (
        r#"{"re":"(?xi)a {2}?  + ?(?#c)(?#d)?)","fl":"xi","s":"aaa"}"#,
        r#""ERR: Regex failure: unmatched close parenthesis""#,
    ),
    (
        r#"{"re":"(a)+?(?#c)?)|b","fl":"x","s":""}"#,
        r#""ERR: Regex failure: unmatched close parenthesis""#,
    ),
    (
        r#"{"re":"(?x) (?x:a ) {1,2} #c\n ?\t(?-x) b","fl":"xi","s":"aaab b"}"#,
        r#"[[4,2," b",[]]]"#,
    ),
    (
        r#"{"re":"(?x)b|(?<n>a){1,2} ?","fl":"n","s":"aaaaa"}"#,
        r#"[[0,2,"aa",[[1,1,"a","n"]]]]"#,
    ),
    (
        r#"{"re":"(?x) (a) {1,2}? (?#c) +?#c\n)","fl":"gn","s":" a a"}"#,
        r#""ERR: Regex failure: unmatched close parenthesis""#,
    ),
    (
        r#"{"re":"(?:(?x) (?:a|b){2} #c\n?)","fl":"xi","s":"b"}"#,
        r#"[[0,0,"",[]]]"#,
    ),
    (
        r#"{"re":"(?:(?x) (?i:A){0,1}  ?(?#c)+))","fl":"gxn","s":"a"}"#,
        r#""ERR: Regex failure: unmatched close parenthesis""#,
    ),
    (
        r#"{"re":"b|(?x)(?i:A)*+\n+ ?a","fl":"x","s":"aaa"}"#,
        r#"[[0,1,"a",[]]]"#,
    ),
    (
        r#"{"re":"((?x)(a?){2}?(?#c)(?#d)?+))","fl":"x","s":" a a"}"#,
        r#""ERR: Regex failure: unmatched close parenthesis""#,
    ),
    (
        r#"{"re":"(?x) (?:a*) +?(?#c)(?#d)+ ?)","fl":null,"s":""}"#,
        r#""ERR: Regex failure: unmatched close parenthesis""#,
    ),
    (
        r#"{"re":"(?i)(?>a) {2,}\t?","fl":"x","s":"a"}"#,
        r#"[[0,0,"",[]]]"#,
    ),
    (
        r#"{"re":"(?x:ab {1,2}#c\n? + #c\na$)","fl":"x","s":"aaa"}"#,
        r#"[[1,2,"aa",[]]]"#,
    ),
    (
        r#"{"re":"b|(?x)(?>a) {2,}\n? #c\n+)","fl":"n","s":""}"#,
        r#""ERR: Regex failure: unmatched close parenthesis""#,
    ),
    (r#"{"re":"(a){,2}++}","fl":"x","s":"a{,2}}b"}"#, "[]"),
    (
        r#"{"re":"(?:a|b){,2}+?b","fl":null,"s":"a{,2}}b"}"#,
        r#"[[0,7,"a{,2}}b",[]]]"#,
    ),
    (r#"{"re":"a{,2}+?","fl":"x","s":"aab"}"#, "[]"),
    (r#"{"re":"a{,2}+?$","fl":"x","s":"a{,2}b"}"#, "[]"),
    (
        r#"{"re":"[ab]{,2}++","fl":"x","s":"a{,2}}b"}"#,
        r#"[[0,6,"a{,2}}",[]]]"#,
    ),
    (r#"{"re":"[ab]{,2}+?$","fl":null,"s":"aab"}"#, "[]"),
    (r#"{"re":"a{,2}+?","fl":null,"s":"aab"}"#, "[]"),
    (r#"{"re":"(?:a|b){,2}?+}","fl":null,"s":"a{,2}b"}"#, "[]"),
    (
        r#"{"re":"(?x)b|(?:a|b) {0,1}#c\n ?\tb","fl":"g","s":"aab"}"#,
        r#"[[1,2,"ab",[]]]"#,
    ),
    (
        r#"{"re":"((?x)(?-x: a) ?? ++(?-x) b)","fl":"x","s":"aaa"}"#,
        "[]",
    ),
    (
        r#"{"re":"((?x)(?-x: a)*?\t? +|b)","fl":null,"s":"aab"}"#,
        r#"[[0,0,"",[[0,0,"",null]]]]"#,
    ),
    (
        r#"{"re":"(?x)(?-x)(?>a) +? ?+(?-x) b","fl":"gxn","s":"aa"}"#,
        "[]",
    ),
    (
        r#"{"re":"(?x)b|(a) *?+)","fl":"g","s":"aaaaa"}"#,
        r#""ERR: Regex failure: unmatched close parenthesis""#,
    ),
    (
        r#"{"re":"b|(?x)(?:a|b){2}+(?#c)(?#d)?+|b","fl":"n","s":"aaa"}"#,
        r#"[[0,2,"aa",[]]]"#,
    ),
];

#[test]
fn a_quantifier_after_a_gap_repeats_the_quantified_atom_7654() -> Result<()> {
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
