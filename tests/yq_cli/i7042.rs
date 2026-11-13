//! #7042: `yq ea -i FILTER f1 f2 ...` evaluates the filter once over every document of every file and
//! writes the results to the first file, leaving the others as they are, as yq does. Every row was
//! captured from yq v4.53.3: (filter, the files before, the files after). `--front-matter` with
//! `--eval-all` stays refused (`limitations.md`): yq's own result for it keeps the body of the last
//! file only.

use anyhow::Result;
use tempfile::TempDir;

use crate::run_yq_files;

/// (filter, files before, files after).
#[rustfmt::skip]
const ROWS: &[(&str, &[&str], &[&str])] = &[
    (".k = 1", &["a: 1\nb: x\n", "c: 3\n"], &["a: 1\nb: x\nk: 1\n---\nc: 3\nk: 1\n", "c: 3\n"]),
    (". as $i ireduce ({}; . * $i)", &["a: 1\nb: x\n", "c: 3\n"], &["a: 1\nb: x\nc: 3\n", "c: 3\n"]),
    (".k = 2", &["a: 1\nb: x\n"], &["a: 1\nb: x\nk: 2\n"]),
    ("select(fi == 0)", &["a: 1\n", "c: 3\n"], &["a: 1\n", "c: 3\n"]),
    (".", &["a: 1\n", "c: 3\n", "e: 5\n"], &["a: 1\n---\nc: 3\n---\ne: 5\n", "c: 3\n", "e: 5\n"]),
    ("select(false)", &["a: 1\n", "c: 3\n"], &["", "c: 3\n"]),
    ("[.]", &["a: 1\n---\nb: 2\n", "c: 3\n"], &["- a: 1\n- b: 2\n- c: 3\n", "c: 3\n"]),
    (".a = 5", &["a: 1\nb:\n  - 1\n  - 2\n", "a: 2\n"], &["a: 5\nb:\n  - 1\n  - 2\n---\na: 5\n", "a: 2\n"]),
    (". as $i ireduce ({}; . * $i)", &["a:\n  b: 1\nl:\n  - 1\n", "a:\n  c: 2\nl:\n  - 2\n", "a:\n  d: 3\n"], &["a:\n  b: 1\n  c: 2\n  d: 3\nl:\n  - 2\n", "a:\n  c: 2\nl:\n  - 2\n", "a:\n  d: 3\n"]),
    ("select(di == 0)", &["a: 1\n---\nb: 2\n", "c: 3\n"], &["a: 1\n---\nc: 3\n", "c: 3\n"]),
];

#[test]
fn eval_all_inplace_writes_the_first_file_7042() -> Result<()> {
    for &(filter, before, after) in ROWS {
        let dir = TempDir::new()?;
        let paths: Vec<_> = before
            .iter()
            .enumerate()
            .map(|(i, text)| {
                let path = dir.path().join(format!("w{}.yaml", i + 1));
                std::fs::write(&path, text).map(|()| path)
            })
            .collect::<std::io::Result<_>>()?;
        let refs: Vec<&std::path::Path> = paths.iter().map(std::path::PathBuf::as_path).collect();
        let (stdout, stderr, status) = run_yq_files(filter, &refs, &["ea", "-i"])?;
        assert_eq!(
            (stdout.as_str(), stderr.as_str(), status),
            ("", "", 0),
            "{filter}"
        );
        for (path, expected) in paths.iter().zip(after) {
            assert_eq!(
                &std::fs::read_to_string(path)?,
                expected,
                "{filter} on {before:?}"
            );
        }
    }
    Ok(())
}
