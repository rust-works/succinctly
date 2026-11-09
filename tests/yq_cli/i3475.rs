//! #3475: an array built from the document's own nodes (`[.users[] | select(.age > 25)]`,
//! `.users | [.[]]`) is printed from their cursors, not from an owned copy of the whole sequence.
//! It peaked at 256 MB against `map(select(...))`'s 38 MB over a 10 MB file, for the same output.
//!
//! The streamers write a sequence without the comments *between* its items, so a YAML document
//! with a comment, or a stream of several documents (whose error path they end with a `---`), keeps
//! the DOM route; JSON output has no comment to lose and always streams. Every answer was captured
//! from yq v4.53.3.

use anyhow::Result;

use crate::run_yq_stdin;

const USERS: &str =
    "users:\n  - id: 1\n    name: Al\n    age: 31\n  - id: 2\n    name: Bo\n    age: 20\n";

#[test]
fn test_array_of_selected_nodes_prints_as_those_nodes_3475() -> Result<()> {
    let (out, code) = run_yq_stdin("[.users[] | select(.age > 25)]", USERS, &[])?;
    assert_eq!(
        (out.as_str(), code),
        ("- id: 1\n  name: Al\n  age: 31\n", 0)
    );
    let (out, code) = run_yq_stdin("[.users[].age]", USERS, &[])?;
    assert_eq!((out.as_str(), code), ("- 31\n- 20\n", 0));
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
    let (out, code) = run_yq_stdin("[.users[].name]", USERS, &["-o=json", "-I=0"])?;
    assert_eq!((out.as_str(), code), ("[\"Al\",\"Bo\"]\n", 0));
    let (out, code) = run_yq_stdin(
        "[.users[] | select(.age > 25)]",
        USERS,
        &["-o=json", "-I=0"],
    )?;
    assert_eq!(
        (out.as_str(), code),
        ("[{\"id\":1,\"name\":\"Al\",\"age\":31}]\n", 0)
    );
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
fn test_an_error_after_the_first_document_prints_no_separator_3475() -> Result<()> {
    // The second document is a sequence, so `.a` fails there. The streamers print a `---` before
    // the failing document; yq prints the first document's array alone.
    let (out, code) = run_yq_stdin("[.a]", "a: [1]\n---\n- 1\n", &[])?;
    assert_eq!((out.as_str(), code), ("- [1]\n", 1));
    Ok(())
}

#[test]
fn test_an_array_of_nodes_keeps_a_merge_key_and_a_complex_key_3475() -> Result<()> {
    // yq writes a `<<` merge as the key it is, not as the entries it expands to.
    let merged = "base: &b {x: 1}\nitems:\n  - *b\n  - <<: *b\n    w: 4\n";
    let (out, code) = run_yq_stdin("[.items[]]", merged, &[])?;
    assert_eq!(
        (out.as_str(), code),
        ("- *b\n- !!merge <<: *b\n  w: 4\n", 0)
    );
    // A key that is itself a collection prints as `? key` / `: value`, not as an error.
    let (out, code) = run_yq_stdin("[.]", "? [a, b]\n: 1\n", &[])?;
    assert_eq!((out.as_str(), code), ("- ? [a, b]\n  : 1\n", 0));
    Ok(())
}

#[test]
fn test_an_array_of_nodes_is_all_or_nothing_3475() -> Result<()> {
    use crate::run_yq_stdin_with_stderr;
    // The streamers write a node as they read it. A scalar that fails to decode, or a complex key
    // that collides with another after its display form, is found only then: the array is atomic,
    // so each node is checked before the first byte is written.
    let bad_escape = "- ok\n- \"x\\qy\"\n";
    let colliding = "- a\n- ? [1]\n  : v1\n  ? [2]\n  : v2\n";
    for (doc, expected) in [
        (bad_escape, "invalid escape sequence"),
        (
            colliding,
            "is ambiguous: a complex or undecodable key's display form",
        ),
    ] {
        for filter in ["[.[]]", "[.[] | select(. != null)]", "[.[1]]"] {
            for args in [&[][..], &["-o=json", "-I=0"][..]] {
                let (out, err, code) = run_yq_stdin_with_stderr(filter, doc, args)?;
                if filter == "[.[1]]" && doc == colliding {
                    continue; // the one node is the colliding mapping: covered by `[.[]]`
                }
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
