//! #4095: `recurse(.[]?; cond)` and the jq-mode input stream. Making the walk a navigational stage
//! for the position walk must not make jq's `first(recurse(.[]?; input))` read inputs jq never
//! reads: `cond` is asked only of the nodes the consumer reaches. jq 1.7.1 reads one input per
//! `cond` call, so after `first` returns the root nothing is consumed.

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

/// jq mode shares the cursor walk's verdict for `. == null` and `. == true`, read off the node
/// without decoding it; it has to agree with the general route on JSON's own spellings.
#[test]
fn test_recurse_cond_null_and_bool_verdicts_agree_with_the_general_route_4095() -> Result<()> {
    let doc = r#"{"a":null,"b":true,"c":false,"d":0,"e":"","f":"null","g":"true","h":[],"i":{},"j":[null,true,[false,{"k":null}]],"l":1.0,"m":-0}"#;
    for cond in [
        r#"type == "object""#,
        r#"type != "string""#,
        r#""number" == type"#,
        r#"type == "object" or type == "array""#,
        r#"type == "number" and . != 0"#,
        ". == null",
        ". != null",
        "null == .",
        ". == true",
        ". != false",
        ". == false",
        ". == null or . == false",
        ". != null and . != false",
        ". == true | not",
    ] {
        let general = format!("[recurse(.[]?; . | ({cond}))]");
        let shallow = format!("[recurse(.[]?; {cond})]");
        let (want, _, c1) = run_jq_full(&["-c", &general], Some(doc))?;
        let (got, stderr, c2) = run_jq_full(&["-c", &shallow], Some(doc))?;
        assert_eq!((c1, c2), (0, 0), "`{cond}` -- {stderr:?}");
        assert!(!want.trim().is_empty(), "`{cond}` produced nothing");
        assert_eq!(got, want, "`{cond}`");
    }
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
        ". != null",
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
