//! #5103: a register spelled through a slice that spans the whole of a non-empty array (`.a[0:]`), with `$x` bound at
//! its container, or the other way round.

use anyhow::Result;

use crate::{assert_path_rows_both_routes_3749, run_jq_full};

// jq's array slice is a view (`jvp_array_slice` moves the offset and size over the shared buffer) and `jv_identical`
// compares pointer, offset and size, so `.a[0:]` of a non-empty array *is* `.a`: `$x` bound at one is the register
// standing at the other, its destructure passes, and jq's later answer or error follows. The slice of an empty array
// is a fresh `[]`, so there `$x` is provably another node and its refusal is jq's catchable one. #5095 read every
// slice as possibly the register and refused uncatchably; every row below exited 5.

/// #5103: the issue's rows and their `first(.a,.b)`/`(.a,.b)` binds, on a non-empty array (`$x` is the register) and an
/// empty one (`$x` is not), under `try`, `?` and bare. Every row captured from `/usr/bin/jq` 1.7.1, both routes.
#[test]
fn test_5103_register_through_a_whole_slice_with_x_at_its_container() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":[{"b":1},{"b":1}]}"#,
            r#"try path(.a as $x | .a[0:] | foreach ($x as [$q] | .) as $y (.; .a?; 1)) catch "c""#,
            "\"c\"\n",
            "",
            0,
        ),
        (
            r#"{"a":[{"b":1},{"b":1}]}"#,
            r"[path(.a as $x | .a[0:] | foreach ($x as [$q] | .) as $y (.; .a?; 1))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":[{"b":1},{"b":1}]}"#,
            r#"try path(.a[0:] as $x | .a | foreach ($x as [$q] | .) as $y (.; .a?; 1)) catch "c""#,
            "\"c\"\n",
            "",
            0,
        ),
        (
            r#"{"a":[{"b":1},{"b":1}]}"#,
            r"[path(.a[0:] as $x | .a | foreach ($x as [$q] | .) as $y (.; .a?; 1))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":[{"b":1},{"b":1}]}"#,
            r#"try path(first(.a,.b) as $x | .a[0:] | foreach ($x as [$q] | .) as $y (.; .a?; 1)) catch "c""#,
            "\"c\"\n",
            "",
            0,
        ),
        (
            r#"{"a":[{"b":1},{"b":1}]}"#,
            r"[path(first(.a,.b) as $x | .a[0:] | foreach ($x as [$q] | .) as $y (.; .a?; 1))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":[{"b":1},{"b":1}]}"#,
            r#"try path((.a,.b) as $x | .a[0:] | foreach ($x as [$q] | .) as $y (.; .a?; 1)) catch "c""#,
            "\"c\"\n",
            "",
            0,
        ),
        (
            r#"{"a":[{"b":1},{"b":1}]}"#,
            r"[path((.a,.b) as $x | .a[0:] | foreach ($x as [$q] | .) as $y (.; .a?; 1))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":[{"b":1},{"b":1}]}"#,
            r"path(.a as $x | .a[0:] | foreach ($x as [$q] | .) as $y (.; .a?; 1))",
            "",
            "Invalid path expression near attempt to access element \"a\" of [{\"b\":1},{\"b\":1}]",
            5,
        ),
        (
            r#"{"a":[],"b":[]}"#,
            r#"try path(.a as $x | .a[0:] | foreach ($x as [$q] | .) as $y (.; .a?; 1)) catch "c""#,
            "\"c\"\n",
            "",
            0,
        ),
        (
            r#"{"a":[],"b":[]}"#,
            r"[path(.a as $x | .a[0:] | foreach ($x as [$q] | .) as $y (.; .a?; 1))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":[],"b":[]}"#,
            r#"try path(.a[0:] as $x | .a | foreach ($x as [$q] | .) as $y (.; .a?; 1)) catch "c""#,
            "\"c\"\n",
            "",
            0,
        ),
        (
            r#"{"a":[],"b":[]}"#,
            r"[path(.a[0:] as $x | .a | foreach ($x as [$q] | .) as $y (.; .a?; 1))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":[],"b":[]}"#,
            r#"try path(first(.a,.b) as $x | .a[0:] | foreach ($x as [$q] | .) as $y (.; .a?; 1)) catch "c""#,
            "\"c\"\n",
            "",
            0,
        ),
        (
            r#"{"a":[],"b":[]}"#,
            r"[path(first(.a,.b) as $x | .a[0:] | foreach ($x as [$q] | .) as $y (.; .a?; 1))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":[],"b":[]}"#,
            r#"try path((.a,.b) as $x | .a[0:] | foreach ($x as [$q] | .) as $y (.; .a?; 1)) catch "c""#,
            "\"c\"\n",
            "",
            0,
        ),
        (
            r#"{"a":[],"b":[]}"#,
            r"[path((.a,.b) as $x | .a[0:] | foreach ($x as [$q] | .) as $y (.; .a?; 1))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":[],"b":[]}"#,
            r"path(.a as $x | .a[0:] | foreach ($x as [$q] | .) as $y (.; .a?; 1))",
            "",
            "Invalid path expression near attempt to access element 0 of []",
            5,
        ),
    ])
}

