//! #5460: a bracketed number key, an object key or an array key read off a `+` or `*` result is
//! nothing in yq when the mapping lacks it, as a string key already was (#4361). `succinctly yq`
//! answered `null` for a number (the parser folded `[0]` into an array index) and failed with
//! `Cannot index object with object` for a container key. Every row is a live yq v4.53.3 capture
//! over the document below, `-o=json -I=0`. An array target, a key the mapping holds, and a write
//! through such a head (a no-op in yq) read as before.
//!
//! Not matched: `del((A + B)[.k])` with a computed key fails with `Invalid path expression`
//! where yq leaves the document alone, as it did before this change.

use anyhow::Result;

use crate::run_yq_stdin;

const DOC: &str = "a: {x: 1}\nl: [1, 2]\nn: null\ns: str\nk: x\ni: 0\n";
const DOC_JSON: &str =
    "{\"a\":{\"x\":1},\"l\":[1,2],\"n\":null,\"s\":\"str\",\"k\":\"x\",\"i\":0}\n";
const JSON: &[&str] = &["-o=json", "-I=0"];

fn check(rows: &[(&str, &str)]) -> Result<()> {
    for (filter, expected) in rows {
        let (stdout, code) = run_yq_stdin(filter, DOC, JSON)?;
        assert_eq!((stdout.as_str(), code), (*expected, 0), "{filter}");
    }
    Ok(())
}

#[test]
fn test_number_key_off_a_sum_or_merge_of_mappings_is_nothing_5460() -> Result<()> {
    check(&[
        ("(.a + {})[0]", ""),
        ("(.a + .a)[0]", ""),
        ("(.a * {})[0]", ""),
        ("(.a + {})[1]", ""),
        ("(.a + {})[-1]", ""),
        ("(.a + {})[0]?", ""),
        ("(.a + {})[0][0]", ""),
        // A float-spelled whole number is the same key.
        ("(.a + {})[1.0]", ""),
        ("(.a + {})[2.00]", ""),
        ("(.a + {})[-1.0]", ""),
        ("(.a + {})[1e0]", ""),
        // `null + {}` is a mapping; a scalar head already answered nothing.
        ("(.n + {})[0]", ""),
        ("(.s + .s)[0]", ""),
        ("(1 + 2)[0]", ""),
        // The neighbours that already agreed.
        ("(.a + {})[1.5]", ""),
        ("(.a + {})[\"\"]", ""),
        ("(.a + {})[true]", ""),
        ("(.a + {})[null]", ""),
        ("(.a + {})[0,1]", ""),
    ])
}

#[test]
fn test_container_key_off_a_sum_or_merge_of_mappings_is_nothing_5460() -> Result<()> {
    check(&[
        ("(.a + {})[.a]", ""),
        ("(.n + {})[.a]", ""),
        ("(.a + {})[[0]]", ""),
    ])
}

/// What the nothing does to the filter around it.
#[test]
fn test_the_missing_read_drops_out_of_what_surrounds_it_5460() -> Result<()> {
    check(&[
        ("[(.a + {})[0]]", "[]\n"),
        ("[(.a + {})[.a]]", "[]\n"),
        ("(.a + {})[0] // \"d\"", "\"d\"\n"),
        ("(.a + {})[0] as $x | 1", "1\n"),
        ("(.a + {})[0] | length", ""),
    ])
}

/// A key the mapping holds, the bracket-free spellings, and an array head read as before.
#[test]
fn test_reads_that_find_something_are_unchanged_5460() -> Result<()> {
    check(&[
        ("(.a + {})[\"x\"]", "1\n"),
        ("(.a + {})[.k]", "1\n"),
        ("(.a + {}) | .[0]", "null\n"),
        ("(.a + {}).[0]", "null\n"),
        ("(.l + [3])[0]", "1\n"),
        ("(.l + [3])[-1]", "3\n"),
        ("(.l + [3])[9]", "null\n"),
        ("(.l + [3])[\"0\"]", "1\n"),
        ("(.n + .n)[0]", "null\n"),
    ])?;
    let (_, code) = run_yq_stdin("(.l + [3])[-9]", DOC, JSON)?;
    assert_eq!(code, 1);
    Ok(())
}

/// A write or a delete through such a head is the no-op it is in yq, not a path error.
#[test]
fn test_write_and_delete_through_a_number_key_off_a_sum_are_no_ops_5460() -> Result<()> {
    check(&[
        ("(.a + {})[0] = 5", DOC_JSON),
        ("(.a + {})[0] |= 5", DOC_JSON),
        ("(.l + [3])[0] = 9", DOC_JSON),
        ("(.a + {})[0].x = 1", DOC_JSON),
        ("del((.a + {})[0])", DOC_JSON),
        ("del((.l + [3])[0])", DOC_JSON),
        ("del((.l + [3])[-1])", DOC_JSON),
        ("del((.a + {})[1.0])", DOC_JSON),
        ("del((.a + {})[0].x)", DOC_JSON),
        ("del((.a + {})[\"x\"])", DOC_JSON),
        // Other targets of the same `del`, or the same assignment, still act.
        (
            "del((.a + {})[0], .k)",
            "{\"a\":{\"x\":1},\"l\":[1,2],\"n\":null,\"s\":\"str\",\"i\":0}\n",
        ),
        (
            "del(.l[0], (.a + {})[0])",
            "{\"a\":{\"x\":1},\"l\":[2],\"n\":null,\"s\":\"str\",\"k\":\"x\",\"i\":0}\n",
        ),
        (
            "((.a + {})[0], .k) = 1",
            "{\"a\":{\"x\":1},\"l\":[1,2],\"n\":null,\"s\":\"str\",\"k\":1,\"i\":0}\n",
        ),
    ])
}

/// `--eval-all` and `-P` take other routes to the same read.
#[test]
fn test_the_other_routes_agree_5460() -> Result<()> {
    for extra in [vec!["-P"], vec!["--eval-all"]] {
        let args: Vec<&str> = JSON.iter().chain(&extra).copied().collect();
        for (filter, expected) in [
            ("(.a + {})[0]", ""),
            ("(.a + {})[.a]", ""),
            ("[(.a + {})[0]]", "[]\n"),
            ("(.l + [3])[-1]", "3\n"),
        ] {
            let (stdout, code) = run_yq_stdin(filter, DOC, &args)?;
            assert_eq!((stdout.as_str(), code), (expected, 0), "{filter} {extra:?}");
        }
    }
    Ok(())
}
