//! #8070: on a mapping, a bracket key spelled as an integer (`0x1`, `01`) names the key of that
//! text (#7999), so `key` and `path` after it report that key, not the member the folded value `1`
//! names; and a key rename through any integer bracket on a mapping renames the member the bracket
//! reads. Every yq row is a live yq v4.53.3 capture, run over a file and over stdin.

use std::io::Write;

use anyhow::Result;
use tempfile::NamedTempFile;

use crate::{run_jq_stdin, run_yq_file, run_yq_stdin};

/// String keys spelled like integers, each a different spelling of 1.
const SPELLED: &str = "m:\n  \"0x1\": five\n  \"1\": six\n  \"01\": seven\n";
/// Typed integer keys beside a quoted one, and a sequence.
const TYPED: &str =
    "m:\n  0x1: hexint\n  1: one\n  \"2\": two\n  01: lz\n  -1: neg\ns: [a, b, c]\n";
/// A quoted `'1'` and a typed `2`, the issue's rename document.
const RENAME: &str = "m:\n  '1': six\n  2: two\n";

/// Every row over a file and over stdin.
fn check(rows: &[(&str, &str, &[&str], &str)]) -> Result<()> {
    for (filter, input, args, want) in rows {
        let mut file = NamedTempFile::new()?;
        file.write_all(input.as_bytes())?;
        let path = file.path().to_str().expect("utf-8 temp path");
        let (out, code) = run_yq_file(filter, path, args)?;
        assert_eq!(
            (out.as_str(), code),
            (*want, 0),
            "{filter} {args:?} over {input:?}"
        );
        let (out, code) = run_yq_stdin(filter, input, args)?;
        assert_eq!(
            (out.as_str(), code),
            (*want, 0),
            "{filter} {args:?} over stdin {input:?}"
        );
    }
    Ok(())
}

const JSON: &[&str] = &["-o", "json", "-I0"];

/// `key` and `path` after a spelled bracket name the key the bracket reads, with that key's value.
#[test]
fn test_key_and_path_after_a_spelled_key_on_a_mapping_8070() -> Result<()> {
    check(&[
        (".m[0x1] | key", SPELLED, JSON, "\"0x1\"\n"),
        (".m[01] | key", SPELLED, JSON, "\"01\"\n"),
        (".m[0x1] | path", SPELLED, JSON, "[\"m\",\"0x1\"]\n"),
        (".m[01] | path", SPELLED, JSON, "[\"m\",\"01\"]\n"),
        (".m[0x1] | [key, .]", SPELLED, JSON, "[\"0x1\",\"five\"]\n"),
        (".m[01] | [key, .]", SPELLED, JSON, "[\"01\",\"seven\"]\n"),
        (".m | .[0x1] | key", SPELLED, JSON, "\"0x1\"\n"),
        (".m[0x1] | key | tag", SPELLED, JSON, "\"!!str\"\n"),
        (
            ".m[1, 0x1] | path",
            SPELLED,
            JSON,
            "[\"m\",\"1\"]\n[\"m\",\"0x1\"]\n",
        ),
        (".m[1, 0x1] | key", SPELLED, JSON, "\"1\"\n\"0x1\"\n"),
        // The control: a canonical bracket and a string bracket were already right.
        (".m[1] | key", SPELLED, JSON, "\"1\"\n"),
        (".m[\"0x1\"] | key", SPELLED, JSON, "\"0x1\"\n"),
        (".m[0x1] | key", SPELLED, &[], "0x1\n"),
    ])
}

/// A typed key keeps its spelling in `key` and is its value in `path`, as for `.m["0x1"]`; a key
/// the mapping lacks is the absent member at the value, never the decimal key `.m[2]` reads.
#[test]
fn test_key_and_path_after_a_spelled_key_on_typed_keys_8070() -> Result<()> {
    check(&[
        (".m[0x1] | key", TYPED, &[], "0x1\n"),
        (".m[01] | key", TYPED, &[], "01\n"),
        (".m[0x1] | path", TYPED, &[], "- m\n- 1\n"),
        (".m[01] | path", TYPED, JSON, "[\"m\",1]\n"),
        (".m[0x1] | key | line", TYPED, &[], "2\n"),
        (".m[0x2] | path", TYPED, &[], "- m\n- 2\n"),
        (".m[0x2] | [key, .]", SPELLED, JSON, "[2,null]\n"),
        // A sequence takes the value, as before.
        (".s[0x1] | path", TYPED, JSON, "[\"s\",1]\n"),
        (".s[01] | key", TYPED, JSON, "1\n"),
    ])
}