/// #5103: the slice-spelled register answers in jq's own spelling (`["a",{"start":0,"end":null}]`), a `del` through it
/// deletes the slice, an assignment raises jq's slice error, and an equal-valued sibling bound as `$x` (`(.a,.b)`) is
/// provably another node, so its refusal is caught.
#[test]
fn test_5103_whole_slice_names_the_node_it_slices() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":[{"b":1},{"b":1}]}"#,
            r"path(.a as $x | .a[0:] | $x)",
            "[\"a\",{\"start\":0,\"end\":null}]\n",
            "",
            0,
        ),
        (
            r#"{"a":[{"b":1},{"b":1}]}"#,
            r"path(.a[0:] as $x | .a | $x)",
            "[\"a\"]\n",
            "",
            0,
        ),
        (
            r#"{"a":[{"b":1},{"b":1}]}"#,
            r"path(.a as $x | .a[0:] | $x | .[0])",
            "[\"a\",{\"start\":0,\"end\":null},0]\n",
            "",
            0,
        ),
        (
            r#"{"a":[{"b":1},{"b":1}]}"#,
            r"path(.a as $x | .a[-9:] | $x)",
            "[\"a\",{\"start\":-9,\"end\":null}]\n",
            "",
            0,
        ),
        (
            r#"{"a":[{"b":1},{"b":1}]}"#,
            r"path(.a[1:] as $x | .a[1:][0:] | $x)",
            "[\"a\",{\"start\":1,\"end\":null},{\"start\":0,\"end\":null}]\n",
            "",
            0,
        ),
        (
            r#"{"a":[{"b":1},{"b":1}]}"#,
            r"del(.a as $x | .a[0:] | $x)",
            "{\"a\":[]}\n",
            "",
            0,
        ),
        (
            r#"{"a":[{"b":1},{"b":1}]}"#,
            r"(.a as $x | .a[0:] | $x) |= 9",
            "",
            "A slice of an array can only be assigned another array",
            5,
        ),
        (
            r#"{"a":[{"b":1},{"b":1}]}"#,
            r"path(.a as $x | .a[0:] | ($x as [$q] | .) | .[1])",
            "",
            "Invalid path expression near attempt to access element 1 of [{\"b\":1},{\"b\":1}]",
            5,
        ),
        (
            r#"{"a":[[1]],"b":[[1]]}"#,
            r#"try path((.a,.b) as $x | .a | .[0:] | $x as [$q] | $q) catch "C""#,
            "[\"a\",{\"start\":0,\"end\":null},0]\n\"C\"\n",
            "",
            0,
        ),
        (
            r#"{"a":[1],"b":[1]}"#,
            r#"try path((.a,.b) as $x | .a[-9:] | reduce ($x as [$q] | .) as $y (.; .)) catch "C""#,
            "[\"a\",{\"start\":-9,\"end\":null}]\n\"C\"\n",
            "",
            0,
        ),
    ])
}

/// #5103: a slice that does not span the whole array (`[1:]`, `[0:0]`), the slice of an empty array and a string slice
/// are not the node they slice, so jq refuses (or `?` empties) and so does this.
#[test]
fn test_5103_partial_empty_and_string_slices_stay_other_nodes() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":[{"b":1},{"b":1}]}"#,
            r"path(.a as $x | .a[1:] | $x)",
            "",
            "Invalid path expression with result [{\"b\":1},{\"b\":1}]",
            5,
        ),
        (
            r#"{"a":[{"b":1},{"b":1}]}"#,
            r"path(.a as $x | .a[0:][1:] | $x)",
            "",
            "Invalid path expression with result [{\"b\":1},{\"b\":1}]",
            5,
        ),
        (
            r#"{"a":[{"b":1},{"b":1}]}"#,
            r"[path(.a as $x | .a[0:0] | $x)?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":[]}"#,
            r"path(.a as $x | .a[0:] | $x)",
            "",
            "Invalid path expression with result []",
            5,
        ),
        (
            r#"{"a":[]}"#,
            r"path(.a[0:] as $x | .a[0:] | $x)",
            "",
            "Invalid path expression with result []",
            5,
        ),
        (
            r#"{"a":"ab"}"#,
            r"path(.a as $x | .a[0:] | $x)",
            "",
            "Invalid path expression with result \"ab\"",
            5,
        ),
    ])
}

