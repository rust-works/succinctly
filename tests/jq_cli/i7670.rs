//! #7670: Oniguruma ends a greedy repeat of a body that can match empty with an iteration that
//! matched empty, and a group in the body keeps that empty iteration, with or without the `n`
//! flag: `"a" | match("(a*)*"; "n") | .captures[0].offset` is `1` in jq. Without `n` the
//! capture rewrite (#6069) writes that iteration into the pattern; under `n` the pattern went to
//! `fancy-regex` as written, which keeps the last non-empty iteration (`0`). The rewrite now runs
//! there too, but not over a pattern the engine reads with an atomic group (a possessive
//! quantifier), which the rewrite cannot read.
//! The rows are a sample of a generated matrix (bodies that can match empty x unbounded and
//! bounded repeats x capture groups x optional and repeated wrappers x `n`, `gn`, `xn`, `in`
//! and no flag), each captured from jq 1.7.1 (`/usr/bin/jq`) and the same in jq 1.8.2: the first
//! ones differed before the change, the last eleven are controls that already agreed (the last of them
//! the one row a first version of the change moved).
//! Over 24,956 generated rows 181 now match jq 1.7.1 and none that matched changed.

use anyhow::Result;

use crate::run_jq_full;

/// Each row's matches: offset, length, text and captures, or the caught error.
const PROGRAM: &str = r#". as $r | $r.s | try ([if $r.fl == null then match($r.re) else match($r.re; $r.fl) end | [.offset, .length, .string, [.captures[] | [.offset, .length, .string, .name]]]]) catch "ERR: \(.)""#;

