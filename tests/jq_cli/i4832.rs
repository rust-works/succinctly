//! #4832: a `$x` bound elsewhere than jq's register, navigated or destructured directly in a `reduce` UPDATE.

use anyhow::Result;

use crate::assert_path_rows_both_routes_3749;

/// #4832: the UPDATE of a `reduce` whose source moved jq's path register onto an element runs with the
/// register there, so a `$x` bound at another node is no longer the register and a navigation or pattern
/// step on it raises at the step (`near attempt to access element "a" of {"a":1}`, the element and `$x`'s
/// value). The binary named the final emission instead (`with result 1`, the accumulator the UPDATE
/// handed back); stdout and the exit code were already jq's. Covers a bare and a `?`-suffixed navigation,
/// an object and an array pattern, a navigation behind another, over a moving source (`.a`, `.a?`,
/// `.[]?`, a comma of two, a `tostring` of one), for `path`/`del`/`|=`, a caught `?`, and `try ... catch .`
/// reading the message. The `$x` that IS the register (`.a as $x` over `.a`) and a source that cannot
/// move it (`1`, a comma of literals) are the controls: they answer, or refuse by the old wording, as
/// before. Every row captured from `/usr/bin/jq` 1.7.1, both routes.
#[test]
fn test_4832_reduce_update_foreign_marker_step_names_the_access() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":1}"#,
            r"path(. as $x | reduce .a as $i (.; $x | .a))",
            r"",
            r#"Invalid path expression near attempt to access element "a" of {"a":1}"#,
            5,
        ),
        (
            r#"{"a":1}"#,
            r"del(. as $x | reduce .a as $i (.; $x | .a))",
            r"",
            r#"Invalid path expression near attempt to access element "a" of {"a":1}"#,
            5,
        ),
        (
            r#"{"a":1}"#,
            r"(. as $x | reduce .a as $i (.; $x | .a)) |= 9",
            r"",
            r#"Invalid path expression near attempt to access element "a" of {"a":1}"#,
            5,
        ),
        (
            r#"{"a":1}"#,
            r"path(. as $x | reduce .a as $i (.; $x as {a:$a} | .))",
            r"",
            r#"Invalid path expression near attempt to access element "a" of {"a":1}"#,
            5,
        ),
        (
            r#"{"a":1}"#,
            r"del(. as $x | reduce .a as $i (.; $x as {a:$a} | .))",
            r"",
            r#"Invalid path expression near attempt to access element "a" of {"a":1}"#,
            5,
        ),
        (
            r#"{"a":1}"#,
            r"(. as $x | reduce .a as $i (.; $x as [$a] | .)) |= 9",
            r"",
            r#"Invalid path expression near attempt to access element 0 of {"a":1}"#,
            5,
        ),
        (
            r#"{"a":1}"#,
            r"path(. as $x | reduce .a? as $i (.; $x | .a))",
            r"",
            r#"Invalid path expression near attempt to access element "a" of {"a":1}"#,
            5,
        ),
        (
            r#"{"a":1}"#,
            r"path(. as $x | reduce (.a, .a) as $i (.; $x | .a))",
            r"",
            r#"Invalid path expression near attempt to access element "a" of {"a":1}"#,
            5,
        ),
        (
            r#"{"a":1}"#,
            r"path(. as $x | reduce .[]? as $i (.; $x | .a))",
            r"",
            r#"Invalid path expression near attempt to access element "a" of {"a":1}"#,
            5,
        ),
        (
            r#"{"a":1}"#,
            r"path(. as $x | reduce (.a | tostring) as $i (.; $x | .a))",
            r"",
            r#"Invalid path expression near attempt to access element "a" of {"a":1}"#,
            5,
        ),
        (
            r#"{"a":1}"#,
            r"path(. as $x | reduce .a as $i (.; $x | .a?))",
            r"",
            r#"Invalid path expression near attempt to access element "a" of {"a":1}"#,
            5,
        ),
        (
            r#"{"a":1}"#,
            r"[path(. as $x | reduce .a as $i (.; $x | .a?))?]",
            "[]\n",
            r"",
            0,
        ),
        (
            r#"{"a":1}"#,
            r"path(. as $x | reduce .a as $i (.; $x | .a | .b))",
            r"",
            r#"Invalid path expression near attempt to access element "a" of {"a":1}"#,
            5,
        ),
        (
            r#"{"a":1}"#,
            r"try path(. as $x | reduce .a as $i (.; $x | .a)) catch .",
            r#""Invalid path expression near attempt to access element \"a\" of {\"a\":1}"
"#,
            r"",
            0,
        ),
        (
            r#"{"a":1}"#,
            r"path(. as $x | reduce 1 as $i (.; $x | .a))",
            r"",
            r"Invalid path expression with result 1",
            5,
        ),
        (
            r#"{"a":1}"#,
            r"path(. as $x | reduce (1,2) as $i (.; $x))",
            "[]\n",
            r"",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"path(. as $x | reduce .a as $i (.; $x | .a))",
            r"",
            r#"Invalid path expression near attempt to access element "a" of {"a":null}"#,
            5,
        ),
        (
            r#"{"a":null}"#,
            r"del(. as $x | reduce .a as $i (.; $x | .a))",
            r"",
            r#"Invalid path expression near attempt to access element "a" of {"a":null}"#,
            5,
        ),
        (
            r#"{"a":null}"#,
            r"(. as $x | reduce .a as $i (.; $x | .a)) |= 9",
            r"",
            r#"Invalid path expression near attempt to access element "a" of {"a":null}"#,
            5,
        ),
        (
            r#"{"a":null}"#,
            r"path(. as $x | reduce .a as $i (.; $x as {a:$a} | .))",
            r"",
            r#"Invalid path expression near attempt to access element "a" of {"a":null}"#,
            5,
        ),
        (
            r#"{"a":null}"#,
            r"del(. as $x | reduce .a as $i (.; $x as {a:$a} | .))",
            r"",
            r#"Invalid path expression near attempt to access element "a" of {"a":null}"#,
            5,
        ),
        (
            r#"{"a":null}"#,
            r"(. as $x | reduce .a as $i (.; $x as [$a] | .)) |= 9",
            r"",
            r#"Invalid path expression near attempt to access element 0 of {"a":null}"#,
            5,
        ),
        (
            r#"{"a":null}"#,
            r"path(. as $x | reduce .a? as $i (.; $x | .a))",
            r"",
            r#"Invalid path expression near attempt to access element "a" of {"a":null}"#,
            5,
        ),
        (
            r#"{"a":null}"#,
            r"path(. as $x | reduce (.a, .a) as $i (.; $x | .a))",
            r"",
            r#"Invalid path expression near attempt to access element "a" of {"a":null}"#,
            5,
        ),
        (
            r#"{"a":null}"#,
            r"path(. as $x | reduce .[]? as $i (.; $x | .a))",
            r"",
            r#"Invalid path expression near attempt to access element "a" of {"a":null}"#,
            5,
        ),
        (
            r#"{"a":null}"#,
            r"path(. as $x | reduce (.a | tostring) as $i (.; $x | .a))",
            r"",
            r#"Invalid path expression near attempt to access element "a" of {"a":null}"#,
            5,
        ),
        (
            r#"{"a":null}"#,
            r"path(. as $x | reduce .a as $i (.; $x | .a?))",
            r"",
            r#"Invalid path expression near attempt to access element "a" of {"a":null}"#,
            5,
        ),
        (
            r#"{"a":null}"#,
            r"[path(. as $x | reduce .a as $i (.; $x | .a?))?]",
            "[]\n",
            r"",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"path(. as $x | reduce .a as $i (.; $x | .a | .b))",
            r"",
            r#"Invalid path expression near attempt to access element "a" of {"a":null}"#,
            5,
        ),
        (
            r#"{"a":null}"#,
            r"try path(. as $x | reduce .a as $i (.; $x | .a)) catch .",
            r#""Invalid path expression near attempt to access element \"a\" of {\"a\":null}"
"#,
            r"",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"path(. as $x | reduce 1 as $i (.; $x | .a))",
            r"",
            r"Invalid path expression with result null",
            5,
        ),
        (
            r#"{"a":null}"#,
            r"path(. as $x | reduce (1,2) as $i (.; $x))",
            "[]\n",
            r"",
            0,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"path(. as $x | reduce .a as $i (.; $x | .a))",
            r"",
            r#"Invalid path expression near attempt to access element "a" of {"a":{"b":1}}"#,
            5,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"del(. as $x | reduce .a as $i (.; $x | .a))",
            r"",
            r#"Invalid path expression near attempt to access element "a" of {"a":{"b":1}}"#,
            5,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"(. as $x | reduce .a as $i (.; $x | .a)) |= 9",
            r"",
            r#"Invalid path expression near attempt to access element "a" of {"a":{"b":1}}"#,
            5,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"path(. as $x | reduce .a as $i (.; $x as {a:$a} | .))",
            r"",
            r#"Invalid path expression near attempt to access element "a" of {"a":{"b":1}}"#,
            5,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"del(. as $x | reduce .a as $i (.; $x as {a:$a} | .))",
            r"",
            r#"Invalid path expression near attempt to access element "a" of {"a":{"b":1}}"#,
            5,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"(. as $x | reduce .a as $i (.; $x as [$a] | .)) |= 9",
            r"",
            r#"Invalid path expression near attempt to access element 0 of {"a":{"b":1}}"#,
            5,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"path(. as $x | reduce .a? as $i (.; $x | .a))",
            r"",
            r#"Invalid path expression near attempt to access element "a" of {"a":{"b":1}}"#,
            5,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"path(. as $x | reduce (.a, .a) as $i (.; $x | .a))",
            r"",
            r#"Invalid path expression near attempt to access element "a" of {"a":{"b":1}}"#,
            5,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"path(. as $x | reduce .[]? as $i (.; $x | .a))",
            r"",
            r#"Invalid path expression near attempt to access element "a" of {"a":{"b":1}}"#,
            5,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"path(. as $x | reduce (.a | tostring) as $i (.; $x | .a))",
            r"",
            r#"Invalid path expression near attempt to access element "a" of {"a":{"b":1}}"#,
            5,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"path(. as $x | reduce .a as $i (.; $x | .a?))",
            r"",
            r#"Invalid path expression near attempt to access element "a" of {"a":{"b":1}}"#,
            5,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"[path(. as $x | reduce .a as $i (.; $x | .a?))?]",
            "[]\n",
            r"",
            0,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"path(. as $x | reduce .a as $i (.; $x | .a | .b))",
            r"",
            r#"Invalid path expression near attempt to access element "a" of {"a":{"b":1}}"#,
            5,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"path(.a as $x | reduce .a as $i (0; $x | . as {a:$a} | .))",
            r"",
            r#"Invalid path expression with result {"b":1}"#,
            5,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"path(.a as $x | reduce .a? as $i (0; $x | .b))",
            r"",
            r"Invalid path expression with result 1",
            5,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"try path(. as $x | reduce .a as $i (.; $x | .a)) catch .",
            r#""Invalid path expression near attempt to access element \"a\" of {\"a\":{\"b\":1}}"
"#,
            r"",
            0,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"path(. as $x | reduce 1 as $i (.; $x | .a))",
            r"",
            r#"Invalid path expression with result {"b":1}"#,
            5,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"path(. as $x | reduce (1,2) as $i (.; $x))",
            "[]\n",
            r"",
            0,
        ),
    ])
}

