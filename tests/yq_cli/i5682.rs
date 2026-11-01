//! #5682: jq mode splits a string value on a literal separator straight from its text. yq mode
//! keeps its own `split` (a `null` input is nothing, a number raises) and must not take that path.
//! Each expectation is a live yq v4.53.3 capture
//! (`printf 'k: "1,2"\nn: ~\nm: 5\nz: ""\n' | yq -o=json -I0`).

use anyhow::Result;

use crate::run_yq_stdin;

#[test]
fn test_split_of_a_document_value_answers_as_yq_does_5682() -> Result<()> {
    const DOC: &str = "k: \"1,2\"\nn: ~\nm: 5\nz: \"\"\n";
    for (filter, expected, status) in [
        ("[.k, .z] | map(split(\",\"))", "[[\"1\",\"2\"],[]]\n", 0),
        (".n | split(\",\")", "", 0),
        (".m | split(\",\")", "", 1),
        (".k | split(\",\")", "[\"1\",\"2\"]\n", 0),
    ] {
        let (stdout, code) = run_yq_stdin(filter, DOC, &["-o=json", "-I0"])?;
        assert_eq!((stdout.as_str(), code), (expected, status), "{filter}");
    }
    Ok(())
}
