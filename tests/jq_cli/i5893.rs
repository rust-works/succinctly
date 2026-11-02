//! #5893: a constructor fold step that reads a scalar off the accumulator (`[., length]`,
//! `{a: ., n: length}`, `{(.s): .}`) is built on the moved accumulator like `[.]` is (#5875), so a
//! deep fold of it is linear. A member that reads a container out of the accumulator (`[., .a]`), a
//! float or `tojson` is answered from a rebuilt copy of it: right, and still quadratic. Every expected string was captured from jq 1.7.1
//! (`/usr/bin/jq`).

use anyhow::Result;

use crate::run_jq_full;

#[test]
fn state_reading_members_answer_as_jq_5893() -> Result<()> {
    for (filter, want, want_code) in [
        (r"reduce range(5) as $i (0; [., length])", "[[[[[0,0],2],2],2],2]\n", 0),
        (r"reduce range(5) as $i ([]; [., length, ($i|tostring)])", "[[[[[[],0,\"0\"],3,\"1\"],3,\"2\"],3,\"3\"],3,\"4\"]\n", 0),
        (r"reduce range(5) as $i ([]; [length, .])", "[2,[2,[2,[2,[0,[]]]]]]\n", 0),
        (r"reduce range(5) as $i (0; {a: ., n: length})", "{\"a\":{\"a\":{\"a\":{\"a\":{\"a\":0,\"n\":0},\"n\":2},\"n\":2},\"n\":2},\"n\":2}\n", 0),
        (r#"reduce range(4) as $i ({"n":1}; {a: ., n: (.n + 1)})"#, "{\"a\":{\"a\":{\"a\":{\"a\":{\"n\":1},\"n\":2},\"n\":3},\"n\":4},\"n\":5}\n", 0),
        (r#"reduce range(4) as $i ({"s":"k"}; {(.s): .})"#, "", 5),
        (r#"reduce range(4) as $i ({"n":1.10}; {a: ., b: .n})"#, "{\"a\":{\"a\":{\"a\":{\"a\":{\"n\":1.10},\"b\":1.10},\"b\":null},\"b\":null},\"b\":null}\n", 0),
        (r"reduce range(3) as $i ([1,2,3]; [., .[0], length])", "[[[[1,2,3],1,3],[1,2,3],3],[[1,2,3],1,3],3]\n", 0),
        (r#"reduce range(3) as $i ({"a":[1]}; [., .a])"#, "", 5),
        (r#"reduce range(3) as $i (0; [., (.|error("x"))])"#, "", 5),
        (r"reduce range(3) as $i (0; [., (.|tojson)])", "[[[0,\"0\"],\"[0,\\\"0\\\"]\"],\"[[0,\\\"0\\\"],\\\"[0,\\\\\\\"0\\\\\\\"]\\\"]\"]\n", 0),
        (r"[foreach range(3) as $i (0; [., length]; .)]", "[[0,0],[[0,0],2],[[[0,0],2],2]]\n", 0),
        (r"0 | until(tojson|length > 20; [., length])", "[[[[[0,0],2],2],2],2]\n", 0),
        (r"reduce range(300) as $i (0; {a: ., n: length}) | [paths]|length", "600\n", 0),
    ] {
        let (stdout, _, code) = run_jq_full(&["-nc", filter], None)?;
        assert_eq!((stdout.as_str(), code), (want, want_code), "{filter}");
    }
    Ok(())
}

/// 300,000 levels was a half-minute at 40,000 before (`[., (.|type)]` re-indexed the whole
/// accumulator every step) and is past where jq itself crashes.
#[test]
fn a_state_reading_constructor_fold_past_the_old_quadratic_wall_answers_5893() -> Result<()> {
    for (filter, want) in [
        (
            "reduce range(300000) as $i (0; [., length]) | length",
            "2\n",
        ),
        (
            "reduce range(300000) as $i (0; [., (.|type)]) | length",
            "2\n",
        ),
        (
            "reduce range(300000) as $i (0; {a: ., n: length}) | .n",
            "2\n",
        ),
        (
            r#"reduce range(300000) as $i ({"n":1}; {p: ., n: (.n + 1)}) | .n"#,
            "300001\n",
        ),
        (
            r#"reduce range(300000) as $i ({"s":"k"}; {(.s): ., s: .s}) | .s"#,
            "\"k\"\n",
        ),
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
