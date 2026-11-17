//! #7311: `.[]` and `map(f)` over a mapping collapse a repeated key (first position, last value)
//! by decoding only the keys whose hash another key shares, unless some key has no hash -- a
//! complex key (`? [x]`) -- and then every key, as the full collapse did. Each row is
//! `(document, filter, stdout)` as yq v4.53.3 answers with `-o json -I0`: a repeated plain key
//! beside repeated complex keys collapses both pairs, at the root and one level down.

use anyhow::Result;

use crate::run_yq_stdin;

const ROWS: &[(&str, &str, &str)] = &[
    ("a: 1\na: 2\n? [x]\n: 3\n? [x]\n: 4\n", "map(.)", "[2,4]\n"),
    ("a: 1\na: 2\n? [x]\n: 3\n? [x]\n: 4\n", "[.[]]", "[2,4]\n"),
    (
        "m:\n  a: 1\n  a: 2\n  ? [x]\n  : 3\n  ? [x]\n  : 4\n",
        ".m | map(.)",
        "[2,4]\n",
    ),
    (
        "m:\n  a: 1\n  a: 2\n  ? [x]\n  : 3\n  ? [x]\n  : 4\n",
        ".m | [.[]]",
        "[2,4]\n",
    ),
    ("a: 1\nb: 2\na: 3\n", "map(.)", "[3,2]\n"),
    ("a: 1\nb: 2\na: 3\n", "[.[]]", "[3,2]\n"),
];

#[test]
fn test_complex_keys_still_collapse_beside_a_repeated_key_7311() -> Result<()> {
    for &(document, filter, stdout) in ROWS {
        let (out, code) = run_yq_stdin(filter, document, &["-o", "json", "-I0"])?;
        assert_eq!(
            (out.as_str(), code),
            (stdout, 0),
            "{filter} on {document:?}"
        );
    }
    Ok(())
}
