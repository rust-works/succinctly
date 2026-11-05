//! #6343: a scalar, alias or flow line that stands level with a mapping's keys (or a block sequence's
//! `-` markers) after an entry with no value to attach it to is `while scanning a simple key` in yq;
//! `succinctly yq` dropped it (`a: |` / `  x` / `stray`), read it as a value (`? k` / `q`, `a:` / `"q"`)
//! or read a key and a value that are in no document (`a: |` / `"q"` / `b: 1`). Every row was captured
//! from yq v4.53.3 (`yq -o=json -I0 .`).

use anyhow::Result;

use crate::run_yq_stdin_with_stderr;

const ARGS: &[&str] = &["-o=json", "-I=0"];

#[test]
fn test_a_node_with_no_parent_at_a_containers_indent_is_refused_as_yq_refuses_it_6343() -> Result<()>
{
    for doc in [
        "a: |\n  x\n|\n  y\n",
        "a: 1\n!!str q\n",
        "a: 1\n&x q\n",
        "a: 1\n|\n x\n",
        "- a\n!!str q\n",
        "a: |\n  x\nstray\n",
        "a: |\n\"q\"\n",
        "a: |\n\"q\"\nb: 1\n",
        "a: >\nstray\n",
        "k: &a\nq\n",
        "? k\nq\n",
        "? k\n\"q\"\n",
        "? k\n[1]\n",
        "?\nq\n: v\n",
        "- |\nstray\n",
        "- |\n  x\ny\n",
        "a:\nq\n",
        "a:\n\"q\"\n",
        "a: 1\n[1]\n",
        "a: 1\n*x\n",
        "- a\n[1]\n",
        "- a\n\"q\"\n",
        "- -\nq\n",
        "a:\n- x\nq\n",
        "- a: 1\n  b\n",
        "a:\n  b: 1\nc\n",
    ] {
        let (stdout, stderr, code) = run_yq_stdin_with_stderr(".", doc, ARGS)?;
        assert_ne!(code, 0, "{doc:?} should be refused, got {stdout:?}");
        assert!(
            stderr.contains("neither a key, a sequence item nor a continuation"),
            "{doc:?}: {stderr:?}"
        );
    }
    Ok(())
}

#[test]
fn test_a_value_on_its_own_line_or_an_entry_after_a_block_scalar_still_reads_6343() -> Result<()> {
    for (doc, expected) in [
        ("a:\n|\n  x\n", "{\"a\":\"x\\n\"}"),
        ("---\na: 1\n---\nq\n", "{\"a\":1}\n\"q\""),
        ("a: 1\n---\n\"q\"\n", "{\"a\":1}\n\"q\""),
        ("a:\n  |\n  x\n", "{\"a\":\"x\\n\"}"),
        ("- |\n  x\n- y\n", "[\"x\\n\",\"y\"]"),
        ("a: |\n  x\nb: 1\n", "{\"a\":\"x\\n\",\"b\":1}"),
        ("a:\n  - x\nb: 1\n", "{\"a\":[\"x\"],\"b\":1}"),
        ("?\n  q\n: v\n", "{\"q\":\"v\"}"),
        ("? k\n: v\n", "{\"k\":\"v\"}"),
        ("a: &a\n  q\n", "{\"a\":\"q\"}"),
        ("a: &a\n  - q\n", "{\"a\":[\"q\"]}"),
        ("- a\n  b\n", "[\"a b\"]"),
        ("- a\n- b\n", "[\"a\",\"b\"]"),
        ("a:\n- x\n- y\nb: 1\n", "{\"a\":[\"x\",\"y\"],\"b\":1}"),
        ("- - x\n  - y\n- z\n", "[[\"x\",\"y\"],\"z\"]"),
        ("- |\n  x\n- y\n", "[\"x\\n\",\"y\"]"),
        ("- &a\n  x\n", "[\"x\"]"),
        ("-\n  x\n- y\n", "[\"x\",\"y\"]"),
        ("- a\n\n- b\n", "[\"a\",\"b\"]"),
        ("- >\n  x\n  y\n- z\n", "[\"x y\\n\",\"z\"]"),
        ("a: &a\n  \"q\"\nb: *a\n", "{\"a\":\"q\",\"b\":\"q\"}"),
        ("? a\n? b\n: c\n", "{\"a\":null,\"b\":\"c\"}"),
        (
            "a: [1,\n  2]\nb: {k:\n  v}\n",
            "{\"a\":[1,2],\"b\":{\"k\":\"v\"}}",
        ),
    ] {
        let (stdout, stderr, code) = run_yq_stdin_with_stderr(".", doc, ARGS)?;
        assert_eq!(code, 0, "{doc:?}: {stderr:?}");
        assert_eq!(stdout.trim(), expected, "{doc:?}");
    }
    Ok(())
}
