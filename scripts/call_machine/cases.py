"""The call machine's corpus (#7977, from the #7957 spikes).

CORRECT: the machine must give jq 1.7.1's stdout, stderr and exit status on each.
  `route` says whether the machine is expected to take the program ("take") or to decline it
  ("decline": the shipped evaluators answer), or whether jq rejects it at compile time, before any
  route is decided ("compile-error"). The machine may take more of them, never fewer of
  the "take" set.
LADDERS: the measurement ladders (`ladders.py`, run on the bench boxes). `{N}` is replaced.

Every "take" program holds a self-recursive def: the machine routes only those.

`check.py capture` records jq's answers into `tests/data/call-machine-corpus-7977.json`, which
`check.py check` and `tests/jq_cli/i7977.rs` compare against. Change a case here, then capture.
"""

# A recursive identity wrapper, so a form is evaluated under a routed program.
W = "def w($k): if $k <= 0 then BODY else w($k - 1) end; "


def wrap(body, inp="null"):
    return W.replace("BODY", body) + f"{inp} | w(2)"


CORRECT = []


def take(cid, body, inp="null", args=("-c", "-n")):
    CORRECT.append(dict(id=cid, args=list(args), filter=wrap(body, inp), route="take", stdin=None))


def raw(cid, filt, route="take", args=("-c", "-n"), stdin=None, off_known=None):
    """`stdin`: the text fed to the run (`None`: nothing, as `-n` reads none). `off_known`: why the
    shipped route (`check.py check --off`) answers otherwise, for a case jq and the machine agree on."""
    row = dict(id=cid, args=list(args), filter=filt, route=route, stdin=stdin)
    if off_known:
        row["off_known"] = off_known
    CORRECT.append(row)


# --- forms, each under a routed recursive def -------------------------------------
take("pipe", ". + 1 | . * 2", "3")
take("comma", "1, 2, (3, 4)")
take("comma_empty", "1, empty, 2")
take("arith", "[1 + 2, 7 - 3, 2 * 5, 9 / 3, 7 % 3, \"a\" + \"b\", [1] + [2], {a:1} + {b:2}]")
take("compare", "[1 < 2, 2 <= 1, 1 == 1.0, \"a\" != \"b\", [1] > [0], null < false]")
take("and_or", "[true and false, false or true, (1,null) and true, (false, 2) or false]")
take("alt", "[null // 1, (false, 2) // 3, (null, false) // 4, empty // 5]")
take("if", "[range(5) | if . < 2 then \"lo\" elif . < 4 then \"mid\" else \"hi\" end]")
take("if_gen_cond", "[if (true, false) then 1 else 2 end]")
take("as_simple", "5 as $x | $x + 1")
take("as_gen", "[(1, 2) as $x | (10, 20) as $y | $x + $y]")
take("as_array_pat", "[1, [2, 3]] as [$a, [$b, $c]] | $a + $b + $c")
take("as_object_pat", "{a: 1, b: {c: 2}} as {a: $a, b: {c: $c}} | [$a, $c]")
take("collect", "[range(3), (4, 5)]")
take("collect_empty", "[empty]")
take("reduce", "reduce range(10) as $i (0; . + $i)")
take("reduce_gen_update", "reduce (1, 2) as $i (0; ., 10)")
take("reduce_empty_update", "reduce (1, 2) as $i (0; empty)")
take("foreach2", "[foreach range(5) as $i (0; . + $i)]")
take("foreach3", "[foreach range(5) as $i (0; . + $i; [$i, .])]")
take("foreach_gen", "[foreach (1, 2) as $i (0; . + $i, . * 10)]")
take("each_array", "[[1, 2, 3] | .[] | . * 2]")
take("each_object", "[{a: 1, b: 2} | .[]]")
take("each_opt", "[1 | .[]?]")
take("each_error", "[1] | try (2 | .[]) catch .")
take("index", "{a: {b: [10, 20, 30]}} | [.a.b[1], .a.b[-1], .a[\"b\"][0], .x]")
take("slice", "[[1, 2, 3, 4] | .[1:3], .[:-1], .[2:]]")
take("range1", "[range(4)]")
take("range2", "[range(2; 6)]")
take("range3", "[range(0; 10; 3)]")
take("range_float", "[range(0; 1; 0.25)]")
take("range_gen_bounds", "[range(1, 2; 4)]")
take("try_catch", "try error(\"x\") catch .")
take("try_value_error", "try error({a: 1}) catch .a")
take("try_null_error", "try error(null) catch .")
take("try_no_catch", "[try error(\"x\")]")
take("try_after_output", "[try (1, error(\"x\"), 3) catch .]")
take("try_end_rule", "try ((try (1, 2) catch \"inner\") | if . == 2 then error(\"late\") else . end) catch \"outer: \" + .")
take("try_end_rule2", "[(try (1, 2) catch 9) as $x | try (if $x == 2 then error(\"e\") else $x end) catch \"c\"]")
take("question", "[(1, error(\"x\"), 3)?]")
take("question_index", "[1, {a: 2}] | [.[] | .a?]")
take("error_uncaught", "1, error(\"boom\"), 3")
take("error_object", "error({a: 1})")
take("error_type", "{} - 1")
take("label_break", "[label $f | range(10) | ., (select(. == 3) | break $f)]")
take("label_nested", "[label $a | label $b | 1, break $b, 2] , [label $a | (label $b | 1, break $a), 3]")
take("first", "first(range(10; 0; -1))")
take("first_lazy_error", "first(1, error(\"never\"))")
take("first_each_lazy", "[1, 2, 3] | first(.[] | if . == 2 then error(\"never\") else . end)")
take("limit", "[limit(3; range(100))]")
take("limit_zero", "[limit(0; 1, error(\"never\"))]")
take("limit_neg", "[limit(-1; 1, 2)]")
take("until", "0 | until(. >= 5; . + 2)")
take("while", "[1 | while(. < 50; . * 3)]")
take("repeat", "[limit(5; 1 | repeat(. * 2))]")
take("select", "[range(10) | select(. % 3 == 0)]")
take("recurse", "[{a: [1, {b: 2}]} | recurse | numbers]")
take("recurse_f", "[2 | recurse(if . < 50 then . * . else empty end)]")
take("map", "[1, 2, 3] | map(. + 1)")
take("add_builtin", "[1, 2, 3] | add")
take("length_keys", "{b: 1, a: [1, 2]} | [length, keys, (.a | length)]")
take("tostring_tojson", "[1, \"a\", [1]] | map(tostring), tojson")
take("string_interp_leaf", "\"v=\\(1 + 2)\"")
take("object_leaf", "{a: 1, b: (2, 3)}")
take("debug", "1 | debug | . + 1")
take("stderr", "\"x\" | stderr | 1")
take("not", "[true, false, null, 0] | map(not)")
take("env_free_vars", "1 as $a | (2 as $a | $a), $a")
take("shadow_def", "def g: 1; def h: g; def g: 2; [g, h]")

