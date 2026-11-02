//! #5469: a `keys` result built into a construction of reads (`[.c | keys]`, `{x: (.c | keys)}`)
//! writes each key as its node, as a bare `.c | keys` does: anchor and tag first, `!!merge` on a
//! plain `<<`, quoting kept. Every matching row was captured from yq v4.53.3.

use anyhow::Result;

use crate::run_yq_stdin_with_stderr;

const DOC: &str = "b: &a xy\nc:\n  &k !!str key: 1\n  !foo bar: 2\n  <<: {p: 1}\n  \"<<\": 3\n  !!str 1: 4\n  plain: 6\n";

const KEYS: &str =
    "- &k !!str key\n  - !foo bar\n  - !!merge <<\n  - \"<<\"\n  - !!str 1\n  - plain\n";

#[test]
fn test_keys_in_a_construction_writes_each_key_as_its_node_5469() -> Result<()> {
    for (filter, want) in [
        ("[.c | keys]", format!("- {KEYS}")),
        (".c | [keys]", format!("- {KEYS}")),
        ("{\"x\": (.c | keys)}", format!("x:\n  {KEYS}")),
        ("[.c | keys, .b]", format!("- {KEYS}- &a xy\n")),
        ("[keys]", "- - b\n  - c\n".to_string()),
    ] {
        let (stdout, stderr, code) = run_yq_stdin_with_stderr(filter, DOC, &[])?;
        assert_eq!(
            (stdout.as_str(), code),
            (want.as_str(), 0),
            "{filter}: {stderr:?}"
        );
    }
    Ok(())
}

/// An alias key in a construction is `*a` once its anchor is written before it, as yq writes it.
/// Without one, the anchor-soundness rule prints the key's text where yq prints an unresolvable
/// `*a` (recorded in `docs/compliance/yq/limitations.md`). `-P` drops a tag that restates the
/// key's type, as it does for a bare `keys`.
#[test]
fn test_keys_in_a_construction_holds_an_alias_key_to_anchor_soundness_5469() -> Result<()> {
    let doc = "b: &a xy\nc:\n  &k !!str key: 1\n  *a : 5\n";
    for (filter, args, want) in [
        (
            "[.b, (.c | keys)]",
            &[][..],
            "- &a xy\n- - &k !!str key\n  - *a\n",
        ),
        ("[.c | keys]", &[][..], "- - &k !!str key\n  - xy\n"),
        ("[.c | keys]", &["-P"][..], "- - &k key\n  - xy\n"),
        (
            "[.c | keys]",
            &["-o=json", "-I0"][..],
            "[[\"key\",\"xy\"]]\n",
        ),
    ] {
        let (stdout, stderr, code) = run_yq_stdin_with_stderr(filter, doc, args)?;
        assert_eq!(
            (stdout.as_str(), code),
            (want, 0),
            "{filter} {args:?}: {stderr:?}"
        );
    }
    Ok(())
}

/// The keys of a sequence are its indices, and the construction keeps its other entries' nodes;
/// a scalar has no keys, an error as before.
#[test]
fn test_keys_in_a_construction_of_a_sequence_or_scalar_5469() -> Result<()> {
    let (stdout, stderr, code) =
        run_yq_stdin_with_stderr("[.b, (.d | keys)]", "b: &a xy\nd: [x, y]\n", &[])?;
    assert_eq!(
        (stdout.as_str(), code),
        ("- &a xy\n- - 0\n  - 1\n", 0),
        "{stderr:?}"
    );
    let (stdout, _, code) = run_yq_stdin_with_stderr("[.d | keys]", "d: x\n", &[])?;
    assert_eq!((stdout.as_str(), code), ("", 1));
    Ok(())
}
