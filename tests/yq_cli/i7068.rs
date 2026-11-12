//! #7068: yq's update-or-create idiom, `(.a // (.a = 0)) += 1`. yq evaluates an update's target in
//! the document it writes to, so the write in the last operand of a `//` target changes the left
//! operand's node, which `//` then tests again: the target is that node if it is truthy now, the
//! document otherwise. Every answer was captured from yq v4.53.3.

use anyhow::Result;

use crate::{run_yq_stdin, run_yq_stdin_with_stderr};

/// (input, filter, yq's stdout).
const ROWS: &[(&str, &str, &str)] = &[
    ("b: camel\n", "(.a // (.a = 0)) += 1", "b: camel\na: 1\n"),
    (
        "a: 3\nb: camel\n",
        "(.a // (.a = 0)) += 1",
        "a: 4\nb: camel\n",
    ),
    ("b: camel\n", "(.b // .a) += \"!\"", "b: camel!\n"),
    (
        "b: camel\n",
        "(.a // (.a = 0)) |= . + 1",
        "b: camel\na: 1\n",
    ),
    ("b: camel\n", "(.a // (.a |= 7)) += 1", "b: camel\na: 8\n"),
    (
        "b: camel\n",
        "(.a // (.a = 0, .b = 1)) += 1",
        "b: 1\na: 1\n",
    ),
    (
        "b: camel\n",
        "(.a // (.b | select(false))) = 1",
        "b: camel\na: 1\n",
    ),
    ("a: false\n", "(.a // (.a = 0)) += 1", "a: 1\n"),
    ("a: 0\n", "(.a // .z) = 1", "a: 1\n"),
    (
        "b: camel\n",
        "with(.k; (.a // (.a = 0)) += 1)",
        "b: camel\nk:\n  a: 1\n",
    ),
    (
        "k: {b: 1}\n",
        ".k |= ((.a // (.a = 0)) += 1)",
        "k: {b: 1, a: 1}\n",
    ),
    (
        "b: camel\n",
        "(.a.b // (.a.b = 0)) += 1",
        "b: camel\na:\n  b: 1\n",
    ),
    ("b: camel\n", "((.a // (.a = 0)) += 1) | .a", "1\n"),
    ("b: camel\n", "(.a // (.a |= 3)) = 9", "b: camel\na: 9\n"),
    (
        "l: [1]\n",
        "(.l[3] // (.l[3] = 0)) += 1",
        "l: [1, null, null, 1]\n",
    ),
    (
        "b: camel\n",
        "(.a // (.a = 0) // .z) += 1",
        "b: camel\na: 1\n",
    ),
    ("a: [1]\n", "(.a // (.a = [])) += [2]", "a: [1, 2]\n"),
    (
        "a: {x: 1}\n",
        "(.a // (.a = {})) *= {\"y\": 2}",
        "a: {x: 1, y: 2}\n",
    ),
    (
        "# head\nb: camel # line\n# foot\n",
        "(.a // (.a = 0)) += 1",
        "# head\nb: camel # line\na: 1\n# foot\n",
    ),
    (
        "x: &a {k: 1}\ny: *a\n",
        "(.x.k // (.x.k = 0)) += 1",
        "x: &a {k: 2}\ny: *a\n",
    ),
    (
        "b: 1\n---\na: 2\n",
        "(.a // (.a = 0)) += 1",
        "b: 1\na: 1\n---\na: 3\n",
    ),
    (
        "b: camel\n",
        "(.a // (.a = 0 | (.q // (.q = 5)) += 1)) += 1",
        "b: camel\na: 1\nq: 6\n",
    ),
    (
        "b: camel\n",
        "(.a // .z // (.a = 1)) += 1",
        "b: camel\na: 2\nz: null\n",
    ),
    (
        "k: {x: 5}\n",
        "with(.k; (.x as $v | .y = 1) | .z = 2)",
        "k: {x: 5, y: 1, z: 2}\n",
    ),
    (
        "k: {x: 5}\n",
        "with(.k; (.x as $v | .y = $v) | .z = 2)",
        "k: {x: 5, y: 5, z: 2}\n",
    ),
    (
        "b: camel\n",
        "(.a // (.a = 0 | (.b = 1, .c = 2))) += 1",
        "b: 1\na: 1\nc: 2\n",
    ),
    ("b: 1\n", "(.a? // (.a = 0)) += 1", "b: 1\na: 1\n"),
    (
        "k: {x: [1, 2], e: [], s: 5} # c\n",
        "with(.k; .e[] as $v | .y = $v)",
        "k: {x: [1, 2], e: [], s: 5, y: null} # c\n",
    ),
    (
        "k: {x: [1, 2], e: [], s: 5} # c\n",
        "with(.k; .nope as $v | .y = ($v | tostring))",
        "k: {x: [1, 2], e: [], s: 5, y: null} # c\n",
    ),
];

/// The write leaves the left operand falsy, so the target is the document, and adding `1` to a
/// mapping is an error, as in yq (`!!int () cannot be added to a !!map ()`).
const ERROR_ROWS: &[(&str, &str)] = &[
    ("b: camel\n", "(.a // (.c.d = 0)) += 1"),
    ("b: camel\n", "(.a // (.a = null)) += 1"),
    ("b: camel\n", "(.a // (.z += 1)) += 1"),
    ("b: camel\n", "(.a // ((.a // .c) = 0)) += 1"),
];

