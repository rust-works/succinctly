//! #6692: the pipes of `eval.rs` answer a slice and the read after it (`.[a:b][k]`, `| length`) and
//! a second slice (`.[a:b][c:d]`) from the slice's resolved range, as the generic evaluator's do.
//! yq mode shares those pipes, and its slice rules are its own: an index below a slice raises, and
//! `last` is not a builtin. The first block is the real yq v4.53.3's answers over the document
//! below, `-p json -o json -I0`. The second block needs `--jq-extensions` (yq has no `range`), so it
//! has no oracle: it is the build's answer before #6692, which the fusion must not move.

use anyhow::Result;

use crate::run_yq_stdin;

#[test]
fn a_fused_slice_read_answers_as_yq_does_6692() -> Result<()> {
    for (input, program, stdout, status) in [
        ("[10,20,30,40]", ".[1:][0]", "20\n", 0),
        ("[10,20,30,40]", ".[-3:][-1]", "40\n", 0),
        ("[10,20,30,40]", ".[2:] | length", "2\n", 0),
        ("[10,20,30,40]", ".[-3:][-4]", "", 1),
        ("[10,20,30,40]", ".[1:][5]", "null\n", 0),
        ("[10,20,30,40]", ".[1:][:2]", "[20,30]\n", 0),
        ("[10,20,30,40]", ".[2:][:3]", "[30,40]\n", 0),
        ("[10,20,30,40]", ".[1:][1:]", "[30,40]\n", 0),
        ("[10,20,30,40]", ".[100:][0]", "null\n", 0),
        ("[10,20,30,40]", ".[:2][-1]", "20\n", 0),
        ("[10,20,30,40]", ".[1:] | last", "", 1),
        ("\"hello\"", ".[1:][:2]", "\"el\"\n", 0),
        ("null", ".[1:][0]", "null\n", 0),
        ("{\"a\":1}", ".[1:][0]", "null\n", 0),
    ] {
        let (out, code) = run_yq_stdin(program, input, &["-p", "json", "-o", "json", "-I0"])?;
        assert_eq!(out, stdout, "{input} | {program}");
        assert_eq!(code, status, "{input} | {program}");
    }
    Ok(())
}

#[test]
fn a_loop_over_a_moving_tail_in_yq_mode_keeps_its_answers_6692() -> Result<()> {
    let input = "[10,20,30,40]";
    for (program, stdout, status) in [
        ("[range(0;3) as $i | .[$i:][0]]", "[10,20,30]\n", 0),
        ("[range(0;3) as $i | .[$i:] | length]", "[4,3,2]\n", 0),
        (
            "[range(0;3) as $i | .[$i:][:2]]",
            "[[10,20],[20,30],[30,40]]\n",
            0,
        ),
        ("[range(0;3) as $i | .[$i:][-1]]", "[40,40,40]\n", 0),
        ("[range(0;5) as $i | .[$i:][-3]?]", "", 1),
        ("[range(0;3) as $i | .[$i:][-9]]", "", 1),
        ("[range(0;3) as $i | .[$i:] | first]", "[10,20,30]\n", 0),
        ("[range(0;4) as $i | first(.[$i:][0])]", "[]\n", 0),
    ] {
        let args = ["--jq-extensions", "-p", "json", "-o", "json", "-I0"];
        let (out, code) = run_yq_stdin(program, input, &args)?;
        assert_eq!(out, stdout, "{program}");
        assert_eq!(code, status, "{program}");
    }
    Ok(())
}
