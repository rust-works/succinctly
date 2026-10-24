//! #5044: the `if C then FE else B end` reading of #5034 (`FE = foreach (try ($x as {a:$a} | .)) as $y (.; .; .)`) in a
//! `reduce` UPDATE, and the mirrored `if C then B else FE end`.

use crate::assert_path_rows_both_routes_3749;
use anyhow::Result;

/// #5044: in a `reduce` UPDATE, `if . then FE else .a end` is empty with `FE` as it is in a `foreach` UPDATE (#5034):
/// the first step of `(.a,.b)` has `$x` off the register, `FE`'s pattern step raises and its `try` catches it, so the
/// state is `null`, and the second step takes the `else` and indexes it (`near attempt to access element "a" of
/// null`, exit 5). The `reduce` was driven by value, emitted, and the state stayed the root (`path` printed `[]`).
/// Every row captured from jq 1.7.1, both routes.
#[test]
fn test_reduce_update_if_with_a_navigating_else_empties_with_the_caught_foreach_5044() -> Result<()>
{
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":1,"b":2}"#,
            r"path(. as $x | reduce (.a,.b) as $i (.; if . then foreach (try ($x as {a:$a} | .)) as $y (.; .; .) else .a end))",
            "",
            r#"Invalid path expression near attempt to access element "a""#,
            5,
        ),
        (
            r#"{"a":1,"b":2}"#,
            r"[path(. as $x | reduce (.a,.b) as $i (.; if . then foreach (try ($x as {a:$a} | .)) as $y (.; .; .) else .a end))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":1,"b":2}"#,
            r"del(. as $x | reduce (.a,.b) as $i (.; if . then foreach (try ($x as {a:$a} | .)) as $y (.; .; .) else .a end))",
            "",
            r#"Invalid path expression near attempt to access element "a""#,
            5,
        ),
        (
            r#"{"a":1,"b":2}"#,
            r"(. as $x | reduce (.a,.b) as $i (.; if . then foreach (try ($x as {a:$a} | .)) as $y (.; .; .) else .a end)) |= 9",
            "",
            r#"Invalid path expression near attempt to access element "a""#,
            5,
        ),
        (
            r#"{"a":1,"b":2}"#,
            r"path(. as $x | reduce (.,.a) as $i (.; if .a == 1 then foreach (try ($x as {a:$a} | .)) as $y (.; .; .) else .b end))",
            "",
            r"Invalid path expression with result",
            5,
        ),
        (
            r#"{"a":null,"b":2}"#,
            r"[path(. as $x | reduce (.a,.b) as $i (.; if . then foreach (try ($x as {a:$a} | .)) as $y (.; .; .) else .a end))?]",
            "[]\n",
            "",
            0,
        ),
    ])
}

/// #5044: the mirrored form, the `foreach` in the `else` (or in an `elif` chain), in a `reduce` UPDATE: the source is
/// driven only on a step that took the `else`, so the UPDATE is empty with the `foreach` there too. Refused (`"C"`
/// under a `try`) or answered `[]` before. Every row captured from jq 1.7.1, both routes.
#[test]
fn test_reduce_update_mirrored_if_with_the_caught_foreach_in_the_else_5044() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":null,"b":2}"#,
            r#"try path(. as $x | reduce (.) as $i (.; if false then .a else foreach (try ($x as {a:$a} | .)) as $y (.; .; .) end)) catch "C""#,
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":false,"b":null}"#,
            r#"try path(. as $x | reduce (.) as $i (.; if .a == 1 then .a else foreach ((. as {a:$a} | .)?) as $y (.; .; .) end)) catch "C""#,
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":1}"#,
            r#"try ((. as $x | reduce (1,2) as $i (.; if true then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | .) else 1 end)) |= 9) catch "C""#,
            "9\n",
            "",
            0,
        ),
        (
            r#"{"a":[1],"b":0}"#,
            r#"try ((. as $x | reduce (.) as $i (.; if .a then foreach ($x as {a:$a} | .) as $y (.; .; .) else .b end)) |= 9) catch "C""#,
            "9\n",
            "",
            0,
        ),
        (
            r#"{"a":1,"b":2}"#,
            r"path(. as $x | reduce (.a,.b) as $i (.; if .a then .b else foreach (try ($x as {a:$a} | .)) as $y (.; .; .) end))",
            "",
            r#"Invalid path expression near attempt to access element "b""#,
            5,
        ),
        (
            r#"{"a":1,"b":2}"#,
            r"del(. as $x | reduce (.a,.b) as $i (.; if .a then (.a)? else foreach (try ($x as {a:$a} | .)) as $y (.; .; .) end))",
            "",
            r"Invalid path expression with result",
            5,
        ),
    ])
}

