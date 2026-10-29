//! #5499: `$var.field` in a `reduce`/`foreach` UPDATE splices the field's value, not the whole
//! object rebuilt as a literal tree. Every row's expected text was captured from jq 1.7.1
//! (`/usr/bin/jq`); the rows below are the places the splice must give the answer the object
//! would have: an absent field, a non-object, a path-position read, a field that is itself a
//! container.

use anyhow::Result;

use crate::run_jq_stdin;

fn jq(filter: &str, input: &str) -> Result<(String, i32)> {
    run_jq_stdin(filter, input, &["-c"])
}

#[test]
fn test_reduce_reads_a_field_of_each_element_5499() -> Result<()> {
    for (filter, input, want) in [
        (
            "reduce .[] as $u (0; . + $u.score)",
            r#"[{"score":1},{"score":2},{"score":3}]"#,
            "6\n",
        ),
        // An absent field is `null`, and `null + n` is `n`.
        (
            "reduce .[] as $u (0; . + $u.score)",
            r#"[{"score":1},{"other":2},{"score":3}]"#,
            "4\n",
        ),
        (
            "reduce .[] as $u (0; . + $u.score)",
            r#"[{"score":1},null,{"score":3}]"#,
            "4\n",
        ),
        (
            "reduce .[] as $u ({}; .[$u.k] = $u.v)",
            r#"[{"k":"a","v":1},{"k":"b","v":[2]}]"#,
            "{\"a\":1,\"b\":[2]}\n",
        ),
        (
            "reduce .[] as $u ([]; . + $u.l)",
            r#"[{"l":[1]},{"l":[2,3]}]"#,
            "[1,2,3]\n",
        ),
        (
            "reduce .[] as $u (0; . + ($u.a | length))",
            r#"[{"a":"abc"},{"a":[1,2]}]"#,
            "5\n",
        ),
        (
            "[foreach .[] as $u (0; . + $u.v; [., $u.v])]",
            r#"[{"v":1},{"v":2}]"#,
            "[[1,1],[3,2]]\n",
        ),
    ] {
        assert_eq!(
            jq(filter, input)?,
            (want.to_string(), 0),
            "{filter} {input}"
        );
    }
    Ok(())
}

#[test]
fn test_a_field_of_a_non_object_still_errors_5499() -> Result<()> {
    assert_eq!(
        jq("reduce .[] as $u (0; . + $u.score)", r#"[{"score":1},5]"#)?,
        (String::new(), 5)
    );
    Ok(())
}

#[test]
fn test_as_binding_reads_a_field_5499() -> Result<()> {
    for (filter, input, want) in [
        ("[.[] as $u | $u.a, $u.b]", r#"[{"a":1,"b":2}]"#, "[1,2]\n"),
        (
            "[.[] as $u | {x: $u.a} | .x]",
            r#"[{"a":null},{"a":false}]"#,
            "[null,false]\n",
        ),
        // A container field compares by value, as the whole object did.
        (
            ".[] as $u | $u.a == $u.a",
            r#"[{"a":[1,{"b":null}]}]"#,
            "true\n",
        ),
    ] {
        assert_eq!(
            jq(filter, input)?,
            (want.to_string(), 0),
            "{filter} {input}"
        );
    }
    Ok(())
}

#[test]
#[cfg_attr(feature = "unshared-containers", ignore = "holdout refusal (#5246)")]
fn test_a_field_read_in_path_position_still_names_the_object_5499() -> Result<()> {
    // The splice must not change the object the error names.
    assert_eq!(
        jq(".[] as $u | try ($u.a = 3) catch .", r#"[{"a":1}]"#)?,
        (
            "\"Invalid path expression near attempt to access element \\\"a\\\" of {\\\"a\\\":1}\"\n"
                .to_string(),
            0
        )
    );
    Ok(())
}

#[test]
#[cfg_attr(feature = "unshared-containers", ignore = "holdout refusal (#5246)")]
fn test_a_fold_reading_its_variable_in_path_position_still_names_the_object_5499() -> Result<()> {
    // The splice is for a fold whose variable is read as a value. A path read keeps the object.
    let want =
        "\"Invalid path expression near attempt to access element \\\"a\\\" of {\\\"a\\\":1}\"\n";
    assert_eq!(
        jq(
            "reduce .[] as $u (0; try ($u.a = 3) catch .)",
            r#"[{"a":1}]"#
        )?,
        (want.to_string(), 0)
    );
    assert_eq!(
        jq(
            "[foreach .[] as $u (0; try ($u.a = 3) catch .)] | .[0]",
            r#"[{"a":1}]"#
        )?,
        (want.to_string(), 0)
    );
    Ok(())
}
