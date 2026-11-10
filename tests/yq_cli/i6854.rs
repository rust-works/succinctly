//! #6854: yq evaluates the body of `LHS |= BODY` against the target node itself, an assignment in it
//! changes that node in place, and the update takes the first output. So `.a |= (.x = 7, .y = 1)`
//! applies both assignments, where a functional evaluation keeps only the first. A body whose first
//! branch is a value (`5`, `.x`, `.x + 0`) keeps its first output, and so does one whose first branch
//! is a `select` that fails while a later branch is a value (`.a |= (select(.x == 2), 5)` is `5`). Every row is a live yq v4.53.3
//! capture (`yq -o=json -I=0`).

use anyhow::Result;

use crate::run_yq_stdin;

const JSON: &[&str] = &["-o=json", "-I=0"];

const K1: &str = "a: {x: 1, y: 2}\n";
const K3: &str = "a: {x: 1}\n";
const K2: &str = "a: {x: 1, y: 2}\nb: {x: 3, y: 4}\n";
/// Two mappings that merge one source.
const MM: &str = "base: &base {x: 1, y: 2}\nlist: [{<<: *base, i: 5}, {<<: *base, i: 6}]\n";

/// `(document, filter, yq's output)`.
const ROWS: &[(&str, &str, &str)] = &[
    (K1, ".a |= (.x = 7, .y = 1)", "{\"a\":{\"x\":7,\"y\":1}}\n"),
    (K1, ".a |= (.x = 7, .y = 1, .z = 3)", "{\"a\":{\"x\":7,\"y\":1,\"z\":3}}\n"),
    (K1, ".a |= (.x = 7, .x = .y)", "{\"a\":{\"x\":2,\"y\":2}}\n"),
    (K1, ".a |= (.x |= . + 1, .y |= . + 1)", "{\"a\":{\"x\":2,\"y\":3}}\n"),
    (K1, ".a |= (del(.x), .y = 5)", "{\"a\":{\"y\":5}}\n"),
    (K1, ".a |= ((.x = 7, .y = 1) | .z = 3)", "{\"a\":{\"x\":7,\"y\":1,\"z\":3}}\n"),
    (K1, ".a |= (select(.x == 1) | .y = 9, .z = 5)", "{\"a\":{\"x\":1,\"y\":9,\"z\":5}}\n"),
    (K1, ".a |= (select(.x == 2) | .y = 9, .z = 5)", "{\"a\":{\"x\":1,\"y\":2,\"z\":5}}\n"),
    (K1, ".a |= (select(.x == 1), .z = 5)", "{\"a\":{\"x\":1,\"y\":2,\"z\":5}}\n"),
    (K1, ".a |= (. , .z = 5)", "{\"a\":{\"x\":1,\"y\":2,\"z\":5}}\n"),
    (K1, ".a |= ((.x = 7, .y = 1), .z = 3)", "{\"a\":{\"x\":7,\"y\":1,\"z\":3}}\n"),
    (K1, ".a |= (.x = 7, (.y = 1, .z = 3))", "{\"a\":{\"x\":7,\"y\":1,\"z\":3}}\n"),
    (K1, ".a |= (.x = 7, .y = 1 | .z = 3)", "{\"a\":{\"x\":7,\"y\":1,\"z\":3}}\n"),
    (K1, ".[] |= (.x = 7, .y = 1)", "{\"a\":{\"x\":7,\"y\":1}}\n"),
    (K1, ".a |= (.x = 7, 5)", "{\"a\":{\"x\":7,\"y\":2}}\n"),
    (K1, ".a |= (.x = 7, .y)", "{\"a\":{\"x\":7,\"y\":2}}\n"),
    (K1, ".a |= (5, .x = 7)", "{\"a\":5}\n"),
    (K1, ".a |= (.x, .y = 5)", "{\"a\":1}\n"),
    (K1, ".a |= (.x + 0, .z = 5)", "{\"a\":1}\n"),
    (K1, ".a.x |= (. + 1, . + 10)", "{\"a\":{\"x\":2,\"y\":2}}\n"),
    (K2, "(.a, .b) |= (.x = 7, .y = 1)", "{\"a\":{\"x\":7,\"y\":1},\"b\":{\"x\":7,\"y\":1}}\n"),
    (MM, ".list[] |= (.x = 7, .y = 1)", "{\"base\":{\"x\":7,\"y\":1},\"list\":[{\"x\":7,\"y\":1,\"i\":5},{\"x\":7,\"y\":1,\"i\":6}]}\n"),
    (K3, ".a |= (select(.x == 2), 5)", "{\"a\":5}\n"),
    (K3, ".a |= (select(.x == 2), .y)", "{\"a\":null}\n"),
    (K3, ".a |= (select(.x == 2) | .y = 5, 7)", "{\"a\":7}\n"),
    (K3, ".a |= (select(.x == 2), .y = 5)", "{\"a\":{\"x\":1,\"y\":5}}\n"),
    (K3, ".a |= (select(.x == 1), .y = 5)", "{\"a\":{\"x\":1,\"y\":5}}\n"),
    (K3, ".a |= (. as $v | .y = $v.x, .x = 9)", "{\"a\":{\"x\":9,\"y\":1}}\n"),
    (K3, ".a |= ((. as $v | .y = $v.x), .x = 9)", "{\"a\":{\"x\":9,\"y\":1}}\n"),
    (K3, ".a |= (.x as $v | .z = $v, .y = 3)", "{\"a\":{\"x\":1,\"z\":1,\"y\":3}}\n"),
    (K3, ".a |= ((.x = 7, .y = 1) // 3)", "{\"a\":{\"x\":7,\"y\":1}}\n"),
    (K3, ".a |= (.x = 7, .y = 1) // 1", "{\"a\":{\"x\":7,\"y\":1}}\n"),
    (K3, ".a |= (5 | (. = 1, . = 2))", "{\"a\":2}\n"),
];

#[test]
fn test_every_assignment_in_a_comma_update_body_applies_6854() -> Result<()> {
    for (document, filter, want) in ROWS {
        let (stdout, code) = run_yq_stdin(filter, document, JSON)?;
        assert_eq!(
            (stdout.as_str(), code),
            (*want, 0),
            "{filter} on {document:?}"
        );
    }
    Ok(())
}

/// A branch after the first that raises still fails the update, as in yq (`Error: e`).
#[test]
fn test_a_later_branch_that_raises_still_fails_the_update_6854() -> Result<()> {
    let (stdout, code) = run_yq_stdin(".a |= (.x = 7, error(\"e\"))", K1, JSON)?;
    assert_eq!(stdout, "");
    assert_eq!(code, 1);
    Ok(())
}
