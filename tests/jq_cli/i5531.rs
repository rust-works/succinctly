//! #5531: the regex builtins now keep the compiled pattern instead of building it again for every
//! input value. The answers must not move: a pattern that is the same for every input, one that
//! changes per input (more distinct ones than the cache holds), a refused pattern and a bad
//! modifier string all answer as before, for every builtin that compiles a pattern.

use anyhow::Result;

use crate::run_jq_full;

fn run(filter: &str, input: &str) -> Result<(String, String, i32)> {
    run_jq_full(&["-c", filter], Some(input))
}

#[test]
fn test_a_repeated_pattern_answers_the_same_for_every_input_5531() -> Result<()> {
    let input = r#"["item1","item22","x","item333","item4a"]"#;
    for (filter, expected) in [
        (
            r#"[.[] | test("[a-z]+[0-9]{2,}")]"#,
            "[false,true,false,true,false]",
        ),
        (
            r#"[.[] | [match("[0-9]+";"g").string]]"#,
            r#"[["1"],["22"],[],["333"],["4"]]"#,
        ),
        (
            r#"[.[] | capture("(?<n>[0-9]+)")?]"#,
            r#"[{"n":"1"},{"n":"22"},{"n":"333"},{"n":"4"}]"#,
        ),
        (
            r##"[.[] | sub("[0-9]";"#")]"##,
            r#"["item#","item#2","x","item#33","item#a"]"#,
        ),
        (
            r##"[.[] | gsub("[0-9]";"#")]"##,
            r#"["item#","item##","x","item###","item#a"]"#,
        ),
        (r#"[.[] | [scan("[0-9]")] | length]"#, "[1,2,0,3,1]"),
        (
            r#"[.[] | split("[0-9]+";null)]"#,
            r#"[["item",""],["item",""],["x"],["item",""],["item","a"]]"#,
        ),
        (
            r#"[.[] | [splits("[0-9]+")]]"#,
            r#"[["item",""],["item",""],["x"],["item",""],["item","a"]]"#,
        ),
        (r#"[.[] | test("ITEM";"i")]"#, "[true,true,false,true,true]"),
    ] {
        let (stdout, stderr, code) = run(filter, input)?;
        assert_eq!(stdout.trim_end(), expected, "{filter}: stderr={stderr:?}");
        assert_eq!(code, 0, "{filter}");
    }
    Ok(())
}

#[test]
fn test_a_pattern_that_changes_per_input_answers_past_the_cache_size_5531() -> Result<()> {
    // 40 distinct patterns, each used twice in a row and then again after all the others: more
    // than the cache holds, so entries are evicted and rebuilt.
    let (stdout, stderr, code) = run_jq_full(
        &[
            "-nc",
            r#"[range(40) as $i | ("item\($i)" | test("^item\($i)$"), test("^item\($i + 1)$"))] | [length, (map(select(.)) | length)]"#,
        ],
        None,
    )?;
    assert_eq!(stdout.trim_end(), "[80,40]", "stderr={stderr:?}");
    assert_eq!(code, 0);

    let (stdout, stderr, code) = run_jq_full(
        &[
            "-nc",
            r#"[range(40) as $i | range(40) as $j | ("v\($j)" | test("^v\($i)$"))] | map(select(.)) | length"#,
        ],
        None,
    )?;
    assert_eq!(stdout.trim_end(), "40", "stderr={stderr:?}");
    assert_eq!(code, 0);
    Ok(())
}

#[test]
fn test_a_refused_pattern_is_reported_for_every_input_5531() -> Result<()> {
    // The refusal is a value `try` sees as an error, once per input, not once overall.
    let (stdout, stderr, code) = run(
        r#"[.[] | try test("(?P<n>a)") catch "refused"]"#,
        r#"["a","b","c"]"#,
    )?;
    assert_eq!(
        stdout.trim_end(),
        r#"["refused","refused","refused"]"#,
        "stderr={stderr:?}"
    );
    assert_eq!(code, 0);

    let (stdout, stderr, code) = run(
        r#"[.[] | try test("a";"z") catch "bad flag"]"#,
        r#"["a","b"]"#,
    )?;
    assert_eq!(
        stdout.trim_end(),
        r#"["bad flag","bad flag"]"#,
        "stderr={stderr:?}"
    );
    assert_eq!(code, 0);

    // And uncaught it is still the exit-5 error, naming the pattern problem.
    let (stdout, stderr, code) = run(r#".[] | test("(?P<n>a)")"#, r#"["a","b"]"#)?;
    assert_eq!(code, 5, "stdout={stdout:?} stderr={stderr:?}");
    assert!(stderr.contains("undefined group option"), "{stderr:?}");
    Ok(())
}

#[test]
fn test_the_same_pattern_with_different_flags_is_not_shared_5531() -> Result<()> {
    let (stdout, stderr, code) = run(
        r#"[.[] | test("abc"), test("abc";"i"), test("abc";"x"), test("ab c";"x")]"#,
        r#"["ABC","abc"]"#,
    )?;
    assert_eq!(
        stdout.trim_end(),
        "[false,true,false,false,true,true,true,true]",
        "stderr={stderr:?}"
    );
    assert_eq!(code, 0);
    Ok(())
}
