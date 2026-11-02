//! #5554: a built value past 384 levels (`MAX_VALUE_TREE_DEPTH`) is refused by the re-index
//! bridge, which jq has no counterpart for. A stage that only renders its input as text answers
//! it from the tree instead, with jq's writer's `<skipped: too deep>` marker past 256 levels.
//! Every expected value was captured from jq 1.7.1 (`/usr/bin/jq`).
//!
//! A pipe whose first stage renders the value is covered, alone or under `try`/`?`, as an `as`
//! source whose body ignores `.`, or as a format-prefixed interpolation. In jq mode the bridge
//! itself is bounded by the native-stack floor since Option 3 (`i5554c.rs`), so this text stage is
//! now the fallback past that floor.

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
        // A leading group is the same stages, and the rest of the pipe still runs.
        ("f(400) | (tojson | length)", "533\n"),
        ("f(400) | (tojson | ascii_downcase) | length", "533\n"),
        // A consumer that stops early stops the peeled rest too.
        ("first(f(400) | tojson | length)", "533\n"),
        ("[limit(1; f(500) | tojson | length, 7)]", "[533]\n"),
        ("isempty(f(400) | tojson)", "false\n"),
        (r#"f(500) | "\(.)\(.)\(.)" | length"#, "1599\n"),
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

#[test]
fn test_text_stage_under_try_as_and_format_interpolation_5554() -> Result<()> {
    for (program, want) in [
        ("f(400) | try tojson | length", "533\n"),
        ("f(400) | try @text | length", "533\n"),
        ("f(400) | tojson as $x | $x | length", "533\n"),
        (r#"f(400) | tojson as $x | "a" + $x | length"#, "534\n"),
        ("f(400) | (tojson as $x | $x) | length", "533\n"),
        ("f(400) | tostring as $x | [$x] | length", "1\n"),
        (r#"f(400) | @json "x\(.)" | length"#, "534\n"),
        (r#"f(400) | @json "x\(.)y\(.)" | length"#, "1068\n"),
        (
            "f(500) | try tostring | .[250:300]",
            "\"[[[[[[[<skipped: too deep>]]]]]]]]]]]]]]]]]]]]]]]]\"\n",
        ),
    ] {
        assert_eq!(answer(program)?, want, "`{program}`");
    }
    Ok(())
}

#[test]
fn test_a_binding_body_that_reads_the_deep_input_answers_as_jq_5554() -> Result<()> {
    // `. as $x | .` reads the value through the bridge, which the native-stack floor bounds since
    // Option 3 of #5554 rather than 384; jq 1.7.1 answers `1`.
    assert_eq!(answer("f(400) | tojson as $x | . | length")?, "1\n");
    Ok(())
}
