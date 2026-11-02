//! #5875: a fold whose UPDATE builds a container around the accumulator (`[.]`, `[., <closed>…]`,
//! `{k: .}`) is linear in the number of steps. It used to re-index the whole accumulator on every
//! step, so a fold that nested past 384 levels (#5832) was quadratic where jq's is linear. Every
//! expected string was captured from jq 1.7.1 (`/usr/bin/jq`), and the shapes that decline the
//! owned step (a second `.`, a member that is not closed) are in the table so the evaluator's own
//! answer stays pinned beside it.

use anyhow::Result;

use crate::run_jq_full;

#[test]
fn constructor_folds_answer_as_jq_5875() -> Result<()> {
    for (filter, input, want, want_code) in [
        (r"reduce range(5) as $i (0; [.])", None, "[[[[[0]]]]]\n", 0),
        (r"reduce range(5) as $i (0; [., $i])", None, "[[[[[0,0],1],2],3],4]\n", 0),
        (r"reduce range(4) as $i (0; [$i, .])", None, "[3,[2,[1,[0,0]]]]\n", 0),
        (r#"reduce range(4) as $i (0; [$i, ., "x", null, true, {"a":[1]}])"#, None, "[3,[2,[1,[0,0,\"x\",null,true,{\"a\":[1]}],\"x\",null,true,{\"a\":[1]}],\"x\",null,true,{\"a\":[1]}],\"x\",null,true,{\"a\":[1]}]\n", 0),
        (r"reduce range(4) as $i (0; {a: .})", None, "{\"a\":{\"a\":{\"a\":{\"a\":0}}}}\n", 0),
        (r"reduce range(4) as $i (0; {a: ., b: $i})", None, "{\"a\":{\"a\":{\"a\":{\"a\":0,\"b\":0},\"b\":1},\"b\":2},\"b\":3}\n", 0),
        (r"reduce range(4) as $i (0; {($i|tostring): .})", None, "{\"3\":{\"2\":{\"1\":{\"0\":0}}}}\n", 0),
        (r"reduce range(4) as $i (0; {b: 1, a: ., b: 3})", None, "{\"b\":3,\"a\":{\"b\":3,\"a\":{\"b\":3,\"a\":{\"b\":3,\"a\":0}}}}\n", 0),
        (r"reduce range(4) as $i (0; {a: ., a: 2})", None, "{\"a\":2}\n", 0),
        (r"[foreach range(4) as $i (0; [., $i]; length)]", None, "[2,2,2,2]\n", 0),
        (r"[foreach range(3) as $i (0; {a: .}; .)]", None, "[{\"a\":0},{\"a\":{\"a\":0}},{\"a\":{\"a\":{\"a\":0}}}]\n", 0),
        (r"0 | until(tojson|length > 12; [.])", None, "[[[[[[0]]]]]]\n", 0),
        (r"reduce range(2) as $i (nan; [.])", None, "[[null]]\n", 0),
        (r"reduce range(2) as $i (100000000000000000000000; [.])", None, "[[100000000000000000000000]]\n", 0),
        (r"reduce range(2) as $i (-0; [.])", None, "[[-0]]\n", 0),
        (r"reduce range(1) as $i (1.10; {a: .})", None, "{\"a\":1.10}\n", 0),
        (r#"reduce range(3) as $i (0; [., error("x")])"#, None, "", 5),
        (r"reduce range(3) as $i (0; [., empty])", None, "[[[0]]]\n", 0),
        (r"reduce range(3) as $i (0; [., .])", None, "[[[0,0],[0,0]],[[0,0],[0,0]]]\n", 0),
        (r"reduce range(3) as $i (0; {a: (., .)})", None, "{\"a\":{\"a\":{\"a\":0}}}\n", 0),
        (r"reduce range(3) as $i (0; {(1): .})", None, "", 3),
        (r"reduce .[] as $x (0; [., $x])", Some(r#"[{"a":1},[2],3]"#), "[[[0,{\"a\":1}],[2]],3]\n", 0),
        (r"reduce .[] as $x (null; {k: ., v: $x})", Some(r#"[{"a":1},[2],3]"#), "{\"k\":{\"k\":{\"k\":null,\"v\":{\"a\":1}},\"v\":[2]},\"v\":3}\n", 0),
        (r"reduce .[] as $x ({}; {p: ., v: $x}) | [paths]", Some(r#"[{"a":1},[2]]"#), "[[\"p\"],[\"p\",\"p\"],[\"p\",\"v\"],[\"p\",\"v\",\"a\"],[\"v\"],[\"v\",0]]\n", 0),
        (r". as $d | reduce range(3) as $i (0; {a: ., b: $d})", Some(r#"{"q":[1,2]}"#), "{\"a\":{\"a\":{\"a\":0,\"b\":{\"q\":[1,2]}},\"b\":{\"q\":[1,2]}},\"b\":{\"q\":[1,2]}}\n", 0),
        (r"reduce range(300) as $i (0; [.]) | flatten", None, "[0]\n", 0),
        (r"reduce range(300) as $i (0; {a: .}) | [paths] | length", None, "300\n", 0),
        (r"reduce range(3) as $i (0; [.]) | path(..)", None, "[]\n[0]\n[0,0]\n[0,0,0]\n", 0),
    ] {
        let args: &[&str] = if input.is_some() {
            &["-c", filter]
        } else {
            &["-nc", filter]
        };
        let (stdout, _, code) = run_jq_full(args, input)?;
        assert_eq!((stdout.as_str(), code), (want, want_code), "{filter}");
    }
    Ok(())
}

/// 300,000 levels is past where jq itself crashes on this shape (it answers at 100,000 and
/// segfaults by 300,000), so the pin is that the fold answers, quickly, and drops what it built
/// without overflowing the stack (#5782). At the old quadratic cost this took minutes in a debug
/// build.
#[test]
fn a_constructor_fold_past_the_old_quadratic_wall_answers_5875() -> Result<()> {
    for (filter, want) in [
        ("reduce range(300000) as $i (0; [.]) | length", "1\n"),
        ("reduce range(300000) as $i (0; [., $i]) | length", "2\n"),
        ("reduce range(300000) as $i (0; {a: .}) | length", "1\n"),
        ("last(foreach range(300000) as $i (0; [.]; 0))", "0\n"),
    ] {
        let (stdout, stderr, code) = run_jq_full(&["-nc", filter], None)?;
        assert_eq!(
            (stdout.as_str(), stderr.as_str(), code),
            (want, "", 0),
            "{filter}"
        );
    }
    Ok(())
}
