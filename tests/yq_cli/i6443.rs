//! #6443: a key that is an empty plain scalar (`&a : 1`, `!!str : 1`, `? ` / `: 1`) is written single-quoted, as
//! go-yaml's emitter does for a simple key that would otherwise be empty. An empty plain *value* stays bare
//! (`a: !!str`), and a key the source quoted keeps its quotes. The streaming route (`.`) wrote `""`, where a write
//! already wrote `''`. Every row was captured from yq v4.53.3.

use anyhow::Result;

use crate::run_yq_stdin;

#[test]
fn an_empty_plain_key_is_written_single_quoted_6443() -> Result<()> {
    for (doc, filter, want) in [
        ("&a : 1\nb: 2\n", ".", "&a '': 1\nb: 2\n"),
        ("&a : 1\nb: 2\n", ".zzkey = 1", "&a '': 1\nb: 2\nzzkey: 1\n"),
        ("!!str : 1\n", ".", "!!str '': 1\n"),
        ("!!str : 1\n", ".zzkey = 1", "!!str '': 1\nzzkey: 1\n"),
        ("? \n: 1\n", ".", "'': 1\n"),
        ("? \n: 1\n", ".zzkey = 1", "'': 1\nzzkey: 1\n"),
        ("- &a :\n    b: 1\n", ".", "- &a '':\n    b: 1\n"),
        ("{&a : 1}\n", ".", "{&a '': 1}\n"),
        ("{&a : 1}\n", ".zzkey = 1", "{&a '': 1, zzkey: 1}\n"),
        ("{? : 1}\n", ".", "{'': 1}\n"),
        ("{? : 1}\n", ".zzkey = 1", "{'': 1, zzkey: 1}\n"),
        ("- ? \n  : 1\n", ".", "- '': 1\n"),
        ("? !!str\n: 1\n", ".", "!!str '': 1\n"),
        ("? !!str\n: 1\n", ".zzkey = 1", "!!str '': 1\nzzkey: 1\n"),
        ("x:\n  &a : 1\n", ".", "x:\n  &a '': 1\n"),
        ("x:\n  &a : 1\n", ".zzkey = 1", "x:\n  &a '': 1\nzzkey: 1\n"),
        ("&a : 1\n", ".", "&a '': 1\n"),
        ("&a : 1\n", ".zzkey = 1", "&a '': 1\nzzkey: 1\n"),
        ("a: !!str\n", ".", "a: !!str\n"),
        ("a: !!str\n", ".zzkey = 1", "a: !!str\nzzkey: 1\n"),
        ("a: &x\n", ".", "a: &x\n"),
        ("a: &x\n", ".zzkey = 1", "a: &x\nzzkey: 1\n"),
        ("'': 1\n", ".", "'': 1\n"),
        ("'': 1\n", ".zzkey = 1", "'': 1\nzzkey: 1\n"),
        ("\"\": 1\n", ".", "\"\": 1\n"),
        ("\"\": 1\n", ".zzkey = 1", "\"\": 1\nzzkey: 1\n"),
        ("? ''\n: 1\n", ".", "'': 1\n"),
        ("? ''\n: 1\n", ".zzkey = 1", "'': 1\nzzkey: 1\n"),
        ("&a '': 1\n", ".", "&a '': 1\n"),
        ("&a '': 1\n", ".zzkey = 1", "&a '': 1\nzzkey: 1\n"),
        ("&a \"\": 1\n", ".", "&a \"\": 1\n"),
        ("&a \"\": 1\n", ".zzkey = 1", "&a \"\": 1\nzzkey: 1\n"),
    ] {
        let (stdout, code) = run_yq_stdin(filter, doc, &[])?;
        assert_eq!((stdout.as_str(), code), (want, 0), "{doc:?} {filter}");
    }
    Ok(())
}
