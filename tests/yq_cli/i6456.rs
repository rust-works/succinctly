//! #6456: under `(?m)`, jq mode reads `^` as Oniguruma does (it does not hold at the end of a text that
//! ends with a newline). `succinctly yq` follows Go, whose `(?m)^` does hold there: yq prints `[0,2]`
//! for `"a\n"` (captured from yq v4.53.3), and the rewrite for jq's regex engine leaves it alone.

use anyhow::Result;

use crate::run_yq_stdin;

#[test]
fn yq_keeps_go_s_multiline_caret_at_the_end_of_the_text_6456() -> Result<()> {
    let (stdout, status) = run_yq_stdin(
        r#"[match("(?m)^";"g") | .offset]"#,
        "\"a\\n\"\n",
        &["-o=json", "-I0"],
    )?;
    assert_eq!((stdout.trim_end(), status), ("[0,2]", 0));
    Ok(())
}
