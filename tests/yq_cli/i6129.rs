//! #6129: yq's `map_values(f)` is `.[] |= f`, and an update whose right side yields nothing leaves
//! the value alone. `succinctly yq` answered jq's `map_values`, which deletes the entry. Every
//! expected output is what yq v4.53.3 prints for the same document and filter (`-o=json -I=0`).

use anyhow::Result;

use crate::run_yq_stdin;

const SEQ: &str = "[1, 2, 3]\n";
const MAP: &str = "a: 1\nb: 2\nc: 3\n";

/// (name, document, filter, expected stdout).
const ORACLE_ROWS: &[(&str, &str, &str, &str)] = &[
    ("select_seq", SEQ, "map_values(select(. > 1))", "[1,2,3]\n"),
    (
        "select_map",
        MAP,
        "map_values(select(. > 1))",
        "{\"a\":1,\"b\":2,\"c\":3}\n",
    ),
    (
        "iterate_of_empty_seqs",
        "- [1, 9]\n- []\n- [3]\n",
        "map_values(.[])",
        "[1,[],3]\n",
    ),
    (
        "stringify_then_select",
        SEQ,
        "map_values(tostring | select(. != \"2\"))",
        "[\"1\",2,\"3\"]\n",
    ),
    (
        "missing_field",
        "- x: 1\n- y: 2\n",
        "map_values(.x)",
        "[1,null]\n",
    ),
    (
        "first_of_two_outputs",
        MAP,
        "map_values(., 5)",
        "{\"a\":1,\"b\":2,\"c\":3}\n",
    ),
    ("arithmetic", SEQ, "map_values(. + 1)", "[2,3,4]\n"),
    (
        "identity_on_empty_map",
        "{}\n",
        "map_values(select(. > 1))",
        "{}\n",
    ),
    (
        "is_the_update",
        MAP,
        ".[] |= select(. > 1)",
        "{\"a\":1,\"b\":2,\"c\":3}\n",
    ),
];

#[test]
fn map_values_without_an_output_keeps_the_value_6129() -> Result<()> {
    for &(name, document, filter, expected) in ORACLE_ROWS {
        let (output, code) = run_yq_stdin(filter, document, &["-o=json", "-I=0"])?;
        assert_eq!(code, 0, "{name}");
        assert_eq!(output, expected, "{name}: `{filter}`");
    }
    Ok(())
}

/// The update keeps what the document wrote around a value `f` leaves alone: its comments, anchor,
/// alias and flow style (`default` YAML output, captured from yq v4.53.3).
#[test]
fn map_values_without_an_output_keeps_the_presentation_6129() -> Result<()> {
    for (document, filter, expected) in [
        (
            "a: &x 1  # c\nb: 2\n",
            "map_values(select(. > 5))",
            "a: &x 1 # c\nb: 2\n",
        ),
        (
            "# head\na: 1 # one\nb: \"two\"\nk: &k [3]\nc: *k\n",
            "map_values(select(. == 9))",
            "# head\na: 1 # one\nb: \"two\"\nk: &k [3]\nc: *k\n",
        ),
        (
            "a: [1, 2, 3]\n",
            ".a |= map_values(select(. > 1))",
            "a: [1, 2, 3]\n",
        ),
        ("5\n", "map_values(.)", "5\n"),
    ] {
        let (output, code) = run_yq_stdin(filter, document, &[])?;
        assert_eq!(code, 0, "{filter}");
        assert_eq!(output, expected, "`{filter}` over {document:?}");
    }
    Ok(())
}

/// jq's `map_values` is unchanged: an entry whose `f` yields nothing is dropped.
#[test]
fn jq_mode_map_values_still_drops_the_entry_6129() -> Result<()> {
    let (output, code) = crate::run_jq_stdin("map_values(select(. > 1))", "[1,2,3]", &["-c"])?;
    assert_eq!((output.as_str(), code), ("[2,3]\n", 0));
    Ok(())
}
