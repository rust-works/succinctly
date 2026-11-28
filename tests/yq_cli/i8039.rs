//! #8039: an update whose target is its own input (`. |= f`) reads `key` and `parent` at that
//! input's position. `with(.b; . |= key)` is `.b |= (. |= key)`, and yq writes `"b"` there as it
//! does for `.b |= key`, but the nested update answered no `key` (nor `parent`) at its own input,
//! so the value was left unchanged. `path` already read the position the enclosing update
//! installed; `key` is now its last component, and `parent` climbs the enclosing update's input.
//! A value yq builds with no position (`5 | . |= key`, whose `5` has no key) gives the updates it
//! reaches none, as a read of it gets none. An update whose own reads cannot be resolved at its
//! target names no position for its filter, as before.
//!
//! Every row was captured from yq v4.53.3. `with` is not a jq builtin, and `succinctly jq` still
//! refuses it as jq 1.7.1 does; a nested update there climbs no `parent` either.

use std::io::Write;
use std::process::{Command, Stdio};

use anyhow::Result;
use tempfile::NamedTempFile;

use crate::{run_jq_stdin, run_jq_stdin_with_stderr, run_yq_stdin, run_yq_stdin_with_stderr};

const FLAT: &str = "{\"a\":1,\"b\":5}";
const NESTED: &str = "{\"a\":{\"b\":{\"c\":1},\"d\":2}}";
const LIST: &str = "{\"a\":[1,2,3]}";
const SPINE: &str = "{\"x\":{\"b\":{\"c\":1},\"d\":2}}";

