//! #5150: `(PATH | .) |= empty` deletes the slot `PATH` names, as `PATH |= empty` does.

use anyhow::Result;

use crate::assert_path_rows_both_routes_3749;

// `PATH |= empty` deletes the slots `PATH` names (jq's `_modify` collects them and ends in `delpaths`). When `PATH` ends
// on a bare `.` stage after a real navigation step (`.a | .`, `.[] | . | .`, `.[-1] | (.)?`), the loop had navigated into the
// slot with no way back to the container holding it, and the terminal `Expr::Identity` arm nulled the child in place.
// `update_path_steps` now resolves the step alone when only a trailing `.` follows it, as `delete_path_steps` does for
// `del(.a | .)` (#2256). A filter that produces a value writes the same either way.

/// #5150: the issue's rows, the trailing-identity spellings, and the non-empty updates that are unchanged. Every row
/// captured from `/usr/bin/jq` 1.7.1 (stdout, exit code, message), both routes.
#[test]
#[allow(clippy::literal_string_with_formatting_args)] // jq object patterns read like format arguments
fn test_5150_update_empty_through_a_trailing_identity_deletes_the_slot() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        (
            r#"{"a":[1,2,3],"b":{"b":1}}"#,
            r"(.a | .) |= empty",
            "{\"b\":{\"b\":1}}\n",
            "",
            0,
        ),
        (
            r#"{"a":[1,2,3],"b":{"b":1}}"#,
            r"(.a | .[] | .) |= empty",
            "{\"a\":[],\"b\":{\"b\":1}}\n",
            "",
            0,
        ),
        (
            r#"{"a":[1,2,3],"b":{"b":1}}"#,
            r"(.a | .[0] | .) |= empty",
            "{\"a\":[2,3],\"b\":{\"b\":1}}\n",
            "",
            0,
        ),
        (
            r#"{"a":[1,2,3],"b":{"b":1}}"#,
            r"(.[] | .) |= empty",
            "{}\n",
            "",
            0,
        ),
        (
            r#"{"a":[1,2,3],"b":{"b":1}}"#,
            r"(.a | . | .) |= empty",
            "{\"b\":{\"b\":1}}\n",
            "",
            0,
        ),
        (
            r#"{"a":[1,2,3],"b":{"b":1}}"#,
            r"(.a | (.)?) |= empty",
            "{\"b\":{\"b\":1}}\n",
            "",
            0,
        ),
        (
            r#"{"a":[1,2,3],"b":{"b":1}}"#,
            r"(.a | .) |= 5",
            "{\"a\":5,\"b\":{\"b\":1}}\n",
            "",
            0,
        ),
        (
            r#"{"a":[1,2,3],"b":{"b":1}}"#,
            r"(.a | .[0] | .) |= .+1",
            "{\"a\":[2,2,3],\"b\":{\"b\":1}}\n",
            "",
            0,
        ),
        (
            r#"{"a":[1,2,3],"b":{"b":1}}"#,
            r"(.b | .b | .) |= empty",
            "{\"a\":[1,2,3],\"b\":{}}\n",
            "",
            0,
        ),
        (
            r#"{"a":[1,2,3],"b":{"b":1}}"#,
            r"(.a | .[1:] | .) |= empty",
            "{\"a\":[1],\"b\":{\"b\":1}}\n",
            "",
            0,
        ),
        (
            r#"{"a":[1,2,3],"b":{"b":1}}"#,
            r"(.a|.[]|select(.>1)|.) |= empty",
            "{\"a\":[1],\"b\":{\"b\":1}}\n",
            "",
            0,
        ),
        (
            r#"{"a":[1,2,3],"b":{"b":1}}"#,
            r"(.a | .[-1] | .?) |= empty",
            "{\"a\":[1,2],\"b\":{\"b\":1}}\n",
            "",
            0,
        ),
        (
            r#"{"a":[1,2,3],"b":{"b":1}}"#,
            r"(.a | .[] | (.)) |= empty",
            "{\"a\":[],\"b\":{\"b\":1}}\n",
            "",
            0,
        ),
        (
            r#"{"a":[1,2,3],"b":{"b":1}}"#,
            r"(.a, .b | .) |= empty",
            "{}\n",
            "",
            0,
        ),
        (
            r#"{"a":[1,2,3],"b":{"b":1}}"#,
            r"(.a | .) |= (empty, 1)",
            "{\"a\":1,\"b\":{\"b\":1}}\n",
            "",
            0,
        ),
        (
            r#"{"a":[1,2,3],"b":{"b":1}}"#,
            r"(.a | .) |= select(false)",
            "{\"b\":{\"b\":1}}\n",
            "",
            0,
        ),
        (
            r#"{"a":[1,2,3],"b":{"b":1}}"#,
            r"(.a | .[] | .) |= select(. > 1)",
            "{\"a\":[2,3],\"b\":{\"b\":1}}\n",
            "",
            0,
        ),
        (
            r"[[1,2],[]]",
            r"(.[-1] | (.)?) |= empty",
            "[[1,2]]\n",
            "",
            0,
        ),
        (r"[[1,2],[],[3]]", r"(.[] | .) |= empty", "[]\n", "", 0),
        (
            r"[[1,2],[],[3]]",
            r"(.[] | .[] | .) |= empty",
            "[[],[],[]]\n",
            "",
            0,
        ),
        (r"[1,2,3]", r"(.[1] | .) |= empty", "[1,3]\n", "", 0),
        (r"[1,2,3]", r"(.[] | .) |= empty", "[]\n", "", 0),
        (r"{}", r"(.a | .) |= empty", "{}\n", "", 0),
        (r"null", r"(.a | .) |= empty", "null\n", "", 0),
        (r"null", r"(.[0] | .) |= empty", "null\n", "", 0),
        (r"[]", r"(.[0] | .) |= empty", "[]\n", "", 0),
        (r#"{"a":null}"#, r"(.a | .) |= empty", "{}\n", "", 0),
        (
            r#"{"a":[{"x":1},{"x":2}]}"#,
            r"(.a[] | .x | .) |= empty",
            "{\"a\":[{},{}]}\n",
            "",
            0,
        ),
        (
            r#"{"a":[{"x":1},{"x":2}]}"#,
            r"(.a[] | .) |= empty",
            "{\"a\":[]}\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":{"a":1}}}"#,
            r"(.a.a | .) |= empty",
            "{\"a\":{}}\n",
            "",
            0,
        ),
        (
            r#"{"a":{"a":{"a":1}}}"#,
            r#"(.. | select(type=="number") | .) |= empty"#,
            "{\"a\":{\"a\":{}}}\n",
            "",
            0,
        ),
        (
            r#"{"a":[1,2,3],"b":{"b":1}}"#,
            r"(.. | .) |= empty",
            "null\n",
            "",
            0,
        ),
        (
            r#"{"a":[1,2,3],"b":{"b":1}}"#,
            r"(.. | numbers | .) |= empty",
            "{\"a\":[],\"b\":{}}\n",
            "",
            0,
        ),
        (
            r#"{"a":[1,2,3],"b":{"b":1}}"#,
            r"((.a, .b) | .) |= empty",
            "{}\n",
            "",
            0,
        ),
        (
            r#"{"a":[1,2,3],"b":{"b":1}}"#,
            r"(.a? | .) |= empty",
            "{\"b\":{\"b\":1}}\n",
            "",
            0,
        ),
        (r"[1,[2],[3]]", r"(.[1:] | .) |= empty", "[1]\n", "", 0),
        (r"5", r"(.a? | .) |= empty", "5\n", "", 0),
        (
            r#"{"a":[1,2,3],"b":{"b":1}}"#,
            r"(.a | limit(2; .[]) | .) |= empty",
            "{\"a\":[3],\"b\":{\"b\":1}}\n",
            "",
            0,
        ),
        (
            r#"{"a":[1,2,3],"b":{"b":1}}"#,
            r"(.a | first(.[]) | .) |= empty",
            "{\"a\":[2,3],\"b\":{\"b\":1}}\n",
            "",
            0,
        ),
        (
            r#"{"a":[1,2,3],"b":{"b":1}}"#,
            r"(.a | getpath([0]) | .) |= empty",
            "{\"a\":[2,3],\"b\":{\"b\":1}}\n",
            "",
            0,
        ),
        (
            r"[1,[2],[3]]",
            r"(.[] | .[0]? | .) |= empty",
            "[1,[],[]]\n",
            "",
            0,
        ),
        (
            r#"{"a":[1,2,3],"b":{"b":1}}"#,
            r"(.a | .) = 5",
            "{\"a\":5,\"b\":{\"b\":1}}\n",
            "",
            0,
        ),
        (
            r#"{"a":[1,2,3],"b":{"b":1}}"#,
            r"(.a | .[0] | .) += 1",
            "{\"a\":[2,2,3],\"b\":{\"b\":1}}\n",
            "",
            0,
        ),
        (
            r#"{"a":[1,2,3],"b":{"b":1}}"#,
            r"(.[] | .) |= 1",
            "{\"a\":1,\"b\":1}\n",
            "",
            0,
        ),
    ])
}
