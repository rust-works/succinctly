//! #7109: a fold whose UPDATE is a slice with literal bounds plus a closed operand
//! (`.[1:] + [$i]`, a sliding queue) is answered on the owned tree, as the piped form
//! `.[1:] | . + [$i]` already was. Through the bridge each step wrote the whole state out as
//! JSON and indexed it again, so the fold was quadratic: 41 s at 20,000 steps against jq's 0.05 s.
//!
//! Every expected output here is jq 1.7.1's (`/usr/bin/jq -nc`). The long rows run at 20,000
//! elements, which took tens of seconds before the change and take milliseconds now, so a
//! regression to a per-step re-index shows as a slow test before it shows as a wrong answer.

use anyhow::Result;

use crate::{run_jq_null, run_jq_stdin};

fn jq_n(filter: &str) -> Result<String> {
    let (stdout, code) = run_jq_null(filter, &["-c"])?;
    assert_eq!(code, 0, "{filter}: {stdout}");
    Ok(stdout.trim_end().to_string())
}

#[test]
fn a_slice_plus_a_closed_operand_as_a_fold_update_matches_jq() -> Result<()> {
    let rows = [
        (
            r#"[range(20000)] | reduce range(20000) as $i (.; .[1:] + [$i]) | [length, .[0], .[-1]]"#,
            r#"[20000,0,19999]"#,
        ),
        (
            r#"[range(20000)] | reduce range(20000) as $i (.; .[:-1] + [$i]) | [length, .[0], .[-1]]"#,
            r#"[20000,0,19999]"#,
        ),
        (
            r#"[range(20000)] | reduce range(20000) as $i (.; .[1:] + [$i, $i]) | [length, .[0], .[-1]]"#,
            r#"[40000,0,19999]"#,
        ),
        (
            r#"[range(20000)] | reduce range(20000) as $i (.; .[1:] - [$i]) | length"#,
            r#"0"#,
        ),
        (
            r#"[range(20000)] | [foreach range(5) as $i (.; .[1:] + [$i]; length)]"#,
            r#"[20000,20000,20000,20000,20000]"#,
        ),
        (
            r#"[range(20000)] | [limit(3; repeat(.[1:] + [0]) | .[0])]"#,
            r#"[1,1,1]"#,
        ),
        (
            r#"[range(5)] | reduce range(3) as $i (.; .[2:4] + [$i])"#,
            r#"[2]"#,
        ),
        (
            r#""abcdef" | reduce range(3) as $i (.; .[1:] + "x")"#,
            r#""defxxx""#,
        ),
        (
            r#"null | reduce range(3) as $i (.; .[1:] + [$i])"#,
            r#"[2]"#,
        ),
        (
            r#"{} | try (reduce range(2) as $i (.; .[1:] + [$i])) catch ."#,
            r#""Cannot index object with object""#,
        ),
        (
            r#"[1,2,3] | try (reduce range(2) as $i (.; .[1:] + $i)) catch ."#,
            r#""array ([2,3]) and number (0) cannot be added""#,
        ),
        (
            r#"[1,2,3] | reduce range(3) as $i (.; .[1:] + (.[0:1]))"#,
            r#"[1,2,3]"#,
        ),
        (
            r#"[1,2,3] | reduce range(3) as $i (.; .[1:] + [$i + 1])"#,
            r#"[1,2,3]"#,
        ),
        (
            r#"[1,2,3] | [.[1:] + [4], .[2:] + [5]]"#,
            r#"[[2,3,4],[3,5]]"#,
        ),
        (
            r#"[range(5)] | reduce range(3) as $i (.; (.[1:]) + [$i])"#,
            r#"[3,4,0,1,2]"#,
        ),
        (
            r#"[range(5)] | reduce (1,2) as $i (.; .[$i:] + [$i])"#,
            r#"[3,4,1,2]"#,
        ),
        (
            r#"[range(5)] | reduce range(3) as $i (.; .[1:] + [$i] | . + [9])"#,
            r#"[3,4,0,9,1,9,2,9]"#,
        ),
        (
            r#"[range(5)] | first(foreach range(3) as $i (.; .[1:] + [$i]; select(length == 3)))"#,
            r#""#,
        ),
        (
            r#"[range(5)] | reduce range(3) as $i (.; .[1:] + ([$i], [9]))"#,
            r#"[3,4,9,9,9]"#,
        ),
        (
            r#"[range(5)] | reduce range(3) as $i (.; .[1:] + [$i]?)"#,
            r#"[3,4,0,1,2]"#,
        ),
    ];
    for (filter, want) in rows {
        assert_eq!(jq_n(filter)?, want, "{filter}");
    }
    Ok(())
}

/// A loop inside a binding of a document node (`. as $d | ...`) keeps the evaluator's own route
/// (the #2889 embed table is live), and `$d` still reads the input it was bound to.
#[test]
fn a_slice_plus_a_closed_operand_inside_a_node_binding_matches_jq() -> Result<()> {
    let rows = [
        (
            ". as $d | reduce range(3) as $i ($d; .[1:] + [$i])",
            "[4,5,0,1,2]",
        ),
        (
            ". as $d | [$d, (reduce range(3) as $i ($d; .[1:] + [$i]))]",
            "[[1,2,3,4,5],[4,5,0,1,2]]",
        ),
    ];
    for (filter, want) in rows {
        let (stdout, code) = run_jq_stdin(filter, "[1,2,3,4,5]", &["-c"])?;
        assert_eq!(code, 0, "{filter}: {stdout}");
        assert_eq!(stdout.trim_end(), want, "{filter}");
    }
    Ok(())
}
