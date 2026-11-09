//! #3475: an array of the document's own nodes (`[.users[] | select(.age > 30)]`, `.users | [.[]]`,
//! `[.users]`) is printed from their cursors, not from an owned copy of the whole sequence. It
//! peaked at 2,350 MB against `map(select(...))`'s 260 MB over a 100 MB file, for the same output.
//!
//! The route is narrow on purpose, and most rows here pin what it must *not* change, each checked
//! against the previous binary and yq v4.53.3:
//!
//! - an array with an owned element (a missing key after a fan-out, a `sort_by` prefix) keeps the
//!   DOM route's spelling of block scalars and floats;
//! - a YAML document with a comment or an alias, and any `--sort-keys` run, keeps the DOM route (the
//!   streamers write a sequence without the comments between its items, write an `*alias` whose
//!   anchor the array does not hold, and sort flow mappings);
//! - a node that fails to decode, or a complex key that collides with another, is found before the
//!   first byte is written, and a failing document writes no `---`.
//!
//! The collision rows pin succinctly's recorded refusal of such keys (yq prints them).

use std::process::{Command, Stdio};

use anyhow::Result;
use tempfile::TempDir;

use crate::{run_yq_stdin, run_yq_stdin_with_stderr};

const USERS: &str =
    "users:\n  - id: 1\n    name: Al\n    age: 31\n  - id: 2\n    name: Bo\n    age: 20\n";

#[test]
fn test_array_of_selected_nodes_prints_as_those_nodes_3475() -> Result<()> {
    let (out, code) = run_yq_stdin("[.users[] | select(.age > 25)]", USERS, &[])?;
    assert_eq!(
        (out.as_str(), code),
        ("- id: 1\n  name: Al\n  age: 31\n", 0)
    );
    // One navigation is a node, or a lone `null` where the path is missing.
    let (out, code) = run_yq_stdin("[.users[0].age]", USERS, &[])?;
    assert_eq!((out.as_str(), code), ("- 31\n", 0));
    let (out, code) = run_yq_stdin("[.users[0].missing]", USERS, &[])?;
    assert_eq!((out.as_str(), code), ("- null\n", 0));
    Ok(())
}

#[test]
fn test_array_of_a_navigation_prints_as_map_of_it_3475() -> Result<()> {
    // The same sequence, by the two routes that can build it.
    for (array, map) in [
        (".users | [.[]]", ".users | map(.)"),
        ("[.users[]]", ".users | map(.)"),
        (
            ".users | [.[] | select(.age > 25)]",
            ".users | map(select(.age > 25))",
        ),
    ] {
        let (a, a_code) = run_yq_stdin(array, USERS, &[])?;
        let (m, m_code) = run_yq_stdin(map, USERS, &[])?;
        assert_eq!((a_code, m_code), (0, 0), "{array}");
        assert_eq!(a, m, "{array} against {map}");
    }
    Ok(())
}

#[test]
fn test_json_output_of_an_array_of_nodes_is_unchanged_3475() -> Result<()> {
    let (out, code) = run_yq_stdin(
        "[.users[] | select(.age > 25)]",
        USERS,
        &["-o=json", "-I=0"],
    )?;
    assert_eq!(
        (out.as_str(), code),
        ("[{\"id\":1,\"name\":\"Al\",\"age\":31}]\n", 0)
    );
    // A JSON-sourced float keeps yq's spelling.
    let doc = "[{\"x\":1e-7},{\"x\":1.5e30},{\"y\":1},{\"x\":1e20}]";
    let args = ["-p=json", "-o=json", "-I=0"];
    let (out, code) = run_yq_stdin("[.[] | .x]", doc, &args)?;
    assert_eq!((out.as_str(), code), ("[1e-07,1.5e+30,null,1e+20]\n", 0));
    let (out, code) = run_yq_stdin(".[] | [.x]", doc, &args)?;
    assert_eq!(
        (out.as_str(), code),
        ("[1e-07]\n[1.5e+30]\n[null]\n[1e+20]\n", 0)
    );
    Ok(())
}

#[test]
fn test_an_array_with_an_owned_element_keeps_the_dom_spelling_3475() -> Result<()> {
    // `.a` is missing from the second element, so the array holds an owned `null` beside a node.
    // The streamers would write the block scalar as a quoted string; yq keeps the `|` block.
    let doc = "- a: |\n    multi\n    line\n- b: 2\n";
    for filter in ["[.[] | .a]", ".[] | [.a]"] {
        let (out, code) = run_yq_stdin(filter, doc, &[])?;
        assert_eq!(
            (out.as_str(), code),
            ("- |\n  multi\n  line\n- null\n", 0),
            "{filter}"
        );
    }
    Ok(())
}

