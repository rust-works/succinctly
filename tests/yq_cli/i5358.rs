//! #5358: under `--eval-all`, yq's `[...]` evaluates its body read-only, so an absent map key
//! yields nothing instead of `null` (`collectTogether` / `SingleReadonlyChildContext`).
//! Rows captured from yq v4.49.2 (`yq ea -o=json -I=0`); the pinned v4.53.3 was not available.

use anyhow::Result;

use crate::run_yq_stdin;

const EA: &[&str] = &["--eval-all", "-o=json", "-I=0"];

#[test]
fn test_eval_all_collect_drops_an_absent_key_5358() -> Result<()> {
    for (filter, input, expected) in [
        ("[.a]", "a: 1\n---\nb: 2\n", "[1]\n"),
        ("[.a]", "b: 2\n", "[]\n"),
        ("[.a|type]", "b: 2\n", "[]\n"),
        ("[.x.y]", "b: 2\n", "[]\n"),
        ("[[.a]]", "b: 2\n", "[[]]\n"),
        ("[.a // null]", "b: 2\n", "[null]\n"),
        ("[.a, null]", "b: 2\n", "[null]\n"),
        ("[.a]", "a: ~\n", "[null]\n"),
    ] {
        let (stdout, code) = run_yq_stdin(filter, input, EA)?;
        assert_eq!(
            (stdout.as_str(), code),
            (expected, 0),
            "{filter} over {input:?}"
        );
    }
    Ok(())
}

#[test]
fn test_plain_collect_still_keeps_the_absent_key_as_null_5358() -> Result<()> {
    let (stdout, code) = run_yq_stdin("[.a]", "b: 2\n", &["-o=json", "-I=0"])?;
    assert_eq!((stdout.as_str(), code), ("[null]\n", 0));
    Ok(())
}
