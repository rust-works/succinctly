//! #7130: a `:` inside a node property before a block scalar header is no key's colon.
//! `- !<tag:yaml.org,2002:str> |2` was read as the `- key: |2` shape, so the content was measured two
//! columns too far right and the scalar read as empty; the same under a chain of dashes (#7114).
//!
//! Every row is the output of yq v4.53.3 (`yq -o=json -I0 .`, and `yq .` where the row has no verbatim
//! tag: yq prints a verbatim core tag as `!!str`, which is a separate gap).

use anyhow::Result;

use crate::run_yq_stdin;

#[test]
fn a_colon_inside_a_tag_before_a_block_scalar_header_reads_as_yq() -> Result<()> {
    let rows: [(&str, &str, Option<&str>); 17] = [
        (
            "- !<tag:yaml.org,2002:str> |2\n   lead\n- x\n",
            "[\" lead\\n\",\"x\"]\n",
            None,
        ),
        (
            "- - !<tag:yaml.org,2002:str> |2\n     lead\n  - x\n",
            "[[\" lead\\n\",\"x\"]]\n",
            None,
        ),
        (
            "- - - !<tag:yaml.org,2002:str> |2\n       lead\n    - x\n",
            "[[[\" lead\\n\",\"x\"]]]\n",
            None,
        ),
        (
            "- !<tag:yaml.org,2002:str> >2\n   lead\n   more\n- x\n",
            "[\" lead\\n more\\n\",\"x\"]\n",
            None,
        ),
        (
            "- !<tag:yaml.org,2002:str> |2-\n   lead\n- x\n",
            "[\" lead\",\"x\"]\n",
            None,
        ),
        (
            "- !a:b |2\n   lead\n- x\n",
            "[\" lead\\n\",\"x\"]\n",
            Some("- !a:b |2\n   lead\n- x\n"),
        ),
        (
            "- !<tag:yaml.org,2002:str> &anc |2\n   lead\n- x\n",
            "[\" lead\\n\",\"x\"]\n",
            None,
        ),
        (
            "- !<tag:yaml.org,2002:map> k: |2\n    lead\n  j: 1\n",
            "[{\"k\":\"lead\\n\",\"j\":1}]\n",
            None,
        ),
        (
            "- \"a:b\": |2\n    lead\n  j: 1\n",
            "[{\"a:b\":\"lead\\n\",\"j\":1}]\n",
            Some("- \"a:b\": |\n    lead\n  j: 1\n"),
        ),
        (
            "- !<tag:yaml.org,2002:str> |2\n   lead\n\n- x\n",
            "[\" lead\\n\",\"x\"]\n",
            None,
        ),
        (
            "a: !<tag:yaml.org,2002:str> |2\n   lead\nb: 1\n",
            "{\"a\":\" lead\\n\",\"b\":1}\n",
            None,
        ),
        (
            "- - k: !<tag:yaml.org,2002:str> |2\n       lead\n    j: 1\n",
            "[[{\"k\":\" lead\\n\",\"j\":1}]]\n",
            None,
        ),
        (
            "- - !!str |2\n     lead\n  - x\n",
            "[[\" lead\\n\",\"x\"]]\n",
            Some("- - !!str |2\n     lead\n  - x\n"),
        ),
        (
            "- k: !!str |2\n    lead\n  j: 1\n",
            "[{\"k\":\"lead\\n\",\"j\":1}]\n",
            Some("- k: !!str |\n    lead\n  j: 1\n"),
        ),
        (
            "- &a k: |2\n    lead\n  j: 1\n",
            "[{\"k\":\"lead\\n\",\"j\":1}]\n",
            Some("- &a k: |\n    lead\n  j: 1\n"),
        ),
        (
            "- \"a: !x\": |2\n    lead\n  j: 1\n",
            "[{\"a: !x\":\"lead\\n\",\"j\":1}]\n",
            Some("- \"a: !x\": |\n    lead\n  j: 1\n"),
        ),
        (
            "- 'a:b': |2\n    lead\n  j: 1\n",
            "[{\"a:b\":\"lead\\n\",\"j\":1}]\n",
            Some("- 'a:b': |\n    lead\n  j: 1\n"),
        ),
    ];
    for (input, json, yaml) in rows {
        let (stdout, code) = run_yq_stdin(".", input, &["-o=json", "-I0"])?;
        assert_eq!(code, 0, "{input:?}: {stdout}");
        assert_eq!(stdout, json, "json of {input:?}");
        if let Some(yaml) = yaml {
            let (stdout, code) = run_yq_stdin(".", input, &[])?;
            assert_eq!(code, 0, "{input:?}: {stdout}");
            assert_eq!(stdout, yaml, "yaml of {input:?}");
        }
    }
    Ok(())
}