# --- definitions, closures and parameters ------------------------------------------
raw("closure_param", "def f(g; $n): if $n <= 0 then g else f(g + 1; $n - 1) end; f(10; 5)")
raw("closure_captures_caller", "def f(g; $n): if $n <= 0 then [g] else 100 as $x | f(g; $n - 1) end; 1 as $x | f($x; 3)")
raw("closure_input", "def f(g; $n): if $n <= 0 then g else f(g | . * 2; $n - 1) end; 3 | f(.; 4)")
raw("closure_gen", "def f(g; $n): if $n <= 0 then [g] else f(g, 9; $n - 1) end; f(1, 2; 2)")
raw("bare_param", "def s(n): if n == 0 then 0 else n + s(n - 1) end; s(10)")
raw("dollar_param_gen", "def f($x; $n): if $n == 0 then $x else f($x * 2; $n - 1) end; [f(1, 3; 3)]")
raw("dollar_param_both", "def f($a): if $a > 0 then [$a, a] , f($a - 1) else empty end; [f(2)]")
raw("nested_def", "def outer($n): def inner($m): if $m == 0 then $n else inner($m - 1) end; inner($n); [outer(3), outer(5)]")
raw("nested_def_closure", "def f(g): def h($n): if $n == 0 then g else h($n - 1) end; h(3); 7 | f(. + 1)")
raw("mutual", "def ev($n): def od($m): if $m == 0 then false else ev($m - 1) end; if $n == 0 then true else od($n - 1) end; [ev(10), ev(7)]")
raw("recursion_linear", "def f($n): if $n == 0 then 0 else f($n - 1) + 1 end; f(1000)")
raw("recursion_tail", "def r($p1; $p2): if $p1 == 0 then $p2 else r($p1 - 1; $p2) end; r(5000; 7)")
raw("recursion_tail_zero", "def f: if . < 3000 then . + 1 | f else . end; 0 | f")
raw("recursion_tree_pipe", "def f($n): if $n == 0 then . + 1 else f($n - 1) | f($n - 1) end; 0 | f(10)")
raw("recursion_tree_plus", "def t($n): if $n <= 0 then 1 else t($n - 1) + t($n - 1) end; t(10)")
raw("recursion_generator", "def f: if . > 0 then (. - 1 | f), . else . end; [5 | f]")
raw("fib", "def fib: if . < 2 then . else (. - 1 | fib) + (. - 2 | fib) end; 15 | fib")
raw("tree_data_dependent_leaf", "def t($n): if $n <= 0 then (1, 2) | select(. == 1) else t($n - 1) + t($n - 1) end; [0 | t(8)] | length")
raw("tree_each_leaf", "def t($n): if $n <= 0 then [1] | .[] as $v | $v else t($n - 1) + t($n - 1) end; [t(8)] | length")
raw("tree_foreach_leaf", "def t($n): if $n <= 0 then (foreach (1, 2) as $x (0; . + $x)) | select(. == 1) else t($n - 1) + t($n - 1) end; [t(6)] | length")
raw("issue_7870_small", "def f(g; $n): if $n <= 0 then g else f(g + (1, 3); $n - 1) end; first(f((1, 3); 12)) | type")
raw("issue_7870_count", "def f(g; $n): if $n <= 0 then g else f(g + (1, 3); $n - 1) end; [f((1, 3); 6)] | length")
raw("issue_7699_small", "def f($n): if $n==0 then .+1 else f($n-1)|f($n-1) end; 0|f(8)")
raw("issue_7803_small", "def Q: def Q(n): n as $n | (if . == null then [1, 1, 1] else . end) as $q | if $q[$n] != null then $q else $q | Q($n - 1) as $q1 | $q1 | Q($n - 2) as $q2 | $q2 | Q($n - $q2[$n - 1]) as $q3 | $q3 | Q($n - $q3[$n - 2]) as $q4 | ($q4[$n - $q4[$n - 1]] + $q4[$n - $q4[$n - 2]]) as $ans | $q4 | setpath([$n]; $ans) end; . as $n | null | Q($n) | .[$n]; 100 | Q")
raw("issue_7893_small", "def f: if . > 0 then (. - 1 | f), . else . end; [300 | f] | length")
raw("issue_7928_small", "def f(g; $n): if $n > 0 then f(try (g + (1, error(\"x\"))) catch 0; $n - 1) + f(g; $n - 1) else g end; first(f(null; 4) | select(. > 30))")
raw("setpath_carry", "def g($n): if $n == 0 then [range(50)] else g($n - 1) | setpath([$n]; 1) end; g(40) | add")
raw("error_in_def_after_output", "def f($n): if $n == 0 then 1, error(\"late\") else f($n - 1) end; [try f(3) catch .]")
raw("first_stops_recursion", "def nat: ., (. + 1 | nat); first(0 | nat | select(. > 5))")
raw("limit_recursion", "def nat: ., (. + 1 | nat); [limit(5; 0 | nat)]")
raw("label_through_def", "def f($n): if $n == 0 then 1, 2, 3 else f($n - 1) end; [label $out | f(3) | if . == 2 then break $out else . end]")
raw("break_out_of_recursion", "def nat: ., (. + 1 | nat); [label $q | 0 | nat | if . > 3 then break $q else . end]")
raw("reduce_over_recursion", "def nat($m): if . < $m then ., (. + 1 | nat($m)) else empty end; reduce (0 | nat(100)) as $x (0; . + $x)")
raw("debug_order", "def f($n): if $n == 0 then (1 | debug), (2 | debug) else f($n - 1) end; first(f(2))")
raw("deep_ok_10000", "def f($n): if $n == 0 then 0 else f($n - 1) + 1 end; f(10000)",
    off_known="the shipped evaluators refuse it at their native-stack floor")

