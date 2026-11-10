//! #6821: yq evaluates a bare `$name` stage **once** over the whole list of nodes its pipe produced,
//! where `succinctly yq` read it once per node: `.n as $v | .x[] | $v` is one `7`, `.x | map($v)` is
//! `[7]`, and `.x[] | select(. > 5) | $v` is one `7` from an empty context. A `$name` that is not a
//! stage (`[$v]`, `{"k": $v}`, `. + $v`, an `as` body) is still read per node. `yq`'s `GET_VARIABLE`
//! returns the list the variable is stored in, and a pipe runs its right side once over the list the
//! left side left. Every row is `yq -o=json -I0 FILTER` on [`DOC`], captured from yq v4.53.3.

use anyhow::Result;

use crate::run_yq_stdin;

const DOC: &str = "x: [1, 2, 3]\nn: 7\nm: {a: 1}\ns: hi\ne: []\nem: {}\nsc: 5\nnu: null\n";

/// (filter, stdout), each the same as yq v4.53.3's with exit code 0.
const ROWS: &[(&str, &str)] = &[
    (".n as $v | .x[] | $v", "7\n"),
    (".n as $v | .x | map($v)", "[7]\n"),
    (".n as $v | [.x[] | $v]", "[7]\n"),
    (".n as $v | .x[] | $v | . + 1", "8\n"),
    (".n as $v | .x[] | ($v | . + 1)", "8\n"),
    (".n as $v | .x[] | $v, 5", "7\n5\n"),
    (".n as $v | .x[] | select(. > 1) | $v", "7\n"),
    (".n as $v | .x[] | select(. > 5) | $v", "7\n"),
    (".n as $v | .zz[] | $v", "7\n"),
    (".n as $v | .m | .[] | $v", "7\n"),
    (".m as $v | .x[] | $v", "{\"a\":1}\n"),
    (".m as $v | .x[] | $v.a", "1\n"),
    (".m as $v | .x | map($v.a)", "[1]\n"),
    (".m as $v | .x | map($v | keys)", "[[\"a\"]]\n"),
    (".x as $v | .x[] | $v", "[1,2,3]\n"),
    (".x as $v | .x[] | $v[0]", "1\n"),
    (".n as $v | .x | map(select(. > 1) | $v)", "[7]\n"),
    (".n as $v | .x | map(select(. > 5) | $v)", "[7]\n"),
    (".n as $v | .x | map($v) | length", "1\n"),
    (".n as $v | .x[] | $v | tostring", "\"7\"\n"),
    (".n as $v | .x[] | tostring | $v", "7\n"),
    (".n as $v | .x[] | $v | select(. > 1)", "7\n"),
    (".n as $v | .x[] | (select(. > 1) | $v)", "7\n"),
    (".n as $v | .x[] | .a? | $v", "7\n"),
    (".n as $v | .x[] | $v | $v", "7\n"),
    (".n as $a | .s as $b | .x[] | $a | $b", "\"hi\"\n"),
    (".n as $a | .s as $b | .x[] | $a, $b", "7\n"),
    (".n as $v | .x | to_entries | .[] | $v", "7\n"),
    (".n as $v | (.x[], .x[]) | $v", "7\n"),
    (".n as $v | .x[] | $v | path", "[\"n\"]\n"),
    (".n as $v | .x[] | $v | key", "\"n\"\n"),
    (".n as $v | .x[] | [$v]", "[7]\n[7]\n[7]\n"),
    (".n as $v | .x[] | {\"k\": $v}", "{\"k\":7}\n{\"k\":7}\n{\"k\":7}\n"),
    (".n as $v | .x[] | . + $v", "8\n9\n10\n"),
    (".n as $v | .x[] | $v + .", "8\n9\n10\n"),
    (".n as $v | .x[] | . as $w | $v", "7\n7\n7\n"),
    (".n as $v | .x[] | (. as $w | $v)", "7\n7\n7\n"),
    (".n as $v | .x | map(. as $w | $v)", "[7,7,7]\n"),
    (".n as $v | .x | map([$v])", "[[7],[7],[7]]\n"),
    (".n as $v | .x | map($v * 2)", "[14,14,14]\n"),
    (".n as $v | .x | map(. * $v)", "[7,14,21]\n"),
    (".x[] as $v | $v", "1\n2\n3\n"),
    (".x[] as $v | .n", "7\n7\n7\n"),
    (".n as $v | .e | map($v)", "[]\n"),
    (".n as $v | .em | map($v)", "{}\n"),
    (".n as $v | .sc | map($v)", "5\n"),
    (".n as $v | .nu | map($v)", "[]\n"),
    (".n as $v | .zz | map($v)", "[]\n"),
    (".n as $v | .e | [.[] | $v]", "[7]\n"),
    (".n as $v | .sc | [.[] | $v]", "[7]\n"),
    (".n as $v | .x[] |= $v", "{\"x\":[7,7,7],\"n\":7,\"m\":{\"a\":1},\"s\":\"hi\",\"e\":[],\"em\":{},\"sc\":5,\"nu\":null}\n"),
    ("(.n as $v | .x[] | $v) = 5", "{\"x\":[1,2,3],\"n\":7,\"m\":{\"a\":1},\"s\":\"hi\",\"e\":[],\"em\":{},\"sc\":5,\"nu\":null}\n"),
];

#[test]
fn every_row_matches_yq_6821() -> Result<()> {
    for (filter, expected) in ROWS {
        let (stdout, code) = run_yq_stdin(filter, DOC, &["-o", "json", "-I0"])?;
        assert_eq!(code, 0, "{filter}");
        assert_eq!(&stdout, expected, "{filter}");
    }
    Ok(())
}

/// An error upstream of the variable still ends the run: the stages before it are evaluated, not skipped.
#[test]
fn an_error_before_the_variable_stage_still_raises_6821() -> Result<()> {
    let (stdout, code) = run_yq_stdin(
        ".n as $v | .x[] | error(\"e\") | $v",
        DOC,
        &["-o", "json", "-I0"],
    )?;
    assert_ne!(code, 0);
    assert_eq!(stdout, "");
    Ok(())
}

/// YAML output prints the one node as yq does.
#[test]
fn the_one_node_prints_as_yaml_6821() -> Result<()> {
    let (stdout, code) = run_yq_stdin(".m as $v | .x[] | $v", DOC, &[])?;
    assert_eq!(code, 0);
    assert_eq!(stdout, "a: 1\n");
    Ok(())
}

/// jq mode reads `$name` per node, as jq does: `1,2,3 | $v` is three values.
#[test]
fn jq_mode_still_reads_the_variable_per_node_6821() -> Result<()> {
    let (stdout, code) = crate::run_jq_stdin(". as $v | [1, 2, 3] | .[] | $v", "7", &["-c"])?;
    assert_eq!(code, 0);
    assert_eq!(stdout, "7\n7\n7\n");
    Ok(())
}
