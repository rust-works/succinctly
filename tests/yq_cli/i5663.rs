//! #5663: a chain of field or index reads over a computed value is stepped without a re-entry per
//! stage. In yq mode an absent key and a read of a scalar answer nothing, where jq answers `null` or
//! raises, and the walk has to stop on exactly those. Each expectation is a live yq v4.53.3 capture
//! (`yq -n -o=json -I0`).

use anyhow::Result;

use crate::run_yq_stdin;

#[test]
fn test_chains_of_reads_over_a_computed_value_answer_as_yq_does_5663() -> Result<()> {
    for (filter, expected, status) in [
        ("{\"a\":{\"a\":{\"a\":3}}} | .a.a.a", "3\n", 0),
        ("{\"a\":{\"a\":{\"a\":3}}} | .a.a.a.a.a", "", 0),
        ("{\"a\":[1,2]} | .a[-1]", "2\n", 0),
        ("{\"a\":[1,2]} | .a[5]", "null\n", 0),
        ("{\"a\":[1,2]} | .a[0].b", "", 0),
        ("{\"a\":{\"b\":1}} | .a.b.c.d", "", 0),
        ("{\"a\":1} | .b.c.d", "null\n", 0),
        ("[[1]] | .[0][0][0]", "", 0),
        ("[[1]] | .[0][0] | . + 1", "2\n", 0),
        ("{\"a\":{\"b\":{\"c\":\"x\"}}} | .a.b.c | length", "1\n", 0),
        ("{\"a\":[{\"b\":[10,20]}]} | .a[0].b[1]", "20\n", 0),
        ("{} | .a.a.a | length", "0\n", 0),
    ] {
        let (stdout, code) = run_yq_stdin(filter, "", &["-n", "-o=json", "-I0"])?;
        assert_eq!((stdout.as_str(), code), (expected, status), "{filter}");
    }
    Ok(())
}