/// #5044: a `foreach` EXTRACT that is `if C then FE else B end` over a source with `.` among its elements keeps its
/// reading ([`EmptiedUpdate::else_beyond_dot`] is not honoured there; a source that hands the register off is read, see
/// `test_foreach_extract_if_with_a_navigating_else_reads_a_source_that_hands_the_register_off_5044`): read as a
/// `foreach` UPDATE is, the `[path(..)?]` row below turned into `[]` for jq's
/// `[["a"]]` and the `del` row into a refusal for jq's `"C"` (16 of 12,000 grid rows that matched jq). Every row
/// captured from jq 1.7.1, both routes. (#5102: the `INIT null` row that was here matched through the first step's
/// wrong emission, a nested `foreach` found at the navigated register, and jq's answer is the last step's, which this
/// reading refuses; it is pinned, ignored, in `i5102.rs` as #5106's.)
#[test]
fn test_foreach_extract_if_with_a_navigating_else_keeps_the_rows_that_matched_5044() -> Result<()> {
    assert_path_rows_both_routes_3749(&[(
        r#"{"a":[1],"b":0}"#,
        r#"try del(. as $x | foreach (.a,.) as $i (.; .; if true then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | .) else if .a then .b else .a end end)) catch "C""#,
        "\"C\"\n",
        "",
        0,
    )])
}

/// #5044: a stage after the `reduce` meets the `null` an emptied step left, which is no node of the register the
/// `reduce` entered with, so jq's `.a` on it raises its own catchable error (`[]` under `?`, `"C"` under a `try`). The
/// resolver stated no register for a `reduce` whose UPDATE holds a `foreach` and answered `[["a"]]` and `["b"]` from the
/// by-value drive. Every row captured from jq 1.7.1, both routes.
#[test]
fn test_reduce_update_if_with_the_caught_foreach_then_a_stage_after_the_reduce_5044() -> Result<()>
{
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":0,"b":{"a":1}}"#,
            r"[path(. as $x | reduce (.a,.b) as $i (.; if . then foreach (try ($x as {a:$a} | .)) as $y (.; .; .) else .a end) | .a?)?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":0,"b":{"a":1}}"#,
            r#"try path(. as $x | reduce (.a,.b) as $i (.; if . then foreach (try ($x as {a:$a} | .)) as $y (.; .; .) else .a end) | .b) catch "C""#,
            "\"C\"\n",
            "",
            0,
        ),
        (
            r#"{"a":1,"b":2}"#,
            r"[path(. as $x | reduce (.a,.b) as $i (.; if . then foreach (try ($x as {a:$a} | .)) as $y (.; .; .) else .a end) | .a)?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":0,"b":{"a":1}}"#,
            r"[path(. as $x | reduce (.a,.b) as $i (.; if .a then .b else foreach (try ($x as {a:$a} | .)) as $y (.; .; .) end) | .a?)?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":1,"b":2}"#,
            r"[path(. as $x | reduce .[] as $i (.; if .a then foreach (try ($x as {a:$a} | .)) as $y (.; .; .) else null end) | .a?)?]",
            "[]\n",
            "",
            0,
        ),
    ])
}

/// #5044: a `reduce` whose INIT is not `.` (`null`, `{}`, `1`, `.a`, `$x`) keeps its reading of the same `if`: read as the
/// others are, a stage after it met a `null` the resolver cannot tell from the register's and raised a guess no `?`
/// catches (51 of 12,000 grid rows that matched jq). Row captured from jq 1.7.1, both routes.
#[test]
fn test_reduce_update_with_an_init_other_than_dot_keeps_the_rows_that_matched_5044() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":0,"b":{"a":1}}"#,
            r"[path(. as $x | reduce (null,.a) as $i (null; if . then foreach (try ($x as {a:$a} | .)) as $y (.; .; .) else 1 end) | .a?)?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":false,"b":null}"#,
            r"[path(. as $x | reduce (.a,.a) as $i (null; if . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | .) elif .b then 1 else 1 end) | .a)?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":false,"b":null}"#,
            r"[path(. as $x | reduce .[]? as $i ({}; if true then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | .) else empty end) | .a?)?]",
            "[]\n",
            "",
            0,
        ),
    ])
}

