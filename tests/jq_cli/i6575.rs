//! #6575: a `reduce`/`foreach`/`until`/`while` step that is a pipe of stages which are not all
//! assignments (`. + [$x] | .[-5:]`, `. + $x | . % 1000`, `[., $x] | max`) applies its stages to
//! the state one at a time, where the whole pipe used to run on a re-indexed copy of the state
//! every step. Every expected string was captured from jq 1.7.1 (`/usr/bin/jq`).

use anyhow::Result;

use crate::run_jq_full;

/// Integers and a float, so the slices, sorts and `max` see repeats and mixed numbers.
const INPUT: &str = "[3, 1, 4, 1, 5, 9, 2, 6, 1.5]";

#[test]
fn pipe_of_stages_steps_answer_as_jq_6575() -> Result<()> {
    for (filter, want, want_code) in [
        (
            r"reduce .[] as $x ([]; . + [$x] | .[-3:])",
            "[2,6,1.5]\n",
            0,
        ),
        (r"reduce .[] as $x (0; . + $x | . % 7)", "4\n", 0),
        (r"reduce .[] as $x (0; [., $x] | max)", "9\n", 0),
        (r"reduce .[] as $x (null; [., $x] | min)", "null\n", 0),
        (r"reduce .[] as $x (0; (. + 1) | (. * 2))", "1022\n", 0),
        (r"reduce .[] as $x (0; . + 1 | . * 2 | . - 1)", "511\n", 0),
        (
            r"reduce .[] as $x ([]; . + [$x] | sort | unique)",
            "[1,1.5,2,3,4,5,6,9]\n",
            0,
        ),
        (
            r"reduce .[] as $x (0; . + 1 | if . > 3 then 0 else . end)",
            "1\n",
            0,
        ),
        (
            r"reduce .[] as $x ([]; . + [$x] | if length > 3 then .[1:] else . end)",
            "[2,6,1.5]\n",
            0,
        ),
        (
            r"reduce .[] as $x ({}; . + {($x|tostring): $x} | to_entries | from_entries)",
            "{\"3\":3,\"1\":1,\"4\":4,\"5\":5,\"9\":9,\"2\":2,\"6\":6,\"1.5\":1.5}\n",
            0,
        ),
        (
            r#"reduce .[] as $x (""; . + "ab" | .[1:])"#,
            "\"babababab\"\n",
            0,
        ),
        (
            r"try reduce .[] as $x (0; . + $x | .a) catch .",
            "\"Cannot index number with string \\\"a\\\"\"\n",
            0,
        ),
        (
            r#"try reduce .[] as $x (0; . + 1 | error("e")) catch ."#,
            "\"e\"\n",
            0,
        ),
        (r"reduce .[] as $x (0; . + 1 | (., .))", "9\n", 0),
        (
            r"[foreach .[] as $x ([]; . + [$x] | .[-2:]; length)]",
            "[1,2,2,2,2,2,2,2,2]\n",
            0,
        ),
        (r"0 | until(. > 50; . + 7 | . * 2)", "98\n", 0),
        (r"[1 | while(. < 50; . + 1 | . * 2)]", "[1,4,10,22,46]\n", 0),
    ] {
        let (stdout, _, code) = run_jq_full(&["-c", filter], Some(INPUT))?;
        assert_eq!((stdout.as_str(), code), (want, want_code), "{filter}");
    }
    Ok(())
}

/// A sliding window over 300,000 steps; the answer is jq 1.7.1's.
#[test]
fn a_long_sliding_window_answers_6575() -> Result<()> {
    let filter = "reduce range(300000) as $i ([]; . + [$i] | .[-5:]) | add";
    let (stdout, _, code) = run_jq_full(&["-nc", filter], None)?;
    assert_eq!((stdout.as_str(), code), ("1499985\n", 0));
    Ok(())
}
