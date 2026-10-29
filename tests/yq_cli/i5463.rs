//! #5463: a file whose size is a multiple of 64 and whose last node is a null made every DOM
//! route fail with `invalid cursor position`: the null's sentinel position is the text length,
//! and the interest-bit vector had no word for it at a 64-byte boundary. Expected output follows
//! yq v4.53.3 (a write appends the key and leaves the trailing comment where it was).

use anyhow::Result;

use crate::run_yq_stdin;

/// `a: 1\nc: # xyyy...\n`, `size` bytes in all.
fn null_tail_with_comment(size: usize) -> String {
    let head = "a: 1\nc: # x";
    format!("{head}{}\n", "y".repeat(size - head.len() - 1))
}

#[test]
fn test_write_across_a_64_byte_boundary_5463() -> Result<()> {
    for size in 56..=200 {
        let input = null_tail_with_comment(size);
        assert_eq!(input.len(), size);
        let (out, code) = run_yq_stdin(".zzkey = 1", &input, &[])?;
        assert_eq!(code, 0, "size {size}");
        assert_eq!(out, format!("{input}zzkey: 1\n"), "size {size}");
    }
    Ok(())
}

#[test]
fn test_reads_across_a_64_byte_boundary_5463() -> Result<()> {
    for size in [63, 64, 65, 127, 128, 129] {
        let input = null_tail_with_comment(size);
        let (out, code) = run_yq_stdin("length", &input, &[])?;
        assert_eq!((out.as_str(), code), ("2\n", 0), "size {size}");
        let (out, code) = run_yq_stdin(".c", &input, &["-o=json"])?;
        assert_eq!((out.as_str(), code), ("null\n", 0), "size {size}");
    }
    Ok(())
}

#[test]
fn test_comment_only_document_across_a_64_byte_boundary_5463() -> Result<()> {
    for size in [63, 64, 65, 128] {
        let input = format!("#{}\n", " ".repeat(size - 2));
        let (_, code) = run_yq_stdin("length", &input, &[])?;
        assert_eq!(code, 0, "size {size}");
    }
    Ok(())
}

#[test]
fn test_alias_and_bare_null_tails_across_a_64_byte_boundary_5463() -> Result<()> {
    for size in [63, 64, 65, 128] {
        // `c:` with no comment, then padding as a trailing comment line.
        let alias = format!("a: &x\nb: *x\nc: #{}\n", "y".repeat(size - 17));
        assert_eq!(alias.len(), size);
        let (out, code) = run_yq_stdin(".zzkey = 1", &alias, &[])?;
        assert_eq!(code, 0, "alias, size {size}");
        assert!(out.ends_with("zzkey: 1\n"), "alias, size {size}: {out}");
        let bare = format!("a: 1\nc:\n#{}\n", "y".repeat(size - 10));
        assert_eq!(bare.len(), size);
        let (out, code) = run_yq_stdin(".zzkey = 1", &bare, &[])?;
        assert_eq!(code, 0, "bare, size {size}");
        assert!(out.ends_with("zzkey: 1\n"), "bare, size {size}: {out}");
    }
    Ok(())
}
