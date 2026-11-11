//! #6904: `succinctly yq` words the error of an arithmetic operator over two kinds that do not
//! combine as yq does, from the operands' tags (`!!float cannot be added to !!null`), not as jq does
//! (`number (1.5) and null (null) cannot be added`).
//!
//! yq's wording is irregular and reproduced bug for bug: `-` over two scalars says `cannot be
//! added to`, `*` names both operands bare, `/` and `%` put a key in parentheses after a tag when
//! a container or a left `null` is involved, `+` over a container names the right operand first,
//! and a mapping on the left of `-` is `maps not yet supported for subtraction`. A container
//! operand here carries no key, so the rows use literal operands, whose key yq also leaves empty.
//! Every row is the first stderr line of yq v4.53.3 for the same filter; the exit code is 1 in
//! both.

use crate::run_yq_stdin_with_stderr;

const DOC: &str = "i: 2\nf: 1.5\ns: x\nb: true\nn: null\nl: [1]\nm: {a: 1}\n";

/// The rows' input: they use literal operands, whose key yq leaves empty.
const ROW_DOC: &str = "a: 1\n";

/// `(filter, first stderr line)`.
const ROWS: &[(&str, &str)] = &[
    ("2 + true", "Error: !!int cannot be added to !!bool"),
    ("2 + null", "Error: !!int cannot be added to !!null"),
    ("2 + [1]", "Error: !!seq () cannot be added to a !!int ()"),
    (
        "2 + {\"a\": 1}",
        "Error: !!map () cannot be added to a !!int ()",
    ),
    ("1.5 + true", "Error: !!float cannot be added to !!bool"),
    ("1.5 + null", "Error: !!float cannot be added to !!null"),
    (
        "1.5 + [1]",
        "Error: !!seq () cannot be added to a !!float ()",
    ),
    (
        "1.5 + {\"a\": 1}",
        "Error: !!map () cannot be added to a !!float ()",
    ),
    (
        "\"x\" + [1]",
        "Error: !!seq () cannot be added to a !!str ()",
    ),
    (
        "\"x\" + {\"a\": 1}",
        "Error: !!map () cannot be added to a !!str ()",
    ),
    ("true + 2", "Error: !!bool cannot be added to !!int"),
    ("true + 1.5", "Error: !!bool cannot be added to !!float"),
    ("true + true", "Error: !!bool cannot be added to !!bool"),
    ("true + null", "Error: !!bool cannot be added to !!null"),
    (
        "true + [1]",
        "Error: !!seq () cannot be added to a !!bool ()",
    ),
    (
        "true + {\"a\": 1}",
        "Error: !!map () cannot be added to a !!bool ()",
    ),
    (
        "{\"a\": 1} + 2",
        "Error: !!int () cannot be added to a !!map ()",
    ),
    (
        "{\"a\": 1} + 1.5",
        "Error: !!float () cannot be added to a !!map ()",
    ),
    (
        "{\"a\": 1} + \"x\"",
        "Error: !!str () cannot be added to a !!map ()",
    ),
    (
        "{\"a\": 1} + true",
        "Error: !!bool () cannot be added to a !!map ()",
    ),
    (
        "{\"a\": 1} + null",
        "Error: !!null () cannot be added to a !!map ()",
    ),
    (
        "{\"a\": 1} + [1]",
        "Error: !!seq () cannot be added to a !!map ()",
    ),
    ("2 - \"x\"", "Error: !!int cannot be added to !!str"),
    ("2 - true", "Error: !!int cannot be added to !!bool"),
    ("2 - null", "Error: !!int cannot be added to !!null"),
    ("2 - [1]", "Error: !!seq () cannot be subtracted from !!int"),
    (
        "2 - {\"a\": 1}",
        "Error: !!map () cannot be subtracted from !!int",
    ),
    ("1.5 - \"x\"", "Error: !!float cannot be added to !!str"),
    ("1.5 - true", "Error: !!float cannot be added to !!bool"),
    ("1.5 - null", "Error: !!float cannot be added to !!null"),
    (
        "1.5 - [1]",
        "Error: !!seq () cannot be subtracted from !!float",
    ),
    (
        "1.5 - {\"a\": 1}",
        "Error: !!map () cannot be subtracted from !!float",
    ),
    ("\"x\" - 2", "Error: strings cannot be subtracted"),
    ("\"x\" - 1.5", "Error: strings cannot be subtracted"),
    ("\"x\" - \"x\"", "Error: strings cannot be subtracted"),
    ("\"x\" - true", "Error: strings cannot be subtracted"),
    ("\"x\" - null", "Error: strings cannot be subtracted"),
    (
        "\"x\" - [1]",
        "Error: !!seq () cannot be subtracted from !!str",
    ),
    (
        "\"x\" - {\"a\": 1}",
        "Error: !!map () cannot be subtracted from !!str",
    ),
    ("true - 2", "Error: !!bool cannot be added to !!int"),
    ("true - 1.5", "Error: !!bool cannot be added to !!float"),
    ("true - \"x\"", "Error: !!bool cannot be added to !!str"),
    ("true - true", "Error: !!bool cannot be added to !!bool"),
    ("true - null", "Error: !!bool cannot be added to !!null"),
    (
        "true - [1]",
        "Error: !!seq () cannot be subtracted from !!bool",
    ),
    (
        "true - {\"a\": 1}",
        "Error: !!map () cannot be subtracted from !!bool",
    ),
    ("[1] - 2", "Error: !!int () cannot be subtracted from !!seq"),
    (
        "[1] - 1.5",
        "Error: !!float () cannot be subtracted from !!seq",
    ),
    (
        "[1] - \"x\"",
        "Error: !!str () cannot be subtracted from !!seq",
    ),
    (
        "[1] - true",
        "Error: !!bool () cannot be subtracted from !!seq",
    ),
    (
        "[1] - null",
        "Error: !!null () cannot be subtracted from !!seq",
    ),
    (
        "[1] - {\"a\": 1}",
        "Error: !!map () cannot be subtracted from !!seq",
    ),
    (
        "{\"a\": 1} - 2",
        "Error: maps not yet supported for subtraction",
    ),
    (
        "{\"a\": 1} - 1.5",
        "Error: maps not yet supported for subtraction",
    ),
    (
        "{\"a\": 1} - \"x\"",
        "Error: maps not yet supported for subtraction",
    ),
    (
        "{\"a\": 1} - true",
        "Error: maps not yet supported for subtraction",
    ),
    (
        "{\"a\": 1} - null",
        "Error: maps not yet supported for subtraction",
    ),
    (
        "{\"a\": 1} - [1]",
        "Error: maps not yet supported for subtraction",
    ),
    (
        "{\"a\": 1} - {\"a\": 1}",
        "Error: maps not yet supported for subtraction",
    ),
    ("2 * true", "Error: cannot multiply !!int with !!bool"),
    ("2 * [1]", "Error: cannot multiply !!int with !!seq"),
    ("2 * {\"a\": 1}", "Error: cannot multiply !!int with !!map"),
    ("1.5 * \"x\"", "Error: cannot multiply !!float with !!str"),
    ("1.5 * true", "Error: cannot multiply !!float with !!bool"),
    ("1.5 * [1]", "Error: cannot multiply !!float with !!seq"),
    (
        "1.5 * {\"a\": 1}",
        "Error: cannot multiply !!float with !!map",
    ),
    ("\"x\" * 1.5", "Error: cannot multiply !!str with !!float"),
    ("\"x\" * \"x\"", "Error: cannot multiply !!str with !!str"),
    ("\"x\" * true", "Error: cannot multiply !!str with !!bool"),
    ("\"x\" * [1]", "Error: cannot multiply !!str with !!seq"),
    (
        "\"x\" * {\"a\": 1}",
        "Error: cannot multiply !!str with !!map",
    ),
    ("true * 2", "Error: cannot multiply !!bool with !!int"),
    ("true * 1.5", "Error: cannot multiply !!bool with !!float"),
    ("true * \"x\"", "Error: cannot multiply !!bool with !!str"),
    ("true * true", "Error: cannot multiply !!bool with !!bool"),
    ("true * [1]", "Error: cannot multiply !!bool with !!seq"),
    (
        "true * {\"a\": 1}",
        "Error: cannot multiply !!bool with !!map",
    ),
    ("null * 2", "Error: cannot multiply !!null with !!int"),
    ("null * 1.5", "Error: cannot multiply !!null with !!float"),
    ("null * \"x\"", "Error: cannot multiply !!null with !!str"),
    ("null * true", "Error: cannot multiply !!null with !!bool"),
    ("[1] * 2", "Error: cannot multiply !!seq with !!int"),
    ("[1] * 1.5", "Error: cannot multiply !!seq with !!float"),
    ("[1] * \"x\"", "Error: cannot multiply !!seq with !!str"),
    ("[1] * true", "Error: cannot multiply !!seq with !!bool"),
    (
        "[1] * {\"a\": 1}",
        "Error: cannot multiply !!seq with !!map",
    ),
    ("{\"a\": 1} * 2", "Error: cannot multiply !!map with !!int"),
    (
        "{\"a\": 1} * 1.5",
        "Error: cannot multiply !!map with !!float",
    ),
    (
        "{\"a\": 1} * \"x\"",
        "Error: cannot multiply !!map with !!str",
    ),
    (
        "{\"a\": 1} * true",
        "Error: cannot multiply !!map with !!bool",
    ),
    (
        "{\"a\": 1} * [1]",
        "Error: cannot multiply !!map with !!seq",
    ),
    ("2 / \"x\"", "Error: !!int cannot be divided by !!str"),
    ("2 / true", "Error: !!int cannot be divided by !!bool"),
    ("2 / null", "Error: !!int cannot be divided by !!null"),
    ("2 / [1]", "Error: !!int () cannot be divided by !!seq ()"),
    (
        "2 / {\"a\": 1}",
        "Error: !!int () cannot be divided by !!map ()",
    ),
    ("1.5 / \"x\"", "Error: !!float cannot be divided by !!str"),
    ("1.5 / true", "Error: !!float cannot be divided by !!bool"),
    ("1.5 / null", "Error: !!float cannot be divided by !!null"),
    (
        "1.5 / [1]",
        "Error: !!float () cannot be divided by !!seq ()",
    ),
    (
        "1.5 / {\"a\": 1}",
        "Error: !!float () cannot be divided by !!map ()",
    ),
    ("\"x\" / 2", "Error: !!str cannot be divided by !!int"),
    ("\"x\" / 1.5", "Error: !!str cannot be divided by !!float"),
    ("\"x\" / true", "Error: !!str cannot be divided by !!bool"),
    ("\"x\" / null", "Error: !!str cannot be divided by !!null"),
    (
        "\"x\" / [1]",
        "Error: !!str () cannot be divided by !!seq ()",
    ),
    (
        "\"x\" / {\"a\": 1}",
        "Error: !!str () cannot be divided by !!map ()",
    ),
    ("true / 2", "Error: !!bool cannot be divided by !!int"),
    ("true / 1.5", "Error: !!bool cannot be divided by !!float"),
    ("true / \"x\"", "Error: !!bool cannot be divided by !!str"),
    ("true / true", "Error: !!bool cannot be divided by !!bool"),
    ("true / null", "Error: !!bool cannot be divided by !!null"),
    (
        "true / [1]",
        "Error: !!bool () cannot be divided by !!seq ()",
    ),
    (
        "true / {\"a\": 1}",
        "Error: !!bool () cannot be divided by !!map ()",
    ),
    ("null / 2", "Error: !!null () cannot be divided by !!int ()"),
    (
        "null / 1.5",
        "Error: !!null () cannot be divided by !!float ()",
    ),
    (
        "null / \"x\"",
        "Error: !!null () cannot be divided by !!str ()",
    ),
    (
        "null / true",
        "Error: !!null () cannot be divided by !!bool ()",
    ),
    (
        "null / null",
        "Error: !!null () cannot be divided by !!null ()",
    ),
    (
        "null / [1]",
        "Error: !!null () cannot be divided by !!seq ()",
    ),
    (
        "null / {\"a\": 1}",
        "Error: !!null () cannot be divided by !!map ()",
    ),
    ("[1] / 2", "Error: !!seq () cannot be divided by !!int ()"),
    (
        "[1] / 1.5",
        "Error: !!seq () cannot be divided by !!float ()",
    ),
    (
        "[1] / \"x\"",
        "Error: !!seq () cannot be divided by !!str ()",
    ),
    (
        "[1] / true",
        "Error: !!seq () cannot be divided by !!bool ()",
    ),
    (
        "[1] / null",
        "Error: !!seq () cannot be divided by !!null ()",
    ),
    ("[1] / [1]", "Error: !!seq () cannot be divided by !!seq ()"),
    (
        "[1] / {\"a\": 1}",
        "Error: !!seq () cannot be divided by !!map ()",
    ),
    (
        "{\"a\": 1} / 2",
        "Error: !!map () cannot be divided by !!int ()",
    ),
    (
        "{\"a\": 1} / 1.5",
        "Error: !!map () cannot be divided by !!float ()",
    ),
    (
        "{\"a\": 1} / \"x\"",
        "Error: !!map () cannot be divided by !!str ()",
    ),
    (
        "{\"a\": 1} / true",
        "Error: !!map () cannot be divided by !!bool ()",
    ),
    (
        "{\"a\": 1} / null",
        "Error: !!map () cannot be divided by !!null ()",
    ),
    (
        "{\"a\": 1} / [1]",
        "Error: !!map () cannot be divided by !!seq ()",
    ),
    (
        "{\"a\": 1} / {\"a\": 1}",
        "Error: !!map () cannot be divided by !!map ()",
    ),
    ("2 % \"x\"", "Error: !!int cannot modulo by !!str"),
    ("2 % true", "Error: !!int cannot modulo by !!bool"),
    ("2 % null", "Error: !!int cannot modulo by !!null"),
    ("2 % [1]", "Error: !!int () cannot modulo by !!seq ()"),
    (
        "2 % {\"a\": 1}",
        "Error: !!int () cannot modulo by !!map ()",
    ),
    ("1.5 % \"x\"", "Error: !!float cannot modulo by !!str"),
    ("1.5 % true", "Error: !!float cannot modulo by !!bool"),
    ("1.5 % null", "Error: !!float cannot modulo by !!null"),
    ("1.5 % [1]", "Error: !!float () cannot modulo by !!seq ()"),
    (
        "1.5 % {\"a\": 1}",
        "Error: !!float () cannot modulo by !!map ()",
    ),
    ("\"x\" % 2", "Error: !!str cannot modulo by !!int"),
    ("\"x\" % 1.5", "Error: !!str cannot modulo by !!float"),
    ("\"x\" % \"x\"", "Error: !!str cannot modulo by !!str"),
    ("\"x\" % true", "Error: !!str cannot modulo by !!bool"),
    ("\"x\" % null", "Error: !!str cannot modulo by !!null"),
    ("\"x\" % [1]", "Error: !!str () cannot modulo by !!seq ()"),
    (
        "\"x\" % {\"a\": 1}",
        "Error: !!str () cannot modulo by !!map ()",
    ),
    ("true % 2", "Error: !!bool cannot modulo by !!int"),
    ("true % 1.5", "Error: !!bool cannot modulo by !!float"),
    ("true % \"x\"", "Error: !!bool cannot modulo by !!str"),
    ("true % true", "Error: !!bool cannot modulo by !!bool"),
    ("true % null", "Error: !!bool cannot modulo by !!null"),
    ("true % [1]", "Error: !!bool () cannot modulo by !!seq ()"),
    (
        "true % {\"a\": 1}",
        "Error: !!bool () cannot modulo by !!map ()",
    ),
    ("null % 2", "Error: !!null () cannot modulo by !!int ()"),
    ("null % 1.5", "Error: !!null () cannot modulo by !!float ()"),
    ("null % \"x\"", "Error: !!null () cannot modulo by !!str ()"),
    ("null % true", "Error: !!null () cannot modulo by !!bool ()"),
    ("null % null", "Error: !!null () cannot modulo by !!null ()"),
    ("null % [1]", "Error: !!null () cannot modulo by !!seq ()"),
    (
        "null % {\"a\": 1}",
        "Error: !!null () cannot modulo by !!map ()",
    ),
    ("[1] % 2", "Error: !!seq () cannot modulo by !!int ()"),
    ("[1] % 1.5", "Error: !!seq () cannot modulo by !!float ()"),
    ("[1] % \"x\"", "Error: !!seq () cannot modulo by !!str ()"),
    ("[1] % true", "Error: !!seq () cannot modulo by !!bool ()"),
    ("[1] % null", "Error: !!seq () cannot modulo by !!null ()"),
    ("[1] % [1]", "Error: !!seq () cannot modulo by !!seq ()"),
    (
        "[1] % {\"a\": 1}",
        "Error: !!seq () cannot modulo by !!map ()",
    ),
    (
        "{\"a\": 1} % 2",
        "Error: !!map () cannot modulo by !!int ()",
    ),
    (
        "{\"a\": 1} % 1.5",
        "Error: !!map () cannot modulo by !!float ()",
    ),
    (
        "{\"a\": 1} % \"x\"",
        "Error: !!map () cannot modulo by !!str ()",
    ),
    (
        "{\"a\": 1} % true",
        "Error: !!map () cannot modulo by !!bool ()",
    ),
    (
        "{\"a\": 1} % null",
        "Error: !!map () cannot modulo by !!null ()",
    ),
    (
        "{\"a\": 1} % [1]",
        "Error: !!map () cannot modulo by !!seq ()",
    ),
    (
        "{\"a\": 1} % {\"a\": 1}",
        "Error: !!map () cannot modulo by !!map ()",
    ),
];

