//! #6092: yq writes a mapping or sequence into an interpolated string as its YAML text, in the style the
//! node was written in (`I {an: apple}`), where `succinctly yq` wrote jq's JSON text (`I {"an":"apple"}`).
//! The `ROWS` expectations are live yq v4.53.3 captures over the document below.

use anyhow::Result;

use crate::run_yq_stdin;

const DOC: &str = "m: {an: apple}\nl: [a, 2]\nnested: {a: {b: [1, {c: d}]}}\nplain:\n  a: 1\n  b: two\nseqb:\n  - x\n  - y: 1\ne: {}\nel: []\nn: null\nt: true\nnum: 3\nf: 1.50\ns: str\n";

/// (filter, expected output as JSON).
const ROWS: &[(&str, &str)] = &[
    (r#""I \(.m)""#, r#""I {an: apple}""#),
    (r#""I \(.l)""#, r#""I [a, 2]""#),
    (r#""I \(.nested)""#, r#""I {a: {b: [1, {c: d}]}}""#),
    (r#""I \(.plain)""#, r#""I a: 1\nb: two""#),
    (r#""I \(.seqb)""#, r#""I - x\n- y: 1""#),
    (r#""\(.e) \(.el)""#, r#""{} []""#),
    (
        r#""\(.n) \(.t) \(.num) \(.f) \(.s)""#,
        r#""null true 3 1.50 str""#,
    ),
    (r#""\(.m) \(.plain)""#, r#""{an: apple} a: 1\nb: two""#),
    (r#""a\((.num, .f))b""#, r#""a3b""#),
    (r#"[.l[] | "<\(.)>"]"#, r#"["<a>","<2>"]"#),
    (r#".plain | "\(.)""#, r#""a: 1\nb: two""#),
    (r#""\(.m | .an)""#, r#""apple""#),
    (r#"{"k": "v=\(.m)"}"#, r#"{"k":"v={an: apple}"}"#),
    (r#""\(.missing)""#, r#""null""#),
    // A slot with several outputs contributes its first.
    (r#""\(.l[])""#, r#""a""#),
    (r#""a\(.l[])b\(.num)""#, r#""aab3""#),
];

#[test]
fn a_container_interpolates_as_its_yaml_text() -> Result<()> {
    for (filter, want) in ROWS {
        let (out, code) = run_yq_stdin(filter, DOC, &["-o=json", "-I=0"])?;
        assert_eq!(code, 0, "{filter}");
        assert_eq!(out.trim_end(), *want, "{filter}");
    }
    Ok(())
}

/// `//` of navigation is read at the cursor, so a flow container keeps its flow style (the string
/// literal inside the slot is jq syntax, so these rows need `--jq-extensions`; yq's lexer rejects it).
#[test]
fn an_alternative_slot_keeps_the_nodes_style() -> Result<()> {
    for (filter, want) in [
        (r#""\(.m // "d")""#, r#""{an: apple}""#),
        (r#""\(.l // "d")""#, r#""[a, 2]""#),
        (r#""\(.zz // "d")""#, r#""d""#),
    ] {
        let (out, code) = run_yq_stdin(filter, DOC, &["--jq-extensions", "-o=json", "-I=0"])?;
        assert_eq!(code, 0, "{filter}");
        assert_eq!(out.trim_end(), want, "{filter}");
    }
    Ok(())
}

/// Known gap, pinned so a change is deliberate: a write's right-hand side is read from an owned
/// copy, which carries no node style, so a flow container interpolates as block. yq writes
/// `got {an: apple}` here.
#[test]
fn a_write_right_hand_side_interpolates_a_flow_container_as_block_gap_6092() -> Result<()> {
    let (out, code) = run_yq_stdin(r#".x = "got \(.m)" | .x"#, DOC, &["-o=json"])?;
    assert_eq!(code, 0);
    assert_eq!(out.trim_end(), r#""got an: apple""#);
    Ok(())
}

/// Not an oracle comparison: both sides are `succinctly yq`, so it pins only that the two spellings
/// share one definition. The `ROWS` above carry the yq captures.
#[test]
fn a_container_interpolates_as_to_string_does() -> Result<()> {
    for slot in [".m", ".l", ".nested", ".plain", ".seqb", ".e", ".el"] {
        let (interpolated, _) = run_yq_stdin(&format!(r#""\({slot})""#), DOC, &["-o=json"])?;
        let (stringified, _) = run_yq_stdin(&format!("{slot} | to_string"), DOC, &["-o=json"])?;
        assert_eq!(interpolated, stringified, "{slot}");
    }
    Ok(())
}
