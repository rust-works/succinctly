//! #6618: `--eval-all` over JSON reads a document nested 256 containers deep, as yq does. The
//! owned-value walk it reads through held every node to the 256-level ceiling, the scalar inside the
//! 256th container included, and refused `[[…[1]…]]` with 256 arrays as `nesting depth exceeds limit
//! of 256`. Both rows were captured from yq v4.53.3 (`yq -p=json -o=json -I0 -N ea '.|length'`); yq has
//! no depth limit, so what it answers at 257 containers and deeper is recorded in
//! `docs/compliance/yq/limitations.md`, not pinned here.

use anyhow::Result;

use crate::run_yq_stdin;

#[test]
fn eval_all_reads_a_json_document_at_the_nesting_ceiling_6618() -> Result<()> {
    for containers in [255usize, 256] {
        let doc = format!("{}1{}\n", "[".repeat(containers), "]".repeat(containers));
        let (out, code) = run_yq_stdin(
            ".|length",
            &doc,
            &["--eval-all", "-p=json", "-o=json", "-I0"],
        )?;
        assert_eq!((out.as_str(), code), ("1\n", 0), "{containers} containers");
    }
    Ok(())
}