/// #4832: the same fix over the other UPDATE shapes that read a foreign `$x` (an `if`-free sample of a
/// 100,800-row sweep: a comma, a `first`, a `try`, a `?`, `..`, a computed `tostring`, `getpath`, a non-`.` INIT, a
/// pattern ahead of the `$x`), each a row the binary answered with the final emission's wording before.
/// Every row captured from `/usr/bin/jq` 1.7.1, both routes.
#[test]
fn test_4832_reduce_update_foreign_marker_step_over_update_shapes() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":false,"b":null}"#,
            r"try path(. as $x | reduce first(.[]?) as $i (.; $x as [$a] | .)) catch .",
            r#""Invalid path expression near attempt to access element 0 of {\"a\":false,\"b\":null}"
"#,
            r"",
            0,
        ),
        (
            r#"{"a":false,"b":null}"#,
            r"try path(. as $x | reduce (.a?, .b?) as $i (.; $x as {a:$a} | .)) catch .",
            r#""Invalid path expression near attempt to access element \"a\" of {\"a\":false,\"b\":null}"
"#,
            r"",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"try path(.a as $x | reduce .a.b? as $i (.; $x | . as {a:$a} | .)) catch .",
            r#""Invalid path expression with result null"
"#,
            r"",
            0,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"try path(.a as $x | reduce first(.[]?) as $i (.; $x | .. )) catch .",
            r#""Invalid path expression with result 1"
"#,
            r"",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"try path(.a as $x | reduce .a as $i (.; $x | .[0]?)) catch .",
            r#""Invalid path expression with result null"
"#,
            r"",
            0,
        ),
        (
            r#"{"a":1,"b":2}"#,
            r"try path(. as $x | reduce (.a?, .) as $i (.; $x | .[]?)) catch .",
            r#""Invalid path expression near attempt to iterate through {\"a\":1,\"b\":2}"
"#,
            r"",
            0,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"try path(. as $x | reduce (.a?, .) as $i (.; $x | .a | .b?)) catch .",
            r#""Invalid path expression near attempt to access element \"a\" of {\"a\":{\"b\":1}}"
"#,
            r"",
            0,
        ),
        (
            r#"{"a":false,"b":null}"#,
            r"try path(. as $x | reduce .. as $i (.; $x | .a)) catch .",
            r#""Invalid path expression near attempt to access element \"a\" of {\"a\":false,\"b\":null}"
"#,
            r"",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"try path(. as $x | reduce (.a | length) as $i (.; $x | .a?)) catch .",
            r#""Invalid path expression near attempt to access element \"a\" of {\"a\":null}"
"#,
            r"",
            0,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"try path(.a as $x | reduce first(.[]?) as $i (.; ($x | .a), 1)) catch .",
            r#""Invalid path expression with result 1"
"#,
            r"",
            0,
        ),
        (
            r#"{"a":1,"b":2}"#,
            r"try path(. as $x | reduce (.a | tostring) as $i (.; ($x | .a)?)) catch .",
            r#""Invalid path expression with result null"
"#,
            r"",
            0,
        ),
        (
            r#"{"a":1,"b":2}"#,
            r"try path(.a as $x | reduce .[]? as $i (.; . as $y | $x | .a)) catch .",
            r#""Cannot index number with string \"a\""
"#,
            r"",
            0,
        ),
        (
            r#"{"a":[1,2],"b":{"a":3}}"#,
            r"path(. as $x | reduce .a as $i (.; . as {a:$a} | $x))",
            r"",
            r#"Invalid path expression near attempt to access element "a" of {"a":[1,2],"b":{"a":3}}"#,
            5,
        ),
        (
            r#"{"a":1,"b":2}"#,
            r"try path(.a as $x | reduce first(.[]?) as $i (.; . as {a:$a} | $x)) catch .",
            r#""Invalid path expression near attempt to access element \"a\" of {\"a\":1,\"b\":2}"
"#,
            r"",
            0,
        ),
        (
            r#"{"a":true}"#,
            r"(. as $x | reduce (.a?, .) as $i (.; . as {a:$a} | $x)) |= 9",
            r"",
            r#"Invalid path expression near attempt to access element "a" of {"a":true}"#,
            5,
        ),
        (
            r#"{"a":1,"b":2}"#,
            r"[path(. as $x | reduce .a? as $i (.; . as {a:$a} | $x))?]",
            "[]\n",
            r"",
            0,
        ),
        (
            r#"{"a":true}"#,
            r"try path(. as $x | reduce .. as $i (.; [$x | .a?])) catch .",
            r#""Invalid path expression near attempt to access element \"a\" of {\"a\":true}"
"#,
            r"",
            0,
        ),
        (
            r"[[1],[1]]",
            r"try path(. as $x | reduce .[]? as $i (.; first($x | .a))) catch .",
            r#""Invalid path expression near attempt to access element \"a\" of [[1],[1]]"
"#,
            r"",
            0,
        ),
        (
            r#"{"a":1,"b":2}"#,
            r"try path(. as $x | reduce first(.[]?) as $i (.; try ($x as {a:$a} | .))) catch .",
            r#""Invalid path expression with result null"
"#,
            r"",
            0,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"try path(. as $x | reduce (.a?, .b?) as $i (.; try ($x | .a))) catch .",
            r#""Invalid path expression with result null"
"#,
            r"",
            0,
        ),
        (
            r"[]",
            r"del(. as $x | reduce .[0]? as $i (0; $x | .. ))",
            r"",
            r"Invalid path expression near attempt to iterate through []",
            5,
        ),
        (
            r#"{"a":[1,2],"b":{"a":3}}"#,
            r"try path(. as $x | reduce first(.[]?) as $i (0; $x | .. )) catch .",
            r#""Invalid path expression near attempt to iterate through {\"a\":[1,2],\"b\":{\"a\":3}}"
"#,
            r"",
            0,
        ),
        (
            r#"{"a":true}"#,
            r"try path(. as $x | reduce .. as $i (0; $x | .[0]?)) catch .",
            r#""Invalid path expression near attempt to access element 0 of {\"a\":true}"
"#,
            r"",
            0,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"try path(.a as $x | reduce .a as $i (0; $x | .a | .b?)) catch .",
            r#""Invalid path expression with result null"
"#,
            r"",
            0,
        ),
        (
            r#"[{"a":1}]"#,
            r"try path(. as $x | reduce .[0]? as $i (0; $x | .a)) catch .",
            r#""Invalid path expression near attempt to access element \"a\" of [{\"a\":1}]"
"#,
            r"",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"try path(.a as $x | reduce .a.b? as $i (0; $x | .a?)) catch .",
            r#""Invalid path expression with result null"
"#,
            r"",
            0,
        ),
        (
            r#"{"a":1,"b":2}"#,
            r"try path(.a as $x | reduce .a? as $i (0; ($x | .a), 1)) catch .",
            r#""Cannot index number with string \"a\""
"#,
            r"",
            0,
        ),
        (
            r#"{"a":[1,2],"b":{"a":3}}"#,
            r"try path(.a as $x | reduce (.a?, .b?) as $i (0; . as $y | $x | .a)) catch .",
            r#""Cannot index array with string \"a\""
"#,
            r"",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"try path(. as $x | reduce (.a?, .) as $i (0; [$x | .a?])) catch .",
            r#""Invalid path expression near attempt to access element \"a\" of {\"a\":null}"
"#,
            r"",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"try path(. as $x | reduce .a as $i (0; first($x | .a))) catch .",
            r#""Invalid path expression near attempt to access element \"a\" of {\"a\":null}"
"#,
            r"",
            0,
        ),
        (
            r#"{"a":[1,2],"b":{"a":3}}"#,
            r"try path(.b as $x | reduce (.a?, .b?) as $i (0; try ($x | .a))) catch .",
            r#""Invalid path expression with result 3"
"#,
            r"",
            0,
        ),
        (
            r#"{"a":1,"b":2}"#,
            r"try path(.a as $x | reduce first(.[]?) as $i (null; $x | . as {a:$a} | .)) catch .",
            r#""Cannot index number with string \"a\""
"#,
            r"",
            0,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"try path(.a as $x | reduce .[]? as $i (null; $x | .. )) catch .",
            r#""Invalid path expression with result 1"
"#,
            r"",
            0,
        ),
        (
            r"[]",
            r"[path(. as $x | reduce .[0]? as $i (null; $x | .. ))?]",
            "[]\n",
            r"",
            0,
        ),
        (
            r#"{"a":1,"b":2}"#,
            r"try path(. as $x | reduce first(.[]?) as $i (null; $x | .[]?)) catch .",
            r#""Invalid path expression near attempt to iterate through {\"a\":1,\"b\":2}"
"#,
            r"",
            0,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"try path(. as $x | reduce .a? as $i (null; $x | .a | .b?)) catch .",
            r#""Invalid path expression near attempt to access element \"a\" of {\"a\":{\"b\":1}}"
"#,
            r"",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"try path(.a as $x | reduce .a.b? as $i (null; $x | .a)) catch .",
            r#""Invalid path expression with result null"
"#,
            r"",
            0,
        ),
        (
            r#"{"a":true}"#,
            r"try path(. as $x | reduce .. as $i (null; $x | .a?)) catch .",
            r#""Invalid path expression near attempt to access element \"a\" of {\"a\":true}"
"#,
            r"",
            0,
        ),
        (
            r#"{"a":null}"#,
            r"try path(. as $x | reduce (.a | length) as $i (null; ($x | .a), 1)) catch .",
            r#""Invalid path expression near attempt to access element \"a\" of {\"a\":null}"
"#,
            r"",
            0,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"try path(. as $x | reduce .a as $i (null; ($x | .a)?)) catch .",
            r#""Invalid path expression with result null"
"#,
            r"",
            0,
        ),
        (
            r#"{"a":true}"#,
            r"try path(. as $x | reduce first(.[]?) as $i (null; . as $y | $x | .a)) catch .",
            r#""Invalid path expression near attempt to access element \"a\" of {\"a\":true}"
"#,
            r"",
            0,
        ),
        (
            r#"{"a":1,"b":2}"#,
            r"try path(.a as $x | reduce .a? as $i (null; [$x | .a?])) catch .",
            r#""Invalid path expression with result []"
"#,
            r"",
            0,
        ),
        (
            r#"{"a":true}"#,
            r"try path(.a as $x | reduce (.a?, .b?) as $i (null; first($x | .a))) catch .",
            r#""Cannot index boolean with string \"a\""
"#,
            r"",
            0,
        ),
        (
            r#"{"a":[1,2],"b":{"a":3}}"#,
            r"try path(. as $x | reduce .a? as $i (null; try ($x | .a))) catch .",
            r#""Invalid path expression with result null"
"#,
            r"",
            0,
        ),
    ])
}