# --- programs the spikes must decline (main answers) ------------------------------
raw("decl_no_recursion", "def k(f): f * 2; [1, 2] | map(k(.))", route="decline")
raw("decl_alt_pattern", "def f($n): if $n == 0 then [1] as [$x] ?// $x | $x else f($n - 1) end; f(2)", route="decline")
raw("decl_input", "def f($n): if $n == 0 then input else f($n - 1) end; f(2)", route="decline", args=("-c", "-n"))
raw("decl_path", "def d($n): if $n == 0 then .a else d($n - 1) end; {a: 1} | path(d(3))", route="decline")
raw("decl_last", "def f($n): if $n == 0 then 1, 2 else f($n - 1) end; last(f(3))", route="decline")

# --- the run around the machine: stdin, several documents, exit statuses, flags (#7977 review) ---
raw("ndjson_caught_break", "def f($n): if $n == 0 then (label $out | try (break $out) catch .) else f($n - 1) end; f(2)",
    args=("-c",), stdin="1\n2\n")
raw("stdin_error_location", "def f($n): if $n == 0 then . + 1 else f($n - 1) end; f(2)",
    args=("-c",), stdin='1\n"x"\n')
raw("halt_error", 'def f($n): if $n == 0 then "bye\\n" | halt_error else f($n - 1) end; f(2)')
raw("exit_status_e", "def f($n): if $n == 0 then false else f($n - 1) end; f(2)", args=("-c", "-e", "-n"))
raw("slurp", "def f($n): if $n == 0 then add else f($n - 1) end; f(2)", args=("-c", "-s"), stdin="1 2 3")

