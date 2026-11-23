//! #7785: `$a | contains(C)` over a bound container, with a needle `C` that never reads `.`,
//! re-indexed the whole container for every call, as `getpath` did before #7721: inside a fold
//! step the closed-pipe walk had no `contains` stage, and in a body that asks once per output the
//! owned doors had no `contains` door for a needle that is not a string literal. jq searches the
//! value in place and stops at the first match. Both routes now ask the bridge's own search
//! (`owned_contains`) of the bound value, after the bridge's own kind check; a needle of another
//! kind (the containment error) or one that is not closed still takes the evaluator's route.
//!
//! Every expected line was captured from jq 1.7.1 (`/usr/bin/jq`); jq 1.8.2 answers the same but
//! for the longer value previews in its containment error (a 1.8 improvement, kept at 1.7.1). The
//! rows cover nested arrays and objects, substrings, mixed kinds, duplicates, an empty needle, a
//! needle longer than the haystack, `1` against `1.0`, `null`, and literal, computed, mixed-number,
//! document, `input` and `--argjson` containers in each fold kind. The in-place proofs are the unit
//! tests `contains_of_a_bound_container_reads_in_place_7785` and
//! `fold_step_storing_a_contains_answer_copies_nothing_7785` in `src/jq/eval.rs`.

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
    // lit_n/reduce/c_one
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | reduce range(6) as $i ([]; . + [($a|contains([$i]))])"], None, "[true,true,true,true,true,true]\n", "", 0),
    // lit_n/reduce/c_dup
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | reduce range(6) as $i ([]; . + [($a|contains([$i, $i]))])"], None, "[true,true,true,true,true,true]\n", "", 0),
    // lit_n/reduce/c_empty
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | reduce range(6) as $i ([]; . + [($a|contains([]))])"], None, "[true,true,true,true,true,true]\n", "", 0),
    // lit_n/reduce/c_long
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | reduce range(6) as $i ([]; . + [($a|contains([range(20)]))])"], None, "[false,false,false,false,false,false]\n", "", 0),
    // lit_n/reduce/c_float
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | reduce range(6) as $i ([]; . + [($a|contains([1.0]))])"], None, "[true,true,true,true,true,true]\n", "", 0),
    // lit_n/reduce/c_float3
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | reduce range(6) as $i ([]; . + [($a|contains([1.000, 3.5]))])"], None, "[false,false,false,false,false,false]\n", "", 0),
    // lit_n/reduce/c_null
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | reduce range(6) as $i ([]; . + [($a|contains([null]))])"], None, "[false,false,false,false,false,false]\n", "", 0),
    // lit_n/reduce/c_str_err
    (&[r"-c", r"-n", r#"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | reduce range(6) as $i ([]; . + [($a|try contains("x") catch .)])"#], None, "[\"array ([0,1,2,3,4,...) and string (\\\"x\\\") cannot have their containment checked\",\"array ([0,1,2,3,4,...) and string (\\\"x\\\") cannot have their containment checked\",\"array ([0,1,2,3,4,...) and string (\\\"x\\\") cannot have their containment checked\",\"array ([0,1,2,3,4,...) and string (\\\"x\\\") cannot have their containment checked\",\"array ([0,1,2,3,4,...) and string (\\\"x\\\") cannot have their containment checked\",\"array ([0,1,2,3,4,...) and string (\\\"x\\\") cannot have their containment checked\"]\n", "", 0),
    // lit_n/reduce/c_obj_err
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | reduce range(6) as $i ([]; . + [($o|try contains([1]) catch .)])"], None, "[\"object ({\\\"k\\\":[0,1,2...) and array ([1]) cannot have their containment checked\",\"object ({\\\"k\\\":[0,1,2...) and array ([1]) cannot have their containment checked\",\"object ({\\\"k\\\":[0,1,2...) and array ([1]) cannot have their containment checked\",\"object ({\\\"k\\\":[0,1,2...) and array ([1]) cannot have their containment checked\",\"object ({\\\"k\\\":[0,1,2...) and array ([1]) cannot have their containment checked\",\"object ({\\\"k\\\":[0,1,2...) and array ([1]) cannot have their containment checked\"]\n", "", 0),
    // lit_n/reduce/c_num_err
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | reduce range(6) as $i ([]; . + [($a|try contains(1) catch .)])"], None, "[\"array ([0,1,2,3,4,...) and number (1) cannot have their containment checked\",\"array ([0,1,2,3,4,...) and number (1) cannot have their containment checked\",\"array ([0,1,2,3,4,...) and number (1) cannot have their containment checked\",\"array ([0,1,2,3,4,...) and number (1) cannot have their containment checked\",\"array ([0,1,2,3,4,...) and number (1) cannot have their containment checked\",\"array ([0,1,2,3,4,...) and number (1) cannot have their containment checked\"]\n", "", 0),
    // lit_n/reduce/c_nested
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | reduce range(6) as $i ([]; . + [($a|contains([[1]]))])"], None, "[false,false,false,false,false,false]\n", "", 0),
    // lit_n/reduce/c_nested2
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | reduce range(6) as $i ([]; . + [($a|contains([[2,1]]))])"], None, "[false,false,false,false,false,false]\n", "", 0),
    // lit_n/reduce/c_objelem
    (&[r"-c", r"-n", r#"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | reduce range(6) as $i ([]; . + [($a|contains([{"x":1}]))])"#], None, "[false,false,false,false,false,false]\n", "", 0),
    // lit_n/reduce/c_emptyobj
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | reduce range(6) as $i ([]; . + [($a|contains([{}]))])"], None, "[false,false,false,false,false,false]\n", "", 0),
    // lit_n/reduce/c_substr
    (&[r"-c", r"-n", r#"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | reduce range(6) as $i ([]; . + [($a|contains(["s"]))])"#], None, "[false,false,false,false,false,false]\n", "", 0),
    // lit_n/reduce/c_interp
    (&[r"-c", r"-n", r#"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | reduce range(6) as $i ([]; . + [($a|contains(["\($i)"]))])"#], None, "[false,false,false,false,false,false]\n", "", 0),
    // lit_n/reduce/c_mixed_inner
    (&[r"-c", r"-n", r#"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | reduce range(6) as $i ([]; . + [($a|contains([[1,"a"]]))])"#], None, "[false,false,false,false,false,false]\n", "", 0),
    // lit_n/reduce/c_obj_k
    (&[r"-c", r"-n", r#"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | reduce range(6) as $i ([]; . + [($o|contains({"k":[$i]}))])"#], None, "[true,true,true,true,true,true]\n", "", 0),
    // lit_n/reduce/c_obj_kempty
    (&[r"-c", r"-n", r#"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | reduce range(6) as $i ([]; . + [($o|contains({"k":[]}))])"#], None, "[true,true,true,true,true,true]\n", "", 0),
    // lit_n/reduce/c_obj_miss
    (&[r"-c", r"-n", r#"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | reduce range(6) as $i ([]; . + [($o|contains({"zz":null}))])"#], None, "[false,false,false,false,false,false]\n", "", 0),
    // lit_n/reduce/c_obj_inner_mismatch
    (&[r"-c", r"-n", r#"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | reduce range(6) as $i ([]; . + [($o|contains({"k":"x"}))])"#], None, "[false,false,false,false,false,false]\n", "", 0),
    // lit_n/reduce/c_s
    (&[r"-c", r"-n", r#"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | reduce range(6) as $i ([]; . + [($s|contains("\($i)"))])"#], None, "[true,true,true,true,true,true]\n", "", 0),
    // lit_n/reduce/c_s_empty
    (&[r"-c", r"-n", r#"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | reduce range(6) as $i ([]; . + [($s|contains(""))])"#], None, "[true,true,true,true,true,true]\n", "", 0),
    // lit_n/reduce/c_s_err
    (&[r"-c", r"-n", r#"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | reduce range(6) as $i ([]; . + [($s|try contains(["1"]) catch .)])"#], None, "[\"string (\\\"[0,1,2,3,4...) and array ([\\\"1\\\"]) cannot have their containment checked\",\"string (\\\"[0,1,2,3,4...) and array ([\\\"1\\\"]) cannot have their containment checked\",\"string (\\\"[0,1,2,3,4...) and array ([\\\"1\\\"]) cannot have their containment checked\",\"string (\\\"[0,1,2,3,4...) and array ([\\\"1\\\"]) cannot have their containment checked\",\"string (\\\"[0,1,2,3,4...) and array ([\\\"1\\\"]) cannot have their containment checked\",\"string (\\\"[0,1,2,3,4...) and array ([\\\"1\\\"]) cannot have their containment checked\"]\n", "", 0),
    // lit_n/reduce/c_not
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | reduce range(6) as $i ([]; . + [($a|contains([$i])|not)])"], None, "[false,false,false,false,false,false]\n", "", 0),
    // lit_n/reduce/c_if
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | reduce range(6) as $i ([]; . + [(if $a|contains([$i]) then 1 else 0 end)])"], None, "[1,1,1,1,1,1]\n", "", 0),
    // lit_n/reduce/c_slice_needle
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | reduce range(6) as $i ([]; . + [($a|contains([$i]|.[0:1]))])"], None, "[true,true,true,true,true,true]\n", "", 0),
    // lit_n/reduce/c_var_needle
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | reduce range(6) as $i ([]; . + [([$i] as $p | $a|contains($p))])"], None, "[true,true,true,true,true,true]\n", "", 0),
    // lit_n/reduce/c_fan
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | reduce range(6) as $i ([]; . + [($a|contains([$i], [$i+100]))])"], None, "[true,false,true,false,true,false,true,false,true,false,true,false]\n", "", 0),
    // lit_n/reduce/c_null_hay
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | reduce range(6) as $i ([]; . + [($n|contains(null))])"], None, "[true,true,true,true,true,true]\n", "", 0),
    // lit_n/reduce/c_num_hay
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | reduce range(6) as $i ([]; . + [($x|contains(1.0))])"], None, "[true,true,true,true,true,true]\n", "", 0),
    // lit_n/reduce/c_bool_err
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | reduce range(6) as $i ([]; . + [($t|try contains(false) catch .)])"], None, "[\"boolean (true) and boolean (false) cannot have their containment checked\",\"boolean (true) and boolean (false) cannot have their containment checked\",\"boolean (true) and boolean (false) cannot have their containment checked\",\"boolean (true) and boolean (false) cannot have their containment checked\",\"boolean (true) and boolean (false) cannot have their containment checked\",\"boolean (true) and boolean (false) cannot have their containment checked\"]\n", "", 0),
    // lit_n/reduce/c_bignum
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | reduce range(6) as $i ([]; . + [($a|contains([100000000000000000000]))])"], None, "[false,false,false,false,false,false]\n", "", 0),
    // lit_n/foreach/c_one
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [foreach range(6) as $i (null; ($a|contains([$i])))]"], None, "[true,true,true,true,true,true]\n", "", 0),
    // lit_n/foreach/c_dup
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [foreach range(6) as $i (null; ($a|contains([$i, $i])))]"], None, "[true,true,true,true,true,true]\n", "", 0),
    // lit_n/foreach/c_empty
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [foreach range(6) as $i (null; ($a|contains([])))]"], None, "[true,true,true,true,true,true]\n", "", 0),
    // lit_n/foreach/c_long
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [foreach range(6) as $i (null; ($a|contains([range(20)])))]"], None, "[false,false,false,false,false,false]\n", "", 0),
    // lit_n/foreach/c_float
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [foreach range(6) as $i (null; ($a|contains([1.0])))]"], None, "[true,true,true,true,true,true]\n", "", 0),
    // lit_n/foreach/c_float3
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [foreach range(6) as $i (null; ($a|contains([1.000, 3.5])))]"], None, "[false,false,false,false,false,false]\n", "", 0),
    // lit_n/foreach/c_null
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [foreach range(6) as $i (null; ($a|contains([null])))]"], None, "[false,false,false,false,false,false]\n", "", 0),
    // lit_n/foreach/c_str_err
    (&[r"-c", r"-n", r#"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [foreach range(6) as $i (null; ($a|try contains("x") catch .))]"#], None, "[\"array ([0,1,2,3,4,...) and string (\\\"x\\\") cannot have their containment checked\",\"array ([0,1,2,3,4,...) and string (\\\"x\\\") cannot have their containment checked\",\"array ([0,1,2,3,4,...) and string (\\\"x\\\") cannot have their containment checked\",\"array ([0,1,2,3,4,...) and string (\\\"x\\\") cannot have their containment checked\",\"array ([0,1,2,3,4,...) and string (\\\"x\\\") cannot have their containment checked\",\"array ([0,1,2,3,4,...) and string (\\\"x\\\") cannot have their containment checked\"]\n", "", 0),
    // lit_n/foreach/c_obj_err
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [foreach range(6) as $i (null; ($o|try contains([1]) catch .))]"], None, "[\"object ({\\\"k\\\":[0,1,2...) and array ([1]) cannot have their containment checked\",\"object ({\\\"k\\\":[0,1,2...) and array ([1]) cannot have their containment checked\",\"object ({\\\"k\\\":[0,1,2...) and array ([1]) cannot have their containment checked\",\"object ({\\\"k\\\":[0,1,2...) and array ([1]) cannot have their containment checked\",\"object ({\\\"k\\\":[0,1,2...) and array ([1]) cannot have their containment checked\",\"object ({\\\"k\\\":[0,1,2...) and array ([1]) cannot have their containment checked\"]\n", "", 0),
    // lit_n/foreach/c_num_err
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [foreach range(6) as $i (null; ($a|try contains(1) catch .))]"], None, "[\"array ([0,1,2,3,4,...) and number (1) cannot have their containment checked\",\"array ([0,1,2,3,4,...) and number (1) cannot have their containment checked\",\"array ([0,1,2,3,4,...) and number (1) cannot have their containment checked\",\"array ([0,1,2,3,4,...) and number (1) cannot have their containment checked\",\"array ([0,1,2,3,4,...) and number (1) cannot have their containment checked\",\"array ([0,1,2,3,4,...) and number (1) cannot have their containment checked\"]\n", "", 0),
    // lit_n/foreach/c_nested
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [foreach range(6) as $i (null; ($a|contains([[1]])))]"], None, "[false,false,false,false,false,false]\n", "", 0),
    // lit_n/foreach/c_nested2
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [foreach range(6) as $i (null; ($a|contains([[2,1]])))]"], None, "[false,false,false,false,false,false]\n", "", 0),
    // lit_n/foreach/c_objelem
    (&[r"-c", r"-n", r#"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [foreach range(6) as $i (null; ($a|contains([{"x":1}])))]"#], None, "[false,false,false,false,false,false]\n", "", 0),
    // lit_n/foreach/c_emptyobj
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [foreach range(6) as $i (null; ($a|contains([{}])))]"], None, "[false,false,false,false,false,false]\n", "", 0),
    // lit_n/foreach/c_substr
    (&[r"-c", r"-n", r#"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [foreach range(6) as $i (null; ($a|contains(["s"])))]"#], None, "[false,false,false,false,false,false]\n", "", 0),
    // lit_n/foreach/c_interp
    (&[r"-c", r"-n", r#"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [foreach range(6) as $i (null; ($a|contains(["\($i)"])))]"#], None, "[false,false,false,false,false,false]\n", "", 0),
    // lit_n/foreach/c_mixed_inner
    (&[r"-c", r"-n", r#"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [foreach range(6) as $i (null; ($a|contains([[1,"a"]])))]"#], None, "[false,false,false,false,false,false]\n", "", 0),
    // lit_n/foreach/c_obj_k
    (&[r"-c", r"-n", r#"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [foreach range(6) as $i (null; ($o|contains({"k":[$i]})))]"#], None, "[true,true,true,true,true,true]\n", "", 0),
    // lit_n/foreach/c_obj_kempty
    (&[r"-c", r"-n", r#"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [foreach range(6) as $i (null; ($o|contains({"k":[]})))]"#], None, "[true,true,true,true,true,true]\n", "", 0),
    // lit_n/foreach/c_obj_miss
    (&[r"-c", r"-n", r#"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [foreach range(6) as $i (null; ($o|contains({"zz":null})))]"#], None, "[false,false,false,false,false,false]\n", "", 0),
    // lit_n/foreach/c_obj_inner_mismatch
    (&[r"-c", r"-n", r#"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [foreach range(6) as $i (null; ($o|contains({"k":"x"})))]"#], None, "[false,false,false,false,false,false]\n", "", 0),
    // lit_n/foreach/c_s
    (&[r"-c", r"-n", r#"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [foreach range(6) as $i (null; ($s|contains("\($i)")))]"#], None, "[true,true,true,true,true,true]\n", "", 0),
    // lit_n/foreach/c_s_empty
    (&[r"-c", r"-n", r#"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [foreach range(6) as $i (null; ($s|contains("")))]"#], None, "[true,true,true,true,true,true]\n", "", 0),
    // lit_n/foreach/c_s_err
    (&[r"-c", r"-n", r#"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [foreach range(6) as $i (null; ($s|try contains(["1"]) catch .))]"#], None, "[\"string (\\\"[0,1,2,3,4...) and array ([\\\"1\\\"]) cannot have their containment checked\",\"string (\\\"[0,1,2,3,4...) and array ([\\\"1\\\"]) cannot have their containment checked\",\"string (\\\"[0,1,2,3,4...) and array ([\\\"1\\\"]) cannot have their containment checked\",\"string (\\\"[0,1,2,3,4...) and array ([\\\"1\\\"]) cannot have their containment checked\",\"string (\\\"[0,1,2,3,4...) and array ([\\\"1\\\"]) cannot have their containment checked\",\"string (\\\"[0,1,2,3,4...) and array ([\\\"1\\\"]) cannot have their containment checked\"]\n", "", 0),
    // lit_n/foreach/c_not
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [foreach range(6) as $i (null; ($a|contains([$i])|not))]"], None, "[false,false,false,false,false,false]\n", "", 0),
    // lit_n/foreach/c_if
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [foreach range(6) as $i (null; (if $a|contains([$i]) then 1 else 0 end))]"], None, "[1,1,1,1,1,1]\n", "", 0),
    // lit_n/foreach/c_slice_needle
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [foreach range(6) as $i (null; ($a|contains([$i]|.[0:1])))]"], None, "[true,true,true,true,true,true]\n", "", 0),
    // lit_n/foreach/c_var_needle
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [foreach range(6) as $i (null; ([$i] as $p | $a|contains($p)))]"], None, "[true,true,true,true,true,true]\n", "", 0),
    // lit_n/foreach/c_fan
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [foreach range(6) as $i (null; ($a|contains([$i], [$i+100])))]"], None, "[true,false,true,false,true,false,true,false,true,false,true,false]\n", "", 0),
    // lit_n/foreach/c_null_hay
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [foreach range(6) as $i (null; ($n|contains(null)))]"], None, "[true,true,true,true,true,true]\n", "", 0),
    // lit_n/foreach/c_num_hay
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [foreach range(6) as $i (null; ($x|contains(1.0)))]"], None, "[true,true,true,true,true,true]\n", "", 0),
    // lit_n/foreach/c_bool_err
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [foreach range(6) as $i (null; ($t|try contains(false) catch .))]"], None, "[\"boolean (true) and boolean (false) cannot have their containment checked\",\"boolean (true) and boolean (false) cannot have their containment checked\",\"boolean (true) and boolean (false) cannot have their containment checked\",\"boolean (true) and boolean (false) cannot have their containment checked\",\"boolean (true) and boolean (false) cannot have their containment checked\",\"boolean (true) and boolean (false) cannot have their containment checked\"]\n", "", 0),
    // lit_n/foreach/c_bignum
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [foreach range(6) as $i (null; ($a|contains([100000000000000000000])))]"], None, "[false,false,false,false,false,false]\n", "", 0),
    // lit_n/until/c_one
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [0, []] | until(.[0] >= 6; .[0] as $i | [$i + 1, .[1] + [($a|contains([$i]))]]) | .[1]"], None, "[true,true,true,true,true,true]\n", "", 0),
    // lit_n/until/c_dup
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [0, []] | until(.[0] >= 6; .[0] as $i | [$i + 1, .[1] + [($a|contains([$i, $i]))]]) | .[1]"], None, "[true,true,true,true,true,true]\n", "", 0),
    // lit_n/until/c_empty
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [0, []] | until(.[0] >= 6; .[0] as $i | [$i + 1, .[1] + [($a|contains([]))]]) | .[1]"], None, "[true,true,true,true,true,true]\n", "", 0),
    // lit_n/until/c_long
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [0, []] | until(.[0] >= 6; .[0] as $i | [$i + 1, .[1] + [($a|contains([range(20)]))]]) | .[1]"], None, "[false,false,false,false,false,false]\n", "", 0),
    // lit_n/until/c_float
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [0, []] | until(.[0] >= 6; .[0] as $i | [$i + 1, .[1] + [($a|contains([1.0]))]]) | .[1]"], None, "[true,true,true,true,true,true]\n", "", 0),
    // lit_n/until/c_float3
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [0, []] | until(.[0] >= 6; .[0] as $i | [$i + 1, .[1] + [($a|contains([1.000, 3.5]))]]) | .[1]"], None, "[false,false,false,false,false,false]\n", "", 0),
    // lit_n/until/c_null
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [0, []] | until(.[0] >= 6; .[0] as $i | [$i + 1, .[1] + [($a|contains([null]))]]) | .[1]"], None, "[false,false,false,false,false,false]\n", "", 0),
    // lit_n/until/c_str_err
    (&[r"-c", r"-n", r#"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [0, []] | until(.[0] >= 6; .[0] as $i | [$i + 1, .[1] + [($a|try contains("x") catch .)]]) | .[1]"#], None, "[\"array ([0,1,2,3,4,...) and string (\\\"x\\\") cannot have their containment checked\",\"array ([0,1,2,3,4,...) and string (\\\"x\\\") cannot have their containment checked\",\"array ([0,1,2,3,4,...) and string (\\\"x\\\") cannot have their containment checked\",\"array ([0,1,2,3,4,...) and string (\\\"x\\\") cannot have their containment checked\",\"array ([0,1,2,3,4,...) and string (\\\"x\\\") cannot have their containment checked\",\"array ([0,1,2,3,4,...) and string (\\\"x\\\") cannot have their containment checked\"]\n", "", 0),
    // lit_n/until/c_obj_err
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [0, []] | until(.[0] >= 6; .[0] as $i | [$i + 1, .[1] + [($o|try contains([1]) catch .)]]) | .[1]"], None, "[\"object ({\\\"k\\\":[0,1,2...) and array ([1]) cannot have their containment checked\",\"object ({\\\"k\\\":[0,1,2...) and array ([1]) cannot have their containment checked\",\"object ({\\\"k\\\":[0,1,2...) and array ([1]) cannot have their containment checked\",\"object ({\\\"k\\\":[0,1,2...) and array ([1]) cannot have their containment checked\",\"object ({\\\"k\\\":[0,1,2...) and array ([1]) cannot have their containment checked\",\"object ({\\\"k\\\":[0,1,2...) and array ([1]) cannot have their containment checked\"]\n", "", 0),
    // lit_n/until/c_num_err
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [0, []] | until(.[0] >= 6; .[0] as $i | [$i + 1, .[1] + [($a|try contains(1) catch .)]]) | .[1]"], None, "[\"array ([0,1,2,3,4,...) and number (1) cannot have their containment checked\",\"array ([0,1,2,3,4,...) and number (1) cannot have their containment checked\",\"array ([0,1,2,3,4,...) and number (1) cannot have their containment checked\",\"array ([0,1,2,3,4,...) and number (1) cannot have their containment checked\",\"array ([0,1,2,3,4,...) and number (1) cannot have their containment checked\",\"array ([0,1,2,3,4,...) and number (1) cannot have their containment checked\"]\n", "", 0),
    // lit_n/until/c_nested
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [0, []] | until(.[0] >= 6; .[0] as $i | [$i + 1, .[1] + [($a|contains([[1]]))]]) | .[1]"], None, "[false,false,false,false,false,false]\n", "", 0),
    // lit_n/until/c_nested2
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [0, []] | until(.[0] >= 6; .[0] as $i | [$i + 1, .[1] + [($a|contains([[2,1]]))]]) | .[1]"], None, "[false,false,false,false,false,false]\n", "", 0),
    // lit_n/until/c_objelem
    (&[r"-c", r"-n", r#"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [0, []] | until(.[0] >= 6; .[0] as $i | [$i + 1, .[1] + [($a|contains([{"x":1}]))]]) | .[1]"#], None, "[false,false,false,false,false,false]\n", "", 0),
    // lit_n/until/c_emptyobj
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [0, []] | until(.[0] >= 6; .[0] as $i | [$i + 1, .[1] + [($a|contains([{}]))]]) | .[1]"], None, "[false,false,false,false,false,false]\n", "", 0),
    // lit_n/until/c_substr
    (&[r"-c", r"-n", r#"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [0, []] | until(.[0] >= 6; .[0] as $i | [$i + 1, .[1] + [($a|contains(["s"]))]]) | .[1]"#], None, "[false,false,false,false,false,false]\n", "", 0),
    // lit_n/until/c_interp
    (&[r"-c", r"-n", r#"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [0, []] | until(.[0] >= 6; .[0] as $i | [$i + 1, .[1] + [($a|contains(["\($i)"]))]]) | .[1]"#], None, "[false,false,false,false,false,false]\n", "", 0),
    // lit_n/until/c_mixed_inner
    (&[r"-c", r"-n", r#"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [0, []] | until(.[0] >= 6; .[0] as $i | [$i + 1, .[1] + [($a|contains([[1,"a"]]))]]) | .[1]"#], None, "[false,false,false,false,false,false]\n", "", 0),
    // lit_n/until/c_obj_k
    (&[r"-c", r"-n", r#"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [0, []] | until(.[0] >= 6; .[0] as $i | [$i + 1, .[1] + [($o|contains({"k":[$i]}))]]) | .[1]"#], None, "[true,true,true,true,true,true]\n", "", 0),
    // lit_n/until/c_obj_kempty
    (&[r"-c", r"-n", r#"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [0, []] | until(.[0] >= 6; .[0] as $i | [$i + 1, .[1] + [($o|contains({"k":[]}))]]) | .[1]"#], None, "[true,true,true,true,true,true]\n", "", 0),
    // lit_n/until/c_obj_miss
    (&[r"-c", r"-n", r#"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [0, []] | until(.[0] >= 6; .[0] as $i | [$i + 1, .[1] + [($o|contains({"zz":null}))]]) | .[1]"#], None, "[false,false,false,false,false,false]\n", "", 0),
    // lit_n/until/c_obj_inner_mismatch
    (&[r"-c", r"-n", r#"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [0, []] | until(.[0] >= 6; .[0] as $i | [$i + 1, .[1] + [($o|contains({"k":"x"}))]]) | .[1]"#], None, "[false,false,false,false,false,false]\n", "", 0),
    // lit_n/until/c_s
    (&[r"-c", r"-n", r#"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [0, []] | until(.[0] >= 6; .[0] as $i | [$i + 1, .[1] + [($s|contains("\($i)"))]]) | .[1]"#], None, "[true,true,true,true,true,true]\n", "", 0),
    // lit_n/until/c_s_empty
    (&[r"-c", r"-n", r#"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [0, []] | until(.[0] >= 6; .[0] as $i | [$i + 1, .[1] + [($s|contains(""))]]) | .[1]"#], None, "[true,true,true,true,true,true]\n", "", 0),
    // lit_n/until/c_s_err
    (&[r"-c", r"-n", r#"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [0, []] | until(.[0] >= 6; .[0] as $i | [$i + 1, .[1] + [($s|try contains(["1"]) catch .)]]) | .[1]"#], None, "[\"string (\\\"[0,1,2,3,4...) and array ([\\\"1\\\"]) cannot have their containment checked\",\"string (\\\"[0,1,2,3,4...) and array ([\\\"1\\\"]) cannot have their containment checked\",\"string (\\\"[0,1,2,3,4...) and array ([\\\"1\\\"]) cannot have their containment checked\",\"string (\\\"[0,1,2,3,4...) and array ([\\\"1\\\"]) cannot have their containment checked\",\"string (\\\"[0,1,2,3,4...) and array ([\\\"1\\\"]) cannot have their containment checked\",\"string (\\\"[0,1,2,3,4...) and array ([\\\"1\\\"]) cannot have their containment checked\"]\n", "", 0),
    // lit_n/until/c_not
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [0, []] | until(.[0] >= 6; .[0] as $i | [$i + 1, .[1] + [($a|contains([$i])|not)]]) | .[1]"], None, "[false,false,false,false,false,false]\n", "", 0),
    // lit_n/until/c_if
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [0, []] | until(.[0] >= 6; .[0] as $i | [$i + 1, .[1] + [(if $a|contains([$i]) then 1 else 0 end)]]) | .[1]"], None, "[1,1,1,1,1,1]\n", "", 0),
    // lit_n/until/c_slice_needle
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [0, []] | until(.[0] >= 6; .[0] as $i | [$i + 1, .[1] + [($a|contains([$i]|.[0:1]))]]) | .[1]"], None, "[true,true,true,true,true,true]\n", "", 0),
    // lit_n/until/c_var_needle
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [0, []] | until(.[0] >= 6; .[0] as $i | [$i + 1, .[1] + [([$i] as $p | $a|contains($p))]]) | .[1]"], None, "[true,true,true,true,true,true]\n", "", 0),
    // lit_n/until/c_fan
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [0, []] | until(.[0] >= 6; .[0] as $i | [$i + 1, .[1] + [($a|contains([$i], [$i+100]))]]) | .[1]"], None, "[true,false,true,false,true,false,true,false,true,false,true,false]\n", "", 0),
    // lit_n/until/c_null_hay
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [0, []] | until(.[0] >= 6; .[0] as $i | [$i + 1, .[1] + [($n|contains(null))]]) | .[1]"], None, "[true,true,true,true,true,true]\n", "", 0),
    // lit_n/until/c_num_hay
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [0, []] | until(.[0] >= 6; .[0] as $i | [$i + 1, .[1] + [($x|contains(1.0))]]) | .[1]"], None, "[true,true,true,true,true,true]\n", "", 0),
    // lit_n/until/c_bool_err
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [0, []] | until(.[0] >= 6; .[0] as $i | [$i + 1, .[1] + [($t|try contains(false) catch .)]]) | .[1]"], None, "[\"boolean (true) and boolean (false) cannot have their containment checked\",\"boolean (true) and boolean (false) cannot have their containment checked\",\"boolean (true) and boolean (false) cannot have their containment checked\",\"boolean (true) and boolean (false) cannot have their containment checked\",\"boolean (true) and boolean (false) cannot have their containment checked\",\"boolean (true) and boolean (false) cannot have their containment checked\"]\n", "", 0),
    // lit_n/until/c_bignum
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [0, []] | until(.[0] >= 6; .[0] as $i | [$i + 1, .[1] + [($a|contains([100000000000000000000]))]]) | .[1]"], None, "[false,false,false,false,false,false]\n", "", 0),
    // lit_n/first_foreach/c_one
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | first(foreach range(6) as $i (null; ($a|contains([$i]))))"], None, "true\n", "", 0),
    // lit_n/first_foreach/c_dup
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | first(foreach range(6) as $i (null; ($a|contains([$i, $i]))))"], None, "true\n", "", 0),
    // lit_n/first_foreach/c_empty
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | first(foreach range(6) as $i (null; ($a|contains([]))))"], None, "true\n", "", 0),
    // lit_n/first_foreach/c_long
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | first(foreach range(6) as $i (null; ($a|contains([range(20)]))))"], None, "false\n", "", 0),
    // lit_n/first_foreach/c_float
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | first(foreach range(6) as $i (null; ($a|contains([1.0]))))"], None, "true\n", "", 0),
    // lit_n/first_foreach/c_float3
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | first(foreach range(6) as $i (null; ($a|contains([1.000, 3.5]))))"], None, "false\n", "", 0),
    // lit_n/first_foreach/c_null
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | first(foreach range(6) as $i (null; ($a|contains([null]))))"], None, "false\n", "", 0),
    // lit_n/first_foreach/c_str_err
    (&[r"-c", r"-n", r#"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | first(foreach range(6) as $i (null; ($a|try contains("x") catch .)))"#], None, "\"array ([0,1,2,3,4,...) and string (\\\"x\\\") cannot have their containment checked\"\n", "", 0),
    // lit_n/first_foreach/c_obj_err
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | first(foreach range(6) as $i (null; ($o|try contains([1]) catch .)))"], None, "\"object ({\\\"k\\\":[0,1,2...) and array ([1]) cannot have their containment checked\"\n", "", 0),
    // lit_n/first_foreach/c_num_err
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | first(foreach range(6) as $i (null; ($a|try contains(1) catch .)))"], None, "\"array ([0,1,2,3,4,...) and number (1) cannot have their containment checked\"\n", "", 0),
    // lit_n/first_foreach/c_nested
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | first(foreach range(6) as $i (null; ($a|contains([[1]]))))"], None, "false\n", "", 0),
    // lit_n/first_foreach/c_nested2
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | first(foreach range(6) as $i (null; ($a|contains([[2,1]]))))"], None, "false\n", "", 0),
    // lit_n/first_foreach/c_objelem
    (&[r"-c", r"-n", r#"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | first(foreach range(6) as $i (null; ($a|contains([{"x":1}]))))"#], None, "false\n", "", 0),
    // lit_n/first_foreach/c_emptyobj
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | first(foreach range(6) as $i (null; ($a|contains([{}]))))"], None, "false\n", "", 0),
    // lit_n/first_foreach/c_substr
    (&[r"-c", r"-n", r#"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | first(foreach range(6) as $i (null; ($a|contains(["s"]))))"#], None, "false\n", "", 0),
    // lit_n/first_foreach/c_interp
    (&[r"-c", r"-n", r#"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | first(foreach range(6) as $i (null; ($a|contains(["\($i)"]))))"#], None, "false\n", "", 0),
    // lit_n/first_foreach/c_mixed_inner
    (&[r"-c", r"-n", r#"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | first(foreach range(6) as $i (null; ($a|contains([[1,"a"]]))))"#], None, "false\n", "", 0),
    // lit_n/first_foreach/c_obj_k
    (&[r"-c", r"-n", r#"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | first(foreach range(6) as $i (null; ($o|contains({"k":[$i]}))))"#], None, "true\n", "", 0),
    // lit_n/first_foreach/c_obj_kempty
    (&[r"-c", r"-n", r#"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | first(foreach range(6) as $i (null; ($o|contains({"k":[]}))))"#], None, "true\n", "", 0),
    // lit_n/first_foreach/c_obj_miss
    (&[r"-c", r"-n", r#"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | first(foreach range(6) as $i (null; ($o|contains({"zz":null}))))"#], None, "false\n", "", 0),
    // lit_n/first_foreach/c_obj_inner_mismatch
    (&[r"-c", r"-n", r#"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | first(foreach range(6) as $i (null; ($o|contains({"k":"x"}))))"#], None, "false\n", "", 0),
    // lit_n/first_foreach/c_s
    (&[r"-c", r"-n", r#"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | first(foreach range(6) as $i (null; ($s|contains("\($i)"))))"#], None, "true\n", "", 0),
    // lit_n/first_foreach/c_s_empty
    (&[r"-c", r"-n", r#"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | first(foreach range(6) as $i (null; ($s|contains(""))))"#], None, "true\n", "", 0),
    // lit_n/first_foreach/c_s_err
    (&[r"-c", r"-n", r#"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | first(foreach range(6) as $i (null; ($s|try contains(["1"]) catch .)))"#], None, "\"string (\\\"[0,1,2,3,4...) and array ([\\\"1\\\"]) cannot have their containment checked\"\n", "", 0),
    // lit_n/first_foreach/c_not
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | first(foreach range(6) as $i (null; ($a|contains([$i])|not)))"], None, "false\n", "", 0),
    // lit_n/first_foreach/c_if
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | first(foreach range(6) as $i (null; (if $a|contains([$i]) then 1 else 0 end)))"], None, "1\n", "", 0),
    // lit_n/first_foreach/c_slice_needle
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | first(foreach range(6) as $i (null; ($a|contains([$i]|.[0:1]))))"], None, "true\n", "", 0),
    // lit_n/first_foreach/c_var_needle
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | first(foreach range(6) as $i (null; ([$i] as $p | $a|contains($p))))"], None, "true\n", "", 0),
    // lit_n/first_foreach/c_fan
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | first(foreach range(6) as $i (null; ($a|contains([$i], [$i+100]))))"], None, "true\n", "", 0),
    // lit_n/first_foreach/c_null_hay
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | first(foreach range(6) as $i (null; ($n|contains(null))))"], None, "true\n", "", 0),
    // lit_n/first_foreach/c_num_hay
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | first(foreach range(6) as $i (null; ($x|contains(1.0))))"], None, "true\n", "", 0),
    // lit_n/first_foreach/c_bool_err
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | first(foreach range(6) as $i (null; ($t|try contains(false) catch .)))"], None, "\"boolean (true) and boolean (false) cannot have their containment checked\"\n", "", 0),
    // lit_n/first_foreach/c_bignum
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | first(foreach range(6) as $i (null; ($a|contains([100000000000000000000]))))"], None, "false\n", "", 0),
    // lit_n/limit/c_one
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [limit(3; range(6) as $i | ($a|contains([$i])))]"], None, "[true,true,true]\n", "", 0),
    // lit_n/limit/c_dup
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [limit(3; range(6) as $i | ($a|contains([$i, $i])))]"], None, "[true,true,true]\n", "", 0),
    // lit_n/limit/c_empty
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [limit(3; range(6) as $i | ($a|contains([])))]"], None, "[true,true,true]\n", "", 0),
    // lit_n/limit/c_long
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [limit(3; range(6) as $i | ($a|contains([range(20)])))]"], None, "[false,false,false]\n", "", 0),
    // lit_n/limit/c_float
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [limit(3; range(6) as $i | ($a|contains([1.0])))]"], None, "[true,true,true]\n", "", 0),
    // lit_n/limit/c_float3
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [limit(3; range(6) as $i | ($a|contains([1.000, 3.5])))]"], None, "[false,false,false]\n", "", 0),
    // lit_n/limit/c_null
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [limit(3; range(6) as $i | ($a|contains([null])))]"], None, "[false,false,false]\n", "", 0),
    // lit_n/limit/c_str_err
    (&[r"-c", r"-n", r#"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [limit(3; range(6) as $i | ($a|try contains("x") catch .))]"#], None, "[\"array ([0,1,2,3,4,...) and string (\\\"x\\\") cannot have their containment checked\",\"array ([0,1,2,3,4,...) and string (\\\"x\\\") cannot have their containment checked\",\"array ([0,1,2,3,4,...) and string (\\\"x\\\") cannot have their containment checked\"]\n", "", 0),
    // lit_n/limit/c_obj_err
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [limit(3; range(6) as $i | ($o|try contains([1]) catch .))]"], None, "[\"object ({\\\"k\\\":[0,1,2...) and array ([1]) cannot have their containment checked\",\"object ({\\\"k\\\":[0,1,2...) and array ([1]) cannot have their containment checked\",\"object ({\\\"k\\\":[0,1,2...) and array ([1]) cannot have their containment checked\"]\n", "", 0),
    // lit_n/limit/c_num_err
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [limit(3; range(6) as $i | ($a|try contains(1) catch .))]"], None, "[\"array ([0,1,2,3,4,...) and number (1) cannot have their containment checked\",\"array ([0,1,2,3,4,...) and number (1) cannot have their containment checked\",\"array ([0,1,2,3,4,...) and number (1) cannot have their containment checked\"]\n", "", 0),
    // lit_n/limit/c_nested
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [limit(3; range(6) as $i | ($a|contains([[1]])))]"], None, "[false,false,false]\n", "", 0),
    // lit_n/limit/c_nested2
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [limit(3; range(6) as $i | ($a|contains([[2,1]])))]"], None, "[false,false,false]\n", "", 0),
    // lit_n/limit/c_objelem
    (&[r"-c", r"-n", r#"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [limit(3; range(6) as $i | ($a|contains([{"x":1}])))]"#], None, "[false,false,false]\n", "", 0),
    // lit_n/limit/c_emptyobj
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [limit(3; range(6) as $i | ($a|contains([{}])))]"], None, "[false,false,false]\n", "", 0),
    // lit_n/limit/c_substr
    (&[r"-c", r"-n", r#"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [limit(3; range(6) as $i | ($a|contains(["s"])))]"#], None, "[false,false,false]\n", "", 0),
    // lit_n/limit/c_interp
    (&[r"-c", r"-n", r#"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [limit(3; range(6) as $i | ($a|contains(["\($i)"])))]"#], None, "[false,false,false]\n", "", 0),
    // lit_n/limit/c_mixed_inner
    (&[r"-c", r"-n", r#"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [limit(3; range(6) as $i | ($a|contains([[1,"a"]])))]"#], None, "[false,false,false]\n", "", 0),
    // lit_n/limit/c_obj_k
    (&[r"-c", r"-n", r#"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [limit(3; range(6) as $i | ($o|contains({"k":[$i]})))]"#], None, "[true,true,true]\n", "", 0),
    // lit_n/limit/c_obj_kempty
    (&[r"-c", r"-n", r#"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [limit(3; range(6) as $i | ($o|contains({"k":[]})))]"#], None, "[true,true,true]\n", "", 0),
    // lit_n/limit/c_obj_miss
    (&[r"-c", r"-n", r#"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [limit(3; range(6) as $i | ($o|contains({"zz":null})))]"#], None, "[false,false,false]\n", "", 0),
    // lit_n/limit/c_obj_inner_mismatch
    (&[r"-c", r"-n", r#"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [limit(3; range(6) as $i | ($o|contains({"k":"x"})))]"#], None, "[false,false,false]\n", "", 0),
    // lit_n/limit/c_s
    (&[r"-c", r"-n", r#"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [limit(3; range(6) as $i | ($s|contains("\($i)")))]"#], None, "[true,true,true]\n", "", 0),
    // lit_n/limit/c_s_empty
    (&[r"-c", r"-n", r#"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [limit(3; range(6) as $i | ($s|contains("")))]"#], None, "[true,true,true]\n", "", 0),
    // lit_n/limit/c_s_err
    (&[r"-c", r"-n", r#"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [limit(3; range(6) as $i | ($s|try contains(["1"]) catch .))]"#], None, "[\"string (\\\"[0,1,2,3,4...) and array ([\\\"1\\\"]) cannot have their containment checked\",\"string (\\\"[0,1,2,3,4...) and array ([\\\"1\\\"]) cannot have their containment checked\",\"string (\\\"[0,1,2,3,4...) and array ([\\\"1\\\"]) cannot have their containment checked\"]\n", "", 0),
    // lit_n/limit/c_not
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [limit(3; range(6) as $i | ($a|contains([$i])|not))]"], None, "[false,false,false]\n", "", 0),
    // lit_n/limit/c_if
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [limit(3; range(6) as $i | (if $a|contains([$i]) then 1 else 0 end))]"], None, "[1,1,1]\n", "", 0),
    // lit_n/limit/c_slice_needle
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [limit(3; range(6) as $i | ($a|contains([$i]|.[0:1])))]"], None, "[true,true,true]\n", "", 0),
    // lit_n/limit/c_var_needle
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [limit(3; range(6) as $i | ([$i] as $p | $a|contains($p)))]"], None, "[true,true,true]\n", "", 0),
    // lit_n/limit/c_fan
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [limit(3; range(6) as $i | ($a|contains([$i], [$i+100])))]"], None, "[true,false,true]\n", "", 0),
    // lit_n/limit/c_null_hay
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [limit(3; range(6) as $i | ($n|contains(null)))]"], None, "[true,true,true]\n", "", 0),
    // lit_n/limit/c_num_hay
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [limit(3; range(6) as $i | ($x|contains(1.0)))]"], None, "[true,true,true]\n", "", 0),
    // lit_n/limit/c_bool_err
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [limit(3; range(6) as $i | ($t|try contains(false) catch .))]"], None, "[\"boolean (true) and boolean (false) cannot have their containment checked\",\"boolean (true) and boolean (false) cannot have their containment checked\",\"boolean (true) and boolean (false) cannot have their containment checked\"]\n", "", 0),
    // lit_n/limit/c_bignum
    (&[r"-c", r"-n", r"[range(8)] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [limit(3; range(6) as $i | ($a|contains([100000000000000000000])))]"], None, "[false,false,false]\n", "", 0),
    // computed_n/reduce/c_one
    (&[r"-c", r"-n", r"[range(8) | . * 3 | tostring] as $a | {k: [range(8)|-.]} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | reduce range(6) as $i ([]; . + [($a|contains([$i]))])"], None, "[false,false,false,false,false,false]\n", "", 0),
    // computed_n/reduce/c_dup
    (&[r"-c", r"-n", r"[range(8) | . * 3 | tostring] as $a | {k: [range(8)|-.]} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | reduce range(6) as $i ([]; . + [($a|contains([$i, $i]))])"], None, "[false,false,false,false,false,false]\n", "", 0),
    // computed_n/reduce/c_long
    (&[r"-c", r"-n", r"[range(8) | . * 3 | tostring] as $a | {k: [range(8)|-.]} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | reduce range(6) as $i ([]; . + [($a|contains([range(20)]))])"], None, "[false,false,false,false,false,false]\n", "", 0),
    // computed_n/reduce/c_float3
    (&[r"-c", r"-n", r"[range(8) | . * 3 | tostring] as $a | {k: [range(8)|-.]} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | reduce range(6) as $i ([]; . + [($a|contains([1.000, 3.5]))])"], None, "[false,false,false,false,false,false]\n", "", 0),
    // computed_n/reduce/c_str_err
    (&[r"-c", r"-n", r#"[range(8) | . * 3 | tostring] as $a | {k: [range(8)|-.]} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | reduce range(6) as $i ([]; . + [($a|try contains("x") catch .)])"#], None, "[\"array ([\\\"0\\\",\\\"3\\\",\\\"6...) and string (\\\"x\\\") cannot have their containment checked\",\"array ([\\\"0\\\",\\\"3\\\",\\\"6...) and string (\\\"x\\\") cannot have their containment checked\",\"array ([\\\"0\\\",\\\"3\\\",\\\"6...) and string (\\\"x\\\") cannot have their containment checked\",\"array ([\\\"0\\\",\\\"3\\\",\\\"6...) and string (\\\"x\\\") cannot have their containment checked\",\"array ([\\\"0\\\",\\\"3\\\",\\\"6...) and string (\\\"x\\\") cannot have their containment checked\",\"array ([\\\"0\\\",\\\"3\\\",\\\"6...) and string (\\\"x\\\") cannot have their containment checked\"]\n", "", 0),
    // computed_n/reduce/c_obj_k
    (&[r"-c", r"-n", r#"[range(8) | . * 3 | tostring] as $a | {k: [range(8)|-.]} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | reduce range(6) as $i ([]; . + [($o|contains({"k":[$i]}))])"#], None, "[true,false,false,false,false,false]\n", "", 0),
    // computed_n/reduce/c_s
    (&[r"-c", r"-n", r#"[range(8) | . * 3 | tostring] as $a | {k: [range(8)|-.]} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | reduce range(6) as $i ([]; . + [($s|contains("\($i)"))])"#], None, "[true,true,true,true,false,true]\n", "", 0),
    // computed_n/reduce/c_bignum
    (&[r"-c", r"-n", r"[range(8) | . * 3 | tostring] as $a | {k: [range(8)|-.]} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | reduce range(6) as $i ([]; . + [($a|contains([100000000000000000000]))])"], None, "[false,false,false,false,false,false]\n", "", 0),
    // computed_n/foreach_acc/c_one
    (&[r"-c", r"-n", r"[range(8) | . * 3 | tostring] as $a | {k: [range(8)|-.]} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [foreach range(6) as $i ([]; . + [($a|contains([$i]))]; length)]"], None, "[1,2,3,4,5,6]\n", "", 0),
    // computed_n/foreach_acc/c_dup
    (&[r"-c", r"-n", r"[range(8) | . * 3 | tostring] as $a | {k: [range(8)|-.]} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [foreach range(6) as $i ([]; . + [($a|contains([$i, $i]))]; length)]"], None, "[1,2,3,4,5,6]\n", "", 0),
    // computed_n/foreach_acc/c_long
    (&[r"-c", r"-n", r"[range(8) | . * 3 | tostring] as $a | {k: [range(8)|-.]} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [foreach range(6) as $i ([]; . + [($a|contains([range(20)]))]; length)]"], None, "[1,2,3,4,5,6]\n", "", 0),
    // computed_n/foreach_acc/c_float3
    (&[r"-c", r"-n", r"[range(8) | . * 3 | tostring] as $a | {k: [range(8)|-.]} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [foreach range(6) as $i ([]; . + [($a|contains([1.000, 3.5]))]; length)]"], None, "[1,2,3,4,5,6]\n", "", 0),
    // computed_n/foreach_acc/c_str_err
    (&[r"-c", r"-n", r#"[range(8) | . * 3 | tostring] as $a | {k: [range(8)|-.]} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [foreach range(6) as $i ([]; . + [($a|try contains("x") catch .)]; length)]"#], None, "[1,2,3,4,5,6]\n", "", 0),
    // computed_n/foreach_acc/c_obj_k
    (&[r"-c", r"-n", r#"[range(8) | . * 3 | tostring] as $a | {k: [range(8)|-.]} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [foreach range(6) as $i ([]; . + [($o|contains({"k":[$i]}))]; length)]"#], None, "[1,2,3,4,5,6]\n", "", 0),
    // computed_n/foreach_acc/c_s
    (&[r"-c", r"-n", r#"[range(8) | . * 3 | tostring] as $a | {k: [range(8)|-.]} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [foreach range(6) as $i ([]; . + [($s|contains("\($i)"))]; length)]"#], None, "[1,2,3,4,5,6]\n", "", 0),
    // computed_n/foreach_acc/c_bignum
    (&[r"-c", r"-n", r"[range(8) | . * 3 | tostring] as $a | {k: [range(8)|-.]} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [foreach range(6) as $i ([]; . + [($a|contains([100000000000000000000]))]; length)]"], None, "[1,2,3,4,5,6]\n", "", 0),
    // computed_n/limit/c_one
    (&[r"-c", r"-n", r"[range(8) | . * 3 | tostring] as $a | {k: [range(8)|-.]} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [limit(3; range(6) as $i | ($a|contains([$i])))]"], None, "[false,false,false]\n", "", 0),
    // computed_n/limit/c_dup
    (&[r"-c", r"-n", r"[range(8) | . * 3 | tostring] as $a | {k: [range(8)|-.]} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [limit(3; range(6) as $i | ($a|contains([$i, $i])))]"], None, "[false,false,false]\n", "", 0),
    // computed_n/limit/c_long
    (&[r"-c", r"-n", r"[range(8) | . * 3 | tostring] as $a | {k: [range(8)|-.]} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [limit(3; range(6) as $i | ($a|contains([range(20)])))]"], None, "[false,false,false]\n", "", 0),
    // computed_n/limit/c_float3
    (&[r"-c", r"-n", r"[range(8) | . * 3 | tostring] as $a | {k: [range(8)|-.]} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [limit(3; range(6) as $i | ($a|contains([1.000, 3.5])))]"], None, "[false,false,false]\n", "", 0),
    // computed_n/limit/c_str_err
    (&[r"-c", r"-n", r#"[range(8) | . * 3 | tostring] as $a | {k: [range(8)|-.]} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [limit(3; range(6) as $i | ($a|try contains("x") catch .))]"#], None, "[\"array ([\\\"0\\\",\\\"3\\\",\\\"6...) and string (\\\"x\\\") cannot have their containment checked\",\"array ([\\\"0\\\",\\\"3\\\",\\\"6...) and string (\\\"x\\\") cannot have their containment checked\",\"array ([\\\"0\\\",\\\"3\\\",\\\"6...) and string (\\\"x\\\") cannot have their containment checked\"]\n", "", 0),
    // computed_n/limit/c_obj_k
    (&[r"-c", r"-n", r#"[range(8) | . * 3 | tostring] as $a | {k: [range(8)|-.]} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [limit(3; range(6) as $i | ($o|contains({"k":[$i]})))]"#], None, "[true,false,false]\n", "", 0),
    // computed_n/limit/c_s
    (&[r"-c", r"-n", r#"[range(8) | . * 3 | tostring] as $a | {k: [range(8)|-.]} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [limit(3; range(6) as $i | ($s|contains("\($i)")))]"#], None, "[true,true,true]\n", "", 0),
    // computed_n/limit/c_bignum
    (&[r"-c", r"-n", r"[range(8) | . * 3 | tostring] as $a | {k: [range(8)|-.]} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [limit(3; range(6) as $i | ($a|contains([100000000000000000000])))]"], None, "[false,false,false]\n", "", 0),
    // mixed_n/reduce/c_one
    (&[r"-c", r"-n", r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | reduce range(6) as $i ([]; . + [($a|contains([$i]))])"#], None, "[true,true,false,false,false,false]\n", "", 0),
    // mixed_n/reduce/c_dup
    (&[r"-c", r"-n", r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | reduce range(6) as $i ([]; . + [($a|contains([$i, $i]))])"#], None, "[true,true,false,false,false,false]\n", "", 0),
    // mixed_n/reduce/c_long
    (&[r"-c", r"-n", r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | reduce range(6) as $i ([]; . + [($a|contains([range(20)]))])"#], None, "[false,false,false,false,false,false]\n", "", 0),
    // mixed_n/reduce/c_float3
    (&[r"-c", r"-n", r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | reduce range(6) as $i ([]; . + [($a|contains([1.000, 3.5]))])"#], None, "[true,true,true,true,true,true]\n", "", 0),
    // mixed_n/reduce/c_str_err
    (&[r"-c", r"-n", r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | reduce range(6) as $i ([]; . + [($a|try contains("x") catch .)])"#], None, "[\"array ([0,1.000,1E...) and string (\\\"x\\\") cannot have their containment checked\",\"array ([0,1.000,1E...) and string (\\\"x\\\") cannot have their containment checked\",\"array ([0,1.000,1E...) and string (\\\"x\\\") cannot have their containment checked\",\"array ([0,1.000,1E...) and string (\\\"x\\\") cannot have their containment checked\",\"array ([0,1.000,1E...) and string (\\\"x\\\") cannot have their containment checked\",\"array ([0,1.000,1E...) and string (\\\"x\\\") cannot have their containment checked\"]\n", "", 0),
    // mixed_n/reduce/c_obj_k
    (&[r"-c", r"-n", r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | reduce range(6) as $i ([]; . + [($o|contains({"k":[$i]}))])"#], None, "[true,true,false,false,false,false]\n", "", 0),
    // mixed_n/reduce/c_s
    (&[r"-c", r"-n", r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | reduce range(6) as $i ([]; . + [($s|contains("\($i)"))])"#], None, "[true,true,true,true,false,true]\n", "", 0),
    // mixed_n/reduce/c_bignum
    (&[r"-c", r"-n", r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | reduce range(6) as $i ([]; . + [($a|contains([100000000000000000000]))])"#], None, "[false,false,false,false,false,false]\n", "", 0),
    // mixed_n/foreach_acc/c_one
    (&[r"-c", r"-n", r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [foreach range(6) as $i ([]; . + [($a|contains([$i]))]; length)]"#], None, "[1,2,3,4,5,6]\n", "", 0),
    // mixed_n/foreach_acc/c_dup
    (&[r"-c", r"-n", r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [foreach range(6) as $i ([]; . + [($a|contains([$i, $i]))]; length)]"#], None, "[1,2,3,4,5,6]\n", "", 0),
    // mixed_n/foreach_acc/c_long
    (&[r"-c", r"-n", r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [foreach range(6) as $i ([]; . + [($a|contains([range(20)]))]; length)]"#], None, "[1,2,3,4,5,6]\n", "", 0),
    // mixed_n/foreach_acc/c_float3
    (&[r"-c", r"-n", r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [foreach range(6) as $i ([]; . + [($a|contains([1.000, 3.5]))]; length)]"#], None, "[1,2,3,4,5,6]\n", "", 0),
    // mixed_n/foreach_acc/c_str_err
    (&[r"-c", r"-n", r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [foreach range(6) as $i ([]; . + [($a|try contains("x") catch .)]; length)]"#], None, "[1,2,3,4,5,6]\n", "", 0),
    // mixed_n/foreach_acc/c_obj_k
    (&[r"-c", r"-n", r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [foreach range(6) as $i ([]; . + [($o|contains({"k":[$i]}))]; length)]"#], None, "[1,2,3,4,5,6]\n", "", 0),
    // mixed_n/foreach_acc/c_s
    (&[r"-c", r"-n", r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [foreach range(6) as $i ([]; . + [($s|contains("\($i)"))]; length)]"#], None, "[1,2,3,4,5,6]\n", "", 0),
    // mixed_n/foreach_acc/c_bignum
    (&[r"-c", r"-n", r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [foreach range(6) as $i ([]; . + [($a|contains([100000000000000000000]))]; length)]"#], None, "[1,2,3,4,5,6]\n", "", 0),
    // mixed_n/limit/c_one
    (&[r"-c", r"-n", r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [limit(3; range(6) as $i | ($a|contains([$i])))]"#], None, "[true,true,false]\n", "", 0),
    // mixed_n/limit/c_dup
    (&[r"-c", r"-n", r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [limit(3; range(6) as $i | ($a|contains([$i, $i])))]"#], None, "[true,true,false]\n", "", 0),
    // mixed_n/limit/c_long
    (&[r"-c", r"-n", r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [limit(3; range(6) as $i | ($a|contains([range(20)])))]"#], None, "[false,false,false]\n", "", 0),
    // mixed_n/limit/c_float3
    (&[r"-c", r"-n", r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [limit(3; range(6) as $i | ($a|contains([1.000, 3.5])))]"#], None, "[true,true,true]\n", "", 0),
    // mixed_n/limit/c_str_err
    (&[r"-c", r"-n", r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [limit(3; range(6) as $i | ($a|try contains("x") catch .))]"#], None, "[\"array ([0,1.000,1E...) and string (\\\"x\\\") cannot have their containment checked\",\"array ([0,1.000,1E...) and string (\\\"x\\\") cannot have their containment checked\",\"array ([0,1.000,1E...) and string (\\\"x\\\") cannot have their containment checked\"]\n", "", 0),
    // mixed_n/limit/c_obj_k
    (&[r"-c", r"-n", r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [limit(3; range(6) as $i | ($o|contains({"k":[$i]})))]"#], None, "[true,true,false]\n", "", 0),
    // mixed_n/limit/c_s
    (&[r"-c", r"-n", r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [limit(3; range(6) as $i | ($s|contains("\($i)")))]"#], None, "[true,true,true]\n", "", 0),
    // mixed_n/limit/c_bignum
    (&[r"-c", r"-n", r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}] as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [limit(3; range(6) as $i | ($a|contains([100000000000000000000])))]"#], None, "[false,false,false]\n", "", 0),
    // doc/reduce/c_one
    (&[r"-c", r". as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | reduce range(6) as $i ([]; . + [($a|contains([$i]))])"], Some(r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]
"#), "[true,true,false,false,false,false]\n", "", 0),
    // doc/reduce/c_dup
    (&[r"-c", r". as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | reduce range(6) as $i ([]; . + [($a|contains([$i, $i]))])"], Some(r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]
"#), "[true,true,false,false,false,false]\n", "", 0),
    // doc/reduce/c_long
    (&[r"-c", r". as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | reduce range(6) as $i ([]; . + [($a|contains([range(20)]))])"], Some(r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]
"#), "[false,false,false,false,false,false]\n", "", 0),
    // doc/reduce/c_float3
    (&[r"-c", r". as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | reduce range(6) as $i ([]; . + [($a|contains([1.000, 3.5]))])"], Some(r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]
"#), "[true,true,true,true,true,true]\n", "", 0),
    // doc/reduce/c_str_err
    (&[r"-c", r#". as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | reduce range(6) as $i ([]; . + [($a|try contains("x") catch .)])"#], Some(r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]
"#), "[\"array ([0,1.000,1E...) and string (\\\"x\\\") cannot have their containment checked\",\"array ([0,1.000,1E...) and string (\\\"x\\\") cannot have their containment checked\",\"array ([0,1.000,1E...) and string (\\\"x\\\") cannot have their containment checked\",\"array ([0,1.000,1E...) and string (\\\"x\\\") cannot have their containment checked\",\"array ([0,1.000,1E...) and string (\\\"x\\\") cannot have their containment checked\",\"array ([0,1.000,1E...) and string (\\\"x\\\") cannot have their containment checked\"]\n", "", 0),
    // doc/reduce/c_obj_k
    (&[r"-c", r#". as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | reduce range(6) as $i ([]; . + [($o|contains({"k":[$i]}))])"#], Some(r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]
"#), "[true,true,false,false,false,false]\n", "", 0),
    // doc/reduce/c_s
    (&[r"-c", r#". as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | reduce range(6) as $i ([]; . + [($s|contains("\($i)"))])"#], Some(r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]
"#), "[true,true,true,true,false,true]\n", "", 0),
    // doc/reduce/c_bignum
    (&[r"-c", r". as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | reduce range(6) as $i ([]; . + [($a|contains([100000000000000000000]))])"], Some(r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]
"#), "[false,false,false,false,false,false]\n", "", 0),
    // doc/foreach_acc/c_one
    (&[r"-c", r". as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [foreach range(6) as $i ([]; . + [($a|contains([$i]))]; length)]"], Some(r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]
"#), "[1,2,3,4,5,6]\n", "", 0),
    // doc/foreach_acc/c_dup
    (&[r"-c", r". as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [foreach range(6) as $i ([]; . + [($a|contains([$i, $i]))]; length)]"], Some(r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]
"#), "[1,2,3,4,5,6]\n", "", 0),
    // doc/foreach_acc/c_long
    (&[r"-c", r". as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [foreach range(6) as $i ([]; . + [($a|contains([range(20)]))]; length)]"], Some(r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]
"#), "[1,2,3,4,5,6]\n", "", 0),
    // doc/foreach_acc/c_float3
    (&[r"-c", r". as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [foreach range(6) as $i ([]; . + [($a|contains([1.000, 3.5]))]; length)]"], Some(r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]
"#), "[1,2,3,4,5,6]\n", "", 0),
    // doc/foreach_acc/c_str_err
    (&[r"-c", r#". as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [foreach range(6) as $i ([]; . + [($a|try contains("x") catch .)]; length)]"#], Some(r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]
"#), "[1,2,3,4,5,6]\n", "", 0),
    // doc/foreach_acc/c_obj_k
    (&[r"-c", r#". as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [foreach range(6) as $i ([]; . + [($o|contains({"k":[$i]}))]; length)]"#], Some(r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]
"#), "[1,2,3,4,5,6]\n", "", 0),
    // doc/foreach_acc/c_s
    (&[r"-c", r#". as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [foreach range(6) as $i ([]; . + [($s|contains("\($i)"))]; length)]"#], Some(r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]
"#), "[1,2,3,4,5,6]\n", "", 0),
    // doc/foreach_acc/c_bignum
    (&[r"-c", r". as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [foreach range(6) as $i ([]; . + [($a|contains([100000000000000000000]))]; length)]"], Some(r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]
"#), "[1,2,3,4,5,6]\n", "", 0),
    // doc/limit/c_one
    (&[r"-c", r". as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [limit(3; range(6) as $i | ($a|contains([$i])))]"], Some(r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]
"#), "[true,true,false]\n", "", 0),
    // doc/limit/c_dup
    (&[r"-c", r". as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [limit(3; range(6) as $i | ($a|contains([$i, $i])))]"], Some(r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]
"#), "[true,true,false]\n", "", 0),
    // doc/limit/c_long
    (&[r"-c", r". as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [limit(3; range(6) as $i | ($a|contains([range(20)])))]"], Some(r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]
"#), "[false,false,false]\n", "", 0),
    // doc/limit/c_float3
    (&[r"-c", r". as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [limit(3; range(6) as $i | ($a|contains([1.000, 3.5])))]"], Some(r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]
"#), "[true,true,true]\n", "", 0),
    // doc/limit/c_str_err
    (&[r"-c", r#". as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [limit(3; range(6) as $i | ($a|try contains("x") catch .))]"#], Some(r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]
"#), "[\"array ([0,1.000,1E...) and string (\\\"x\\\") cannot have their containment checked\",\"array ([0,1.000,1E...) and string (\\\"x\\\") cannot have their containment checked\",\"array ([0,1.000,1E...) and string (\\\"x\\\") cannot have their containment checked\"]\n", "", 0),
    // doc/limit/c_obj_k
    (&[r"-c", r#". as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [limit(3; range(6) as $i | ($o|contains({"k":[$i]})))]"#], Some(r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]
"#), "[true,true,false]\n", "", 0),
    // doc/limit/c_s
    (&[r"-c", r#". as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [limit(3; range(6) as $i | ($s|contains("\($i)")))]"#], Some(r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]
"#), "[true,true,true]\n", "", 0),
    // doc/limit/c_bignum
    (&[r"-c", r". as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [limit(3; range(6) as $i | ($a|contains([100000000000000000000])))]"], Some(r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]
"#), "[false,false,false]\n", "", 0),
    // doc_obj/reduce/c_one
    (&[r"-c", r".a as $a | . as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | reduce range(6) as $i ([]; . + [($a|contains([$i]))])"], Some(r#"{"a":[0,1.000,"s",null,[1,2],3.5,7,8],"k":[10,1.000,1e1000,{"z":1},null,-0]}
"#), "[true,true,false,false,false,false]\n", "", 0),
    // doc_obj/reduce/c_dup
    (&[r"-c", r".a as $a | . as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | reduce range(6) as $i ([]; . + [($a|contains([$i, $i]))])"], Some(r#"{"a":[0,1.000,"s",null,[1,2],3.5,7,8],"k":[10,1.000,1e1000,{"z":1},null,-0]}
"#), "[true,true,false,false,false,false]\n", "", 0),
    // doc_obj/reduce/c_long
    (&[r"-c", r".a as $a | . as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | reduce range(6) as $i ([]; . + [($a|contains([range(20)]))])"], Some(r#"{"a":[0,1.000,"s",null,[1,2],3.5,7,8],"k":[10,1.000,1e1000,{"z":1},null,-0]}
"#), "[false,false,false,false,false,false]\n", "", 0),
    // doc_obj/reduce/c_float3
    (&[r"-c", r".a as $a | . as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | reduce range(6) as $i ([]; . + [($a|contains([1.000, 3.5]))])"], Some(r#"{"a":[0,1.000,"s",null,[1,2],3.5,7,8],"k":[10,1.000,1e1000,{"z":1},null,-0]}
"#), "[true,true,true,true,true,true]\n", "", 0),
    // doc_obj/reduce/c_str_err
    (&[r"-c", r#".a as $a | . as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | reduce range(6) as $i ([]; . + [($a|try contains("x") catch .)])"#], Some(r#"{"a":[0,1.000,"s",null,[1,2],3.5,7,8],"k":[10,1.000,1e1000,{"z":1},null,-0]}
"#), "[\"array ([0,1.000,\\\"s...) and string (\\\"x\\\") cannot have their containment checked\",\"array ([0,1.000,\\\"s...) and string (\\\"x\\\") cannot have their containment checked\",\"array ([0,1.000,\\\"s...) and string (\\\"x\\\") cannot have their containment checked\",\"array ([0,1.000,\\\"s...) and string (\\\"x\\\") cannot have their containment checked\",\"array ([0,1.000,\\\"s...) and string (\\\"x\\\") cannot have their containment checked\",\"array ([0,1.000,\\\"s...) and string (\\\"x\\\") cannot have their containment checked\"]\n", "", 0),
    // doc_obj/reduce/c_obj_k
    (&[r"-c", r#".a as $a | . as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | reduce range(6) as $i ([]; . + [($o|contains({"k":[$i]}))])"#], Some(r#"{"a":[0,1.000,"s",null,[1,2],3.5,7,8],"k":[10,1.000,1e1000,{"z":1},null,-0]}
"#), "[true,true,false,false,false,false]\n", "", 0),
    // doc_obj/reduce/c_s
    (&[r"-c", r#".a as $a | . as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | reduce range(6) as $i ([]; . + [($s|contains("\($i)"))])"#], Some(r#"{"a":[0,1.000,"s",null,[1,2],3.5,7,8],"k":[10,1.000,1e1000,{"z":1},null,-0]}
"#), "[true,true,true,true,false,true]\n", "", 0),
    // doc_obj/reduce/c_bignum
    (&[r"-c", r".a as $a | . as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | reduce range(6) as $i ([]; . + [($a|contains([100000000000000000000]))])"], Some(r#"{"a":[0,1.000,"s",null,[1,2],3.5,7,8],"k":[10,1.000,1e1000,{"z":1},null,-0]}
"#), "[false,false,false,false,false,false]\n", "", 0),
    // doc_obj/foreach_acc/c_one
    (&[r"-c", r".a as $a | . as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [foreach range(6) as $i ([]; . + [($a|contains([$i]))]; length)]"], Some(r#"{"a":[0,1.000,"s",null,[1,2],3.5,7,8],"k":[10,1.000,1e1000,{"z":1},null,-0]}
"#), "[1,2,3,4,5,6]\n", "", 0),
    // doc_obj/foreach_acc/c_dup
    (&[r"-c", r".a as $a | . as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [foreach range(6) as $i ([]; . + [($a|contains([$i, $i]))]; length)]"], Some(r#"{"a":[0,1.000,"s",null,[1,2],3.5,7,8],"k":[10,1.000,1e1000,{"z":1},null,-0]}
"#), "[1,2,3,4,5,6]\n", "", 0),
    // doc_obj/foreach_acc/c_long
    (&[r"-c", r".a as $a | . as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [foreach range(6) as $i ([]; . + [($a|contains([range(20)]))]; length)]"], Some(r#"{"a":[0,1.000,"s",null,[1,2],3.5,7,8],"k":[10,1.000,1e1000,{"z":1},null,-0]}
"#), "[1,2,3,4,5,6]\n", "", 0),
    // doc_obj/foreach_acc/c_float3
    (&[r"-c", r".a as $a | . as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [foreach range(6) as $i ([]; . + [($a|contains([1.000, 3.5]))]; length)]"], Some(r#"{"a":[0,1.000,"s",null,[1,2],3.5,7,8],"k":[10,1.000,1e1000,{"z":1},null,-0]}
"#), "[1,2,3,4,5,6]\n", "", 0),
    // doc_obj/foreach_acc/c_str_err
    (&[r"-c", r#".a as $a | . as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [foreach range(6) as $i ([]; . + [($a|try contains("x") catch .)]; length)]"#], Some(r#"{"a":[0,1.000,"s",null,[1,2],3.5,7,8],"k":[10,1.000,1e1000,{"z":1},null,-0]}
"#), "[1,2,3,4,5,6]\n", "", 0),
    // doc_obj/foreach_acc/c_obj_k
    (&[r"-c", r#".a as $a | . as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [foreach range(6) as $i ([]; . + [($o|contains({"k":[$i]}))]; length)]"#], Some(r#"{"a":[0,1.000,"s",null,[1,2],3.5,7,8],"k":[10,1.000,1e1000,{"z":1},null,-0]}
"#), "[1,2,3,4,5,6]\n", "", 0),
    // doc_obj/foreach_acc/c_s
    (&[r"-c", r#".a as $a | . as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [foreach range(6) as $i ([]; . + [($s|contains("\($i)"))]; length)]"#], Some(r#"{"a":[0,1.000,"s",null,[1,2],3.5,7,8],"k":[10,1.000,1e1000,{"z":1},null,-0]}
"#), "[1,2,3,4,5,6]\n", "", 0),
    // doc_obj/foreach_acc/c_bignum
    (&[r"-c", r".a as $a | . as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [foreach range(6) as $i ([]; . + [($a|contains([100000000000000000000]))]; length)]"], Some(r#"{"a":[0,1.000,"s",null,[1,2],3.5,7,8],"k":[10,1.000,1e1000,{"z":1},null,-0]}
"#), "[1,2,3,4,5,6]\n", "", 0),
    // doc_obj/limit/c_one
    (&[r"-c", r".a as $a | . as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [limit(3; range(6) as $i | ($a|contains([$i])))]"], Some(r#"{"a":[0,1.000,"s",null,[1,2],3.5,7,8],"k":[10,1.000,1e1000,{"z":1},null,-0]}
"#), "[true,true,false]\n", "", 0),
    // doc_obj/limit/c_dup
    (&[r"-c", r".a as $a | . as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [limit(3; range(6) as $i | ($a|contains([$i, $i])))]"], Some(r#"{"a":[0,1.000,"s",null,[1,2],3.5,7,8],"k":[10,1.000,1e1000,{"z":1},null,-0]}
"#), "[true,true,false]\n", "", 0),
    // doc_obj/limit/c_long
    (&[r"-c", r".a as $a | . as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [limit(3; range(6) as $i | ($a|contains([range(20)])))]"], Some(r#"{"a":[0,1.000,"s",null,[1,2],3.5,7,8],"k":[10,1.000,1e1000,{"z":1},null,-0]}
"#), "[false,false,false]\n", "", 0),
    // doc_obj/limit/c_float3
    (&[r"-c", r".a as $a | . as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [limit(3; range(6) as $i | ($a|contains([1.000, 3.5])))]"], Some(r#"{"a":[0,1.000,"s",null,[1,2],3.5,7,8],"k":[10,1.000,1e1000,{"z":1},null,-0]}
"#), "[true,true,true]\n", "", 0),
    // doc_obj/limit/c_str_err
    (&[r"-c", r#".a as $a | . as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [limit(3; range(6) as $i | ($a|try contains("x") catch .))]"#], Some(r#"{"a":[0,1.000,"s",null,[1,2],3.5,7,8],"k":[10,1.000,1e1000,{"z":1},null,-0]}
"#), "[\"array ([0,1.000,\\\"s...) and string (\\\"x\\\") cannot have their containment checked\",\"array ([0,1.000,\\\"s...) and string (\\\"x\\\") cannot have their containment checked\",\"array ([0,1.000,\\\"s...) and string (\\\"x\\\") cannot have their containment checked\"]\n", "", 0),
    // doc_obj/limit/c_obj_k
    (&[r"-c", r#".a as $a | . as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [limit(3; range(6) as $i | ($o|contains({"k":[$i]})))]"#], Some(r#"{"a":[0,1.000,"s",null,[1,2],3.5,7,8],"k":[10,1.000,1e1000,{"z":1},null,-0]}
"#), "[true,true,false]\n", "", 0),
    // doc_obj/limit/c_s
    (&[r"-c", r#".a as $a | . as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [limit(3; range(6) as $i | ($s|contains("\($i)")))]"#], Some(r#"{"a":[0,1.000,"s",null,[1,2],3.5,7,8],"k":[10,1.000,1e1000,{"z":1},null,-0]}
"#), "[true,true,true]\n", "", 0),
    // doc_obj/limit/c_bignum
    (&[r"-c", r".a as $a | . as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [limit(3; range(6) as $i | ($a|contains([100000000000000000000])))]"], Some(r#"{"a":[0,1.000,"s",null,[1,2],3.5,7,8],"k":[10,1.000,1e1000,{"z":1},null,-0]}
"#), "[false,false,false]\n", "", 0),
    // input_n/reduce/c_one
    (&[r"-c", r"-n", r"input as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | reduce range(6) as $i ([]; . + [($a|contains([$i]))])"], Some(r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]
"#), "[true,true,false,false,false,false]\n", "", 0),
    // input_n/reduce/c_dup
    (&[r"-c", r"-n", r"input as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | reduce range(6) as $i ([]; . + [($a|contains([$i, $i]))])"], Some(r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]
"#), "[true,true,false,false,false,false]\n", "", 0),
    // input_n/reduce/c_long
    (&[r"-c", r"-n", r"input as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | reduce range(6) as $i ([]; . + [($a|contains([range(20)]))])"], Some(r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]
"#), "[false,false,false,false,false,false]\n", "", 0),
    // input_n/reduce/c_float3
    (&[r"-c", r"-n", r"input as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | reduce range(6) as $i ([]; . + [($a|contains([1.000, 3.5]))])"], Some(r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]
"#), "[true,true,true,true,true,true]\n", "", 0),
    // input_n/reduce/c_str_err
    (&[r"-c", r"-n", r#"input as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | reduce range(6) as $i ([]; . + [($a|try contains("x") catch .)])"#], Some(r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]
"#), "[\"array ([0,1.000,1E...) and string (\\\"x\\\") cannot have their containment checked\",\"array ([0,1.000,1E...) and string (\\\"x\\\") cannot have their containment checked\",\"array ([0,1.000,1E...) and string (\\\"x\\\") cannot have their containment checked\",\"array ([0,1.000,1E...) and string (\\\"x\\\") cannot have their containment checked\",\"array ([0,1.000,1E...) and string (\\\"x\\\") cannot have their containment checked\",\"array ([0,1.000,1E...) and string (\\\"x\\\") cannot have their containment checked\"]\n", "", 0),
    // input_n/reduce/c_obj_k
    (&[r"-c", r"-n", r#"input as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | reduce range(6) as $i ([]; . + [($o|contains({"k":[$i]}))])"#], Some(r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]
"#), "[true,true,false,false,false,false]\n", "", 0),
    // input_n/reduce/c_s
    (&[r"-c", r"-n", r#"input as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | reduce range(6) as $i ([]; . + [($s|contains("\($i)"))])"#], Some(r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]
"#), "[true,true,true,true,false,true]\n", "", 0),
    // input_n/reduce/c_bignum
    (&[r"-c", r"-n", r"input as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | reduce range(6) as $i ([]; . + [($a|contains([100000000000000000000]))])"], Some(r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]
"#), "[false,false,false,false,false,false]\n", "", 0),
    // input_n/foreach_acc/c_one
    (&[r"-c", r"-n", r"input as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [foreach range(6) as $i ([]; . + [($a|contains([$i]))]; length)]"], Some(r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]
"#), "[1,2,3,4,5,6]\n", "", 0),
    // input_n/foreach_acc/c_dup
    (&[r"-c", r"-n", r"input as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [foreach range(6) as $i ([]; . + [($a|contains([$i, $i]))]; length)]"], Some(r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]
"#), "[1,2,3,4,5,6]\n", "", 0),
    // input_n/foreach_acc/c_long
    (&[r"-c", r"-n", r"input as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [foreach range(6) as $i ([]; . + [($a|contains([range(20)]))]; length)]"], Some(r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]
"#), "[1,2,3,4,5,6]\n", "", 0),
    // input_n/foreach_acc/c_float3
    (&[r"-c", r"-n", r"input as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [foreach range(6) as $i ([]; . + [($a|contains([1.000, 3.5]))]; length)]"], Some(r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]
"#), "[1,2,3,4,5,6]\n", "", 0),
    // input_n/foreach_acc/c_str_err
    (&[r"-c", r"-n", r#"input as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [foreach range(6) as $i ([]; . + [($a|try contains("x") catch .)]; length)]"#], Some(r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]
"#), "[1,2,3,4,5,6]\n", "", 0),
    // input_n/foreach_acc/c_obj_k
    (&[r"-c", r"-n", r#"input as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [foreach range(6) as $i ([]; . + [($o|contains({"k":[$i]}))]; length)]"#], Some(r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]
"#), "[1,2,3,4,5,6]\n", "", 0),
    // input_n/foreach_acc/c_s
    (&[r"-c", r"-n", r#"input as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [foreach range(6) as $i ([]; . + [($s|contains("\($i)"))]; length)]"#], Some(r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]
"#), "[1,2,3,4,5,6]\n", "", 0),
    // input_n/foreach_acc/c_bignum
    (&[r"-c", r"-n", r"input as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [foreach range(6) as $i ([]; . + [($a|contains([100000000000000000000]))]; length)]"], Some(r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]
"#), "[1,2,3,4,5,6]\n", "", 0),
    // input_n/limit/c_one
    (&[r"-c", r"-n", r"input as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [limit(3; range(6) as $i | ($a|contains([$i])))]"], Some(r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]
"#), "[true,true,false]\n", "", 0),
    // input_n/limit/c_dup
    (&[r"-c", r"-n", r"input as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [limit(3; range(6) as $i | ($a|contains([$i, $i])))]"], Some(r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]
"#), "[true,true,false]\n", "", 0),
    // input_n/limit/c_long
    (&[r"-c", r"-n", r"input as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [limit(3; range(6) as $i | ($a|contains([range(20)])))]"], Some(r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]
"#), "[false,false,false]\n", "", 0),
    // input_n/limit/c_float3
    (&[r"-c", r"-n", r"input as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [limit(3; range(6) as $i | ($a|contains([1.000, 3.5])))]"], Some(r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]
"#), "[true,true,true]\n", "", 0),
    // input_n/limit/c_str_err
    (&[r"-c", r"-n", r#"input as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [limit(3; range(6) as $i | ($a|try contains("x") catch .))]"#], Some(r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]
"#), "[\"array ([0,1.000,1E...) and string (\\\"x\\\") cannot have their containment checked\",\"array ([0,1.000,1E...) and string (\\\"x\\\") cannot have their containment checked\",\"array ([0,1.000,1E...) and string (\\\"x\\\") cannot have their containment checked\"]\n", "", 0),
    // input_n/limit/c_obj_k
    (&[r"-c", r"-n", r#"input as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [limit(3; range(6) as $i | ($o|contains({"k":[$i]})))]"#], Some(r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]
"#), "[true,true,false]\n", "", 0),
    // input_n/limit/c_s
    (&[r"-c", r"-n", r#"input as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [limit(3; range(6) as $i | ($s|contains("\($i)")))]"#], Some(r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]
"#), "[true,true,true]\n", "", 0),
    // input_n/limit/c_bignum
    (&[r"-c", r"-n", r"input as $a | {k: $a} as $o | ($a|tojson) as $s | null as $n | 1 as $x | true as $t | [limit(3; range(6) as $i | ($a|contains([100000000000000000000])))]"], Some(r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]
"#), "[false,false,false]\n", "", 0),
    // argjson/reduce/c_one
    (&[r"-c", r"-n", r"--argjson", r"a", r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]"#, r"--argjson", r"o", r#"{"k":[7,8,9]}"#, r"($a|tojson) as $s | null as $n | 1 as $x | true as $t | reduce range(6) as $i ([]; . + [($a|contains([$i]))])"], None, "[true,true,false,false,false,false]\n", "", 0),
    // argjson/reduce/c_dup
    (&[r"-c", r"-n", r"--argjson", r"a", r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]"#, r"--argjson", r"o", r#"{"k":[7,8,9]}"#, r"($a|tojson) as $s | null as $n | 1 as $x | true as $t | reduce range(6) as $i ([]; . + [($a|contains([$i, $i]))])"], None, "[true,true,false,false,false,false]\n", "", 0),
    // argjson/reduce/c_long
    (&[r"-c", r"-n", r"--argjson", r"a", r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]"#, r"--argjson", r"o", r#"{"k":[7,8,9]}"#, r"($a|tojson) as $s | null as $n | 1 as $x | true as $t | reduce range(6) as $i ([]; . + [($a|contains([range(20)]))])"], None, "[false,false,false,false,false,false]\n", "", 0),
    // argjson/reduce/c_float3
    (&[r"-c", r"-n", r"--argjson", r"a", r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]"#, r"--argjson", r"o", r#"{"k":[7,8,9]}"#, r"($a|tojson) as $s | null as $n | 1 as $x | true as $t | reduce range(6) as $i ([]; . + [($a|contains([1.000, 3.5]))])"], None, "[true,true,true,true,true,true]\n", "", 0),
    // argjson/reduce/c_str_err
    (&[r"-c", r"-n", r"--argjson", r"a", r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]"#, r"--argjson", r"o", r#"{"k":[7,8,9]}"#, r#"($a|tojson) as $s | null as $n | 1 as $x | true as $t | reduce range(6) as $i ([]; . + [($a|try contains("x") catch .)])"#], None, "[\"array ([0,1.000,1E...) and string (\\\"x\\\") cannot have their containment checked\",\"array ([0,1.000,1E...) and string (\\\"x\\\") cannot have their containment checked\",\"array ([0,1.000,1E...) and string (\\\"x\\\") cannot have their containment checked\",\"array ([0,1.000,1E...) and string (\\\"x\\\") cannot have their containment checked\",\"array ([0,1.000,1E...) and string (\\\"x\\\") cannot have their containment checked\",\"array ([0,1.000,1E...) and string (\\\"x\\\") cannot have their containment checked\"]\n", "", 0),
    // argjson/reduce/c_obj_k
    (&[r"-c", r"-n", r"--argjson", r"a", r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]"#, r"--argjson", r"o", r#"{"k":[7,8,9]}"#, r#"($a|tojson) as $s | null as $n | 1 as $x | true as $t | reduce range(6) as $i ([]; . + [($o|contains({"k":[$i]}))])"#], None, "[false,false,false,false,false,false]\n", "", 0),
    // argjson/reduce/c_s
    (&[r"-c", r"-n", r"--argjson", r"a", r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]"#, r"--argjson", r"o", r#"{"k":[7,8,9]}"#, r#"($a|tojson) as $s | null as $n | 1 as $x | true as $t | reduce range(6) as $i ([]; . + [($s|contains("\($i)"))])"#], None, "[true,true,true,true,false,true]\n", "", 0),
    // argjson/reduce/c_bignum
    (&[r"-c", r"-n", r"--argjson", r"a", r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]"#, r"--argjson", r"o", r#"{"k":[7,8,9]}"#, r"($a|tojson) as $s | null as $n | 1 as $x | true as $t | reduce range(6) as $i ([]; . + [($a|contains([100000000000000000000]))])"], None, "[false,false,false,false,false,false]\n", "", 0),
    // argjson/foreach_acc/c_one
    (&[r"-c", r"-n", r"--argjson", r"a", r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]"#, r"--argjson", r"o", r#"{"k":[7,8,9]}"#, r"($a|tojson) as $s | null as $n | 1 as $x | true as $t | [foreach range(6) as $i ([]; . + [($a|contains([$i]))]; length)]"], None, "[1,2,3,4,5,6]\n", "", 0),
    // argjson/foreach_acc/c_dup
    (&[r"-c", r"-n", r"--argjson", r"a", r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]"#, r"--argjson", r"o", r#"{"k":[7,8,9]}"#, r"($a|tojson) as $s | null as $n | 1 as $x | true as $t | [foreach range(6) as $i ([]; . + [($a|contains([$i, $i]))]; length)]"], None, "[1,2,3,4,5,6]\n", "", 0),
    // argjson/foreach_acc/c_long
    (&[r"-c", r"-n", r"--argjson", r"a", r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]"#, r"--argjson", r"o", r#"{"k":[7,8,9]}"#, r"($a|tojson) as $s | null as $n | 1 as $x | true as $t | [foreach range(6) as $i ([]; . + [($a|contains([range(20)]))]; length)]"], None, "[1,2,3,4,5,6]\n", "", 0),
    // argjson/foreach_acc/c_float3
    (&[r"-c", r"-n", r"--argjson", r"a", r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]"#, r"--argjson", r"o", r#"{"k":[7,8,9]}"#, r"($a|tojson) as $s | null as $n | 1 as $x | true as $t | [foreach range(6) as $i ([]; . + [($a|contains([1.000, 3.5]))]; length)]"], None, "[1,2,3,4,5,6]\n", "", 0),
    // argjson/foreach_acc/c_str_err
    (&[r"-c", r"-n", r"--argjson", r"a", r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]"#, r"--argjson", r"o", r#"{"k":[7,8,9]}"#, r#"($a|tojson) as $s | null as $n | 1 as $x | true as $t | [foreach range(6) as $i ([]; . + [($a|try contains("x") catch .)]; length)]"#], None, "[1,2,3,4,5,6]\n", "", 0),
    // argjson/foreach_acc/c_obj_k
    (&[r"-c", r"-n", r"--argjson", r"a", r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]"#, r"--argjson", r"o", r#"{"k":[7,8,9]}"#, r#"($a|tojson) as $s | null as $n | 1 as $x | true as $t | [foreach range(6) as $i ([]; . + [($o|contains({"k":[$i]}))]; length)]"#], None, "[1,2,3,4,5,6]\n", "", 0),
    // argjson/foreach_acc/c_s
    (&[r"-c", r"-n", r"--argjson", r"a", r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]"#, r"--argjson", r"o", r#"{"k":[7,8,9]}"#, r#"($a|tojson) as $s | null as $n | 1 as $x | true as $t | [foreach range(6) as $i ([]; . + [($s|contains("\($i)"))]; length)]"#], None, "[1,2,3,4,5,6]\n", "", 0),
    // argjson/foreach_acc/c_bignum
    (&[r"-c", r"-n", r"--argjson", r"a", r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]"#, r"--argjson", r"o", r#"{"k":[7,8,9]}"#, r"($a|tojson) as $s | null as $n | 1 as $x | true as $t | [foreach range(6) as $i ([]; . + [($a|contains([100000000000000000000]))]; length)]"], None, "[1,2,3,4,5,6]\n", "", 0),
    // argjson/limit/c_one
    (&[r"-c", r"-n", r"--argjson", r"a", r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]"#, r"--argjson", r"o", r#"{"k":[7,8,9]}"#, r"($a|tojson) as $s | null as $n | 1 as $x | true as $t | [limit(3; range(6) as $i | ($a|contains([$i])))]"], None, "[true,true,false]\n", "", 0),
    // argjson/limit/c_dup
    (&[r"-c", r"-n", r"--argjson", r"a", r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]"#, r"--argjson", r"o", r#"{"k":[7,8,9]}"#, r"($a|tojson) as $s | null as $n | 1 as $x | true as $t | [limit(3; range(6) as $i | ($a|contains([$i, $i])))]"], None, "[true,true,false]\n", "", 0),
    // argjson/limit/c_long
    (&[r"-c", r"-n", r"--argjson", r"a", r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]"#, r"--argjson", r"o", r#"{"k":[7,8,9]}"#, r"($a|tojson) as $s | null as $n | 1 as $x | true as $t | [limit(3; range(6) as $i | ($a|contains([range(20)])))]"], None, "[false,false,false]\n", "", 0),
    // argjson/limit/c_float3
    (&[r"-c", r"-n", r"--argjson", r"a", r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]"#, r"--argjson", r"o", r#"{"k":[7,8,9]}"#, r"($a|tojson) as $s | null as $n | 1 as $x | true as $t | [limit(3; range(6) as $i | ($a|contains([1.000, 3.5])))]"], None, "[true,true,true]\n", "", 0),
    // argjson/limit/c_str_err
    (&[r"-c", r"-n", r"--argjson", r"a", r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]"#, r"--argjson", r"o", r#"{"k":[7,8,9]}"#, r#"($a|tojson) as $s | null as $n | 1 as $x | true as $t | [limit(3; range(6) as $i | ($a|try contains("x") catch .))]"#], None, "[\"array ([0,1.000,1E...) and string (\\\"x\\\") cannot have their containment checked\",\"array ([0,1.000,1E...) and string (\\\"x\\\") cannot have their containment checked\",\"array ([0,1.000,1E...) and string (\\\"x\\\") cannot have their containment checked\"]\n", "", 0),
    // argjson/limit/c_obj_k
    (&[r"-c", r"-n", r"--argjson", r"a", r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]"#, r"--argjson", r"o", r#"{"k":[7,8,9]}"#, r#"($a|tojson) as $s | null as $n | 1 as $x | true as $t | [limit(3; range(6) as $i | ($o|contains({"k":[$i]})))]"#], None, "[false,false,false]\n", "", 0),
    // argjson/limit/c_s
    (&[r"-c", r"-n", r"--argjson", r"a", r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]"#, r"--argjson", r"o", r#"{"k":[7,8,9]}"#, r#"($a|tojson) as $s | null as $n | 1 as $x | true as $t | [limit(3; range(6) as $i | ($s|contains("\($i)")))]"#], None, "[true,true,true]\n", "", 0),
    // argjson/limit/c_bignum
    (&[r"-c", r"-n", r"--argjson", r"a", r#"[0,1.000,1e1000,100000000000000000001,"s",null,{"x":1},[1,2],-0,3.5,{"k":[5,6]}]"#, r"--argjson", r"o", r#"{"k":[7,8,9]}"#, r"($a|tojson) as $s | null as $n | 1 as $x | true as $t | [limit(3; range(6) as $i | ($a|contains([100000000000000000000])))]"], None, "[false,false,false]\n", "", 0),
];

#[test]
fn test_contains_of_a_bound_container_in_a_fold_matches_jq_7785() -> Result<()> {
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

/// (program, stdout, stderr, exit code) for `succinctly jq -nc <program>`: needles of another kind
/// that raise, `null`, booleans, fan-out and computed needles, duplicates, substrings inside
/// elements and members, `1` against `1.0`, `nan`, numbers past a double, NUL in a string, and
/// reads under `?`, `try`, `debug`, `label`/`break`, `first(foreach ...)` and an `as` bind.
const EDGES: &[(&str, &str, &str, i32)] = &[
    (r#"[range(4)] as $a | reduce range(3) as $i (0; if $a|contains("x") then 1 else 0 end)"#, "", "jq: error (at <unknown>): array ([0,1,2,3]) and string (\"x\") cannot have their containment checked\n", 5),
    (r"{} as $o | [range(2) as $i | $o|contains([1])]", "", "jq: error (at <unknown>): object ({}) and array ([1]) cannot have their containment checked\n", 5),
    (r#""abc" as $s | [range(2) as $i | $s | contains(1)]"#, "", "jq: error (at <unknown>): string (\"abc\") and number (1) cannot have their containment checked\n", 5),
    (r"null as $n | [range(2) as $i | $n | contains(null)]", "[true,true]\n", "", 0),
    (r"null as $n | [range(2) as $i | $n | contains([])]", "", "jq: error (at <unknown>): null (null) and array ([]) cannot have their containment checked\n", 5),
    (r#"[1,[2,3],{"a":[4]}] as $a | [range(2) as $i | $a | contains([{"a":[4]}], [[3]])]"#, "[true,true,true,true]\n", "", 0),
    (r"[1,1,1] as $a | reduce range(2) as $i (0; . + ($a|contains([1,1,1,1])|if . then 1 else 0 end))", "2\n", "", 0),
    (r#"["foobar"] as $a | [range(2) as $i | $a|contains(["oba"])]"#, "[true,true]\n", "", 0),
    (r#"{"a":"foobar","b":[1,2]} as $o | [range(2) as $i | $o | contains({"a":"bar","b":[2]})]"#, "[true,true]\n", "", 0),
    (r#"{"a":"foobar","b":[1,2]} as $o | reduce range(2) as $i ([]; . + [$o | contains({"a":"baz"})])"#, "[false,false]\n", "", 0),
    (r"[1.0, 2] as $a | [range(2) as $i | $a|contains([1])]", "[true,true]\n", "", 0),
    (r"[1, 2] as $a | reduce range(2) as $i ([]; . + [$a|contains([1.000])])", "[true,true]\n", "", 0),
    (r#"[{"a":1,"b":2}] as $a | [range(2) as $i | $a | contains([{"a":1}])]"#, "[true,true]\n", "", 0),
    (r"[[]] as $a | [range(2) as $i | $a|contains([[]])]", "[true,true]\n", "", 0),
    (r"[] as $a | [range(2) as $i | $a|contains([])]", "[true,true]\n", "", 0),
    (r"[] as $a | [range(2) as $i | $a|contains([1])]", "[false,false]\n", "", 0),
    (r#""a\u0000b" as $s | [range(2) as $i | $s | contains("b")]"#, "[true,true]\n", "", 0),
    (r"[range(4)] as $a | reduce range(3) as $i (0; . + ($a|contains([$i])?|if . then 1 else 0 end))", "3\n", "", 0),
    (r"[range(4)] as $a | [range(3) as $i | $a | contains([$i]) | debug]", "[true,true,true]\n", "[\"DEBUG:\",true]\n[\"DEBUG:\",true]\n[\"DEBUG:\",true]\n", 0),
    (r"[range(4)] as $a | [first(foreach range(3) as $i (0; . + ($a | contains([$i]) | if . then 1 else 0 end)))]", "[1]\n", "", 0),
    (r"true as $t | [range(2) as $i | $t | contains(true)]", "[true,true]\n", "", 0),
    (r"true as $t | [range(2) as $i | $t | contains(false)]", "", "jq: error (at <unknown>): boolean (true) and boolean (false) cannot have their containment checked\n", 5),
    (r#""é" as $s | [range(2) as $i | $s | contains("é")]"#, "[true,true]\n", "", 0),
    (r"[nan] as $a | [range(2) as $i | $a | contains([nan])]", "[false,false]\n", "", 0),
    (r"[1,2] as $a | [range(2) as $i | $a | inside([1,2,3])]", "[true,true]\n", "", 0),
    (r"[100000000000000000001] as $a | [range(2) as $i | $a | contains([100000000000000000000])]", "[false,false]\n", "", 0),
    (r"[9007199254740993] as $a | reduce range(2) as $i ([]; . + [$a | contains([9007199254740992])])", "[false,false]\n", "", 0),
    (r"[range(4)] as $a | label $out | reduce range(3) as $i (0; if $i == 2 then break $out elif $a|contains([$i]) then . + 1 else . end)", "", "", 0),
    (r#"[range(4)] as $a | [range(3) as $i | $a | try contains(error("e")) catch .]"#, "[\"e\",\"e\",\"e\"]\n", "", 0),
    (r"[range(4)] as $a | [range(3) as $i | $a | contains([$i] | map(. + 1))]", "[true,true,true]\n", "", 0),
    (r#"{"k":[1,{"x":[2,3]}]} as $o | [range(2) as $i | $o | contains({"k":[{"x":[3]}]})]"#, "[true,true]\n", "", 0),
    (r"[range(4)] as $a | reduce range(3) as $i (0; ($a|contains([$i])) as $v | if $v then . + 1 else . end)", "3\n", "", 0),
];

#[test]
fn test_contains_of_a_bound_container_edges_match_jq_7785() -> Result<()> {
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
