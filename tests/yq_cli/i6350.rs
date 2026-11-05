//! #6350 in yq mode: `select(cond)` answers a `cond` in the direct grammar from the node. yq
//! compares a scalar with a literal by its text (`e: "null"` is `== null`), and that rule is the
//! one `try_compare_op` applies on both routes. `COND | .` is the same condition outside the
//! direct grammar, so it takes the general route, and the two must keep the same nodes.

use anyhow::Result;

use crate::run_yq_stdin_with_stderr;

const DOC: &str = "a: null\nb: [~, true, \"true\", false, 0, \"0\", 0x1F, 1.0, 1e3, a, \"\", \"null\", yes, !!bool yes, !!str 1, [], {}, [null], {k: null}]\nc: &x {d: null, e: x}\nf: *x\ng: !!null x\nh: !!bool\n";

const CONDS: &[&str] = &[
    ". == null",
    ". != null",
    "null == .",
    ". == true",
    ". != false",
    ". == 0",
    ". == \"a\"",
    ". == \"\"",
    ". == \"null\"",
    ". == \"true\"",
    ". == 1.0",
    ". == 1",
    "tag == \"!!str\"",
    "type == \"!!str\"",
    "type != \"!!map\"",
    "\"!!null\" == type",
    "type == \"!!bool\" and . == true",
    ". == null or . == false",
    ". != null and . != \"\"",
    "not",
    ".",
    "true",
    "false",
    "null",
];

#[test]
fn test_select_agrees_with_its_general_route_in_yq_mode_6350() -> Result<()> {
    for cond in CONDS {
        for shape in [
            "[.. | select(COND)]",
            "[.. | select(COND)] | length",
            "[.[] | select(COND)]",
            "map(select(COND))",
            "[.. | select(COND) | tag]",
            "del(.. | select(COND))",
            "(.. | select(COND)) = \"X\"",
        ] {
            let direct = shape.replace("COND", cond);
            let general = shape.replace("COND", &format!("{cond} | ."));
            let args = ["-o=json", "-I=0"];
            let (a, ea, ca) = run_yq_stdin_with_stderr(&direct, DOC, &args)?;
            let (b, eb, cb) = run_yq_stdin_with_stderr(&general, DOC, &args)?;
            assert_eq!((ca, &a), (cb, &b), "{direct}: {ea} / {eb}");
        }
    }
    Ok(())
}
