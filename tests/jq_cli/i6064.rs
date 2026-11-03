//! #6064: `range(FROM; TO)` emits `FROM` itself first, so an integer past 2^53 keeps its digits in jq
//! 1.7.1 -- also when `TO` is a float (`.+10`, `infinite`), where the range runs on doubles. The float
//! route used to start from the rounded double. Every row is what `/usr/bin/jq` 1.7.1 printed for the
//! same program over the same input.

use anyhow::Result;

use crate::run_jq_stdin;

fn out(program: &str, input: &str) -> Result<String> {
    let (stdout, code) = run_jq_stdin(program, input, &["-c"])?;
    assert_eq!(code, 0, "`{program}` over {input} failed");
    Ok(stdout.trim().to_string())
}

#[test]
fn the_first_value_of_a_float_bounded_range_keeps_an_integer_past_2_53() -> Result<()> {
    let rows: &[(&str, &str, &str)] = &[
        (
            "[limit(1;range(.;.+10))]",
            "9007199254740993",
            "[9007199254740993]",
        ),
        (
            "[limit(1;range(.;.+10))]",
            "-9007199254740993",
            "[-9007199254740993]",
        ),
        (
            "[limit(1;range(.;.+10;1))]",
            "9007199254740993",
            "[9007199254740993]",
        ),
        (
            "[limit(1;range(.;.+10))]",
            "18014398509481985",
            "[18014398509481985]",
        ),
        ("range(.;.+1)", "-9007199254740993", "-9007199254740993"),
        (
            ". as $x | [limit(1;range($x;$x+10))]",
            "9007199254740993",
            "[9007199254740993]",
        ),
        (
            "[limit(2;range(.;.-10;-1))]",
            "9007199254740993",
            "[9007199254740993,9007199254740991]",
        ),
        (
            "[limit(3;range(.;infinite))]",
            "9223372036854775806",
            "[9223372036854775806,9223372036854776000,9223372036854776000]",
        ),
        // Neighbours that must not change: an exact-integer bound, a start the double holds, and
        // the ordinary small ranges.
        (
            "[limit(1;range(.;10000000000000000))]",
            "9007199254740993",
            "[9007199254740993]",
        ),
        (
            "[limit(1;range(.;.+10))]",
            "9007199254740994",
            "[9007199254740994]",
        ),
        (
            "[limit(1;range(.[0];.[1]))]",
            "[9007199254740993,9007199254741000]",
            "[9007199254740993]",
        ),
        // Program text takes the evaluator's other route; the boundary is 2^53.
        (
            "[limit(1;range(9007199254740993;9007199254741000.5))]",
            "null",
            "[9007199254740993]",
        ),
        (
            "[limit(1;range(9007199254740992;9007199254741000.5))]",
            "null",
            "[9007199254740992]",
        ),
        (
            "first(range(9007199254740993;1e17))",
            "null",
            "9007199254740993",
        ),
        (
            "[limit(1;range(.;.+10))]",
            "9007199254740992",
            "[9007199254740992]",
        ),
        (
            "[limit(1;range(.;.+10))]",
            "9007199254740991",
            "[9007199254740991]",
        ),
        (
            "[limit(1;range(.;.+10))]",
            "-9007199254740992",
            "[-9007199254740992]",
        ),
        ("[range(.;.+3)]", "5", "[5,6,7]"),
        ("[range(0;.)]", "5", "[0,1,2,3,4]"),
    ];
    for (program, input, want) in rows {
        assert_eq!(out(program, input)?, *want, "`{program}` over {input}");
    }
    Ok(())
}
