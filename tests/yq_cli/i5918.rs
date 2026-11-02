//! #5918: yq's `map(f)` over an **empty mapping** is that mapping (`!!map`, `f` never runs), not an
//! empty sequence. A non-empty mapping, an empty or non-empty sequence, `null` and a scalar already
//! answered as yq does and are pinned here so the rule stays exactly "an empty mapping". Every
//! expected string is a live yq v4.53.3 capture (`-p=json -o=json -I0`).

use anyhow::Result;

use crate::run_yq_stdin;

/// (document, filter, expected stdout).
const ROWS: &[(&str, &str, &str)] = &[
    ("{}", "map(.)", "{}\n"),
    ("{}", "map(select(false))", "{}\n"),
    ("{}", "map(.) | tag", "\"!!map\"\n"),
    ("{}", "[map(.)]", "[{}]\n"),
    ("{}", "{\"x\": map(.)}", "{\"x\":{}}\n"),
    ("{}", "map(.) | . + {\"a\":1}", "{\"a\":1}\n"),
    ("{}", "map(1)", "{}\n"),
    ("{}", "map(.a)", "{}\n"),
    ("{}", "map(.) as $x | $x | kind", "\"map\"\n"),
    ("{}", "map(.) | length", "0\n"),
    ("{\"a\":{}}", ".a | map(.)", "{}\n"),
    ("{\"a\":{}}", ".[] | map(.)", "{}\n"),
    ("{\"a\":{},\"b\":[]}", ".[] | map(.)", "{}\n[]\n"),
    ("{\"a\":{}}", "map(.)", "[{}]\n"),
    ("{\"a\":1}", "map(select(false))", "[]\n"),
    ("{\"a\":1,\"b\":2}", "map(.)", "[1,2]\n"),
    ("[]", "map(.)", "[]\n"),
    ("[{}]", ".[] | map(.)", "{}\n"),
    ("null", "map(.)", "[]\n"),
    ("5", "map(error(\"x\"))", "5\n"),
    // A body that reads path context takes the positioned evaluator, a third site of the rule.
    ("{}", "map(key)", "{}\n"),
    ("{}", "map(parent)", "{}\n"),
    ("{}", "map(path)", "{}\n"),
    ("{}", "map(map(key))", "{}\n"),
    ("{}", "map(.) | map(key)", "{}\n"),
    ("{\"a\":{}}", "map(map(key))", "[{}]\n"),
    ("[{}]", "map(map(key))", "[{}]\n"),
    ("[{}]", "map(key)", "[0]\n"),
];

#[test]
fn test_map_over_an_empty_mapping_is_that_mapping_in_yq_5918() -> Result<()> {
    for (doc, filter, expected) in ROWS {
        let (stdout, code) = run_yq_stdin(filter, doc, &["-p=json", "-o=json", "-I0"])?;
        assert_eq!((stdout.as_str(), code), (*expected, 0), "{doc} | {filter}");
    }
    Ok(())
}

/// `-n` builds the mapping in the program, which the owned evaluator reads: same answer.
#[test]
fn test_map_over_an_empty_mapping_built_by_the_program_5918() -> Result<()> {
    for (filter, expected) in [
        ("{} | map(.) | tag", "!!map\n"),
        ("{} | map(select(false))", "{}\n"),
        ("[{}] | .[0] | map(.) | kind", "map\n"),
        ("{\"a\": 1} | map(select(false)) | tag", "!!seq\n"),
    ] {
        let (stdout, code) = run_yq_stdin(filter, "", &["-n"])?;
        assert_eq!((stdout.as_str(), code), (expected, 0), "{filter}");
    }
    Ok(())
}
