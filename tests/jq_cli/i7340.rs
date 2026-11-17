//! #7340: jq's `--seq` reader treats a NUL byte two ways that are not an ordinary byte's. An
//! `fgets` chunk that holds no newline is measured with `strlen` (`jq_util_input_read_more`,
//! `src/util.c`), so jq's parser never sees the chunk from its first NUL on; and a number token is
//! read as a C string (`check_literal`), so a NUL in it ends the number. Each row is run as a
//! plain read, a slurp and an `input` loop, the three routes that build the values. Every expected
//! string was captured from jq 1.7.1 (`/usr/bin/jq`, macOS).

use anyhow::Result;

use crate::run_jq_over_byte_files;

const RS: &str = "\u{1e}";

/// `(name, files, [plain, slurp, inputs])`, each route's `(stdout, stderr)`; jq exits 0 on all.
type Row = (&'static str, Vec<Vec<u8>>, [(String, String); 3]);

fn warn(body: &str) -> String {
    format!("jq: ignoring parse error: {body}\n")
}

fn rows() -> Vec<Row> {
    let breaks = |n: usize| ",\"break\"".repeat(n);
    let long_string = format!("\"x{}\"", "y".repeat(4111));
    vec![
        (
            // The issue's first repro: the `1` is yielded, with no warning.
            "a NUL straight after a number",
            vec![b"\x1e\"ab\"\n\x1e1\x00\n\x1e7 \n\x1e{\"id\":1}\n".to_vec()],
            [
                (
                    format!("{RS}\"ab\"\n{RS}1\n{RS}7\n{RS}{{\"id\":1}}\n"),
                    String::new(),
                ),
                (format!("{RS}[\"ab\",1,7,{{\"id\":1}}]\n"), String::new()),
                (
                    format!("{RS}[\"ab\",1,7,{{\"id\":1}}{}]\n", breaks(4)),
                    String::new(),
                ),
            ],
        ),
        (
            // The issue's second repro: the rest of the 4095-byte chunk, ` 7 [8] ` and spaces, is
            // never parsed.
            "a NUL in a chunk with no newline",
            vec![[
                &b"\x1e{\"id\":1}\n\x1e\"ab\" \x00 7 [8] "[..],
                &[b' '; 5000],
                b"9 10\n\x1e{\"id\":2}\n",
            ]
            .concat()],
            [
                (
                    format!("{RS}{{\"id\":1}}\n{RS}\"ab\"\n{RS}9\n{RS}10\n{RS}{{\"id\":2}}\n"),
                    String::new(),
                ),
                (
                    format!("{RS}[{{\"id\":1}},\"ab\",9,10,{{\"id\":2}}]\n"),
                    String::new(),
                ),
                (
                    format!("{RS}[{{\"id\":1}},\"ab\",9,10,{{\"id\":2}}{}]\n", breaks(3)),
                    String::new(),
                ),
            ],
        ),
        (
            // Inside a container too; a number keeps its spelling up to the NUL, `nan` is a
            // number, and a keyword or a lone `-` is not.
            "numbers ended by a NUL",
            vec![
                b"\x1e[1\x00]\n\x1e{\"a\":2.50\x00x}\n\x1enan\x00\n\x1etrue\x00\n\x1e-\x00\n"
                    .to_vec(),
            ],
            {
                let w1 = "Invalid literal at line 5, column 0 (need RS to resync)";
                let w2 = "Invalid numeric literal at line 6, column 0 (need RS to resync)";
                let stderr = warn(w1) + &warn(w2);
                [
                    (
                        format!("{RS}[1]\n{RS}{{\"a\":2.50}}\n{RS}null\n"),
                        stderr.clone(),
                    ),
                    (format!("{RS}[[1],{{\"a\":2.50}},null]\n"), stderr),
                    (
                        format!(
                            "{RS}[[1],{{\"a\":2.50}},null,\"{w1}\",\"{w2}\"{}]\n",
                            breaks(3)
                        ),
                        String::new(),
                    ),
                ]
            },
        ),
        (
            // A value open across the bytes jq drops is read as if they were not there.
            "an array across the dropped bytes",
            vec![[
                &b"\x1e[1,"[..],
                &[b' '; 4000],
                b"\x00garbage",
                &[b' '; 200],
                b"2]\n",
            ]
            .concat()],
            [
                (format!("{RS}[1,2]\n"), String::new()),
                (format!("{RS}[[1,2]]\n"), String::new()),
                (format!("{RS}[[1,2]{}]\n", breaks(7)), String::new()),
            ],
        ),
        (
            "a string across the dropped bytes",
            vec![[
                &b"\x1e\"x"[..],
                &[b'y'; 4000],
                b"\x00zz",
                &[b'y'; 200],
                b"\"\n",
            ]
            .concat()],
            [
                (format!("{RS}{long_string}\n"), String::new()),
                (format!("{RS}[{long_string}]\n"), String::new()),
                (format!("{RS}[{long_string}{}]\n", breaks(7)), String::new()),
            ],
        ),
        (
            // A file's last chunk is cut to its end, and the next file is read on from there: the
            // chunks are each file's own.
            "a NUL in a file's last chunk",
            vec![
                [&b"\x1e[1,"[..], &[b' '; 100], b"\x00zz"].concat(),
                b"2]\n".to_vec(),
            ],
            [
                (format!("{RS}[1,2]\n"), String::new()),
                (format!("{RS}[[1,2]]\n"), String::new()),
                (format!("{RS}[[1,2]{}]\n", breaks(7)), String::new()),
            ],
        ),
        (
            // An RS among the dropped bytes is no RS to jq: `2` and `3` are not records.
            "an RS among the dropped bytes",
            vec![[&b"\x1e1 \x00\x1e2\x1e3 "[..], &[b' '; 4100], b"\n\x1e4\n"].concat()],
            [
                (format!("{RS}1\n{RS}4\n"), String::new()),
                (format!("{RS}[1,4]\n"), String::new()),
                (format!("{RS}[1,4{}]\n", breaks(6)), String::new()),
            ],
        ),
        (
            // With no RS byte at all, the column stops at the NUL.
            "no RS byte, and a NUL",
            vec![b"ab\x00cd".to_vec()],
            {
                let w = "Unfinished abandoned text at EOF at line 1, column 2";
                [
                    (String::new(), warn(w)),
                    (format!("{RS}[]\n"), warn(w)),
                    (format!("{RS}[\"{w}\"{}]\n", breaks(7)), String::new()),
                ]
            },
        ),
        (
            // ... and an RS after a NUL is not one, but an RS in the next chunk is.
            "an RS after a NUL, then one in the next chunk",
            vec![b"x\nab\x00cd\x1e1\nzz".to_vec()],
            {
                let w = "Invalid numeric literal at EOF at line 3, column 2";
                [
                    (format!("{RS}1\n"), warn(w)),
                    (format!("{RS}[1]\n"), warn(w)),
                    (format!("{RS}[1,\"{w}\"{}]\n", breaks(6)), String::new()),
                ]
            },
        ),
    ]
}

#[test]
fn a_nul_ends_the_number_and_the_fgets_chunk_it_is_in_7340() -> Result<()> {
    let routes: [&[&str]; 3] = [
        &["--seq", "-c", "."],
        &["--seq", "-s", "-c", "."],
        &["--seq", "-nc", "[limit(8; repeat(try input catch .))]"],
    ];
    for (name, files, want) in rows() {
        let files: Vec<&[u8]> = files.iter().map(Vec::as_slice).collect();
        for (args, (stdout, stderr)) in routes.iter().zip(&want) {
            let (got_out, got_err, code, _) = run_jq_over_byte_files(args, &files)?;
            assert_eq!(
                (got_out.as_str(), got_err.as_str(), code),
                (stdout.as_str(), stderr.as_str(), 0),
                "{name}: {args:?}"
            );
        }
    }
    Ok(())
}