/// #5103 review: jq copies a slice whose offset reaches 65,536 (`jv`'s `offset` is 16 bits), so each evaluation of it
/// is a new array: the first build of this fix folded `.a[70000:][0:]` onto `.a[70000:]` and `del` removed three
/// elements where jq raises, and `main` deleted through `.a[65536:] as $x | .a[65536:]`. One evaluation bound as `. as
/// $x` is still the copy the register stands on. Captured from `/usr/bin/jq` 1.7.1 on a 70,003-element array, on the
/// `-n` route (the document built in the filter) and on stdin.
#[cfg(not(feature = "unshared-containers"))]
#[test]
fn test_5103_copied_slice_past_offset_65535_is_a_new_array_per_evaluation() -> Result<()> {
    // (filter, stdout, first stderr line, exit code)
    let rows: &[(&str, &str, &str, i32)] = &[
        (
            r"del(.a[70000:] as $x | .a[70000:][0:] | $x) | .a | length",
            "",
            "Invalid path expression with result [70000,70001,70002]",
            5,
        ),
        (
            r#"try path(.a[70000:] as $x | .a[70000:][0:] | $x) catch "c""#,
            "\"c\"\n",
            "",
            0,
        ),
        (
            r"del(.a[65536:] as $x | .a[65536:] | $x) | .a | length",
            "",
            "Invalid path expression with result [65536,65537,65538,65539",
            5,
        ),
        (
            r"[path(.a[65536:] as $x | .a[65536:] | $x)?]",
            "[]\n",
            "",
            0,
        ),
        (
            r"path(.a[70000:] | . as $x | .[0:] | $x)",
            "[\"a\",{\"start\":70000,\"end\":null},{\"start\":0,\"end\":null}]\n",
            "",
            0,
        ),
        (
            r"[path(.a[70000:] | . as $x | 1 | $x as [$q] | $q)?]",
            "[[\"a\",{\"start\":70000,\"end\":null},0]]\n",
            "",
            0,
        ),
        // jq answers (a view at offset 65,535): #5118's gate refused it, the path-mode interpreter (#5223) shares the view.
        (
            r"path(.a[65535:] as $x | .a[65535:] | $x)",
            "[\"a\",{\"start\":65535,\"end\":null}]\n",
            "",
            0,
        ),
    ];
    let document = big_document_5103();
    for &(filter, stdout, message, exit) in rows {
        for (out, err, code) in big_document_routes_5103(&document, filter)? {
            assert_eq!(
                (out.as_str(), code),
                (stdout, exit),
                "`{filter}`: stderr {err:?}"
            );
            if message.is_empty() {
                assert!(err.is_empty(), "`{filter}`: unexpected stderr {err:?}");
            } else {
                assert!(
                    err.contains(message),
                    "`{filter}`: stderr {err:?} lacks {message:?}"
                );
            }
        }
    }
    Ok(())
}

/// #5103 review: a copied slice spelled alike on both sides may be one evaluation (`.a[70000:] | .[0:] as $x` binds the
/// copy the register stands on: jq answers) or two (`.a[70000:] as $x | .a[70000:]`: jq's catchable raise), which
/// positions cannot tell apart, so these stay refused: never a caught refusal that drops jq's answer, nor an answer
/// where jq raises. Both routes.
#[test]
fn test_5103_copied_slice_spelled_alike_stays_refused() -> Result<()> {
    // (filter, jq's stdout)
    let rows: &[(&str, &str)] = &[
        (
            r"[path(.a[70000:] as $x | .a[70000:] | $x as [$q] | .)?]",
            "[]\n",
        ),
        (
            r"[path(.a[70000:] | .[0:] as $x | 1 | $x as [$q] | $q)?]",
            "[[\"a\",{\"start\":70000,\"end\":null},0]]\n",
        ),
        (
            r#"try path(.a[65536:] as $x | .a[65536:] | $x as [$q] | .) catch "c""#,
            "\"c\"\n",
        ),
    ];
    let document = big_document_5103();
    for &(filter, jq_stdout) in rows {
        for (out, err, code) in big_document_routes_5103(&document, filter)? {
            assert!(
                (code == 5 && out.is_empty()) || (out == jq_stdout && code == 0 && err.is_empty()),
                "`{filter}`: neither jq's {jq_stdout:?} nor a refusal: {out:?} {err:?} exit {code}"
            );
        }
    }
    Ok(())
}

/// `{"a":[0, 1, ..., 70002]}`, long enough for a slice past offset 65,535.
fn big_document_5103() -> String {
    let items: Vec<String> = (0..70_003).map(|i| i.to_string()).collect();
    format!("{{\"a\":[{}]}}", items.join(","))
}

/// `filter` run on `document` from stdin and on the `-n` route with the same document built in the filter.
fn big_document_routes_5103(document: &str, filter: &str) -> Result<[(String, String, i32); 2]> {
    Ok([
        run_jq_full(&["-c", filter], Some(document))?,
        run_jq_full(
            &["-nc", &format!("{{\"a\":[range(70003)]}} | {filter}")],
            None,
        )?,
    ])
}
