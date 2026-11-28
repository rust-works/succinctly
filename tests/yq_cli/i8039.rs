//! #8039: an update whose target is its own input (`. |= f`) reads `key` and `parent` at that
//! input's position. `with(.b; . |= key)` is `.b |= (. |= key)`, and yq writes `"b"` there as it
//! does for `.b |= key`, but the nested update answered no `key` (nor `parent`) at its own input,
//! so the value was left unchanged. `path` already read the position the enclosing update
//! installed; `key` is now its last component, and `parent` climbs the enclosing update's input.
//! An update whose own reads cannot be resolved at its target still names that target for the
//! updates nested in its filter, unless a value yq builds with no position comes first
//! (`5 | . |= key`, whose `5` has no key).
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
    // An update whose own reads cannot all be resolved at its target still names it for the
    // updates nested in its filter: after a write, through navigation, `to_entries` and a binding.
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
        ".x |= (.b |= (.c | . |= path))",
        "{\"x\":{\"b\":[\"x\",\"b\",\"c\"],\"d\":2}}\n",
    ),
    (
        SPINE,
        ".x |= (.b |= (.d = 1 | .c |= path))",
        "{\"x\":{\"b\":{\"c\":[\"x\",\"b\",\"c\"],\"d\":1},\"d\":2}}\n",
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

/// An error the positioned filter raises ends the run, reading the target's key as yq does.
#[test]
fn an_error_in_a_positioned_filter_reads_the_key_8039() -> Result<()> {
    for filter in [".b |= error(key)", "with(.b; . |= error(key))"] {
        let (stdout, stderr, code) =
            run_yq_stdin_with_stderr(filter, FLAT, &["-p=json", "-o=json", "-I0"])?;
        assert_eq!(code, 1, "{filter}");
        assert_eq!(stdout, "", "{filter}");
        assert_eq!(stderr, "Error: b\n", "{filter}");
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
