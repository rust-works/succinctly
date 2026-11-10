//! #6833: a call to a `def` with no parameters whose body is a string's `length` is answered from
//! the string in a fold step, as the same stage written inline is. yq's lexer rejects `def`, so
//! under `--jq-extensions` the oracle is the inline stage, which is yq v4.53.3's answer.
//! `tests/jq_cli/i6833.rs` has the jq-mode rows against jq 1.7.1.

use anyhow::Result;

use crate::run_yq_stdin_with_stderr;

#[test]
fn test_6833_a_def_of_length_in_a_fold_step_answers_as_the_inline_stage_does() -> Result<()> {
    let tail = r#"("h" + "éllo") as $t | [1,2,3][] as $i ireduce(0; . + ($t | "#;
    let inline = format!("{tail}length))");
    let called = format!("def f: length; {tail}f))");
    for program in [&inline, &called] {
        let (out, err, code) = run_yq_stdin_with_stderr(
            program,
            "null\n",
            &["--jq-extensions", "-n", "-o=json", "-I=0"],
        )?;
        assert_eq!(
            (out.as_str(), err.as_str(), code),
            ("18\n", "", 0),
            "{program}"
        );
    }
    Ok(())
}
