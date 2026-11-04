//! #6106: jq mode ends a greedy loop at an empty iteration, as Oniguruma does. yq's Go `regexp`
//! does not, so yq mode keeps the longer match. The expectations are live yq v4.53.3 captures.

use anyhow::Result;

use crate::run_yq_stdin;

#[test]
fn test_yq_keeps_the_longer_match_of_a_loop_with_an_empty_way_6106() -> Result<()> {
    for (filter, want) in [
        (r#".s | [match("(?:b*|a)+") | .string]"#, r#"["ba"]"#),
        (r#".s | sub("(?:b*|a)+"; "X")"#, r#""X""#),
        (r#".t | [match("(?:b*|a)+") | .string]"#, r#"["bab"]"#),
    ] {
        let (out, code) = run_yq_stdin(filter, "s: ba\nt: bab\n", &["-o=json", "-I=0"])?;
        assert_eq!((out.trim_end(), code), (want, 0), "{filter}");
    }
    Ok(())
}
