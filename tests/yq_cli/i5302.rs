//! #5302: the U+0000 -> `\0` rewrite is jq-mode only. Real yq passes the NUL through raw (checked
//! live: `.a | @sh`, `[.a] | @csv` and `[.a] | @tsv` all keep the byte), so `succinctly yq` does too.

use anyhow::Result;

use crate::run_yq_stdin;

#[test]
fn test_yq_formats_keep_raw_nul_5302() -> Result<()> {
    for filter in [".a | @sh", "[.a] | @csv", "[.a] | @tsv", ".a | @html"] {
        // `@html` is not a yq token, so it needs the flag (#5444); the rest are yq's own.
        let (out, code) = run_yq_stdin(filter, "a: \"x\\0y\"\n", &["--jq-extensions"])?;
        assert_eq!(code, 0, "{filter}");
        assert!(out.contains('\0'), "{filter}: {out:?}");
        assert!(!out.contains("\\0"), "{filter}: {out:?}");
    }
    Ok(())
}
