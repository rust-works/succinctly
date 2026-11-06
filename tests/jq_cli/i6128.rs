//! #6128: a lazy repeat that must run twice or more ends at its body's empty way instead of
//! trying the later copies first, as #6106 did for the greedy ones: `"bab" |
//! match("(?:(?:b|c)||[ab]){2,3}?"; "gn")` is `"b"`, `"ab"` in jq, and was `"b"`, `"a"`, `"b"`.
//! Every row was captured from jq 1.7.1 (`/usr/bin/jq -nc`).

use anyhow::Result;

use crate::run_jq_full;

#[test]
fn test_a_lazy_counted_repeat_ends_at_its_empty_way_6128() -> Result<()> {
    for (filter, expected) in [
        (
            r#""bab" | [match("(?:(?:b|c)||[ab]){2,3}?"; "gn") | .string]"#,
            r#"["b","ab"]"#,
        ),
        (
            r#""bab" | [match("(?:(?:b|c)||[ab]){2}?"; "gn") | .string]"#,
            r#"["b","ab"]"#,
        ),
        (
            r#""bab" | [match("(?:(?:b|c)||[ab]){2,}?"; "gn") | .string]"#,
            r#"["b","ab"]"#,
        ),
        (
            r#""bab" | [match("(?:(?:b|c)||[ab]){0,2}?"; "gn") | .string]"#,
            r#"["b","a","b"]"#,
        ),
        (
            r#""bab" | [match("(?:(?:b|c)||[ab]){1,3}?"; "gn") | .string]"#,
            r#"["b","a","b"]"#,
        ),
        (
            r#""ba" | [match("(?:(?:a|)|(?:b|c)){2,3}?"; "gn") | .string]"#,
            r#"["ba"]"#,
        ),
        (
            r#""bab" | [match("(?:(?:b|c)||[ab]){2,3}?"; "g") | .string]"#,
            r#"["b","","b",""]"#,
        ),
        (
            r#""ba" | [match("(?:(?:a|)|[ab]){3,}?"; "gn") | .string]"#,
            r#"["ba"]"#,
        ),
        (
            r#""aabbcc" | [match("(?:(?:a|)[ab]|(?:a|)|\\w){3,}?"; "gn") | .string]"#,
            r#"["aabb","c","c"]"#,
        ),
        (
            r#""bab" | [match("(?:b*|a){2,}?"; "gn") | .string]"#,
            r#"["b","ab"]"#,
        ),
        (
            r#""babab" | [match("(?:(?:b|c)||[ab]){2,6}?"; "gn") | .string]"#,
            r#"["b","ab","ab"]"#,
        ),
        (
            r#""babab" | [match("(?:b*|a){2,6}?"; "gn") | .string]"#,
            r#"["b","ab","ab"]"#,
        ),
    ] {
        let (stdout, stderr, code) = run_jq_full(&["-nc", filter], None)?;
        assert_eq!(
            (stdout.trim_end(), code),
            (expected, 0),
            "{filter}: stderr {stderr}"
        );
    }
    Ok(())
}

/// An unbounded `{n,}?` (n of 2 or more) over a body with a consuming way before its empty one.
/// Oniguruma writes a repeat out as plain copies of its body when the body's compiled length
/// times the count is at most ten, and the empty way then does not end the loop: `(?:b?|\w)`
/// is five operations, so `{2,}?` is plain copies and `{3,}?` is a counted loop that leaves at
/// the empty way. The rewrite is for the counted loop only. Rows from jq 1.7.1.
#[test]
fn test_an_unbounded_lazy_repeat_over_a_leading_consuming_way_is_unrolled_by_size_6128(
) -> Result<()> {
    for (filter, expected) in [
        // Two copies of five operations: unrolled, the plain copies the `regex` crate reads.
        (
            r#""babab" | [match("(?:b?|\\w){2,}?"; "gn") | .string]"#,
            r#"["b","a","b","a","b"]"#,
        ),
        (
            r#""ab" | [match("(?:b?|\\w){2,}?"; "gn") | .string]"#,
            r#"["a","b"]"#,
        ),
        (
            r#""abb" | [match("(?:b?|a){2,}?"; "gn") | .string]"#,
            r#"["a","bb"]"#,
        ),
        (
            r#""abab" | [match("(?:b?|ab){2,}?"; "gn") | .string]"#,
            r#"["ab","ab"]"#,
        ),
        // Three copies make fifteen: a counted loop, which leaves at the empty way.
        (
            r#""abbb" | [match("(?:b?|a){3,}?"; "gn") | .string]"#,
            r#"["abb","b"]"#,
        ),
        (
            r#""ab" | [match("(?:b?|\\w){4,}?"; "gn") | .string]"#,
            r#"["ab"]"#,
        ),
        // A count past the old size limit of the rewrite, over a body that is a Unicode class.
        (
            r#""abababab" | [match("(?:b*|\\w){6,}?"; "gn") | .string]"#,
            r#"["ab","ab","ab","ab"]"#,
        ),
        // Under an ignore-case flag a letter that starts no fold is still one operation.
        (
            r#""ab" | [match("(?i)(?:b?|.){2,}?"; "gn") | .string]"#,
            r#"["a","b"]"#,
        ),
        // Without `n` the two forms agree.
        (
            r#""abb" | [match("(?:b?|\\w){2,}?"; "g") | .string]"#,
            r#"["","bb",""]"#,
        ),
    ] {
        let (stdout, stderr, code) = run_jq_full(&["-nc", filter], None)?;
        assert_eq!(
            (stdout.trim_end(), code),
            (expected, 0),
            "{filter}: stderr {stderr}"
        );
    }
    Ok(())
}

