//! #7999: yq indexes a mapping by a bracket key's text and a sequence by its value. A hex key
//! (`0x1`, #7615) or a decimal one with leading zeros (`01`) on a mapping names the key `0x1` or
//! `01`; the parser used to fold it to the integer index `1`, so a read, write or `del` landed on
//! the key `1`, one the filter never named. In yq mode `01` is now the spelled integer a
//! document's `01` is, as `0x1` already was. Every yq row is a live yq v4.53.3 capture; the jq
//! rows are jq 1.7.1's, and jq mode is unchanged.

use std::io::Write;

use anyhow::Result;
use tempfile::NamedTempFile;

use crate::{run_jq_stdin, run_jq_stdin_with_stderr, run_yq_file, run_yq_stdin};

fn yq_json(filter: &str, input: &str, extra: &[&str]) -> Result<(String, i32)> {
    let mut args = vec!["-o=json", "-I0"];
    args.extend_from_slice(extra);
    let (out, code) = run_yq_stdin(filter, input, &args)?;
    Ok((out.trim_end().to_string(), code))
}

/// A read through a spelled key on a mapping takes the key with that text.
#[test]
fn test_spelled_bracket_key_reads_a_mapping_by_its_text_7999() -> Result<()> {
    let input = "m: {\"0x1\": 5, \"1\": 6, \"01\": 7, \"0X1\": 8}\n";
    for (filter, want) in [
        (".m[0x1]", "5"),
        (".m[0X1]", "8"),
        (".m[01]", "7"),
        (".m[(0x1)]", "5"),
        (".m[((0x1))]", "5"),
        (".m.[0x1]", "5"),
        (".m | .[0x1]", "5"),
        (".m | .[(0x1)]", "5"),
        (".m[0x1]?", "5"),
        (".m.[0x1]?", "5"),
        (".m[0x1, 1]", "5\n6"),
        (".m[0x01]", "null"),
        (".m[00]", "null"),
        // A canonical key reads as before.
        (".m[1]", "6"),
        (".m[0]", "null"),
    ] {
        assert_eq!(
            yq_json(filter, input, &[])?,
            (want.to_string(), 0),
            "{filter}"
        );
    }
    Ok(())
}

