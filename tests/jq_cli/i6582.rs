//! #6582: a `reduce`/`foreach`/`until` step `SOURCE as $v | BODY` whose source is one scalar read
//! off the state (`. as $s | $s + $x`, `.n as $n | .n = $n + 1`) binds it and answers `BODY` on the
//! state, where the whole binding used to run on a re-indexed copy of the state every step. Every
//! expected string was captured from jq 1.7.1 (`/usr/bin/jq`).

use anyhow::Result;

use crate::run_jq_full;

/// Integers and a float, so the bound value is spliced in with its own spelling.
const INPUT: &str = "[3, 1, 4, 1, 5, 9, 2, 6, 1.5]";

#[test]
fn as_binding_steps_answer_as_jq_6582() -> Result<()> {
    for (filter, want, want_code) in [
        (r"reduce .[] as $x (0; . as $s | $s + $x)", "32.5\n", 0),
        (r"reduce .[] as $x (0; (. + $x) as $t | $t)", "32.5\n", 0),
        (
            r"reduce .[] as $x (0; (. + $x) as $t | if $t > 10 then 0 else $t end)",
            "7.5\n",
            0,
        ),
        (
            r"reduce .[] as $x ({n: 0}; .n as $n | .n = $n + 1)",
            "{\"n\":9}\n",
            0,
        ),
        (
            r"reduce .[] as $x ({n: 0}; .n as $n | {n: ($n + $x)})",
            "{\"n\":32.5}\n",
            0,
        ),
        (
            r"reduce .[] as $x ([]; length as $k | . + [$k])",
            "[0,1,2,3,4,5,6,7,8]\n",
            0,
        ),
        (r"reduce .[] as $x (0; . as $x | $x + 1)", "9\n", 0),
        (
            r"reduce .[] as $x ([]; . as $s | $s + [$x])",
            "[3,1,4,1,5,9,2,6,1.5]\n",
            0,
        ),
        (
            r"reduce .[] as $x ([]; . as [$a] | . + [$a])",
            "[null,null,null,null,null,null,null,null,null]\n",
            0,
        ),
        (
            r#"try reduce .[] as $x (0; (. + "a") as $t | $t) catch ."#,
            "\"number (0) and string (\\\"a\\\") cannot be added\"\n",
            0,
        ),
        (r"reduce .[] as $x (0; . as $s | ($s, $s + 1))", "9\n", 0),
        (
            r"[foreach .[] as $x (0; (. + 1) as $t | $t * 2)]",
            "[2,6,14,30,62,126,254,510,1022]\n",
            0,
        ),
        (r"0 | until(. > 50; . as $s | $s * 2 + 1)", "63\n", 0),
        (r"reduce .[] as $x (1.50; . as $s | $s)", "1.50\n", 0),
    ] {
        let (stdout, _, code) = run_jq_full(&["-c", filter], Some(INPUT))?;
        assert_eq!((stdout.as_str(), code), (want, want_code), "{filter}");
    }
    Ok(())
}

/// 300,000 steps that bind a field each step; the answer is jq 1.7.1's.
#[test]
fn a_long_as_binding_fold_answers_6582() -> Result<()> {
    let filter = "reduce range(300000) as $i ({n: 0}; .n as $n | .n = ($n + $i) % 1000) | .n";
    let (stdout, _, code) = run_jq_full(&["-nc", filter], None)?;
    assert_eq!((stdout.as_str(), code), ("0\n", 0));
    Ok(())
}