/// A pattern with syntax only `fancy-regex` reads (lookaround, an atomic group, a backreference,
/// `\K`, `\G`, a comment) used to keep its loops as written, which `fancy-regex` reads as the
/// `regex` crate does: `"ba" | match("(?:b*|a)+(?=)")` was `"ba"`. Its loops now end at the empty
/// iteration too, and so do those inside a lookaround or atomic group.
#[test]
fn test_a_loop_beside_fancy_regex_syntax_ends_at_its_empty_iteration_6128() -> Result<()> {
    for (filter, expected) in [
        (r#""ba" | [match("(?:b*|a)+(?=)") | .string]"#, r#"["b"]"#),
        (r#""ba" | [match("(?=b)(?:b*|a)+") | .string]"#, r#"["b"]"#),
        (
            r#""xba" | [match("(?<=x)(?:b*|a)+") | .string]"#,
            r#"["b"]"#,
        ),
        (r#""ba" | [match("(?#c)(?:b*|a)+") | .string]"#, r#"["b"]"#),
        (r#""ba" | [match("\\G(?:b*|a)+") | .string]"#, r#"["b"]"#),
        (r#""xba" | [match("x\\K(?:b*|a)+") | .string]"#, r#"["b"]"#),
        (r#""ba" | [match("(?>(?:b*|a)+)") | .string]"#, r#"["b"]"#),
        (
            r#""bba" | [match("(b)\\1(?:b*|a)+") | .string]"#,
            r#"["bb"]"#,
        ),
        (
            r#""xba" | [match("(?<x>x)\\k<x>?(?:b*|a)+") | .string]"#,
            r#"["xb"]"#,
        ),
        (
            r#""ba" | [match("(?:b*|a)+(?=)"; "g") | .string]"#,
            r#"["b","",""]"#,
        ),
        (r#""ba" | sub("(?:b*|a)+(?=)"; "X")"#, r#""Xa""#),
        // A comment in the body matches nothing; under `x`, a `(?#` inside a `#` comment is text.
        (r#""ba" | [match("(?:(?#c)b*|a)+") | .string]"#, r#"["b"]"#),
        (
            r#""ba" | [match("(?:b*|a)+ # note (?#"; "x") | .string]"#,
            r#"["b"]"#,
        ),
        // A loop whose body holds such syntax is still as written, and agrees here.
        (r#""ba" | [match("(?:(?=b)b*|a)+") | .string]"#, r#"["ba"]"#),
    ] {
        let (stdout, stderr, code) = run_jq_full(&["-nc", filter], None)?;
        assert_eq!(
            (stdout.trim_end(), code),
            (expected, 0),
            "{filter}: stderr {stderr}"
        );
    }
    Ok(())
}

/// A quantifier after a comment has no target in jq (`(?#)?` is refused), so a loop over one is not
/// rewritten into a pattern the `regex` crate would accept.
#[test]
fn test_a_quantified_comment_in_a_loop_body_is_still_refused_6128() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(
        &["-nc", r#""bab" | [match("(?:(?#)?|b)+"; "g") | .string]"#],
        None,
    )?;
    assert_eq!((stdout.as_str(), code), ("", 5), "stderr {stderr}");
    assert!(stderr.contains("Regex failure"), "{stderr}");
    Ok(())
}
