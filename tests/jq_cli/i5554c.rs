//! #5554 (Option 3): a walk over a value the program built is bounded by ADR-0025's native-stack
//! floor past 384 levels, not by `MAX_VALUE_TREE_DEPTH`, in jq mode. The re-index bridge, a fold's
//! step, `[., 1]`, `sort`, `error` and the rest answer as jq does; past the floor they refuse at
//! exit 5 instead of overflowing the stack. Every expected string was captured from jq 1.7.1
//! (`/usr/bin/jq`).

use anyhow::Result;

use crate::run_jq_full;

const DEF: &str = "def f(n): if n == 0 then 0 else [f(n-1)] end;";

/// A value two million levels deep, built through `.[0] = .`'s owned door in linear time. Both the
/// release thread's floor (about 700,000 levels for the bridge) and the debug one's (under
/// 1,000,000) are shallower, and dropping it still fits the stack.
const PAST_THE_FLOOR: &str = "(reduce range(2000000) as $i ([]; .[0] = .))";

const FLOOR_REFUSAL: &str = "nesting depth exceeds limit of the native stack";

/// jq's render of an array more than 256 levels deep: `levels` brackets, the marker, 257 closers.
fn skipped(levels: usize) -> String {
    format!(
        "{}<skipped: too deep>{}",
        "[".repeat(levels),
        "]".repeat(257)
    )
}

#[test]
fn the_rows_of_the_issue_answer_as_jq_5554() -> Result<()> {
    for (filter, want) in [
        // The body's table.
        ("reduce range(500) as $i (0; [.]) | length", "1"),
        ("reduce range(400) as $i (0; [.]) | flatten", "[0]"),
        ("last(foreach range(500) as $i (0; [.])) | length", "1"),
        ("f(400) | tojson | length", "533"),
        // Group 1: the refusal fired inside the step.
        ("[foreach range(500) as $i (0; [.]; 1)] | length", "500"),
        ("reduce range(385) as $i (0; [.]) | 1", "1"),
        // Group 2: the text stage where the peel was not wired.
        ("f(400) | tojson, tojson | length", "533\n533"),
        ("[f(400)] | map(tojson) | length", "1"),
        ("[f(400)] | .[] | tojson | length", "533"),
        ("f(400) | @json \"\\(tojson)\" | length", "535"),
        // The consumers #5780 left refusing inside the evaluator.
        ("f(400) | [., 1] | length", "2"),
        ("f(400) | (., 1) | length", "1\n1"),
        ("first(f(400), 1) | length", "1"),
        ("f(400) | sort | length", "1"),
        ("f(400) == f(400)", "true"),
        ("f(400) | walk(.) | length", "1"),
        ("f(400) | [paths] | length", "400"),
        ("f(400) | .[0][0] = 1 | length", "1"),
        ("f(400) | {a: .} | .a | length", "1"),
        ("f(400) | if . then 1 else 2 end", "1"),
        ("f(400) | $__loc__", "{\"file\":\"<top-level>\",\"line\":1}"),
        // The splice sites #5526 netted.
        ("reduce f(400) as $x (0; if $x then 1 else 2 end)", "1"),
        ("null | .a //= f(400) | length", "1"),
    ] {
        let program = format!("{DEF} {filter}");
        let (stdout, stderr, code) = run_jq_full(&["-nc", &program], None)?;
        assert_eq!(
            (stdout.as_str(), stderr.as_str(), code),
            (format!("{want}\n").as_str(), "", 0),
            "{filter}"
        );
    }
    Ok(())
}

