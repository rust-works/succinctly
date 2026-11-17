//! #7234: a tag written with a named handle that a `%TAG` directive declares (`!e!x` under
//! `%TAG !e! tag:e.com,2000:`) is the tag the prefix makes, which go-yaml resolves before yq sees
//! it: yq prints it verbatim (`!<tag:e.com,2000:x>`) and `tag` answers the full tag. The
//! directives of one document are not the next one's. Each row is
//! `(document, args, stdout)` as yq v4.53.3 answers.

use anyhow::Result;

use crate::run_yq_stdin;

const ROWS: &[(&str, &[&str], &str)] = &[
    ("%TAG !e! tag:e.com,2000:\n---\nk: !e!x v\nm: !e!y {a: 1}\nl: !local z\n", &["."], "k: !<tag:e.com,2000:x> v\nm: !<tag:e.com,2000:y> {a: 1}\nl: !local z\n"),
    ("%TAG !e! tag:e.com,2000:\n---\nk: !e!x v\nm: !e!y {a: 1}\nl: !local z\n", &[".k = 1"], "k: !<tag:e.com,2000:x> 1\nm: !<tag:e.com,2000:y> {a: 1}\nl: !local z\n"),
    ("%TAG !e! tag:e.com,2000:\n---\nk: !e!x v\nm: !e!y {a: 1}\nl: !local z\n", &[".m"], "!<tag:e.com,2000:y> {a: 1}\n"),
    ("%TAG !e! tag:e.com,2000:\n---\nk: !e!x v\nm: !e!y {a: 1}\nl: !local z\n", &[".k | tag"], "tag:e.com,2000:x\n"),
    ("%TAG !e! tag:e.com,2000:\n---\nk: !e!x v\nm: !e!y {a: 1}\nl: !local z\n", &[".m | tag"], "tag:e.com,2000:y\n"),
    ("%TAG !e! tag:e.com,2000:\n---\nk: !e!x v\nm: !e!y {a: 1}\nl: !local z\n", &[".l | tag"], "!local\n"),
    ("%TAG !p! tag:p.com,2000:\n---\n!p!x key: v\n", &["."], "!<tag:p.com,2000:x> key: v\n"),
    ("%TAG !e! tag:e.com,2000:\n---\n- !e!a 1\n- !e!b [1, 2]\n- !e!c\n  x: 1\n", &["."], "- !<tag:e.com,2000:a> 1\n- !<tag:e.com,2000:b> [1, 2]\n- !<tag:e.com,2000:c>\n  x: 1\n"),
    ("%TAG !e! tag:e.com,2000:\n---\n- !e!a 1\n- !e!b [1, 2]\n- !e!c\n  x: 1\n", &["-o=json", "-I0", "[.. | tag]"], "[\"!!seq\",\"tag:e.com,2000:a\",\"tag:e.com,2000:b\",\"!!int\",\"!!int\",\"tag:e.com,2000:c\",\"!!int\"]\n"),
    ("%TAG !e! tag:e.com,2000:\n---\nk: &a !e!x v\nj: *a\n", &["."], "k: &a !<tag:e.com,2000:x> v\nj: *a\n"),
    ("%TAG !e! tag:e.com,2000:\n---\nk: &a !e!x v\nj: *a\n", &[".j | tag"], "\n"),
    ("%TAG !e! tag:e.com,2000:\n---\na: {k: !e!x v, j: [!e!y 1]}\n", &["."], "a: {k: !<tag:e.com,2000:x> v, j: [!<tag:e.com,2000:y> 1]}\n"),
    ("%TAG !e! tag:e.com,2000:\n---\n!e!r\na: 1\n", &["."], "!<tag:e.com,2000:r>\na: 1\n"),
    ("%TAG !e! tag:e.com,2000:\n---\na: !e!x 1\n---\n", &[".a | tag"], "tag:e.com,2000:x\n---\n!!null\n"),
    ("%TAG !e! tag:e.com,2000:\n%TAG !f! tag:f.com,2000:\n---\na: !e!x 1\nb: !f!x 2\n", &["."], "a: !<tag:e.com,2000:x> 1\nb: !<tag:f.com,2000:x> 2\n"),
    ("%TAG !e! tag:e.com,2000:\n---\na: !e!x 1\n...\n%TAG !e! tag:f.com,2000:\n---\nb: !e!x 2\n", &["."], "a: !<tag:e.com,2000:x> 1\n---\nb: !<tag:f.com,2000:x> 2\n"),
    ("%TAG !e! tag:e.com,2000:   # c\n---\na: !e!x 1\n", &["."], "a: !<tag:e.com,2000:x> 1\n"),
    ("%YAML 1.1\n%TAG !e! tag:e.com,2000:\n---\na: !e!x 1\n", &["."], "%YAML 1.1\na: !<tag:e.com,2000:x> 1\n"),
    ("%TAG !e! tag:e.com,2000:a/b:\n---\na: !e!x 1\n", &[".a | tag"], "tag:e.com,2000:a/b:x\n"),
    ("%TAG !l! !\n---\na: !l!x 1\n", &["."], "a: !x 1\n"),
    ("%TAG !e! tag:e.com,2000:\n---\na: !!str 1\nb: !loc 2\nc: !<tag:z.com,2000:t> 3\n", &["."], "a: !!str 1\nb: !loc 2\nc: !<tag:z.com,2000:t> 3\n"),
    ("%TAG !e! tag:e.com,2000:\n---\na: !e!x |\n  text\n", &["."], "a: !<tag:e.com,2000:x> |\n  text\n"),
    ("%TAG !e! tag:e.com,2000:\n---\nk: !e!x v\n", &["-P", "."], "k: !<tag:e.com,2000:x> v\n"),
    ("---\n%TAG !e! tag:e.com,2000:\n---\na: !e!x 1\n", &["."], "---\na: !<tag:e.com,2000:x> 1\n"),
];

#[test]
fn test_a_declared_tag_handle_is_resolved_to_the_verbatim_tag_7234() -> Result<()> {
    for &(document, args, stdout) in ROWS {
        let (filter, flags) = args.split_last().expect("a filter");
        let (out, code) = run_yq_stdin(filter, document, flags)?;
        assert_eq!(
            (out.as_str(), code),
            (stdout, 0),
            "{args:?} on {document:?}"
        );
    }
    Ok(())
}
