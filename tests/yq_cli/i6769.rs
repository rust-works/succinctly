//! #6769: yq reads the arguments of `sub` and `with_dtf` as the left and right child of whatever
//! node holds them, so a comma spells them as well as `;` does: `sub("dogs", "cats")` is how yq's
//! own string-operators page writes it, and `with_dtf("layout", .a += "3h1m")` is how its datetime
//! pages do. Every answer was captured from yq v4.53.3 (`-o=json -I=0`).

use anyhow::Result;

use crate::{run_jq_stdin, run_yq_stdin};

const DOC: &str = "a: dogs are great\nb: [1, 2]\n";
const DATE: &str = "a: Saturday, 15-Dec-01 at 2:59AM GMT\n";
const LAYOUT: &str = "Monday, 02-Jan-06 at 3:04PM MST";

#[test]
fn test_yq_sub_reads_a_comma_for_its_pattern_and_replacement_6769() -> Result<()> {
    for (filter, want) in [
        (
            ".a |= sub(\"dogs\", \"cats\")",
            "{\"a\":\"cats are great\",\"b\":[1,2]}\n",
        ),
        (".a | sub(\"dogs\"; \"cats\")", "\"cats are great\"\n"),
        // The rest of the list is the replacement, and its first output is the one read.
        (".a | sub(\"dogs\", \"X\", \"Y\")", "\"X are great\"\n"),
        (
            ".a | sub(\"dogs\", \"X\", \"Y\", \"Z\")",
            "\"X are great\"\n",
        ),
        // Parentheses only group: `sub((a, b))` is `sub(a, b)`, and a grouped pattern is
        // still the first child.
        (".a | sub((\"dogs\", \"X\"))", "\"X are great\"\n"),
        (".a | sub((\"dogs\", \"X\"), \"Y\")", "\"Y are great\"\n"),
        // The replacement is an expression read against the input.
        (".a | sub(\"d\", .b)", "\"ogs are great\"\n"),
        (
            ".a |= (sub(\"dogs\", \"cats\") | sub(\"are\", \"were\"))",
            "{\"a\":\"cats were great\",\"b\":[1,2]}\n",
        ),
        (".a | [sub(\"dogs\", \"X\")]", "[\"X are great\"]\n"),
    ] {
        let (stdout, code) = run_yq_stdin(filter, DOC, &["-o=json", "-I=0"])?;
        assert_eq!((stdout.as_str(), code), (want, 0), "{filter}");
    }
    Ok(())
}

#[test]
fn test_yq_with_dtf_reads_a_comma_for_its_layout_and_body_6769() -> Result<()> {
    for (filter, want) in [
        (
            format!("with_dtf(\"{LAYOUT}\", .a += \"3h1m\")"),
            "{\"a\":\"Saturday, 15-Dec-01 at 6:00AM GMT\"}\n",
        ),
        (
            format!("with_dtf(\"{LAYOUT}\", .a -= \"3h1m\")"),
            "{\"a\":\"Friday, 14-Dec-01 at 11:58PM GMT\"}\n",
        ),
        (
            format!("with_dtf((\"{LAYOUT}\", .a += \"3h1m\"))"),
            "{\"a\":\"Saturday, 15-Dec-01 at 6:00AM GMT\"}\n",
        ),
        // The `;` spelling is unchanged.
        (
            format!("with_dtf(\"{LAYOUT}\"; .a += \"3h1m\")"),
            "{\"a\":\"Saturday, 15-Dec-01 at 6:00AM GMT\"}\n",
        ),
    ] {
        let (stdout, code) = run_yq_stdin(&filter, DATE, &["-o=json", "-I=0"])?;
        assert_eq!((stdout.as_str(), code), (want, 0), "{filter}");
    }
    Ok(())
}

/// jq has no comma spelling for `sub`: it is `sub/1`, which is not defined (jq 1.7.1).
#[test]
fn test_jq_sub_still_refuses_a_comma_6769() -> Result<()> {
    let (stdout, code) = run_jq_stdin("\"a\" | sub(\"a\", \"b\")", "null", &[])?;
    assert_eq!((stdout.as_str(), code), ("", 3));
    Ok(())
}
