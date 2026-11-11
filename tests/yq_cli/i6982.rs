//! #6982: when the left operand of `-`, `*` (every merge-flag form), `/` or `%` produces nothing,
//! yq never evaluates the right one, so an error there is silent. `+`, the comparisons, `and`, `or`
//! and `//` still evaluate it, as yq does. Both evaluators: a plain operand takes the cursor route,
//! a `parent` in the right operand the path-context route.
//!
//! Every row is captured from yq v4.53.3 with `-o=json -I=0`.

use anyhow::Result;

use crate::run_yq_stdin_with_stderr;

/// `(filter, document)`: nothing printed, exit 0.
const SILENT: &[(&str, &str)] = &[
    (".[] - error(\"x\")", "[]"),
    (".[] * error(\"x\")", "[]"),
    (".[] / error(\"x\")", "[]"),
    (".[] % error(\"x\")", "[]"),
    (".[] *+ error(\"x\")", "[]"),
    (".[] *d error(\"x\")", "[]"),
    (".[] *? error(\"x\")", "[]"),
    (".[] *n error(\"x\")", "[]"),
    (".[] *c error(\"x\")", "[]"),
    (".[]? * (.[-1])", "[]"),
    (".[] * (parent | error(\"x\"))", "[]"),
    ("(.[] | select(false)) - error(\"x\")", "{\"a\":1}"),
    (".zz * error(\"x\")", "{\"a\":1}"),
    (".[] * 2", "[]"),
    ("2 * .[]", "[]"),
];

/// `(filter, document)`: the right operand still runs, so `error("x")` raises.
const RAISED: &[(&str, &str)] = &[
    (".[] + error(\"x\")", "[]"),
    (".[] == error(\"x\")", "[]"),
    (".[] < error(\"x\")", "[]"),
    (".[] or error(\"x\")", "[]"),
    (".[] // error(\"x\")", "[]"),
    ("error(\"x\") * .[]", "[]"),
    (".a * error(\"x\")", "{\"a\":1}"),
];

#[test]
fn yq_empty_left_operand_skips_the_right_one_6982() -> Result<()> {
    for &(filter, doc) in SILENT {
        let (stdout, stderr, code) = run_yq_stdin_with_stderr(filter, doc, &["-o=json", "-I=0"])?;
        assert_eq!(
            (stdout.as_str(), stderr.as_str(), code),
            ("", "", 0),
            "{filter} on {doc}"
        );
    }
    for &(filter, doc) in RAISED {
        let (stdout, stderr, code) = run_yq_stdin_with_stderr(filter, doc, &["-o=json", "-I=0"])?;
        assert_eq!((stdout.as_str(), code), ("", 1), "{filter} on {doc}");
        assert_eq!(stderr.trim_end(), "Error: x", "{filter} on {doc}");
    }
    // An empty left operand of `and` is one `false`, as before.
    let (stdout, _, code) = run_yq_stdin_with_stderr(".[] and error(\"x\")", "[]", &["-o=json"])?;
    assert_eq!((stdout.as_str(), code), ("false\n", 0));
    Ok(())
}
