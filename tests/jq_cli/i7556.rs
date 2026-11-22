//! #7556, recorded in `docs/compliance/jq/limitations.md` ("A field lookup reads the members of
//! its own object, not the values it steps over"): a lookup of a key the object does not hold
//! answers as on a well-formed object when a `,` or `:` between two of its members is missing
//! or doubled (`{"z":1 "y":2} | .a` is `null`, exit 0), where jq 1.7.1 and 1.8.2 refuse the
//! document. Checking every member's gaps on that path measured +13-16% instructions on
//! well-formed input at best (+41-63% for a second key-only walk), so the lookup checks the
//! gaps of the member it returns and of the last member only, as #2288 decided for a lookup that
//! finds its key.
//!
//! `has("a")`, which resolves every key, raises jq's parse error on the same objects, and the
//! rows below pin that too. jq's answer for every lookup row is the `has` row's stderr and exit
//! 5, captured from `/usr/bin/jq` (1.8.2 agrees).

use anyhow::Result;

use crate::run_jq_full;

/// A faulty object, and jq's parse error for it at the top level.
const OBJECTS: &[(&str, &str)] = &[
    (
        r#"{"z":1,"y" 2}"#,
        "Expected separator between values at line 1, column 13",
    ),
    (
        r#"{"z":1 "y":2}"#,
        "Expected separator between values at line 1, column 10",
    ),
    (
        r#"{"z":1,,"y":2}"#,
        "Expected value before ',' at line 1, column 8",
    ),
    (
        r#"{"z":1,"y"::2}"#,
        "Expected string key before ':' at line 1, column 12",
    ),
    (
        r#"{"z" 1,"y":2}"#,
        "Expected separator between values at line 1, column 7",
    ),
    (
        r#"{"z":1,"y":2 "x":3}"#,
        "Expected separator between values at line 1, column 16",
    ),
    (
        r#"{,"z":1}"#,
        "Expected value before ',' at line 1, column 2",
    ),
    (
        r#"{"z":[1,{"q":2}] "y":2}"#,
        "Expected separator between values at line 1, column 20",
    ),
    (
        r#"{"z":{"q":[]},"y" 2}"#,
        "Expected separator between values at line 1, column 20",
    ),
];

/// A lookup of a key the object does not hold, and what it answers (as on `{}`).
const ABSENT_READERS: &[(&str, &str)] = &[
    (".a", "null"),
    (r#".["a"]"#, "null"),
    (".a?", "null"),
    (r#"getpath(["a"])"#, "null"),
    (r#".a // "d""#, r#""d""#),
    ("{a}", r#"{"a":null}"#),
    ("[.a, .b]", "[null,null]"),
];

#[test]
fn test_an_absent_key_steps_over_a_members_gap_fault_as_recorded_7556() -> Result<()> {
    let mut failures = Vec::new();
    for &(doc, error) in OBJECTS {
        for &(reader, answer) in ABSENT_READERS {
            let got = run_jq_full(&["-c", reader], Some(doc))?;
            let want = (format!("{answer}\n"), String::new(), 0);
            if got != want {
                failures.push(format!("{doc} | {reader}: {got:?}, want {want:?}"));
            }
        }
        // `has` resolves every key, and raises jq's error.
        let got = run_jq_full(&["-c", r#"has("a")"#], Some(doc))?;
        let want = (String::new(), format!("jq: parse error: {error}\n"), 5);
        if got != want {
            failures.push(format!("{doc} | has(\"a\"): {got:?}, want {want:?}"));
        }
    }
    assert!(
        failures.is_empty(),
        "{} rows moved:\n{}",
        failures.len(),
        failures.join("\n")
    );
    Ok(())
}

/// A wide object's absent lookups are answered from its key index (#3913) once a lookup has
/// proved it wide, with the same answer as the walk: `null`, whatever its members' gaps hold.
#[test]
fn test_an_absent_key_on_a_wide_object_with_a_gap_fault_is_null_7556() -> Result<()> {
    let mut doc = String::from("{");
    for i in 0..200 {
        if i > 0 {
            doc.push_str(if i == 100 { " " } else { "," });
        }
        doc.push_str(&format!("\"k{i}\":{i}"));
    }
    doc.push('}');
    // jq: `jq: parse error: Expected separator between values at line 1, column <the k100 key's
    // closing quote>`, exit 5, for every filter here.
    let got = run_jq_full(&["-c", ".k0, .k1, .a"], Some(&doc))?;
    assert_eq!(got, ("0\n1\nnull\n".to_string(), String::new(), 0));
    let got = run_jq_full(&["-c", ".a"], Some(&doc))?;
    assert_eq!(got, ("null\n".to_string(), String::new(), 0));
    Ok(())
}
