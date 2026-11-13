//! #7114: a block scalar with an explicit indentation indicator, as the first element of a
//! sequence nested directly in a sequence (`- - |2-`), is measured against the inner sequence's
//! dash. The scalar's content was stripped as if its parent were the outer dash, so the next
//! sibling's line (`- x`) became part of the text and the lines were indented two columns too far.
//!
//! Every row is the output of yq v4.53.3 (`yq -o=json -I0 .` and `yq .`) over the same input.

use anyhow::Result;

use crate::run_yq_stdin;

#[test]
fn an_explicit_indicator_in_a_compact_nested_sequence_reads_as_yq() -> Result<()> {
    let rows = [
        (
            "name:\n  - - |2-\n       lead\n      more\n    - x\n  - z\n",
            "{\"name\":[[\" lead\\nmore\",\"x\"],\"z\"]}\n",
            "name:\n  - - |2-\n       lead\n      more\n    - x\n  - z\n",
        ),
        (
            "a:\n  - - |2-\n       lead\n    - x\n",
            "{\"a\":[[\" lead\",\"x\"]]}\n",
            "a:\n  - - |2-\n       lead\n    - x\n",
        ),
        (
            "- - |2-\n     lead\n    more\n  - x\n",
            "[[\" lead\\nmore\",\"x\"]]\n",
            "- - |2-\n     lead\n    more\n  - x\n",
        ),
        (
            "- - |2\n     lead\n  - x\n",
            "[[\" lead\\n\",\"x\"]]\n",
            "- - |2\n     lead\n  - x\n",
        ),
        (
            "- - - |2-\n         lead\n      - x\n",
            "[[[\"   lead\\n- x\"]]]\n",
            "- - - |2-\n         lead\n      - x\n",
        ),
        (
            "- - k: |2\n       lead\n    j: 1\n",
            "[[{\"k\":\" lead\\n\",\"j\":1}]]\n",
            "- - k: |2\n       lead\n    j: 1\n",
        ),
        (
            "- - >2\n     lead\n     more\n  - x\n",
            "[[\" lead\\n more\\n\",\"x\"]]\n",
            "- - >2\n     lead\n     more\n  - x\n",
        ),
        (
            "-   - |2\n       lead\n    - x\n",
            "[[\" lead\\n\",\"x\"]]\n",
            "- - |2\n     lead\n  - x\n",
        ),
        (
            "- - |-\n    lead\n  - x\n",
            "[[\"lead\",\"x\"]]\n",
            "- - |-\n    lead\n  - x\n",
        ),
        (
            "- - |2+\n     lead\n\n  - x\n",
            "[[\" lead\\n\\n\",\"x\"]]\n",
            "- - |2+\n     lead\n\n  - x\n",
        ),
        (
            "- - - k: |2-\n         lead\n      j: 1\n",
            "[[[{\"k\":\" lead\",\"j\":1}]]]\n",
            "- - - k: |2-\n         lead\n      j: 1\n",
        ),
        (
            "- k:\n    - |2-\n         lead\n    - x\n",
            "[{\"k\":[\"   lead\",\"x\"]}]\n",
            "- k:\n    - |2-\n         lead\n    - x\n",
        ),
        (
            "- - |1\n   lead\n  - x\n",
            "[[\"lead\\n\",\"x\"]]\n",
            "- - |\n    lead\n  - x\n",
        ),
    ];
    for (input, json, yaml) in rows {
        let (stdout, code) = run_yq_stdin(".", input, &["-o=json", "-I0"])?;
        assert_eq!(code, 0, "{input:?}: {stdout}");
        assert_eq!(stdout, json, "json of {input:?}");
        let (stdout, code) = run_yq_stdin(".", input, &[])?;
        assert_eq!(code, 0, "{input:?}: {stdout}");
        assert_eq!(stdout, yaml, "yaml of {input:?}");
    }
    Ok(())
}
