//! #5450: a navigation in front of `map(key)` or `[.[] | key]` keeps each key's quoting, `~`
//! spelling and anchor, as the listing does at the root. Every output is captured from yq v4.53.3.

use anyhow::Result;

use crate::run_yq_stdin;

const D: &str =
    "a:\n  \"k\": 1\n  ~: 2\n  &z m: 3\n  3: 4\nx:\n  y:\n    \"q\": 1\n    'r': 2\nc: s\nd: {}\n";

fn check(rows: &[(&str, &str)]) -> Result<()> {
    for (filter, want) in rows {
        let (stdout, code) = run_yq_stdin(filter, D, &[])?;
        assert_eq!((stdout.as_str(), code), (*want, 0), "`{filter}`");
    }
    Ok(())
}

#[test]
fn test_map_key_behind_a_navigation_keeps_each_key_5450() -> Result<()> {
    let a = "- \"k\"\n- ~\n- &z m\n- 3\n";
    let y = "- \"q\"\n- 'r'\n";
    check(&[
        (".a | map(key)", a),
        (".a | (map(key))", a),
        (".x.y | map(key)", y),
        (".x | .y | map(key)", y),
        // A later stage reads the keys the listing made.
        (".x.y | map(key) | .[1]", "r\n"),
        (".x.y | map(key) | length", "2\n"),
    ])
}

#[test]
fn test_key_array_behind_a_navigation_keeps_each_key_5450() -> Result<()> {
    let a = "- \"k\"\n- ~\n- &z m\n- 3\n";
    let y = "- \"q\"\n- 'r'\n";
    check(&[
        (".a | [.[] | key]", a),
        (".x.y | [.[] | key]", y),
        (".x | .y | [.[] | key]", y),
        ("(.x.y | [.[] | key])", y),
        (".x.y | [.[] | key] | .[0]", "q\n"),
    ])
}

#[test]
fn test_key_listing_behind_a_navigation_on_a_non_mapping_is_unchanged_5450() -> Result<()> {
    // yq: an empty mapping is itself, a scalar passes through `map(key)` and lists nothing in
    // `[.[] | key]`, and a missing key lists nothing.
    check(&[
        (".d | map(key)", "{}\n"),
        (".c | map(key)", "s\n"),
        (".d | [.[] | key]", "[]\n"),
        (".zz | [.[] | key]", "[]\n"),
        (".zz | map(key)", "[]\n"),
    ])?;
    check(&[(".c | [.[] | key]", "[]\n")])
}

#[test]
fn test_key_listing_behind_a_navigation_in_json_and_pretty_output_5450() -> Result<()> {
    for filter in [".a | map(key)", ".a | [.[] | key]"] {
        let (out, code) = run_yq_stdin(filter, D, &["-o=json", "-I=0"])?;
        assert_eq!(
            (out.as_str(), code),
            ("[\"k\",null,\"m\",3]\n", 0),
            "{filter}"
        );
        let (out, code) = run_yq_stdin(filter, D, &["-P"])?;
        assert_eq!(
            (out.as_str(), code),
            ("- k\n- ~\n- &z m\n- 3\n", 0),
            "{filter}"
        );
    }
    Ok(())
}
