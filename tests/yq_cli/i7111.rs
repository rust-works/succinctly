//! #7111: through an unexploded alias, yq answers `length` with `0`, a slice with `[]` and refuses
//! `keys`, because those three read the alias node's own kind and content (empty), where indexing,
//! iteration and a field read through it to the anchored value (`.c[0]`, `.c[]`, `.c.p`).
//! `explode(.)` leaves no alias, and the anchor's own position is not an alias, so both keep answering
//! from the value. `succinctly yq` read the anchored value for all three.
//!
//! The rule holds where the value is still read from the document (a cursor): a path, a pipe stage,
//! a variable, an array or object built from it that is read back, `select` and `map` over the root.
//! Where the alias has been copied out (the right side of an assignment, `to_entries`, `map_values`,
//! `[.c, .n] | map(length)`) it is the anchored value again, as recorded in limitations.md.
//!
//! Every row was captured from yq v4.53.3 (`yq -o=json -I=0`); a refused row is checked for the
//! refusal alone (yq words it `cannot get keys of , keys only works for maps and arrays`).

use anyhow::Result;

use crate::run_yq_stdin;

const D1: &str =
    "a: [1, 2, 3, 4]\nb: &x [5, 6]\nc: *x\nm: &y {p: 1, q: 2}\nn: *y\ns: &z hello\nt: *z\n";
const D2: &str = "x: &a {k: [1, 2, 3], s: hi}\ny: *a\nz:\n  - *a\n  - [*a, 7]\nw: &e []\nv: *e\n";

/// `(document, filter, stdout)`.
const ROWS: &[(&str, &str, &str)] = &[
    (D1, ".c | length", "0\n"),
    (D1, ".c[0:1]", "[]\n"),
    (D1, ".c[1:]", "[]\n"),
    (D1, ".c[-1:]", "[]\n"),
    (D1, ".c | .[0:1]", "[]\n"),
    (D1, ".n | length", "0\n"),
    (D1, ".n[0:1]", "[]\n"),
    (D1, ".n[1:]", "[]\n"),
    (D1, ".t | length", "0\n"),
    (D1, ".t[0:1]", "[]\n"),
    (D1, ".b | length", "2\n"),
    (D1, ".b | keys", "[0,1]\n"),
    (D1, ".b[0:1]", "[5]\n"),
    (D1, ".m | length", "2\n"),
    (D1, ".m | keys", "[\"p\",\"q\"]\n"),
    (D1, ".s | length", "5\n"),
    (D1, ".c[0]", "5\n"),
    (D1, ".c[-1]", "6\n"),
    (D1, ".c[]", "5\n6\n"),
    (D1, ".c | map(. + 1)", "[6,7]\n"),
    (D1, ".c | .[]", "5\n6\n"),
    (D1, ".n.p", "1\n"),
    (D1, ".n.q", "2\n"),
    (D1, ".n | .[]", "1\n2\n"),
    (D1, ".n | map_values(. + 1)", "{\"p\":2,\"q\":3}\n"),
    (D1, "explode(.) | .c[0:1]", "[5]\n"),
    (D1, "explode(.) | .c | length", "2\n"),
    (D1, "explode(.) | .n | length", "2\n"),
    (D1, "explode(.) | .n | keys", "[\"p\",\"q\"]\n"),
    (D1, "explode(.) | .t | length", "5\n"),
    (D1, "[.c] | .[0] | length", "0\n"),
    (D1, ".c as $v | $v | length", "0\n"),
    (D1, "(.c, .a) | length", "0\n4\n"),
    (D1, ".c | select(length > 0)", ""),
    (D1, ".c | select(length == 0)", "[5,6]\n"),
    (D1, "[.[] | length]", "[4,2,0,2,0,5,0]\n"),
    (D1, "map(length)", "[4,2,0,2,0,5,0]\n"),
    (D1, "[.. | length]", "[7,4,1,1,1,1,2,1,1,0,2,1,1,0,5,0]\n"),
    (D1, "[.. | select(tag == \"!!seq\") | length]", "[4,2]\n"),
    (
        D1,
        "(.[] | select(length == 0))",
        "[5,6]\n{\"p\":1,\"q\":2}\n\"hello\"\n",
    ),
    (D1, ".c | length > 1", "false\n"),
    (D1, ".c | length == 0", "true\n"),
    (D1, ".c | [length]", "[0]\n"),
    (D1, ".c | {\"n\": length}", "{\"n\":0}\n"),
    (D1, ".c | length | . + 1", "1\n"),
    (D1, ".a | length", "4\n"),
    (D1, ".a[1:3]", "[2,3]\n"),
    (D1, ".c | type", "\"\"\n"),
    (D1, ".c | tag", "\"\"\n"),
    (D1, ".c | line", "3\n"),
    (D2, ".y | length", "0\n"),
    (D2, ".y.k | length", "3\n"),
    (D2, ".y.k[0:1]", "[1]\n"),
    (D2, ".z[0] | length", "0\n"),
    (D2, ".z[1][0] | length", "0\n"),
    (D2, ".z[] | length", "0\n2\n"),
    (D2, ".v | length", "0\n"),
    (D2, ".v[0:1]", "[]\n"),
    (D2, ".w | length", "0\n"),
    (D2, "[.. | length]", "[5,2,3,1,1,1,2,0,2,0,2,0,1,0,0]\n"),
    (
        D2,
        "explode(.) | [.. | length]",
        "[5,2,3,1,1,1,2,2,3,1,1,1,2,2,2,3,1,1,1,2,2,2,3,1,1,1,2,1,0,0]\n",
    ),
    (D2, "(.x, .y) | length", "2\n0\n"),
];

