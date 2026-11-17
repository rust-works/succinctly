//! #4476: a value derived from a navigated node keeps the node's slot in yq (`CopyAsReplacement`),
//! so `del(.b | tostring)` deletes `.b` and `del(.arr[] | tostring)` empties `.arr`; succinctly's
//! untracked branches carry no provenance and refused with `Invalid path expression`. A pipe of
//! navigation ending in `tostring`, `length`, `not`, `@json`, `tojson` or `type` (stages that answer every
//! node) now deletes the navigation, through `?`, parentheses and commas, with or without a `select(...)`
//! stage in between (the last test). The bare-root slice rows of the same issue were matched by #1325
//! (PR #4979).
//!
//! Every output is captured from yq v4.53.3.

use anyhow::Result;

use crate::run_yq_stdin;

const DOC: &str =
    r#"{"a":1,"b":2,"c":3,"s":"x","arr":[1,2,3],"o":{"p":5,"q":6},"t":true,"z":null}"#;

/// (filter, `yq -o=json -I0` output over [`DOC`]).
const ROWS: &[(&str, &str)] = &[
    ("del(.b | tostring)", "{\"a\":1,\"c\":3,\"s\":\"x\",\"arr\":[1,2,3],\"o\":{\"p\":5,\"q\":6},\"t\":true,\"z\":null}\n"),
    ("del(.b | length)", "{\"a\":1,\"c\":3,\"s\":\"x\",\"arr\":[1,2,3],\"o\":{\"p\":5,\"q\":6},\"t\":true,\"z\":null}\n"),
    ("del(.arr | length)", "{\"a\":1,\"b\":2,\"c\":3,\"s\":\"x\",\"o\":{\"p\":5,\"q\":6},\"t\":true,\"z\":null}\n"),
    ("del(.b | not)", "{\"a\":1,\"c\":3,\"s\":\"x\",\"arr\":[1,2,3],\"o\":{\"p\":5,\"q\":6},\"t\":true,\"z\":null}\n"),
    ("del(.b | @json)", "{\"a\":1,\"c\":3,\"s\":\"x\",\"arr\":[1,2,3],\"o\":{\"p\":5,\"q\":6},\"t\":true,\"z\":null}\n"),
    ("del(.arr | .[0] | tostring)", "{\"a\":1,\"b\":2,\"c\":3,\"s\":\"x\",\"arr\":[2,3],\"o\":{\"p\":5,\"q\":6},\"t\":true,\"z\":null}\n"),
    ("del((.b | tostring), .c)", "{\"a\":1,\"s\":\"x\",\"arr\":[1,2,3],\"o\":{\"p\":5,\"q\":6},\"t\":true,\"z\":null}\n"),
    ("del(.c, (.b | tostring))", "{\"a\":1,\"s\":\"x\",\"arr\":[1,2,3],\"o\":{\"p\":5,\"q\":6},\"t\":true,\"z\":null}\n"),
    ("del(.arr[] | tostring)", "{\"a\":1,\"b\":2,\"c\":3,\"s\":\"x\",\"arr\":[],\"o\":{\"p\":5,\"q\":6},\"t\":true,\"z\":null}\n"),
    ("del(.o.p | tostring)", "{\"a\":1,\"b\":2,\"c\":3,\"s\":\"x\",\"arr\":[1,2,3],\"o\":{\"q\":6},\"t\":true,\"z\":null}\n"),
    ("del(.o[] | tostring)", "{\"a\":1,\"b\":2,\"c\":3,\"s\":\"x\",\"arr\":[1,2,3],\"o\":{},\"t\":true,\"z\":null}\n"),
    ("del(.o | length)", "{\"a\":1,\"b\":2,\"c\":3,\"s\":\"x\",\"arr\":[1,2,3],\"t\":true,\"z\":null}\n"),
    ("del(.zz | tostring)", "{\"a\":1,\"b\":2,\"c\":3,\"s\":\"x\",\"arr\":[1,2,3],\"o\":{\"p\":5,\"q\":6},\"t\":true,\"z\":null}\n"),
    ("del(.b | tostring | tostring)", "{\"a\":1,\"c\":3,\"s\":\"x\",\"arr\":[1,2,3],\"o\":{\"p\":5,\"q\":6},\"t\":true,\"z\":null}\n"),
    ("del(.t | length)", "{\"a\":1,\"b\":2,\"c\":3,\"s\":\"x\",\"arr\":[1,2,3],\"o\":{\"p\":5,\"q\":6},\"z\":null}\n"),
    ("del(.z | tostring)", "{\"a\":1,\"b\":2,\"c\":3,\"s\":\"x\",\"arr\":[1,2,3],\"o\":{\"p\":5,\"q\":6},\"t\":true}\n"),
    ("del(.z | length)", "{\"a\":1,\"b\":2,\"c\":3,\"s\":\"x\",\"arr\":[1,2,3],\"o\":{\"p\":5,\"q\":6},\"t\":true}\n"),
    ("del((.b | tostring), 1)", "{\"a\":1,\"b\":2,\"c\":3,\"s\":\"x\",\"arr\":[1,2,3],\"o\":{\"p\":5,\"q\":6},\"t\":true,\"z\":null}\n"),
    ("del(1, (.b | tostring))", "{\"a\":1,\"c\":3,\"s\":\"x\",\"arr\":[1,2,3],\"o\":{\"p\":5,\"q\":6},\"t\":true,\"z\":null}\n"),
    ("del(.arr[1:2] | tostring)", "{\"a\":1,\"b\":2,\"c\":3,\"s\":\"x\",\"o\":{\"p\":5,\"q\":6},\"t\":true,\"z\":null}\n"),
    ("del(.o | not)", "{\"a\":1,\"b\":2,\"c\":3,\"s\":\"x\",\"arr\":[1,2,3],\"t\":true,\"z\":null}\n"),
    ("del(.b | tostring), 1", "{\"a\":1,\"c\":3,\"s\":\"x\",\"arr\":[1,2,3],\"o\":{\"p\":5,\"q\":6},\"t\":true,\"z\":null}\n1\n"),
    ("del(.s | tojson)", "{\"a\":1,\"b\":2,\"c\":3,\"arr\":[1,2,3],\"o\":{\"p\":5,\"q\":6},\"t\":true,\"z\":null}\n"),
    ("del(.s | type)", "{\"a\":1,\"b\":2,\"c\":3,\"arr\":[1,2,3],\"o\":{\"p\":5,\"q\":6},\"t\":true,\"z\":null}\n"),
    ("del(.o | tojson)", "{\"a\":1,\"b\":2,\"c\":3,\"s\":\"x\",\"arr\":[1,2,3],\"t\":true,\"z\":null}\n"),
    ("del((.a, .b) | tostring)", "{\"c\":3,\"s\":\"x\",\"arr\":[1,2,3],\"o\":{\"p\":5,\"q\":6},\"t\":true,\"z\":null}\n"),
    ("del(.b? | tostring)", "{\"a\":1,\"c\":3,\"s\":\"x\",\"arr\":[1,2,3],\"o\":{\"p\":5,\"q\":6},\"t\":true,\"z\":null}\n"),
    ("del(.zz? | tostring)", "{\"a\":1,\"b\":2,\"c\":3,\"s\":\"x\",\"arr\":[1,2,3],\"o\":{\"p\":5,\"q\":6},\"t\":true,\"z\":null}\n"),
    ("del(.arr[]? | type)", "{\"a\":1,\"b\":2,\"c\":3,\"s\":\"x\",\"arr\":[],\"o\":{\"p\":5,\"q\":6},\"t\":true,\"z\":null}\n"),
    ("del(((.a, .b), .c) | tostring)", "{\"s\":\"x\",\"arr\":[1,2,3],\"o\":{\"p\":5,\"q\":6},\"t\":true,\"z\":null}\n"),
];