/// #5044: a `foreach` EXTRACT `if C then FE else B end` with an `else` beyond `.` is read as an UPDATE's is (the
/// `foreach` is empty on every step, so the EXTRACT emits nothing) when the fold's source never hands the register on
/// (`(.a,.b)`, `.[]`, `.[]?`, [`source_hands_register_off`]): `$x` is a node the register is never on, so the destructure
/// raises each step. `main` read it by value and a `try`/`del`/`|=` around it answered `"C"` or refused where jq
/// finishes. Every row captured from jq 1.7.1, both routes.
#[test]
fn test_foreach_extract_if_with_a_navigating_else_reads_a_source_that_hands_the_register_off_5044(
) -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":1,"b":2}"#,
            r#"try path(. as $x | foreach .[] as $i (.; .; if . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | .) elif .b then . | .a else . | .a end)) catch "C""#,
            "",
            "",
            0,
        ),
        (
            r#"{"a":false,"b":null}"#,
            r#"try path(. as $x | foreach .[]? as $i (null; .; if true then try (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) else (.a,.b) end)) catch "C""#,
            "",
            "",
            0,
        ),
        (
            r#"{"a":[1],"b":0}"#,
            r#"try del(. as $x | foreach (.a,.a) as $i (.; .; if true then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | .) else null end)) catch "C""#,
            "{\"a\":[1],\"b\":0}\n",
            "",
            0,
        ),
        (
            r#"{"a":null,"b":2}"#,
            r#"try ((. as $x | foreach (.a,.a) as $i (.; .; if true then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | .) else empty end)) |= 9) catch "C""#,
            "{\"a\":null,\"b\":2}\n",
            "",
            0,
        ),
        (
            r#"{"a":1,"b":2}"#,
            r#"try del(. as $x | foreach .[] as $i (null; .; if true then try (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) else .b end)) catch "C""#,
            "{\"a\":1,\"b\":2}\n",
            "",
            0,
        ),
        (
            r#"{"a":1,"b":2}"#,
            r#"try del(. as $x | foreach (.a,.b) as $i (null; .; if true then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | .) elif .b then . else . end)) catch "C""#,
            "{\"a\":1,\"b\":2}\n",
            "",
            0,
        ),
    ])
}

/// #5044: the same for the mirrored `if C then B else FE end` in a `foreach` UPDATE: the `foreach` in the `else` is
/// read as the one in the `then` is when the source hands the register off. Every row captured from jq 1.7.1, both
/// routes.
#[test]
fn test_foreach_update_mirrored_if_reads_a_source_that_hands_the_register_off_5044() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":null,"b":2}"#,
            r#"try ((. as $x | foreach .[] as $i (null; if .a == 1 then .a else (foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | .) end; $i)) |= 9) catch "C""#,
            "{\"a\":null,\"b\":2}\n",
            "",
            0,
        ),
        (
            r#"{"a":[1],"b":0}"#,
            r#"try ((. as $x | foreach .[]? as $i (.; if .a == 1 then . else (foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | .) end; $i)) |= 9) catch "C""#,
            "{\"a\":[1],\"b\":0}\n",
            "",
            0,
        ),
        (
            r#"{"a":false,"b":null}"#,
            r#"try del(. as $x | foreach (.a,.a) as $i (.; if .a == 1 then . else foreach (try ($x as {a:$a} | .)) as $y (.; .; .) end; true)) catch "C""#,
            "{\"a\":false,\"b\":null}\n",
            "",
            0,
        ),
        (
            r#"{"a":null,"b":2}"#,
            r#"try del(. as $x | foreach .[] as $i (null; if .a then empty else try (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) end; .a)) catch "C""#,
            "{\"a\":null,\"b\":2}\n",
            "",
            0,
        ),
        (
            r#"{"a":[1],"b":0}"#,
            r#"try del(. as $x | foreach .[]? as $i (null; if .a then 1 else try (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) end; null)) catch "C""#,
            "{\"a\":[1],\"b\":0}\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":1},"b":2}"#,
            r#"try ((. as $x | foreach (.a,.b) as $i (null; if .a == 1 then .a else try (foreach (try ($x as {a:$a} | .)) as $y (.; .; .)) end; .)) |= 9) catch "C""#,
            "{\"a\":{\"a\":1},\"b\":2}\n",
            "",
            0,
        ),
    ])
}