#[test]
fn an_arithmetic_type_mismatch_is_worded_as_yq_words_it_6904() {
    for (filter, stderr) in ROWS {
        let (out, err, status) =
            run_yq_stdin_with_stderr(filter, ROW_DOC, &[]).expect("run succinctly yq");
        assert_eq!(out, "", "{filter}: stdout");
        assert_eq!(
            err.lines().next().unwrap_or(""),
            *stderr,
            "{filter}: stderr"
        );
        assert_eq!(status, 1, "{filter}: exit code");
    }
}

/// A document path that is a container carries its key in yq's message (`!!seq (l)`), which a value
/// does not here: the text is yq's with `()` in that place (`docs/compliance/yq/limitations.md`).
#[test]
fn a_container_read_from_a_path_names_no_key_in_the_message_6904() {
    let (_, err, status) =
        run_yq_stdin_with_stderr(".f + .l", DOC, &[]).expect("run succinctly yq");
    assert_eq!(
        err.lines().next().unwrap_or(""),
        "Error: !!seq () cannot be added to a !!float ()"
    );
    assert_eq!(status, 1);
}

/// `x * "s"` with a fractional count names the operands in the order they were written (#4269
/// printed `!!str` first whichever side the number was on).
#[test]
fn a_fractional_string_repeat_names_the_operands_in_written_order_6904() {
    for (filter, stderr) in [
        (".s * .f", "Error: cannot multiply !!str with !!float"),
        (".f * .s", "Error: cannot multiply !!float with !!str"),
    ] {
        let (_, err, status) =
            run_yq_stdin_with_stderr(filter, DOC, &[]).expect("run succinctly yq");
        assert_eq!(err.lines().next().unwrap_or(""), stderr, "{filter}");
        assert_eq!(status, 1, "{filter}");
    }
}

