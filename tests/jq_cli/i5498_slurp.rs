//! #5498: `--stream -s` reads through the stream reader, as a plain `--stream` walk does, and answers
//! as jq's streaming parser does: every occurrence of a repeated key is an event, a document of any
//! depth is read, the array closes where jq's `--slurp` loop closes it (a scalar's closing event can
//! come after the array), and the first fault is jq's own `parse error` text, with nothing printed.
//!
//! The rows are `tests/data/jq-stream-slurp-oracle-5498.tsv`: stream, program, flags, stdout, stderr
//! and exit status, captured from `/usr/bin/jq` 1.7.1 and kept only where `succinctly jq` answered the
//! same (over stdin). Fields escape `\\`, tab, newline, carriage return and any other non-ASCII or
//! control character as `\u{..}`.

use anyhow::Result;

use crate::run_jq_full;

fn unescape(field: &str) -> String {
    let mut out = String::new();
    let mut chars = field.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some('\\') => out.push('\\'),
            Some('u') => {
                assert_eq!(chars.next(), Some('{'));
                let hex: String = chars.by_ref().take_while(|&c| c != '}').collect();
                out.push(char::from_u32(u32::from_str_radix(&hex, 16).unwrap()).unwrap());
            }
            other => panic!("bad escape {other:?} in {field:?}"),
        }
    }
    out
}

#[test]
fn a_slurped_stream_answers_as_jq_5498() -> Result<()> {
    let table = include_str!("../data/jq-stream-slurp-oracle-5498.tsv");
    let mut count = 0;
    for line in table.lines() {
        let [stream, program, flags, stdout, stderr, code]: [&str; 6] = line
            .split('\t')
            .collect::<Vec<_>>()
            .try_into()
            .unwrap_or_else(|_| panic!("malformed row {line:?}"));
        let (stream, program) = (unescape(stream), unescape(program));
        let mut args: Vec<&str> = flags.split(' ').collect();
        args.push(&program);
        let (got_out, got_err, got_code) = run_jq_full(&args, Some(&stream))?;
        let context = format!("{args:?} over {stream:?}");
        assert_eq!(got_out, unescape(stdout), "stdout of {context}");
        assert_eq!(got_err, unescape(stderr), "stderr of {context}");
        assert_eq!(got_code, code.parse::<i32>()?, "status of {context}");
        count += 1;
    }
    assert_eq!(count, 1311, "every row of the table was replayed");
    Ok(())
}

/// The files are one stream to jq's parser: a token open where one ends continues in the next, and
/// a fault in a later file names its line and column there. Expected strings from `/usr/bin/jq`
/// 1.7.1; `{DIR}` is the directory the files are written to.
#[test]
fn a_slurped_stream_over_several_files_is_one_stream_5498() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let prefix = dir.path().to_string_lossy().into_owned();
    #[allow(clippy::type_complexity)]
    let rows: &[(&[&str], &[&str], &str, &str, i32)] = &[
        (&["[1,", "2]\n"], &["-c", "--stream", "-s", "."], "[[[0],1],[[1],2],[[1]]]\n", "", 0),
        (&["[1", ",2", "]\n"], &["-c", "--stream", "-s", "."], "[[[0],1],[[1],2],[[1]]]\n", "", 0),
        (
            &["{\"a\":1}\n", "[2]\n"],
            &["-c", "--stream", "-s", "."],
            "[[[\"a\"],1],[[\"a\"]],[[0],2],[[0]]]\n",
            "",
            0,
        ),
        (&["{\"a\":1}\n", "{\"a\":", "2}\n", "[3]\n"], &["-c", "--stream", "-s", "length"], "6\n", "", 0),
        (&["[1]\n", "[2]\n"], &["-c", "--stream", "-s", "input_filename"], "\"{DIR}/f1.json\"\n", "", 0),
        (
            &["[1]\n", "}\n"],
            &["-c", "--stream", "-s", "."],
            "",
            "jq: parse error: Unmatched '}' at the top-level at line 2, column 1\n",
            5,
        ),
        (
            &["[1]\n", "[2,\n}\n"],
            &["-c", "--stream", "-s", "."],
            "",
            "jq: parse error: Expected another key:value pair at line 3, column 1\n",
            5,
        ),
        (
            &["1 ", "2\n3 }\n"],
            &["-c", "--stream", "-s", "."],
            "",
            "jq: parse error: Unmatched '}' at the top-level at line 2, column 3\n",
            5,
        ),
        // `--stream-errors` makes the fault an input, inside the array.
        (
            &["[1]\n", "[2]\n}\n"],
            &["-c", "--stream-errors", "-s", "."],
            "[[[0],1],[[0]],[[0],2],[[0]],[\"Unmatched '}' at the top-level at line 3, column 1\",[]]]\n",
            "",
            0,
        ),
    ];
    for (files, flags, want_out, want_err, want_code) in rows {
        let paths = files
            .iter()
            .enumerate()
            .map(|(i, text)| {
                let path = dir.path().join(format!("f{i}.json"));
                std::fs::write(&path, text)?;
                Ok(path.to_string_lossy().into_owned())
            })
            .collect::<Result<Vec<_>>>()?;
        let mut args: Vec<&str> = flags.to_vec();
        args.extend(paths.iter().map(String::as_str));
        let (stdout, stderr, code) = run_jq_full(&args, None)?;
        assert_eq!(
            (stdout.as_str(), stderr.as_str(), code),
            (
                want_out.replace("{DIR}", &prefix).as_str(),
                want_err.replace("{DIR}", &prefix).as_str(),
                *want_code
            ),
            "{files:?} under {flags:?}"
        );
    }
    Ok(())
}
