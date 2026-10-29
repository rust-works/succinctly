//! #4095: `recurse(.[]?; cond)` in jq mode. The position walk the yq-mode work added is shared with
//! jq's `| path` stage, so these pin what it must not change: an effectful `cond` is not asked by a
//! route that collects, the shared M2 gate does not hand jq's owned results to the fast JSON
//! writer, and the sink arm's direct verdicts answer what the general route answers.

use anyhow::Result;

use crate::run_jq_full;

#[test]
fn test_recurse_cond_reads_no_input_past_the_first_node_4095() -> Result<()> {
    let (stdout, _, code) = run_jq_full(
        &[
            "-c",
            "-n",
            "input | [first(recurse(.[]?; input)) | path], [inputs]",
        ],
        Some("{\"a\":[1,2]}\n1\n2\n3\n4\n5\n"),
    )?;
    assert_eq!(code, 0);
    assert_eq!(stdout, "[[]]\n[1,2,3,4,5]\n");
    Ok(())
}

/// The same walk named through `path(...)`, which jq 1.7.1 answers identically (captured live).
#[test]
#[cfg_attr(feature = "unshared-containers", ignore = "holdout refusal (#5246)")]
fn test_path_of_recurse_cond_reads_no_input_past_the_first_node_4095() -> Result<()> {
    let (stdout, _, code) = run_jq_full(
        &[
            "-c",
            "-n",
            "input | [path(first(recurse(.[]?; input)))], [inputs]",
        ],
        Some("{\"a\":[1,2]}\n1\n2\n3\n4\n5\n"),
    )?;
    assert_eq!(code, 0);
    assert_eq!(stdout, "[[]]\n[1,2,3,4,5]\n");
    Ok(())
}

/// jq's own sink arm answers `type` and comparison conds through the direct verdict; `limit`
/// drives it (the array constructor's collecting form keeps the owned walk), and it has to agree
/// with the general route on every JSON kind.
#[test]
fn test_recurse_cond_sink_arm_agrees_with_the_general_route_4095() -> Result<()> {
    let doc = r#"{"a":null,"b":true,"c":[1,"s",{"d":2.50}],"e":{},"f":[],"g":-0,"h":1e2}"#;
    for cond in [
        r#"type == "object" or type == "array""#,
        r#"type != "number""#,
        r#""string" == type"#,
        r#"type == "number" and . != 0"#,
        ". != null",
        ". == null",
        "null == .",
        ". == true",
        ". != false",
        ". == null or . == false",
        ". == true | not",
        ". == 1 | not",
        "null",
        "(. != 1)",
        ". != null and . != false",
    ] {
        let general = format!("[limit(100000; recurse(.[]?; . | ({cond})))]");
        let direct = format!("[limit(100000; recurse(.[]?; {cond}))]");
        let (want, _, c1) = run_jq_full(&["-c", &general], Some(doc))?;
        let (got, stderr, c2) = run_jq_full(&["-c", &direct], Some(doc))?;
        assert_eq!((c1, c2), (0, 0), "`{cond}` -- {stderr:?}");
        assert_eq!(got, want, "`{cond}`");
    }
    Ok(())
}

/// A malformed keyword token raises where the general route raises (#3035), not an answer from
/// the direct verdict.
#[test]
fn test_recurse_cond_sink_arm_raises_on_a_malformed_token_like_the_general_route_4095() -> Result<()>
{
    let doc = "[1,nul]";
    let general = r#"[limit(10; recurse(.[]?; . | (type == "number")))]"#;
    let direct = r#"[limit(10; recurse(.[]?; type == "number"))]"#;
    let (w_out, w_err, w_code) = run_jq_full(&["-c", general], Some(doc))?;
    let (g_out, g_err, g_code) = run_jq_full(&["-c", direct], Some(doc))?;
    assert_eq!((g_out, g_code), (w_out, w_code));
    assert_eq!(g_err.trim().is_empty(), w_err.trim().is_empty());
    Ok(())
}

/// The shared M2 gate admits a gated `recurse(.[]?; cond)`, but jq mode's collecting route hands a
/// wrapper owned results, whose fast JSON writer prints a number beyond the `f64` range as `null`.
/// A program holding the builtin takes the general path, which prints jq's own spelling.
#[test]
fn test_recurse_cond_under_a_wrapper_keeps_a_huge_number_4095() -> Result<()> {
    for (filter, doc, want) in [
        (
            "map(recurse(.[]?; true))",
            "[[1e1000, 1.5]]",
            "[[1E+1000,1.5],1E+1000,1.5]\n",
        ),
        ("last(recurse(.[]?; true))", "[[1e1000, 1.5]]", "1.5\n"),
        (
            "last(recurse(.[]?; false))",
            "[1E400, -1E400]",
            "[1E+400,-1E+400]\n",
        ),
        (
            "nth(1; recurse(.[]?; true))",
            "[[1e1000, 1.5]]",
            "[1E+1000,1.5]\n",
        ),
    ] {
        let (stdout, stderr, code) = run_jq_full(&["-c", filter], Some(doc))?;
        assert_eq!(code, 0, "`{filter}` -- {stderr:?}");
        assert_eq!(stdout, want, "`{filter}`");
    }
    Ok(())
}
