//! #5489: `fromstream` emits a value it has completed before it reaches a malformed later event,
//! then raises, as jq's lazy `foreach` does; `try`, `?`, `first` and `limit` see the value too.
//! Every row was captured from jq 1.7.1 (`/usr/bin/jq -n -c`): its stdout and exit status.

use anyhow::Result;

use crate::run_jq_full;

#[test]
fn test_fromstream_emits_a_completed_value_before_a_malformed_event_5489() -> Result<()> {
    for (filter, want_stdout, want_code) in [
        ("fromstream([[0],1],[[0]],[\"x\",[1]])", "[1]\n", 5),
        (
            "try fromstream([[0],1],[[0]],[\"x\",[1]]) catch \"caught\"",
            "[1]\n\"caught\"\n",
            0,
        ),
        ("fromstream([[0],1],[[0]],[\"x\",[1]])?", "[1]\n", 0),
        ("[fromstream([[0],1],[[0]],[\"x\",[1]])?]", "[[1]]\n", 0),
        (
            "fromstream(([[0],1],[[0]],[[0],2],[[0]],[\"x\",[1]]))",
            "[1]\n[2]\n",
            5,
        ),
        ("first(fromstream([[0],1],[[0]],[\"x\",[1]]))", "[1]\n", 0),
        (
            "[limit(1; fromstream([[0],1],[[0]],[\"x\",[1]]))]",
            "[[1]]\n",
            0,
        ),
        (
            "isempty(fromstream([[0],1],[[0]],[\"x\",[1]]))",
            "false\n",
            0,
        ),
        ("fromstream([\"x\",[1]])", "", 5),
        (
            "try fromstream([\"x\",[1]]) catch \"caught\"",
            "\"caught\"\n",
            0,
        ),
        ("fromstream(([[0],1],[[0]],error(\"src\")))", "[1]\n", 5),
        (
            "try fromstream(([[0],1],[[0]],error(\"src\"))) catch .",
            "[1]\n\"src\"\n",
            0,
        ),
        (
            "[fromstream(([[0],1],[[0]],[[1],2],[[1]],[\"y\"]))?]",
            "[[1],[null,2],null]\n",
            0,
        ),
        ("[fromstream(tostream)]", "[null]\n", 0),
    ] {
        let (stdout, _stderr, code) = run_jq_full(&["-n", "-c", filter], None)?;
        assert_eq!(
            (stdout.as_str(), code),
            (want_stdout, want_code),
            "{filter}"
        );
    }
    Ok(())
}
