//! #5612 lets a postfix follow `first`, `last`, `limit`, `range`, `repeat`, `until`, `while` and
//! `error` in jq mode. yq's grammar takes one after `first` and `error` only; the other six are
//! gated outside `--jq-extensions` and stay refused there, as they are in real yq (4.53.3).
//! Expected values were captured from yq 4.53.3.

use crate::run_yq_stdin;

const SEQ: &str = "- id: 1\n- id: 2\n";

#[test]
fn test_yq_postfix_after_first_and_error_parses_and_the_gated_names_stay_refused_5612() {
    for (program, expected) in [
        ("first.id", "1\n"),
        ("first[0]", "null\n"),
        ("first(.id).x", "null\n"),
        ("first(.id)[0]", "null\n"),
        ("first(.id == 1).id", "1\n"),
    ] {
        let (out, code) = run_yq_stdin(program, SEQ, &["-o=json", "-I0"]).unwrap();
        assert_eq!((out.as_str(), code), (expected, 0), "{program}");
    }
    let (out, code) = run_yq_stdin(r#"error("x").y"#, SEQ, &["-o=json", "-I0"]).unwrap();
    assert_eq!((out.as_str(), code), ("", 1));
    for program in ["last(.id).x", "range(3).x", "limit(1;.).x", "until(.;.).x"] {
        let (_, code) = run_yq_stdin(program, SEQ, &["-o=json", "-I0"]).unwrap();
        assert_ne!(code, 0, "{program}");
    }
}

#[test]
fn test_yq_jq_extensions_postfix_after_a_gated_name_follows_jq_5612() {
    let (out, code) = run_yq_stdin(
        "[limit(2;.[]).id]",
        "- id: 1\n- id: 2\n- id: 3\n",
        &["--jq-extensions", "-o=json", "-I0"],
    )
    .unwrap();
    assert_eq!((out.as_str(), code), ("[1,2]\n", 0));
}
