//! #6356: `.a.a... | path` past 384 components answers as yq does, where the path-context walk
//! stopped at the constant. The first filter is a live yq v4.53.3 capture; `path(f)` is gated behind
//! `--jq-extensions` and has no yq row, and is pinned to agree with it.

use anyhow::Result;

use crate::run_yq_stdin;

#[test]
fn a_static_chain_past_384_components_answers_as_yq_6356() -> Result<()> {
    for components in [385, 1000] {
        let chain = ".a".repeat(components);
        let want = format!("{components}\n");
        assert_eq!(
            run_yq_stdin(&format!("{chain} | path | length"), "{}", &["-o=json"])?,
            (want.clone(), 0)
        );
        assert_eq!(
            run_yq_stdin(
                &format!("path({chain}) | length"),
                "{}",
                &["-o=json", "--jq-extensions"]
            )?,
            (want, 0)
        );
    }
    Ok(())
}