#[test]
fn test_a_comment_between_items_keeps_the_array_on_the_dom_route_3475() -> Result<()> {
    // The streamers drop `# between`, as they do for `map(.)`; yq keeps it.
    let commented = "# head\n- a: 1\n# between\n- a: 3\n# foot\n";
    let (out, code) = run_yq_stdin("[.[]]", commented, &[])?;
    assert_eq!((out.as_str(), code), ("- a: 1\n# between\n- a: 3\n", 0));

    let users = "users:\n  - id: 1\n    name: Al # c\n    age: 31\n    # between\n  - id: 2\n    name: Bo\n    age: 20\n";
    let (out, code) = run_yq_stdin("[.users[]]", users, &[])?;
    assert_eq!(
        (out.as_str(), code),
        (
            "- id: 1\n  name: Al # c\n  age: 31\n  # between\n- id: 2\n  name: Bo\n  age: 20\n",
            0
        )
    );
    // JSON output has no comment to lose.
    let (out, code) = run_yq_stdin("[.[]]", commented, &["-o=json", "-I=0"])?;
    assert_eq!((out.as_str(), code), ("[{\"a\":1},{\"a\":3}]\n", 0));
    Ok(())
}

#[test]
fn test_an_alias_whose_anchor_is_not_in_the_array_is_expanded_3475() -> Result<()> {
    // yq writes `- *b` with no `&b` anywhere and then cannot read it back; succinctly never emits
    // YAML it cannot read back (CLAUDE.md, ADR-0018 rule 4(a)), so the DOM route expands it.
    let doc = "base: &b {x: 1}\nitems:\n  - *b\n  - y: 2\n";
    let (out, code) = run_yq_stdin("[.items[]]", doc, &[])?;
    assert_eq!((out.as_str(), code), ("- x: 1\n- y: 2\n", 0));
    let compose = "x-common: &common {restart: always}\nservices:\n  web: {<<: *common, image: nginx}\n  db: {image: pg}\n";
    let (out, code) = run_yq_stdin("[.services[]]", compose, &[])?;
    assert_eq!(code, 0);
    assert!(
        !out.contains("<<") && out.contains("restart: always"),
        "{out:?}"
    );
    Ok(())
}

#[test]
fn test_sort_keys_keeps_the_dom_route_for_an_array_of_nodes_3475() -> Result<()> {
    // The streamers sort a flow mapping and the DOM route does not, so the answer must not depend
    // on whether the document happens to have a comment.
    let plain = "- {b: 1, a: 2}\n";
    let commented = "# c\n- {b: 1, a: 2}\n";
    for doc in [plain, commented] {
        let (out, code) = run_yq_stdin("[.[]]", doc, &["-S"])?;
        assert_eq!((out.as_str(), code), ("- {b: 1, a: 2}\n", 0), "{doc:?}");
    }
    Ok(())
}

#[test]
fn test_an_error_after_the_first_document_prints_no_separator_3475() -> Result<()> {
    // The second document is a sequence, so `.a` fails there. A failing document writes no `---`
    // ahead of its diagnostic, so yq's answer is the first document's array alone.
    let (out, code) = run_yq_stdin("[.a]", "a: [1]\n---\n- 1\n", &[])?;
    assert_eq!((out.as_str(), code), ("- [1]\n", 1));
    Ok(())
}

#[test]
fn test_a_failing_file_among_several_writes_no_separator_3475() -> Result<()> {
    let dir = TempDir::new()?;
    std::fs::write(dir.path().join("f1.yaml"), "a: [1]\n")?;
    std::fs::write(dir.path().join("f2.yaml"), "- 1\n")?;
    std::fs::write(dir.path().join("f3.yaml"), "a: \"x\\qy\"\n")?;
    std::fs::write(dir.path().join("f4.yaml"), "a: [3]\n")?;
    let run = |files: &[&str]| -> Result<(String, i32)> {
        let output = Command::new(env!("CARGO_BIN_EXE_succinctly"))
            .arg("yq")
            .arg("[.a]")
            .args(files)
            .current_dir(dir.path())
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()?;
        Ok((
            String::from_utf8(output.stdout)?,
            output.status.code().unwrap_or(-1),
        ))
    };
    // A wrong-shaped document (the error is the query's) and an undecodable one (the error is the
    // array's own check) each print the diagnostic and nothing else.
    assert_eq!(run(&["f1.yaml", "f2.yaml"])?, ("- [1]\n".to_string(), 1));
    assert_eq!(run(&["f1.yaml", "f3.yaml"])?, ("- [1]\n".to_string(), 1));
    // The run goes on past a failing file, as for any other query, and the failed one leaves no
    // empty document between the others.
    assert_eq!(
        run(&["f1.yaml", "f3.yaml", "f4.yaml"])?,
        ("- [1]\n---\n- [3]\n".to_string(), 1)
    );
    Ok(())
}

