//! #5417: a parenthesized pipe over computed values that is driven one output at a time (`select`,
//! `limit`, `first`, `last`, `nth`, `any`, `isempty`, a streamed result) keeps the owned copy of
//! each tail on the pipe instead of copying the rest of it for every value at every stage. The
//! copy count is pinned by the unit test `test_sink_driven_pipe_rest_is_copied_once_per_program_
//! not_per_value_5417`; these rows pin that the answers are the ones jq gives. Expected outputs
//! were captured from jq 1.7.1 (`/usr/bin/jq`).

use anyhow::Result;

use crate::run_jq_full;

/// Four computing stages, so each tail the pipe hands on is a pipe of its own.
const T: &str = ". + 1 | . + 1 | . + 1 | . + 1";

#[test]
fn test_parenthesized_pipe_driven_one_output_at_a_time_matches_jq_5417() -> Result<()> {
    for (filter, want, code) in [
        (
            format!("[range(0;6) | ({T}) | select(. % 2 == 0)]"),
            "[4,6,8]\n",
            0,
        ),
        (format!("last(range(0;6) | ({T}))"), "9\n", 0),
        (format!("[limit(3; range(0;6) | ({T}))]"), "[4,5,6]\n", 0),
        (
            format!("first(range(0;6) | ({T}) | select(. > 6))"),
            "7\n",
            0,
        ),
        (format!("nth(2; range(0;6) | ({T}))"), "6\n", 0),
        (format!("any(range(0;6) | ({T}); . == 8)"), "true\n", 0),
        (
            format!("isempty(range(0;6) | ({T}) | select(. == 99))"),
            "true\n",
            0,
        ),
        (format!("range(0;4) | ({T})"), "4\n5\n6\n7\n", 0),
        // A stage that fans out, then more stages: each output of the fan-out goes down the
        // rest of the pipe, in order.
        (
            "[range(0;3) | (. + 1 | (., . + 100) | . + 1 | . + 1 | . + 1)]".to_string(),
            "[4,104,5,105,6,106]\n",
            0,
        ),
        (
            "[limit(4; range(0;3) | (. + 1 | (., . + 100) | . + 1 | . + 1 | . + 1))]".to_string(),
            "[4,104,5,105]\n",
            0,
        ),
        (
            "[range(0;3) | (. + 1 | (empty, .) | . + 1 | . + 1 | . + 1)]".to_string(),
            "[4,5,6]\n",
            0,
        ),
        // An effect before the last stage drives the pipe stage by stage.
        (
            format!("[range(0;3) | (. + input_line_number | {T})]"),
            "[4,5,6]\n",
            0,
        ),
        // An error in the pipe: caught per value, or raised after the outputs before it.
        (
            format!(
                "[range(0;6) | try ({T} | if . == 6 then error(\"boom\") else . end) catch \"c\"]"
            ),
            "[4,5,\"c\",7,8,9]\n",
            0,
        ),
        (
            format!("range(0;6) | ({T} | if . == 6 then error(\"boom\") else . end)"),
            "4\n5\n",
            5,
        ),
    ] {
        let (stdout, stderr, status) = run_jq_full(&["-n", "-c", &filter], None)?;
        assert_eq!(
            (stdout.as_str(), status),
            (want, code),
            "`{filter}` -- {stderr:?}"
        );
    }
    Ok(())
}

/// A pipe long enough that the quadratic copy was the cost answers as jq does, for every
/// consumer. The unit test pins the copies; this is the end-to-end check that the answers did
/// not move. Values captured from jq 1.7.1 for 60 stages over 300 values.
#[test]
fn test_long_parenthesized_pipe_matches_jq_5417() -> Result<()> {
    let long = vec![". + 1"; 60].join(" | ");
    for (shape, want) in [
        (
            "[range(0;300) | (PIPE) | select(. % 7 == 0)] | length",
            "43\n",
        ),
        ("last(range(0;300) | (PIPE))", "359\n"),
        ("[limit(100; range(0;300) | (PIPE))] | add", "10950\n"),
        ("first(range(0;300) | (PIPE) | select(. > 200))", "201\n"),
        (
            "[range(0;300) | (. + input_line_number | PIPE)] | add",
            "62850\n",
        ),
        // A body that is rebuilt for each value that reads its variable still copies its rest per value (#5424); its
        // answer is the same.
        (
            "reduce range(0;300) as $i (0; ($i | (PIPE)) + .)",
            "62850\n",
        ),
    ] {
        let filter = shape.replace("PIPE", &long);
        let (stdout, stderr, status) = run_jq_full(&["-n", "-c", &filter], None)?;
        assert_eq!(
            (stdout.as_str(), status),
            (want, 0),
            "`{shape}` -- {stderr:?}"
        );
    }
    Ok(())
}

/// The same pipes over a document: the values of its first stage are cursors into the input,
/// the computed ones owned, and the pipe is driven through the generic evaluator.
#[test]
fn test_parenthesized_pipe_over_a_document_matches_jq_5417() -> Result<()> {
    for (filter, want) in [
        (format!("[.[] | ({T}) | select(. > 6)]"), "[7,8]\n"),
        (format!("[.[] | ({T})] | add"), "26\n"),
        (format!("last(.[] | ({T}))"), "8\n"),
        (format!("[limit(2; .[] | ({T}))]"), "[5,6]\n"),
        (format!("first(.[] | ({T}) | select(. > 6))"), "7\n"),
    ] {
        let (stdout, stderr, status) = run_jq_full(&["-c", &filter], Some("[1,2,3,4]"))?;
        assert_eq!(
            (stdout.as_str(), status),
            (want, 0),
            "`{filter}` -- {stderr:?}"
        );
    }
    Ok(())
}