/// `(document, filter)`: refused by yq and here.
const REFUSED: &[(&str, &str)] = &[
    (D1, ".c | keys"),
    (D1, ".n | keys"),
    (D1, ".t | keys"),
    (D1, ".c | if length == 0 then \"alias\" else \"anchor\" end"),
    (D2, ".y | keys"),
    (D2, ".z[1][0] | keys"),
    (D2, ".v | keys"),
];

#[test]
fn length_a_slice_and_keys_read_an_alias_node_as_yq_does_7111() -> Result<()> {
    for &(doc, filter, want) in ROWS {
        assert_eq!(
            run_yq_stdin(filter, doc, &["-o=json", "-I=0"])?,
            (want.to_string(), 0),
            "{filter}"
        );
    }
    Ok(())
}

#[test]
fn keys_of_an_alias_is_refused_7111() -> Result<()> {
    for &(doc, filter) in REFUSED {
        let (stdout, code) = run_yq_stdin(filter, doc, &["-o=json", "-I=0"])?;
        assert_eq!((stdout.as_str(), code), ("", 1), "{filter}");
    }
    Ok(())
}

#[test]
fn the_same_filters_in_jq_mode_and_on_json_are_unchanged_7111() -> Result<()> {
    // No alias exists in JSON, and `succinctly jq` reads the value, as jq does.
    let json = r#"{"b":[5,6],"c":[5,6]}"#;
    let jq = |filter: &str| -> Result<String> {
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_succinctly"))
            .args(["jq", "-c", filter])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .and_then(|mut child| {
                use std::io::Write;
                child.stdin.take().unwrap().write_all(json.as_bytes())?;
                child.wait_with_output()
            })?;
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    };
    assert_eq!(jq(".c | length")?, "2\n");
    assert_eq!(jq(".c[0:1]")?, "[5]\n");
    assert_eq!(jq(".c | keys")?, "[0,1]\n");
    assert_eq!(
        run_yq_stdin(".c | length", json, &["-p=json", "-o=json"])?,
        ("2\n".to_string(), 0)
    );
    Ok(())
}