/// Recorded divergences (`docs/compliance/yq/limitations.md`): a target whose last operand does not
/// write does not create the left operand (yq leaves `a: null`), in value position the write is not
/// seen by the left operand, a comma behind a binding in the last operand targets the left node
/// once (twice in yq), so does a binding over several values there, a write that makes the document
/// `null` is not followed to the next operand, and `with`'s binding over several values keeps the first
/// value's writes (yq keeps each value's). (input, filter, succinctly's stdout; yq's is in the comment.)
const RESIDUAL_ROWS: &[(&str, &str, &str)] = &[
    // yq: "b: camel\na: null\nz: 1\n"
    ("b: camel\n", "(.a // .z) += 1", "b: camel\nz: 1\n"),
    // yq: "b: camel\na: null\n"
    ("b: camel\n", "(.a // \"x\") = 1", "b: camel\n"),
    // yq: "b: camel\na: null\nz: null\ny: 1\n"
    ("b: camel\n", "(.a // .z // .y) = 1", "b: camel\ny: 1\n"),
    // yq: "5\n"
    ("b: camel\n", ".a // (.a = 5)", "b: camel\na: 5\n"),
    // yq: "x: 1\na: 2\nb: 1\n"
    (
        "x: 1\n",
        "(.a // (.x as $v | .a = 0, .b = 1)) += 1",
        "x: 1\na: 1\nb: 1\n",
    ),
    // yq: "Error: Error: !!int () cannot be added to a  ()"
    ("b: 1\n", "(.a // (. = null) // .z) += 1", "z: 1\n"),
    // yq: "b: 1\nx: [1, 2]\na: 22\n"
    (
        "b: 1\nx: [1, 2]\n",
        "(.a // (.x[] as $v | .a = $v)) += 10",
        "b: 1\nx: [1, 2]\na: 11\n",
    ),
    // yq: "k: {x: [1, 2], e: [], s: 5, y: 2} # c\n"
    (
        "k: {x: [1, 2], e: [], s: 5} # c\n",
        "with(.k; .x[] as $v | .y = $v)",
        "k: {x: [1, 2], e: [], s: 5, y: 1} # c\n",
    ),
    // yq: "k: {x: [1, 2], e: [], s: 5, y: 3, z: 2} # c\n"
    (
        "k: {x: [1, 2], e: [], s: 5} # c\n",
        "with(.k; (.x[] as $v | .y += $v) | .z = 2)",
        "k: {x: [1, 2], e: [], s: 5, y: 1, z: 2} # c\n",
    ),
    // yq: "k: {x: [1, 2], e: [], s: 5, y: 2, w: 1} # c\n"
    (
        "k: {x: [1, 2], e: [], s: 5} # c\n",
        "with(.k; .x[] as $v | .y = $v, .w = 1)",
        "k: {x: [1, 2], e: [], s: 5, y: 1, w: 1} # c\n",
    ),
];

#[test]
fn the_update_or_create_idiom_writes_as_yq_does_7068() -> Result<()> {
    for (input, filter, expected) in ROWS.iter().chain(RESIDUAL_ROWS) {
        let (stdout, status) = run_yq_stdin(filter, input, &[])?;
        assert_eq!(
            (stdout.as_str(), status),
            (*expected, 0),
            "{filter} over {input:?}"
        );
    }
    Ok(())
}

#[test]
fn a_write_that_leaves_the_left_falsy_targets_the_document_7068() -> Result<()> {
    for (input, filter) in ERROR_ROWS {
        let (stdout, stderr, status) = run_yq_stdin_with_stderr(filter, input, &[])?;
        assert_eq!((stdout.as_str(), status), ("", 1), "{filter}");
        assert!(
            stderr.contains("cannot be added to a !!map"),
            "{filter}: {stderr}"
        );
    }
    Ok(())
}

/// The update's value is written once whatever the chain's length, so nesting the idiom in its own
/// value grows the parsed filter linearly (the first draft copied it per operand: eight levels
/// took more than 3 GB to parse).
#[test]
fn a_nested_idiom_parses_in_bounded_memory_7068() -> Result<()> {
    let mut filter = String::from("1");
    for _ in 0..8 {
        filter = format!("(.a // .b // .c // .d // .e // .f // (.g = 0)) |= ({filter})");
    }
    let started = std::time::Instant::now();
    let (stdout, status) = run_yq_stdin(&filter, "b: 1\n", &[])?;
    assert_eq!((stdout.as_str(), status), ("b: 1\na: null\n", 0));
    // A copy per operand took seconds and gigabytes at this depth; linear growth takes milliseconds.
    assert!(started.elapsed() < std::time::Duration::from_secs(10));
    Ok(())
}

/// The idiom nested in its own last operand, and a chain of bindings in `with`, also grow the parsed
/// filter linearly: a binding's body is written once (a draft wrote it twice, and either shape took
/// more than 3 GB to parse at about twenty levels).
#[test]
fn a_nested_last_operand_and_chained_bindings_parse_in_bounded_memory_7068() -> Result<()> {
    let started = std::time::Instant::now();
    let mut filter = String::from("(.z = 0)");
    for depth in 0..22 {
        filter = format!("((.a{depth} // {filter}) += 1)");
    }
    // `.z = 0` leaves `a0` falsy, so the innermost update adds `1` to the document, as in yq.
    let (stdout, stderr, status) = run_yq_stdin_with_stderr(&filter, "b: 1\n", &[])?;
    assert_eq!((stdout.as_str(), status), ("", 1));
    assert!(stderr.contains("cannot be added to a !!map"), "{stderr}");
    let bindings: Vec<String> = (0..20).map(|i| format!(".x as $v{i}")).collect();
    let filter = format!("with(.k; {} | .y = 1)", bindings.join(" | "));
    let (stdout, status) = run_yq_stdin(&filter, "k: {x: 1}\n", &[])?;
    assert_eq!((stdout.as_str(), status), ("k: {x: 1, y: 1}\n", 0));
    // Doubling the body at each binding took gigabytes; linear growth takes milliseconds.
    assert!(started.elapsed() < std::time::Duration::from_secs(10));
    Ok(())
}