#[test]
fn test_4476_a_a_derived_target_deletes_the_node_it_derives_from() -> Result<()> {
    for (filter, want) in ROWS {
        let (out, code) = run_yq_stdin(filter, DOC, &["-o=json", "-I=0"])?;
        assert_eq!(code, 0, "{filter}");
        assert_eq!(&out, want, "{filter}");
    }
    Ok(())
}

/// Arithmetic on a node and the other derivations are still refused (a type error such as
/// `.b + [1]` is an error in yq, and a rewrite would turn it into a delete), as is a derived
/// document root, which yq keeps (`del(.. | tostring)` is `{}`). Recorded in `limitations.md`.
#[test]
fn test_4476_b_the_other_derived_targets_are_still_refused() -> Result<()> {
    for filter in [
        "del(.b + 1)",
        "del(.b | . + [1])",
        "del(.arr | map(.))",
        "del(. | tostring)",
        "del(.. | tostring)",
        "del((. , .a) | tostring)",
    ] {
        let (out, code) = run_yq_stdin(filter, DOC, &["-o=json", "-I=0"])?;
        assert_eq!((out.as_str(), code), ("", 1), "{filter}");
    }
    Ok(())
}

/// The same, with a `select` between the navigation and the deriving stage: it hands its input on
/// with its slot, so `del(.arr[] | select(. > 1) | tostring)` deletes the elements the filter kept, and
/// a target that selects nothing is a target with no parent that stops yq's back-to-front loop
/// (`del(.a | select(false), 1)` leaves the document, where it raised). Captured from yq v4.53.3
/// over [`SELECT_DOC`].
const SELECT_DOC: &str = r#"{"a":1,"b":2,"c":3,"s":"x","arr":[1,2,3],"o":{"p":5,"q":6},"t":true,"z":null,"l":[{"k":1},{"k":2}]}"#;