/// #5044: a source with `.` among its elements (`(.a,.)`) is not read: there `$x` is the register at the last step and
/// the `foreach` emits, so an EXTRACT read as empty refused 19 of 12,000 grid rows that matched jq. Rows captured from
/// jq 1.7.1, both routes. (#5102: the `INIT null` row that was here is in `i5102.rs`, ignored, see above.)
#[test]
fn test_foreach_extract_if_with_a_source_that_hands_the_register_on_keeps_the_rows_that_matched_5044(
) -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":1,"b":2}"#,
            r#"try del(. as $x | foreach (.a,.) as $i (.; .; if true then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | .) else empty end)) catch "C""#,
            "\"C\"\n",
            "",
            0,
        ),
        (
            r"{}",
            r"[path(. as $x | foreach (.a,.) as $i (.; .; if . then (foreach (try ($x as {a:$a} | .)) as $y (.; .; .) | .) elif .b then .b else .b end))?]",
            "[]\n",
            "",
            0,
        ),
    ])
}

/// #5223 Phase 3: this issue's repro rows, with `path`, `del` and `|=`
/// wherever the row is a `path(...)`, as jq 1.7.1 answers them (stdout, the
/// first error message, the exit code). The path-mode interpreter carries
/// jq's register, so they match by construction, on both routes.
#[cfg(not(feature = "unshared-containers"))]
#[test]
fn test_superseded_rows_match_jq_through_the_path_mode_interpreter_5044() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":1,"b":2}"#,
            r"path(. as $x | reduce (.a,.b) as $i (.; if . then foreach (try ($x as {a:$a} | .)) as $y (.; .; .) else .a end))",
            "",
            "Invalid path expression near attempt to access element \"a\" of null",
            5,
        ),
        (
            r#"{"a":1,"b":2}"#,
            r"del(. as $x | reduce (.a,.b) as $i (.; if . then foreach (try ($x as {a:$a} | .)) as $y (.; .; .) else .a end))",
            "",
            "Invalid path expression near attempt to access element \"a\" of null",
            5,
        ),
        (
            r#"{"a":1,"b":2}"#,
            r"(. as $x | reduce (.a,.b) as $i (.; if . then foreach (try ($x as {a:$a} | .)) as $y (.; .; .) else .a end)) |= 9",
            "",
            "Invalid path expression near attempt to access element \"a\" of null",
            5,
        ),
        (
            r#"{"a":1,"b":2}"#,
            r"[path(. as $x | reduce (.a,.b) as $i (.; if . then foreach (try ($x as {a:$a} | .)) as $y (.; .; .) else .a end))?]",
            "[]\n",
            "",
            0,
        ),
        (
            r#"{"a":1,"b":2}"#,
            r"path(. as $x | foreach (.a,.b) as $i (.; .; if . then foreach (try ($x as {a:$a} | .)) as $y (.; .; .) else .a end))",
            "",
            "",
            0,
        ),
        (
            r#"{"a":1,"b":2}"#,
            r"del(. as $x | foreach (.a,.b) as $i (.; .; if . then foreach (try ($x as {a:$a} | .)) as $y (.; .; .) else .a end))",
            "{\"a\":1,\"b\":2}\n",
            "",
            0,
        ),
        (
            r#"{"a":1,"b":2}"#,
            r"(. as $x | foreach (.a,.b) as $i (.; .; if . then foreach (try ($x as {a:$a} | .)) as $y (.; .; .) else .a end)) |= 9",
            "{\"a\":1,\"b\":2}\n",
            "",
            0,
        ),
        (
            r#"{"a":1}"#,
            r#"try del(. as $x | foreach (null,.a) as $i (.; if .a then .b else foreach (try ($x as {a:$a} | .)) as $y (.; .; .) end; .a)) catch "C""#,
            "{\"a\":1}\n",
            "",
            0,
        ),
    ])
}