/// #4832: the rows that already matched before the change keep matching: a `$x` that IS the register, an
/// UPDATE that never navigates it, and the shapes whose refusal still names the final emission.
#[test]
fn test_4832_reduce_update_foreign_marker_controls_keep_matching() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":[1,2],"b":{"a":3}}"#,
            r"path(.a as $x | reduce .[]? as $i (.b; $x as {a:$a} | .))",
            r"",
            r#"Invalid path expression near attempt to iterate through {"a":[1,2],"b":{"a":3}}"#,
            5,
        ),
        (
            r"null",
            r"[path(. as $x | reduce .[0]? as $i (.a; $x | .a?))?]",
            r#"[["a"]]
"#,
            r"",
            0,
        ),
        (
            r#"[{"a":1}]"#,
            r"try path(.b as $x | reduce .a? as $i (0; $x | keys?)) catch .",
            r#""Cannot index array with string \"b\""
"#,
            r"",
            0,
        ),
        (
            r"true",
            r"del(.a as $x | reduce .[]? as $i (.; $x | .a?))",
            r"",
            r#"Cannot index boolean with string "a""#,
            5,
        ),
        (
            r"null",
            r"try path(.b as $x | reduce .a as $i (.a; . as {a:$a} | $x)) catch .",
            r#"["a"]
"#,
            r"",
            0,
        ),
        (
            r#"[{"a":1}]"#,
            r"(. as $x | reduce first(.[]?) as $i (null; $x as [$a] | .)) |= 9",
            r"",
            r#"Invalid path expression near attempt to access element 0 of [{"a":1}]"#,
            5,
        ),
        (
            r#"{"a":true}"#,
            r"[path(. as $x | reduce first(.[]?) as $i (.; {a: ($x | .a)}))?]",
            "[]\n",
            r"",
            0,
        ),
        (
            r#"[{"a":1}]"#,
            r"try path(.b as $x | reduce (.a?, .b?) as $i (.b; . as $y | $x | .a)) catch .",
            r#""Cannot index array with string \"b\""
"#,
            r"",
            0,
        ),
        (
            r"[]",
            r"try path(. as $x | reduce null as $i (null; $x | .a | .b?)) catch .",
            r#""Cannot index array with string \"a\""
"#,
            r"",
            0,
        ),
        (
            r"true",
            r"try path(.a as $x | reduce empty as $i (0; try ($x | .a))) catch .",
            r#""Cannot index boolean with string \"a\""
"#,
            r"",
            0,
        ),
        (
            r#"{"a":false,"b":null}"#,
            r"path(.a as $x | reduce empty as $i (.; $x | .a))",
            "[]\n",
            r"",
            0,
        ),
        (
            r#"{"a":[1,2],"b":{"a":3}}"#,
            r#"[path(.a as $x | reduce .a? as $i (.; $x | getpath(["a"])))?]"#,
            "[]\n",
            r"",
            0,
        ),
        (
            r#"{"a":[1,2],"b":{"a":3}}"#,
            r"[path(.b as $x | reduce empty as $i (.a; ($x | .a), 1))?]",
            r#"[["a"]]
"#,
            r"",
            0,
        ),
        (
            r"null",
            r"[path(. as $x | reduce .a as $i (.b; $x | .. ))?]",
            r#"[["b"]]
"#,
            r"",
            0,
        ),
        (
            r#"{"a":[1,2],"b":{"a":3}}"#,
            r"(. as $x | reduce .a.b? as $i (.a; .)) |= 9",
            r"",
            r#"Invalid path expression near attempt to access element "a" of {"a":[1,2],"b":{"a":3}}"#,
            5,
        ),
        (
            r#"{"a":[1,2],"b":{"a":3}}"#,
            r"try path(.b as $x | reduce .a? as $i (.; ($x | .a)?)) catch .",
            r#""Invalid path expression with result null"
"#,
            r"",
            0,
        ),
        (
            r"[]",
            r"try path(. as $x | reduce .[]? as $i (.; {a: ($x | .a)})) catch .",
            "[]\n",
            r"",
            0,
        ),
        (
            r#"{"a":[1,2],"b":{"a":3}}"#,
            r"(. as $x | reduce .. as $i (0; ($x | .a)?)) |= 9",
            r"",
            r"Invalid path expression with result null",
            5,
        ),
        (
            r#"{"a":[1,2],"b":{"a":3}}"#,
            r#"try path(. as $x | reduce 1 as $i (.a; $x | getpath(["a"]))) catch ."#,
            r#"["a"]
"#,
            r"",
            0,
        ),
        (
            r#"{"a":false,"b":null}"#,
            r#"(.a as $x | reduce 1 as $i (.; $x | has("a")?)) |= 9"#,
            r"",
            r"Invalid path expression with result null",
            5,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"del(.a as $x | reduce .. as $i (null; $x | tostring))",
            r"",
            r#"Invalid path expression with result "{\"b\":1}""#,
            5,
        ),
        (
            r"true",
            r"del(. as $x | reduce (.a?, .b?) as $i (.; . as {a:$a} | $x))",
            "null\n",
            r"",
            0,
        ),
        (
            r#"{"a":[1,2],"b":{"a":3}}"#,
            r"try path(.b as $x | reduce (.a?, .b?) as $i (.a; $x | .[0]?)) catch .",
            r#""Invalid path expression near attempt to access element \"a\" of {\"a\":[1,2],\"b\":{\"a\":3}}"
"#,
            r"",
            0,
        ),
        (
            r#"{"a":true}"#,
            r"[path(.b as $x | reduce .. as $i (.a; $x | tostring))?]",
            "[]\n",
            r"",
            0,
        ),
        (
            r"true",
            r"path(.b as $x | reduce .[]? as $i (.b; try ($x as {a:$a} | .)))",
            r"",
            r#"Cannot index boolean with string "b""#,
            5,
        ),
        (
            r#"[{"a":1}]"#,
            r"path(. as $x | reduce null as $i (null; {a: ($x | .a)}))",
            r"",
            r#"Cannot index array with string "a""#,
            5,
        ),
        (
            r#"{"a":1,"b":2}"#,
            r"[path(.a as $x | reduce null as $i (null; $x | . as {a:$a} | .))?]",
            "[]\n",
            r"",
            0,
        ),
        (
            r#"[{"a":1}]"#,
            r"path(.a as $x | reduce (.a?, .) as $i (0; $x | keys?))",
            r"",
            r#"Cannot index array with string "a""#,
            5,
        ),
        (
            r#"{"a":null}"#,
            r"try path(.b as $x | reduce 1 as $i (.b; .a)) catch .",
            r#"["b"]
"#,
            r"",
            0,
        ),
        (
            r"[]",
            r"[path(. as $x | reduce .. as $i (.b; $x | .a | .b?))?]",
            "[]\n",
            r"",
            0,
        ),
    ])
}

