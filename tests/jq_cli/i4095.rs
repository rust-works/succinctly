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
