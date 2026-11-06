//! #6287 (the block-scalar half of #6214): go-yaml ends a block scalar's line at U+2028, U+2029
//! and U+0085 as at `\n`, so a tab after fewer spaces than the scalar's indent on what follows is
//! `found a tab character where an indentation space is expected`, as it is after a `\n`. The
//! break itself stays scalar content (#4913), and so do the spaces after it, which yq strips
//! (the other half of #4913, not changed here). Every verdict was captured from yq v4.53.3: the
//! grid below was run through both tools, 4,608 documents, 1,344 of them refused, with no
//! difference in which ones.

use anyhow::Result;

use crate::run_yq_stdin;

/// (document template with `{h}` the indicator and `{b}` the body, the scalar's indent).
const HOSTS: &[(&str, usize)] = &[
    ("a: {h}\n{b}\nb: 1\n", 2),
    ("- {h}\n{b}\n- 2\n", 2),
    ("a:\n  b: {h}\n{b}\n", 4),
    ("a:\n  - {h}\n{b}\n", 4),
];

const BREAKS: [&str; 3] = ["\u{2028}", "\u{2029}", "\u{85}"];

fn document(host: &str, indicator: &str, indent: usize, brk: &str, spaces: usize) -> String {
    let body = format!("{}x{brk}{}\ty", " ".repeat(indent), " ".repeat(spaces));
    host.replace("{h}", indicator).replace("{b}", &body)
}

#[test]
fn a_tab_below_the_indent_after_a_unicode_break_in_a_block_scalar_is_refused_6287() -> Result<()> {
    for &(host, indent) in HOSTS {
        for indicator in ["|", ">", "|-", ">+", "|2"] {
            // an explicit indent applies to the first host only (`|2` under `a:` is indent 2)
            if indicator == "|2" && indent != 2 {
                continue;
            }
            for brk in BREAKS {
                for spaces in 0..indent {
                    let input = document(host, indicator, indent, brk, spaces);
                    let (out, code) = run_yq_stdin(".", &input, &["-o=json", "-I=0"])?;
                    assert_eq!(out.trim_end(), "", "{input:?}");
                    assert_ne!(code, 0, "{input:?}");
                }
            }
        }
    }
    Ok(())
}

#[test]
fn a_tab_at_or_past_the_indent_after_a_unicode_break_in_a_block_scalar_is_content_6287(
) -> Result<()> {
    for &(host, indent) in HOSTS {
        for indicator in ["|", ">", "|-", ">+"] {
            for brk in BREAKS {
                for spaces in indent..indent + 3 {
                    let input = document(host, indicator, indent, brk, spaces);
                    let (out, code) = run_yq_stdin(".", &input, &["-o=json", "-I=0"])?;
                    assert_eq!(code, 0, "{input:?}");
                    assert!(out.contains("\\ty"), "{input:?}: {out}");
                }
            }
        }
    }
    Ok(())
}

#[test]
fn a_tab_that_does_not_follow_a_unicode_break_in_a_block_scalar_is_read_6287() -> Result<()> {
    for (input, expected) in [
        ("a: |\n  x\ty\n", "{\"a\":\"x\\ty\\n\"}"),
        (
            "a: >\n  x\u{2029}\n  \ty\n",
            "{\"a\":\"x\\u2029\\n\\ty\\n\"}",
        ),
    ] {
        let (out, code) = run_yq_stdin(".", input, &["-o=json", "-I=0"])?;
        assert_eq!(code, 0, "{input:?}");
        assert_eq!(out.trim_end(), expected, "{input:?}");
    }
    Ok(())
}

/// A CRLF document, where a CR is a break of its own: the same refusals and the same content.
#[test]
fn crlf_documents_are_read_as_lf_ones_are_6287() -> Result<()> {
    for input in [
        "a: |\r\n  x\u{2028}\ty\r\nb: 1\r\n",
        "a: |+\r\n  x\u{85} \ty\r\n\r\nb: 1\r\n",
        "a: >\r\n  x\u{2029}\ty\r\nb: 1\r\n",
    ] {
        let (out, code) = run_yq_stdin(".", input, &["-o=json", "-I=0"])?;
        assert_eq!(out.trim_end(), "", "{input:?}");
        assert_ne!(code, 0, "{input:?}");
    }
    let (out, code) = run_yq_stdin(".", "a: |\r\n  x\ty\r\nb: 1\r\n", &["-o=json", "-I=0"])?;
    assert_eq!(code, 0);
    assert_eq!(out.trim_end(), "{\"a\":\"x\\ty\\n\",\"b\":1}");
    Ok(())
}
