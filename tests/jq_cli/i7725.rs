//! #7725: negating a number literal of 2^63 (`9223372036854775808`) keeps a value that agrees with
//! its own text.
//!
//! #7453 made a negated literal a literal, as jq 1.8.2 does, and negated a `Float` repr directly on
//! the premise that the negated text parses to the negated repr. That fails at one magnitude: 2^63
//! is a `Float`, while its negation `-9223372036854775808` is spelled as `i64::MIN`, an `Int`. The
//! release build answered as jq 1.8.2 does; a debug build (which this test binary drives) panicked
//! on the #3069 bridge-provenance check and on `Literal::NumberLiteral`'s fresh-parse check. Every
//! row is captured from jq 1.8.2 (`/opt/homebrew/bin/jq`), the reference for negated literals since
//! #7453; jq 1.7.1 turns the negation into a double and prints `-9223372036854776000` throughout.

use anyhow::Result;

use crate::run_jq_stdin_streams;

/// The value read, negated, printed, compared, used as a key, and handed to a math builtin.
const PROGRAM: &str = "[([-.] | tojson), (-. | tostring), (-(-.) | tojson), \
     ([-., -9223372036854775808] | unique), (-. == -9223372036854775808), \
     ({(-. | tostring): 1} | keys), (scalb(nan; -.) | isnan), ([-., 0] | sort)]";

/// `(number text, stdout)`. `9223372036854775809` rounds to the same double as 2^63 and its
/// negation stays a `Float`; `-9223372036854775808` is the other direction, `i64::MIN`.
const VALUES: &[(&str, &str)] = &[
    (
        "9223372036854775808",
        "[\"[-9223372036854775808]\",\"-9223372036854775808\",\"9223372036854775808\",\
         [-9223372036854775808],true,[\"-9223372036854775808\"],true,[-9223372036854775808,0]]\n",
    ),
    (
        "9223372036854775809",
        "[\"[-9223372036854775809]\",\"-9223372036854775809\",\"9223372036854775809\",\
         [-9223372036854775809,-9223372036854775808],false,[\"-9223372036854775809\"],true,\
         [-9223372036854775809,0]]\n",
    ),
    (
        "-9223372036854775808",
        "[\"[9223372036854775808]\",\"9223372036854775808\",\"-9223372036854775808\",\
         [-9223372036854775808,9223372036854775808],false,[\"9223372036854775808\"],true,\
         [0,9223372036854775808]]\n",
    ),
];

#[test]
fn test_7725_negated_2_pow_63_agrees_with_its_text_on_every_route() -> Result<()> {
    for (text, want) in VALUES {
        let argjson = format!("$x | {PROGRAM}");
        let from_text = format!("$s | tonumber | {PROGRAM}");
        let from_json = format!("$s | fromjson | {PROGRAM}");
        let positional = format!("$ARGS.positional[0] | {PROGRAM}");
        let literal = format!("({text}) | {PROGRAM}");
        // (filter, stdin, args before the filter, args after it)
        let routes: [(&str, &str, Vec<&str>, Vec<&str>); 6] = [
            (PROGRAM, text, vec!["-c"], vec![]),
            (&argjson, "", vec!["-nc", "--argjson", "x", text], vec![]),
            (&from_text, "", vec!["-nc", "--arg", "s", text], vec![]),
            (&from_json, "", vec!["-nc", "--arg", "s", text], vec![]),
            (&positional, "", vec!["-nc"], vec!["--jsonargs", text]),
            (&literal, "", vec!["-nc"], vec![]),
        ];
        for (filter, input, before, after) in routes {
            let mut args = before.clone();
            args.push(filter);
            args.extend_from_slice(&after);
            let (out, err, code) = crate::run_jq_full(&args, Some(input))?;
            assert_eq!(
                (out.as_str(), err.as_str(), code),
                (*want, "", 0),
                "{filter} on {text} ({before:?} {after:?})"
            );
        }
    }
    Ok(())
}

/// The finding's two programs, as written.
#[test]
fn test_7725_finding_rows() -> Result<()> {
    for (filter, want) in [
        ("[-.] | tojson", "\"[-9223372036854775808]\"\n"),
        ("scalb(nan; -.)", "null\n"),
    ] {
        let (out, err, code) = run_jq_stdin_streams(filter, "9223372036854775808", &["-c"])?;
        assert_eq!(
            (out.as_str(), err.as_str(), code),
            (want, "", 0),
            "{filter}"
        );
    }
    Ok(())
}
