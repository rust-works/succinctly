//! #6637: `--argjson` and `--jsonargs` read a text as deep as jq's parser does. The validator they
//! went through stopped at its default of 128 containers, so a text of 129 to 256 nested arrays was
//! refused where jq reads it. jq's ceiling is its parser's stack of 256 entries, and an object takes
//! two of them (the object and its key), so a chain of objects is refused at 129 and a mix of the two
//! in between. Every row was captured from jq 1.7.1 (stdout and exit status; the usage text names jq's
//! own binary).

use anyhow::Result;

use crate::run_jq_full;

/// `depth` nested containers of `shape` around `1`: arrays (`[`), objects (`{"a":`), or the two
/// alternating, objects first.
fn nested(shape: &str, depth: usize) -> String {
    let opener = |level: usize| match shape {
        "arr" | "empty" => "[",
        "obj" => "{\"a\":",
        _ if level % 2 == 0 => "{\"a\":",
        _ => "[",
    };
    let closer = |level: usize| match shape {
        "arr" | "empty" => "]",
        "obj" => "}",
        _ if level % 2 == 0 => "}",
        _ => "]",
    };
    let open: String = (0..depth).map(opener).collect();
    let close: String = (0..depth).rev().map(closer).collect();
    // `empty` has no leaf: the innermost array is `[]`.
    let leaf = if shape == "empty" { "" } else { "1" };
    format!("{open}{leaf}{close}")
}

#[test]
fn a_named_json_value_is_read_as_deep_as_jq_reads_it_6637() -> Result<()> {
    for (flag, shape, depth, stdout, status) in [
        ("--argjson", "arr", 128, "1\n", 0),
        ("--argjson", "arr", 129, "1\n", 0),
        ("--argjson", "arr", 256, "1\n", 0),
        ("--argjson", "arr", 257, "", 2),
        ("--argjson", "obj", 128, "1\n", 0),
        ("--argjson", "obj", 129, "", 2),
        ("--argjson", "mix", 128, "1\n", 0),
        ("--argjson", "mix", 129, "1\n", 0),
        ("--argjson", "mix", 171, "1\n", 0),
        ("--argjson", "mix", 172, "", 2),
        ("--argjson", "mix", 200, "", 2),
        ("--argjson", "empty", 256, "1\n", 0),
        ("--argjson", "empty", 257, "", 2),
        ("--jsonargs", "arr", 128, "1\n", 0),
        ("--jsonargs", "arr", 129, "1\n", 0),
        ("--jsonargs", "arr", 256, "1\n", 0),
        ("--jsonargs", "arr", 257, "", 2),
        ("--jsonargs", "obj", 128, "1\n", 0),
        ("--jsonargs", "obj", 129, "", 2),
        ("--jsonargs", "mix", 128, "1\n", 0),
        ("--jsonargs", "mix", 129, "1\n", 0),
        ("--jsonargs", "mix", 171, "1\n", 0),
        ("--jsonargs", "mix", 172, "", 2),
        ("--jsonargs", "mix", 200, "", 2),
        ("--jsonargs", "empty", 256, "1\n", 0),
        ("--jsonargs", "empty", 257, "", 2),
    ] {
        let text = nested(shape, depth);
        let args: Vec<&str> = if flag == "--argjson" {
            vec!["-nc", "--argjson", "v", &text, "$v|length"]
        } else {
            vec!["-nc", "$ARGS.positional|length", "--jsonargs", &text]
        };
        let (out, _, code) = run_jq_full(&args, None)?;
        assert_eq!(
            (out.as_str(), code),
            (stdout, status),
            "{flag} {shape} {depth}"
        );
    }
    Ok(())
}
