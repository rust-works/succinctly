//! #7468: a negative zero spelled with extra zeros (`-00`, `-000`) stringifies as `-0`, as jq prints it
//! (1.7.1 and 1.8.2 agree), through every consumer that turns a number into text: `tostring`, `@text`,
//! `@csv`, `@tsv`, `@sh`, `@base64`, `@uri`, `@html`, string interpolation and `join`. It came out as `0`
//! there, though `.` and `tojson` already kept the sign. Every expectation below was captured from
//! jq 1.7.1 and 1.8.2, which print the same text for each row.

use anyhow::Result;

use crate::run_jq_full;

/// A program over the number `.` and the text jq prints for it when that number is `-0`.
const ROWS: [(&str, &str); 17] = [
    (".", "-0"),
    ("tojson", "\"-0\""),
    ("tostring", "\"-0\""),
    ("@text", "\"-0\""),
    ("[.]|@csv", "\"-0\""),
    ("[.]|@tsv", "\"-0\""),
    ("@sh", "\"-0\""),
    ("[.]|@sh", "\"-0\""),
    ("@base64", "\"LTA=\""),
    ("@uri", "\"-0\""),
    ("@html", "\"-0\""),
    ("tostring|length", "2"),
    ("\"x\\(.)\"", "\"x-0\""),
    ("[.]|join(\",\")", "\"-0\""),
    ("[., 1]|map(tostring)", "[\"-0\",\"1\"]"),
    ("[.]|tostring", "\"[-0]\""),
    ("{a:.}|tostring", "\"{\\\"a\\\":-0}\""),
];

const SPELLINGS: [&str; 4] = ["-0", "-00", "-000", "-0000"];

#[test]
fn a_negative_zero_with_extra_zeros_stringifies_with_its_sign_7468() -> Result<()> {
    for spelling in SPELLINGS {
        // As the whole document, as an element and as a member: the number is reached by
        // the same program through `.`, `.[0]` and `.a`.
        let contexts = [
            ("", spelling.to_string()),
            (".[0] | ", format!("[{spelling}]")),
            (".a | ", format!("{{\"a\":{spelling}}}")),
        ];
        for (path, input) in &contexts {
            for (program, want) in ROWS {
                let filter = format!("{path}{program}");
                let (out, err, status) = run_jq_full(&["-c", &filter], Some(input))?;
                assert_eq!(
                    (out.as_str(), err.as_str(), status),
                    (format!("{want}\n").as_str(), "", 0),
                    "{filter} over {input}"
                );
            }
        }
    }
    Ok(())
}

#[test]
fn a_negative_zero_with_extra_zeros_beside_other_numbers_7468() -> Result<()> {
    // The sign belongs to the spelling, so a neighbour must not lend or take it.
    let (out, err, status) = run_jq_full(
        &["-c", "map(tostring)"],
        Some("[-00, 00, -01, 0, -0, -000.0, -00e0]"),
    )?;
    assert_eq!(
        (out.as_str(), err.as_str(), status),
        (
            "[\"-0\",\"0\",\"-1\",\"0\",\"-0\",\"-0.0\",\"-0\"]\n",
            "",
            0
        )
    );
    Ok(())
}
