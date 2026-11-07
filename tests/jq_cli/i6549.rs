//! #6549: a `reduce`/`foreach` step whose `.`-free constructor extends a slot further down
//! (`{a: {b: (.a.b + [$i])}}`), chains `+` (`.a + [$i] + [0]`) or counts the slot it grows
//! (`n: (.a | length)`) moves the slot out of the state and extends it in place, as the one-step
//! slot of #6538 does. Every expected string was captured from jq 1.7.1 (`/usr/bin/jq`).

use anyhow::Result;

use crate::run_jq_full;

/// Elements of every kind, so a slot grows by numbers, a string, `null` and an object.
const INPUT: &str = r#"[1, 2.5, "x", null, {"k": 1}]"#;

#[test]
fn nested_chained_and_counted_slot_steps_answer_as_jq_6549() -> Result<()> {
    for (filter, want, want_code) in [
        (
            r"reduce .[] as $i ({a: {b: []}}; {a: {b: (.a.b + [$i])}})",
            "{\"a\":{\"b\":[1,2.5,\"x\",null,{\"k\":1}]}}\n",
            0,
        ),
        (
            r"reduce .[] as $i ({g: {l: [], n: 0}}; {g: {l: (.g.l + [$i]), n: (.g.n + 1)}, t: (.g.l | length)})",
            "{\"g\":{\"l\":[1,2.5,\"x\",null,{\"k\":1}],\"n\":5},\"t\":4}\n",
            0,
        ),
        (
            r"reduce .[] as $i ({}; {a: {b: (.a.b + [$i])}})",
            "{\"a\":{\"b\":[1,2.5,\"x\",null,{\"k\":1}]}}\n",
            0,
        ),
        (
            r"reduce .[] as $i ({a: [[1], {}]}; {a: [(.a[0] + [$i]), (.a[1] + {k: 0})]})",
            "{\"a\":[[1,1,2.5,\"x\",null,{\"k\":1}],{\"k\":0}]}\n",
            0,
        ),
        (
            r"reduce .[] as $i ({a: [[1], {}]}; {x: .a, y: (.a[0] + [$i])})",
            "{\"x\":null,\"y\":[{\"k\":1}]}\n",
            0,
        ),
        (
            r"reduce .[] as $i ({a: {b: []}}; {a: {b: (.a.b + [$i]), b: [9]}})",
            "{\"a\":{\"b\":[9]}}\n",
            0,
        ),
        (
            r"reduce .[] as $i ({a: []}; {a: (.a + [$i] + [0])})",
            "{\"a\":[1,0,2.5,0,\"x\",0,null,0,{\"k\":1},0]}\n",
            0,
        ),
        (
            r"reduce .[] as $i ({a: []}; {a: (.a + [$i] + null + [1])})",
            "{\"a\":[1,1,2.5,1,\"x\",1,null,1,{\"k\":1},1]}\n",
            0,
        ),
        (
            r"reduce .[] as $i ({a: []}; {a: ((.a // []) + [$i] + [2])})",
            "{\"a\":[1,2,2.5,2,\"x\",2,null,2,{\"k\":1},2]}\n",
            0,
        ),
        (
            r#"reduce .[] as $i ({s: ""}; {s: (.s + "a" + "b")})"#,
            "{\"s\":\"ababababab\"}\n",
            0,
        ),
        (
            r"reduce .[] as $i ({o: {}}; {o: (.o + {x: 1} + {x: 2, y: 3})})",
            "{\"o\":{\"x\":2,\"y\":3}}\n",
            0,
        ),
        (
            r#"try reduce .[] as $i ({a: []}; {a: (.a + [$i] + "x")}) catch ."#,
            "\"array ([1]) and string (\\\"x\\\") cannot be added\"\n",
            0,
        ),
        (
            r"reduce .[] as $i ({a: [], n: 0}; {a: (.a + [$i]), n: (.a | length)})",
            "{\"a\":[1,2.5,\"x\",null,{\"k\":1}],\"n\":4}\n",
            0,
        ),
        (
            r#"reduce .[] as $i ({a: "", n: 0}; {a: (.a + "xy"), n: (.a | length)})"#,
            "{\"a\":\"xyxyxyxyxy\",\"n\":8}\n",
            0,
        ),
        (
            r"reduce .[] as $i ({a: null, n: 0}; {a: (.a + [1]), n: (.a | length)})",
            "{\"a\":[1,1,1,1,1],\"n\":4}\n",
            0,
        ),
        (r"reduce .[] as $i ({x: []}; [length, (.x + [$i])])", "", 5),
        (
            r"try reduce .[] as $i ({a: true}; {n: (.a | length)}) catch .",
            "\"boolean (true) has no length\"\n",
            0,
        ),
        (
            r"[foreach .[] as $i ({a: {b: []}}; {a: {b: (.a.b + [$i])}}; .a.b | length)]",
            "[1,2,3,4,5]\n",
            0,
        ),
        (
            r"{a: {b: [7]}} as $s | reduce .[] as $i ($s; {a: {b: (.a.b + [$i])}}) | ., $s",
            "{\"a\":{\"b\":[7,1,2.5,\"x\",null,{\"k\":1}]}}\n{\"a\":{\"b\":[7]}}\n",
            0,
        ),
    ] {
        let (stdout, _, code) = run_jq_full(&["-c", filter], Some(INPUT))?;
        assert_eq!((stdout.as_str(), code), (want, want_code), "{filter}");
    }
    Ok(())
}

/// 20,000 steps of a nested, chained, counted slot re-indexed the whole state every step before
/// (about 6 s); the slot is extended in place now. The answer is jq 1.7.1's.
#[test]
fn a_long_nested_slot_constructor_fold_answers_6549() -> Result<()> {
    let filter = "reduce range(20000) as $i ({g: {l: [], n: 0}}; {g: {l: (.g.l + [$i] + [0]), n: (.g.l | length)}}) | [.g.n, (.g.l | add)]";
    let (stdout, _, code) = run_jq_full(&["-nc", filter], None)?;
    assert_eq!((stdout.as_str(), code), ("[39998,199990000]\n", 0));
    Ok(())
}