/// (input, filter, yq's stdout with `-p=json -o=json -I0`).
const ROWS: &[(&str, &str, &str)] = &[
    // The issue's rows.
    (FLAT, "with(.b; . |= key)", "{\"a\":1,\"b\":\"b\"}\n"),
    (
        FLAT,
        "with(.b; . |= (select(. > 3) | key))",
        "{\"a\":1,\"b\":\"b\"}\n",
    ),
    (FLAT, ".b |= key", "{\"a\":1,\"b\":\"b\"}\n"),
    // The desugared spelling, and the same update reached mid-pipe.
    (FLAT, ".b |= (. |= key)", "{\"a\":1,\"b\":\"b\"}\n"),
    (FLAT, ".b | . |= key", "\"b\"\n"),
    (NESTED, ".a.b | . |= key", "\"b\"\n"),
    // `path` already read the position; a `key` and a `path` together.
    (FLAT, "with(.b; . |= path)", "{\"a\":1,\"b\":[\"b\"]}\n"),
    (
        FLAT,
        "with(.b; . |= [key, path])",
        "{\"a\":1,\"b\":[\"b\",[\"b\"]]}\n",
    ),
    // A plain write, a read whose value `with` discards, and a write below a scalar.
    (FLAT, "with(.b; . = key)", "{\"a\":1,\"b\":\"b\"}\n"),
    (FLAT, "with(.b; key)", "{\"a\":1,\"b\":5}\n"),
    (FLAT, "with(.b; .x |= key)", "{\"a\":1,\"b\":5}\n"),
    // A select that fails writes nothing, and a `key` is no literal to re-emit after it (#8002).
    (
        FLAT,
        "with(.b; . |= (select(false) | key))",
        "{\"a\":1,\"b\":5}\n",
    ),
    (
        FLAT,
        "with(.b; . |= (select(. > 9) | key))",
        "{\"a\":1,\"b\":5}\n",
    ),
    // The body's other shapes: a select stage, a binding.
    (
        FLAT,
        "with(.b; select(. > 3) | . |= key)",
        "{\"a\":1,\"b\":\"b\"}\n",
    ),
    (
        FLAT,
        "with(.b; . as $x | . |= key)",
        "{\"a\":1,\"b\":\"b\"}\n",
    ),
    // A missing target is created with its key.
    (
        FLAT,
        "with(.zz; . |= key)",
        "{\"a\":1,\"b\":5,\"zz\":\"zz\"}\n",
    ),
    // A missing target several levels deep is created with its key and path.
    (
        FLAT,
        "with(.zz.q; . |= key)",
        "{\"a\":1,\"b\":5,\"zz\":{\"q\":\"q\"}}\n",
    ),
    (
        FLAT,
        "with(.zz.q.r; . |= path)",
        "{\"a\":1,\"b\":5,\"zz\":{\"q\":{\"r\":[\"zz\",\"q\",\"r\"]}}}\n",
    ),
    // A node built in the body has no position: its `key` answers nothing, its `path` is `[]`.
    (FLAT, "with(.b; 5 | . |= key)", "{\"a\":1,\"b\":5}\n"),
    (FLAT, ".b |= (5 | . |= key)", "{\"a\":1,\"b\":5}\n"),
    (FLAT, ".b |= (5 | . |= path)", "{\"a\":1,\"b\":[]}\n"),
    (
        NESTED,
        ".a |= (.b |= (5 | . |= key))",
        "{\"a\":{\"b\":5,\"d\":2}}\n",
    ),
    (
        NESTED,
        ".a |= (.b |= (5 | . |= path))",
        "{\"a\":{\"b\":[],\"d\":2}}\n",
    ),
    // The document root has no key.
    (FLAT, ". |= key", "{\"a\":1,\"b\":5}\n"),
    // Nested `with`.
    (
        NESTED,
        "with(.a; with(.b; . |= key))",
        "{\"a\":{\"b\":\"b\",\"d\":2}}\n",
    ),
    (
        NESTED,
        "with(.a; with(.b; . |= path))",
        "{\"a\":{\"b\":[\"a\",\"b\"],\"d\":2}}\n",
    ),
    (NESTED, "with(.a; . |= key)", "{\"a\":\"a\"}\n"),
    (
        NESTED,
        "with(.a; .b |= key)",
        "{\"a\":{\"b\":\"b\",\"d\":2}}\n",
    ),
    // `parent` above the nested update's own input.
    (
        NESTED,
        "with(.a.b; . |= (parent | keys))",
        "{\"a\":{\"b\":[\"b\",\"d\"],\"d\":2}}\n",
    ),
    (
        NESTED,
        "with(.a.b; . |= (parent(2) | keys))",
        "{\"a\":{\"b\":[\"a\"],\"d\":2}}\n",
    ),
    // A climb past the enclosing update's input reaches the update around that one.
    (
        NESTED,
        "with(.a; with(.b; . |= (parent(2) | keys)))",
        "{\"a\":{\"b\":[\"a\"],\"d\":2}}\n",
    ),
    (
        NESTED,
        ".a |= (.b |= (. |= (parent(2) | keys)))",
        "{\"a\":{\"b\":[\"a\"],\"d\":2}}\n",
    ),
    (
        NESTED,
        ".a.b |= (parent | keys)",
        "{\"a\":{\"b\":[\"b\",\"d\"],\"d\":2}}\n",
    ),
    // Array members.
    (LIST, "with(.a[]; . |= key)", "{\"a\":[0,1,2]}\n"),
    (LIST, "with(.a[1]; . |= key)", "{\"a\":[1,1,3]}\n"),
    (
        LIST,
        "with(.a[]; . |= path)",
        "{\"a\":[[\"a\",0],[\"a\",1],[\"a\",2]]}\n",
    ),
    (LIST, ".a[] |= (. |= key)", "{\"a\":[0,1,2]}\n"),
    // An update nested in one whose own reads cannot all be resolved at its target: its walk, or
    // the navigation that reaches it, names its position below the enclosing one's.
    (
        SPINE,
        "with(.x; . |= (.d = 1 | .b |= path))",
        "{\"x\":{\"b\":[\"x\",\"b\"],\"d\":1}}\n",
    ),
    (
        SPINE,
        "with(.x; . |= (.b | .c |= path))",
        "{\"x\":{\"c\":[\"x\",\"b\",\"c\"]}}\n",
    ),
    (
        SPINE,
        ".x |= (. |= (.b | . |= path))",
        "{\"x\":[\"x\",\"b\"]}\n",
    ),
    (
        SPINE,
        ".x |= (. |= (to_entries | .[0] |= path))",
        "{\"x\":[[\"x\",0],{\"key\":\"d\",\"value\":2}]}\n",
    ),
    (
        SPINE,
        ".x |= (. as $v | . |= (.b | . |= path))",
        "{\"x\":[\"x\",\"b\"]}\n",
    ),
    (
        SPINE,
        ".x |= (.b |= (.c |= path, (5 | key)))",
        "{\"x\":{\"b\":{\"c\":[\"x\",\"b\",\"c\"]},\"d\":2}}\n",
    ),
    (
        SPINE,
        "with(.x.b; .d = 1 | .c |= key)",
        "{\"x\":{\"b\":{\"c\":\"c\",\"d\":1},\"d\":2}}\n",
    ),
    // Whether a stage leaves its value a position is the owned identity rule's answer, as for a
    // read (`.x | (map(.) | path)` is `[]`): a literal, `now`, arithmetic on a literal, a bound
    // literal, `map` and `keys` leave none, and a node inside a positionless value stands at its
    // own path, a variable bound from one included.
    (SPINE, ".x |= (now | (. |= path))", "{\"x\":[]}\n"),
    (SPINE, ".x |= (1 + 2 | (. |= path))", "{\"x\":[]}\n"),
    (SPINE, ".x |= (\"a\" + .d | (. |= path))", "{\"x\":[]}\n"),
    (SPINE, ".x |= (5 as $v | $v | (. |= path))", "{\"x\":[]}\n"),
    (SPINE, ".x |= ((5 | .) | (. |= path))", "{\"x\":[]}\n"),
    (SPINE, ".x |= (.zz // \"q\" | (. |= path))", "{\"x\":[]}\n"),
    (SPINE, ".x |= (.zz // 3 | (. |= path))", "{\"x\":[]}\n"),
    (SPINE, ".x |= (map(.) | (. |= path))", "{\"x\":[]}\n"),
    (SPINE, ".x |= (keys | (. |= path))", "{\"x\":[]}\n"),
    (SPINE, ".x |= ({} | .k | (. |= path))", "{\"x\":[\"k\"]}\n"),
    (
        SPINE,
        ".x |= ({\"a\":1} | .a as $v | $v | . |= path)",
        "{\"x\":[\"a\"]}\n",
    ),
    (
        SPINE,
        ".x |= (keys | .[0] as $v | $v | . |= path)",
        "{\"x\":[0]}\n",
    ),
    (
        SPINE,
        ".x |= (map(.) | .[0] as $v | $v | . |= path)",
        "{\"x\":[0]}\n",
    ),
    (
        SPINE,
        ".x |= ({\"a\":{\"q\":1}} as $v | $v | .a |= path)",
        "{\"x\":{\"a\":[\"a\"]}}\n",
    ),
    (SPINE, ".x |= (keys as $v | $v | . |= path)", "{\"x\":[]}\n"),
    (
        SPINE,
        ".x |= (splitDoc | (. |= (parent | keys)))",
        "{\"x\":{\"b\":{\"c\":1},\"d\":2}}\n",
    ),
    // A scalar the body builds still has no position under such an update.
    (
        SPINE,
        ".x |= (.b |= (5 | . |= key))",
        "{\"x\":{\"b\":5,\"d\":2}}\n",
    ),
    (
        SPINE,
        ".x |= (.b |= (5 | . |= path))",
        "{\"x\":{\"b\":[],\"d\":2}}\n",
    ),
];

