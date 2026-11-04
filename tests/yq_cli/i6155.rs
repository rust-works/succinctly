//! #6155: yq reads `.[N]` on a mapping as a lookup of the key `N`, and under its read-only rule (an
//! arithmetic or boolean operand, an assignment's right side, a `select` condition) a key that is
//! absent finds no node. `succinctly yq` read it as `null` there, so `.a + .[0]` raised a type
//! error where yq answers `1`. Every expected output is what yq v4.53.3 prints (`-o=json -I=0`).

use anyhow::Result;

use crate::run_yq_stdin;

const DOC: &str = "a: 1\nb: 2\n";

/// (filter, expected stdout), over [`DOC`].
const ROWS: &[(&str, &str)] = &[
    (".a + .[0]", "1\n"),
    (".a - .[0]", ""),
    (".a / .[0]", ""),
    (".a * .[0]", ""),
    (".a + .[5]", "1\n"),
    (".x = .a + .[0]", "{\"a\":1,\"b\":2,\"x\":1}\n"),
    ("select(.[0] | not)", ""),
    ("select(.[0] == null)", "{\"a\":1,\"b\":2}\n"),
    (".a == .[0]", "false\n"),
    ("[.a, .[0]]", "[1,null]\n"),
    (".a and .[0]", "false\n"),
    (".a + .zz.y", "1\n"),
    (".a + .[0].y", "1\n"),
    (".x = .[0]", "{\"a\":1,\"b\":2,\"x\":null}\n"),
    (".a + (.[0] // 5)", "6\n"),
    ("map(. + .[0])", "[1,2]\n"),
];

#[test]
fn an_integer_index_that_misses_a_mapping_finds_no_node_when_read_only_6155() -> Result<()> {
    for &(filter, expected) in ROWS {
        let (output, code) = run_yq_stdin(filter, DOC, &["-o=json", "-I=0"])?;
        assert_eq!(code, 0, "{filter}");
        assert_eq!(output, expected, "`{filter}`");
    }
    Ok(())
}

/// A sequence is not a mapping: an index past its end is still a `null` read, and the arithmetic
/// over it still raises, as yq does.
#[test]
fn an_integer_index_past_a_sequence_is_still_null_6155() -> Result<()> {
    let (output, code) = run_yq_stdin(".[0] + .[1]", "- 1\n- 2\n", &["-o=json", "-I=0"])?;
    assert_eq!((output.as_str(), code), ("3\n", 0));
    let (_, code) = run_yq_stdin(".[0] + .[5]", "- 1\n- 2\n", &[])?;
    assert_eq!(code, 1);
    Ok(())
}
