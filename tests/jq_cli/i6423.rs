//! #6423: a regex compile error is `Regex failure: <message>` in jq, and through `try ... catch .`
//! that message is a value on stdout. `succinctly jq` said `invalid regex:`, and for a refusal the
//! `regex` crate's parser made it printed the crate's multi-line parse error. Every row was captured
//! from jq 1.7.1 (`/usr/bin/jq -c 'try test(P) catch .' <<< '"a"'`), and jq answers the last group.

use anyhow::Result;

use crate::run_jq_full;

/// What `try test(pattern; flags) catch .` prints for `"a"`, and the exit code.
fn caught(pattern: &str, flags: Option<&str>) -> Result<(String, i32)> {
    let quoted = serde_json::to_string(pattern)?;
    let filter = match flags {
        Some(flags) => format!(r#"try test({quoted}; "{flags}") catch ."#),
        None => format!("try test({quoted}) catch ."),
    };
    let (stdout, stderr, code) = run_jq_full(&["-c", &filter], Some(r#""a""#))?;
    assert_eq!(stderr, "", "{pattern}");
    Ok((stdout.trim_end().to_owned(), code))
}

/// Patterns jq refuses, with the message it gives. Each is caught the same with the `x` flag, which
/// puts the crate's parse behind a `(?x)` prefix the error's span has to look past.
const REFUSED: &[(&str, &str)] = &[
    // The prefix alone used to differ for these.
    ("[", "premature end of char-class"),
    ("[]", "empty char-class"),
    ("*", "target of repeat operator is not specified"),
    ("(?z)", "undefined group option"),
    (r"\", "end pattern at escape"),
    ("[b-a]", "empty range in char class"),
    (r"\1", "invalid backref number/name"),
    ("(?<1a>b)", "invalid group name <1a>"),
    ("[[:foo:]]", "invalid POSIX bracket type"),
    ("[[:ALPHA:]]", "invalid POSIX bracket type"),
    // The crate's parse error, in Oniguruma's words.
    ("(", "end pattern with unmatched parenthesis"),
    ("(a", "end pattern with unmatched parenthesis"),
    ("((a)", "end pattern with unmatched parenthesis"),
    ("(?:", "end pattern with unmatched parenthesis"),
    ("(?<x>", "end pattern with unmatched parenthesis"),
    ("(?=", "end pattern with unmatched parenthesis"),
    ("(?>a", "end pattern with unmatched parenthesis"),
    (r"\p{L", "end pattern with unmatched parenthesis"),
    (")", "unmatched close parenthesis"),
    ("a)", "unmatched close parenthesis"),
    ("(?>a)b)", "unmatched close parenthesis"),
    ("a{2,1}", "upper is smaller than lower in repeat range"),
    ("x{2,1}?", "upper is smaller than lower in repeat range"),
    ("(?", "end pattern in group"),
    ("(?i", "end pattern in group"),
    ("(?#abc", "end pattern in group"),
    ("(?<x", "invalid group name <x>"),
    (r"\k<x", "invalid group name <x>"),
    (r"\k<x>", "undefined name <x> reference"),
    (r"(?<x>a)\k<y>", "undefined name <y> reference"),
    (r"\g<x>", "undefined name <x> reference"),
    (r"\k<1>", "invalid backref number/name"),
    (r"\k<-1>", "invalid backref number/name"),
    (r"\p{Foo}", "invalid character property name {Foo}"),
    (r"\P{Foo}", "invalid character property name {Foo}"),
    (r"\p{^Foo}", "invalid character property name {Foo}"),
    (r"[\p{Foo}]", "invalid character property name {Foo}"),
    (r"\p{}", "invalid character property name {}"),
    (r"\p{In_Foo}", "invalid character property name {In_Foo}"),
    // Past Oniguruma's largest repeat count, whether the crate compiled it or refused its size.
    ("x{100001}", "too big number for repeat range"),
    ("x{100001}y", "too big number for repeat range"),
    ("(x{100001})?", "too big number for repeat range"),
    ("x{200000,}", "too big number for repeat range"),
    ("x{1000,100001}", "too big number for repeat range"),
    ("a{100000,100001}", "too big number for repeat range"),
    ("x{99999999}", "too big number for repeat range"),
    ("a|x{100001}", "too big number for repeat range"),
    // The crate refuses these first, for the order of the counts or their size.
    ("a{100001,5}", "too big number for repeat range"),
    ("a{100001,100000}", "too big number for repeat range"),
    ("x{4294967296}", "too big number for repeat range"),
    ("x{1,4294967296}", "too big number for repeat range"),
    // A name with a recursion level is the group's name; one that starts with a digit is no name.
    (r"\k<x-1>", "undefined name <x> reference"),
    (r"\k<1x>", "invalid group name <1x>"),
];

#[test]
fn a_refused_pattern_is_caught_in_jq_s_words_6423() -> Result<()> {
    for &(pattern, message) in REFUSED {
        for flags in [None, Some("x")] {
            let (stdout, code) = caught(pattern, flags)?;
            assert_eq!(
                (stdout.as_str(), code),
                (format!(r#""Regex failure: {message}""#).as_str(), 0),
                "{pattern} {flags:?}"
            );
        }
    }
    Ok(())
}

#[test]
fn an_uncaught_refusal_exits_5_with_jq_s_message_6423() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(&["-c", r#"test("(a")"#], Some("\"a\"\n"))?;
    assert_eq!((stdout.as_str(), code), ("", 5));
    assert_eq!(
        stderr.trim_end(),
        "jq: error (at <stdin>:1): Regex failure: end pattern with unmatched parenthesis"
    );
    Ok(())
}

/// Counts at Oniguruma's limit, `{,m}` (which jq reads as written, and answers), and braces that
/// are not a repeat are not refused.
#[test]
fn a_count_at_the_limit_or_a_brace_that_is_no_repeat_is_answered_6423() -> Result<()> {
    for (pattern, answer) in [
        ("x{100000}", "false"),
        ("x{0,100000}", "true"),
        ("x{,100001}", "false"),
        ("x{,200000}", "false"),
        (r"x\{100001}", "false"),
        ("[{100001}]", "false"),
        ("a123456", "false"),
    ] {
        assert_eq!(caught(pattern, None)?, (answer.to_owned(), 0), "{pattern}");
    }
    Ok(())
}

/// A name the pattern defines is a reference or a call jq answers, which the crate cannot: it is not
/// worded as an undefined name.
#[test]
fn a_call_to_a_defined_group_keeps_the_crate_s_refusal_6423() -> Result<()> {
    let (stdout, code) = caught(r"(?<a>.)\g<a>", None)?;
    assert_eq!(code, 0);
    assert!(
        stdout.starts_with(r#""Regex failure: regex parse error:"#),
        "{stdout}"
    );
    assert!(!stdout.contains("undefined name"), "{stdout}");
    Ok(())
}

/// A refusal for the crate's size limit has no Oniguruma counterpart and keeps the crate's words.
/// jq answers this pattern (`false`): #6439.
#[test]
fn a_pattern_past_the_crate_s_size_limit_keeps_its_words_6423() -> Result<()> {
    let (stdout, code) = caught(r"\w{100000}", None)?;
    assert_eq!(code, 0);
    assert!(
        stdout.starts_with(r#""Regex failure: Compiled regex exceeds size limit"#),
        "{stdout}"
    );
    Ok(())
}