# --- a side effect inside a leaf that can answer twice: declined, so jq's order holds (#7977 review) ---
raw("leaf_debug_first", "def f($n): if $n == 0 then {a: ((1,2) | debug)} else f($n - 1) end; first(f(2))", route="decline")
raw("leaf_debug_first_object", "def r: if false then r else . end; first({a: (1, (2|debug))})", route="decline")
raw("leaf_stderr_interp", 'def r: if false then r else . end; first("\\((1,2)|stderr)")', route="decline")
raw("leaf_debug_index", "def r: if false then r else . end; [5,6] | first(.[((0,1)|debug)])", route="decline")
raw("leaf_debug_order", "def f($n): if $n == 0 then {a: ((1,2) | debug)} else f($n - 1) end; f(0) | debug | empty",
    route="decline")
raw("native_debug_optional", "def r: if false then r else . end; first(((1,2)|debug)?)")
raw("native_debug_negate", "def r: if false then r else . end; first(-((1,2)|debug))")

# --- a name bound twice in one pattern is its first store (#7977 review) ---
R = "def r: if false then r else . end; "
raw("dup_array", R + "[1,2] as [$a, $a] | $a")
raw("dup_object", R + '{"a":1,"b":2} as {a: $x, b: $x} | $x')
raw("dup_nested_array", R + "[[1,2],3] as [[$a, $a], $a] | $a")
raw("dup_object_array", R + '{"a":1,"b":[2]} as {a: $x, b: [$x]} | $x')
raw("dup_array_object", R + '[{"a":1},2] as [{a: $x}, $x] | $x')
raw("dup_reduce", R + "reduce ([1,2],[3,4]) as [$a, $a] (0; . + $a)")
raw("dup_foreach", R + "[foreach ([1,2],[3,4]) as [$a, $a] (0; . + $a)]")

# --- `range`'s first output negates as jq's (#7977 review) ---
raw("range2_negate", R + "range(0;1) | -.")
raw("range3_negate", R + "[range(0;3;1) | -.]")
raw("range2_pairs", R + "[range(0;2) | [., -.]]")

# --- from the re-review of #7984 ---
raw("leaf_halt_error_first", R + r'first({a: (1, ("bye\n" | halt_error(1)))})', route="decline")
raw("leaf_halt_error_interp", R + r'first("\(1, ("bye\n" | halt_error(1)))")', route="decline")
raw("range3_negate_var", R + "0 as $x | [range($x; 2; 1) | -.]")
raw("range3_negate_input", R + "[0 | range(.;3;1) | -.]")
raw("range3_negate_gen", R + "[range((0,1);3;1) | -.]")
raw("range3_negate_param", R + "def g($n): [range($n; 2; 1) | -.]; g(0)")
raw("pattern_key_outer", R + '"z" as $x | {"a":"b","b":"c","z":9} as {a: $x, ($x): $y} | [$x, $y]')
raw("pattern_key_outer_shorthand", R + '"z" as $x | {"x":1,"z":9} as {$x, ($x): $y} | [$x,$y]')
raw("pattern_key_outer_reduce", R + '"z" as $x | reduce ({"a":"b","z":1},{"a":"c","z":2}) as {a: $x, ($x): $y} (0; . + $y)')
raw("pattern_key_outer_nested", R + '"z" as $x | {"a":{"k":"b"},"z":5} as {a: {k: $x}, ($x): $y} | [$x,$y]')
raw("pattern_key_unbound", R + '{"a":"b"} as {a: $x, ($x): $y} | $y', route="compile-error")