/// `(.m[N] | key) = ...` renames the member `.m[N]` reads: quoted or typed, spelled or not, and a
/// member the mapping lacks is created as `null` and renamed.
#[test]
fn test_key_rename_through_an_integer_bracket_on_a_mapping_8070() -> Result<()> {
    check(&[
        (
            "(.m[1] | key) = \"q\"",
            RENAME,
            &[],
            "m:\n  'q': six\n  2: two\n",
        ),
        (
            "(.m[2] | key) = \"q\"",
            RENAME,
            &[],
            "m:\n  '1': six\n  q: two\n",
        ),
        (
            "(.m[1] | key) |= \"q\"",
            RENAME,
            &[],
            "m:\n  'q': six\n  2: two\n",
        ),
        (
            "(.m[2] | key) |= \"q\"",
            RENAME,
            &[],
            "m:\n  '1': six\n  q: two\n",
        ),
        ("(.m[1] | key) = 5", RENAME, &[], "m:\n  5: six\n  2: two\n"),
        (
            "(.m[3] | key) = \"q\"",
            RENAME,
            &[],
            "m:\n  '1': six\n  2: two\n  q: null\n",
        ),
        (
            "(.m[-1] | key) = \"q\"",
            RENAME,
            &[],
            "m:\n  '1': six\n  2: two\n  q: null\n",
        ),
        (
            "(.m[0x2] | key) = \"q\"",
            RENAME,
            &[],
            "m:\n  '1': six\n  2: two\n  q: null\n",
        ),
        (
            "(.m[1] | key) = \"q\"",
            TYPED,
            &[],
            "m:\n  0x1: hexint\n  q: one\n  \"2\": two\n  01: lz\n  -1: neg\ns: [a, b, c]\n",
        ),
        (
            "(.m[2] | key) = \"q\"",
            TYPED,
            &[],
            "m:\n  0x1: hexint\n  1: one\n  \"q\": two\n  01: lz\n  -1: neg\ns: [a, b, c]\n",
        ),
        (
            "(.m[-1] | key) = \"q\"",
            TYPED,
            &[],
            "m:\n  0x1: hexint\n  1: one\n  \"2\": two\n  01: lz\n  q: neg\ns: [a, b, c]\n",
        ),
        (
            "(.m[01] | key) = \"q\"",
            TYPED,
            &[],
            "m:\n  0x1: hexint\n  1: one\n  \"2\": two\n  q: lz\n  -1: neg\ns: [a, b, c]\n",
        ),
        (
            "(.m | .[1] | key) = \"q\"",
            RENAME,
            &[],
            "m:\n  'q': six\n  2: two\n",
        ),
    ])
}

/// The rename over several documents with `eval-all`, each document renaming its own member and
/// creating the one it lacks. (`eval-all` loses a quoted key's quoting in any rename, `.m["1"]`
/// too, so the documents here quote nothing.)
#[test]
fn test_key_rename_through_an_integer_bracket_eval_all_8070() -> Result<()> {
    let input = "m:\n  1: a\n---\nm:\n  2: b\n  1: c\n";
    check(&[
        (
            "(.m[1] | key) = \"q\"",
            input,
            &["ea"],
            "m:\n  q: a\n---\nm:\n  2: b\n  q: c\n",
        ),
        (
            "(.m[2] | key) |= \"q\"",
            input,
            &["ea"],
            "m:\n  1: a\n  q: null\n---\nm:\n  q: b\n  1: c\n",
        ),
    ])
}

/// The new step is yq's alone: jq reads `01` as the number 1, so jq mode's `path` through an index
/// is unchanged (jq 1.7.1).
#[test]
fn test_jq_mode_integer_index_path_is_unchanged_8070() -> Result<()> {
    for (filter, want) in [
        ("[1,2,3] | path(.[01])", "[1]"),
        ("[1,2,3] | path(.[1])", "[1]"),
        ("{\"1\": 5} | path(.[\"1\"])", "[\"1\"]"),
        ("{\"a\":{\"1\":2}} | path(.a[\"1\"])", "[\"a\",\"1\"]"),
    ] {
        let (out, code) = run_jq_stdin(filter, "", &["-n", "-c"])?;
        assert_eq!((out.trim_end(), code), (want, 0), "{filter}");
    }
    Ok(())
}
