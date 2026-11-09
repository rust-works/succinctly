//! #6716: the first write of a chain reads its right side from the document as it came in, so a
//! target that replaces the node it reads (`.b = [.b]` on `b: *x`) still finds the source's alias.
//! Every expected output is what yq prints for the same input and filter.

use anyhow::Result;

use crate::run_yq_stdin;

#[test]
fn test_a_write_over_the_node_it_reads_keeps_the_alias_6716() -> Result<()> {
    const B: &str = "a: &x 1\nb: *x\nc: 0\n";
    for (input, filter, expected) in [
        (B, ".b = [.b]", "a: &x 1\nb:\n  - *x\nc: 0\n"),
        (B, ".b = {\"k\": .b}", "a: &x 1\nb:\n  k: *x\nc: 0\n"),
        (B, ".b = [.b, .b]", "a: &x 1\nb:\n  - *x\n  - *x\nc: 0\n"),
        (B, ".b = .b", "a: &x 1\nb: *x\nc: 0\n"),
        ("a: &x 1\nb: [0, *x]\n", ".b = .b[-1]", "a: &x 1\nb: *x\n"),
        // A later stage reads what the earlier one wrote.
        (B, ".c = .b | .b = [.c]", "a: &x 1\nb:\n  - *x\nc: *x\n"),
    ] {
        let (output, exit_code) = run_yq_stdin(filter, input, &[])?;
        assert_eq!(exit_code, 0, "filter: {filter} on {input:?}");
        assert_eq!(output, expected, "filter: {filter} on {input:?}");
    }
    Ok(())
}
