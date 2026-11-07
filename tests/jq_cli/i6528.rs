//! #6528: a `reduce`/`foreach` step that builds the next state with a constructor reading the old
//! one, with no `.` member (`[.[0] + $x, .[1] + 1]`, `{s: (.s + $x), n: (.n + 1)}`), is built
//! from members settled against the state, floats included, instead of taking the evaluator's
//! route and re-indexing the state for each read. Every expected string was captured from jq 1.7.1
//! (`/usr/bin/jq`).

use anyhow::Result;

use crate::run_jq_full;

/// Floats in every spelling the input can carry: plain, `1E3`, `1.50`, a literal past a double.
const INPUT: &str = "[1.5, 2.25, 3, -0.5, 1E3, 1.50, 100000000000000000001, 0.1, 0.2]";

#[test]
fn constructor_fold_steps_answer_as_jq_6528() -> Result<()> {
    for (filter, want, want_code) in [
        (r"reduce .[] as $x ([0, 0]; [(.[0] + $x), (.[1] + 1)]) | .[0] / .[1]", "11111111111111110000\n", 0),
        (r"reduce .[] as $x ([0, 0]; [.[0] + $x, .[1] + 1])", "[1e+20,9]\n", 0),
        (r"reduce .[] as $x ({s: 0, n: 0}; {s: (.s + $x), n: (.n + 1)})", "{\"s\":1e+20,\"n\":9}\n", 0),
        (r"reduce .[] as $x ([0,0]; [.[0] + $x, .[1]])", "[1e+20,0]\n", 0),
        (r"reduce .[] as $x ([null,null]; [($x), .[0]])", "[0.2,0.1]\n", 0),
        (r"reduce .[] as $x (null; [$x, .[0]])", "[0.2,0.1]\n", 0),
        (r"reduce .[] as $x ([1,1]; [.[1], .[0] + .[1]])", "[55,89]\n", 0),
        (r"reduce .[] as $x ([0,0,1,1]; [.[0] + 1, .[1] - $x, .[2] * $x, .[3] / 2])", "[9,-1e+20,-15187500000000002000000,0.001953125]\n", 0),
        (r"reduce .[] as $x ({}; {a: .b, b: .a})", "{\"a\":null,\"b\":null}\n", 0),
        (r"reduce .[] as $x ({a: 0}; {a: (.a + 1), a: (.a + 10)})", "{\"a\":90}\n", 0),
        (r"reduce .[] as $x ({a: 0}; {(.a|tostring): 1, a: (.a + 1)})", "{\"8\":1,\"a\":9}\n", 0),
        (r"reduce .[] as $x ([0.1]; [.[0] + 0.2])", "[1.8999999999999997]\n", 0),
        (r"reduce .[] as $x ([1.50]; [.[0]])", "[1.50]\n", 0),
        (r"reduce .[] as $x ([1.50]; [.[0], .[0] + 0])", "[1.50,1.5]\n", 0),
        (r"reduce .[] as $x ([9007199254740992]; [.[0] + 1])", "[9007199254740992]\n", 0),
        (r"reduce .[] as $x ([1e300]; [.[0] * 1e300])", "[1.7976931348623157e+308]\n", 0),
        (r"reduce .[] as $x ([1]; [.[0] / 0])", "", 5),
        (r"reduce .[] as $x ([0]; [.[0] + 1, empty])", "[9]\n", 0),
        (r"reduce .[] as $x ([0]; [(.[0], 5)])", "[0,5]\n", 0),
        (r#"try reduce .[] as $x ([0]; [.[0] + "a"]) catch ."#, "\"number (0) and string (\\\"a\\\") cannot be added\"\n", 0),
        (r"reduce .[] as $x ([[1]]; [.[0] + [2]])", "[[1,2,2,2,2,2,2,2,2,2]]\n", 0),
        (r"reduce .[] as $x ({}; {})", "{}\n", 0),
        (r"reduce .[] as $x ([0, 0]; if .[0] > 2 then [.[0], .[1] + 1] else [.[0] + 1, .[1]] end)", "[3,6]\n", 0),
        (r"[foreach .[] as $x ([0, 0]; [.[0] + $x, .[1] + 1]; .[0] / .[1])]", "[1.5,1.875,2.25,1.5625,201.25,167.95833333333334,14285714285714287000,1.25e+19,11111111111111110000]\n", 0),
        (r"[foreach .[] as $x ({s: 0}; {s: (.s + $x)}; .s)]", "[1.5,3.75,6.75,6.25,1006.25,1007.75,1e+20,1e+20,1e+20]\n", 0),
        (r"[limit(3; foreach .[] as $x ([0]; [.[0] + 1]))]", "[[1],[2],[3]]\n", 0),
    ] {
        let (stdout, _, code) = run_jq_full(&["-c", filter], Some(INPUT))?;
        assert_eq!((stdout.as_str(), code), (want, want_code), "{filter}");
    }
    Ok(())
}
