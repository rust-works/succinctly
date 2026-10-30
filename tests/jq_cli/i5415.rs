//! #5415: `-C` renders a result through the streaming printers and colors it a block at a time,
//! instead of materializing the record and building two whole-output strings. The expected bytes
//! of the pinned rows were captured from the `main` build that did the old, materializing
//! render (`format_json` then `colorize_json`), because that change must not move a byte of `-C`'s
//! stdout. #5473 then moved them to jq 1.7.1's own bytes, re-captured from `/usr/bin/jq`.

use anyhow::Result;

use crate::{run_jq_full, run_jq_stdin};

/// `text` without its ANSI escape sequences (`ESC [ ... m`).
fn strip_ansi(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c != '\u{1b}' {
            out.push(c);
            continue;
        }
        for skipped in chars.by_ref() {
            if skipped == 'm' {
                break;
            }
        }
    }
    out
}

/// A NaN is `null` in jq's output, but colored as a number wrapping a null (#3413). The streamed
/// render marks it for the colorizer: from a computed owned value, whose literal text is `null`
/// and whose repr is the NaN, and from jq's own `nan`/`sNaN12` words.
#[test]
fn test_nan_keeps_its_double_wrapped_color_5415() -> Result<()> {
    let nan = "\u{1b}[0;39m\u{1b}[0;90mnull\u{1b}[0m\u{1b}[0m";
    // Every row's bytes were captured from jq 1.7.1 (#5473).
    for (doc, filter, want) in [
        ("[nan]", "map(.)?", "\u{1b}[1;39m[\u{1b}[0;39m\u{1b}[0;90mnull\u{1b}[0m\u{1b}[0m\u{1b}[1;39m\u{1b}[1;39m]\u{1b}[0m\n"),
        ("nan", "[.,.]", "\u{1b}[1;39m[\u{1b}[0;39m\u{1b}[0;90mnull\u{1b}[0m\u{1b}[0m\u{1b}[1;39m,\u{1b}[0;39m\u{1b}[0;90mnull\u{1b}[0m\u{1b}[0m\u{1b}[1;39m\u{1b}[1;39m]\u{1b}[0m\n"),
        ("[nan]", ".", "\u{1b}[1;39m[\u{1b}[0;39m\u{1b}[0;90mnull\u{1b}[0m\u{1b}[0m\u{1b}[1;39m\u{1b}[1;39m]\u{1b}[0m\n"),
        ("[1,NaN]", ".", "\u{1b}[1;39m[\u{1b}[0;39m1\u{1b}[0m\u{1b}[1;39m,\u{1b}[0;39m\u{1b}[0;90mnull\u{1b}[0m\u{1b}[0m\u{1b}[1;39m\u{1b}[1;39m]\u{1b}[0m\n"),
        ("null", "[nan]", "\u{1b}[1;39m[\u{1b}[0;39m\u{1b}[0;90mnull\u{1b}[0m\u{1b}[0m\u{1b}[1;39m\u{1b}[1;39m]\u{1b}[0m\n"),
    ] {
        let (out, code) = run_jq_stdin(filter, doc, &["-C", "-c"])?;
        assert_eq!((out.as_str(), code), (want, 0), "{doc} | {filter}");
    }
    let (out, code) = run_jq_stdin("to_entries?", r#"{"a":nan,"b":sNaN12}"#, &["-C", "-c"])?;
    assert_eq!(code, 0);
    assert_eq!(out.matches(nan).count(), 2, "{out:?}");
    // A string that merely holds the word is not a NaN.
    let (out, _) = run_jq_stdin(".", r#"["nan","banana"]"#, &["-C", "-c"])?;
    assert!(!out.contains("\u{1b}[0;90m"), "{out:?}");
    Ok(())
}

/// `-C` collapses a repeated key under `--preserve-input` too, as the materializing render
/// always did, though `-c --preserve-input` echoes both.
#[test]
fn test_preserve_input_collapses_a_repeated_key_under_color_5415() -> Result<()> {
    let doc = r#"{"a":1,"a":2,"b":{"c":1,"c":3}}"#;
    let (out, code) = run_jq_stdin(".", doc, &["-C", "-c", "--preserve-input"])?;
    assert_eq!(code, 0);
    assert_eq!(
        out,
        "\u{1b}[1;39m{\u{1b}[0m\u{1b}[1;34m\"a\"\u{1b}[0m\u{1b}[1;39m:\u{1b}[0m\u{1b}[0;39m2\u{1b}[0m\u{1b}[1;39m,\u{1b}[0m\u{1b}[1;34m\"b\"\u{1b}[0m\u{1b}[1;39m:\u{1b}[0m\u{1b}[1;39m{\u{1b}[0m\u{1b}[1;34m\"c\"\u{1b}[0m\u{1b}[1;39m:\u{1b}[0m\u{1b}[0;39m3\u{1b}[0m\u{1b}[1;39m\u{1b}[1;39m}\u{1b}[0m\u{1b}[1;39m\u{1b}[1;39m}\u{1b}[0m\n"
    );
    // The default route's rendering of the same input is unchanged.
    let (out, _) = run_jq_stdin(".", doc, &["-c", "--preserve-input"])?;
    assert_eq!(out, "{\"a\":1,\"a\":2,\"b\":{\"c\":1,\"c\":3}}\n");
    Ok(())
}

/// An overflowing literal in an owned result keeps the spelling the materializing render gave it.
#[test]
fn test_overflow_literal_spelling_is_unchanged_under_color_5415() -> Result<()> {
    let (out, code) = run_jq_stdin("map(.)?", "[1e999]", &["-C", "-c"])?;
    assert_eq!(code, 0);
    assert_eq!(
        out,
        "\u{1b}[1;39m[\u{1b}[0;39m1E+999\u{1b}[0m\u{1b}[1;39m\u{1b}[1;39m]\u{1b}[0m\n"
    );
    Ok(())
}

/// `-S` still renders from the whole value, and its text is colored in blocks.
#[test]
fn test_sorted_keys_and_tab_indent_color_5415() -> Result<()> {
    let (out, code) = run_jq_stdin(".", r#"[1,{"a":[]},"x"]"#, &["-C", "--tab", "-S"])?;
    assert_eq!(code, 0);
    assert_eq!(
        out,
        "\u{1b}[1;39m[\n\t\u{1b}[0;39m1\u{1b}[0m\u{1b}[1;39m,\n\t\u{1b}[1;39m{\n\t\t\u{1b}[0m\u{1b}[1;34m\"a\"\u{1b}[0m\u{1b}[1;39m: \u{1b}[0m\u{1b}[1;39m[]\u{1b}[0m\u{1b}[1;39m\n\t\u{1b}[1;39m}\u{1b}[0m\u{1b}[1;39m,\n\t\u{1b}[0;32m\"x\"\u{1b}[0m\u{1b}[1;39m\n\u{1b}[1;39m]\u{1b}[0m\n"
    );
    Ok(())
}

/// A result far larger than the colorizer's block, with strings of multi-byte characters so that
/// a block boundary lands inside one for some lengths: the colored text is the uncolored text
/// plus escapes, on every route that writes it (the cursor streamer, `print_json`, the owned
/// printer, and `-S`/`-a`, which materialize).
#[test]
fn test_colored_text_is_the_plain_text_across_block_boundaries_5415() -> Result<()> {
    // The second tail holds a surrogate pair, which the streamed route hands back to the
    // materializing one (`colored_stream_safe`); both must give the plain text.
    for tail in ["\\n\\u00e9", "\\n\\u00e9\\ud83d\\ude00"] {
        colored_matches_plain(tail)?;
    }
    Ok(())
}

fn colored_matches_plain(tail: &str) -> Result<()> {
    let mut items = Vec::new();
    for n in 0..400 {
        let word = "é".repeat(n % 41) + &"☃".repeat(n % 7) + tail;
        items.push(format!(
            r#"{{"k{n}":"{word}","n":[{n},{n}.5,-{n}e3,true,false,null],"o":{{}}}}"#
        ));
    }
    let doc = format!("[{}]", items.join(","));
    for filter in [".", ".[]", "[.[]]", "[.[] | .n[0]]", "map(.n)?", ".[3]"] {
        for flags in [
            &["-c"][..],
            &[][..],
            &["--tab"][..],
            &["-S"][..],
            &["-a"][..],
        ] {
            let (plain, _, plain_code) = run_jq_full(&[flags, &[filter]].concat(), Some(&doc))?;
            let mut colored_flags = vec!["-C"];
            colored_flags.extend_from_slice(flags);
            colored_flags.push(filter);
            let (colored, _, code) = run_jq_full(&colored_flags, Some(&doc))?;
            assert_eq!(code, plain_code, "{filter} {flags:?}");
            assert!(colored.len() > plain.len(), "{filter} {flags:?}");
            assert_eq!(strip_ansi(&colored), plain, "{filter} {flags:?}");
        }
    }
    Ok(())
}

/// A malformed scalar is refused with nothing on stdout, now with the message the default route
/// gives it rather than the materializer's.
#[test]
fn test_malformed_scalar_is_refused_with_the_default_routes_message_5415() -> Result<()> {
    for doc in [r#"{"a":tru}"#, "[[1],[tru]]"] {
        let (out, err, code) = run_jq_full(&["-C", "-c", "."], Some(doc))?;
        let (plain_out, plain_err, plain_code) = run_jq_full(&["-c", "."], Some(doc))?;
        assert_eq!((out.as_str(), code), ("", 5), "{doc}");
        assert_eq!((plain_out.as_str(), plain_code), ("", 5), "{doc}");
        assert_eq!(err, plain_err, "{doc}");
        assert!(err.contains("invalid keyword 'tru'"), "{err}");
    }
    Ok(())
}

/// A document the streamed route cannot vouch for keeps the refusals of the materializing one:
/// an unpaired surrogate (#1247) and nesting past the limit (#1818), at a scalar 256 levels down,
/// the deepest document jq's parser accepts (#5497).
#[test]
fn test_undecodable_and_over_deep_documents_are_still_refused_under_color_5415() -> Result<()> {
    let (out, err, code) = run_jq_full(&["-C", "-c", "."], Some(r#"{"a":"\ud800"}"#))?;
    assert_eq!((out.as_str(), code), ("", 5), "{err}");
    assert!(err.contains("invalid unicode escape sequence"), "{err}");
    // The default route still echoes it.
    let (out, _, code) = run_jq_full(&["-c", "."], Some(r#"{"a":"\ud800"}"#))?;
    assert_eq!((out.as_str(), code), ("{\"a\":\"\\ud800\"}\n", 0));
    let deep = format!("{}1{}", "[".repeat(256), "]".repeat(256));
    let (out, err, code) = run_jq_full(&["-C", "-c", "."], Some(&deep))?;
    assert_eq!((out.as_str(), code), ("", 5), "{err}");
    assert!(err.contains("nesting depth exceeds limit of 256"), "{err}");
    Ok(())
}

/// `--preserve-input` echoes a number's source text (`1.`) on the default route, but `-C` has
/// always rendered it, and still does.
#[test]
fn test_preserve_input_still_renders_numbers_under_color_5415() -> Result<()> {
    for filter in [".", ".[0]", "[.[]]"] {
        let (out, code) = run_jq_stdin(filter, "[1.]", &["-C", "-c", "--preserve-input"])?;
        assert_eq!(code, 0, "{filter}");
        assert!(out.contains("\u{1b}[0;39m1\u{1b}[0m"), "{filter}: {out:?}");
    }
    Ok(())
}
