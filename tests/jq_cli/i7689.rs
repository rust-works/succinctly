//! #7689: four regex shapes the quantifier fuzz of #7654, #7669, #7670 and #7682 left differing
//! from jq, and the shapes the same fuzz found beside them, each captured from jq 1.7.1
//! (`/usr/bin/jq`) and the same in jq 1.8.2.
//!
//! - Under `n`, an alternative that can only match empty (`(a){0}`, `[^\s\S]?`) hid a later
//!   alternative: `regex-syntax` gives a pattern with a part that never matches no minimum length,
//!   so the pattern was taken for one that cannot match empty and kept the `regex` crate's route.
//! - A repeated group whose body is a `?` or `*` repeat (`(a?)+`, `(a*)*`) was merged with its
//!   repeat by `fancy-regex`'s optimizer, so under `l` and `n` the group spanned every iteration
//!   (`(a?)+` at `[0,3]` on `"aaa"`) instead of the final, empty one.
//! - A `{1}` marked lazy or possessive beside another quantifier (`(a){1}?++(a)`) was refused or
//!   left a comment for the `regex` crate; a `{1}` on a plain group of characters (`(?:ab){1}*`)
//!   leaves Oniguruma's string node, whose last character the next quantifier repeats; and a count
//!   of zero after a gap, a `{1}` or before another count lost its capture entry.
//!
//! The rows are a sample of the rows these changes fixed (46 of 52 fail before them); the last
//! rows are controls that already agreed.

use anyhow::Result;

use crate::run_jq_full;

/// Each row's matches: offset, length, text and captures, or the caught error.
const PROGRAM: &str = r#". as $r | $r.s | try ([if $r.fl == null then match($r.re) else match($r.re; $r.fl) end | [.offset, .length, .string, [.captures[] | [.offset, .length, .string, .name]]]]) catch "ERR: \(.)""#;

