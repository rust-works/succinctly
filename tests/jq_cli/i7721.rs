//! #7721: `$a | getpath(P)` over a bound container, with a path `P` that never reads `.`, re-indexed
//! the whole container for every read: inside a `reduce`/`foreach`/`until` step the closed-pipe walk
//! had no `getpath` stage, so the step declined to the evaluator's route, and in a body that reads
//! once per output (`[range(n) as $i | $a | getpath([$i])]`, `limit`) the owned doors had no
//! `getpath` door, so the read crossed the reindex bridge. Linear per read, quadratic over the run,
//! where jq 1.7.1 reads the element in place. Both now walk the bound value, through the walk the
//! evaluator's own `getpath` of an owned value takes; a path that raises or is not an array still
//! takes the evaluator's route and its text.
//!
//! Every expected line was captured from jq 1.7.1 (`/usr/bin/jq`); jq 1.8.2 answers the same but for
//! its reworded `Cannot index` messages (a 1.8 improvement, kept at 1.7.1) and `-0` (no row here).
//! The rows are reads of literal, computed, mixed-number, document, `input` and `--argjson`
//! containers in each fold kind. The in-place proofs are the unit tests
//! `getpath_of_a_bound_container_reads_in_place_7721` and
//! `fold_step_storing_a_getpath_read_copies_nothing_7721` in `src/jq/eval.rs`.

use anyhow::Result;

use crate::run_jq_full;

/// (arguments, stdin, stdout, stderr, exit code) for `succinctly jq <arguments>`.
type Row = (
    &'static [&'static str],
    Option<&'static str>,
    &'static str,
    &'static str,
    i32,
);

