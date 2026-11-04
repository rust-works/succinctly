//! #6134: yq's Go `regexp` factors only a simple prefix out of an alternation, so `a*a|a*b` keeps
//! its first branch's match there too; `regex-syntax` factored the `a*` out and the second branch
//! won. The expectations are live yq v4.53.3 captures.

use anyhow::Result;

use crate::run_yq_stdin;

#[test]
fn test_yq_takes_the_first_branch_of_a_shared_prefix_alternation_6134() -> Result<()> {
    for (filter, want) in [
        (r#".s | [match("a*a|a*b") | .string]"#, r#"["aa"]"#),
        (r#".s | sub("a*a|a*b"; "X")"#, r#""XX""#),
        (r#".t | [match("ab*b|ab*c") | .string]"#, r#"["abb"]"#),
        (r#".u | [match("a*?b|a*?a") | .string]"#, r#"["ab"]"#),
        (r#".s | [match("(?:a*a|a*b)+?") | .string]"#, r#"["aa"]"#),
    ] {
        let (out, code) = run_yq_stdin(filter, "s: aab\nt: abbc\nu: ab\n", &["-o=json", "-I=0"])?;
        assert_eq!((out.trim_end(), code), (want, 0), "{filter}");
    }
    Ok(())
}