#[test]
fn a_nested_update_reads_key_and_parent_at_its_own_input_8039() -> Result<()> {
    for (input, filter, expected) in ROWS {
        let (stdout, code) = run_yq_stdin(filter, input, &["-p=json", "-o=json", "-I0"])?;
        assert_eq!(code, 0, "{filter}");
        assert_eq!(&stdout, expected, "{filter}");
    }
    Ok(())
}

/// Where the owned identity rule places a value differently from yq (`.b.c + 1` over an absent
/// operand, an `ireduce` that answers INIT), an update reads that place as a read does (#8039,
/// recorded in `docs/compliance/yq/limitations.md`). Main answered these as yq does by accident,
/// from no position at all; a fix to the rule changes them together with the read rows below.
#[test]
fn an_update_reads_the_identity_rules_position_where_it_differs_from_yq_8039() -> Result<()> {
    for (filter, expected, yq) in [
        // `.b.c + 1` over an absent `.b.c`: yq gives the sum no position (`key` answers nothing,
        // `path` is `[]`); the rule places it at its left operand.
        (
            ".x |= (.b |= (.b.c + 1 | . |= key))",
            "{\"x\":{\"b\":\"c\",\"d\":2}}\n",
            "{\"x\":{\"b\":1,\"d\":2}}",
        ),
        (".x | .b | (.b.c + 1 | key)", "\"c\"\n", "(nothing)"),
        // An `ireduce` over a source that yields nothing answers INIT, which has no position in yq;
        // the rule keeps the source's.
        (
            ".x |= (.d // 3 | .[]? as $v ireduce (0; $v) | . |= key)",
            "{\"x\":\"d\"}\n",
            "{\"x\":0}",
        ),
        (
            ".x | (.d // 3 | .[]? as $v ireduce (0; $v) | key)",
            "\"d\"\n",
            "(nothing)",
        ),
        // #2435: yq creates `.zz.q` on the way, so its parent is a map; the snapshot holds `zz` as
        // the `null` it was created as.
        (
            ".zz |= (.q | . |= (parent | kind))",
            "{\"x\":{\"b\":{\"c\":1},\"d\":2},\"zz\":null}\n",
            "{\"x\":{\"b\":{\"c\":1},\"d\":2},\"zz\":\"map\"}",
        ),
    ] {
        let (stdout, code) = run_yq_stdin(filter, SPINE, &["-p=json", "-o=json", "-I0"])?;
        assert_eq!(code, 0, "{filter}");
        assert_eq!(stdout, expected, "{filter} (yq v4.53.3: {yq})");
    }
    Ok(())
}

