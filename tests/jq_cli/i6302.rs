//! #6302: a lazy unbounded `X{n,}?` over a body with a consuming way before its empty one
//! (`(?:b*||\w)`) is a counted loop in Oniguruma when the body's compiled length times `n` is more
//! than ten, and leaves at the body's empty way even before its minimum. The rewrite that writes
//! that out doubles with every count, because each level holds the next one twice with the empty
//! way between the two, so the earlier engine gave it up when one text passed 32 KiB and the repeat
//! kept the `regex` crate's reading, a recorded divergence. The count at which that happened
//! depends on the body, because a class is written out in full in each copy. On Oniguruma the
//! counts past that boundary answer as jq 1.7.1 does (#7965). Every expectation is what
//! `/usr/bin/jq` 1.7.1 printed. `AGREE` is the counts below the boundary.

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

/// (text, pattern, flags, jq 1.7.1): the counts past the earlier engine's size cap.
const PAST_THE_CAP: &[(&str, &str, &str, &str)] = &[
    ("ab", r"(?:b*||\w){8,}?", "gn", r#"["ab"]"#),
    ("ab", r"(?:b*||\w){12,}?", "gn", r#"["ab"]"#),
    ("ab", r"(?:b*||\p{L}){10,}?", "gn", r#"["ab"]"#),
    ("ab", r"(?:b*||[a-z]){11,}?", "gn", r#"["ab"]"#),
];

#[test]
fn test_a_lazy_counted_repeat_past_the_size_cap_answers_as_jq_does_6302() -> Result<()> {
    for &(text, pattern, flags, expected) in PAST_THE_CAP {
        assert_eq!(
            match_on(text, pattern, flags)?,
            expected,
            "{text:?} ~ {pattern:?} [{flags}]"
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
