//! #7464: an inline `x` flag (`(?x)`, `(?-x)`, `(?x:...)`) is read where Oniguruma reads it when a
//! possessive quantifier is rewritten as the atomic group of its atom (#6172). The scan did not
//! model the inline flag, so any pattern naming one kept the refusal `possessive quantifier is not
//! supported` (`"aab" | test("(?x) a*+ b")`), where jq matches and the `x` argument already worked.
//! A flag group of its own sets the flag for its body; a switch sets it to the end of the group it
//! stands in, alternatives included. The rows are a sample of a generated matrix (inline flag
//! placement x atom x greedy, lazy and possessive quantifiers x blanks and `#` comments x flags x
//! inputs), each captured from jq 1.7.1 (`/usr/bin/jq`) and the same in jq 1.8.2; the last rows
//! are controls with no inline flag, which must not move.

use anyhow::Result;

use crate::run_jq_full;

/// Each row's matches: offset, length, text and captures, or the caught error.
const PROGRAM: &str = r#". as $r | $r.s | try ([if $r.fl == null then match($r.re) else match($r.re; $r.fl) end | [.offset, .length, .string, [.captures[] | [.offset, .length, .string, .name]]]]) catch "ERR: \(.)""#;

/// `(row, jq 1.7.1's output)`: `re` the pattern, `fl` the flags (`null` for `match/1`), `s` the input.
const ROWS: &[(&str, &str)] = &[
    (r#"{"re":"(?x) a*+a","fl":"g","s":"aab"}"#, "[]"),
    (
        r#"{"re":"(?x) a*+ b","fl":null,"s":"aab"}"#,
        r#"[[0,3,"aab",[]]]"#,
    ),
    (
        r#"{"re":"(?x)[ab]{2,} ? #c\n(?#c)a?+?(?x) b","fl":"i","s":"aab"}"#,
        r#"[[0,3,"aab",[]]]"#,
    ),
    (
        r#"{"re":"(?x) (?i:A)\t{1,2}+b*+","fl":"xi","s":"aaab b"}"#,
        r#"[[0,2,"aa",[]]]"#,
    ),
    (
        r#"{"re":"(?x) \na b++ (?x:a ){2,}+?$","fl":"xi","s":"b"}"#,
        "[]",
    ),
    (
        r#"{"re":"(?x)  a\n{1,2}+ #c\n(?x:a )+? +b*+","fl":"g","s":"aaa"}"#,
        r#"[[0,3,"aaa",[]]]"#,
    ),
    (
        r#"{"re":"(?x: a\n*+|b)","fl":"x","s":"aaa"}"#,
        r#"[[0,3,"aaa",[]]]"#,
    ),
    (
        r#"{"re":"(?:(?x) \n(?x:a )\t{0,1}+b*+)","fl":"gn","s":"AAB"}"#,
        "[]",
    ),
    (
        r#"{"re":"b|(?x)(a)\n{0,1}++","fl":"gn","s":"b"}"#,
        r#"[[0,1,"b",[[-1,0,null,null]]]]"#,
    ),
    (
        r#"{"re":"b|(?x) (?#c)a ?++","fl":"g","s":"a\nb"}"#,
        r#"[[0,1,"a",[]],[1,0,"",[]],[2,1,"b",[]],[3,0,"",[]]]"#,
    ),
    (
        r#"{"re":"b|(?x)  (?:a|b)+++ a","fl":"gx","s":"a  a"}"#,
        "[]",
    ),
    (
        r#"{"re":"(?x)b|\n(a){1,2}? \\w*++ b","fl":"gxn","s":"a\nb"}"#,
        r#"[[2,1,"b",[[-1,0,null,null]]]]"#,
    ),
    (
        r#"{"re":"(?x)b|(?:(?x) a)\t++ a","fl":"gn","s":"a\nb"}"#,
        r#"[[2,1,"b",[]]]"#,
    ),
    (r#"{"re":"(?x)b|a b???+ a","fl":"x","s":"a  a"}"#, "[]"),
    (
        r#"{"re":"a(?x) .\t+??+(?:a)+??(?-x) b","fl":"gxn","s":"aaab b"}"#,
        r#"[[2,4,"ab b",[]]]"#,
    ),
    (
        r#"{"re":"(?-x) (?i:A)\n?++|b","fl":"gn","s":"ab a b"}"#,
        r#"[[1,1,"b",[]],[2,2," a",[]],[5,1,"b",[]]]"#,
    ),
    (r#"{"re":"(?-x)(?:a) #c\n+++","fl":null,"s":"b"}"#, "[]"),
    (
        r#"{"re":"((?x) (a)?++ b)","fl":"x","s":"ab a b"}"#,
        r#"[[0,2,"ab",[[0,2,"ab",null],[0,1,"a",null]]]]"#,
    ),
    (
        r#"{"re":"((?x) (a) {0,1}+?#c\n a)","fl":"n","s":" a a"}"#,
        r#"[[1,1,"a",[[1,1,"a",null],[-1,0,null,null]]]]"#,
    ),
    (
        r#"{"re":"((?x) (?:a)\t{1,2}++#c\n a)","fl":"gn","s":"AAB"}"#,
        "[]",
    ),
    (
        r#"{"re":"(?:(?x) a)*??+ \\d* ?b*+","fl":"gx","s":"aab"}"#,
        r#"[[0,0,"",[]],[1,0,"",[]],[2,1,"b",[]],[3,0,"",[]]]"#,
    ),
    (
        r#"{"re":"(?:(?x)  [ab] #c\n{0,1}++ # x\nb)","fl":"n","s":"aab"}"#,
        r#"[[0,3,"aab",[]]]"#,
    ),
    (
        r#"{"re":"(?:(?x)  (?>a)\t+++b)","fl":"xi","s":" a a"}"#,
        "[]",
    ),
    (
        r#"{"re":"(?xi) (?:a|b) #c\n?+b","fl":"xn","s":"aaab b"}"#,
        r#"[[2,2,"ab",[]]]"#,
    ),
    (
        r#"{"re":"(?xi) \\  #c\n+++ a","fl":"gn","s":"a  a"}"#,
        r#"[[1,3,"  a",[]]]"#,
    ),
    (
        r#"{"re":"(?xi)(?-x: a) +??+ #c\n( a )*? ?b","fl":"g","s":"aaa"}"#,
        "[]",
    ),
    (
        r#"{"re":"(?x)(?-x:  (?:a)*+?a)","fl":"xn","s":"a  a"}"#,
        r#"[[1,3,"  a",[]]]"#,
    ),
    (
        r#"{"re":"(?x)(?-x)  ( a )  {1,2}+? |b","fl":null,"s":"aaab b"}"#,
        r#"[[3,1,"b",[[-1,0,null,null]]]]"#,
    ),
    (
        r#"{"re":"b|(?x)(?-x: a)\n+ + a *+ b","fl":"gx","s":"a  a"}"#,
        "[]",
    ),
    (
        r#"{"re":"a(?x)\na *? (?#c)a{2,}+ b","fl":"gxn","s":"aaab b"}"#,
        r#"[[0,4,"aaab",[]]]"#,
    ),
    (
        r#"{"re":"a(?x)\n( a ) #c\n{1,2}??+ #c\n\\w{2}?","fl":"g","s":"aaab b"}"#,
        r#"[[0,4,"aaab",[[1,1,"a",null]]]]"#,
    ),
    (
        r#"{"re":"a(?x)(?#c)a {2}+?\t.{2,}++$","fl":"x","s":"b"}"#,
        "[]",
    ),
    (
        r#"{"re":"(?mx)\\  #c\n*+? #c\na{2,}+?(?-x) b","fl":"gx","s":"ab a b"}"#,
        r#"[[4,2," b",[]]]"#,
    ),
    (
        r#"{"re":"(?mx) (?:(?x) a)\t{2}+\n.*??+ |b","fl":"i","s":"a\nb"}"#,
        r#"[[2,1,"b",[]]]"#,
    ),
    (r#"{"re":"(?mx)a?+ a *+ b","fl":"x","s":" a a"}"#, "[]"),
    (
        r#"{"re":"(?ix-s)(?:a) #c\n*+? # x\nb","fl":null,"s":"AAB"}"#,
        r#"[[0,3,"AAB",[]]]"#,
    ),
    (
        r#"{"re":"(?:(?x) \n\\  #c\n{1,2}?? a{1,2}+ a)","fl":"xn","s":"aaa"}"#,
        r#"[[0,3,"aaa",[]]]"#,
    ),
    (
        r#"{"re":"(?x)(?-x: (?:a)  ++?#c\n a)","fl":"gx","s":"a  a"}"#,
        "[]",
    ),
    (
        r#"{"re":"((?x)\n\\ \t{1,2}??+ a)","fl":"n","s":"a\nb"}"#,
        r#"[[0,1,"a",[[0,1,"a",null]]]]"#,
    ),
    (
        r#"{"re":"(?x:\n(?:a*+){2}+?\t\\w*? ?$)","fl":"n","s":"a  a"}"#,
        r#"[[3,1,"a",[]]]"#,
    ),
    (
        r#"{"re":"(?mx)(?#c)a\t{0,1}+ a *+ b","fl":"xn","s":"AAB"}"#,
        "[]",
    ),
    (
        r#"{"re":"(?x)b|\n(?: a | b )\n{2}a{2,}+?","fl":"g","s":"aab"}"#,
        r#"[[0,2,"aa",[]],[2,1,"b",[]]]"#,
    ),
    (
        r#"{"re":"((?x) (?x:a ) #c\n++?b)","fl":null,"s":"b"}"#,
        r#"[[0,1,"b",[[0,1,"b",null]]]]"#,
    ),
    (
        r#"{"re":"(?x)(?-x:(a) #c\n++ |b)","fl":"gx","s":" a a"}"#,
        "[]",
    ),
    (r#"{"re":"a+??+b","fl":"gx","s":""}"#, "[]"),
    (r#"{"re":" ( a ){2}+","fl":"n","s":"aaab b"}"#, "[]"),
    (r#"{"re":"(a){2}+ |b","fl":null,"s":"aaa"}"#, "[]"),
    (
        r#"{"re":" a b{1,2}++","fl":"n","s":"ab a b"}"#,
        r#"[[2,4," a b",[]]]"#,
    ),
];

#[test]
fn an_inline_x_flag_is_read_by_the_possessive_rewrite_7464() -> Result<()> {
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
