//! #6832: a computed string of 4 KiB or more that is bound with `as` is read the same in every
//! round of a loop: it is held behind a refcount in its marker, so the re-entry's clone of it is a
//! pointer bump and not a copy. The loop is `--jq-extensions` surface, so these rows have no yq
//! capture; each answer is what the build before the change printed, and the unit is yq's own
//! (`length` of a string counts bytes). A short string, a string a stage rewrites, and a write
//! through the bound string are the shapes the shared storage must leave alone.

use anyhow::Result;

use crate::run_yq_stdin;

const ARGS: &[&str] = &["--jq-extensions", "-n", "-o=json", "-I=0"];

/// `(filter, output)`, the string being `"é" * 2100` (4200 bytes) unless a row says otherwise.
const ROWS: &[(&str, &str)] = &[
    // The bound string is read the same every round, long and short, `$w`-fresh each round.
    (
        r#"("é" * 2100) as $t | [range(3) | . as $w | ($t | length)]"#,
        "[4200,4200,4200]\n",
    ),
    (
        r#"("é" * 2) as $t | [range(3) | . as $w | ($t | length)]"#,
        "[4,4,4]\n",
    ),
    // A stage that rewrites the string leaves the bound one as it was.
    (
        r#"("é" * 2100) as $t | [range(2) | . as $w | ($t | ascii_upcase | length), ($t | length)]"#,
        "[4200,4200,4200]\n",
    ),
    // `. as $t` over the input itself, the identity binding the marker re-roots each round.
    (
        r#"("é" * 2100) | . as $t | [range(2) | . as $w | ($t | length)]"#,
        "[4200,4200]\n",
    ),
    // A write on one copy is not seen by the bound string.
    (
        r#"("é" * 2100) as $t | [range(2) | . as $w | ($t + "z" | length), ($t | length)]"#,
        "[4201,4201,4200]\n",
    ),
    // The bound string's own text is intact after another read was rewritten: what is printed
    // is the string, not only its length.
    (
        r#"("é" * 2100) as $t | [range(2) | . as $w | ($t | sub("^é+"; "tail")), ($t | sub("é+$"; "head"))]"#,
        "[\"tail\",\"tail\",\"head\"]\n",
    ),
    // Equality and ordering of two reads of one bound string.
    (
        r#"("é" * 2100) as $t | [range(2) | . as $w | ($t == $t), ($t < ($t + "a"))]"#,
        "[true,true,true]\n",
    ),
];

#[test]
fn test_a_large_bound_string_is_read_the_same_every_round_6832() -> Result<()> {
    for (filter, want) in ROWS {
        let (stdout, code) = run_yq_stdin(filter, "", ARGS)?;
        assert_eq!(code, 0, "exit code of {filter:?}");
        assert_eq!(&stdout, want, "{filter:?}");
    }
    Ok(())
}