/// #4832: a foreign `$x` read by a fold nested in the `reduce` UPDATE keeps the persistent register's
/// certification. A per-step register for these accepted what jq refuses (`path(...)` printed `[]`, `del`
/// deleted the document, `|= 9` replaced it). The message still names the end of the path where jq names
/// the step, which is why only the common prefix is pinned. Every row captured from `/usr/bin/jq` 1.7.1.
#[test]
fn test_4832_reduce_update_nested_fold_marker_still_refuses() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":true}"#,
            r"path(. as $x | reduce .a? as $i (.; foreach (select($x) | $x as {a:$a} | .) as $y (.; .; .)))",
            "",
            "Invalid path expression",
            5,
        ),
        (
            r#"{"a":true}"#,
            r"del(. as $x | reduce .a? as $i (.; foreach (select($x) | $x as {a:$a} | .) as $y (.; .; .)))",
            "",
            "Invalid path expression",
            5,
        ),
        (
            r#"{"a":true}"#,
            r"(. as $x | reduce .a? as $i (.; foreach (select($x) | $x as {a:$a} | .) as $y (.; .; .))) |= 9",
            "",
            "Invalid path expression",
            5,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"path(. as $x | reduce (.a?, .) as $i (.; foreach (select($x) | $x as {a:$a} | .) as $y (.; .; .)))",
            "",
            "Invalid path expression",
            5,
        ),
        (
            r#"{"a":{"b":1}}"#,
            r"path(. as $x | reduce .a as $j (.; reduce .c[]? as $i (.; $x | .b)))",
            "",
            r#"Invalid path expression near attempt to access element "c" of {"a":{"b":1}}"#,
            5,
        ),
    ])
}
