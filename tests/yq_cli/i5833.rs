//! #5833: YAML resolves a plain `<<` to the `!!merge` tag wherever it sits, value as well as key,
//! and yq writes that tag (`a: !!merge <<`) and reports it (`tag`, `type`). A quoted `"<<"` and an
//! explicitly tagged `!!str <<` are strings, and a computed `"<<"` is written quoted, since a plain
//! one would read back as the merge tag. Every row was captured from yq v4.53.3.

use anyhow::Result;

use crate::run_yq_stdin_with_stderr;

const DOC: &str = "a: <<\nq: \"<<\"\ns: '<<'\nt: !!str <<\nb: [<<, x]\nc:\n  - <<\n  - y\nd: &m <<\ne: *m\nf: {k: <<}\n";

const WRITTEN: &str = "a: !!merge <<\nq: \"<<\"\ns: '<<'\nt: !!str <<\nb: [!!merge <<, x]\nc:\n  - !!merge <<\n  - y\nd: &m !!merge <<\ne: *m\nf: {k: !!merge <<}\n";

/// The streaming route (`.`) and the DOM routes a write or a construction takes.
#[test]
fn test_plain_merge_scalar_is_written_with_its_tag_5833() -> Result<()> {
    for (filter, want) in [
        (".", WRITTEN.to_string()),
        (".x = 1", format!("{WRITTEN}x: 1\n")),
        (
            "[.a, .q, .t, .d, .b]",
            "- !!merge <<\n- \"<<\"\n- !!str <<\n- &m !!merge <<\n- [!!merge <<, x]\n".to_string(),
        ),
        (".c", "- !!merge <<\n- y\n".to_string()),
        // A scalar result is printed unwrapped, without its tag, as yq does.
        (".a", "<<\n".to_string()),
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

/// `-P` keeps `!!merge` and drops the `!!str` that only restates a string, which then has to be
/// quoted, as `s` is.
#[test]
fn test_plain_merge_scalar_under_pretty_print_5833() -> Result<()> {
    let (stdout, stderr, code) = run_yq_stdin_with_stderr(".", DOC, &["-P"])?;
    assert_eq!(
        (stdout.as_str(), code),
        (
            "a: !!merge <<\nq: \"<<\"\ns: \"<<\"\nt: \"<<\"\nb:\n  - !!merge <<\n  - x\nc:\n  - !!merge <<\n  - y\nd: &m !!merge <<\ne: *m\nf:\n  k: !!merge <<\n",
            0
        ),
        "{stderr:?}"
    );
    Ok(())
}

#[test]
fn test_tag_and_type_of_a_plain_merge_scalar_5833() -> Result<()> {
    for (filter, want) in [
        (".a | tag", "!!merge\n"),
        (".a | type", "!!merge\n"),
        (".b[0] | tag", "!!merge\n"),
        ("[.a] | .[0] | tag", "!!merge\n"),
        (".d | tag", "!!merge\n"),
        (".q | tag", "!!str\n"),
        (".s | tag", "!!str\n"),
        (".t | tag", "!!str\n"),
        // An alias node's tag is empty, as for any alias.
        (".e | tag", "\n"),
        (".a | kind", "scalar\n"),
        (".a == \"<<\"", "true\n"),
    ] {
        let (stdout, stderr, code) = run_yq_stdin_with_stderr(filter, DOC, &[])?;
        assert_eq!((stdout.as_str(), code), (want, 0), "{filter}: {stderr:?}");
    }
    // A mapping whose first key is `<<` starts at the same byte; it is still a mapping.
    let (stdout, stderr, code) =
        run_yq_stdin_with_stderr(".h | tag", "h:\n  <<: {p: 1}\n  z: 2\n", &[])?;
    assert_eq!((stdout.as_str(), code), ("!!map\n", 0), "{stderr:?}");
    Ok(())
}

/// A computed `"<<"` is a string, which a plain `<<` would not read back as.
#[test]
fn test_a_computed_merge_spelled_string_is_quoted_5833() -> Result<()> {
    for (filter, want) in [
        (".z = \"<<\"", format!("{WRITTEN}z: \"<<\"\n")),
        (".z = [\"<<\"]", format!("{WRITTEN}z:\n  - \"<<\"\n")),
        (". * {\"n\": \"<<\"}", format!("{WRITTEN}n: \"<<\"\n")),
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

/// A JSON document's `"<<"` is an ordinary string: quoted, with no tag, and `!!str`.
#[test]
fn test_a_json_merge_spelled_string_stays_a_string_5833() -> Result<()> {
    let json = "{\"a\": \"<<\", \"b\": [\"<<\"]}";
    for (filter, want) in [
        (".", "a: \"<<\"\nb:\n  - \"<<\"\n"),
        (".x = 1", "a: \"<<\"\nb:\n  - \"<<\"\nx: 1\n"),
        (".a | tag", "!!str\n"),
    ] {
        let (stdout, stderr, code) =
            run_yq_stdin_with_stderr(filter, json, &["-p", "json", "-o", "yaml"])?;
        assert_eq!((stdout.as_str(), code), (want, 0), "{filter}: {stderr:?}");
    }
    Ok(())
}

/// A plain scalar that only starts with `<<` is an ordinary string, on every route (`-i` among
/// them, which must not tag a heredoc-style value in the user's file).
#[test]
fn test_a_plain_scalar_starting_with_merge_spelling_is_a_string_5833() -> Result<()> {
    let doc = "run: << EOF\nb: [<< x, y]\nc: <<\n  foo\nd: <<\n";
    let written = "run: << EOF\nb: [<< x, y]\nc: << foo\nd: !!merge <<\nx: 1\n";
    for (filter, args, want) in [
        (".run | tag", &[][..], "!!str\n"),
        (".c | tag", &[][..], "!!str\n"),
        (".b[0] | tag", &[][..], "!!str\n"),
        (".x = 1", &[][..], written),
        (
            ".x = 1",
            &["-P"][..],
            "run: << EOF\nb:\n  - << x\n  - y\nc: << foo\nd: !!merge <<\nx: 1\n",
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

/// `tag =` on a plain `<<`: `!!str` makes it a string, quoted, and `!!merge` is the tag it
/// already resolves to, written. Clearing the tag gives `!!merge <<` where yq writes `<<`, which
/// reads back the same (recorded in `docs/compliance/yq/limitations.md`).
#[test]
fn test_tag_assignment_on_a_plain_merge_scalar_5833() -> Result<()> {
    for (filter, want) in [
        (".a tag = \"!!str\"", "a: \"<<\"\n"),
        (".a tag = \"!!merge\"", "a: !!merge <<\n"),
        (".a tag = \"!x\"", "a: !x <<\n"),
        (".a tag = \"\"", "a: !!merge <<\n"),
    ] {
        let (stdout, stderr, code) = run_yq_stdin_with_stderr(filter, "a: <<\n", &[])?;
        assert_eq!((stdout.as_str(), code), (want, 0), "{filter}: {stderr:?}");
    }
    Ok(())
}
