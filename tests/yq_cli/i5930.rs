//! #5930: a timestamp on either side of `*`, `/` or `%` is refused as yq refuses it (`cannot multiply
//! !!timestamp with !!int`, `!!timestamp cannot be divided by !!int`, `!!int cannot modulo by !!timestamp`),
//! where `.t * 2` repeated the text and exited 0; a timestamp times `null` is the timestamp. `from_unix`
//! names the tag of a non-number (`from_unix only works on numbers, found !!str instead`). Every row was
//! captured from yq v4.53.3; a container or `null` operand is left out, since yq writes the operands' keys
//! after their tags there and the evaluator does not know them.

use anyhow::Result;

use crate::run_yq_stdin_with_stderr;

const DOC: &str = "t: 2026-01-01T00:00:00Z\nq: \"2026-01-01T00:00:00Z\"\nd: 2026-01-01\ns: x\nn: 3\nf: 1.5\nb: true\nz: ~\nm: {a: 1}\nl: [1]\n";

#[test]
fn a_timestamp_in_multiplication_division_and_modulo_answers_as_yq_does_5930() -> Result<()> {
    let rows: &[(&str, bool, &str)] = &[
        (".t * 2", false, "cannot multiply !!timestamp with !!int"),
        (
            ".t * 1.5",
            false,
            "cannot multiply !!timestamp with !!float",
        ),
        (
            ".t * \"x\"",
            false,
            "cannot multiply !!timestamp with !!str",
        ),
        (
            ".t * true",
            false,
            "cannot multiply !!timestamp with !!bool",
        ),
        (".t * null", true, "\"2026-01-01T00:00:00Z\""),
        (".t * .s", false, "cannot multiply !!timestamp with !!str"),
        (".t * .n", false, "cannot multiply !!timestamp with !!int"),
        (".t * .f", false, "cannot multiply !!timestamp with !!float"),
        (".t * .b", false, "cannot multiply !!timestamp with !!bool"),
        (".t * .z", true, "\"2026-01-01T00:00:00Z\""),
        (
            ".t * .t",
            false,
            "cannot multiply !!timestamp with !!timestamp",
        ),
        ("2 * .t", false, "cannot multiply !!int with !!timestamp"),
        (
            "1.5 * .t",
            false,
            "cannot multiply !!float with !!timestamp",
        ),
        (
            "\"x\" * .t",
            false,
            "cannot multiply !!str with !!timestamp",
        ),
        (
            "true * .t",
            false,
            "cannot multiply !!bool with !!timestamp",
        ),
        (".s * .t", false, "cannot multiply !!str with !!timestamp"),
        (".n * .t", false, "cannot multiply !!int with !!timestamp"),
        (".f * .t", false, "cannot multiply !!float with !!timestamp"),
        (".b * .t", false, "cannot multiply !!bool with !!timestamp"),
        (".d * 2", false, "cannot multiply !!timestamp with !!int"),
        (
            ".d * .t",
            false,
            "cannot multiply !!timestamp with !!timestamp",
        ),
        (".t / 2", false, "!!timestamp cannot be divided by !!int"),
        (
            ".t / 1.5",
            false,
            "!!timestamp cannot be divided by !!float",
        ),
        (
            ".t / \"x\"",
            false,
            "!!timestamp cannot be divided by !!str",
        ),
        (
            ".t / true",
            false,
            "!!timestamp cannot be divided by !!bool",
        ),
        (
            ".t / null",
            false,
            "!!timestamp cannot be divided by !!null",
        ),
        (".t / .s", false, "!!timestamp cannot be divided by !!str"),
        (".t / .n", false, "!!timestamp cannot be divided by !!int"),
        (".t / .f", false, "!!timestamp cannot be divided by !!float"),
        (".t / .b", false, "!!timestamp cannot be divided by !!bool"),
        (".t / .z", false, "!!timestamp cannot be divided by !!null"),
        (
            ".t / .t",
            false,
            "!!timestamp cannot be divided by !!timestamp",
        ),
        ("2 / .t", false, "!!int cannot be divided by !!timestamp"),
        (
            "1.5 / .t",
            false,
            "!!float cannot be divided by !!timestamp",
        ),
        (
            "\"x\" / .t",
            false,
            "!!str cannot be divided by !!timestamp",
        ),
        (
            "true / .t",
            false,
            "!!bool cannot be divided by !!timestamp",
        ),
        (".s / .t", false, "!!str cannot be divided by !!timestamp"),
        (".n / .t", false, "!!int cannot be divided by !!timestamp"),
        (".f / .t", false, "!!float cannot be divided by !!timestamp"),
        (".b / .t", false, "!!bool cannot be divided by !!timestamp"),
        (".d / 2", false, "!!timestamp cannot be divided by !!int"),
        (
            ".d / .t",
            false,
            "!!timestamp cannot be divided by !!timestamp",
        ),
        (".t % 2", false, "!!timestamp cannot modulo by !!int"),
        (".t % 1.5", false, "!!timestamp cannot modulo by !!float"),
        (".t % \"x\"", false, "!!timestamp cannot modulo by !!str"),
        (".t % true", false, "!!timestamp cannot modulo by !!bool"),
        (".t % null", false, "!!timestamp cannot modulo by !!null"),
        (".t % .s", false, "!!timestamp cannot modulo by !!str"),
        (".t % .n", false, "!!timestamp cannot modulo by !!int"),
        (".t % .f", false, "!!timestamp cannot modulo by !!float"),
        (".t % .b", false, "!!timestamp cannot modulo by !!bool"),
        (".t % .z", false, "!!timestamp cannot modulo by !!null"),
        (".t % .t", false, "!!timestamp cannot modulo by !!timestamp"),
        ("2 % .t", false, "!!int cannot modulo by !!timestamp"),
        ("1.5 % .t", false, "!!float cannot modulo by !!timestamp"),
        ("\"x\" % .t", false, "!!str cannot modulo by !!timestamp"),
        ("true % .t", false, "!!bool cannot modulo by !!timestamp"),
        (".s % .t", false, "!!str cannot modulo by !!timestamp"),
        (".n % .t", false, "!!int cannot modulo by !!timestamp"),
        (".f % .t", false, "!!float cannot modulo by !!timestamp"),
        (".b % .t", false, "!!bool cannot modulo by !!timestamp"),
        (".d % 2", false, "!!timestamp cannot modulo by !!int"),
        (".d % .t", false, "!!timestamp cannot modulo by !!timestamp"),
        (
            ".t * {\"a\":1}",
            false,
            "cannot multiply !!timestamp with !!map",
        ),
        (".t * [1]", false, "cannot multiply !!timestamp with !!seq"),
        (".t * .m", false, "cannot multiply !!timestamp with !!map"),
        (".t * .l", false, "cannot multiply !!timestamp with !!seq"),
        (
            "{\"a\":1} * .t",
            false,
            "cannot multiply !!map with !!timestamp",
        ),
        ("[1] * .t", false, "cannot multiply !!seq with !!timestamp"),
        (".m * .t", false, "cannot multiply !!map with !!timestamp"),
        (".l * .t", false, "cannot multiply !!seq with !!timestamp"),
        (".z * .t", false, "cannot multiply !!null with !!timestamp"),
        (
            "null * .t",
            false,
            "cannot multiply !!null with !!timestamp",
        ),
        (
            "\"x\" | from_unix",
            false,
            "from_unix only works on numbers, found !!str instead",
        ),
        (
            ".s | from_unix",
            false,
            "from_unix only works on numbers, found !!str instead",
        ),
        (
            ".b | from_unix",
            false,
            "from_unix only works on numbers, found !!bool instead",
        ),
        (
            ".l | from_unix",
            false,
            "from_unix only works on numbers, found !!seq instead",
        ),
        (
            ".m | from_unix",
            false,
            "from_unix only works on numbers, found !!map instead",
        ),
        (
            ".z | from_unix",
            false,
            "from_unix only works on numbers, found !!null instead",
        ),
        (
            "null | from_unix",
            false,
            "from_unix only works on numbers, found !!null instead",
        ),
        (
            "[1] | from_unix",
            false,
            "from_unix only works on numbers, found !!seq instead",
        ),
        (
            "{} | from_unix",
            false,
            "from_unix only works on numbers, found !!map instead",
        ),
        (
            ".t | from_unix",
            false,
            "from_unix only works on numbers, found !!timestamp instead",
        ),
        (".n | from_unix", true, "\"1970-01-01T10:00:03+10:00\""),
        (".f | from_unix", true, "\"1970-01-01T10:00:01+10:00\""),
        (
            "\"1\" | from_unix",
            false,
            "from_unix only works on numbers, found !!str instead",
        ),
        (
            ".d | from_unix",
            false,
            "from_unix only works on numbers, found !!timestamp instead",
        ),
        (
            ".t * 2 // \"d\"",
            false,
            "cannot multiply !!timestamp with !!int",
        ),
        ("[.t * 2]", false, "cannot multiply !!timestamp with !!int"),
        (".t *= 2", false, "cannot multiply !!timestamp with !!int"),
        (
            ".t |= . * 2",
            false,
            "cannot multiply !!timestamp with !!int",
        ),
        (".t *? 2", false, "cannot multiply !!timestamp with !!int"),
        (".n * 2", true, "6"),
        (".s * 2", true, "\"xx\""),
        (".t * null", true, "\"2026-01-01T00:00:00Z\""),
        (".z * .t", false, "cannot multiply !!null with !!timestamp"),
    ];
    for &(filter, ok, expected) in rows {
        let (stdout, stderr, status) =
            run_yq_stdin_with_stderr(filter, DOC, &["-o", "json", "-I0"])?;
        if ok {
            assert_eq!((stdout.trim_end(), status), (expected, 0), "{filter}");
        } else {
            assert_ne!(status, 0, "{filter}");
            assert!(
                stderr.contains(&format!("Error: {expected}")),
                "{filter}: {stderr}"
            );
        }
    }
    Ok(())
}
