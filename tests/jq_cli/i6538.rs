//! #6538: a `reduce`/`foreach` step that rebuilds its state with a constructor whose members read
//! a container slot of the old state (`{a: (.a + [$i]), n: (.n + 1)}`) moves that slot out of the
//! state and extends it in place, instead of re-indexing the whole state every step. Every
//! expected string was captured from jq 1.7.1 (`/usr/bin/jq`).

use anyhow::Result;

use crate::run_jq_full;

/// Elements of every kind, so a slot grows by numbers, a string, `null` and an object.
const INPUT: &str = r#"[1, 2.5, "x", null, {"k": 1}]"#;

#[test]
fn container_slot_constructor_steps_answer_as_jq_6538() -> Result<()> {
    for (filter, want, want_code) in [
        (r"reduce .[] as $i ({a: [], n: 0}; {a: (.a + [$i]), n: (.n + 1)})", "{\"a\":[1,2.5,\"x\",null,{\"k\":1}],\"n\":5}\n", 0),
        (r"reduce .[] as $i ({a: [], b: []}; {a: (.a + [$i]), b: (.b + [$i, $i])})", "{\"a\":[1,2.5,\"x\",null,{\"k\":1}],\"b\":[1,1,2.5,2.5,\"x\",\"x\",null,null,{\"k\":1},{\"k\":1}]}\n", 0),
        (r"reduce .[] as $i ({a: [], b: []}; {b: .a, a: (.b + [$i])})", "{\"b\":[2.5,null],\"a\":[1,\"x\",{\"k\":1}]}\n", 0),
        (r"reduce .[] as $i ({a: [], b: []}; {a: .a, b: .a})", "{\"a\":[],\"b\":[]}\n", 0),
        (r"reduce .[] as $i ({a: []}; {n: (.a | length), a: (.a + [$i])})", "{\"n\":4,\"a\":[1,2.5,\"x\",null,{\"k\":1}]}\n", 0),
        (r"reduce .[] as $i ({}; {a: (.a + [$i])})", "{\"a\":[1,2.5,\"x\",null,{\"k\":1}]}\n", 0),
        (r"reduce .[] as $i (null; {a: (.a + [$i])})", "{\"a\":[1,2.5,\"x\",null,{\"k\":1}]}\n", 0),
        (r#"reduce .[] as $i ({s: ""}; {s: (.s + ($i | tostring))})"#, "{\"s\":\"12.5xnull{\\\"k\\\":1}\"}\n", 0),
        (r"reduce .[] as $i ({o: {}}; {o: (.o + {($i | tojson): $i})})", "{\"o\":{\"1\":1,\"2.5\":2.5,\"\\\"x\\\"\":\"x\",\"null\":null,\"{\\\"k\\\":1}\":{\"k\":1}}}\n", 0),
        (r"try reduce .[] as $i ({a: 1}; {a: (.a + [$i])}) catch .", "\"number (1) and array ([1]) cannot be added\"\n", 0),
        (r"reduce .[] as $i ({a: [1]}; {a: (.a + .a)})", "{\"a\":[1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1]}\n", 0),
        (r"reduce .[] as $i ([[], 0]; [.[0] + [$i], .[1] + 1])", "[[1,2.5,\"x\",null,{\"k\":1}],5]\n", 0),
        (r"reduce .[] as $i ([[], []]; [.[1], .[0] + [$i]])", "[[2.5,null],[1,\"x\",{\"k\":1}]]\n", 0),
        (r"reduce .[] as $i ([[]]; [.[1] + [$i]])", "[[{\"k\":1}]]\n", 0),
        (r"reduce .[] as $i ([[]]; [.[-1] + [$i]])", "[[1,2.5,\"x\",null,{\"k\":1}]]\n", 0),
        (r"reduce .[] as $i ({a: []}; {a: (.a + [$i]), a: (.a + [0])})", "{\"a\":[0,0,0,0,0]}\n", 0),
        (r"reduce .[] as $i ({a: []}; {a: 1, a: (.a + [$i])})", "{\"a\":[1,2.5,\"x\",null,{\"k\":1}]}\n", 0),
        (r#"reduce .[] as $i ({a: [], k: "a"}; {(.k): (.a + [$i]), k: .k})"#, "{\"a\":[1,2.5,\"x\",null,{\"k\":1}],\"k\":\"a\"}\n", 0),
        (r"[foreach .[] as $i ({a: []}; {a: (.a + [$i])}; .a | length)]", "[1,2,3,4,5]\n", 0),
        (r"{a: [9]} as $s | reduce .[] as $i ($s; {a: (.a + [$i])}) | ., $s", "{\"a\":[9,1,2.5,\"x\",null,{\"k\":1}]}\n{\"a\":[9]}\n", 0),
        (r"[0] as $s | reduce .[] as $i ({a: $s}; {a: (.a + [$i])}) | ., $s", "{\"a\":[0,1,2.5,\"x\",null,{\"k\":1}]}\n[0]\n", 0),
        (r#"try reduce .[] as $i ({a: []}; {a: (.a + [$i]), b: error("x")}) catch ."#, "\"x\"\n", 0),
    ] {
        let (stdout, _, code) = run_jq_full(&["-c", filter], Some(INPUT))?;
        assert_eq!((stdout.as_str(), code), (want, want_code), "{filter}");
    }
    Ok(())
}

/// 20,000 steps re-indexed a state holding up to 20,000 elements on every step before (minutes);
/// the slot is extended in place now. The answer is jq 1.7.1's.
#[test]
fn a_long_container_slot_constructor_fold_answers_6538() -> Result<()> {
    let filter = "reduce range(20000) as $i ({a: [], n: 0}; {a: (.a + [$i]), n: (.n + 1)}) | [.n, (.a | add)]";
    let (stdout, _, code) = run_jq_full(&["-nc", filter], None)?;
    assert_eq!((stdout.as_str(), code), ("[20000,199990000]\n", 0));
    Ok(())
}
