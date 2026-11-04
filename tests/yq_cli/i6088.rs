//! #6088: yq binds `$i` in `SRC as $i ireduce (INIT; UPDATE)` to the node `SRC` selected, so
//! `$i | path`, `$i | key`, `$i | line` and the other node reads answer from where that node sits.
//! The fold bound the element's value, and a value has no position: `$i | path` was `[]`, which
//! pruned the documented `setpath($i | path; $i)` example to its last value.
//!
//! Every expected output is what yq v4.53.3 prints for the same document and filter.

use anyhow::Result;

use crate::run_yq_stdin;

const DOC: &str = "p: bob\nb:\n  c1: x\n  c2: y\nc:\n  c1: z\n";

/// (name, filter, expected stdout), all over [`DOC`].
const ORACLE_ROWS: &[(&str, &str, &str)] = &[
    (
        "documented_prune_example",
        "(.b.c2, .c.c1) as $i ireduce({}; setpath($i | path; $i))",
        "b:\n  c2: y\nc:\n  c1: z\n",
    ),
    (
        "paths_collected",
        "(.b.c2, .c.c1) as $i ireduce([]; . + [($i|path)])",
        "- - b\n  - c2\n- - c\n  - c1\n",
    ),
    (
        "plain_as_agrees",
        "[(.b.c2, .c.c1) as $i | $i | path]",
        "- - b\n  - c2\n- - c\n  - c1\n",
    ),
    (
        "key",
        ".b[] as $i ireduce([]; . + [($i|key)])",
        "- c1\n- c2\n",
    ),
    (
        "parent",
        ".b[] as $i ireduce([]; . + [($i|parent|keys)])",
        "- - c1\n  - c2\n- - c1\n  - c2\n",
    ),
    (
        "line",
        ".b[] as $i ireduce([]; . + [($i|line)])",
        "- 3\n- 4\n",
    ),
    (
        "column",
        ".b[] as $i ireduce([]; . + [($i|column)])",
        "- 7\n- 7\n",
    ),
    (
        "path_then_more_stages",
        r#".b[] as $i ireduce([]; . + [($i|path|join("."))])"#,
        "- b.c1\n- b.c2\n",
    ),
    (
        "path_bound_by_as",
        ".b[] as $i ireduce([]; . + [($i|path) as $p | $p])",
        "- - b\n  - c1\n- - b\n  - c2\n",
    ),
    (
        "two_reads_in_one_step",
        ".b[] as $i ireduce([]; . + [($i|key), ($i|path|length)])",
        "- c1\n- 2\n- c2\n- 2\n",
    ),
    (
        "one_read_used_twice",
        ".b[] as $i ireduce([]; . + [($i|line), ($i|line)])",
        "- 3\n- 3\n- 4\n- 4\n",
    ),
    (
        "read_after_a_write",
        "(.b.c2, .c.c1) as $i ireduce({}; setpath($i | path; $i) | .n = ($i|path|length))",
        "b:\n  c2: y\nn: 2\nc:\n  c1: z\n",
    ),
    (
        "empty_source_keeps_init",
        ".zz[] as $i ireduce(5; . + ($i|line))",
        "5\n",
    ),
    (
        "computed_element_has_no_path",
        "(1,2) as $i ireduce([]; . + [($i|path)])",
        "- []\n- []\n",
    ),
    (
        "rebound_name_is_not_the_loop_variable",
        ".b[] as $i ireduce([]; . as $i | . + [$i|path])",
        "- []\n- []\n",
    ),
    // No node read: the fold is left as written.
    (
        "value_only_fold_unchanged",
        r#".b[] as $i ireduce(""; . + $i)"#,
        "xy\n",
    ),
];

#[test]
fn ireduce_loop_variable_answers_node_reads_from_its_node_6088() -> Result<()> {
    for &(name, filter, expected) in ORACLE_ROWS {
        let (output, code) = run_yq_stdin(filter, DOC, &[])?;
        assert_eq!(code, 0, "{name}");
        assert_eq!(output, expected, "{name}: `{filter}`");
    }
    Ok(())
}
