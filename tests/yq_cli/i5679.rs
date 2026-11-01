//! #5679: a long pipe hands each call a bounded piece of the stages after it in jq mode only. In yq
//! mode the rest is copied as before, and has to answer as yq does. Each expectation is a live yq
//! v4.53.3 capture (`yq -n -o=json -I0`).

use anyhow::Result;

use crate::run_yq_stdin;

fn pipe(stage: &str, stages: usize) -> String {
    vec![stage; stages].join(" | ")
}

#[test]
fn test_long_pipes_of_computed_stages_answer_as_yq_does_5679() -> Result<()> {
    for (filter, expected, status) in [
        (format!("0 | {}", pipe(". + 1", 300)), "300\n", 0),
        (format!("[1] | {}", pipe("map(.)", 200)), "[1]\n", 0),
        (
            format!("0 | {} | select(. > 1000)", pipe(". + 1", 150)),
            "",
            0,
        ),
    ] {
        let (stdout, code) = run_yq_stdin(&filter, "", &["-n", "-o=json", "-I0"])?;
        assert_eq!((stdout.as_str(), code), (expected, status), "{filter:.60}");
    }
    Ok(())
}
