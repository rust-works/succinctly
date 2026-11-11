//! #6993: jq's `range/3` compares its step with a literal `0` exactly. yq's numbers
//! are doubles, so under `--jq-extensions` a step whose double is `0.0` is still zero there, as
//! `succinctly yq` read it before #6993. (yq's `flatten` takes an integer depth only, so a depth
//! literal too small for a double never reaches its comparison; the row below is a control.)

use anyhow::Result;

use crate::run_yq_stdin;

#[test]
fn yq_mode_reads_a_range_step_by_the_double_6993() -> Result<()> {
    for (filter, want) in [
        ("[limit(2; range(0; 1; 1e-1000))]", "[]\n"),
        ("[range(0; 1e-1000; 1)]", "[]\n"),
        ("[range(0; 3; 1)]", "[0,1,2]\n"),
        ("[[1, [2, [3]]]] | flatten(1)", "[1,[2,[3]]]\n"),
    ] {
        let (stdout, code) =
            run_yq_stdin(filter, "null\n", &["--jq-extensions", "-o=json", "-I=0"])?;
        assert_eq!((stdout.as_str(), code), (want, 0), "{filter}");
    }
    Ok(())
}