/// A write, an update and a delete land on the key the filter names, not on `1`.
#[test]
fn test_spelled_bracket_key_writes_the_mapping_key_it_names_7999() -> Result<()> {
    let input = "m: {\"1\": 6, \"0x1\": 5}\n";
    for (filter, want) in [
        (".m[0x1] = 9", r#"{"m":{"1":6,"0x1":9}}"#),
        (".m[0x1] |= 9", r#"{"m":{"1":6,"0x1":9}}"#),
        (".m[0x1] |= . + 1", r#"{"m":{"1":6,"0x1":6}}"#),
        (".m[0x1] += 1", r#"{"m":{"1":6,"0x1":6}}"#),
        (".m[0x1] *= 2", r#"{"m":{"1":6,"0x1":10}}"#),
        ("(.m[0x1]) = 9", r#"{"m":{"1":6,"0x1":9}}"#),
        (".m[(0x1)] = 9", r#"{"m":{"1":6,"0x1":9}}"#),
        (".m.[0x1] = 9", r#"{"m":{"1":6,"0x1":9}}"#),
        (".m | .[(0x1)] = 9", r#"{"1":6,"0x1":9}"#),
        ("with(.m[0x1]; . = 9)", r#"{"m":{"1":6,"0x1":9}}"#),
        (".m[01] = 9", r#"{"m":{"1":6,"0x1":5,"01":9}}"#),
        (".m[01] |= 9", r#"{"m":{"1":6,"0x1":5,"01":9}}"#),
        ("del(.m[0x1])", r#"{"m":{"1":6}}"#),
        ("del(.m[(0x1)])", r#"{"m":{"1":6}}"#),
        (".m | del(.[0x1])", r#"{"1":6}"#),
        ("del(.m[01])", r#"{"m":{"1":6,"0x1":5}}"#),
        (".m | has(0x1)", "true"),
        (".m | has(01)", "false"),
        (".m[0x1] as $v | $v", "5"),
        // A canonical key writes as before.
        (".m[1] = 9", r#"{"m":{"1":9,"0x1":5}}"#),
    ] {
        assert_eq!(
            yq_json(filter, input, &[])?,
            (want.to_string(), 0),
            "{filter}"
        );
    }
    for (filter, input, want) in [
        (".e[0x1] = 9", "e: {}\n", r#"{"e":{"0x1":9}}"#),
        (".e[01] = 9", "e: {}\n", r#"{"e":{"01":9}}"#),
        (".e[1] = 9", "e: {}\n", r#"{"e":{"1":9}}"#),
        (".m[0x1]", "m: {\"1\": 6}\n", "null"),
        (".m[0x1] = 9", "m: {\"1\": 6}\n", r#"{"m":{"1":6,"0x1":9}}"#),
        ("del(.m[0x1])", "m: {\"1\": 6}\n", r#"{"m":{"1":6}}"#),
        (".m | has(0x1)", "m: {\"1\": 6}\n", "false"),
        (".m | has(0x1)", "m: {\"0x1\": 6}\n", "true"),
        (".n[0x1][0x1] = 3", "n: {}\n", r#"{"n":{"0x1":[null,3]}}"#),
        // An absent or `null` target still becomes a sequence: the key is a number.
        (".e[0x1] = 9", "m: {}\n", r#"{"m":{},"e":[null,9]}"#),
        (".e[01] = 9", "m: {}\n", r#"{"m":{},"e":[null,9]}"#),
        (".[0x1] = 1", "null\n", "[null,1]"),
        (".[01] = 1", "null\n", "[null,1]"),
        ("del(.[0x1])", "null\n", "[null]"),
        // A write through a sum is a no-op in yq whatever the key.
        (
            "(.a + .b)[0x1] = 9",
            "a: {\"0x1\": 1}\nb: {\"1\": 2}\n",
            r#"{"a":{"0x1":1},"b":{"1":2}}"#,
        ),
        ("(.a + .b)[0x1]", "a: {\"0x1\": 1}\nb: {\"1\": 2}\n", "1"),
    ] {
        assert_eq!(
            yq_json(filter, input, &[])?,
            (want.to_string(), 0),
            "{filter} over {input:?}"
        );
    }
    // The written key keeps its spelling in YAML.
    let (out, code) = run_yq_stdin(".m[01] = 9", "m: {\"1\": 6, \"0x1\": 5}\n", &[])?;
    assert_eq!(
        (out.as_str(), code),
        ("m: {\"1\": 6, \"0x1\": 5, 01: 9}\n", 0)
    );
    let (out, code) = run_yq_stdin(".e[0x1] = 9", "e: {}\n", &[])?;
    assert_eq!((out.as_str(), code), ("e:\n  0x1: 9\n", 0));
    Ok(())
}

/// The YAML a write prints keeps the document's presentation as the static `.s[1]` and `.m["0x1"]`
/// do: comments, quoting, flow style and anchors, by `eval` and by `eval-all` (whose plain-route
/// reroute takes a spelled key as navigation), on a file and on stdin. The positional fallback this
/// replaced moved `del(.s[01])`'s comments onto the wrong item and printed a quoted `"01"` key bare,
/// which reads back as an integer.
#[test]
fn test_spelled_bracket_key_keeps_presentation_7999() -> Result<()> {
    const FLOW: &str = "s: [a, b]\n\"01\": &k x # c1\n";
    const BLOCK: &str = "s:\n  - a # c0\n  - 'b' # c1\n  - \"c\" # c2\n";
    const ANCHORS: &str = "s: [&x {k: v}, &w {k: v}, *x]\n";
    const MAP: &str = "m:\n  \"0x1\": 'five' # h\n  \"1\": six # o\n  \"01\": &a seven # z\n  \"-0\": nz # n\nr: *a\n";
    const MAP_FLOW: &str = "m: {\"0x1\": 5, \"1\": 6} # fm\nq: \"x\" # tail\n";
    let rows: &[(&str, &str, &str)] = &[
        (".s[01] = 9", FLOW, "s: [a, 9]\n\"01\": &k x # c1\n"),
        (".s[0x1] |= \"z\"", FLOW, "s: [a, z]\n\"01\": &k x # c1\n"),
        ("del(.s[01])", FLOW, "s: [a]\n\"01\": &k x # c1\n"),
        ("del(.s[01])", BLOCK, "s:\n  - a # c0\n  - \"c\" # c2\n"),
        ("del(.s[0x0])", BLOCK, "s:\n  - 'b' # c1\n  - \"c\" # c2\n"),
        ("del(.s[-0])", BLOCK, "s:\n  - 'b' # c1\n  - \"c\" # c2\n"),
        ("del(.s[-01])", BLOCK, "s:\n  - a # c0\n  - 'b' # c1\n"),
        (".s[01] = \"q\"", BLOCK, "s:\n  - a # c0\n  - 'q' # c1\n  - \"c\" # c2\n"),
        (".s[0x2] |= \"q\"", BLOCK, "s:\n  - a # c0\n  - 'b' # c1\n  - \"q\" # c2\n"),
        ("del(.s[01])", ANCHORS, "s: [&x {k: v}, *x]\n"),
        (".s[0x1].k = \"z\"", ANCHORS, "s: [&x {k: v}, &w {k: z}, *x]\n"),
        (".s[0x0].k = \"z\"", ANCHORS, "s: [&x {k: z}, &w {k: v}, *x]\n"),
        (
            ".m[0x1] = 9",
            MAP,
            "m:\n  \"0x1\": 9 # h\n  \"1\": six # o\n  \"01\": &a seven # z\n  \"-0\": nz # n\nr: *a\n",
        ),
        (
            ".m[01] |= \"q\"",
            MAP,
            "m:\n  \"0x1\": 'five' # h\n  \"1\": six # o\n  \"01\": &a q # z\n  \"-0\": nz # n\nr: *a\n",
        ),
        (
            "del(.m[0x1])",
            MAP,
            "m:\n  \"1\": six # o\n  \"01\": &a seven # z\n  \"-0\": nz # n\nr: *a\n",
        ),
        (
            "del(.m[-0])",
            MAP,
            "m:\n  \"0x1\": 'five' # h\n  \"1\": six # o\n  \"01\": &a seven # z\nr: *a\n",
        ),
        (
            ".m[0x2] = 9",
            MAP,
            "m:\n  \"0x1\": 'five' # h\n  \"1\": six # o\n  \"01\": &a seven # z\n  \"-0\": nz # n\n  0x2: 9\nr: *a\n",
        ),
        (".m[0x1] = 9", MAP_FLOW, "m: {\"0x1\": 9, \"1\": 6} # fm\nq: \"x\" # tail\n"),
        ("del(.m[0x1])", MAP_FLOW, "m: {\"1\": 6} # fm\nq: \"x\" # tail\n"),
    ];
    for (filter, input, want) in rows {
        let mut file = NamedTempFile::new()?;
        file.write_all(input.as_bytes())?;
        let path = file.path().to_str().expect("utf-8 temp path");
        for mode in [&[][..], &["ea"][..]] {
            let (out, code) = run_yq_file(filter, path, mode)?;
            assert_eq!(
                (out.as_str(), code),
                (*want, 0),
                "{mode:?} {filter} over {input:?}"
            );
            let (out, code) = run_yq_stdin(filter, input, mode)?;
            assert_eq!(
                (out.as_str(), code),
                (*want, 0),
                "{mode:?} {filter} over stdin {input:?}"
            );
        }
    }
    // A copy takes the source's comment, as the static `.m["0x1"] = .m["01"]` does.
    let (out, code) = run_yq_stdin(".m[0x1] = .m[01]", MAP, &[])?;
    assert_eq!(
        (out.as_str(), code),
        (
            "m:\n  \"0x1\": 'seven' # z\n  \"1\": six # o\n  \"01\": &a seven # z\n  \"-0\": nz # n\nr: *a\n",
            0
        )
    );
    Ok(())
}

/// A spelled key in a pipe of writes, and as the navigation in front of a write or behind one,
/// keeps the presentation the static `.s[1]` and `.m["0x1"]` keep. A later stage's key under a node
/// an earlier stage renumbered is the slot of its value; a navigation in front of a metadata write
/// is taken whole, not refused as a write behind a navigation (#2679).
#[test]
fn test_spelled_bracket_key_in_a_pipe_keeps_presentation_7999() -> Result<()> {
    const NESTED: &str = "s:\n- [a] # c0\n- - p # p0\n  - \"q\" # q1\n- [c] # c2\n";
    const FLOWS: &str = "s:\n  - [a] # c0\n  - [b] # c1\n  - [c] # c2\n";
    const INNER: &str = "s:\n  - x\n  - - a # c0\n    - \"b\" # c1\n    - c # c2\n";
    const MAP_SEQ: &str = "m:\n  \"0x1\":\n    - p # p0\n    - \"q\" # q1\n  \"1\": [z] # z1\n";
    const MAP_MAP: &str = "m:\n  \"0x1\":\n    a: 1 # c\n    b: \"two\" # d\n  \"1\": {a: 3} # e\n";
    const ALIASED: &str = "a: &x 1\nb: *x\nm:\n  \"0x1\": null\n  k: 2\n";
    let rows: &[(&str, &str, &str)] = &[
        (
            "del(.s[0]) | del(.s[0][0x0])",
            NESTED,
            "s:\n  - - \"q\" # q1\n  - [c] # c2\n",
        ),
        (
            "del(.s[0x0]) | del(.s[0x0][0x0])",
            NESTED,
            "s:\n  - - \"q\" # q1\n  - [c] # c2\n",
        ),
        (
            "del(.s[0]) | .s[0][01] = 9",
            NESTED,
            "s:\n  - - p # p0\n    - 9 # q1\n  - [c] # c2\n",
        ),
        (
            "del(.s[0]) | .s[0][01] = 9",
            FLOWS,
            "s:\n  - [b, 9] # c1\n  - [c] # c2\n",
        ),
        (
            "del(.s[0x0]) | .s[0x0][01] = 9",
            FLOWS,
            "s:\n  - [b, 9] # c1\n  - [c] # c2\n",
        ),
        (
            ".s[0x1][0x0] = 9 | del(.s[0x0])",
            FLOWS,
            "s:\n  - [9] # c1\n  - [c] # c2\n",
        ),
        (".s[01] | del(.[0])", INNER, "- \"b\" # c1\n- c # c2\n"),
        (".s[01] | del(.[0x0])", INNER, "- \"b\" # c1\n- c # c2\n"),
        (
            ".s[01] | .[0] line_comment = \"z\"",
            INNER,
            "- a # z\n- \"b\" # c1\n- c # c2\n",
        ),
        (
            ".s[01] | .[0x1] = \"q\"",
            INNER,
            "- a # c0\n- \"q\" # c1\n- c # c2\n",
        ),
        (
            ".s[0x0] = \"y\" | .s[01]",
            INNER,
            "- a # c0\n- \"b\" # c1\n- c # c2\n",
        ),
        (".m[0x1] | del(.[0])", MAP_SEQ, "- \"q\" # q1\n"),
        (
            ".m[0x1] | .[0] line_comment = \"z\"",
            MAP_SEQ,
            "- p # z\n- \"q\" # q1\n",
        ),
        (
            "del(.m[0x1][0]) | .m[0x1][0] = \"r\"",
            MAP_SEQ,
            "m:\n  \"0x1\":\n    - \"r\" # q1\n  \"1\": [z] # z1\n",
        ),
        (".m[1] = 5 | .m[0x1]", MAP_SEQ, "- p # p0\n- \"q\" # q1\n"),
        (
            ".m[0x1] | .a line_comment = \"x\"",
            MAP_MAP,
            "a: 1 # x\nb: \"two\" # d\n",
        ),
        (".m[0x1] | .b = \"z\"", MAP_MAP, "a: 1 # c\nb: \"z\" # d\n"),
        (".m[0x1] | del(.a)", MAP_MAP, "b: \"two\" # d\n"),
        (
            ".m[0x1].a = 5 | .m[0x1]",
            MAP_MAP,
            "a: 5 # c\nb: \"two\" # d\n",
        ),
        // An alias copied into the key keeps its mark, by `+=` and by a stage of a chain.
        (
            ".m[0x1] += .b",
            ALIASED,
            "a: &x 1\nb: *x\nm:\n  \"0x1\": *x\n  k: 2\n",
        ),
        (
            ".m[0x1] = .b | .z = 1",
            ALIASED,
            "a: &x 1\nb: *x\nm:\n  \"0x1\": *x\n  k: 2\nz: 1\n",
        ),
    ];
    for (filter, input, want) in rows {
        let mut file = NamedTempFile::new()?;
        file.write_all(input.as_bytes())?;
        let path = file.path().to_str().expect("utf-8 temp path");
        let (out, code) = run_yq_file(filter, path, &[])?;
        assert_eq!((out.as_str(), code), (*want, 0), "{filter} over {input:?}");
        let (out, code) = run_yq_stdin(filter, input, &[])?;
        assert_eq!(
            (out.as_str(), code),
            (*want, 0),
            "{filter} over stdin {input:?}"
        );
    }
    Ok(())
}

/// A key spelled `-0` is the mapping key `-0` and the sequence slot 0, as in yq.
#[test]
fn test_minus_zero_bracket_key_7999() -> Result<()> {
    let input = "m: {\"-0\": 1, \"0\": 2}\ns: [a, b]\n";
    for (filter, want) in [
        (".m[-0]", "1"),
        (".m[0]", "2"),
        (".m[-0] = 9 | .m", r#"{"-0":9,"0":2}"#),
        ("del(.m[-0]) | .m", r#"{"0":2}"#),
        (".s[-0]", r#""a""#),
        ("del(.s[-0]) | .s", r#"["b"]"#),
    ] {
        assert_eq!(
            yq_json(filter, input, &[])?,
            (want.to_string(), 0),
            "{filter}"
        );
    }
    Ok(())
}

/// A sequence is still indexed by the key's value, and a slice bound is unchanged.
#[test]
fn test_spelled_bracket_key_indexes_a_sequence_by_value_7999() -> Result<()> {
    let input = "s: [a, b, c]\n";
    for (filter, want) in [
        (".s[0x1]", r#""b""#),
        (".s[01]", r#""b""#),
        (".s[0X2]", r#""c""#),
        (".s[(0x1)]", r#""b""#),
        (".s.[0x1]", r#""b""#),
        (".s | .[0x1]", r#""b""#),
        (".s[0x1]?", r#""b""#),
        (".s[0x1] = \"z\"", r#"{"s":["a","z","c"]}"#),
        (".s[01] = \"z\"", r#"{"s":["a","z","c"]}"#),
        (".s[0x1] |= \"z\"", r#"{"s":["a","z","c"]}"#),
        (".s[0x1] += \"q\"", r#"{"s":["a","bq","c"]}"#),
        (".s[0x5] = \"z\"", r#"{"s":["a","b","c",null,null,"z"]}"#),
        ("del(.s[0x1])", r#"{"s":["a","c"]}"#),
        ("del(.s[01])", r#"{"s":["a","c"]}"#),
        (".s[0x1:]", r#"["b","c"]"#),
        (".s[:0x2]", r#"["a","b"]"#),
        (".s[01:]", r#"["b","c"]"#),
        (".s[0x1:0x2]", r#"["b"]"#),
        (".s | pick([0x1])", r#"["b"]"#),
    ] {
        assert_eq!(
            yq_json(filter, input, &[])?,
            (want.to_string(), 0),
            "{filter}"
        );
    }
    for (filter, want) in [
        ("[1,2,3] | .[01]", "2"),
        ("[1,2,3] | .[-01]", "3"),
        ("[1,2,3] | .[-01:]", "[3]"),
        ("[1,2,3] | .[01:02]", "[2]"),
        ("{0x1: 2} | .[0x1]", "2"),
        ("{01: 2} | .[01]", "2"),
    ] {
        assert_eq!(
            yq_json(filter, "", &["-n"])?,
            (want.to_string(), 0),
            "{filter}"
        );
    }
    Ok(())
}

/// A large document takes the same rule on the file and stdin routes, sequence and mapping alike.
#[test]
fn test_spelled_bracket_key_on_a_large_document_7999() -> Result<()> {
    let mut yaml = String::new();
    for i in 0..20000 {
        yaml.push_str(&format!(
            "- {{\"0x1\": {i}, \"1\": {}, \"01\": {}}}\n",
            -i,
            i * 10
        ));
    }
    let mut file = NamedTempFile::new()?;
    file.write_all(yaml.as_bytes())?;
    let path = file.path().to_str().expect("utf-8 temp path");
    for (filter, want) in [
        (".[0x0] | .[0x1]", "0"),
        (".[0x0][0x1]", "0"),
        (".[0x0].[0x1]", "0"),
        (".[0x4e1f][0x1]", "19999"),
        (".[01][01]", "10"),
        (".[0x0][1]", "0"),
        (".[0][0x1]", "0"),
        (".[0x0][0x1] = 9 | .[0x0]", r#"{"0x1":9,"1":0,"01":0}"#),
        (".[0x0][01] |= 9 | .[0x0]", r#"{"0x1":0,"1":0,"01":9}"#),
        ("del(.[0x0][0x1]) | .[0x0]", r#"{"1":0,"01":0}"#),
    ] {
        let (out, code) = run_yq_file(filter, path, &["-o=json", "-I0"])?;
        assert_eq!((out.trim_end(), code), (want, 0), "{filter} on a file");
        assert_eq!(
            yq_json(filter, &yaml, &[])?,
            (want.to_string(), 0),
            "{filter} on stdin"
        );
    }
    Ok(())
}

/// In yq mode a `01` literal is the spelled integer a document's `01` is.
#[test]
fn test_leading_zero_literal_keeps_its_spelling_7999() -> Result<()> {
    for (filter, want) in [
        ("01", "1"),
        ("007", "7"),
        ("01 | tostring", r#""01""#),
        ("00 | tostring", r#""00""#),
        ("-01 | tostring", r#""-01""#),
        ("\"x\" + (01 | tostring)", r#""x01""#),
        ("01 as $x | $x | tostring", r#""01""#),
        ("[01] | .[0] | tostring", r#""01""#),
        ("1 == 01", "false"),
        ("01 == 01", "true"),
        ("01 == 1.0", "false"),
        ("01 == \"01\"", "true"),
        ("[01, 1] | unique", "[1,1]"),
        ("[00, 0] | unique", "[0,0]"),
        ("{01: 2}", r#"{"01":2}"#),
        ("{\"01\": 1} | has(01)", "true"),
        ("[1,2] | has(01)", "true"),
        ("01 + 1", "2"),
        ("1 - 01", "0"),
        ("01 * 2", "2"),
        ("01 < 2", "true"),
        ("01 | tag", r#""!!int""#),
        ("01 | @json", r#""1""#),
        ("01 | @yaml", r#""01\n""#),
    ] {
        assert_eq!(
            yq_json(filter, "", &["-n", "--"])?,
            (want.to_string(), 0),
            "{filter}"
        );
    }
    for (filter, want) in [
        ("01", "01\n"),
        ("007", "007\n"),
        ("-01", "-01\n"),
        ("[01, 1]", "- 01\n- 1\n"),
        ("{01: 2}", "01: 2\n"),
        ("[01, 1] | sort", "- 01\n- 1\n"),
    ] {
        let (out, code) = run_yq_stdin(filter, "", &["-n", "--"])?;
        assert_eq!((out.as_str(), code), (want, 0), "{filter}");
    }
    let (out, code) = run_yq_stdin(".a = [01, 007] | .b = -01", "{}\n", &[])?;
    assert_eq!((out.as_str(), code), ("a:\n  - 01\n  - 007\nb: -01\n", 0));
    Ok(())
}

/// `path()` (a `--jq-extensions` builtin) names the mapping key by its text and the sequence
/// position by its value, as the read and the write do.
#[test]
fn test_spelled_bracket_key_path_under_jq_extensions_7999() -> Result<()> {
    let input = "m: {\"1\": 6, \"0x1\": 5}\ns: [a, b]\n";
    for (filter, want) in [
        ("path(.m[0x1])", r#"["m","0x1"]"#),
        ("path(.m[01])", r#"["m","01"]"#),
        ("path(.m | .[0x1])", r#"["m","0x1"]"#),
        ("path(.s[0x1])", r#"["s",1]"#),
    ] {
        assert_eq!(
            yq_json(filter, input, &["--jq-extensions"])?,
            (want.to_string(), 0),
            "{filter}"
        );
    }
    Ok(())
}

/// jq reads `01` as the number 1 (jq 1.7.1), so jq mode is unchanged.
#[test]
fn test_jq_mode_leading_zero_literal_is_unchanged_7999() -> Result<()> {
    for (filter, want) in [
        ("01", "1"),
        ("01 | tostring", r#""1""#),
        ("-01 | tostring", r#""-1""#),
        ("1 == 01", "true"),
        ("[01, 1] | unique", "[1]"),
        ("[1,2,3] | .[01]", "2"),
        ("[1,2,3] | .[01:]", "[2,3]"),
        ("path(.[01])", "[1]"),
        ("[1,2,3] | .[01] = 9", "[1,9,3]"),
    ] {
        let (out, code) = run_jq_stdin(filter, "", &["-n", "-c"])?;
        assert_eq!((out.trim_end(), code), (want, 0), "{filter}");
    }
    let (out, err, code) = run_jq_stdin_with_stderr("{\"1\": 5} | .[01]", "", &["-n"])?;
    assert_eq!((out.as_str(), code), ("", 5));
    assert!(err.contains("Cannot index object with number"), "{err}");
    Ok(())
}