# --- step 3: a call-free `[...]` or `reduce` is one leaf with the generators it collects ---
raw("leaf_collect_range", R + "[range(5)] | length")
raw("leaf_collect_each", R + "[1, 2, 3] | [.[] | . * 2]")
raw("leaf_collect_nested", R + "[[1, 2], [3]] | [.[] | [.[] | . * 10]]")
raw("leaf_reduce_each", R + "[1, 2, 3] | reduce .[] as $x (0; . + $x)")
raw("leaf_reduce_range", R + "reduce range(5) as $i ([]; . + [$i])")
raw("leaf_collect_object", R + "{a: [1, 2]} | {b: [.a[] | . + 1]}")
raw("leaf_collect_interp", R + '"\\([range(3)])"')
raw("leaf_collect_error", R + '[range(3) | if . == 1 then error("e") else . end]')
raw("leaf_collect_try", R + "[[1], 2] | try [.[] | .[]] catch .")
raw("leaf_collect_negate", R + "[range(3) | -.]")
raw("leaf_collect_vars", R + "3 as $a | def g($m): [range($m) | . * $a]; [g(2), (1 as $a | [range(2) | $a])]")
raw("leaf_first_each_stops", R + '[1, 2, 3] | first(.[] | if . == 2 then error("never") else . end)')
raw("leaf_limit_range_stops", R + "[limit(3; range(1e9))]")
raw("leaf_label_each_stops", R + "[1, 2, 3] | [label $x | .[] | ., break $x]")
raw("leaf_collect_debug", R + "[1, 2] | [.[] | debug]")
raw("leaf_pattern_twice", R + "[[1,2],[3,4]] | reduce .[] as [$a, $a] (0; . + $a)")
raw("leaf_pattern_object_declined", R + "{a: ([1,2] as [$a, $a] | $a)}", route="decline")
raw("first_each_ladder", "def w($n): if $n == 0 then first(.[]) else w($n - 1) end; [range(1000)] | w(1)")


# --- measurement ladders (shape and cost; run on the bench boxes) ---------------
LADDERS = [
    dict(id="L01_7870_first", filter="def f(g; $n): if $n <= 0 then g else f(g + (1,3); $n - 1) end; first(f((1,3); {N})) | type", sizes=[16, 500, 20000, 128000]),
    dict(id="L02_7699_pipe2", filter="def f($n): if $n==0 then .+1 else f($n-1)|f($n-1) end; 0|f({N})", sizes=[12, 14, 16]),
    dict(id="L03_plus2", filter="def f($n): if $n == 0 then 1 else f($n-1) + f($n-1) end; f({N})", sizes=[12, 14, 16]),
    dict(id="L04_fib", filter="def fib: if . < 2 then . else (. - 1 | fib) + (. - 2 | fib) end; {N} | fib", sizes=[18, 20, 22]),
    dict(id="L05_7803_memo", filter=CORRECT[[c["id"] for c in CORRECT].index("issue_7803_small")]["filter"].replace("100 | Q", "{N} | Q"), sizes=[400, 800, 1600, 3200]),
    dict(id="L06_7787_select_leaf", filter="def t($n): if $n <= 0 then (1, 2) | select(. == 1) else t($n-1) + t($n-1) end; [0 | t({N})] | length", sizes=[12, 14, 16, 18]),
    dict(id="L07_7787_each_leaf", filter="def t($n): if $n <= 0 then [1] | .[] as $v | $v else t($n-1) + t($n-1) end; [0 | t({N})] | length", sizes=[12, 14, 16]),
    dict(id="L08_7893_gen", filter="def f: if . > 0 then (. - 1 | f), . else . end; [{N} | f] | length", sizes=[2000, 4000, 8000, 64000]),
    dict(id="L09_linear", filter="def f($n): if $n == 0 then 0 else f($n - 1) + 1 end; f({N})", sizes=[10000, 100000, 1000000]),
    dict(id="L10_tail_args", filter="def r($p1; $p2): if $p1 == 0 then $p2 else r($p1 - 1; $p2) end; r({N}; 7)", sizes=[10000, 1000000]),
    dict(id="L11_tail_zero", filter="def f: if . < {N} then . + 1 | f else . end; 0 | f", sizes=[100000, 1000000]),
    dict(id="L12_setpath_carry", filter="def g($n): if $n == 0 then [range({N})] else g($n - 1) | setpath([$n]; 1) end; g({N}) | length", sizes=[1600, 3200, 6400]),
    dict(id="L13_first_each", filter="def w($n): if $n == 0 then first(.[]) else w($n - 1) end; [range({N})] | w(1)", sizes=[100000, 1000000]),
    dict(id="L13c_first_each_control", filter="def w($n): if $n == 0 then .[0] else w($n - 1) end; [range({N})] | w(1)", sizes=[100000, 1000000]),
    dict(id="L14_7928", filter="def f(g; $n): if $n > 0 then f(try (g + (1, error(\"x\"))) catch 0; $n - 1) + f(g; $n - 1) else g end; first(f(null; {N}) | select(. > 30))", sizes=[4, 5, 6, 8]),
    dict(id="L15_startup", filter="1", sizes=[0]),
]
