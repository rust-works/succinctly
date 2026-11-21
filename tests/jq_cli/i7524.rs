//! #7524: an error message that truncates the value it embeds never splits a character: the cut moves back to
//! the character start and no U+FFFD is written (jq 1.8, jqlang/jq#3249; jq 1.7.1 wrote the U+FFFD, and #6434
//! had matched that). The budget stays 1.7.1's 11 bytes: jq 1.8.2's wider previews (jqlang/jq#3478) are not
//! adopted. The six rows are the regression test of jq's own #3249 (`tests/jq.test` at jq-1.8.0, which still has
//! the 1.7.1 budget).

use anyhow::Result;

use crate::run_jq_full;

#[test]
fn the_rows_of_jq_3249_7524() -> Result<()> {
    let (out, err, code) = run_jq_full(
        &["-nc", r#""x" * range(0; 12; 2) + "☆" * 5 | try -. catch ."#],
        None,
    )?;
    assert_eq!((err.as_str(), code), ("", 0));
    assert_eq!(
        out,
        "\"string (\\\"☆☆☆...) cannot be negated\"\n\
         \"string (\\\"xx☆☆...) cannot be negated\"\n\
         \"string (\\\"xxxx☆☆...) cannot be negated\"\n\
         \"string (\\\"xxxxxx☆...) cannot be negated\"\n\
         \"string (\\\"xxxxxxxx...) cannot be negated\"\n\
         \"string (\\\"xxxxxxxxxx...) cannot be negated\"\n"
    );
    Ok(())
}

/// No message carries a U+FFFD, wherever the cut falls in a 2, 3 or 4 byte character.
#[test]
fn no_cut_leaves_a_replacement_character_7524() -> Result<()> {
    for character in ["é", "あ", "☆", "😀"] {
        for prefix in 0..6 {
            let program = format!(
                r#""{}{}" | try -. catch ., try (.[]) catch ."#,
                "x".repeat(prefix),
                character.repeat(8)
            );
            let (out, err, code) = run_jq_full(&["-nc", &program], None)?;
            assert_eq!((err.as_str(), code), ("", 0), "{program}");
            assert!(!out.contains('\u{FFFD}'), "{program}: {out}");
            assert!(out.contains("...)"), "{program}: {out}");
        }
    }
    Ok(())
}