#[test]
fn a_deep_value_written_to_stderr_renders_as_jq_renders_it_5554() -> Result<()> {
    // `["DEBUG:", v]` puts `v` a level down, so it meets the marker a level sooner, and the
    // wrapper's own closer is the 257th.
    let debug = format!("[\"DEBUG:\",{}\n", skipped(256));
    let error = format!(
        "jq: error (at <unknown>) (not a string): {}\n",
        skipped(257)
    );
    let halted = format!("{}\n", skipped(257));
    for (filter, want_stdout, want_stderr, want_code) in [
        ("f(400) | debug | length", "1\n", debug.as_str(), 0),
        ("f(400) | stderr | length", "1\n", skipped(257).as_str(), 0),
        ("f(400) | error", "", error.as_str(), 5),
        ("f(400) | halt_error", "", halted.as_str(), 5),
    ] {
        let program = format!("{DEF} {filter}");
        let (stdout, stderr, code) = run_jq_full(&["-nc", &program], None)?;
        assert_eq!(
            (stdout.as_str(), stderr.as_str(), code),
            (want_stdout, want_stderr, want_code),
            "{filter}"
        );
    }
    Ok(())
}

#[test]
fn the_rows_answer_on_the_document_route_too_5554() -> Result<()> {
    for (filter, want) in [
        ("reduce range(500) as $i (.; [.]) | length", "1"),
        ("reduce range(400) as $i (.; [.]) | flatten", "[1]"),
        ("f(400) | [., 1] | length", "2"),
        (".a //= f(400) | .a | length", "1"),
    ] {
        let program = format!("{DEF} {filter}");
        let input = if filter.starts_with(".a") { "{}" } else { "1" };
        let (stdout, stderr, code) = run_jq_full(&["-c", &program], Some(input))?;
        assert_eq!(
            (stdout.as_str(), stderr.as_str(), code),
            (format!("{want}\n").as_str(), "", 0),
            "{filter}"
        );
    }
    Ok(())
}

/// Past the floor a walk refuses, uncatchably, at exit 5 -- never a stack overflow (134), a
/// segfault (139) or a raw panic (101), which is where jq itself has long since crashed.
#[test]
fn a_walk_past_the_native_stack_floor_refuses_at_exit_5_5554() -> Result<()> {
    for filter in [
        // The re-index bridge (a generator element keeps it: `[., 1]` is answered off the tree,
        // #6620), and a walk of its own.
        format!("{PAST_THE_FLOOR} | [., (1, 2)] | length"),
        format!("{PAST_THE_FLOOR} | flatten"),
        // A splice site's panicking guard, caught by #5526's net.
        format!("{PAST_THE_FLOOR} as $v | reduce $v as $x (0; if $x then 1 else 2 end)"),
        // A resource limit (#2132's table, #3261's row): no spelling of `try` absorbs it.
        format!("try ({PAST_THE_FLOOR} | flatten) catch 0"),
        format!("[{PAST_THE_FLOOR} | [., (1, 2)]?] | length"),
        format!("[try ({PAST_THE_FLOOR} | [., (1, 2)])] | length"),
    ] {
        let (stdout, stderr, code) = run_jq_full(&["-nc", &filter], None)?;
        assert_eq!(code, 5, "{filter}: {stderr}");
        assert_eq!(stdout, "", "{filter}");
        assert_eq!(
            stderr,
            format!("jq: error (at <unknown>): {FLOOR_REFUSAL}\n"),
            "{filter}"
        );
    }
    // The text stage still answers from the tree there (#5598), and `length` never walks.
    for (filter, want) in [
        (format!("{PAST_THE_FLOOR} | tojson | length"), "533\n"),
        (format!("{PAST_THE_FLOOR} | length"), "1\n"),
        // #6620: and a constructor of reads builds from the tree without walking the value it holds.
        (format!("{PAST_THE_FLOOR} | [., 1] | length"), "2\n"),
        (format!("{PAST_THE_FLOOR} | [., .] | length"), "2\n"),
    ] {
        let (stdout, stderr, code) = run_jq_full(&["-nc", &filter], None)?;
        assert_eq!(
            (stdout.as_str(), stderr.as_str(), code),
            (want, "", 0),
            "{filter}"
        );
    }
    Ok(())
}
