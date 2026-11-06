//! #6412: a `%` directive line before a stream's first `---` that go-yaml's scanner refuses is refused, not
//! skipped. yq's header slurp sets aside only a line that starts `%YAML`; every other directive reaches
//! go-yaml, which wants a name that is `YAML` or `TAG` (an unknown name is `found unknown directive name`, none
//! is `could not find expected directive name`) and, for `%TAG`, a handle (`!`, `!!` or `!word!`), a prefix
//! and then at most a `#` comment, with no handle given twice. The library loader keeps the YAML 1.2 reading,
//! where an unknown directive is ignored. Every row was captured from yq v4.53.3.

use anyhow::Result;

use crate::run_yq_stdin_with_stderr;

const JSON: &[&str] = &["-o=json", "-I=0"];

#[test]
fn a_directive_go_yaml_refuses_is_refused_6412() -> Result<()> {
    for doc in [
        "%FOO\n---\na: 1\n",
        "%\n---\na: 1\n",
        "% FOO\n---\na: 1\n",
        "%TAG\n---\na: 1\n",
        "%TAG !\n---\na: 1\n",
        "%TAG ! x: y\n---\na: 1\n",
        "%TAG x y\n---\na: 1\n",
        "%TAG ! x:# c\n---\na: 1\n",
        "%TAG !a b! x\n---\na: 1\n",
        "%FOO bar\n---\na: 1\n",
        "%foo\n---\na: 1\n",
        "%F-O_o\n---\na: 1\n",
        "%1\n---\na: 1\n",
        "%-\n---\na: 1\n",
        "%FOO # c\n---\na: 1\n",
        "%FOO#c\n---\na: 1\n",
        "%YAM\n---\na: 1\n",
        "%YA\n---\na: 1\n",
        "%TAG ! x:\n%TAG ! y:\n---\na: 1\n",
        "%TAG !e! x\n%TAG !f! y\n%TAG !e! z\n---\na: 1\n",
        "%TAG ! x:\n%FOO\n---\na: 1\n",
        "%FOO\n%TAG ! x:\n---\na: 1\n",
        "%YAML 1.2\n%FOO\n---\na: 1\n",
        "# c\n%FOO\n---\na: 1\n",
        "\n%FOO\n---\na: 1\n",
        "%FOO\n---\n",
        "%FOO\n...\n---\na: 1\n",
        "%TAG\n...\n---\na: 1\n",
    ] {
        let (out, _err, code) = run_yq_stdin_with_stderr(".", doc, JSON)?;
        assert_eq!(out, "", "{doc:?}");
        assert_ne!(code, 0, "{doc:?}");
    }
    Ok(())
}

#[test]
fn a_directive_go_yaml_reads_is_still_read_6412() -> Result<()> {
    for (doc, want) in [
        ("%TAG ! x:\n---\na: 1\n", "{\"a\":1}\n"),
        ("%TAG !! tag:y,2000:\n---\na: 1\n", "{\"a\":1}\n"),
        ("%TAG ! x: # c\n---\na: 1\n", "{\"a\":1}\n"),
        ("%TAG !e! x\n%TAG !f! y\n---\na: 1\n", "{\"a\":1}\n"),
        ("%TAG !a-b_1! z\n---\na: 1\n", "{\"a\":1}\n"),
        ("%TAG\t!\tx\n---\na: 1\n", "{\"a\":1}\n"),
        ("%TAG  !  x  \n---\na: 1\n", "{\"a\":1}\n"),
        ("%YAMLL 1.2\n---\na: 1\n", "{\"a\":1}\n"),
        ("%YAML 9\n---\na: 1\n", "{\"a\":1}\n"),
        ("%YAML\n---\na: 1\n", "{\"a\":1}\n"),
        ("%YAML 1.2\n%TAG ! x:\n---\na: 1\n", "{\"a\":1}\n"),
        ("%TAG !e! tag:e.com,2000:\n---\n!e!x 1\n", "1\n"),
        (
            "a: 1\n...\n%TAG ! x:\n---\nb: 1\n",
            "{\"a\":1}\n{\"b\":1}\n",
        ),
    ] {
        let (out, _err, code) = run_yq_stdin_with_stderr(".", doc, JSON)?;
        assert_eq!((out.as_str(), code), (want, 0), "{doc:?}");
    }
    Ok(())
}
