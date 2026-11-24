//! #7881: since #7712 every `--argjson` text and `--jsonargs` word was parsed twice, once where
//! jq's option loop checks it and once to bind it. The value the check builds is now the value
//! bound, so these rows pin that nothing a program can see moved: number literals keep their
//! spelling, each text has a value of its own (two `nan`s from two texts are not one value),
//! a name bound earlier skips its text unread, and a text that is not JSON is still refused
//! where it stands.
//!
//! Every row was captured from `/usr/bin/jq` (1.7.1) with `argv[0]` `jq`; jq 1.8.2
//! (`/opt/homebrew/bin/jq`) gives the same output and exit status on each (with its own URL in
//! the usage hint).

use anyhow::Result;

use crate::run_jq_full;

const NOT_JSON_ARGJSON: &str = "jq: invalid JSON text passed to --argjson\nUse jq --help for help with command-line options,\nor see the jq manpage, or online docs  at https://jqlang.github.io/jq\n";
const NOT_JSON_JSONARGS: &str = "jq: invalid JSON text passed to --jsonargs\nUse jq --help for help with command-line options,\nor see the jq manpage, or online docs  at https://jqlang.github.io/jq\n";

#[test]
fn test_7881_a_checked_text_binds_the_value_jq_binds() -> Result<()> {
    let mut failures = Vec::new();
    let rows: [(&[&str], &str); 7] = [
        (
            &[
                "-nc",
                "--argjson",
                "a",
                "1.000",
                "--argjson",
                "b",
                "1e1000",
                "--argjson",
                "c",
                "100000000000000000000",
                "[$a,$b,$c]",
            ],
            "[1.000,1E+1000,100000000000000000000]\n",
        ),
        (
            &[
                "-nc",
                "$ARGS",
                "--jsonargs",
                "1.000",
                r#"{"a":[1.50,"x"]}"#,
                "--args",
                "1.000",
                "--jsonargs",
                "null",
                "1e1000",
            ],
            "{\"positional\":[1.000,{\"a\":[1.50,\"x\"]},\"1.000\",null,1E+1000],\"named\":{}}\n",
        ),
        // Two texts are two values, the same text or not: a `nan` equals itself and no other.
        (
            &[
                "-nc",
                "--argjson",
                "a",
                "[nan]",
                "--argjson",
                "b",
                "[nan]",
                "[$a,$b,($a==$a),($a==$b),($a<$a)]",
            ],
            "[[null],[null],true,false,true]\n",
        ),
        // One text four times, as two named values and two words.
        (
            &[
                "-nc",
                "--argjson",
                "a",
                r#"{"x":[1.0]}"#,
                "--argjson",
                "b",
                r#"{"x":[1.0]}"#,
                "[$a,$b,($a==$b),$ARGS.positional]",
                "--jsonargs",
                r#"{"x":[1.0]}"#,
                r#"{"x":[1.0]}"#,
            ],
            "[{\"x\":[1.0]},{\"x\":[1.0]},true,[{\"x\":[1.0]},{\"x\":[1.0]}]]\n",
        ),
        // A name bound earlier skips its text unread, and the text after it is still its own.
        (
            &[
                "-nc",
                "--arg",
                "a",
                "x",
                "--argjson",
                "a",
                "{bad",
                "--argjson",
                "b",
                " [1 , 2.0] ",
                "[$a,$b]",
            ],
            "[\"x\",[1,2.0]]\n",
        ),
        (
            &[
                "-nc",
                "--argjson",
                "a",
                r#"{"k":1,"k":2.0}"#,
                "$a",
                "--jsonargs",
                r#""😀""#,
            ],
            "{\"k\":2.0}\n",
        ),
        (
            &[
                "-nc",
                "--argjson",
                "a",
                "[1.10]",
                "$a,$ARGS.named,($a|tojson),($a[0]+0)",
            ],
            "[1.10]\n{\"a\":[1.10]}\n\"[1.10]\"\n1.1\n",
        ),
    ];
    for (argv, expected) in rows {
        let got = run_jq_full(argv, Some(""))?;
        if got != (expected.to_string(), String::new(), 0) {
            failures.push(format!("{argv:?}: {got:?}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    Ok(())
}

#[test]
fn test_7881_a_text_that_is_not_json_after_one_that_is_is_still_refused() -> Result<()> {
    let mut failures = Vec::new();
    let rows: [(&[&str], &str); 4] = [
        (
            &[
                "-nc",
                "--argjson",
                "a",
                "[1,2]",
                "--argjson",
                "b",
                "[1,2",
                "[$a,$b]",
            ],
            NOT_JSON_ARGJSON,
        ),
        (
            &["-nc", "$ARGS.positional", "--jsonargs", "[1,2]", "[1,2"],
            NOT_JSON_JSONARGS,
        ),
        // The valid text before a help option is read, and the bad one before it is reported.
        (
            &["-n", "--argjson", "a", "[1,2]", "--argjson", "b", "{", "-h"],
            NOT_JSON_ARGJSON,
        ),
        (
            &["-n", ".", "--jsonargs", "[1,2]", "nope", "-V"],
            NOT_JSON_JSONARGS,
        ),
    ];
    for (argv, stderr) in rows {
        let got = run_jq_full(argv, Some(""))?;
        if got != (String::new(), stderr.to_string(), 2) {
            failures.push(format!("{argv:?}: {got:?}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    Ok(())
}