const SELECT_ROWS: &[(&str, &str)] = &[
    ("del(.a | select(false), 1)", "{\"a\":1,\"b\":2,\"c\":3,\"s\":\"x\",\"arr\":[1,2,3],\"o\":{\"p\":5,\"q\":6},\"t\":true,\"z\":null,\"l\":[{\"k\":1},{\"k\":2}]}\n"),
    ("del(1, .a | select(false))", "{\"a\":1,\"b\":2,\"c\":3,\"s\":\"x\",\"arr\":[1,2,3],\"o\":{\"p\":5,\"q\":6},\"t\":true,\"z\":null,\"l\":[{\"k\":1},{\"k\":2}]}\n"),
    ("del(.arr[] | select(. > 1) | tostring)", "{\"a\":1,\"b\":2,\"c\":3,\"s\":\"x\",\"arr\":[1],\"o\":{\"p\":5,\"q\":6},\"t\":true,\"z\":null,\"l\":[{\"k\":1},{\"k\":2}]}\n"),
    ("del(.arr[] | select(. > 1) | length)", "{\"a\":1,\"b\":2,\"c\":3,\"s\":\"x\",\"arr\":[1],\"o\":{\"p\":5,\"q\":6},\"t\":true,\"z\":null,\"l\":[{\"k\":1},{\"k\":2}]}\n"),
    ("del(.arr[] | select(. > 1) | not)", "{\"a\":1,\"b\":2,\"c\":3,\"s\":\"x\",\"arr\":[1],\"o\":{\"p\":5,\"q\":6},\"t\":true,\"z\":null,\"l\":[{\"k\":1},{\"k\":2}]}\n"),
    ("del(.arr[] | select(. > 1) | @json)", "{\"a\":1,\"b\":2,\"c\":3,\"s\":\"x\",\"arr\":[1],\"o\":{\"p\":5,\"q\":6},\"t\":true,\"z\":null,\"l\":[{\"k\":1},{\"k\":2}]}\n"),
    ("del(.arr[] | select(. > 1) | type)", "{\"a\":1,\"b\":2,\"c\":3,\"s\":\"x\",\"arr\":[1],\"o\":{\"p\":5,\"q\":6},\"t\":true,\"z\":null,\"l\":[{\"k\":1},{\"k\":2}]}\n"),
    ("del(.arr[] | select(. > 1) | tojson)", "{\"a\":1,\"b\":2,\"c\":3,\"s\":\"x\",\"arr\":[1],\"o\":{\"p\":5,\"q\":6},\"t\":true,\"z\":null,\"l\":[{\"k\":1},{\"k\":2}]}\n"),
    ("del(.a | select(true) | tostring)", "{\"b\":2,\"c\":3,\"s\":\"x\",\"arr\":[1,2,3],\"o\":{\"p\":5,\"q\":6},\"t\":true,\"z\":null,\"l\":[{\"k\":1},{\"k\":2}]}\n"),
    ("del(.a | select(false) | tostring)", "{\"a\":1,\"b\":2,\"c\":3,\"s\":\"x\",\"arr\":[1,2,3],\"o\":{\"p\":5,\"q\":6},\"t\":true,\"z\":null,\"l\":[{\"k\":1},{\"k\":2}]}\n"),
    ("del(.l[] | select(.k > 1) | tostring)", "{\"a\":1,\"b\":2,\"c\":3,\"s\":\"x\",\"arr\":[1,2,3],\"o\":{\"p\":5,\"q\":6},\"t\":true,\"z\":null,\"l\":[{\"k\":1}]}\n"),
    ("del((.a, .b) | select(. > 1) | tostring)", "{\"a\":1,\"c\":3,\"s\":\"x\",\"arr\":[1,2,3],\"o\":{\"p\":5,\"q\":6},\"t\":true,\"z\":null,\"l\":[{\"k\":1},{\"k\":2}]}\n"),
    ("del(.o[] | select(. > 5) | tostring)", "{\"a\":1,\"b\":2,\"c\":3,\"s\":\"x\",\"arr\":[1,2,3],\"o\":{\"p\":5},\"t\":true,\"z\":null,\"l\":[{\"k\":1},{\"k\":2}]}\n"),
    ("del(.arr[] | select(. > 1) | select(. > 2) | tostring)", "{\"a\":1,\"b\":2,\"c\":3,\"s\":\"x\",\"arr\":[1,2],\"o\":{\"p\":5,\"q\":6},\"t\":true,\"z\":null,\"l\":[{\"k\":1},{\"k\":2}]}\n"),
    ("del(.arr[] | select(false) | tostring)", "{\"a\":1,\"b\":2,\"c\":3,\"s\":\"x\",\"arr\":[1,2,3],\"o\":{\"p\":5,\"q\":6},\"t\":true,\"z\":null,\"l\":[{\"k\":1},{\"k\":2}]}\n"),
    ("del((.arr[] | select(. > 1) | tostring), .a)", "{\"b\":2,\"c\":3,\"s\":\"x\",\"arr\":[1],\"o\":{\"p\":5,\"q\":6},\"t\":true,\"z\":null,\"l\":[{\"k\":1},{\"k\":2}]}\n"),
    ("del(.a, (.arr[] | select(. > 1) | tostring))", "{\"b\":2,\"c\":3,\"s\":\"x\",\"arr\":[1],\"o\":{\"p\":5,\"q\":6},\"t\":true,\"z\":null,\"l\":[{\"k\":1},{\"k\":2}]}\n"),
    ("del(.zz | select(.) | tostring)", "{\"a\":1,\"b\":2,\"c\":3,\"s\":\"x\",\"arr\":[1,2,3],\"o\":{\"p\":5,\"q\":6},\"t\":true,\"z\":null,\"l\":[{\"k\":1},{\"k\":2}]}\n"),
    ("del(.a | select(false), .b)", "{\"a\":1,\"c\":3,\"s\":\"x\",\"arr\":[1,2,3],\"o\":{\"p\":5,\"q\":6},\"t\":true,\"z\":null,\"l\":[{\"k\":1},{\"k\":2}]}\n"),
    ("del(.b, .a | select(false))", "{\"a\":1,\"c\":3,\"s\":\"x\",\"arr\":[1,2,3],\"o\":{\"p\":5,\"q\":6},\"t\":true,\"z\":null,\"l\":[{\"k\":1},{\"k\":2}]}\n"),
    ("del(.c | select(true), 1)", "{\"a\":1,\"b\":2,\"c\":3,\"s\":\"x\",\"arr\":[1,2,3],\"o\":{\"p\":5,\"q\":6},\"t\":true,\"z\":null,\"l\":[{\"k\":1},{\"k\":2}]}\n"),
    ("del(.arr[]? | select(. > 1) | tostring)", "{\"a\":1,\"b\":2,\"c\":3,\"s\":\"x\",\"arr\":[1],\"o\":{\"p\":5,\"q\":6},\"t\":true,\"z\":null,\"l\":[{\"k\":1},{\"k\":2}]}\n"),
    ("del((.arr[] | select(. > 1)) | tostring)", "{\"a\":1,\"b\":2,\"c\":3,\"s\":\"x\",\"arr\":[1],\"o\":{\"p\":5,\"q\":6},\"t\":true,\"z\":null,\"l\":[{\"k\":1},{\"k\":2}]}\n"),
    ("del(.l[] | select(.[0] > 1) | tostring)", "{\"a\":1,\"b\":2,\"c\":3,\"s\":\"x\",\"arr\":[1,2,3],\"o\":{\"p\":5,\"q\":6},\"t\":true,\"z\":null,\"l\":[{\"k\":1},{\"k\":2}]}\n"),
    ("del(.o | select(.p) | length)", "{\"a\":1,\"b\":2,\"c\":3,\"s\":\"x\",\"arr\":[1,2,3],\"t\":true,\"z\":null,\"l\":[{\"k\":1},{\"k\":2}]}\n"),
    ("del(.arr | select(length > 1) | length)", "{\"a\":1,\"b\":2,\"c\":3,\"s\":\"x\",\"o\":{\"p\":5,\"q\":6},\"t\":true,\"z\":null,\"l\":[{\"k\":1},{\"k\":2}]}\n"),
    ("del(select(true))", ""),
    ("del(select(false))", "{\"a\":1,\"b\":2,\"c\":3,\"s\":\"x\",\"arr\":[1,2,3],\"o\":{\"p\":5,\"q\":6},\"t\":true,\"z\":null,\"l\":[{\"k\":1},{\"k\":2}]}\n"),
];

#[test]
fn test_4476_c_a_select_stage_keeps_the_slot_of_its_input() -> Result<()> {
    for (filter, want) in SELECT_ROWS {
        let (out, code) = run_yq_stdin(filter, SELECT_DOC, &["-o=json", "-I=0"])?;
        assert_eq!(code, 0, "{filter}");
        assert_eq!(&out, want, "{filter}");
    }
    Ok(())
}

/// What stays refused after a `select`: arithmetic or `map` after it, and a `select` after the
/// deriving stage. yq deletes the node there, and a rewrite that only looked at the `select` would
/// have to be sure of the stage after it, which these are not (`.b + [1]` is a type error in yq).
#[test]
fn test_4476_d_a_select_before_arithmetic_or_after_the_derived_value_is_still_refused() -> Result<()>
{
    for filter in [
        "del(.arr[] | select(. > 1) | . + 1)",
        "del(.arr[] | select(. > 1) | map(.))",
        "del(.a | select(.) | . + 1)",
        "del(.a | tostring | select(true))",
    ] {
        let (out, code) = run_yq_stdin(filter, SELECT_DOC, &["-o=json", "-I=0"])?;
        assert_eq!((out.as_str(), code), ("", 1), "{filter}");
    }
    Ok(())
}
