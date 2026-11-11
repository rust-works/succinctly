//! #6899: a zero-padded decimal with an `8` or a `9` (`08`, `0009`, `-09`) is the float `8.0` in yq's JSON
//! encoder (#6866), on every route -- not only the ones that print the document node. A value that is
//! bound, copied, merged, written through or rebuilt (`. as $x | $x`, `. * .`, `.zz = 1`, `with_entries`,
//! `tojson`) is an owned value holding `NumberLiteral(Float(8.0), "08")`, and the JSON writers used to ask
//! the text alone, which a decimal parse reads as the integer `8`. The repr tells the two apart now.
//!
//! Every row is captured from yq v4.53.3. The spellings that stay integers (`007`) are in the document too.

use anyhow::Result;

use crate::run_yq_stdin;

const DOC: &str =
    "a: 00000000000000000000000008\nb: 08\nc: [08, 0009]\nd: -09\ne: 007\nf: {k: 09}\n";

/// `(extra arguments, filter, expected output)`.
const ROWS: &[(&[&str], &str, &str)] = &[
    (
        &["-o=json", "-I0"],
        ".",
        "{\"a\":8.0,\"b\":8.0,\"c\":[8.0,9.0],\"d\":-9.0,\"e\":7,\"f\":{\"k\":9.0}}\n",
    ),
    (
        &["-o=json", "-I0"],
        ". as $x | $x",
        "{\"a\":8.0,\"b\":8.0,\"c\":[8.0,9.0],\"d\":-9.0,\"e\":7,\"f\":{\"k\":9.0}}\n",
    ),
    (
        &["-o=json", "-I0"],
        ". * .",
        "{\"a\":8.0,\"b\":8.0,\"c\":[8.0,9.0],\"d\":-9.0,\"e\":7,\"f\":{\"k\":9.0}}\n",
    ),
    (
        &["-o=json", "-I0"],
        ". + .",
        "{\"a\":8.0,\"b\":8.0,\"c\":[8.0,9.0],\"d\":-9.0,\"e\":7,\"f\":{\"k\":9.0}}\n",
    ),
    (
        &["-o=json", "-I0"],
        ".zz = 1",
        "{\"a\":8.0,\"b\":8.0,\"c\":[8.0,9.0],\"d\":-9.0,\"e\":7,\"f\":{\"k\":9.0},\"zz\":1}\n",
    ),
    (
        &["-o=json", "-I0"],
        ".b = .b",
        "{\"a\":8.0,\"b\":8.0,\"c\":[8.0,9.0],\"d\":-9.0,\"e\":7,\"f\":{\"k\":9.0}}\n",
    ),
    (
        &["-o=json", "-I0"],
        "with_entries(.)",
        "{\"a\":8.0,\"b\":8.0,\"c\":[8.0,9.0],\"d\":-9.0,\"e\":7,\"f\":{\"k\":9.0}}\n",
    ),
    (
        &["-o=json", "-I0"],
        "to_entries | from_entries",
        "{\"a\":8.0,\"b\":8.0,\"c\":[8.0,9.0],\"d\":-9.0,\"e\":7,\"f\":{\"k\":9.0}}\n",
    ),
    (
        &["-o=json", "-I0"],
        "map_values(.)",
        "{\"a\":8.0,\"b\":8.0,\"c\":[8.0,9.0],\"d\":-9.0,\"e\":7,\"f\":{\"k\":9.0}}\n",
    ),
    (&["-o=json", "-I0"], "[.b]", "[8.0]\n"),
    (&["-o=json", "-I0"], "{\"k\": .b}", "{\"k\":8.0}\n"),
    (&["-o=json", "-I0"], ".b | tojson", "\"8.0\\n\"\n"),
    (&["-o=json", "-I0"], ".c | @json", "\"[8.0,9.0]\"\n"),
    (
        &["-o=json", "-I0"],
        ".f | tojson",
        "\"{\\n  \\\"k\\\": 9.0\\n}\\n\"\n",
    ),
    (&["-o=json", "-I0"], ".d | tojson", "\"-9.0\\n\"\n"),
    (&["-o=json", "-I0"], ".e | tojson", "\"7\\n\"\n"),
    (
        &["-o=json", "-I0"],
        "[.b, .c[1], .d] | sort",
        "[-9.0,8.0,9.0]\n",
    ),
    (&["-o=json", "-I0"], ".b | tostring", "\"08\"\n"),
    (&["-o=json", "-I0"], ".b | tag", "\"!!float\"\n"),
    (&["-o=json", "-I0"], ". as $x | $x.b | tag", "\"!!float\"\n"),
    (
        &[],
        ". as $x | $x",
        "a: 00000000000000000000000008\nb: 08\nc: [08, 0009]\nd: -09\ne: 007\nf: {k: 09}\n",
    ),
    (
        &[],
        ".zz = 1",
        "a: 00000000000000000000000008\nb: 08\nc: [08, 0009]\nd: -09\ne: 007\nf: {k: 09}\nzz: 1\n",
    ),
    (
        &[],
        ". * .",
        "a: 00000000000000000000000008\nb: 08\nc: [08, 0009]\nd: -09\ne: 007\nf: {k: 09}\n",
    ),
    (
        &[],
        "with_entries(.)",
        "a: 00000000000000000000000008\nb: 08\nc: [08, 0009]\nd: -09\ne: 007\nf: {k: 09}\n",
    ),
];

#[test]
fn a_zero_padded_float_prints_as_a_float_on_every_route_6899() -> Result<()> {
    for (extra, filter, want) in ROWS {
        let (out, code) = run_yq_stdin(filter, DOC, extra)?;
        assert_eq!(code, 0, "{extra:?} {filter}");
        assert_eq!(out, *want, "{extra:?} {filter}");
    }
    Ok(())
}