#[test]
fn test_multi_value_json_with_yaml_output_keeps_its_spelling_3475() -> Result<()> {
    // Two top-level JSON values are two documents. Each is JSON-sourced, so `1.50` is `1.5` and
    // `2e2` is `200`, as for any other query.
    let doc = "{\"a\":1.50,\"b\":[1e2]}\n{\"a\":2e2,\"b\":[]}\n";
    let (out, code) = run_yq_stdin("[.a]", doc, &["-p=json"])?;
    assert_eq!((out.as_str(), code), ("- 1.5\n---\n- 200\n", 0));
    Ok(())
}

#[test]
fn test_an_array_of_nodes_prints_a_collection_key_as_its_key_3475() -> Result<()> {
    // A key that is itself a collection prints as `? key` / `: value`, not as `'': value`.
    let (out, code) = run_yq_stdin("[.]", "? [a, b]\n: 1\n", &[])?;
    assert_eq!((out.as_str(), code), ("- ? [a, b]\n  : 1\n", 0));
    Ok(())
}

#[test]
fn test_an_array_of_nodes_is_all_or_nothing_3475() -> Result<()> {
    // The streamers write a node as they read it. A scalar that fails to decode, or a complex key
    // that collides with another after its display form, is found only then: the array is atomic,
    // so each node is checked before the first byte is written.
    let bad_escape = "- ok\n- \"x\\qy\"\n";
    let colliding = "- a\n- ? [1]\n  : v1\n  ? [2]\n  : v2\n";
    for (doc, filters, expected) in [
        (
            bad_escape,
            // `select(true)` reads nothing, so the failure is the array's own check.
            &["[.[]]", "[.[] | select(true)]", "[.[1]]"][..],
            "invalid escape sequence",
        ),
        (
            colliding,
            &["[.[]]", "[.[] | select(true)]", "[.[1]]"][..],
            "is ambiguous: a complex or undecodable key's display form",
        ),
    ] {
        for filter in filters {
            for args in [&[][..], &["-o=json", "-I=0"][..]] {
                let (out, err, code) = run_yq_stdin_with_stderr(filter, doc, args)?;
                assert_eq!(out, "", "`{filter}` {args:?} on {doc:?}: stdout");
                assert_eq!(code, 1, "`{filter}` {args:?} on {doc:?}");
                assert!(
                    err.contains(expected),
                    "`{filter}` {args:?} on {doc:?}: stderr {err:?}"
                );
            }
        }
    }
    Ok(())
}

#[test]
fn test_an_array_of_nodes_honours_the_output_flags_3475() -> Result<()> {
    // `-e`: an empty array and an array of one `null` are both values, so the exit status is 0.
    let (out, code) = run_yq_stdin("[.users[] | select(.age > 99)]", USERS, &["-e"])?;
    assert_eq!((out.as_str(), code), ("[]\n", 0));
    let (out, code) = run_yq_stdin("[.users[0].missing]", USERS, &["-e"])?;
    assert_eq!((out.as_str(), code), ("- null\n", 0));
    // `-0` ends each result with a NUL.
    let (out, code) = run_yq_stdin("[.users[0].age]", USERS, &["-0"])?;
    assert_eq!((out.as_str(), code), ("- 31\0", 0));
    // `--doc N` selects one document of several.
    let (out, code) = run_yq_stdin(
        "[.a]",
        "a: [1]\n---\na: [2]\n---\na: [3]\n",
        &["--doc", "1"],
    )?;
    assert_eq!((out.as_str(), code), ("- [2]\n", 0));
    // `-C` colours it, and the colours leave the text as it was.
    let (plain, _) = run_yq_stdin("[.users[0]]", USERS, &[])?;
    let (coloured, code) = run_yq_stdin("[.users[0]]", USERS, &["-C"])?;
    assert_eq!(code, 0);
    assert!(coloured.contains('\u{1b}'), "{coloured:?}");
    let mut stripped = String::new();
    let mut chars = coloured.chars();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            for skipped in chars.by_ref() {
                if skipped == 'm' {
                    break;
                }
            }
        } else {
            stripped.push(c);
        }
    }
    assert_eq!(stripped, plain);
    Ok(())
}

#[test]
fn test_an_array_of_nodes_keeps_tags_and_anchors_3475() -> Result<()> {
    // The nodes are written from the source, so a tag and an anchor with no alias stay as written
    // (an alias takes the DOM route, `test_an_alias_whose_anchor_is_not_in_the_array_is_expanded_3475`).
    let doc = "- !!binary aGk=\n- &x v\n- !foo bar\n- !!str 12\n";
    let (out, code) = run_yq_stdin("[.[]]", doc, &[])?;
    assert_eq!((out.as_str(), code), (doc, 0));
    Ok(())
}
