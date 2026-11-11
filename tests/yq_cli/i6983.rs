//! #6983: jq's `limit` and `nth` compare a count literal with `0` exactly, as jq compares two number
//! literals. yq has no such comparison: its numbers are doubles, and its parser keeps `-1e-400` a
//! literal where jq's negates `1e-400`. So under `--jq-extensions` a count whose double is `-0.0`
//! stays zero, as `succinctly yq` read it before #6983 and as jq reads the same program text.

use anyhow::Result;

use crate::run_yq_stdin;

#[test]
fn yq_mode_reads_a_count_literal_by_its_double_6983() -> Result<()> {
    for (filter, want) in [
        ("[limit(-1e-400; 1, 2)]", "[]\n"),
        ("[nth(-1e-400; 1, 2)]", "[1]\n"),
        ("[limit(.n; 1, 2)]", "[]\n"),
        ("[nth(.n; 1, 2)]", "[1]\n"),
        ("[limit(2; 1, 2, 3)]", "[1,2]\n"),
    ] {
        let (stdout, code) = run_yq_stdin(
            filter,
            "n: -1e-400\n",
            &["--jq-extensions", "-o=json", "-I=0"],
        )?;
        assert_eq!((stdout.as_str(), code), (want, 0), "{filter}");
    }
    Ok(())
}
