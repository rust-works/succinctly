//! #5469: yq single-quotes a plain timestamp (or any plain scalar with a flow indicator)
//! inside `[...]`/`{...}` on every route; the DOM writer (a write, `-P`) left it bare. Rows
//! compared with the pinned yq's flow-context quoting rule, which decides on characters alone.

use anyhow::Result;

use crate::run_yq_stdin;

#[test]
fn test_dom_write_single_quotes_a_flow_timestamp_5469() -> Result<()> {
    for (filter, input, want) in [
        (
            ".zzkey = 1",
            "a: [2001-12-14T21:59:43Z]\n",
            "a: ['2001-12-14T21:59:43Z']\nzzkey: 1\n",
        ),
        (
            ".z = 1",
            "a: {k: 2001-12-14T21:59:43Z, j: 12:30:45}\n",
            "a: {k: '2001-12-14T21:59:43Z', j: '12:30:45'}\nz: 1\n",
        ),
        (
            ".z = 1",
            "a: [12:30:45, a?b, x]\n",
            "a: ['12:30:45', 'a?b', x]\nz: 1\n",
        ),
        // Block context leaves a timestamp bare.
        (
            ".z = 1",
            "a: 2001-12-14T21:59:43Z\n",
            "a: 2001-12-14T21:59:43Z\nz: 1\n",
        ),
    ] {
        let (out, code) = run_yq_stdin(filter, input, &[])?;
        assert_eq!((out.as_str(), code), (want, 0), "{filter} over {input:?}");
    }
    Ok(())
}