/// An update whose own reads cannot all be resolved at its target names no position for its
/// filter, as before #8039 (`docs/compliance/yq/limitations.md`): the prefix the updates nested in
/// it read stays the enclosing update's, and a stage that hands the position on gives them only the
/// part it adds. Each row's yq answer is in the assert message.
#[test]
fn an_unpositioned_update_names_no_position_for_its_filter_8039() -> Result<()> {
    for (filter, expected, yq) in [
        (
            ".x |= (.b |= (.c | . |= path))",
            "{\"x\":{\"b\":[\"x\",\"c\"],\"d\":2}}\n",
            "[\"x\",\"b\",\"c\"]",
        ),
        (
            ".x |= (.b |= (.d = 1 | .c |= path))",
            "{\"x\":{\"b\":{\"c\":[\"x\",\"c\"],\"d\":1},\"d\":2}}\n",
            "[\"x\",\"b\",\"c\"]",
        ),
        (
            ".x |= (.d // 3 | (. |= path))",
            "{\"x\":[\"d\"]}\n",
            "[\"x\",\"d\"]",
        ),
        (".x |= ([.[]] | (. |= path))", "{\"x\":[]}\n", "[\"x\"]"),
        (
            ".x |= (to_entries | (. |= path))",
            "{\"x\":[]}\n",
            "[\"x\"]",
        ),
        (
            ".x |= (.b |= (5 | . |= path) | .d |= path)",
            "{\"x\":{\"b\":[],\"d\":[\"d\"]}}\n",
            "d: [\"x\",\"d\"]",
        ),
    ] {
        let (stdout, code) = run_yq_stdin(filter, SPINE, &["-p=json", "-o=json", "-I0"])?;
        assert_eq!(code, 0, "{filter}");
        assert_eq!(stdout, expected, "{filter} (yq v4.53.3: {yq})");
    }
    Ok(())
}

/// An error the positioned filter raises ends the run, at the target's key: `key | keys` fails on
/// the string `"b"` in yq and here (the wording of the `keys` error differs, as recorded in
/// `docs/compliance/yq/limitations.md`).
#[test]
fn an_error_in_a_positioned_filter_ends_the_run_8039() -> Result<()> {
    for filter in [".b |= (key | keys)", "with(.b; . |= (key | keys))"] {
        let (stdout, stderr, code) =
            run_yq_stdin_with_stderr(filter, FLAT, &["-p=json", "-o=json", "-I0"])?;
        assert_ne!(code, 0, "{filter}");
        assert_eq!(stdout, "", "{filter}");
        assert!(stderr.contains("\"b\""), "{filter}: {stderr}");
    }
    Ok(())
}

