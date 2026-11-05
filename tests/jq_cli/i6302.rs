//! #6302: a lazy unbounded `X{n,}?` over a body with a consuming way before its empty one
//! (`(?:b*||\w)`) is a counted loop in Oniguruma when the body's compiled length times `n` is
//! more than ten, and leaves at the body's empty way even before its minimum. The rewrite that
//! writes that out doubles with every count, because each level holds the next one twice with the
//! empty way between the two, so it is given up when one text passes 32 KiB and the repeat keeps
//! the `regex` crate's reading. The count at which that happens depends on the body, because a
//! class is written out in full in each copy. The first column is what `/usr/bin/jq` 1.7.1
//! printed; the second is this tool's answer, pinned so `docs/compliance/jq/limitations.md` stays
//! true. `AGREE` is the counts below the boundary.

use anyhow::Result;

use crate::run_jq_full;

fn match_on(text: &str, pattern: &str, flags: &str) -> Result<String> {
    let program = r#"$s | try ([match($p; $f) | .string]) catch "ERR""#;
    let (stdout, stderr, code) = run_jq_full(
        &[
            "-nc", "--arg", "s", text, "--arg", "p", pattern, "--arg", "f", flags, program,
        ],
        None,
    )?;
    assert_eq!(code, 0, "{text:?} ~ {pattern:?} [{flags}]: {stderr}");
    Ok(stdout.trim().to_string())
}

/// (text, pattern, flags, jq 1.7.1, this tool).
const GAPS: &[(&str, &str, &str, &str, &str)] = &[
    ("ab", r"(?:b*||\w){8,}?", "gn", r#"["ab"]"#, r#"["a","b"]"#),
    ("ab", r"(?:b*||\w){12,}?", "gn", r#"["ab"]"#, r#"["a","b"]"#),
    (
        "ab",
        r"(?:b*||\p{L}){10,}?",
        "gn",
        r#"["ab"]"#,
        r#"["a","b"]"#,
    ),
    (
        "ab",
        r"(?:b*||[a-z]){11,}?",
        "gn",
        r#"["ab"]"#,
        r#"["a","b"]"#,
    ),
];

#[test]
fn test_a_lazy_counted_repeat_past_the_size_cap_keeps_the_crates_reading_6302() -> Result<()> {
    for &(text, pattern, flags, jq, ours) in GAPS {
        assert_eq!(
            match_on(text, pattern, flags)?,
            ours,
            "{text:?} ~ {pattern:?} [{flags}]"
        );
        assert_ne!(
            jq, ours,
            "{text:?} ~ {pattern:?}: the gap is closed, update the limitations page"
        );
    }
    Ok(())
}

/// One count below each boundary, written out in full and answering as jq does.
const AGREE: &[(&str, &str, &str, &str)] = &[
    ("ab", r"(?:b*||\w){7,}?", "gn", r#"["ab"]"#),
    ("ab", r"(?:b*||\p{L}){9,}?", "gn", r#"["ab"]"#),
    ("ab", r"(?:b*||[a-z]){10,}?", "gn", r#"["ab"]"#),
];

#[test]
fn test_a_lazy_counted_repeat_below_the_size_cap_answers_as_jq_does_6302() -> Result<()> {
    for &(text, pattern, flags, expected) in AGREE {
        assert_eq!(
            match_on(text, pattern, flags)?,
            expected,
            "{text:?} ~ {pattern:?} [{flags}]"
        );
    }
    Ok(())
}