/// jq mode keeps jq's wording, including for the pairings yq words differently.
#[test]
fn jq_mode_keeps_jq_wording_for_an_arithmetic_type_mismatch_6904() {
    for (filter, stderr) in [
        (
            "1.5 + true",
            "jq: error (at <stdin>:0): number (1.5) and boolean (true) cannot be added",
        ),
        (
            "1.5 * true",
            "jq: error (at <stdin>:0): number (1.5) and boolean (true) cannot be multiplied",
        ),
    ] {
        let (_, err, status) =
            crate::run_jq_stdin_with_stderr(filter, "null", &[]).expect("run succinctly jq");
        assert!(
            err.contains(stderr.trim_start_matches("jq: error (at <stdin>:0): ")),
            "{filter}: {err}"
        );
        assert_eq!(status, 5, "{filter}");
    }
}

/// A spelled integer (`0x1f`) is an `!!int` in the message, not a number to quote (it used to be
/// jq's `number (31)`, pinned by `i2802.rs` until this change).
#[test]
fn a_hex_integer_is_named_by_its_tag_in_the_message_6904() {
    for (filter, stderr) in [
        (r#".a - "x""#, "Error: !!int cannot be added to !!str"),
        (".a + null", "Error: !!int cannot be added to !!null"),
    ] {
        let (_, err, status) =
            run_yq_stdin_with_stderr(filter, "a: 0x1f\n", &[]).expect("run succinctly yq");
        assert_eq!(err.lines().next().unwrap_or(""), stderr, "{filter}");
        assert_eq!(status, 1, "{filter}");
    }
}
