//! #6574: a fold step whose right operand holds a closed `if` (`. + (if $i % 2 == 0 then 1 else 0 end)`)
//! is answered from the branch the condition picks, where the whole step used to run on the evaluator's
//! route every round. Every expected string was captured from jq 1.7.1 (`/usr/bin/jq`).

use anyhow::Result;

use crate::run_jq_full;

#[test]
fn a_closed_if_operand_answers_as_jq_6574() -> Result<()> {
    for (filter, want, want_code) in [
        (
            r"reduce range(10) as $i (0; . + (if $i % 2 == 0 then 1 else 0 end))",
            "5\n",
            0,
        ),
        (
            r"reduce range(10) as $i (0; . + (if $i > 5 then 1 elif $i > 2 then 2 else 3 end))",
            "19\n",
            0,
        ),
        (
            r#"reduce range(10) as $i (""; . + (if $i > 5 then "b" else "a" end))"#,
            "\"aaaaaabbbb\"\n",
            0,
        ),
        (
            r"reduce range(6) as $i ([]; . + (if $i % 2 == 0 then [$i] else [] end))",
            "[0,2,4]\n",
            0,
        ),
        (
            r"reduce range(4) as $i ({}; . + (if $i > 1 then {big: $i} else {small: $i} end))",
            "{\"small\":1,\"big\":3}\n",
            0,
        ),
        (
            r#"reduce range(6) as $i (0; . + (if $i > 3 then "s" else 1 end))"#,
            "",
            5,
        ),
        (
            r#"try reduce range(6) as $i (0; . + (if $i > 3 then "s" else 1 end)) catch ."#,
            "\"number (4) and string (\\\"s\\\") cannot be added\"\n",
            0,
        ),
        (
            r#"reduce range(6) as $i (0; . + (if $i > 99 then error("x") else 1 end))"#,
            "6\n",
            0,
        ),
        (
            r#"try reduce range(6) as $i (0; . + (if $i > 3 then error("x") else 1 end)) catch ."#,
            "\"x\"\n",
            0,
        ),
        (
            r"reduce range(6) as $i (0; . + (if $i > 3 then empty else 1 end))",
            "null\n",
            0,
        ),
        (
            r"reduce range(4) as $i (0; . + (if $i > 1 then (10, 20) else 1 end))",
            "42\n",
            0,
        ),
        (
            r"reduce range(6) as $i (0; . + (if $i % 2 == 0 then null else false end // 7))",
            "42\n",
            0,
        ),
        (
            r"reduce range(6) as $i (0; . + (if . > 2 then 100 else 1 end))",
            "303\n",
            0,
        ),
        (
            r"reduce range(5) as $i (0; . + (if $i then 1 else 0 end))",
            "5\n",
            0,
        ),
        (
            r"reduce range(5) as $i (0; . + (if $i > 2 then 1 end))",
            "2\n",
            0,
        ),
        (
            r"try reduce range(5) as $i (0; . + (if empty then 1 else 0 end)) catch .",
            "null\n",
            0,
        ),
        (
            r"[foreach range(6) as $i (0; . + (if $i % 2 == 0 then 1 else 10 end))]",
            "[1,11,12,22,23,33]\n",
            0,
        ),
        (
            r"[foreach range(6) as $i (0; . + (if $i % 2 == 0 then 1 else 10 end); [$i, .])]",
            "[[0,1],[1,11],[2,12],[3,22],[4,23],[5,33]]\n",
            0,
        ),
        (
            r"[0 | while(. < 20; . + (if . > 5 then 10 else 3 end))]",
            "[0,3,6,16]\n",
            0,
        ),
        (
            r"reduce range(5) as $i (0; . + (if ($i > 2, $i > 3) then 1 else 0 end))",
            "1\n",
            0,
        ),
        (
            r"reduce range(5) as $i (0; . + (if $i > 2 then [$i] else [$i, 0] end | length))",
            "8\n",
            0,
        ),
        (
            r"reduce range(4) as $i (null; . + (if $i == 0 then null else {a: $i} end))",
            "{\"a\":3}\n",
            0,
        ),
        (
            r"reduce range(6) as $i ({n: 0}; .n += (if $i % 2 == 0 then 1 else 10 end))",
            "{\"n\":33}\n",
            0,
        ),
        (
            r"reduce range(6) as $i ({n: 0}; .n |= . + (if $i % 2 == 0 then 1 else 10 end))",
            "{\"n\":33}\n",
            0,
        ),
        (
            r"reduce range(6) as $i ({n: 0}; .n -= (if $i > 2 then 1 else 0 end))",
            "{\"n\":-3}\n",
            0,
        ),
        (
            r#"reduce range(4) as $i ({}; .k = (if $i > 1 then "big" else "small" end))"#,
            "{\"k\":\"big\"}\n",
            0,
        ),
        (
            r#"reduce range(4) as $i ({}; .[if $i > 1 then "hi" else "lo" end] += 1)"#,
            "{\"lo\":2,\"hi\":2}\n",
            0,
        ),
        (
            r"reduce range(5) as $i ([0, 0]; .[if $i > 2 then 0 else 1 end] += 1)",
            "[2,3]\n",
            0,
        ),
        (
            r"[0 | until(. > 10; . + (if . > 4 then 5 else 2 end))]",
            "[11]\n",
            0,
        ),
        (
            r"0 | until(. > 10; . + (if . > 4 then 5 else 2 end))",
            "11\n",
            0,
        ),
        (
            r#"try reduce range(5) as $i (0; . + (if $i > 2 then 1 else 2 end | error("e"))) catch ."#,
            "\"e\"\n",
            0,
        ),
        (
            r"reduce range(5) as $i (0; . + (if $i > 2 then 1 else 2 end | tostring | length))",
            "5\n",
            0,
        ),
        (
            r#"try reduce range(5) as $i ({n: 0}; .n += (if $i > 2 then "s" else 1 end)) catch ."#,
            "\"number (3) and string (\\\"s\\\") cannot be added\"\n",
            0,
        ),
        (
            r"reduce range(4) as $i ([]; . + (if $i > 1 then [1] else [] end) | if length > 2 then .[1:] else . end)",
            "[1,1]\n",
            0,
        ),
    ] {
        let (stdout, _, code) = run_jq_full(&["-nc", filter], None)?;
        assert_eq!((stdout.as_str(), code), (want, want_code), "{filter}");
    }
    Ok(())
}

/// A long fold, where the per-step route is the cost; and yq mode, which has no such operand step
/// and answers as before.
#[test]
fn a_long_closed_if_operand_fold_answers_6574() -> Result<()> {
    let filter = r#"reduce range(300000) as $i (""; . + (if $i % 2 == 0 then "a" else "b" end))"#;
    let (stdout, _, code) = run_jq_full(&["-nc", filter], None)?;
    assert_eq!(stdout.len(), 300_000 + 3);
    assert_eq!(code, 0);
    Ok(())
}
