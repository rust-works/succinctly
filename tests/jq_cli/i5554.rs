//! #5554: a built value past 384 levels (`MAX_VALUE_TREE_DEPTH`) is refused by the re-index
//! bridge, which jq has no counterpart for. A stage that only renders its input as text answers
//! it from the tree instead, with jq's writer's `<skipped: too deep>` marker past 256 levels.
//! Every expected value was captured from jq 1.7.1 (`/usr/bin/jq`).
//!
//! Only a pipe whose first stage renders the value is covered; the same stage inside `try`, a
//! comma, an `as` source or `map` still reads the refusal (recorded in the limitations page).

use anyhow::Result;

use crate::run_jq_full;

const DEF: &str = "def f(n): if n == 0 then 0 else [f(n-1)] end;";

fn answer(program: &str) -> Result<String> {
    let (stdout, stderr, code) = run_jq_full(&["-nc", &format!("{DEF} {program}")], None)?;
    assert_eq!(code, 0, "`{program}`: {stderr}");
    Ok(stdout)
}

#[test]
fn test_text_stages_render_a_value_past_384_levels_5554() -> Result<()> {
    for (program, want) in [
        ("f(400) | tojson | length", "533\n"),
        ("f(400) | tostring | length", "533\n"),
        (r#"f(400) | "\(.)" | length"#, "533\n"),
        ("f(384) | tojson | length", "533\n"),
        ("f(1000) | tojson | length", "533\n"),
        ("[f(1000)] | tojson | length", "533\n"),
        ("f(500) | @text | length", "533\n"),
        ("f(500) | @json | length", "533\n"),
        (r#"f(500) | "a\(.)b\(.)c" | length"#, "1069\n"),
        // The marker is where jq puts it, 256 levels down.
        (
            "f(500) | tojson | .[250:300]",
            "\"[[[[[[[<skipped: too deep>]]]]]]]]]]]]]]]]]]]]]]]]\"\n",
        ),
    ] {
        assert_eq!(answer(program)?, want, "`{program}`");
    }
    Ok(())
}

#[test]
fn test_a_lone_stage_renders_a_value_past_384_levels_5554() -> Result<()> {
    // Not a rest of several stages: the one stage reaches the same bridge.
    for program in ["f(1000) | tojson", "f(1000) | @json", "f(500) | tostring"] {
        let out = answer(program)?;
        assert!(out.starts_with("\"[[[["), "`{program}`");
        assert!(out.contains("<skipped: too deep>"), "`{program}`");
        // jq's 533 characters, and the two quotes of the string.
        assert_eq!(out.len(), 533 + 2 + 1, "`{program}`");
    }
    Ok(())
}

#[test]
fn test_a_value_the_bridge_accepts_is_unchanged_5554() -> Result<()> {
    // Under the refusal depth nothing asks the text peel, so these read as they did.
    assert_eq!(answer("f(383) | tojson | length")?, "533\n");
    assert_eq!(answer("f(250) | tojson | length")?, "501\n");
    assert_eq!(answer("[1,{\"a\":2.50}] | tojson | length")?, "14\n");
    Ok(())
}
