//! #6408: yq's header slurp trims a line before it looks for `%YA`, so a `%YAML` directive with a space
//! or tab before it, in the head of a stream (before the first content line, and after a leading `---`),
//! is set aside as a column-0 one is (#6342): the stream reads as if it were not there, and it prints
//! back as a comment, `# ` and the line with its indent (`# ⇥%YAML 1.2`). Any other directive, a
//! misspelt `%YAML` and a `...` after the head are go-yaml's, which refuses them. Every row was
//! captured from yq v4.53.3.

use anyhow::Result;

use crate::run_yq_stdin;

#[test]
fn an_indented_yaml_directive_in_the_head_is_set_aside_and_printed_as_a_comment_6408() -> Result<()>
{
    for (doc, json, yaml) in [
        (" %YAML 1.2\n", "null\n", "#  %YAML 1.2\n"),
        (
            "  %YAML 1.2\na: 1\n",
            "{\"a\":1}\n",
            "#   %YAML 1.2\na: 1\n",
        ),
        (
            " %YAML 1.2\n---\na: 1\n",
            "{\"a\":1}\n",
            "#  %YAML 1.2\n---\na: 1\n",
        ),
        ("\n %YAML 1.2\n", "null\n", "\n#  %YAML 1.2\n"),
        (
            "# c\n %YAML 1.2\na: 1\n",
            "{\"a\":1}\n",
            "# c\n#  %YAML 1.2\na: 1\n",
        ),
        (" %YAML 1.2\nb\n", "\"b\"\n", "#  %YAML 1.2\nb\n"),
        ("\t%YAML 1.2\n", "null\n", "# \t%YAML 1.2\n"),
        (
            " \t%YAML 1.2\na: 1\n",
            "{\"a\":1}\n",
            "#  \t%YAML 1.2\na: 1\n",
        ),
        ("---\n %YAML 1.2\n", "null\n", "---\n#  %YAML 1.2\n"),
        (
            "---\n %YAML 1.2\na: 1\n",
            "{\"a\":1}\n",
            "---\n#  %YAML 1.2\na: 1\n",
        ),
        (" %YAML 1.1\n", "null\n", "#  %YAML 1.1\n"),
        (" %YAML\n", "null\n", "#  %YAML\n"),
        (
            " %YAML 1.2 # c\na: 1\n",
            "{\"a\":1}\n",
            "#  %YAML 1.2 # c\na: 1\n",
        ),
        (
            "  %YAML 1.2\n  %YAML 1.2\n",
            "null\n",
            "#   %YAML 1.2\n#   %YAML 1.2\n",
        ),
        (
            "%YAML 1.2\n %YAML 1.2\na: 1\n",
            "{\"a\":1}\n",
            "%YAML 1.2\n#  %YAML 1.2\na: 1\n",
        ),
        (" %YAMLX y\na: 1\n", "{\"a\":1}\n", "#  %YAMLX y\na: 1\n"),
        (
            "  # c\n %YAML 1.2\na: 1\n",
            "{\"a\":1}\n",
            "  # c\n#  %YAML 1.2\na: 1\n",
        ),
        (" %YAML 1.2\n- a\n", "[\"a\"]\n", "#  %YAML 1.2\n- a\n"),
        (" %YAML 1.2\n\"q\"\n", "\"q\"\n", "#  %YAML 1.2\nq\n"),
        (
            " %YAML 1.2\n\na: 1\n",
            "{\"a\":1}\n",
            "#  %YAML 1.2\n\na: 1\n",
        ),
        (
            " %YAML 1.2\n %YAML 1.1\na: 1\n",
            "{\"a\":1}\n",
            "#  %YAML 1.2\n#  %YAML 1.1\na: 1\n",
        ),
        (
            "---\n---\n %YAML 1.2\na: 1\n",
            "{\"a\":1}\n",
            "---\n---\n#  %YAML 1.2\na: 1\n",
        ),
        (
            " %YAML 1.2\n%TAG ! x:\n---\na: 1\n",
            "{\"a\":1}\n",
            "#  %YAML 1.2\na: 1\n",
        ),
        (
            "a: 1\n %YAML 1.2\n---\n",
            "{\"a\":\"1 %YAML 1.2\"}\nnull\n",
            "a: 1 %YAML 1.2\n---\n\n",
        ),
    ] {
        let (stdout, code) = run_yq_stdin(".", doc, &["-o=json", "-I=0"])?;
        assert_eq!((stdout.as_str(), code), (json, 0), "{doc:?} as JSON");
        let (stdout, code) = run_yq_stdin(".", doc, &[])?;
        assert_eq!((stdout.as_str(), code), (yaml, 0), "{doc:?} as YAML");
    }
    Ok(())
}

#[test]
fn an_indented_directive_that_is_not_the_heads_yaml_one_is_refused_6408() -> Result<()> {
    for doc in [
        " %YAML 1.2\n...\n",
        " %YAM 1.2\n",
        " %yaml 1.2\n",
        " %TAG ! x:\n",
        "%TAG ! x:\n %YAML 1.2\n---\na: 1\n",
    ] {
        let (stdout, code) = run_yq_stdin(".", doc, &["-o=json", "-I=0"])?;
        assert_ne!(code, 0, "{doc:?} is refused");
        assert_eq!(stdout, "", "{doc:?} prints nothing");
    }
    Ok(())
}
