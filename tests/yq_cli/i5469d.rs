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
