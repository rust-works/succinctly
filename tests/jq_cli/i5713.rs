//! #5713: the `$ARGS` object holds a copy of every named argument, so it is built only for a
//! program that names `$ARGS`. A program that does not read it answers as before, and one that
//! does, in any position, still sees the same object. Outputs were captured from jq 1.7.1
//! (`/usr/bin/jq`).

use anyhow::Result;

use crate::run_jq_full;

#[test]
fn programs_that_read_args_and_programs_that_do_not_answer_as_jq_5713() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let raw = dir.path().join("r.txt");
    std::fs::write(&raw, "hello\n")?;
    let raw = raw.to_str().unwrap();
    for (filter, expected) in [
        (
            "$ARGS",
            "{\"positional\":[\"p\"],\"named\":{\"a\":\"1\",\"b\":[1,2],\"r\":\"hello\\n\"}}\n",
        ),
        ("$ARGS.named | keys", "[\"a\",\"b\",\"r\"]\n"),
        ("$ARGS.positional", "[\"p\"]\n"),
        ("def f: $ARGS.named.a; f", "\"1\"\n"),
        ("[limit(1; $ARGS.named.b[])]", "[1]\n"),
        ("\"v=\\($ARGS.positional)\"", "\"v=[\\\"p\\\"]\"\n"),
        (". as $ARGS | $ARGS", "null\n"),
        ("$a", "\"1\"\n"),
        ("$b | length", "2\n"),
        ("$r | length", "6\n"),
        ("[$a, $r]", "[\"1\",\"hello\\n\"]\n"),
        ("1", "1\n"),
    ] {
        let (stdout, stderr, code) = run_jq_full(
            &[
                "-nc",
                "--arg",
                "a",
                "1",
                "--argjson",
                "b",
                "[1,2]",
                "--rawfile",
                "r",
                raw,
                filter,
                "--args",
                "p",
            ],
            None,
        )?;
        assert_eq!(code, 0, "{filter}: {stderr:?}");
        assert_eq!(stdout, expected, "{filter}");
    }
    Ok(())
}