const ROWS: &[Row] = &[
    // lit_n/reduce/getpath
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | reduce range(6) as $i ([]; . + [($a|getpath([$i]))])"], None, "[0,1,2,3,4,5]\n", "", 0),
    // lit_n/reduce/gp_neg
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | reduce range(6) as $i ([]; . + [($a|getpath([-1-$i]))])"], None, "[7,6,5,4,3,2]\n", "", 0),
    // lit_n/reduce/gp_float
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | reduce range(6) as $i ([]; . + [($a|getpath([$i+0.5]))])"], None, "[0,1,2,3,4,5]\n", "", 0),
    // lit_n/reduce/gp_slice
    (&[r"-c", r"-n", r#"[range(8)] as $a | {k: $a} as $o | reduce range(6) as $i ([]; . + [($a|getpath([{"start":$i,"end":($i+2)}]))])"#], None, "[[0,1],[1,2],[2,3],[3,4],[4,5],[5,6]]\n", "", 0),
    // lit_n/reduce/gp_sub
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | reduce range(6) as $i ([]; . + [($a|getpath([[$i]]))])"], None, "[[0],[1],[2],[3],[4],[5]]\n", "", 0),
    // lit_n/reduce/gp_empty
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | reduce range(6) as $i ([]; . + [($a|getpath([])|length)])"], None, "[8,8,8,8,8,8]\n", "", 0),
    // lit_n/reduce/gp_deep
    (&[r"-c", r"-n", r#"[range(8)] as $a | {k: $a} as $o | reduce range(6) as $i ([]; . + [($a|getpath([$i,"x"])?)])"#], None, "[]\n", "", 0),
    // lit_n/reduce/gp_err
    (&[r"-c", r"-n", r#"[range(8)] as $a | {k: $a} as $o | reduce range(6) as $i ([]; . + [($a|try getpath(["x"]) catch .)])"#], None, "[\"Cannot index array with string \\\"x\\\"\",\"Cannot index array with string \\\"x\\\"\",\"Cannot index array with string \\\"x\\\"\",\"Cannot index array with string \\\"x\\\"\",\"Cannot index array with string \\\"x\\\"\",\"Cannot index array with string \\\"x\\\"\"]\n", "", 0),
    // lit_n/reduce/gp_nullpath
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | reduce range(6) as $i ([]; . + [($a|try getpath(null) catch .)])"], None, "[\"Path must be specified as an array\",\"Path must be specified as an array\",\"Path must be specified as an array\",\"Path must be specified as an array\",\"Path must be specified as an array\",\"Path must be specified as an array\"]\n", "", 0),
    // lit_n/reduce/gp_obj
    (&[r"-c", r"-n", r#"[range(8)] as $a | {k: $a} as $o | reduce range(6) as $i ([]; . + [($o|getpath(["k",$i]))])"#], None, "[0,1,2,3,4,5]\n", "", 0),
    // lit_n/reduce/gp_objmiss
    (&[r"-c", r"-n", r#"[range(8)] as $a | {k: $a} as $o | reduce range(6) as $i ([]; . + [($o|getpath(["m",$i,"z"]))])"#], None, "[null,null,null,null,null,null]\n", "", 0),
    // lit_n/reduce/gp_objk
    (&[r"-c", r"-n", r#"[range(8)] as $a | {k: $a} as $o | reduce range(6) as $i ([]; . + [($o|getpath(["k"])|length)])"#], None, "[8,8,8,8,8,8]\n", "", 0),
    // lit_n/reduce/gp_then
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | reduce range(6) as $i ([]; . + [($a|getpath([$i])|tojson)])"], None, "[\"0\",\"1\",\"2\",\"3\",\"4\",\"5\"]\n", "", 0),
    // lit_n/reduce/gp_two
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | reduce range(6) as $i ([]; . + [($a|getpath([$i],[$i+1]))])"], None, "[0,1,1,2,2,3,3,4,4,5,5,6]\n", "", 0),
    // lit_n/reduce/gp_gen_path
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | reduce range(6) as $i ([]; . + [($a|getpath([$i,$i] | .[0:1]))])"], None, "[0,1,2,3,4,5]\n", "", 0),
    // lit_n/reduce/gp_var_path
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | reduce range(6) as $i ([]; . + [([$i] as $p | $a|getpath($p))])"], None, "[0,1,2,3,4,5]\n", "", 0),
    // lit_n/reduce/gp_in_if
    (&[r"-c", r"-n", r#"[range(8)] as $a | {k: $a} as $o | reduce range(6) as $i ([]; . + [(if ($a|getpath([$i])) == null then "n" else "v" end)])"#], None, "[\"v\",\"v\",\"v\",\"v\",\"v\",\"v\"]\n", "", 0),
    // lit_n/reduce_set/getpath
    (&[r"-c", r"-n", r#"[range(8)] as $a | {k: $a} as $o | reduce range(6) as $i ({}; .["\($i)"] = (($a|getpath([$i]))))"#], None, "{\"0\":0,\"1\":1,\"2\":2,\"3\":3,\"4\":4,\"5\":5}\n", "", 0),
    // lit_n/reduce_set/gp_neg
    (&[r"-c", r"-n", r#"[range(8)] as $a | {k: $a} as $o | reduce range(6) as $i ({}; .["\($i)"] = (($a|getpath([-1-$i]))))"#], None, "{\"0\":7,\"1\":6,\"2\":5,\"3\":4,\"4\":3,\"5\":2}\n", "", 0),
    // lit_n/reduce_set/gp_float
    (&[r"-c", r"-n", r#"[range(8)] as $a | {k: $a} as $o | reduce range(6) as $i ({}; .["\($i)"] = (($a|getpath([$i+0.5]))))"#], None, "{\"0\":0,\"1\":1,\"2\":2,\"3\":3,\"4\":4,\"5\":5}\n", "", 0),
    // lit_n/reduce_set/gp_slice
    (&[r"-c", r"-n", r#"[range(8)] as $a | {k: $a} as $o | reduce range(6) as $i ({}; .["\($i)"] = (($a|getpath([{"start":$i,"end":($i+2)}]))))"#], None, "{\"0\":[0,1],\"1\":[1,2],\"2\":[2,3],\"3\":[3,4],\"4\":[4,5],\"5\":[5,6]}\n", "", 0),
    // lit_n/reduce_set/gp_sub
    (&[r"-c", r"-n", r#"[range(8)] as $a | {k: $a} as $o | reduce range(6) as $i ({}; .["\($i)"] = (($a|getpath([[$i]]))))"#], None, "{\"0\":[0],\"1\":[1],\"2\":[2],\"3\":[3],\"4\":[4],\"5\":[5]}\n", "", 0),
    // lit_n/reduce_set/gp_empty
    (&[r"-c", r"-n", r#"[range(8)] as $a | {k: $a} as $o | reduce range(6) as $i ({}; .["\($i)"] = (($a|getpath([])|length)))"#], None, "{\"0\":8,\"1\":8,\"2\":8,\"3\":8,\"4\":8,\"5\":8}\n", "", 0),
    // lit_n/reduce_set/gp_deep
    (&[r"-c", r"-n", r#"[range(8)] as $a | {k: $a} as $o | reduce range(6) as $i ({}; .["\($i)"] = (($a|getpath([$i,"x"])?)))"#], None, "null\n", "", 0),
    // lit_n/reduce_set/gp_err
    (&[r"-c", r"-n", r#"[range(8)] as $a | {k: $a} as $o | reduce range(6) as $i ({}; .["\($i)"] = (($a|try getpath(["x"]) catch .)))"#], None, "{\"0\":\"Cannot index array with string \\\"x\\\"\",\"1\":\"Cannot index array with string \\\"x\\\"\",\"2\":\"Cannot index array with string \\\"x\\\"\",\"3\":\"Cannot index array with string \\\"x\\\"\",\"4\":\"Cannot index array with string \\\"x\\\"\",\"5\":\"Cannot index array with string \\\"x\\\"\"}\n", "", 0),
    // lit_n/reduce_set/gp_nullpath
    (&[r"-c", r"-n", r#"[range(8)] as $a | {k: $a} as $o | reduce range(6) as $i ({}; .["\($i)"] = (($a|try getpath(null) catch .)))"#], None, "{\"0\":\"Path must be specified as an array\",\"1\":\"Path must be specified as an array\",\"2\":\"Path must be specified as an array\",\"3\":\"Path must be specified as an array\",\"4\":\"Path must be specified as an array\",\"5\":\"Path must be specified as an array\"}\n", "", 0),
    // lit_n/reduce_set/gp_obj
    (&[r"-c", r"-n", r#"[range(8)] as $a | {k: $a} as $o | reduce range(6) as $i ({}; .["\($i)"] = (($o|getpath(["k",$i]))))"#], None, "{\"0\":0,\"1\":1,\"2\":2,\"3\":3,\"4\":4,\"5\":5}\n", "", 0),
    // lit_n/reduce_set/gp_objmiss
    (&[r"-c", r"-n", r#"[range(8)] as $a | {k: $a} as $o | reduce range(6) as $i ({}; .["\($i)"] = (($o|getpath(["m",$i,"z"]))))"#], None, "{\"0\":null,\"1\":null,\"2\":null,\"3\":null,\"4\":null,\"5\":null}\n", "", 0),
    // lit_n/reduce_set/gp_objk
    (&[r"-c", r"-n", r#"[range(8)] as $a | {k: $a} as $o | reduce range(6) as $i ({}; .["\($i)"] = (($o|getpath(["k"])|length)))"#], None, "{\"0\":8,\"1\":8,\"2\":8,\"3\":8,\"4\":8,\"5\":8}\n", "", 0),
    // lit_n/reduce_set/gp_then
    (&[r"-c", r"-n", r#"[range(8)] as $a | {k: $a} as $o | reduce range(6) as $i ({}; .["\($i)"] = (($a|getpath([$i])|tojson)))"#], None, "{\"0\":\"0\",\"1\":\"1\",\"2\":\"2\",\"3\":\"3\",\"4\":\"4\",\"5\":\"5\"}\n", "", 0),
    // lit_n/reduce_set/gp_two
    (&[r"-c", r"-n", r#"[range(8)] as $a | {k: $a} as $o | reduce range(6) as $i ({}; .["\($i)"] = (($a|getpath([$i],[$i+1]))))"#], None, "{\"0\":1,\"1\":2,\"2\":3,\"3\":4,\"4\":5,\"5\":6}\n", "", 0),
    // lit_n/reduce_set/gp_gen_path
    (&[r"-c", r"-n", r#"[range(8)] as $a | {k: $a} as $o | reduce range(6) as $i ({}; .["\($i)"] = (($a|getpath([$i,$i] | .[0:1]))))"#], None, "{\"0\":0,\"1\":1,\"2\":2,\"3\":3,\"4\":4,\"5\":5}\n", "", 0),
    // lit_n/reduce_set/gp_var_path
    (&[r"-c", r"-n", r#"[range(8)] as $a | {k: $a} as $o | reduce range(6) as $i ({}; .["\($i)"] = (([$i] as $p | $a|getpath($p))))"#], None, "{\"0\":0,\"1\":1,\"2\":2,\"3\":3,\"4\":4,\"5\":5}\n", "", 0),
    // lit_n/reduce_set/gp_in_if
    (&[r"-c", r"-n", r#"[range(8)] as $a | {k: $a} as $o | reduce range(6) as $i ({}; .["\($i)"] = ((if ($a|getpath([$i])) == null then "n" else "v" end)))"#], None, "{\"0\":\"v\",\"1\":\"v\",\"2\":\"v\",\"3\":\"v\",\"4\":\"v\",\"5\":\"v\"}\n", "", 0),
    // lit_n/foreach/getpath
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | [foreach range(6) as $i (null; ($a|getpath([$i])))]"], None, "[0,1,2,3,4,5]\n", "", 0),
    // lit_n/foreach/gp_neg
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | [foreach range(6) as $i (null; ($a|getpath([-1-$i])))]"], None, "[7,6,5,4,3,2]\n", "", 0),
    // lit_n/foreach/gp_float
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | [foreach range(6) as $i (null; ($a|getpath([$i+0.5])))]"], None, "[0,1,2,3,4,5]\n", "", 0),
    // lit_n/foreach/gp_slice
    (&[r"-c", r"-n", r#"[range(8)] as $a | {k: $a} as $o | [foreach range(6) as $i (null; ($a|getpath([{"start":$i,"end":($i+2)}])))]"#], None, "[[0,1],[1,2],[2,3],[3,4],[4,5],[5,6]]\n", "", 0),
    // lit_n/foreach/gp_sub
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | [foreach range(6) as $i (null; ($a|getpath([[$i]])))]"], None, "[[0],[1],[2],[3],[4],[5]]\n", "", 0),
    // lit_n/foreach/gp_empty
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | [foreach range(6) as $i (null; ($a|getpath([])|length))]"], None, "[8,8,8,8,8,8]\n", "", 0),
    // lit_n/foreach/gp_deep
    (&[r"-c", r"-n", r#"[range(8)] as $a | {k: $a} as $o | [foreach range(6) as $i (null; ($a|getpath([$i,"x"])?))]"#], None, "[]\n", "", 0),
    // lit_n/foreach/gp_err
    (&[r"-c", r"-n", r#"[range(8)] as $a | {k: $a} as $o | [foreach range(6) as $i (null; ($a|try getpath(["x"]) catch .))]"#], None, "[\"Cannot index array with string \\\"x\\\"\",\"Cannot index array with string \\\"x\\\"\",\"Cannot index array with string \\\"x\\\"\",\"Cannot index array with string \\\"x\\\"\",\"Cannot index array with string \\\"x\\\"\",\"Cannot index array with string \\\"x\\\"\"]\n", "", 0),
    // lit_n/foreach/gp_nullpath
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | [foreach range(6) as $i (null; ($a|try getpath(null) catch .))]"], None, "[\"Path must be specified as an array\",\"Path must be specified as an array\",\"Path must be specified as an array\",\"Path must be specified as an array\",\"Path must be specified as an array\",\"Path must be specified as an array\"]\n", "", 0),
    // lit_n/foreach/gp_obj
    (&[r"-c", r"-n", r#"[range(8)] as $a | {k: $a} as $o | [foreach range(6) as $i (null; ($o|getpath(["k",$i])))]"#], None, "[0,1,2,3,4,5]\n", "", 0),
    // lit_n/foreach/gp_objmiss
    (&[r"-c", r"-n", r#"[range(8)] as $a | {k: $a} as $o | [foreach range(6) as $i (null; ($o|getpath(["m",$i,"z"])))]"#], None, "[null,null,null,null,null,null]\n", "", 0),
    // lit_n/foreach/gp_objk
    (&[r"-c", r"-n", r#"[range(8)] as $a | {k: $a} as $o | [foreach range(6) as $i (null; ($o|getpath(["k"])|length))]"#], None, "[8,8,8,8,8,8]\n", "", 0),
    // lit_n/foreach/gp_then
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | [foreach range(6) as $i (null; ($a|getpath([$i])|tojson))]"], None, "[\"0\",\"1\",\"2\",\"3\",\"4\",\"5\"]\n", "", 0),
    // lit_n/foreach/gp_two
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | [foreach range(6) as $i (null; ($a|getpath([$i],[$i+1])))]"], None, "[0,1,1,2,2,3,3,4,4,5,5,6]\n", "", 0),
    // lit_n/foreach/gp_gen_path
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | [foreach range(6) as $i (null; ($a|getpath([$i,$i] | .[0:1])))]"], None, "[0,1,2,3,4,5]\n", "", 0),
    // lit_n/foreach/gp_var_path
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | [foreach range(6) as $i (null; ([$i] as $p | $a|getpath($p)))]"], None, "[0,1,2,3,4,5]\n", "", 0),
    // lit_n/foreach/gp_in_if
    (&[r"-c", r"-n", r#"[range(8)] as $a | {k: $a} as $o | [foreach range(6) as $i (null; (if ($a|getpath([$i])) == null then "n" else "v" end))]"#], None, "[\"v\",\"v\",\"v\",\"v\",\"v\",\"v\"]\n", "", 0),
    // lit_n/foreach_acc/getpath
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | [foreach range(6) as $i ([]; . + [($a|getpath([$i]))]; length)]"], None, "[1,2,3,4,5,6]\n", "", 0),
    // lit_n/foreach_acc/gp_neg
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | [foreach range(6) as $i ([]; . + [($a|getpath([-1-$i]))]; length)]"], None, "[1,2,3,4,5,6]\n", "", 0),
    // lit_n/foreach_acc/gp_float
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | [foreach range(6) as $i ([]; . + [($a|getpath([$i+0.5]))]; length)]"], None, "[1,2,3,4,5,6]\n", "", 0),
    // lit_n/foreach_acc/gp_slice
    (&[r"-c", r"-n", r#"[range(8)] as $a | {k: $a} as $o | [foreach range(6) as $i ([]; . + [($a|getpath([{"start":$i,"end":($i+2)}]))]; length)]"#], None, "[1,2,3,4,5,6]\n", "", 0),
    // lit_n/foreach_acc/gp_sub
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | [foreach range(6) as $i ([]; . + [($a|getpath([[$i]]))]; length)]"], None, "[1,2,3,4,5,6]\n", "", 0),
    // lit_n/foreach_acc/gp_empty
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | [foreach range(6) as $i ([]; . + [($a|getpath([])|length)]; length)]"], None, "[1,2,3,4,5,6]\n", "", 0),
    // lit_n/foreach_acc/gp_deep
    (&[r"-c", r"-n", r#"[range(8)] as $a | {k: $a} as $o | [foreach range(6) as $i ([]; . + [($a|getpath([$i,"x"])?)]; length)]"#], None, "[0,0,0,0,0,0]\n", "", 0),
    // lit_n/foreach_acc/gp_err
    (&[r"-c", r"-n", r#"[range(8)] as $a | {k: $a} as $o | [foreach range(6) as $i ([]; . + [($a|try getpath(["x"]) catch .)]; length)]"#], None, "[1,2,3,4,5,6]\n", "", 0),
    // lit_n/foreach_acc/gp_nullpath
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | [foreach range(6) as $i ([]; . + [($a|try getpath(null) catch .)]; length)]"], None, "[1,2,3,4,5,6]\n", "", 0),
    // lit_n/foreach_acc/gp_obj
    (&[r"-c", r"-n", r#"[range(8)] as $a | {k: $a} as $o | [foreach range(6) as $i ([]; . + [($o|getpath(["k",$i]))]; length)]"#], None, "[1,2,3,4,5,6]\n", "", 0),
    // lit_n/foreach_acc/gp_objmiss
    (&[r"-c", r"-n", r#"[range(8)] as $a | {k: $a} as $o | [foreach range(6) as $i ([]; . + [($o|getpath(["m",$i,"z"]))]; length)]"#], None, "[1,2,3,4,5,6]\n", "", 0),
    // lit_n/foreach_acc/gp_objk
    (&[r"-c", r"-n", r#"[range(8)] as $a | {k: $a} as $o | [foreach range(6) as $i ([]; . + [($o|getpath(["k"])|length)]; length)]"#], None, "[1,2,3,4,5,6]\n", "", 0),
    // lit_n/foreach_acc/gp_then
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | [foreach range(6) as $i ([]; . + [($a|getpath([$i])|tojson)]; length)]"], None, "[1,2,3,4,5,6]\n", "", 0),
    // lit_n/foreach_acc/gp_two
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | [foreach range(6) as $i ([]; . + [($a|getpath([$i],[$i+1]))]; length)]"], None, "[2,4,6,8,10,12]\n", "", 0),
    // lit_n/foreach_acc/gp_gen_path
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | [foreach range(6) as $i ([]; . + [($a|getpath([$i,$i] | .[0:1]))]; length)]"], None, "[1,2,3,4,5,6]\n", "", 0),
    // lit_n/foreach_acc/gp_var_path
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | [foreach range(6) as $i ([]; . + [([$i] as $p | $a|getpath($p))]; length)]"], None, "[1,2,3,4,5,6]\n", "", 0),
    // lit_n/foreach_acc/gp_in_if
    (&[r"-c", r"-n", r#"[range(8)] as $a | {k: $a} as $o | [foreach range(6) as $i ([]; . + [(if ($a|getpath([$i])) == null then "n" else "v" end)]; length)]"#], None, "[1,2,3,4,5,6]\n", "", 0),
    // lit_n/until/getpath
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | [0, []] | until(.[0] >= 6; .[0] as $i | [$i + 1, .[1] + [($a|getpath([$i]))]]) | .[1]"], None, "[0,1,2,3,4,5]\n", "", 0),
    // lit_n/until/gp_neg
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | [0, []] | until(.[0] >= 6; .[0] as $i | [$i + 1, .[1] + [($a|getpath([-1-$i]))]]) | .[1]"], None, "[7,6,5,4,3,2]\n", "", 0),
    // lit_n/until/gp_float
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | [0, []] | until(.[0] >= 6; .[0] as $i | [$i + 1, .[1] + [($a|getpath([$i+0.5]))]]) | .[1]"], None, "[0,1,2,3,4,5]\n", "", 0),
    // lit_n/until/gp_slice
    (&[r"-c", r"-n", r#"[range(8)] as $a | {k: $a} as $o | [0, []] | until(.[0] >= 6; .[0] as $i | [$i + 1, .[1] + [($a|getpath([{"start":$i,"end":($i+2)}]))]]) | .[1]"#], None, "[[0,1],[1,2],[2,3],[3,4],[4,5],[5,6]]\n", "", 0),
    // lit_n/until/gp_sub
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | [0, []] | until(.[0] >= 6; .[0] as $i | [$i + 1, .[1] + [($a|getpath([[$i]]))]]) | .[1]"], None, "[[0],[1],[2],[3],[4],[5]]\n", "", 0),
    // lit_n/until/gp_empty
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | [0, []] | until(.[0] >= 6; .[0] as $i | [$i + 1, .[1] + [($a|getpath([])|length)]]) | .[1]"], None, "[8,8,8,8,8,8]\n", "", 0),
    // lit_n/until/gp_deep
    (&[r"-c", r"-n", r#"[range(8)] as $a | {k: $a} as $o | [0, []] | until(.[0] >= 6; .[0] as $i | [$i + 1, .[1] + [($a|getpath([$i,"x"])?)]]) | .[1]"#], None, "[]\n", "", 0),
    // lit_n/until/gp_err
    (&[r"-c", r"-n", r#"[range(8)] as $a | {k: $a} as $o | [0, []] | until(.[0] >= 6; .[0] as $i | [$i + 1, .[1] + [($a|try getpath(["x"]) catch .)]]) | .[1]"#], None, "[\"Cannot index array with string \\\"x\\\"\",\"Cannot index array with string \\\"x\\\"\",\"Cannot index array with string \\\"x\\\"\",\"Cannot index array with string \\\"x\\\"\",\"Cannot index array with string \\\"x\\\"\",\"Cannot index array with string \\\"x\\\"\"]\n", "", 0),
    // lit_n/until/gp_nullpath
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | [0, []] | until(.[0] >= 6; .[0] as $i | [$i + 1, .[1] + [($a|try getpath(null) catch .)]]) | .[1]"], None, "[\"Path must be specified as an array\",\"Path must be specified as an array\",\"Path must be specified as an array\",\"Path must be specified as an array\",\"Path must be specified as an array\",\"Path must be specified as an array\"]\n", "", 0),
    // lit_n/until/gp_obj
    (&[r"-c", r"-n", r#"[range(8)] as $a | {k: $a} as $o | [0, []] | until(.[0] >= 6; .[0] as $i | [$i + 1, .[1] + [($o|getpath(["k",$i]))]]) | .[1]"#], None, "[0,1,2,3,4,5]\n", "", 0),
    // lit_n/until/gp_objmiss
    (&[r"-c", r"-n", r#"[range(8)] as $a | {k: $a} as $o | [0, []] | until(.[0] >= 6; .[0] as $i | [$i + 1, .[1] + [($o|getpath(["m",$i,"z"]))]]) | .[1]"#], None, "[null,null,null,null,null,null]\n", "", 0),
    // lit_n/until/gp_objk
    (&[r"-c", r"-n", r#"[range(8)] as $a | {k: $a} as $o | [0, []] | until(.[0] >= 6; .[0] as $i | [$i + 1, .[1] + [($o|getpath(["k"])|length)]]) | .[1]"#], None, "[8,8,8,8,8,8]\n", "", 0),
    // lit_n/until/gp_then
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | [0, []] | until(.[0] >= 6; .[0] as $i | [$i + 1, .[1] + [($a|getpath([$i])|tojson)]]) | .[1]"], None, "[\"0\",\"1\",\"2\",\"3\",\"4\",\"5\"]\n", "", 0),
    // lit_n/until/gp_two
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | [0, []] | until(.[0] >= 6; .[0] as $i | [$i + 1, .[1] + [($a|getpath([$i],[$i+1]))]]) | .[1]"], None, "[0,1,1,2,2,3,3,4,4,5,5,6]\n", "", 0),
    // lit_n/until/gp_gen_path
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | [0, []] | until(.[0] >= 6; .[0] as $i | [$i + 1, .[1] + [($a|getpath([$i,$i] | .[0:1]))]]) | .[1]"], None, "[0,1,2,3,4,5]\n", "", 0),
    // lit_n/until/gp_var_path
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | [0, []] | until(.[0] >= 6; .[0] as $i | [$i + 1, .[1] + [([$i] as $p | $a|getpath($p))]]) | .[1]"], None, "[0,1,2,3,4,5]\n", "", 0),
    // lit_n/until/gp_in_if
    (&[r"-c", r"-n", r#"[range(8)] as $a | {k: $a} as $o | [0, []] | until(.[0] >= 6; .[0] as $i | [$i + 1, .[1] + [(if ($a|getpath([$i])) == null then "n" else "v" end)]]) | .[1]"#], None, "[\"v\",\"v\",\"v\",\"v\",\"v\",\"v\"]\n", "", 0),
    // lit_n/first_foreach/getpath
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | first(foreach range(6) as $i (null; ($a|getpath([$i]))))"], None, "0\n", "", 0),
    // lit_n/first_foreach/gp_neg
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | first(foreach range(6) as $i (null; ($a|getpath([-1-$i]))))"], None, "7\n", "", 0),
    // lit_n/first_foreach/gp_float
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | first(foreach range(6) as $i (null; ($a|getpath([$i+0.5]))))"], None, "0\n", "", 0),
    // lit_n/first_foreach/gp_slice
    (&[r"-c", r"-n", r#"[range(8)] as $a | {k: $a} as $o | first(foreach range(6) as $i (null; ($a|getpath([{"start":$i,"end":($i+2)}]))))"#], None, "[0,1]\n", "", 0),
    // lit_n/first_foreach/gp_sub
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | first(foreach range(6) as $i (null; ($a|getpath([[$i]]))))"], None, "[0]\n", "", 0),
    // lit_n/first_foreach/gp_empty
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | first(foreach range(6) as $i (null; ($a|getpath([])|length)))"], None, "8\n", "", 0),
    // lit_n/first_foreach/gp_deep
    (&[r"-c", r"-n", r#"[range(8)] as $a | {k: $a} as $o | first(foreach range(6) as $i (null; ($a|getpath([$i,"x"])?)))"#], None, "", "", 0),
    // lit_n/first_foreach/gp_err
    (&[r"-c", r"-n", r#"[range(8)] as $a | {k: $a} as $o | first(foreach range(6) as $i (null; ($a|try getpath(["x"]) catch .)))"#], None, "\"Cannot index array with string \\\"x\\\"\"\n", "", 0),
    // lit_n/first_foreach/gp_nullpath
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | first(foreach range(6) as $i (null; ($a|try getpath(null) catch .)))"], None, "\"Path must be specified as an array\"\n", "", 0),
    // lit_n/first_foreach/gp_obj
    (&[r"-c", r"-n", r#"[range(8)] as $a | {k: $a} as $o | first(foreach range(6) as $i (null; ($o|getpath(["k",$i]))))"#], None, "0\n", "", 0),
    // lit_n/first_foreach/gp_objmiss
    (&[r"-c", r"-n", r#"[range(8)] as $a | {k: $a} as $o | first(foreach range(6) as $i (null; ($o|getpath(["m",$i,"z"]))))"#], None, "null\n", "", 0),
    // lit_n/first_foreach/gp_objk
    (&[r"-c", r"-n", r#"[range(8)] as $a | {k: $a} as $o | first(foreach range(6) as $i (null; ($o|getpath(["k"])|length)))"#], None, "8\n", "", 0),
    // lit_n/first_foreach/gp_then
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | first(foreach range(6) as $i (null; ($a|getpath([$i])|tojson)))"], None, "\"0\"\n", "", 0),
    // lit_n/first_foreach/gp_two
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | first(foreach range(6) as $i (null; ($a|getpath([$i],[$i+1]))))"], None, "0\n", "", 0),
    // lit_n/first_foreach/gp_gen_path
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | first(foreach range(6) as $i (null; ($a|getpath([$i,$i] | .[0:1]))))"], None, "0\n", "", 0),
    // lit_n/first_foreach/gp_var_path
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | first(foreach range(6) as $i (null; ([$i] as $p | $a|getpath($p))))"], None, "0\n", "", 0),
    // lit_n/first_foreach/gp_in_if
    (&[r"-c", r"-n", r#"[range(8)] as $a | {k: $a} as $o | first(foreach range(6) as $i (null; (if ($a|getpath([$i])) == null then "n" else "v" end)))"#], None, "\"v\"\n", "", 0),
    // lit_n/limit/getpath
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | [limit(3; range(6) as $i | ($a|getpath([$i])))]"], None, "[0,1,2]\n", "", 0),
    // lit_n/limit/gp_neg
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | [limit(3; range(6) as $i | ($a|getpath([-1-$i])))]"], None, "[7,6,5]\n", "", 0),
    // lit_n/limit/gp_float
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | [limit(3; range(6) as $i | ($a|getpath([$i+0.5])))]"], None, "[0,1,2]\n", "", 0),
    // lit_n/limit/gp_slice
    (&[r"-c", r"-n", r#"[range(8)] as $a | {k: $a} as $o | [limit(3; range(6) as $i | ($a|getpath([{"start":$i,"end":($i+2)}])))]"#], None, "[[0,1],[1,2],[2,3]]\n", "", 0),
    // lit_n/limit/gp_sub
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | [limit(3; range(6) as $i | ($a|getpath([[$i]])))]"], None, "[[0],[1],[2]]\n", "", 0),
    // lit_n/limit/gp_empty
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | [limit(3; range(6) as $i | ($a|getpath([])|length))]"], None, "[8,8,8]\n", "", 0),
    // lit_n/limit/gp_deep
    (&[r"-c", r"-n", r#"[range(8)] as $a | {k: $a} as $o | [limit(3; range(6) as $i | ($a|getpath([$i,"x"])?))]"#], None, "[]\n", "", 0),
    // lit_n/limit/gp_err
    (&[r"-c", r"-n", r#"[range(8)] as $a | {k: $a} as $o | [limit(3; range(6) as $i | ($a|try getpath(["x"]) catch .))]"#], None, "[\"Cannot index array with string \\\"x\\\"\",\"Cannot index array with string \\\"x\\\"\",\"Cannot index array with string \\\"x\\\"\"]\n", "", 0),
    // lit_n/limit/gp_nullpath
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | [limit(3; range(6) as $i | ($a|try getpath(null) catch .))]"], None, "[\"Path must be specified as an array\",\"Path must be specified as an array\",\"Path must be specified as an array\"]\n", "", 0),
    // lit_n/limit/gp_obj
    (&[r"-c", r"-n", r#"[range(8)] as $a | {k: $a} as $o | [limit(3; range(6) as $i | ($o|getpath(["k",$i])))]"#], None, "[0,1,2]\n", "", 0),
    // lit_n/limit/gp_objmiss
    (&[r"-c", r"-n", r#"[range(8)] as $a | {k: $a} as $o | [limit(3; range(6) as $i | ($o|getpath(["m",$i,"z"])))]"#], None, "[null,null,null]\n", "", 0),
    // lit_n/limit/gp_objk
    (&[r"-c", r"-n", r#"[range(8)] as $a | {k: $a} as $o | [limit(3; range(6) as $i | ($o|getpath(["k"])|length))]"#], None, "[8,8,8]\n", "", 0),
    // lit_n/limit/gp_then
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | [limit(3; range(6) as $i | ($a|getpath([$i])|tojson))]"], None, "[\"0\",\"1\",\"2\"]\n", "", 0),
    // lit_n/limit/gp_two
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | [limit(3; range(6) as $i | ($a|getpath([$i],[$i+1])))]"], None, "[0,1,1]\n", "", 0),
    // lit_n/limit/gp_gen_path
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | [limit(3; range(6) as $i | ($a|getpath([$i,$i] | .[0:1])))]"], None, "[0,1,2]\n", "", 0),
    // lit_n/limit/gp_var_path
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | [limit(3; range(6) as $i | ([$i] as $p | $a|getpath($p)))]"], None, "[0,1,2]\n", "", 0),
    // lit_n/limit/gp_in_if
    (&[r"-c", r"-n", r#"[range(8)] as $a | {k: $a} as $o | [limit(3; range(6) as $i | (if ($a|getpath([$i])) == null then "n" else "v" end))]"#], None, "[\"v\",\"v\",\"v\"]\n", "", 0),
    // computed_n/reduce/getpath
    (&[r"-c", r"-n", r"[range(8) | . * 3 | tostring] as $a | {k: [range(8)|-.]} as $o | reduce range(6) as $i ([]; . + [($a|getpath([$i]))])"], None, "[\"0\",\"3\",\"6\",\"9\",\"12\",\"15\"]\n", "", 0),
    // computed_n/reduce/gp_float
    (&[r"-c", r"-n", r"[range(8) | . * 3 | tostring] as $a | {k: [range(8)|-.]} as $o | reduce range(6) as $i ([]; . + [($a|getpath([$i+0.5]))])"], None, "[\"0\",\"3\",\"6\",\"9\",\"12\",\"15\"]\n", "", 0),
    // computed_n/reduce/gp_slice
    (&[r"-c", r"-n", r#"[range(8) | . * 3 | tostring] as $a | {k: [range(8)|-.]} as $o | reduce range(6) as $i ([]; . + [($a|getpath([{"start":$i,"end":($i+2)}]))])"#], None, "[[\"0\",\"3\"],[\"3\",\"6\"],[\"6\",\"9\"],[\"9\",\"12\"],[\"12\",\"15\"],[\"15\",\"18\"]]\n", "", 0),
    // computed_n/reduce/gp_deep
    (&[r"-c", r"-n", r#"[range(8) | . * 3 | tostring] as $a | {k: [range(8)|-.]} as $o | reduce range(6) as $i ([]; . + [($a|getpath([$i,"x"])?)])"#], None, "[]\n", "", 0),
    // computed_n/reduce/gp_err
    (&[r"-c", r"-n", r#"[range(8) | . * 3 | tostring] as $a | {k: [range(8)|-.]} as $o | reduce range(6) as $i ([]; . + [($a|try getpath(["x"]) catch .)])"#], None, "[\"Cannot index array with string \\\"x\\\"\",\"Cannot index array with string \\\"x\\\"\",\"Cannot index array with string \\\"x\\\"\",\"Cannot index array with string \\\"x\\\"\",\"Cannot index array with string \\\"x\\\"\",\"Cannot index array with string \\\"x\\\"\"]\n", "", 0),
    // computed_n/reduce/gp_obj
    (&[r"-c", r"-n", r#"[range(8) | . * 3 | tostring] as $a | {k: [range(8)|-.]} as $o | reduce range(6) as $i ([]; . + [($o|getpath(["k",$i]))])"#], None, "[-0,-1,-2,-3,-4,-5]\n", "", 0),
    // computed_n/reduce/gp_two
    (&[r"-c", r"-n", r"[range(8) | . * 3 | tostring] as $a | {k: [range(8)|-.]} as $o | reduce range(6) as $i ([]; . + [($a|getpath([$i],[$i+1]))])"], None, "[\"0\",\"3\",\"3\",\"6\",\"6\",\"9\",\"9\",\"12\",\"12\",\"15\",\"15\",\"18\"]\n", "", 0),
    // computed_n/reduce/gp_in_if
    (&[r"-c", r"-n", r#"[range(8) | . * 3 | tostring] as $a | {k: [range(8)|-.]} as $o | reduce range(6) as $i ([]; . + [(if ($a|getpath([$i])) == null then "n" else "v" end)])"#], None, "[\"v\",\"v\",\"v\",\"v\",\"v\",\"v\"]\n", "", 0),
    // computed_n/foreach_acc/getpath
    (&[r"-c", r"-n", r"[range(8) | . * 3 | tostring] as $a | {k: [range(8)|-.]} as $o | [foreach range(6) as $i ([]; . + [($a|getpath([$i]))]; length)]"], None, "[1,2,3,4,5,6]\n", "", 0),
    // computed_n/foreach_acc/gp_float
    (&[r"-c", r"-n", r"[range(8) | . * 3 | tostring] as $a | {k: [range(8)|-.]} as $o | [foreach range(6) as $i ([]; . + [($a|getpath([$i+0.5]))]; length)]"], None, "[1,2,3,4,5,6]\n", "", 0),
    // computed_n/foreach_acc/gp_slice
    (&[r"-c", r"-n", r#"[range(8) | . * 3 | tostring] as $a | {k: [range(8)|-.]} as $o | [foreach range(6) as $i ([]; . + [($a|getpath([{"start":$i,"end":($i+2)}]))]; length)]"#], None, "[1,2,3,4,5,6]\n", "", 0),
    // computed_n/foreach_acc/gp_deep
    (&[r"-c", r"-n", r#"[range(8) | . * 3 | tostring] as $a | {k: [range(8)|-.]} as $o | [foreach range(6) as $i ([]; . + [($a|getpath([$i,"x"])?)]; length)]"#], None, "[0,0,0,0,0,0]\n", "", 0),
    // computed_n/foreach_acc/gp_err
    (&[r"-c", r"-n", r#"[range(8) | . * 3 | tostring] as $a | {k: [range(8)|-.]} as $o | [foreach range(6) as $i ([]; . + [($a|try getpath(["x"]) catch .)]; length)]"#], None, "[1,2,3,4,5,6]\n", "", 0),
    // computed_n/foreach_acc/gp_obj
    (&[r"-c", r"-n", r#"[range(8) | . * 3 | tostring] as $a | {k: [range(8)|-.]} as $o | [foreach range(6) as $i ([]; . + [($o|getpath(["k",$i]))]; length)]"#], None, "[1,2,3,4,5,6]\n", "", 0),
    // computed_n/foreach_acc/gp_two
    (&[r"-c", r"-n", r"[range(8) | . * 3 | tostring] as $a | {k: [range(8)|-.]} as $o | [foreach range(6) as $i ([]; . + [($a|getpath([$i],[$i+1]))]; length)]"], None, "[2,4,6,8,10,12]\n", "", 0),
    // computed_n/foreach_acc/gp_in_if
    (&[r"-c", r"-n", r#"[range(8) | . * 3 | tostring] as $a | {k: [range(8)|-.]} as $o | [foreach range(6) as $i ([]; . + [(if ($a|getpath([$i])) == null then "n" else "v" end)]; length)]"#], None, "[1,2,3,4,5,6]\n", "", 0),
    // computed_n/until/getpath
    (&[r"-c", r"-n", r"[range(8) | . * 3 | tostring] as $a | {k: [range(8)|-.]} as $o | [0, []] | until(.[0] >= 6; .[0] as $i | [$i + 1, .[1] + [($a|getpath([$i]))]]) | .[1]"], None, "[\"0\",\"3\",\"6\",\"9\",\"12\",\"15\"]\n", "", 0),
    // computed_n/until/gp_float
    (&[r"-c", r"-n", r"[range(8) | . * 3 | tostring] as $a | {k: [range(8)|-.]} as $o | [0, []] | until(.[0] >= 6; .[0] as $i | [$i + 1, .[1] + [($a|getpath([$i+0.5]))]]) | .[1]"], None, "[\"0\",\"3\",\"6\",\"9\",\"12\",\"15\"]\n", "", 0),
    // computed_n/until/gp_slice
    (&[r"-c", r"-n", r#"[range(8) | . * 3 | tostring] as $a | {k: [range(8)|-.]} as $o | [0, []] | until(.[0] >= 6; .[0] as $i | [$i + 1, .[1] + [($a|getpath([{"start":$i,"end":($i+2)}]))]]) | .[1]"#], None, "[[\"0\",\"3\"],[\"3\",\"6\"],[\"6\",\"9\"],[\"9\",\"12\"],[\"12\",\"15\"],[\"15\",\"18\"]]\n", "", 0),
    // computed_n/until/gp_deep
    (&[r"-c", r"-n", r#"[range(8) | . * 3 | tostring] as $a | {k: [range(8)|-.]} as $o | [0, []] | until(.[0] >= 6; .[0] as $i | [$i + 1, .[1] + [($a|getpath([$i,"x"])?)]]) | .[1]"#], None, "[]\n", "", 0),
    // computed_n/until/gp_err
    (&[r"-c", r"-n", r#"[range(8) | . * 3 | tostring] as $a | {k: [range(8)|-.]} as $o | [0, []] | until(.[0] >= 6; .[0] as $i | [$i + 1, .[1] + [($a|try getpath(["x"]) catch .)]]) | .[1]"#], None, "[\"Cannot index array with string \\\"x\\\"\",\"Cannot index array with string \\\"x\\\"\",\"Cannot index array with string \\\"x\\\"\",\"Cannot index array with string \\\"x\\\"\",\"Cannot index array with string \\\"x\\\"\",\"Cannot index array with string \\\"x\\\"\"]\n", "", 0),
    // computed_n/until/gp_obj
    (&[r"-c", r"-n", r#"[range(8) | . * 3 | tostring] as $a | {k: [range(8)|-.]} as $o | [0, []] | until(.[0] >= 6; .[0] as $i | [$i + 1, .[1] + [($o|getpath(["k",$i]))]]) | .[1]"#], None, "[-0,-1,-2,-3,-4,-5]\n", "", 0),
    // computed_n/until/gp_two
    (&[r"-c", r"-n", r"[range(8) | . * 3 | tostring] as $a | {k: [range(8)|-.]} as $o | [0, []] | until(.[0] >= 6; .[0] as $i | [$i + 1, .[1] + [($a|getpath([$i],[$i+1]))]]) | .[1]"], None, "[\"0\",\"3\",\"3\",\"6\",\"6\",\"9\",\"9\",\"12\",\"12\",\"15\",\"15\",\"18\"]\n", "", 0),
    // computed_n/until/gp_in_if
    (&[r"-c", r"-n", r#"[range(8) | . * 3 | tostring] as $a | {k: [range(8)|-.]} as $o | [0, []] | until(.[0] >= 6; .[0] as $i | [$i + 1, .[1] + [(if ($a|getpath([$i])) == null then "n" else "v" end)]]) | .[1]"#], None, "[\"v\",\"v\",\"v\",\"v\",\"v\",\"v\"]\n", "", 0),
    // computed_n/limit/getpath
    (&[r"-c", r"-n", r"[range(8) | . * 3 | tostring] as $a | {k: [range(8)|-.]} as $o | [limit(3; range(6) as $i | ($a|getpath([$i])))]"], None, "[\"0\",\"3\",\"6\"]\n", "", 0),
    // computed_n/limit/gp_float
    (&[r"-c", r"-n", r"[range(8) | . * 3 | tostring] as $a | {k: [range(8)|-.]} as $o | [limit(3; range(6) as $i | ($a|getpath([$i+0.5])))]"], None, "[\"0\",\"3\",\"6\"]\n", "", 0),
    // computed_n/limit/gp_slice
    (&[r"-c", r"-n", r#"[range(8) | . * 3 | tostring] as $a | {k: [range(8)|-.]} as $o | [limit(3; range(6) as $i | ($a|getpath([{"start":$i,"end":($i+2)}])))]"#], None, "[[\"0\",\"3\"],[\"3\",\"6\"],[\"6\",\"9\"]]\n", "", 0),
    // computed_n/limit/gp_deep
    (&[r"-c", r"-n", r#"[range(8) | . * 3 | tostring] as $a | {k: [range(8)|-.]} as $o | [limit(3; range(6) as $i | ($a|getpath([$i,"x"])?))]"#], None, "[]\n", "", 0),
    // computed_n/limit/gp_err
    (&[r"-c", r"-n", r#"[range(8) | . * 3 | tostring] as $a | {k: [range(8)|-.]} as $o | [limit(3; range(6) as $i | ($a|try getpath(["x"]) catch .))]"#], None, "[\"Cannot index array with string \\\"x\\\"\",\"Cannot index array with string \\\"x\\\"\",\"Cannot index array with string \\\"x\\\"\"]\n", "", 0),
    // computed_n/limit/gp_obj
    (&[r"-c", r"-n", r#"[range(8) | . * 3 | tostring] as $a | {k: [range(8)|-.]} as $o | [limit(3; range(6) as $i | ($o|getpath(["k",$i])))]"#], None, "[-0,-1,-2]\n", "", 0),
    // computed_n/limit/gp_two
    (&[r"-c", r"-n", r"[range(8) | . * 3 | tostring] as $a | {k: [range(8)|-.]} as $o | [limit(3; range(6) as $i | ($a|getpath([$i],[$i+1])))]"], None, "[\"0\",\"3\",\"3\"]\n", "", 0),
    // computed_n/limit/gp_in_if
    (&[r"-c", r"-n", r#"[range(8) | . * 3 | tostring] as $a | {k: [range(8)|-.]} as $o | [limit(3; range(6) as $i | (if ($a|getpath([$i])) == null then "n" else "v" end))]"#], None, "[\"v\",\"v\",\"v\"]\n", "", 0),
    // mixed_n/reduce/getpath
    (&[r"-c", r"-n", r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}] as $a | {k: $a} as $o | reduce range(6) as $i ([]; . + [($a|getpath([$i]))])"#], None, "[0,1.000,1E+1000,100000000000000000001,\"s\",null]\n", "", 0),
    // mixed_n/reduce/gp_float
    (&[r"-c", r"-n", r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}] as $a | {k: $a} as $o | reduce range(6) as $i ([]; . + [($a|getpath([$i+0.5]))])"#], None, "[0,1.000,1E+1000,100000000000000000001,\"s\",null]\n", "", 0),
    // mixed_n/reduce/gp_slice
    (&[r"-c", r"-n", r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}] as $a | {k: $a} as $o | reduce range(6) as $i ([]; . + [($a|getpath([{"start":$i,"end":($i+2)}]))])"#], None, "[[0,1.000],[1.000,1E+1000],[1E+1000,100000000000000000001],[100000000000000000001,\"s\"],[\"s\",null],[null,{\"x\":1}]]\n", "", 0),
    // mixed_n/reduce/gp_deep
    (&[r"-c", r"-n", r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}] as $a | {k: $a} as $o | reduce range(6) as $i ([]; . + [($a|getpath([$i,"x"])?)])"#], None, "[null]\n", "", 0),
    // mixed_n/reduce/gp_err
    (&[r"-c", r"-n", r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}] as $a | {k: $a} as $o | reduce range(6) as $i ([]; . + [($a|try getpath(["x"]) catch .)])"#], None, "[\"Cannot index array with string \\\"x\\\"\",\"Cannot index array with string \\\"x\\\"\",\"Cannot index array with string \\\"x\\\"\",\"Cannot index array with string \\\"x\\\"\",\"Cannot index array with string \\\"x\\\"\",\"Cannot index array with string \\\"x\\\"\"]\n", "", 0),
    // mixed_n/reduce/gp_obj
    (&[r"-c", r"-n", r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}] as $a | {k: $a} as $o | reduce range(6) as $i ([]; . + [($o|getpath(["k",$i]))])"#], None, "[0,1.000,1E+1000,100000000000000000001,\"s\",null]\n", "", 0),
    // mixed_n/reduce/gp_two
    (&[r"-c", r"-n", r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}] as $a | {k: $a} as $o | reduce range(6) as $i ([]; . + [($a|getpath([$i],[$i+1]))])"#], None, "[0,1.000,1.000,1E+1000,1E+1000,100000000000000000001,100000000000000000001,\"s\",\"s\",null,null,{\"x\":1}]\n", "", 0),
    // mixed_n/reduce/gp_in_if
    (&[r"-c", r"-n", r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}] as $a | {k: $a} as $o | reduce range(6) as $i ([]; . + [(if ($a|getpath([$i])) == null then "n" else "v" end)])"#], None, "[\"v\",\"v\",\"v\",\"v\",\"v\",\"n\"]\n", "", 0),
    // mixed_n/foreach_acc/getpath
    (&[r"-c", r"-n", r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}] as $a | {k: $a} as $o | [foreach range(6) as $i ([]; . + [($a|getpath([$i]))]; length)]"#], None, "[1,2,3,4,5,6]\n", "", 0),
    // mixed_n/foreach_acc/gp_float
    (&[r"-c", r"-n", r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}] as $a | {k: $a} as $o | [foreach range(6) as $i ([]; . + [($a|getpath([$i+0.5]))]; length)]"#], None, "[1,2,3,4,5,6]\n", "", 0),
    // mixed_n/foreach_acc/gp_slice
    (&[r"-c", r"-n", r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}] as $a | {k: $a} as $o | [foreach range(6) as $i ([]; . + [($a|getpath([{"start":$i,"end":($i+2)}]))]; length)]"#], None, "[1,2,3,4,5,6]\n", "", 0),
    // mixed_n/foreach_acc/gp_deep
    (&[r"-c", r"-n", r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}] as $a | {k: $a} as $o | [foreach range(6) as $i ([]; . + [($a|getpath([$i,"x"])?)]; length)]"#], None, "[0,0,0,0,0,1]\n", "", 0),
    // mixed_n/foreach_acc/gp_err
    (&[r"-c", r"-n", r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}] as $a | {k: $a} as $o | [foreach range(6) as $i ([]; . + [($a|try getpath(["x"]) catch .)]; length)]"#], None, "[1,2,3,4,5,6]\n", "", 0),
    // mixed_n/foreach_acc/gp_obj
    (&[r"-c", r"-n", r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}] as $a | {k: $a} as $o | [foreach range(6) as $i ([]; . + [($o|getpath(["k",$i]))]; length)]"#], None, "[1,2,3,4,5,6]\n", "", 0),
    // mixed_n/foreach_acc/gp_two
    (&[r"-c", r"-n", r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}] as $a | {k: $a} as $o | [foreach range(6) as $i ([]; . + [($a|getpath([$i],[$i+1]))]; length)]"#], None, "[2,4,6,8,10,12]\n", "", 0),
    // mixed_n/foreach_acc/gp_in_if
    (&[r"-c", r"-n", r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}] as $a | {k: $a} as $o | [foreach range(6) as $i ([]; . + [(if ($a|getpath([$i])) == null then "n" else "v" end)]; length)]"#], None, "[1,2,3,4,5,6]\n", "", 0),
    // mixed_n/until/getpath
    (&[r"-c", r"-n", r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}] as $a | {k: $a} as $o | [0, []] | until(.[0] >= 6; .[0] as $i | [$i + 1, .[1] + [($a|getpath([$i]))]]) | .[1]"#], None, "[0,1.000,1E+1000,100000000000000000001,\"s\",null]\n", "", 0),
    // mixed_n/until/gp_float
    (&[r"-c", r"-n", r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}] as $a | {k: $a} as $o | [0, []] | until(.[0] >= 6; .[0] as $i | [$i + 1, .[1] + [($a|getpath([$i+0.5]))]]) | .[1]"#], None, "[0,1.000,1E+1000,100000000000000000001,\"s\",null]\n", "", 0),
    // mixed_n/until/gp_slice
    (&[r"-c", r"-n", r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}] as $a | {k: $a} as $o | [0, []] | until(.[0] >= 6; .[0] as $i | [$i + 1, .[1] + [($a|getpath([{"start":$i,"end":($i+2)}]))]]) | .[1]"#], None, "[[0,1.000],[1.000,1E+1000],[1E+1000,100000000000000000001],[100000000000000000001,\"s\"],[\"s\",null],[null,{\"x\":1}]]\n", "", 0),
    // mixed_n/until/gp_deep
    (&[r"-c", r"-n", r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}] as $a | {k: $a} as $o | [0, []] | until(.[0] >= 6; .[0] as $i | [$i + 1, .[1] + [($a|getpath([$i,"x"])?)]]) | .[1]"#], None, "[null]\n", "", 0),
    // mixed_n/until/gp_err
    (&[r"-c", r"-n", r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}] as $a | {k: $a} as $o | [0, []] | until(.[0] >= 6; .[0] as $i | [$i + 1, .[1] + [($a|try getpath(["x"]) catch .)]]) | .[1]"#], None, "[\"Cannot index array with string \\\"x\\\"\",\"Cannot index array with string \\\"x\\\"\",\"Cannot index array with string \\\"x\\\"\",\"Cannot index array with string \\\"x\\\"\",\"Cannot index array with string \\\"x\\\"\",\"Cannot index array with string \\\"x\\\"\"]\n", "", 0),
    // mixed_n/until/gp_obj
    (&[r"-c", r"-n", r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}] as $a | {k: $a} as $o | [0, []] | until(.[0] >= 6; .[0] as $i | [$i + 1, .[1] + [($o|getpath(["k",$i]))]]) | .[1]"#], None, "[0,1.000,1E+1000,100000000000000000001,\"s\",null]\n", "", 0),
    // mixed_n/until/gp_two
    (&[r"-c", r"-n", r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}] as $a | {k: $a} as $o | [0, []] | until(.[0] >= 6; .[0] as $i | [$i + 1, .[1] + [($a|getpath([$i],[$i+1]))]]) | .[1]"#], None, "[0,1.000,1.000,1E+1000,1E+1000,100000000000000000001,100000000000000000001,\"s\",\"s\",null,null,{\"x\":1}]\n", "", 0),
    // mixed_n/until/gp_in_if
    (&[r"-c", r"-n", r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}] as $a | {k: $a} as $o | [0, []] | until(.[0] >= 6; .[0] as $i | [$i + 1, .[1] + [(if ($a|getpath([$i])) == null then "n" else "v" end)]]) | .[1]"#], None, "[\"v\",\"v\",\"v\",\"v\",\"v\",\"n\"]\n", "", 0),
    // mixed_n/limit/getpath
    (&[r"-c", r"-n", r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}] as $a | {k: $a} as $o | [limit(3; range(6) as $i | ($a|getpath([$i])))]"#], None, "[0,1.000,1E+1000]\n", "", 0),
    // mixed_n/limit/gp_float
    (&[r"-c", r"-n", r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}] as $a | {k: $a} as $o | [limit(3; range(6) as $i | ($a|getpath([$i+0.5])))]"#], None, "[0,1.000,1E+1000]\n", "", 0),
    // mixed_n/limit/gp_slice
    (&[r"-c", r"-n", r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}] as $a | {k: $a} as $o | [limit(3; range(6) as $i | ($a|getpath([{"start":$i,"end":($i+2)}])))]"#], None, "[[0,1.000],[1.000,1E+1000],[1E+1000,100000000000000000001]]\n", "", 0),
    // mixed_n/limit/gp_deep
    (&[r"-c", r"-n", r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}] as $a | {k: $a} as $o | [limit(3; range(6) as $i | ($a|getpath([$i,"x"])?))]"#], None, "[null]\n", "", 0),
    // mixed_n/limit/gp_err
    (&[r"-c", r"-n", r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}] as $a | {k: $a} as $o | [limit(3; range(6) as $i | ($a|try getpath(["x"]) catch .))]"#], None, "[\"Cannot index array with string \\\"x\\\"\",\"Cannot index array with string \\\"x\\\"\",\"Cannot index array with string \\\"x\\\"\"]\n", "", 0),
    // mixed_n/limit/gp_obj
    (&[r"-c", r"-n", r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}] as $a | {k: $a} as $o | [limit(3; range(6) as $i | ($o|getpath(["k",$i])))]"#], None, "[0,1.000,1E+1000]\n", "", 0),
    // mixed_n/limit/gp_two
    (&[r"-c", r"-n", r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}] as $a | {k: $a} as $o | [limit(3; range(6) as $i | ($a|getpath([$i],[$i+1])))]"#], None, "[0,1.000,1.000]\n", "", 0),
    // mixed_n/limit/gp_in_if
    (&[r"-c", r"-n", r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}] as $a | {k: $a} as $o | [limit(3; range(6) as $i | (if ($a|getpath([$i])) == null then "n" else "v" end))]"#], None, "[\"v\",\"v\",\"v\"]\n", "", 0),
    // doc/reduce/getpath
    (&[r"-c", r". as $a | {k: $a} as $o | reduce range(6) as $i ([]; . + [($a|getpath([$i]))])"], Some(r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]
"#), "[0,1.000,1E+1000,100000000000000000001,\"s\",null]\n", "", 0),
    // doc/reduce/gp_float
    (&[r"-c", r". as $a | {k: $a} as $o | reduce range(6) as $i ([]; . + [($a|getpath([$i+0.5]))])"], Some(r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]
"#), "[0,1.000,1E+1000,100000000000000000001,\"s\",null]\n", "", 0),
    // doc/reduce/gp_slice
    (&[r"-c", r#". as $a | {k: $a} as $o | reduce range(6) as $i ([]; . + [($a|getpath([{"start":$i,"end":($i+2)}]))])"#], Some(r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]
"#), "[[0,1.000],[1.000,1E+1000],[1E+1000,100000000000000000001],[100000000000000000001,\"s\"],[\"s\",null],[null,{\"x\":1}]]\n", "", 0),
    // doc/reduce/gp_deep
    (&[r"-c", r#". as $a | {k: $a} as $o | reduce range(6) as $i ([]; . + [($a|getpath([$i,"x"])?)])"#], Some(r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]
"#), "[null]\n", "", 0),
    // doc/reduce/gp_err
    (&[r"-c", r#". as $a | {k: $a} as $o | reduce range(6) as $i ([]; . + [($a|try getpath(["x"]) catch .)])"#], Some(r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]
"#), "[\"Cannot index array with string \\\"x\\\"\",\"Cannot index array with string \\\"x\\\"\",\"Cannot index array with string \\\"x\\\"\",\"Cannot index array with string \\\"x\\\"\",\"Cannot index array with string \\\"x\\\"\",\"Cannot index array with string \\\"x\\\"\"]\n", "", 0),
    // doc/reduce/gp_obj
    (&[r"-c", r#". as $a | {k: $a} as $o | reduce range(6) as $i ([]; . + [($o|getpath(["k",$i]))])"#], Some(r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]
"#), "[0,1.000,1E+1000,100000000000000000001,\"s\",null]\n", "", 0),
    // doc/reduce/gp_two
    (&[r"-c", r". as $a | {k: $a} as $o | reduce range(6) as $i ([]; . + [($a|getpath([$i],[$i+1]))])"], Some(r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]
"#), "[0,1.000,1.000,1E+1000,1E+1000,100000000000000000001,100000000000000000001,\"s\",\"s\",null,null,{\"x\":1}]\n", "", 0),
    // doc/reduce/gp_in_if
    (&[r"-c", r#". as $a | {k: $a} as $o | reduce range(6) as $i ([]; . + [(if ($a|getpath([$i])) == null then "n" else "v" end)])"#], Some(r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]
"#), "[\"v\",\"v\",\"v\",\"v\",\"v\",\"n\"]\n", "", 0),
    // doc/foreach_acc/getpath
    (&[r"-c", r". as $a | {k: $a} as $o | [foreach range(6) as $i ([]; . + [($a|getpath([$i]))]; length)]"], Some(r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]
"#), "[1,2,3,4,5,6]\n", "", 0),
    // doc/foreach_acc/gp_float
    (&[r"-c", r". as $a | {k: $a} as $o | [foreach range(6) as $i ([]; . + [($a|getpath([$i+0.5]))]; length)]"], Some(r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]
"#), "[1,2,3,4,5,6]\n", "", 0),
    // doc/foreach_acc/gp_slice
    (&[r"-c", r#". as $a | {k: $a} as $o | [foreach range(6) as $i ([]; . + [($a|getpath([{"start":$i,"end":($i+2)}]))]; length)]"#], Some(r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]
"#), "[1,2,3,4,5,6]\n", "", 0),
    // doc/foreach_acc/gp_deep
    (&[r"-c", r#". as $a | {k: $a} as $o | [foreach range(6) as $i ([]; . + [($a|getpath([$i,"x"])?)]; length)]"#], Some(r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]
"#), "[0,0,0,0,0,1]\n", "", 0),
    // doc/foreach_acc/gp_err
    (&[r"-c", r#". as $a | {k: $a} as $o | [foreach range(6) as $i ([]; . + [($a|try getpath(["x"]) catch .)]; length)]"#], Some(r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]
"#), "[1,2,3,4,5,6]\n", "", 0),
    // doc/foreach_acc/gp_obj
    (&[r"-c", r#". as $a | {k: $a} as $o | [foreach range(6) as $i ([]; . + [($o|getpath(["k",$i]))]; length)]"#], Some(r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]
"#), "[1,2,3,4,5,6]\n", "", 0),
    // doc/foreach_acc/gp_two
    (&[r"-c", r". as $a | {k: $a} as $o | [foreach range(6) as $i ([]; . + [($a|getpath([$i],[$i+1]))]; length)]"], Some(r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]
"#), "[2,4,6,8,10,12]\n", "", 0),
    // doc/foreach_acc/gp_in_if
    (&[r"-c", r#". as $a | {k: $a} as $o | [foreach range(6) as $i ([]; . + [(if ($a|getpath([$i])) == null then "n" else "v" end)]; length)]"#], Some(r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]
"#), "[1,2,3,4,5,6]\n", "", 0),
    // doc/until/getpath
    (&[r"-c", r". as $a | {k: $a} as $o | [0, []] | until(.[0] >= 6; .[0] as $i | [$i + 1, .[1] + [($a|getpath([$i]))]]) | .[1]"], Some(r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]
"#), "[0,1.000,1E+1000,100000000000000000001,\"s\",null]\n", "", 0),
    // doc/until/gp_float
    (&[r"-c", r". as $a | {k: $a} as $o | [0, []] | until(.[0] >= 6; .[0] as $i | [$i + 1, .[1] + [($a|getpath([$i+0.5]))]]) | .[1]"], Some(r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]
"#), "[0,1.000,1E+1000,100000000000000000001,\"s\",null]\n", "", 0),
    // doc/until/gp_slice
    (&[r"-c", r#". as $a | {k: $a} as $o | [0, []] | until(.[0] >= 6; .[0] as $i | [$i + 1, .[1] + [($a|getpath([{"start":$i,"end":($i+2)}]))]]) | .[1]"#], Some(r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]
"#), "[[0,1.000],[1.000,1E+1000],[1E+1000,100000000000000000001],[100000000000000000001,\"s\"],[\"s\",null],[null,{\"x\":1}]]\n", "", 0),
    // doc/until/gp_deep
    (&[r"-c", r#". as $a | {k: $a} as $o | [0, []] | until(.[0] >= 6; .[0] as $i | [$i + 1, .[1] + [($a|getpath([$i,"x"])?)]]) | .[1]"#], Some(r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]
"#), "[null]\n", "", 0),
    // doc/until/gp_err
    (&[r"-c", r#". as $a | {k: $a} as $o | [0, []] | until(.[0] >= 6; .[0] as $i | [$i + 1, .[1] + [($a|try getpath(["x"]) catch .)]]) | .[1]"#], Some(r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]
"#), "[\"Cannot index array with string \\\"x\\\"\",\"Cannot index array with string \\\"x\\\"\",\"Cannot index array with string \\\"x\\\"\",\"Cannot index array with string \\\"x\\\"\",\"Cannot index array with string \\\"x\\\"\",\"Cannot index array with string \\\"x\\\"\"]\n", "", 0),
    // doc/until/gp_obj
    (&[r"-c", r#". as $a | {k: $a} as $o | [0, []] | until(.[0] >= 6; .[0] as $i | [$i + 1, .[1] + [($o|getpath(["k",$i]))]]) | .[1]"#], Some(r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]
"#), "[0,1.000,1E+1000,100000000000000000001,\"s\",null]\n", "", 0),
    // doc/until/gp_two
    (&[r"-c", r". as $a | {k: $a} as $o | [0, []] | until(.[0] >= 6; .[0] as $i | [$i + 1, .[1] + [($a|getpath([$i],[$i+1]))]]) | .[1]"], Some(r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]
"#), "[0,1.000,1.000,1E+1000,1E+1000,100000000000000000001,100000000000000000001,\"s\",\"s\",null,null,{\"x\":1}]\n", "", 0),
    // doc/until/gp_in_if
    (&[r"-c", r#". as $a | {k: $a} as $o | [0, []] | until(.[0] >= 6; .[0] as $i | [$i + 1, .[1] + [(if ($a|getpath([$i])) == null then "n" else "v" end)]]) | .[1]"#], Some(r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]
"#), "[\"v\",\"v\",\"v\",\"v\",\"v\",\"n\"]\n", "", 0),
    // doc/limit/getpath
    (&[r"-c", r". as $a | {k: $a} as $o | [limit(3; range(6) as $i | ($a|getpath([$i])))]"], Some(r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]
"#), "[0,1.000,1E+1000]\n", "", 0),
    // doc/limit/gp_float
    (&[r"-c", r". as $a | {k: $a} as $o | [limit(3; range(6) as $i | ($a|getpath([$i+0.5])))]"], Some(r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]
"#), "[0,1.000,1E+1000]\n", "", 0),
    // doc/limit/gp_slice
    (&[r"-c", r#". as $a | {k: $a} as $o | [limit(3; range(6) as $i | ($a|getpath([{"start":$i,"end":($i+2)}])))]"#], Some(r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]
"#), "[[0,1.000],[1.000,1E+1000],[1E+1000,100000000000000000001]]\n", "", 0),
    // doc/limit/gp_deep
    (&[r"-c", r#". as $a | {k: $a} as $o | [limit(3; range(6) as $i | ($a|getpath([$i,"x"])?))]"#], Some(r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]
"#), "[null]\n", "", 0),
    // doc/limit/gp_err
    (&[r"-c", r#". as $a | {k: $a} as $o | [limit(3; range(6) as $i | ($a|try getpath(["x"]) catch .))]"#], Some(r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]
"#), "[\"Cannot index array with string \\\"x\\\"\",\"Cannot index array with string \\\"x\\\"\",\"Cannot index array with string \\\"x\\\"\"]\n", "", 0),
    // doc/limit/gp_obj
    (&[r"-c", r#". as $a | {k: $a} as $o | [limit(3; range(6) as $i | ($o|getpath(["k",$i])))]"#], Some(r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]
"#), "[0,1.000,1E+1000]\n", "", 0),
    // doc/limit/gp_two
    (&[r"-c", r". as $a | {k: $a} as $o | [limit(3; range(6) as $i | ($a|getpath([$i],[$i+1])))]"], Some(r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]
"#), "[0,1.000,1.000]\n", "", 0),
    // doc/limit/gp_in_if
    (&[r"-c", r#". as $a | {k: $a} as $o | [limit(3; range(6) as $i | (if ($a|getpath([$i])) == null then "n" else "v" end))]"#], Some(r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]
"#), "[\"v\",\"v\",\"v\"]\n", "", 0),
    // doc_obj/reduce/getpath
    (&[r"-c", r".a as $a | . as $o | reduce range(6) as $i ([]; . + [($a|getpath([$i]))])"], Some(r#"{"a":[0,1.000,"s",null,[1,2],3.5,7,8],"k":[10,1.000,1e1000,{"z":1},null,-0]}
"#), "[0,1.000,\"s\",null,[1,2],3.5]\n", "", 0),
    // doc_obj/reduce/gp_float
    (&[r"-c", r".a as $a | . as $o | reduce range(6) as $i ([]; . + [($a|getpath([$i+0.5]))])"], Some(r#"{"a":[0,1.000,"s",null,[1,2],3.5,7,8],"k":[10,1.000,1e1000,{"z":1},null,-0]}
"#), "[0,1.000,\"s\",null,[1,2],3.5]\n", "", 0),
    // doc_obj/reduce/gp_slice
    (&[r"-c", r#".a as $a | . as $o | reduce range(6) as $i ([]; . + [($a|getpath([{"start":$i,"end":($i+2)}]))])"#], Some(r#"{"a":[0,1.000,"s",null,[1,2],3.5,7,8],"k":[10,1.000,1e1000,{"z":1},null,-0]}
"#), "[[0,1.000],[1.000,\"s\"],[\"s\",null],[null,[1,2]],[[1,2],3.5],[3.5,7]]\n", "", 0),
    // doc_obj/reduce/gp_deep
    (&[r"-c", r#".a as $a | . as $o | reduce range(6) as $i ([]; . + [($a|getpath([$i,"x"])?)])"#], Some(r#"{"a":[0,1.000,"s",null,[1,2],3.5,7,8],"k":[10,1.000,1e1000,{"z":1},null,-0]}
"#), "[null]\n", "", 0),
    // doc_obj/reduce/gp_err
    (&[r"-c", r#".a as $a | . as $o | reduce range(6) as $i ([]; . + [($a|try getpath(["x"]) catch .)])"#], Some(r#"{"a":[0,1.000,"s",null,[1,2],3.5,7,8],"k":[10,1.000,1e1000,{"z":1},null,-0]}
"#), "[\"Cannot index array with string \\\"x\\\"\",\"Cannot index array with string \\\"x\\\"\",\"Cannot index array with string \\\"x\\\"\",\"Cannot index array with string \\\"x\\\"\",\"Cannot index array with string \\\"x\\\"\",\"Cannot index array with string \\\"x\\\"\"]\n", "", 0),
    // doc_obj/reduce/gp_obj
    (&[r"-c", r#".a as $a | . as $o | reduce range(6) as $i ([]; . + [($o|getpath(["k",$i]))])"#], Some(r#"{"a":[0,1.000,"s",null,[1,2],3.5,7,8],"k":[10,1.000,1e1000,{"z":1},null,-0]}
"#), "[10,1.000,1E+1000,{\"z\":1},null,-0]\n", "", 0),
    // doc_obj/reduce/gp_two
    (&[r"-c", r".a as $a | . as $o | reduce range(6) as $i ([]; . + [($a|getpath([$i],[$i+1]))])"], Some(r#"{"a":[0,1.000,"s",null,[1,2],3.5,7,8],"k":[10,1.000,1e1000,{"z":1},null,-0]}
"#), "[0,1.000,1.000,\"s\",\"s\",null,null,[1,2],[1,2],3.5,3.5,7]\n", "", 0),
    // doc_obj/reduce/gp_in_if
    (&[r"-c", r#".a as $a | . as $o | reduce range(6) as $i ([]; . + [(if ($a|getpath([$i])) == null then "n" else "v" end)])"#], Some(r#"{"a":[0,1.000,"s",null,[1,2],3.5,7,8],"k":[10,1.000,1e1000,{"z":1},null,-0]}
"#), "[\"v\",\"v\",\"v\",\"n\",\"v\",\"v\"]\n", "", 0),
    // doc_obj/foreach_acc/getpath
    (&[r"-c", r".a as $a | . as $o | [foreach range(6) as $i ([]; . + [($a|getpath([$i]))]; length)]"], Some(r#"{"a":[0,1.000,"s",null,[1,2],3.5,7,8],"k":[10,1.000,1e1000,{"z":1},null,-0]}
"#), "[1,2,3,4,5,6]\n", "", 0),
    // doc_obj/foreach_acc/gp_float
    (&[r"-c", r".a as $a | . as $o | [foreach range(6) as $i ([]; . + [($a|getpath([$i+0.5]))]; length)]"], Some(r#"{"a":[0,1.000,"s",null,[1,2],3.5,7,8],"k":[10,1.000,1e1000,{"z":1},null,-0]}
"#), "[1,2,3,4,5,6]\n", "", 0),
    // doc_obj/foreach_acc/gp_slice
    (&[r"-c", r#".a as $a | . as $o | [foreach range(6) as $i ([]; . + [($a|getpath([{"start":$i,"end":($i+2)}]))]; length)]"#], Some(r#"{"a":[0,1.000,"s",null,[1,2],3.5,7,8],"k":[10,1.000,1e1000,{"z":1},null,-0]}
"#), "[1,2,3,4,5,6]\n", "", 0),
    // doc_obj/foreach_acc/gp_deep
    (&[r"-c", r#".a as $a | . as $o | [foreach range(6) as $i ([]; . + [($a|getpath([$i,"x"])?)]; length)]"#], Some(r#"{"a":[0,1.000,"s",null,[1,2],3.5,7,8],"k":[10,1.000,1e1000,{"z":1},null,-0]}
"#), "[0,0,0,1,1,1]\n", "", 0),
    // doc_obj/foreach_acc/gp_err
    (&[r"-c", r#".a as $a | . as $o | [foreach range(6) as $i ([]; . + [($a|try getpath(["x"]) catch .)]; length)]"#], Some(r#"{"a":[0,1.000,"s",null,[1,2],3.5,7,8],"k":[10,1.000,1e1000,{"z":1},null,-0]}
"#), "[1,2,3,4,5,6]\n", "", 0),
    // doc_obj/foreach_acc/gp_obj
    (&[r"-c", r#".a as $a | . as $o | [foreach range(6) as $i ([]; . + [($o|getpath(["k",$i]))]; length)]"#], Some(r#"{"a":[0,1.000,"s",null,[1,2],3.5,7,8],"k":[10,1.000,1e1000,{"z":1},null,-0]}
"#), "[1,2,3,4,5,6]\n", "", 0),
    // doc_obj/foreach_acc/gp_two
    (&[r"-c", r".a as $a | . as $o | [foreach range(6) as $i ([]; . + [($a|getpath([$i],[$i+1]))]; length)]"], Some(r#"{"a":[0,1.000,"s",null,[1,2],3.5,7,8],"k":[10,1.000,1e1000,{"z":1},null,-0]}
"#), "[2,4,6,8,10,12]\n", "", 0),
    // doc_obj/foreach_acc/gp_in_if
    (&[r"-c", r#".a as $a | . as $o | [foreach range(6) as $i ([]; . + [(if ($a|getpath([$i])) == null then "n" else "v" end)]; length)]"#], Some(r#"{"a":[0,1.000,"s",null,[1,2],3.5,7,8],"k":[10,1.000,1e1000,{"z":1},null,-0]}
"#), "[1,2,3,4,5,6]\n", "", 0),
    // doc_obj/until/getpath
    (&[r"-c", r".a as $a | . as $o | [0, []] | until(.[0] >= 6; .[0] as $i | [$i + 1, .[1] + [($a|getpath([$i]))]]) | .[1]"], Some(r#"{"a":[0,1.000,"s",null,[1,2],3.5,7,8],"k":[10,1.000,1e1000,{"z":1},null,-0]}
"#), "[0,1.000,\"s\",null,[1,2],3.5]\n", "", 0),
    // doc_obj/until/gp_float
    (&[r"-c", r".a as $a | . as $o | [0, []] | until(.[0] >= 6; .[0] as $i | [$i + 1, .[1] + [($a|getpath([$i+0.5]))]]) | .[1]"], Some(r#"{"a":[0,1.000,"s",null,[1,2],3.5,7,8],"k":[10,1.000,1e1000,{"z":1},null,-0]}
"#), "[0,1.000,\"s\",null,[1,2],3.5]\n", "", 0),
    // doc_obj/until/gp_slice
    (&[r"-c", r#".a as $a | . as $o | [0, []] | until(.[0] >= 6; .[0] as $i | [$i + 1, .[1] + [($a|getpath([{"start":$i,"end":($i+2)}]))]]) | .[1]"#], Some(r#"{"a":[0,1.000,"s",null,[1,2],3.5,7,8],"k":[10,1.000,1e1000,{"z":1},null,-0]}
"#), "[[0,1.000],[1.000,\"s\"],[\"s\",null],[null,[1,2]],[[1,2],3.5],[3.5,7]]\n", "", 0),
    // doc_obj/until/gp_deep
    (&[r"-c", r#".a as $a | . as $o | [0, []] | until(.[0] >= 6; .[0] as $i | [$i + 1, .[1] + [($a|getpath([$i,"x"])?)]]) | .[1]"#], Some(r#"{"a":[0,1.000,"s",null,[1,2],3.5,7,8],"k":[10,1.000,1e1000,{"z":1},null,-0]}
"#), "[null]\n", "", 0),
    // doc_obj/until/gp_err
    (&[r"-c", r#".a as $a | . as $o | [0, []] | until(.[0] >= 6; .[0] as $i | [$i + 1, .[1] + [($a|try getpath(["x"]) catch .)]]) | .[1]"#], Some(r#"{"a":[0,1.000,"s",null,[1,2],3.5,7,8],"k":[10,1.000,1e1000,{"z":1},null,-0]}
"#), "[\"Cannot index array with string \\\"x\\\"\",\"Cannot index array with string \\\"x\\\"\",\"Cannot index array with string \\\"x\\\"\",\"Cannot index array with string \\\"x\\\"\",\"Cannot index array with string \\\"x\\\"\",\"Cannot index array with string \\\"x\\\"\"]\n", "", 0),
    // doc_obj/until/gp_obj
    (&[r"-c", r#".a as $a | . as $o | [0, []] | until(.[0] >= 6; .[0] as $i | [$i + 1, .[1] + [($o|getpath(["k",$i]))]]) | .[1]"#], Some(r#"{"a":[0,1.000,"s",null,[1,2],3.5,7,8],"k":[10,1.000,1e1000,{"z":1},null,-0]}
"#), "[10,1.000,1E+1000,{\"z\":1},null,-0]\n", "", 0),
    // doc_obj/until/gp_two
    (&[r"-c", r".a as $a | . as $o | [0, []] | until(.[0] >= 6; .[0] as $i | [$i + 1, .[1] + [($a|getpath([$i],[$i+1]))]]) | .[1]"], Some(r#"{"a":[0,1.000,"s",null,[1,2],3.5,7,8],"k":[10,1.000,1e1000,{"z":1},null,-0]}
"#), "[0,1.000,1.000,\"s\",\"s\",null,null,[1,2],[1,2],3.5,3.5,7]\n", "", 0),
    // doc_obj/until/gp_in_if
    (&[r"-c", r#".a as $a | . as $o | [0, []] | until(.[0] >= 6; .[0] as $i | [$i + 1, .[1] + [(if ($a|getpath([$i])) == null then "n" else "v" end)]]) | .[1]"#], Some(r#"{"a":[0,1.000,"s",null,[1,2],3.5,7,8],"k":[10,1.000,1e1000,{"z":1},null,-0]}
"#), "[\"v\",\"v\",\"v\",\"n\",\"v\",\"v\"]\n", "", 0),
    // doc_obj/limit/getpath
    (&[r"-c", r".a as $a | . as $o | [limit(3; range(6) as $i | ($a|getpath([$i])))]"], Some(r#"{"a":[0,1.000,"s",null,[1,2],3.5,7,8],"k":[10,1.000,1e1000,{"z":1},null,-0]}
"#), "[0,1.000,\"s\"]\n", "", 0),
    // doc_obj/limit/gp_float
    (&[r"-c", r".a as $a | . as $o | [limit(3; range(6) as $i | ($a|getpath([$i+0.5])))]"], Some(r#"{"a":[0,1.000,"s",null,[1,2],3.5,7,8],"k":[10,1.000,1e1000,{"z":1},null,-0]}
"#), "[0,1.000,\"s\"]\n", "", 0),
    // doc_obj/limit/gp_slice
    (&[r"-c", r#".a as $a | . as $o | [limit(3; range(6) as $i | ($a|getpath([{"start":$i,"end":($i+2)}])))]"#], Some(r#"{"a":[0,1.000,"s",null,[1,2],3.5,7,8],"k":[10,1.000,1e1000,{"z":1},null,-0]}
"#), "[[0,1.000],[1.000,\"s\"],[\"s\",null]]\n", "", 0),
    // doc_obj/limit/gp_deep
    (&[r"-c", r#".a as $a | . as $o | [limit(3; range(6) as $i | ($a|getpath([$i,"x"])?))]"#], Some(r#"{"a":[0,1.000,"s",null,[1,2],3.5,7,8],"k":[10,1.000,1e1000,{"z":1},null,-0]}
"#), "[null]\n", "", 0),
    // doc_obj/limit/gp_err
    (&[r"-c", r#".a as $a | . as $o | [limit(3; range(6) as $i | ($a|try getpath(["x"]) catch .))]"#], Some(r#"{"a":[0,1.000,"s",null,[1,2],3.5,7,8],"k":[10,1.000,1e1000,{"z":1},null,-0]}
"#), "[\"Cannot index array with string \\\"x\\\"\",\"Cannot index array with string \\\"x\\\"\",\"Cannot index array with string \\\"x\\\"\"]\n", "", 0),
    // doc_obj/limit/gp_obj
    (&[r"-c", r#".a as $a | . as $o | [limit(3; range(6) as $i | ($o|getpath(["k",$i])))]"#], Some(r#"{"a":[0,1.000,"s",null,[1,2],3.5,7,8],"k":[10,1.000,1e1000,{"z":1},null,-0]}
"#), "[10,1.000,1E+1000]\n", "", 0),
    // doc_obj/limit/gp_two
    (&[r"-c", r".a as $a | . as $o | [limit(3; range(6) as $i | ($a|getpath([$i],[$i+1])))]"], Some(r#"{"a":[0,1.000,"s",null,[1,2],3.5,7,8],"k":[10,1.000,1e1000,{"z":1},null,-0]}
"#), "[0,1.000,1.000]\n", "", 0),
    // doc_obj/limit/gp_in_if
    (&[r"-c", r#".a as $a | . as $o | [limit(3; range(6) as $i | (if ($a|getpath([$i])) == null then "n" else "v" end))]"#], Some(r#"{"a":[0,1.000,"s",null,[1,2],3.5,7,8],"k":[10,1.000,1e1000,{"z":1},null,-0]}
"#), "[\"v\",\"v\",\"v\"]\n", "", 0),
    // input_n/reduce/getpath
    (&[r"-c", r"-n", r"input as $a | {k: $a} as $o | reduce range(6) as $i ([]; . + [($a|getpath([$i]))])"], Some(r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]
"#), "[0,1.000,1E+1000,100000000000000000001,\"s\",null]\n", "", 0),
    // input_n/reduce/gp_float
    (&[r"-c", r"-n", r"input as $a | {k: $a} as $o | reduce range(6) as $i ([]; . + [($a|getpath([$i+0.5]))])"], Some(r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]
"#), "[0,1.000,1E+1000,100000000000000000001,\"s\",null]\n", "", 0),
    // input_n/reduce/gp_slice
    (&[r"-c", r"-n", r#"input as $a | {k: $a} as $o | reduce range(6) as $i ([]; . + [($a|getpath([{"start":$i,"end":($i+2)}]))])"#], Some(r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]
"#), "[[0,1.000],[1.000,1E+1000],[1E+1000,100000000000000000001],[100000000000000000001,\"s\"],[\"s\",null],[null,{\"x\":1}]]\n", "", 0),
    // input_n/reduce/gp_deep
    (&[r"-c", r"-n", r#"input as $a | {k: $a} as $o | reduce range(6) as $i ([]; . + [($a|getpath([$i,"x"])?)])"#], Some(r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]
"#), "[null]\n", "", 0),
    // input_n/reduce/gp_err
    (&[r"-c", r"-n", r#"input as $a | {k: $a} as $o | reduce range(6) as $i ([]; . + [($a|try getpath(["x"]) catch .)])"#], Some(r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]
"#), "[\"Cannot index array with string \\\"x\\\"\",\"Cannot index array with string \\\"x\\\"\",\"Cannot index array with string \\\"x\\\"\",\"Cannot index array with string \\\"x\\\"\",\"Cannot index array with string \\\"x\\\"\",\"Cannot index array with string \\\"x\\\"\"]\n", "", 0),
    // input_n/reduce/gp_obj
    (&[r"-c", r"-n", r#"input as $a | {k: $a} as $o | reduce range(6) as $i ([]; . + [($o|getpath(["k",$i]))])"#], Some(r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]
"#), "[0,1.000,1E+1000,100000000000000000001,\"s\",null]\n", "", 0),
    // input_n/reduce/gp_two
    (&[r"-c", r"-n", r"input as $a | {k: $a} as $o | reduce range(6) as $i ([]; . + [($a|getpath([$i],[$i+1]))])"], Some(r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]
"#), "[0,1.000,1.000,1E+1000,1E+1000,100000000000000000001,100000000000000000001,\"s\",\"s\",null,null,{\"x\":1}]\n", "", 0),
    // input_n/reduce/gp_in_if
    (&[r"-c", r"-n", r#"input as $a | {k: $a} as $o | reduce range(6) as $i ([]; . + [(if ($a|getpath([$i])) == null then "n" else "v" end)])"#], Some(r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]
"#), "[\"v\",\"v\",\"v\",\"v\",\"v\",\"n\"]\n", "", 0),
    // input_n/foreach_acc/getpath
    (&[r"-c", r"-n", r"input as $a | {k: $a} as $o | [foreach range(6) as $i ([]; . + [($a|getpath([$i]))]; length)]"], Some(r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]
"#), "[1,2,3,4,5,6]\n", "", 0),
    // input_n/foreach_acc/gp_float
    (&[r"-c", r"-n", r"input as $a | {k: $a} as $o | [foreach range(6) as $i ([]; . + [($a|getpath([$i+0.5]))]; length)]"], Some(r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]
"#), "[1,2,3,4,5,6]\n", "", 0),
    // input_n/foreach_acc/gp_slice
    (&[r"-c", r"-n", r#"input as $a | {k: $a} as $o | [foreach range(6) as $i ([]; . + [($a|getpath([{"start":$i,"end":($i+2)}]))]; length)]"#], Some(r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]
"#), "[1,2,3,4,5,6]\n", "", 0),
    // input_n/foreach_acc/gp_deep
    (&[r"-c", r"-n", r#"input as $a | {k: $a} as $o | [foreach range(6) as $i ([]; . + [($a|getpath([$i,"x"])?)]; length)]"#], Some(r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]
"#), "[0,0,0,0,0,1]\n", "", 0),
    // input_n/foreach_acc/gp_err
    (&[r"-c", r"-n", r#"input as $a | {k: $a} as $o | [foreach range(6) as $i ([]; . + [($a|try getpath(["x"]) catch .)]; length)]"#], Some(r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]
"#), "[1,2,3,4,5,6]\n", "", 0),
    // input_n/foreach_acc/gp_obj
    (&[r"-c", r"-n", r#"input as $a | {k: $a} as $o | [foreach range(6) as $i ([]; . + [($o|getpath(["k",$i]))]; length)]"#], Some(r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]
"#), "[1,2,3,4,5,6]\n", "", 0),
    // input_n/foreach_acc/gp_two
    (&[r"-c", r"-n", r"input as $a | {k: $a} as $o | [foreach range(6) as $i ([]; . + [($a|getpath([$i],[$i+1]))]; length)]"], Some(r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]
"#), "[2,4,6,8,10,12]\n", "", 0),
    // input_n/foreach_acc/gp_in_if
    (&[r"-c", r"-n", r#"input as $a | {k: $a} as $o | [foreach range(6) as $i ([]; . + [(if ($a|getpath([$i])) == null then "n" else "v" end)]; length)]"#], Some(r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]
"#), "[1,2,3,4,5,6]\n", "", 0),
    // input_n/until/getpath
    (&[r"-c", r"-n", r"input as $a | {k: $a} as $o | [0, []] | until(.[0] >= 6; .[0] as $i | [$i + 1, .[1] + [($a|getpath([$i]))]]) | .[1]"], Some(r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]
"#), "[0,1.000,1E+1000,100000000000000000001,\"s\",null]\n", "", 0),
    // input_n/until/gp_float
    (&[r"-c", r"-n", r"input as $a | {k: $a} as $o | [0, []] | until(.[0] >= 6; .[0] as $i | [$i + 1, .[1] + [($a|getpath([$i+0.5]))]]) | .[1]"], Some(r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]
"#), "[0,1.000,1E+1000,100000000000000000001,\"s\",null]\n", "", 0),
    // input_n/until/gp_slice
    (&[r"-c", r"-n", r#"input as $a | {k: $a} as $o | [0, []] | until(.[0] >= 6; .[0] as $i | [$i + 1, .[1] + [($a|getpath([{"start":$i,"end":($i+2)}]))]]) | .[1]"#], Some(r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]
"#), "[[0,1.000],[1.000,1E+1000],[1E+1000,100000000000000000001],[100000000000000000001,\"s\"],[\"s\",null],[null,{\"x\":1}]]\n", "", 0),
    // input_n/until/gp_deep
    (&[r"-c", r"-n", r#"input as $a | {k: $a} as $o | [0, []] | until(.[0] >= 6; .[0] as $i | [$i + 1, .[1] + [($a|getpath([$i,"x"])?)]]) | .[1]"#], Some(r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]
"#), "[null]\n", "", 0),
    // input_n/until/gp_err
    (&[r"-c", r"-n", r#"input as $a | {k: $a} as $o | [0, []] | until(.[0] >= 6; .[0] as $i | [$i + 1, .[1] + [($a|try getpath(["x"]) catch .)]]) | .[1]"#], Some(r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]
"#), "[\"Cannot index array with string \\\"x\\\"\",\"Cannot index array with string \\\"x\\\"\",\"Cannot index array with string \\\"x\\\"\",\"Cannot index array with string \\\"x\\\"\",\"Cannot index array with string \\\"x\\\"\",\"Cannot index array with string \\\"x\\\"\"]\n", "", 0),
    // input_n/until/gp_obj
    (&[r"-c", r"-n", r#"input as $a | {k: $a} as $o | [0, []] | until(.[0] >= 6; .[0] as $i | [$i + 1, .[1] + [($o|getpath(["k",$i]))]]) | .[1]"#], Some(r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]
"#), "[0,1.000,1E+1000,100000000000000000001,\"s\",null]\n", "", 0),
    // input_n/until/gp_two
    (&[r"-c", r"-n", r"input as $a | {k: $a} as $o | [0, []] | until(.[0] >= 6; .[0] as $i | [$i + 1, .[1] + [($a|getpath([$i],[$i+1]))]]) | .[1]"], Some(r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]
"#), "[0,1.000,1.000,1E+1000,1E+1000,100000000000000000001,100000000000000000001,\"s\",\"s\",null,null,{\"x\":1}]\n", "", 0),
    // input_n/until/gp_in_if
    (&[r"-c", r"-n", r#"input as $a | {k: $a} as $o | [0, []] | until(.[0] >= 6; .[0] as $i | [$i + 1, .[1] + [(if ($a|getpath([$i])) == null then "n" else "v" end)]]) | .[1]"#], Some(r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]
"#), "[\"v\",\"v\",\"v\",\"v\",\"v\",\"n\"]\n", "", 0),
    // input_n/limit/getpath
    (&[r"-c", r"-n", r"input as $a | {k: $a} as $o | [limit(3; range(6) as $i | ($a|getpath([$i])))]"], Some(r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]
"#), "[0,1.000,1E+1000]\n", "", 0),
    // input_n/limit/gp_float
    (&[r"-c", r"-n", r"input as $a | {k: $a} as $o | [limit(3; range(6) as $i | ($a|getpath([$i+0.5])))]"], Some(r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]
"#), "[0,1.000,1E+1000]\n", "", 0),
    // input_n/limit/gp_slice
    (&[r"-c", r"-n", r#"input as $a | {k: $a} as $o | [limit(3; range(6) as $i | ($a|getpath([{"start":$i,"end":($i+2)}])))]"#], Some(r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]
"#), "[[0,1.000],[1.000,1E+1000],[1E+1000,100000000000000000001]]\n", "", 0),
    // input_n/limit/gp_deep
    (&[r"-c", r"-n", r#"input as $a | {k: $a} as $o | [limit(3; range(6) as $i | ($a|getpath([$i,"x"])?))]"#], Some(r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]
"#), "[null]\n", "", 0),
    // input_n/limit/gp_err
    (&[r"-c", r"-n", r#"input as $a | {k: $a} as $o | [limit(3; range(6) as $i | ($a|try getpath(["x"]) catch .))]"#], Some(r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]
"#), "[\"Cannot index array with string \\\"x\\\"\",\"Cannot index array with string \\\"x\\\"\",\"Cannot index array with string \\\"x\\\"\"]\n", "", 0),
    // input_n/limit/gp_obj
    (&[r"-c", r"-n", r#"input as $a | {k: $a} as $o | [limit(3; range(6) as $i | ($o|getpath(["k",$i])))]"#], Some(r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]
"#), "[0,1.000,1E+1000]\n", "", 0),
    // input_n/limit/gp_two
    (&[r"-c", r"-n", r"input as $a | {k: $a} as $o | [limit(3; range(6) as $i | ($a|getpath([$i],[$i+1])))]"], Some(r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]
"#), "[0,1.000,1.000]\n", "", 0),
    // input_n/limit/gp_in_if
    (&[r"-c", r"-n", r#"input as $a | {k: $a} as $o | [limit(3; range(6) as $i | (if ($a|getpath([$i])) == null then "n" else "v" end))]"#], Some(r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]
"#), "[\"v\",\"v\",\"v\"]\n", "", 0),
    // argjson/reduce/getpath
    (&[r"-c", r"-n", r"--argjson", r"a", r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]"#, r"--argjson", r"o", r#"{"k":[7,8,9]}"#, r"reduce range(6) as $i ([]; . + [($a|getpath([$i]))])"], None, "[0,1.000,1E+1000,100000000000000000001,\"s\",null]\n", "", 0),
    // argjson/reduce/gp_float
    (&[r"-c", r"-n", r"--argjson", r"a", r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]"#, r"--argjson", r"o", r#"{"k":[7,8,9]}"#, r"reduce range(6) as $i ([]; . + [($a|getpath([$i+0.5]))])"], None, "[0,1.000,1E+1000,100000000000000000001,\"s\",null]\n", "", 0),
    // argjson/reduce/gp_slice
    (&[r"-c", r"-n", r"--argjson", r"a", r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]"#, r"--argjson", r"o", r#"{"k":[7,8,9]}"#, r#"reduce range(6) as $i ([]; . + [($a|getpath([{"start":$i,"end":($i+2)}]))])"#], None, "[[0,1.000],[1.000,1E+1000],[1E+1000,100000000000000000001],[100000000000000000001,\"s\"],[\"s\",null],[null,{\"x\":1}]]\n", "", 0),
    // argjson/reduce/gp_deep
    (&[r"-c", r"-n", r"--argjson", r"a", r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]"#, r"--argjson", r"o", r#"{"k":[7,8,9]}"#, r#"reduce range(6) as $i ([]; . + [($a|getpath([$i,"x"])?)])"#], None, "[null]\n", "", 0),
    // argjson/reduce/gp_err
    (&[r"-c", r"-n", r"--argjson", r"a", r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]"#, r"--argjson", r"o", r#"{"k":[7,8,9]}"#, r#"reduce range(6) as $i ([]; . + [($a|try getpath(["x"]) catch .)])"#], None, "[\"Cannot index array with string \\\"x\\\"\",\"Cannot index array with string \\\"x\\\"\",\"Cannot index array with string \\\"x\\\"\",\"Cannot index array with string \\\"x\\\"\",\"Cannot index array with string \\\"x\\\"\",\"Cannot index array with string \\\"x\\\"\"]\n", "", 0),
    // argjson/reduce/gp_obj
    (&[r"-c", r"-n", r"--argjson", r"a", r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]"#, r"--argjson", r"o", r#"{"k":[7,8,9]}"#, r#"reduce range(6) as $i ([]; . + [($o|getpath(["k",$i]))])"#], None, "[7,8,9,null,null,null]\n", "", 0),
    // argjson/reduce/gp_two
    (&[r"-c", r"-n", r"--argjson", r"a", r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]"#, r"--argjson", r"o", r#"{"k":[7,8,9]}"#, r"reduce range(6) as $i ([]; . + [($a|getpath([$i],[$i+1]))])"], None, "[0,1.000,1.000,1E+1000,1E+1000,100000000000000000001,100000000000000000001,\"s\",\"s\",null,null,{\"x\":1}]\n", "", 0),
    // argjson/reduce/gp_in_if
    (&[r"-c", r"-n", r"--argjson", r"a", r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]"#, r"--argjson", r"o", r#"{"k":[7,8,9]}"#, r#"reduce range(6) as $i ([]; . + [(if ($a|getpath([$i])) == null then "n" else "v" end)])"#], None, "[\"v\",\"v\",\"v\",\"v\",\"v\",\"n\"]\n", "", 0),
    // argjson/foreach_acc/getpath
    (&[r"-c", r"-n", r"--argjson", r"a", r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]"#, r"--argjson", r"o", r#"{"k":[7,8,9]}"#, r"[foreach range(6) as $i ([]; . + [($a|getpath([$i]))]; length)]"], None, "[1,2,3,4,5,6]\n", "", 0),
    // argjson/foreach_acc/gp_float
    (&[r"-c", r"-n", r"--argjson", r"a", r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]"#, r"--argjson", r"o", r#"{"k":[7,8,9]}"#, r"[foreach range(6) as $i ([]; . + [($a|getpath([$i+0.5]))]; length)]"], None, "[1,2,3,4,5,6]\n", "", 0),
    // argjson/foreach_acc/gp_slice
    (&[r"-c", r"-n", r"--argjson", r"a", r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]"#, r"--argjson", r"o", r#"{"k":[7,8,9]}"#, r#"[foreach range(6) as $i ([]; . + [($a|getpath([{"start":$i,"end":($i+2)}]))]; length)]"#], None, "[1,2,3,4,5,6]\n", "", 0),
    // argjson/foreach_acc/gp_deep
    (&[r"-c", r"-n", r"--argjson", r"a", r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]"#, r"--argjson", r"o", r#"{"k":[7,8,9]}"#, r#"[foreach range(6) as $i ([]; . + [($a|getpath([$i,"x"])?)]; length)]"#], None, "[0,0,0,0,0,1]\n", "", 0),
    // argjson/foreach_acc/gp_err
    (&[r"-c", r"-n", r"--argjson", r"a", r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]"#, r"--argjson", r"o", r#"{"k":[7,8,9]}"#, r#"[foreach range(6) as $i ([]; . + [($a|try getpath(["x"]) catch .)]; length)]"#], None, "[1,2,3,4,5,6]\n", "", 0),
    // argjson/foreach_acc/gp_obj
    (&[r"-c", r"-n", r"--argjson", r"a", r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]"#, r"--argjson", r"o", r#"{"k":[7,8,9]}"#, r#"[foreach range(6) as $i ([]; . + [($o|getpath(["k",$i]))]; length)]"#], None, "[1,2,3,4,5,6]\n", "", 0),
    // argjson/foreach_acc/gp_two
    (&[r"-c", r"-n", r"--argjson", r"a", r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]"#, r"--argjson", r"o", r#"{"k":[7,8,9]}"#, r"[foreach range(6) as $i ([]; . + [($a|getpath([$i],[$i+1]))]; length)]"], None, "[2,4,6,8,10,12]\n", "", 0),
    // argjson/foreach_acc/gp_in_if
    (&[r"-c", r"-n", r"--argjson", r"a", r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]"#, r"--argjson", r"o", r#"{"k":[7,8,9]}"#, r#"[foreach range(6) as $i ([]; . + [(if ($a|getpath([$i])) == null then "n" else "v" end)]; length)]"#], None, "[1,2,3,4,5,6]\n", "", 0),
    // argjson/until/getpath
    (&[r"-c", r"-n", r"--argjson", r"a", r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]"#, r"--argjson", r"o", r#"{"k":[7,8,9]}"#, r"[0, []] | until(.[0] >= 6; .[0] as $i | [$i + 1, .[1] + [($a|getpath([$i]))]]) | .[1]"], None, "[0,1.000,1E+1000,100000000000000000001,\"s\",null]\n", "", 0),
    // argjson/until/gp_float
    (&[r"-c", r"-n", r"--argjson", r"a", r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]"#, r"--argjson", r"o", r#"{"k":[7,8,9]}"#, r"[0, []] | until(.[0] >= 6; .[0] as $i | [$i + 1, .[1] + [($a|getpath([$i+0.5]))]]) | .[1]"], None, "[0,1.000,1E+1000,100000000000000000001,\"s\",null]\n", "", 0),
    // argjson/until/gp_slice
    (&[r"-c", r"-n", r"--argjson", r"a", r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]"#, r"--argjson", r"o", r#"{"k":[7,8,9]}"#, r#"[0, []] | until(.[0] >= 6; .[0] as $i | [$i + 1, .[1] + [($a|getpath([{"start":$i,"end":($i+2)}]))]]) | .[1]"#], None, "[[0,1.000],[1.000,1E+1000],[1E+1000,100000000000000000001],[100000000000000000001,\"s\"],[\"s\",null],[null,{\"x\":1}]]\n", "", 0),
    // argjson/until/gp_deep
    (&[r"-c", r"-n", r"--argjson", r"a", r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]"#, r"--argjson", r"o", r#"{"k":[7,8,9]}"#, r#"[0, []] | until(.[0] >= 6; .[0] as $i | [$i + 1, .[1] + [($a|getpath([$i,"x"])?)]]) | .[1]"#], None, "[null]\n", "", 0),
    // argjson/until/gp_err
    (&[r"-c", r"-n", r"--argjson", r"a", r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]"#, r"--argjson", r"o", r#"{"k":[7,8,9]}"#, r#"[0, []] | until(.[0] >= 6; .[0] as $i | [$i + 1, .[1] + [($a|try getpath(["x"]) catch .)]]) | .[1]"#], None, "[\"Cannot index array with string \\\"x\\\"\",\"Cannot index array with string \\\"x\\\"\",\"Cannot index array with string \\\"x\\\"\",\"Cannot index array with string \\\"x\\\"\",\"Cannot index array with string \\\"x\\\"\",\"Cannot index array with string \\\"x\\\"\"]\n", "", 0),
    // argjson/until/gp_obj
    (&[r"-c", r"-n", r"--argjson", r"a", r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]"#, r"--argjson", r"o", r#"{"k":[7,8,9]}"#, r#"[0, []] | until(.[0] >= 6; .[0] as $i | [$i + 1, .[1] + [($o|getpath(["k",$i]))]]) | .[1]"#], None, "[7,8,9,null,null,null]\n", "", 0),
    // argjson/until/gp_two
    (&[r"-c", r"-n", r"--argjson", r"a", r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]"#, r"--argjson", r"o", r#"{"k":[7,8,9]}"#, r"[0, []] | until(.[0] >= 6; .[0] as $i | [$i + 1, .[1] + [($a|getpath([$i],[$i+1]))]]) | .[1]"], None, "[0,1.000,1.000,1E+1000,1E+1000,100000000000000000001,100000000000000000001,\"s\",\"s\",null,null,{\"x\":1}]\n", "", 0),
    // argjson/until/gp_in_if
    (&[r"-c", r"-n", r"--argjson", r"a", r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]"#, r"--argjson", r"o", r#"{"k":[7,8,9]}"#, r#"[0, []] | until(.[0] >= 6; .[0] as $i | [$i + 1, .[1] + [(if ($a|getpath([$i])) == null then "n" else "v" end)]]) | .[1]"#], None, "[\"v\",\"v\",\"v\",\"v\",\"v\",\"n\"]\n", "", 0),
    // argjson/limit/getpath
    (&[r"-c", r"-n", r"--argjson", r"a", r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]"#, r"--argjson", r"o", r#"{"k":[7,8,9]}"#, r"[limit(3; range(6) as $i | ($a|getpath([$i])))]"], None, "[0,1.000,1E+1000]\n", "", 0),
    // argjson/limit/gp_float
    (&[r"-c", r"-n", r"--argjson", r"a", r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]"#, r"--argjson", r"o", r#"{"k":[7,8,9]}"#, r"[limit(3; range(6) as $i | ($a|getpath([$i+0.5])))]"], None, "[0,1.000,1E+1000]\n", "", 0),
    // argjson/limit/gp_slice
    (&[r"-c", r"-n", r"--argjson", r"a", r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]"#, r"--argjson", r"o", r#"{"k":[7,8,9]}"#, r#"[limit(3; range(6) as $i | ($a|getpath([{"start":$i,"end":($i+2)}])))]"#], None, "[[0,1.000],[1.000,1E+1000],[1E+1000,100000000000000000001]]\n", "", 0),
    // argjson/limit/gp_deep
    (&[r"-c", r"-n", r"--argjson", r"a", r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]"#, r"--argjson", r"o", r#"{"k":[7,8,9]}"#, r#"[limit(3; range(6) as $i | ($a|getpath([$i,"x"])?))]"#], None, "[null]\n", "", 0),
    // argjson/limit/gp_err
    (&[r"-c", r"-n", r"--argjson", r"a", r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]"#, r"--argjson", r"o", r#"{"k":[7,8,9]}"#, r#"[limit(3; range(6) as $i | ($a|try getpath(["x"]) catch .))]"#], None, "[\"Cannot index array with string \\\"x\\\"\",\"Cannot index array with string \\\"x\\\"\",\"Cannot index array with string \\\"x\\\"\"]\n", "", 0),
    // argjson/limit/gp_obj
    (&[r"-c", r"-n", r"--argjson", r"a", r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]"#, r"--argjson", r"o", r#"{"k":[7,8,9]}"#, r#"[limit(3; range(6) as $i | ($o|getpath(["k",$i])))]"#], None, "[7,8,9]\n", "", 0),
    // argjson/limit/gp_two
    (&[r"-c", r"-n", r"--argjson", r"a", r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]"#, r"--argjson", r"o", r#"{"k":[7,8,9]}"#, r"[limit(3; range(6) as $i | ($a|getpath([$i],[$i+1])))]"], None, "[0,1.000,1.000]\n", "", 0),
    // argjson/limit/gp_in_if
    (&[r"-c", r"-n", r"--argjson", r"a", r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]"#, r"--argjson", r"o", r#"{"k":[7,8,9]}"#, r#"[limit(3; range(6) as $i | (if ($a|getpath([$i])) == null then "n" else "v" end))]"#], None, "[\"v\",\"v\",\"v\"]\n", "", 0),
];

#[test]
fn test_getpath_of_a_bound_container_in_a_fold_matches_jq_7721() -> Result<()> {
    for (args, stdin, stdout, stderr, code) in ROWS {
        let got = run_jq_full(args, *stdin)?;
        assert_eq!(
            (got.0.as_str(), got.1.as_str(), got.2),
            (*stdout, *stderr, *code),
            "{args:?}"
        );
    }
    Ok(())
}

/// (program, stdout, stderr, exit code) for `succinctly jq -nc <program>`: paths that raise, are not
/// arrays, slice, fan out or are computed; reads under `?`, `try`, `debug`, `label`/`break`,
/// `first(foreach ...)`; numbers the bridge spells from their literal; and the `as` binds of a read.
const EDGES: &[(&str, &str, &str, i32)] = &[
    (
        r#"[range(4)] as $a | reduce range(3) as $i (0; . + ($a|getpath(["x"])))"#,
        "",
        "jq: error (at <unknown>): Cannot index array with string \"x\"\n",
        5,
    ),
    (
        r"[range(4)] as $a | reduce range(3) as $i (0; . + ($a|getpath(null)))",
        "",
        "jq: error (at <unknown>): Path must be specified as an array\n",
        5,
    ),
    (
        r#"{k:[1,2]} as $o | reduce range(3) as $i (0; . + ($o|getpath(["k","x"])))"#,
        "",
        "jq: error (at <unknown>): Cannot index array with string \"x\"\n",
        5,
    ),
    (
        r"[range(4)] as $a | [range(3) as $i | $a|getpath([$i,0])]",
        "",
        "jq: error (at <unknown>): Cannot index number with number\n",
        5,
    ),
    (
        r"[range(4)] as $a | [foreach range(3) as $i (0; . + ($a|getpath([$i,[1]])))]",
        "",
        "jq: error (at <unknown>): Cannot index number with array\n",
        5,
    ),
    (
        r#"[[1,2],[3]] as $a | [limit(2; range(3) as $i | $a|getpath([$i,{"start":1}]))]"#,
        "",
        "jq: error (at <unknown>): Array/string slice indices must be integers\n",
        5,
    ),
    (
        r#"[[1,2],[3]] as $a | [limit(2; range(3) as $i | $a|getpath([$i,{"start":1,"end":null}]))]"#,
        "[[2],[]]\n",
        "",
        0,
    ),
    (
        r"[range(4)] as $a | reduce range(3) as $i ([]; . + [$a|getpath([$i])?])",
        "[0,1,2]\n",
        "",
        0,
    ),
    (
        r"[range(4)] as $a | reduce range(3) as $i (0; . + ($a|getpath([$i])|debug))",
        "3\n",
        "[\"DEBUG:\",0]\n[\"DEBUG:\",1]\n[\"DEBUG:\",2]\n",
        0,
    ),
    (
        r#""abc" as $s | reduce range(3) as $i ([]; . + [$s|getpath([{"start":$i}])])"#,
        "",
        "jq: error (at <unknown>): Array/string slice indices must be integers\n",
        5,
    ),
    (
        r#""abc" as $s | reduce range(3) as $i ([]; . + [$s|getpath([{"start":$i,"end":3}])])"#,
        "[\"abc\",\"bc\",\"c\"]\n",
        "",
        0,
    ),
    (
        r#"null as $n | reduce range(3) as $i ([]; . + [$n|getpath(["a",$i])])"#,
        "[null,null,null]\n",
        "",
        0,
    ),
    (
        r"null as $n | reduce range(3) as $i ([]; . + [$n|getpath([true])])",
        "",
        "jq: error (at <unknown>): Cannot index null with boolean\n",
        5,
    ),
    (
        r"[range(3)] as $a | [range(3) as $i | $a | getpath([$i]) | . * 2]",
        "[0,2,4]\n",
        "",
        0,
    ),
    (
        r"[[0,1],[2,3]] as $a | reduce range(2) as $i (0; . + ($a|getpath([$i,1])))",
        "4\n",
        "",
        0,
    ),
    (
        r"[9007199254740993, 100000000000000000001, 1.000, 1e1000] as $a | [range(4) as $i | $a|getpath([$i])]",
        "[9007199254740993,100000000000000000001,1.000,1E+1000]\n",
        "",
        0,
    ),
    (
        r"[9007199254740993, 100000000000000000001, 1.000, 1e1000] as $a | reduce range(4) as $i ([]; . + [$a|getpath([$i])])",
        "[9007199254740993,100000000000000000001,1.000,1E+1000]\n",
        "",
        0,
    ),
    (
        r"[range(4)] as $a | reduce range(3) as $i ([]; . + [$a|getpath([$i], [$i+1])])",
        "[0,1,1,2,2,3]\n",
        "",
        0,
    ),
    (
        r"[range(4)] as $a | [range(3) as $i | $a|getpath([$i], [$i+1])]",
        "[0,1,1,2,2,3]\n",
        "",
        0,
    ),
    (
        r"[range(4)] as $a | [range(3) as $i | $a|getpath([$i] | (., .))] | length",
        "6\n",
        "",
        0,
    ),
    (
        r"[range(4)] as $a | reduce range(3) as $i (0; . + ($a|getpath([$i|tostring|tonumber])))",
        "3\n",
        "",
        0,
    ),
    (
        r"[[range(4)]] as $a | reduce range(3) as $i ([]; . + [$a|getpath([0])|length])",
        "[4,4,4]\n",
        "",
        0,
    ),
    (
        r"[range(4)] as $a | label $out | reduce range(3) as $i (0; if $i == 2 then break $out else . + ($a|getpath([$i])) end)",
        "",
        "",
        0,
    ),
    (
        r"[range(4)] as $a | [first(foreach range(3) as $i (0; . + ($a|getpath([$i]))))]",
        "[0]\n",
        "",
        0,
    ),
    (
        r#"[range(4)] as $a | [range(3) as $i | $a | try getpath(["x"]) catch "caught"]"#,
        "[\"caught\",\"caught\",\"caught\"]\n",
        "",
        0,
    ),
    (
        r#"{"a":{"b":[7,8]}} as $o | [range(2) as $i | $o|getpath(["a","b",$i])]"#,
        "[7,8]\n",
        "",
        0,
    ),
    (
        r#"{"a":{"b":[7,8]}} as $o | reduce range(2) as $i ({}; .["\($i)"] = ($o|getpath(["a","b",$i])))"#,
        "{\"0\":7,\"1\":8}\n",
        "",
        0,
    ),
    (
        r#"[range(5)] as $a | reduce range(3) as $i ([]; .[$i] = ($a|getpath([{"start":$i,"end":($i+2)}])))"#,
        "[[0,1],[1,2],[2,3]]\n",
        "",
        0,
    ),
    (
        r"[range(4)] as $a | [range(3) as $i | $a | getpath([$i]) as $v | $v + 1]",
        "[1,2,3]\n",
        "",
        0,
    ),
    (
        r"[range(4)] as $a | reduce range(3) as $i (0; ($a|getpath([$i])) as $v | . + $v)",
        "3\n",
        "",
        0,
    ),
];

#[test]
fn test_getpath_of_a_bound_container_edges_match_jq_7721() -> Result<()> {
    for (program, stdout, stderr, code) in EDGES {
        let got = run_jq_full(&["-nc", program], None)?;
        assert_eq!(
            (got.0.as_str(), got.1.as_str(), got.2),
            (*stdout, *stderr, *code),
            "{program}"
        );
    }
    Ok(())
}
