//! #6169: a `\K` inside a look-behind starts the match before the offset a global search resumed
//! from, so resuming at the match's end found the same match again and the search never ended,
//! pushing a match each time (1.4 GB in 20 s). jq 1.7.1 loops the same way on the patterns below
//! that end a match with the `\K` (`"cab" | [match("(?<=a\\Kb)";"g")]` never answers), so there is
//! no jq answer to copy for them: a match that ends where the search began, from before it, was
//! found by the search before and is skipped. The rows jq does answer (`(?<=\K)`, `(?<=a\K)`,
//! `(?<!a\K)`) were captured from jq 1.7.1 (`/usr/bin/jq -n`) and must not move.

use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::Result;

use crate::succinctly_bin;

/// Runs `succinctly jq -nc <program>`, killing it past `LIMIT` so a search that no longer ends
/// fails the test instead of growing without bound.
fn run_bounded(program: &str) -> Result<(String, Option<i32>)> {
    const LIMIT: Duration = Duration::from_secs(15);
    let mut child = Command::new(succinctly_bin())
        .args(["jq", "-nc", program])
        .env("LC_ALL", "C")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    let started = Instant::now();
    loop {
        if let Some(status) = child.try_wait()? {
            let mut stdout = String::new();
            if let Some(mut pipe) = child.stdout.take() {
                pipe.read_to_string(&mut stdout)?;
            }
            return Ok((stdout, status.code()));
        }
        if started.elapsed() > LIMIT {
            child.kill()?;
            child.wait()?;
            return Ok((String::new(), None));
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn answer(program: &str) -> Result<String> {
    let (stdout, code) = run_bounded(program)?;
    assert_eq!(code, Some(0), "{program}: no answer within the limit");
    Ok(stdout.trim_end().to_string())
}

#[test]
fn test_a_k_ending_a_look_behind_match_ends_the_global_search_6169() -> Result<()> {
    for (program, expected) in [
        (r#""cab" | [match("(?<=a\\Kb)";"g") | .string]"#, r#"["b"]"#),
        (r#""cab" | [match("(?<=\\Ka)";"g") | .string]"#, r#"["a"]"#),
        (r#""cab" | [match("(?<=a\\K.)";"g") | .string]"#, r#"["b"]"#),
        (r#""cab" | [match("(?<=\\K.)";"g") | .string]"#, r#"["c"]"#),
        (r#""cabab" | [match("(?<=a\\Kb)";"g") | .offset]"#, "[2,4]"),
        (
            r#""cab" | [match("(?<=a\\Kb)";"gl") | .string]"#,
            r#"["b"]"#,
        ),
        (
            r#""cab" | [match("(?<=a\\Kb)";"gn") | .string]"#,
            r#"["b"]"#,
        ),
        (r#""cab" | [scan("(?<=a\\Kb)")]"#, r#"["b"]"#),
        (r#""cab" | gsub("(?<=a\\Kb)"; "X")"#, r#""caX""#),
        (r#""cab" | [splits("(?<=a\\Kb)")]"#, r#"["ca",""]"#),
        // Offsets are in codepoints, past a multi-byte character.
        (r#""éabéab" | [match("(?<=a\\Kb)";"g") | .offset]"#, "[2,5]"),
        (r#""éab" | [match("(?<=é\\Ka)";"g") | .string]"#, r#"["a"]"#),
        (r#""éabéab" | gsub("(?<=a\\Kb)"; "é")"#, r#""éaééaé""#),
        // The refused-in-jq look-behinds (`.?`, `.*` after the `\K`) end too.
        (
            r#""cab" | [match("(?<=a\\K.?)b?";"g") | .string]"#,
            r#"["b"]"#,
        ),
        (r#""cab" | [match("(?<=\\K.?)";"g") | .string]"#, r#"[""]"#),
    ] {
        assert_eq!(answer(program)?, expected, "{program}");
    }
    Ok(())
}

/// What jq 1.7.1 answers, which the skip must leave alone.
#[test]
fn test_a_global_search_that_jq_ends_is_unchanged_6169() -> Result<()> {
    for (program, expected) in [
        (
            r#""cab" | [match("(?<=\\K)";"g") | .string]"#,
            r#"["","","",""]"#,
        ),
        (r#""cab" | [match("(?<=a\\K)";"g") | .string]"#, r#"[""]"#),
        (
            r#""cab" | [match("(?<!a\\K)";"g") | .string]"#,
            r#"["","",""]"#,
        ),
        (
            r#""cab" | [match("(?<=(?:a\\K))";"g") | .string]"#,
            r#"[""]"#,
        ),
        (
            r#""cab" | [match("(?<=\\G)";"g") | .string]"#,
            r#"["","","",""]"#,
        ),
        (r#""cab" | [match("(?=a\\K)";"g") | .string]"#, r#"[""]"#),
        (r#""cab" | [match("(?<=a\\Kb)") | .string]"#, r#"["b"]"#),
    ] {
        assert_eq!(answer(program)?, expected, "{program}");
    }
    Ok(())
}
