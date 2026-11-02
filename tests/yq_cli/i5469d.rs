//! #5469: `keys` writes each key as its node, as yq does: the key's own anchor and explicit tag
//! come first (`&k !!str key`), a plain `<<` takes yq's `!!merge`, a tagged plain key stays plain
//! (`!!str 1`), and an alias key is `*a`. Both routes do, the streaming one and the DOM one a
//! flag such as `-P` takes. Before, the streaming route kept only a key's quoting and the DOM one
//! kept nothing. Every matching row was captured from yq v4.53.3.

use anyhow::Result;

use crate::run_yq_stdin_with_stderr;

const DOC: &str = "b: &a xy\nc:\n  &k !!str key: 1\n  !foo bar: 2\n  <<: {p: 1}\n  \"<<\": 3\n  !!str 1: 4\n  *a : 5\n  plain: 6\n";

#[test]
fn test_keys_writes_each_key_as_its_node_5469() -> Result<()> {
    let streamed = "- &k !!str key\n- !foo bar\n- !!merge <<\n- \"<<\"\n- !!str 1\n- *a\n- plain\n";
    for (args, want) in [
        (&[][..], streamed),
        (&["-I4"][..], streamed),
        (
            &["-o=json", "-I0"][..],
            "[\"key\",\"bar\",\"<<\",\"<<\",\"1\",\"xy\",\"plain\"]\n",
        ),
    ] {
        let (stdout, stderr, code) = run_yq_stdin_with_stderr(".c | keys", DOC, args)?;
        assert_eq!((stdout.as_str(), code), (want, 0), "{args:?}: {stderr:?}");
    }
    Ok(())
}

/// `-P` drops a core tag that only restates the key's type (`!!str key`, `!!str 1`, which then
/// needs its quotes) and keeps the rest, as yq does. The alias key is the one recorded
/// divergence (anchor soundness, `docs/compliance/yq/limitations.md`): the result holds no
/// `&a`, so where yq prints an unresolvable `*a` this route prints the key's text.
#[test]
fn test_keys_under_pretty_print_keeps_each_key_node_5469() -> Result<()> {
    let (stdout, stderr, code) = run_yq_stdin_with_stderr(".c | keys", DOC, &["-P"])?;
    assert_eq!(
        (stdout.as_str(), code),
        (
            "- &k key\n- !foo bar\n- !!merge <<\n- \"<<\"\n- \"1\"\n- xy\n- plain\n",
            0
        ),
        "{stderr:?}"
    );
    Ok(())
}

/// A JSON document's `"<<"` key is an ordinary string: yq prints it quoted, with no `!!merge`.
#[test]
fn test_keys_of_a_json_document_gives_a_merge_spelled_key_no_tag_5469() -> Result<()> {
    for args in [
        &["-p", "json", "-o", "yaml"][..],
        &["-p", "json", "-o", "yaml", "-P"][..],
    ] {
        let (stdout, stderr, code) =
            run_yq_stdin_with_stderr("keys", "{\"a\": 1, \"<<\": 2}", args)?;
        assert_eq!(
            (stdout.as_str(), code),
            ("- a\n- \"<<\"\n", 0),
            "{args:?}: {stderr:?}"
        );
    }
    Ok(())
}

/// The DOM routes a flag takes (`--arg`, `--sort-keys`, `-P`) give each key its tag too, for a
/// document with no merge key or alias, which used to reach a route that dropped it (yq v4.53.3).
#[test]
fn test_keys_on_the_dom_routes_keeps_a_key_tag_5469() -> Result<()> {
    for (args, want) in [
        (&["--arg", "x", "y"][..], "- !foo q\n- &k !!str r\n"),
        (&["--sort-keys"][..], "- !foo q\n- &k !!str r\n"),
        (&["-P"][..], "- !foo q\n- &k r\n"),
    ] {
        let (stdout, stderr, code) =
            run_yq_stdin_with_stderr("keys", "!foo q: 1\n&k !!str r: 2\n", args)?;
        assert_eq!((stdout.as_str(), code), (want, 0), "{args:?}: {stderr:?}");
    }
    Ok(())
}

/// The edges of a key's properties, each as yq v4.53.3 writes it on the streaming route: an empty
/// tagged or anchored key is its properties alone, and the non-specific `!` is dropped.
#[test]
fn test_keys_writes_an_empty_or_non_specific_tagged_key_as_yq_5469() -> Result<()> {
    for (doc, want) in [
        ("!!str : 1\n", "- !!str\n"),
        ("!foo : 1\n", "- !foo\n"),
        ("&k : 1\n", "- &k\n"),
        ("&k !!str : 1\n", "- &k !!str\n"),
        ("{!!str : 1}\n", "- !!str\n"),
        ("! z: 1\n", "- z\n"),
    ] {
        let (stdout, stderr, code) = run_yq_stdin_with_stderr("keys", doc, &[])?;
        assert_eq!((stdout.as_str(), code), (want, 0), "{doc:?}: {stderr:?}");
    }
    Ok(())
}

/// A tagged plain key that decodes to a line break cannot be written as it was spelled. yq writes
/// a literal block, as the `--arg` route here does; the streaming route and `--sort-keys`
/// double-quote it (recorded in `limitations.md`). Both read back as the key `"a\nb"`, where writing the text raw did not.
#[test]
fn test_keys_quotes_a_tagged_key_holding_a_line_break_5469() -> Result<()> {
    let doc = "? !!str a\n\n  b\n: 1\n";
    for (args, want) in [
        (&[][..], "- !!str \"a\\nb\"\n"),
        (&["--arg", "x", "y"][..], "- !!str |-\n  a\n  b\n"),
        (&["--sort-keys"][..], "- !!str \"a\\nb\"\n"),
    ] {
        let (stdout, stderr, code) = run_yq_stdin_with_stderr("keys", doc, args)?;
        assert_eq!((stdout.as_str(), code), (want, 0), "{args:?}: {stderr:?}");
        let (reread, stderr, code) = run_yq_stdin_with_stderr(".", &stdout, &["-o=json", "-I0"])?;
        assert_eq!(
            (reread.as_str(), code),
            ("[\"a\\nb\"]\n", 0),
            "{args:?}: {stderr:?}"
        );
    }
    Ok(())
}