/// The YAML document route, a multi-document file, `--eval-all`, `-n` and `-i`.
#[test]
fn the_other_routes_read_the_same_position_8039() -> Result<()> {
    const DOC: &str = "a: 1\nb: 5\nc:\n  d: 2\n";
    for (filter, expected) in [
        ("with(.b; . |= key)", "a: 1\nb: b\nc:\n  d: 2\n"),
        (
            "with(.c.d; . |= (parent | keys))",
            "a: 1\nb: 5\nc:\n  d:\n    - d\n",
        ),
    ] {
        let (stdout, code) = run_yq_stdin(filter, DOC, &[])?;
        assert_eq!(code, 0, "{filter}");
        assert_eq!(stdout, expected, "{filter}");
    }
    const MULTI: &str = "a: 1\nb: 5\n---\nb: 7\n";
    for args in [&[][..], &["--eval-all"][..]] {
        let (stdout, code) = run_yq_stdin("with(.b; . |= key)", MULTI, args)?;
        assert_eq!(code, 0, "{args:?}");
        assert_eq!(stdout, "a: 1\nb: b\n---\nb: b\n", "{args:?}");
    }
    let (stdout, code) = run_yq_stdin("with(.b; . |= (parent | keys))", MULTI, &["--eval-all"])?;
    assert_eq!(code, 0);
    assert_eq!(stdout, "a: 1\nb:\n  - a\n  - b\n---\nb:\n  - b\n");
    for (filter, expected) in [
        (
            "{\"a\":{\"b\":1}} | with(.a.b; . |= key)",
            "{\"a\":{\"b\":\"b\"}}\n",
        ),
        (
            "{\"a\":{\"b\":1}} | with(.a.b; . |= path)",
            "{\"a\":{\"b\":[\"a\",\"b\"]}}\n",
        ),
        (
            "{\"a\":{\"b\":1}} | with(.a.b; . |= (parent | keys))",
            "{\"a\":{\"b\":[\"b\"]}}\n",
        ),
    ] {
        let (stdout, code) = run_yq_stdin(filter, "", &["-n", "-o=json", "-I0"])?;
        assert_eq!(code, 0, "-n {filter}");
        assert_eq!(stdout, expected, "-n {filter}");
    }
    let mut file = NamedTempFile::new()?;
    write!(file, "a: 1 # one\nb: 5 # five\nc:\n  d: 2\n")?;
    let output = Command::new(env!("CARGO_BIN_EXE_succinctly"))
        .args(["yq", "-i", "with(.b; . |= key)"])
        .arg(file.path())
        .stdin(Stdio::null())
        .output()?;
    assert!(output.status.success(), "-i");
    assert_eq!(
        std::fs::read_to_string(file.path())?,
        "a: 1 # one\nb: b # five\nc:\n  d: 2\n"
    );
    Ok(())
}

/// `with` is not a jq builtin: `succinctly jq` refuses it as jq 1.7.1 does, and a nested `. |= f`
/// is unchanged there, `parent` included.
#[test]
fn jq_mode_is_unchanged_8039() -> Result<()> {
    let (stdout, stderr, code) = run_jq_stdin_with_stderr("with(.b; . |= 7)", FLAT, &["-c"])?;
    assert_eq!(code, 3);
    assert_eq!(stdout, "");
    assert!(
        stderr.contains("with/2 is not defined"),
        "unexpected stderr: {stderr}"
    );
    let (stdout, code) = run_jq_stdin(".b |= (. |= 7)", FLAT, &["-c"])?;
    assert_eq!(code, 0);
    assert_eq!(stdout, "{\"a\":1,\"b\":7}\n");
    // `parent` is a succinctly extension in jq mode, and a nested update at its own input still
    // climbs nothing there: the yq-mode ancestry is not installed.
    let (stdout, code) = run_jq_stdin(
        ".a.b |= (. |= (parent | keys))",
        "{\"a\":{\"b\":1,\"c\":2}}",
        &["-c"],
    )?;
    assert_eq!(code, 0);
    assert_eq!(stdout, "{\"a\":{\"b\":null,\"c\":2}}\n");
    Ok(())
}
