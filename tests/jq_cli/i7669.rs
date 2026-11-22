//! #7669: jq compiles its patterns with Oniguruma's Perl_NT syntax, where a `?` right after a
//! fixed interval makes it reluctant, and a reluctant `x{n}` still matches `n` times:
//! `"a" | test("(?>a){2}?a")` is `false`. The second regex engine (`fancy-regex`, which takes a
//! pattern with an atomic group, a look-around, a backreference or the `l` flag) read the `?` as
//! another quantifier, Ruby's `(?:x{n})?`, and answered `true`. A quantifier after the reluctant
//! interval repeats it (`x{n}??` is `(?:x{n})?`, `x{n}?+` is `(?:x{n})+`).
//! The rows are a sample of a generated matrix (atom x fixed or ranged interval x a following
//! `?`, `??`, `?+` and others x look-around, atomic and backreference heads x flags, `l` and
//! `n` included), each captured from jq 1.7.1 (`/usr/bin/jq`) and the same in jq 1.8.2: the
//! first ones differed before the change, the last ten are controls that already agreed.
//! Over 21,956 generated rows 170 now match jq 1.7.1 and none that matched changed.

use anyhow::Result;

use crate::run_jq_full;

/// Each row's matches: offset, length, text and captures, or the caught error.
const PROGRAM: &str = r#". as $r | $r.s | try ([if $r.fl == null then match($r.re) else match($r.re; $r.fl) end | [.offset, .length, .string, [.captures[] | [.offset, .length, .string, .name]]]]) catch "ERR: \(.)""#;