/// `(row, jq 1.7.1's output)`: `re` the pattern, `fl` the flags (`null` for `match/1`), `s` the input.
const ROWS: &[(&str, &str)] = &[
    (
        r#"{"re":"(?:(a*)*)+","fl":"n","s":"ba"}"#,
        r#"[[1,1,"a",[[2,0,"",null]]]]"#,
    ),
    (
        r#"{"re":"((?<n>a*){1,})?","fl":"gn","s":"aaa"}"#,
        r#"[[0,3,"aaa",[[0,3,"aaa",null],[3,0,"","n"]]]]"#,
    ),
    (
        r#"{"re":"((?<n>a*){1,})?","fl":"n","s":"aab"}"#,
        r#"[[0,2,"aa",[[0,2,"aa",null],[2,0,"","n"]]]]"#,
    ),
    (
        r#"{"re":"x?(?<n>a?){1,}","fl":"in","s":"a"}"#,
        r#"[[0,1,"a",[[1,0,"","n"]]]]"#,
    ),
    (
        r#"{"re":"^(?<n>a?){1,}","fl":"n","s":"a"}"#,
        r#"[[0,1,"a",[[1,0,"","n"]]]]"#,
    ),
    (
        r#"{"re":"(?:((\\w*)){0,})+","fl":"n","s":"abab"}"#,
        r#"[[0,4,"abab",[[4,0,"",null],[4,0,"",null]]]]"#,
    ),
    (
        r#"{"re":"(?:(?<n>\\w*)?)+","fl":"n","s":"ab"}"#,
        r#"[[0,2,"ab",[[2,0,"","n"]]]]"#,
    ),
    (
        r#"{"re":"((?<n>a*?){0,})?","fl":"n","s":"aaa"}"#,
        r#"[[0,1,"a",[[0,1,"a",null],[1,0,"","n"]]]]"#,
    ),
    (
        r#"{"re":"(?:(a*?)*)?","fl":"n","s":"aab"}"#,
        r#"[[0,1,"a",[[1,0,"",null]]]]"#,
    ),
    (
        r#"{"re":"(?<n>(?:a?)){1,}","fl":"xn","s":"aaa"}"#,
        r#"[[0,3,"aaa",[[3,0,"","n"]]]]"#,
    ),
    (
        r#"{"re":"^([ab]?){1,}","fl":"xn","s":"b"}"#,
        r#"[[0,1,"b",[[1,0,"",null]]]]"#,
    ),
    (
        r#"{"re":"(?:(?<n>[ab]?){1,})?","fl":"n","s":"xa"}"#,
        r#"[[1,1,"a",[[2,0,"","n"]]]]"#,
    ),
    (
        r#"{"re":"(?:(a*)*)?","fl":"gn","s":"ab"}"#,
        r#"[[0,1,"a",[[1,0,"",null]]]]"#,
    ),
    (
        r#"{"re":"(a*){0,}","fl":"xn","s":"ba"}"#,
        r#"[[1,1,"a",[[2,0,"",null]]]]"#,
    ),
    (
        r#"{"re":"(?:(?<n>(?:a|b)?){1,})+","fl":"n","s":"b"}"#,
        r#"[[0,1,"b",[[1,0,"","n"]]]]"#,
    ),
    (
        r#"{"re":"(a*?)*b?","fl":"n","s":"ab"}"#,
        r#"[[0,2,"ab",[[1,0,"",null]]]]"#,
    ),
    (
        r#"{"re":"(?:(?<n>(?:a|b)?)+)+","fl":"gn","s":"aa"}"#,
        r#"[[0,2,"aa",[[2,0,"","n"]]]]"#,
    ),
    (
        r#"{"re":"x?(\\w*){0,}","fl":"gn","s":"aa"}"#,
        r#"[[0,2,"aa",[[2,0,"",null]]]]"#,
    ),
    (
        r#"{"re":"x?(?<n>a*?){0,}","fl":"n","s":"ba"}"#,
        r#"[[1,1,"a",[[2,0,"","n"]]]]"#,
    ),
    (
        r#"{"re":"(?:([ab]?){1,})?","fl":"n","s":"aaa"}"#,
        r#"[[0,3,"aaa",[[3,0,"",null]]]]"#,
    ),
    (
        r#"{"re":"((?:a|b)?){1,}b?","fl":"n","s":"abab"}"#,
        r#"[[0,4,"abab",[[4,0,"",null]]]]"#,
    ),
    (
        r#"{"re":"(?:(?<n>a*?){1,})?","fl":"gn","s":"xa"}"#,
        r#"[[1,1,"a",[[2,0,"","n"]]]]"#,
    ),
    (
        r#"{"re":"^(?<n>\\w*)+","fl":"xn","s":"ab"}"#,
        r#"[[0,2,"ab",[[2,0,"","n"]]]]"#,
    ),
    (
        r#"{"re":"(?:(?<n>(?:a?))+)+","fl":"n","s":"ab"}"#,
        r#"[[0,1,"a",[[1,0,"","n"]]]]"#,
    ),
    (
        r#"{"re":"^(?<n>a*)+","fl":"n","s":"aa"}"#,
        r#"[[0,2,"aa",[[2,0,"","n"]]]]"#,
    ),
    (
        r#"{"re":"(a*)*","fl":"xn","s":"aaa"}"#,
        r#"[[0,3,"aaa",[[3,0,"",null]]]]"#,
    ),
    (
        r#"{"re":"((?:a|b)?){1,}$","fl":"n","s":"aa"}"#,
        r#"[[0,2,"aa",[[2,0,"",null]]]]"#,
    ),
    (
        r#"{"re":"^(?<n>a*?)*","fl":"xn","s":"a"}"#,
        r#"[[0,1,"a",[[1,0,"","n"]]]]"#,
    ),
    (
        r#"{"re":"x?(?<n>a*)+","fl":"n","s":"a"}"#,
        r#"[[0,1,"a",[[1,0,"","n"]]]]"#,
    ),
    (
        r#"{"re":"(?<n>[ab]?){1,}b?","fl":"xn","s":"ab"}"#,
        r#"[[0,2,"ab",[[2,0,"","n"]]]]"#,
    ),
    (
        r#"{"re":"(((?:a?))+)?","fl":"in","s":"xa"}"#,
        r#"[[1,1,"a",[[1,1,"a",null],[2,0,"",null]]]]"#,
    ),
    (
        r#"{"re":"(?:(\\w*){0,})?","fl":"n","s":"ab"}"#,
        r#"[[0,2,"ab",[[2,0,"",null]]]]"#,
    ),
    (
        r#"{"re":"(?<n>\\w*)*$","fl":"gn","s":"aa"}"#,
        r#"[[0,2,"aa",[[2,0,"","n"]]]]"#,
    ),
    (
        r#"{"re":"x?(?<n>(?:a|b)?){1,}","fl":"n","s":"ab"}"#,
        r#"[[0,2,"ab",[[2,0,"","n"]]]]"#,
    ),
    (
        r#"{"re":"x?(?<n>\\w*){1,}","fl":"n","s":"xa"}"#,
        r#"[[0,2,"xa",[[2,0,"","n"]]]]"#,
    ),
    (
        r#"{"re":"(?:(\\w*)*)+","fl":"xn","s":"aa"}"#,
        r#"[[0,2,"aa",[[2,0,"",null]]]]"#,
    ),
    (
        r#"{"re":"(?:(?x) (a?) +#c\n? +)","fl":"n","s":"a"}"#,
        r#"[[0,1,"a",[[1,0,"",null]]]]"#,
    ),
    (
        r#"{"re":"^((a?)){1,3}","fl":"g","s":"abab"}"#,
        r#"[[0,1,"a",[[1,0,"",null],[1,0,"",null]]]]"#,
    ),
    (
        r#"{"re":"(?<n>a{0,2})*b?","fl":"xn","s":"xa"}"#,
        r#"[[1,1,"a",[[2,0,"","n"]]]]"#,
    ),
    (r#"{"re":"((a??))+$","fl":"xn","s":"aab"}"#, "[]"),
    (
        r#"{"re":"x?((\\w*))+","fl":"in","s":"xa"}"#,
        r#"[[0,2,"xa",[[2,0,"",null],[2,0,"",null]]]]"#,
    ),
    (
        r#"{"re":"(?:((a?))?)+","fl":"gn","s":"aa"}"#,
        r#"[[0,2,"aa",[[2,0,"",null],[2,0,"",null]]]]"#,
    ),
    (
        r#"{"re":"(?<n>a*){2,}$","fl":"xn","s":"aaa"}"#,
        r#"[[0,3,"aaa",[[3,0,"","n"]]]]"#,
    ),
    (r#"{"re":"(a?){1,}$","fl":"n","s":""}"#, "[]"),
    (
        r#"{"re":"((?<n>\\w*){1,})?","fl":null,"s":"abab"}"#,
        r#"[[0,4,"abab",[[0,4,"abab",null],[4,0,"","n"]]]]"#,
    ),
    (
        r#"{"re":"(([ab]?)){2,}$","fl":"n","s":"abab"}"#,
        r#"[[0,4,"abab",[[4,0,"",null],[4,0,"",null]]]]"#,
    ),
    (r#"{"re":"(?<n>(a?))*$","fl":"gn","s":"abab"}"#, "[]"),
    // The engine reads a possessive quantifier as an atomic group, which the rewrite cannot read.
    (r#"{"re":"(?xi) ( a )\n??++","fl":"gn","s":"aaa"}"#, "[]"),
];

#[test]
fn a_repeat_that_can_match_empty_keeps_its_empty_iteration_under_n_7670() -> Result<()> {
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