/// `(row, jq 1.7.1's output)`: `re` the pattern, `fl` the flags (`null` for `match/1`), `s` the input.
const ROWS: &[(&str, &str)] = &[
    (
        r#"{"re":"^(a|b){0}+|.+?(b)","fl":"n","s":"bb"}"#,
        r#"[[0,2,"bb",[[-1,0,null,null],[1,1,"b",null]]]]"#,
    ),
    (
        r#"{"re":"(?x)(b*|a){0}|(a*)b?","fl":"gn","s":"aab"}"#,
        r#"[[0,3,"aab",[[-1,0,null,null],[0,2,"aa",null]]]]"#,
    ),
    (
        r#"{"re":"(?x)(a|b){1,2} {0}+|b","fl":"n","s":"abb"}"#,
        r#"[[1,1,"b",[[-1,0,null,null]]]]"#,
    ),
    (
        r#"{"re":"(?i)(a){2}?{2,}+{0}|b","fl":"n","s":"bab"}"#,
        r#"[[0,1,"b",[[-1,0,null,null]]]]"#,
    ),
    (
        r#"{"re":"(?x)(|a){0}|(a)?{2,}(a)","fl":"n","s":"abab"}"#,
        r#"[[0,1,"a",[[-1,0,null,null],[-1,0,null,null],[0,1,"a",null]]]]"#,
    ),
    (
        r#"{"re":"(?x)(a|b){0}+(?#c)??+|b","fl":"gn","s":"ba"}"#,
        r#"[[0,1,"b",[[-1,0,null,null]]]]"#,
    ),
    (
        r#"{"re":"(?:b|)(a?){0}|(b*|a)|","fl":"n","s":"aaab"}"#,
        r#"[[0,1,"a",[[-1,0,null,null],[0,1,"a",null]]]]"#,
    ),
    (
        r#"{"re":"^(b*|a){2,}+{0}|b","fl":"n","s":"bb"}"#,
        r#"[[0,1,"b",[[-1,0,null,null]]]]"#,
    ),
    (
        r#"{"re":"(?x)(b*|a){0}|(a?)|","fl":"xn","s":"ba"}"#,
        r#"[[1,1,"a",[[-1,0,null,null],[1,1,"a",null]]]]"#,
    ),
    (
        r#"{"re":"(?x)(a?)+|b","fl":"ln","s":"a"}"#,
        r#"[[0,1,"a",[[1,0,"",null]]]]"#,
    ),
    (
        r#"{"re":"(?x)(a?)\t++b","fl":"gl","s":"bab"}"#,
        r#"[[1,2,"ab",[[2,0,"",null]]]]"#,
    ),
    (
        r#"{"re":"^(a?)++|(a)+(b)","fl":"gl","s":"aaa"}"#,
        r#"[[0,3,"aaa",[[3,0,"",null],[-1,0,null,null],[-1,0,null,null]]]]"#,
    ),
    (
        r#"{"re":"(?x)(a?)+|(a?){1,2}(b)","fl":"ln","s":"a"}"#,
        r#"[[0,1,"a",[[1,0,"",null],[-1,0,null,null],[-1,0,null,null]]]]"#,
    ),
    (
        r#"{"re":"(a?)+|a{1}b","fl":"ln","s":"aab"}"#,
        r#"[[0,2,"aa",[[2,0,"",null]]]]"#,
    ),
    (
        r#"{"re":"^(a?)+*$","fl":"l","s":"aa"}"#,
        r#"[[0,2,"aa",[[2,0,"",null]]]]"#,
    ),
    (
        r#"{"re":"b(a?)++?b?","fl":"ln","s":"bab"}"#,
        r#"[[0,3,"bab",[[2,0,"",null]]]]"#,
    ),
    (
        r#"{"re":"(?:b|)(a*)++|[ab]+?\\z","fl":"gn","s":"aAa"}"#,
        r#"[[0,1,"a",[[1,0,"",null]]],[2,1,"a",[[3,0,"",null]]]]"#,
    ),
    (
        r#"{"re":"(?x)(a*)*+|(a*)a","fl":"xn","s":"ba"}"#,
        r#"[[1,1,"a",[[2,0,"",null],[-1,0,null,null]]]]"#,
    ),
    (
        r#"{"re":"(?i)(a?)++|(?:ab){2,}?\\z","fl":"gn","s":"bab"}"#,
        r#"[[1,1,"a",[[2,0,"",null]]]]"#,
    ),
    (
        r#"{"re":"^(a*)*+\t?+b?","fl":"n","s":"aab"}"#,
        r#"[[0,3,"aab",[[2,0,"",null]]]]"#,
    ),
    (
        r#"{"re":"(?i)(a?)++|a(b)","fl":"gn","s":"aaab"}"#,
        r#"[[0,3,"aaa",[[3,0,"",null],[-1,0,null,null]]]]"#,
    ),
    (
        r#"{"re":"(?:b|)(a*)*+|.","fl":"n","s":"aab"}"#,
        r#"[[0,2,"aa",[[2,0,"",null]]]]"#,
    ),
    (
        r#"{"re":"(?:b|)(a?)++|b","fl":"n","s":"aaa"}"#,
        r#"[[0,3,"aaa",[[3,0,"",null]]]]"#,
    ),
    (
        r#"{"re":"b(a*)*+|(|a){1,2}+(a)","fl":"gn","s":"abab"}"#,
        r#"[[0,1,"a",[[-1,0,null,null],[0,0,"",null],[0,1,"a",null]]],[1,2,"ba",[[3,0,"",null],[-1,0,null,null],[-1,0,null,null]]],[3,1,"b",[[4,0,"",null],[-1,0,null,null],[-1,0,null,null]]]]"#,
    ),
    (
        r#"{"re":"^(a*)*+|(?:a|){,2}+","fl":"xn","s":"aa"}"#,
        r#"[[0,2,"aa",[[2,0,"",null]]]]"#,
    ),
    (
        r#"{"re":"b(a){1}? {0}{1,2}|b","fl":"x","s":"bab"}"#,
        r#"[[0,1,"b",[[-1,0,null,null]]]]"#,
    ),
    (r#"{"re":"(?#c).{1,2} {1}?++b","fl":"gn","s":"a"}"#, "[]"),
    (
        r#"{"re":"(?i)(|a){2,}(?#c){1}?\\z","fl":"x","s":"aaa"}"#,
        r#"[[0,3,"aaa",[[3,0,"",null]]]]"#,
    ),
    (r#"{"re":"^a{,2}? {1}?++(a)","fl":null,"s":"aa"}"#, "[]"),
    (
        r#"{"re":"(?x)(a|b){1}?{0}+b?","fl":null,"s":"b"}"#,
        r#"[[0,1,"b",[[-1,0,null,null]]]]"#,
    ),
    (
        r#"{"re":"(?#c)(a?){1}?+?+|","fl":"ln","s":"aaa"}"#,
        r#"[[0,3,"aaa",[[3,0,"",null]]]]"#,
    ),
    (
        r#"{"re":"(?x)(a?)*{1}++|b","fl":"gl","s":"aab"}"#,
        r#"[[0,2,"aa",[[2,0,"",null]]],[2,1,"b",[[-1,0,null,null]]],[3,0,"",[[3,0,"",null]]]]"#,
    ),
    (
        r#"{"re":"a[ab]{1}?++|","fl":"g","s":"b"}"#,
        r#"[[0,0,"",[]],[1,0,"",[]]]"#,
    ),
    (
        r#"{"re":"(?x)(?:ab){1} *++a","fl":"gl","s":"aaab"}"#,
        r#"[[0,2,"aa",[]]]"#,
    ),
    (r#"{"re":"(?#c)(?:ab){1}???*\\z","fl":"l","s":"bb"}"#, "[]"),
    (
        r#"{"re":"a(?:ab){1}??(a)","fl":"gn","s":"aaab"}"#,
        r#"[[0,3,"aaa",[[2,1,"a",null]]]]"#,
    ),
    (r#"{"re":"a(?:ab){1}{0,1}(b)","fl":"x","s":"abab"}"#, "[]"),
    (
        r#"{"re":"(?i)(?:ab){1}?*?$","fl":null,"s":"A"}"#,
        r#"[[0,1,"A",[]]]"#,
    ),
    (
        r#"{"re":"(?#c)(?:ab){1}{0}+{0,1}|b","fl":"g","s":"abb"}"#,
        r#"[[0,1,"a",[]],[1,1,"b",[]],[2,1,"b",[]]]"#,
    ),
    (r#"{"re":"^(?:ab){1}(?#c)**?(b)","fl":"n","s":"bab"}"#, "[]"),
    (
        r#"{"re":"(?x)(?:ab){1} {0}+|","fl":"xn","s":"a"}"#,
        r#"[[0,1,"a",[]]]"#,
    ),
    (
        r#"{"re":"(?#c)(?:ab){1}#c\n??+","fl":"x","s":"aaa"}"#,
        r#"[[0,1,"a",[]]]"#,
    ),
    (
        r#"{"re":"(?x)(|a){0}#c\n{2,}?b?","fl":null,"s":"b"}"#,
        r#"[[0,1,"b",[[-1,0,null,null]]]]"#,
    ),
    (
        r#"{"re":"(?x)(|a){1}#c\n{0}+b","fl":null,"s":"bb"}"#,
        r#"[[0,1,"b",[[-1,0,null,null]]]]"#,
    ),
    (
        r#"{"re":"b(b*|a){1} {0}+$","fl":"x","s":"bb"}"#,
        r#"[[1,1,"b",[[-1,0,null,null]]]]"#,
    ),
    (
        r#"{"re":"(?x)(a|b){1} {0}b?","fl":"x","s":"bb"}"#,
        r#"[[0,1,"b",[[-1,0,null,null]]]]"#,
    ),
    (
        r#"{"re":"(a?)+","fl":null,"s":"aaa"}"#,
        r#"[[0,3,"aaa",[[3,0,"",null]]]]"#,
    ),
    (
        r#"{"re":"(a+)+","fl":"l","s":"aaa"}"#,
        r#"[[0,3,"aaa",[[0,3,"aaa",null]]]]"#,
    ),
    (r#"{"re":"x?|b","fl":"n","s":"ab"}"#, r#"[[1,1,"b",[]]]"#),
    (r#"{"re":"(a)??++","fl":"gn","s":"aaa"}"#, "[]"),
    (
        r#"{"re":"a{1}+","fl":"g","s":"aaab"}"#,
        r#"[[0,1,"a",[]],[1,1,"a",[]],[2,1,"a",[]]]"#,
    ),
    (
        r#"{"re":"(?:a\\x62){1}+","fl":"g","s":"abbbab"}"#,
        r#"[[0,2,"ab",[]],[4,2,"ab",[]]]"#,
    ),
];

#[test]
fn quantifier_fuzz_rows_answer_as_jq_7689() -> Result<()> {
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

fn jq_n(program: &str) -> Result<(String, String, i32)> {
    run_jq_full(&["-nc", program], None)
}

/// Shape A of the issue: jq 1.7.1 and 1.8.2 print `1`; this printed `0`.
#[test]
fn an_alternative_that_only_matches_empty_does_not_hide_a_later_one_under_n_7689() -> Result<()> {
    for program in [
        r#""ab" | [match("(a){0}|b"; "n")] | length"#,
        r#""ab" | [match("[^\\s\\S]?|b"; "n")] | length"#,
    ] {
        assert_eq!(
            jq_n(program)?,
            ("1\n".to_string(), String::new(), 0),
            "{program}"
        );
    }
    Ok(())
}

/// Shape B: jq 1.7.1 and 1.8.2 print `[3,0]`; this printed `[0,3]`, a span `(a?)` cannot match.
#[test]
fn a_group_in_a_repeat_keeps_its_last_iteration_under_l_7689() -> Result<()> {
    let out = jq_n(r#""aaa" | match("(a?)+"; "l").captures[0] | [.offset,.length]"#)?;
    assert_eq!(out, ("[3,0]\n".to_string(), String::new(), 0));
    Ok(())
}

/// Shape C: jq 1.7.1 and 1.8.2 print `[2,0]`; this printed `[1,1]`.
#[test]
fn a_group_in_a_possessive_repeat_keeps_its_empty_iteration_under_n_7689() -> Result<()> {
    let out = jq_n(r#""ba" | match("(a?)++$"; "n").captures[0] | [.offset,.length]"#)?;
    assert_eq!(out, ("[2,0]\n".to_string(), String::new(), 0));
    Ok(())
}

/// Shape D answered as jq on `main` already (a possessive interval after a comment, #7682); kept as
/// a control, with the `x` flag. jq 1.7.1 and 1.8.2 print `1`.
#[test]
fn a_possessive_interval_after_a_comment_matches_7689() -> Result<()> {
    for program in [
        r#""abb" | [match("a(?#c)b{2}++")] | length"#,
        r#""abb" | [match("a #c\nb{2}++"; "x")] | length"#,
    ] {
        assert_eq!(
            jq_n(program)?,
            ("1\n".to_string(), String::new(), 0),
            "{program}"
        );
    }
    Ok(())
}

/// Under `l`, a repeated group whose body is a `*` repeat answers on 31 characters, where the
/// earlier engine's longest-match search over the unmerged loop ran out of budget; jq 1.7.1 and
/// 1.8.2 print `false`. The group ends at its last, empty iteration (`[3,0]` in jq 1.7.1 and
/// 1.8.2), where the earlier engine kept its merged loop's last non-empty one (`[0,3]`), a recorded
/// divergence that now matches jq 1.7.1 (#7965).
#[test]
fn a_star_body_under_l_ends_at_the_empty_iteration_7689() -> Result<()> {
    let out = jq_n(r#"("a" * 31 + "!") | test("(a*)*b"; "l")"#)?;
    assert_eq!(out, ("false\n".to_string(), String::new(), 0));
    let out = jq_n(r#""aaa" | match("(a*)*"; "l").captures[0] | [.offset,.length]"#)?;
    assert_eq!(out, ("[3,0]\n".to_string(), String::new(), 0));
    Ok(())
}

/// A possessive repeat is an atomic group, which sent the pattern to `fancy-regex` on the earlier
/// engine. There its group kept the last non-empty iteration, a loop inside it did not end at its
/// empty iteration, and `l` read past the atomic group's first way; and under `x`, `(?:a b){1}*`
/// did not repeat the group's last character as Oniguruma does. These were recorded divergences
/// and now match jq 1.7.1 and 1.8.2 (#7965).
#[test]
fn possessive_repeats_answer_as_jq_does_7689() -> Result<()> {
    for (program, here) in [
        (
            r#""aa" | match("(a*)*+$").captures[0] | [.offset,.length]"#,
            "[2,0]\n",
        ),
        (
            r#""ba" | match("(?:b|)(b*|a)++$") | [.offset,.length]"#,
            "[2,0]\n",
        ),
        (
            r#""aaab" | match("a(|a){1,2}+"; "l") | [.offset,.length]"#,
            "[0,1]\n",
        ),
        (
            r#""abbbab" | [match("(?:a b){1}*"; "gx") | .string]"#,
            "[\"abbb\",\"ab\"]\n",
        ),
    ] {
        assert_eq!(
            jq_n(program)?,
            (here.to_string(), String::new(), 0),
            "{program}"
        );
    }
    Ok(())
}