/// `(row, jq 1.7.1's output)`: `re` the pattern, `fl` the flags (`null` for `match/1`), `s` the input.
const ROWS: &[(&str, &str)] = &[
    (
        r#"{"re":"(a)\\1?(?=a)a{02}?{2}","fl":null,"s":"aaa"}"#,
        "[]",
    ),
    (r#"{"re":"b|(?>a|b){1}?{2}a","fl":null,"s":"a"}"#, "[]"),
    (
        r#"{"re":"(a)\\1?[ab]{02}?+|b","fl":"gl","s":"aaaaa"}"#,
        r#"[[0,5,"aaaaa",[[0,1,"a",null]]]]"#,
    ),
    (
        r#"{"re":"b|(?>a){2}?","fl":"g","s":"aab"}"#,
        r#"[[0,2,"aa",[]],[2,1,"b",[]]]"#,
    ),
    (
        r#"{"re":"(?x)(?=a)a{1}??|b","fl":"x","s":"ab"}"#,
        r#"[[0,1,"a",[]]]"#,
    ),
    (
        r#"{"re":"(?:(?>a|b){3}?{2}(?=a))","fl":"gl","s":"a"}"#,
        "[]",
    ),
    (
        r#"{"re":"(?>(a?){3}?+a)","fl":null,"s":"aa"}"#,
        r#"[[0,2,"aa",[[1,0,"",null]]]]"#,
    ),
    (
        r#"{"re":"(?=a).{2}?\\b","fl":"g","s":"aaaaaa"}"#,
        r#"[[4,2,"aa",[]]]"#,
    ),
    (
        r#"{"re":"(?>(?:a*){3}?+a$)","fl":"l","s":"aaaaaa"}"#,
        r#"[[0,6,"aaaaaa",[]]]"#,
    ),
    (
        r#"{"re":"(?=a)[ab]{2}?{2}(?=a)","fl":"gl","s":"aab"}"#,
        "[]",
    ),
    (
        r#"{"re":"(?:(?>a*){02}?{2}(?=a))","fl":"g","s":"aaa"}"#,
        "[]",
    ),
    (r#"{"re":"(?:(?>a|b){2}?+b)","fl":"gn","s":"ab"}"#, "[]"),
    (
        r#"{"re":"(a)\\1?[ab]{3}?{2}\\b","fl":"xn","s":"aaaaaa"}"#,
        "[]",
    ),
    (
        r#"{"re":"(?!b)a{1}?+","fl":"gl","s":"ba"}"#,
        r#"[[1,1,"a",[]]]"#,
    ),
    (r#"{"re":"(?>(?!b)a{2}?a$)","fl":null,"s":"a"}"#, "[]"),
    (
        r#"{"re":"b|a{3}??(?=a)","fl":"l","s":"aab"}"#,
        r#"[[2,1,"b",[]]]"#,
    ),
    (
        r#"{"re":"(?:(?>a){1}?+)","fl":"x","s":"aaaa"}"#,
        r#"[[0,4,"aaaa",[]]]"#,
    ),
    (r#"{"re":"(?>a{3}?{2}(?=a))","fl":"gl","s":"aaaaa"}"#, "[]"),
    (
        r#"{"re":"(a)\\1?(a?){1}??(?<=a)","fl":"gn","s":"ab"}"#,
        r#"[[0,1,"a",[[0,1,"a",null],[1,0,"",null]]]]"#,
    ),
    (r#"{"re":"(?i)(?<n>a){2}?{2}","fl":"gl","s":"aa"}"#, "[]"),
    (
        r#"{"re":"(?:a){2}??(?=a)","fl":"g","s":"aaa"}"#,
        r#"[[0,2,"aa",[]],[2,0,"",[]]]"#,
    ),
    (r#"{"re":"(?=a)(?:a){02}?{2}","fl":null,"s":"ab"}"#, "[]"),
    (r#"{"re":"(?>a{3}?+(?<=a))","fl":"g","s":"aab"}"#, "[]"),
    (r#"{"re":"(?>(?:a){3}?{2})","fl":"i","s":"aaaaa"}"#, "[]"),
    (
        r#"{"re":"(?x)(?>a|b){02}?+","fl":"l","s":"aaaa"}"#,
        r#"[[0,4,"aaaa",[]]]"#,
    ),
    (r#"{"re":"(a)\\1?a{2}?+","fl":"gn","s":"ba"}"#, "[]"),
    (r#"{"re":"(?=a).{3}?{2}a$","fl":"n","s":"aa"}"#, "[]"),
    (
        r#"{"re":"(?>(?>a*){02}?{2}(?=a))","fl":"i","s":"aab"}"#,
        "[]",
    ),
    (
        r#"{"re":"(?:(?>a|b){3}?{2}(?<=a))","fl":"i","s":"ba"}"#,
        "[]",
    ),
    (
        r#"{"re":"(?=a)a{2}?+a$","fl":"l","s":"aaaaaa"}"#,
        r#"[[1,5,"aaaaa",[]]]"#,
    ),
    (r#"{"re":"(?>(?!b)a{02}?{2}|b)","fl":"gn","s":"aaa"}"#, "[]"),
    (r#"{"re":"(?=a)(?<n>a){3}?a$","fl":"l","s":"aa"}"#, "[]"),
    (r#"{"re":"(a)\\1?(?>a){02}?+","fl":"n","s":"aab"}"#, "[]"),
    (
        r#"{"re":"(?>(?!b)a{1}?+)","fl":"l","s":"aaaaaa"}"#,
        r#"[[0,6,"aaaaaa",[]]]"#,
    ),
    (
        r#"{"re":"(a)\\1?[ab]{2}??a","fl":"gl","s":"aaaaaa"}"#,
        r#"[[0,5,"aaaaa",[[0,1,"a",null]]]]"#,
    ),
    (
        r#"{"re":"(?>.{02}?+$)","fl":null,"s":"aaaaa"}"#,
        r#"[[1,4,"aaaa",[]]]"#,
    ),
    (
        r#"{"re":"((?x) [ ]*++\\w{2}?a)","fl":"i","s":"ab a b"}"#,
        "[]",
    ),
    (
        r#"{"re":"((?x) (?>a) ?? + ( a ){2}?+ a)","fl":"i","s":"ab a b"}"#,
        "[]",
    ),
    (
        r#"{"re":"(?:(?x) \n(?#c)a {2}? #c\n(?<n>a){1,2}++)","fl":"xi","s":"ab a b"}"#,
        "[]",
    ),
    (
        r#"{"re":"((?x)\n(?: a+ b)\t{2,} ?\t(?>a){2}?+a)","fl":"xi","s":"a\nb"}"#,
        "[]",
    ),
    (
        r#"{"re":"(?x:\n(?>a) #c\n{1,2}\t(?x:a ){2}?+b)","fl":"gn","s":"aab"}"#,
        "[]",
    ),
    (r#"{"re":"b|(?x) (?#c)a {2}?+","fl":null,"s":"AAB"}"#, "[]"),
    (r#"{"re":"(a){2}?\t++ #c\n a","fl":"xn","s":"a"}"#, "[]"),
    (
        r#"{"re":"(?-x)(?:a*) {2}?(?#c){2}","fl":"gn","s":"a\nb"}"#,
        "[]",
    ),
    (
        r#"{"re":"(?:(?x) ( a ){2}? (?#c) {2}(?-x) b)","fl":"x","s":"ab a b"}"#,
        "[]",
    ),
    (r#"{"re":"(?xi). {2}? (?#c) ++$","fl":"gx","s":"a"}"#, "[]"),
    (r#"{"re":"b|a{02}?{2}$","fl":"g","s":""}"#, "[]"),
    (
        r#"{"re":"(?:(?:a*){2}?)\\b)","fl":"l","s":""}"#,
        r#""ERR: Regex failure: unmatched close parenthesis""#,
    ),
    (r#"{"re":"b|.{2}?a$","fl":"xn","s":""}"#, "[]"),
    (r#"{"re":"(?>(?=a)a{2,2}?*(?<=a))","fl":"gl","s":""}"#, "[]"),
    (
        r#"{"re":"(?x)(?>a){1,2}?+","fl":"x","s":"aa"}"#,
        r#"[[0,2,"aa",[]]]"#,
    ),
    (
        r#"{"re":"b|a(?=){1,1}??\\b","fl":"l","s":"aa"}"#,
        r#""ERR: Regex failure: target of repeat operator is invalid""#,
    ),
    (
        r#"{"re":"(?=a)(?>a*){1,1}?+(?<=a)","fl":null,"s":"a\nb"}"#,
        r#"[[0,1,"a",[]]]"#,
    ),
    (r#"{"re":"(?i)(?=a)a{1}??(?=a)","fl":"x","s":""}"#, "[]"),
    (
        r#"{"re":"a{1,2}??b","fl":"i","s":"ab"}"#,
        r#"[[0,2,"ab",[]]]"#,
    ),
    (
        r#"{"re":"(a)\\1?(?:a){3}?*(?=a)","fl":"i","s":"aaaa"}"#,
        r#"[[0,2,"aa",[[0,1,"a",null]]]]"#,
    ),
];

#[test]
fn a_lazy_fixed_interval_repeats_exactly_n_times_on_every_engine_7669() -> Result<()> {
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
