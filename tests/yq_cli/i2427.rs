//! #2427: `--eval-all` reads the whole document list the way yq's `reduceOperator` does, so
//! `SRC as $x ireduce (INIT; UPDATE)` folds every document of every file -- `. as $item ireduce
//! ({}; . * $item)` is yq's own recipe for merging several files -- and an empty `{}` is one
//! mapping, not one per document. Every row is `yq ea -o=json -I0 FILTER f1 f2 f3` captured from
//! yq v4.53.3; the files are [`FILES`]. `docs/compliance/yq/limitations.md` records where
//! succinctly answers differently.

use std::io::Write;

use anyhow::Result;
use tempfile::NamedTempFile;

use crate::run_yq_files;

/// Three files, four documents: the first file holds two.
const FILES: [&str; 3] = [
    "a: 1\nb: {x: 1}\n---\nd: 4\n",
    "b: {y: 2}\nc: [1, 2]\n",
    "c: [3]\na: 9\n",
];

/// (filter, stdout), each the same as yq v4.53.3's with exit code 0.
const ROWS: &[(&str, &str)] = &[
    (
        ". as $item ireduce ({}; . * $item)",
        "{\"a\":9,\"b\":{\"x\":1,\"y\":2},\"d\":4,\"c\":[3]}\n",
    ),
    (
        ". as $item ireduce ({}; . *+ $item)",
        "{\"a\":9,\"b\":{\"x\":1,\"y\":2},\"d\":4,\"c\":[1,2,3]}\n",
    ),
    (
        ". as $item ireduce ({}; . *d $item)",
        "{\"a\":9,\"b\":{\"x\":1,\"y\":2},\"d\":4,\"c\":[3,2]}\n",
    ),
    (
        ". as $item ireduce ({}; . + $item)",
        "{\"a\":9,\"b\":{\"y\":2},\"d\":4,\"c\":[3]}\n",
    ),
    (". as $item ireduce ({}; . * $item) | .a", "9\n"),
    (
        ". as $item ireduce ({}; . * $item) | keys",
        "[\"a\",\"b\",\"d\",\"c\"]\n",
    ),
    (". as $i ireduce (0; . + 1)", "4\n4\n4\n4\n"),
    (". as $i ireduce ({}; .z = $i.a)", "{\"z\":9}\n"),
    (
        ". as $i ireduce ({\"k\": 0}; .k += $i.a)",
        "{\"k\":10}\n{\"k\":10}\n{\"k\":10}\n{\"k\":10}\n",
    ),
    (
        ". as $i ireduce ([]; . + [$i | keys | length])",
        "[2,1,2,2]\n",
    ),
    (". as $i ireduce (.a; . + 1)", "5\n4\n4\n13\n"),
    (".a as $i ireduce (0; . + 1)", "4\n4\n4\n4\n"),
    (".[] as $i ireduce (0; . + 1)", "7\n7\n7\n7\n"),
    (
        ". as $i ireduce ({}; . * $i) | [.]",
        "[{\"a\":9,\"b\":{\"x\":1,\"y\":2},\"d\":4,\"c\":[3]}]\n",
    ),
    ("[. as $i ireduce (0; . + 1)]", "[1,1,1,1]\n"),
    (
        "(. as $i ireduce (0; . + 1)) | . + 100",
        "104\n104\n104\n104\n",
    ),
    (
        "select(fi == 0) | . as $i ireduce ({}; . * $i)",
        "{\"a\":1,\"b\":{\"x\":1},\"d\":4}\n",
    ),
    (
        ". as $i ireduce ([]; . + [$i | document_index])",
        "[0,1,0,0]\n",
    ),
    (". as $i ireduce ([]; . + [$i | file_index])", "[0,0,1,2]\n"),
    (
        ". as $i ireduce ({}; .[$i | document_index | tostring] = $i.a)",
        "{\"0\":9,\"1\":null}\n",
    ),
    (". as $i ireduce ([]; . + [$i | path])", "[[],[],[],[]]\n"),
    (". as $i ireduce ([]; . + [$i | line])", "[1,4,1,1]\n"),
    ("{}", "{}\n"),
    ("{} | length", "0\n"),
    (". | {}", "{}\n"),
    ("{}, {}", "{}\n{}\n"),
    ("({}, 1)", "{}\n1\n1\n1\n1\n"),
    ("{} * {}", "{}\n"),
    (
        "{} + {\"k\": 1}",
        "{\"k\":1}\n{\"k\":1}\n{\"k\":1}\n{\"k\":1}\n",
    ),
    ("{\"k\": 0}", "{\"k\":0}\n{\"k\":0}\n{\"k\":0}\n{\"k\":0}\n"),
    ("{} | [.]", "[{}]\n"),
    ("[.] | {}", "{}\n"),
    (". as $x | {}", "{}\n{}\n{}\n{}\n"),
    ("select(.a > 1) | {}", "{}\n"),
    (".zz as $i ireduce ({}; . * $i)", "{}\n"),
    (".zz[] as $i ireduce (0; . + 1)", "0\n0\n0\n0\n"),
    ("select(fi == 9) | . as $i ireduce ({}; . * $i)", "{}\n"),
];

fn files() -> Result<Vec<NamedTempFile>> {
    FILES
        .iter()
        .map(|text| {
            let mut file = NamedTempFile::new()?;
            write!(file, "{text}")?;
            Ok(file)
        })
        .collect()
}

#[test]
fn every_row_matches_yq_2427() -> Result<()> {
    let files = files()?;
    let paths: Vec<&std::path::Path> = files.iter().map(NamedTempFile::path).collect();
    for (filter, expected) in ROWS {
        let (stdout, stderr, code) =
            run_yq_files(filter, &paths, &["--eval-all", "-o", "json", "-I0"])?;
        assert_eq!(code, 0, "{filter}: {stderr}");
        assert_eq!(&stdout, expected, "{filter}");
    }
    Ok(())
}

/// An error in UPDATE ends the run with no output, as yq's does.
#[test]
fn an_error_in_the_update_prints_nothing_2427() -> Result<()> {
    let files = files()?;
    let paths: Vec<&std::path::Path> = files.iter().map(NamedTempFile::path).collect();
    let (stdout, stderr, code) = run_yq_files(
        ". as $i ireduce ({}; error(\"boom\"))",
        &paths,
        &["--eval-all", "-o", "json", "-I0"],
    )?;
    assert_ne!(code, 0);
    assert_eq!(stdout, "");
    assert!(stderr.contains("boom"), "{stderr}");
    Ok(())
}

/// The merge recipe over YAML output, as the documentation shows it, keeps every key of every
/// file; yq's flow style (`b: {x: 1, y: 2}`) is the presentation gap #2651 records.
#[test]
fn the_documented_merge_recipe_merges_every_file_2427() -> Result<()> {
    let files = files()?;
    let paths: Vec<&std::path::Path> = files.iter().map(NamedTempFile::path).collect();
    let (stdout, stderr, code) = run_yq_files(
        ". as $item ireduce ({}; . * $item)",
        &paths,
        &["--eval-all"],
    )?;
    assert_eq!(code, 0, "{stderr}");
    let json = run_yq_files(
        ". as $item ireduce ({}; . * $item)",
        &paths,
        &["--eval-all", "-o", "json", "-I0"],
    )?
    .0;
    assert_eq!(
        json,
        "{\"a\":9,\"b\":{\"x\":1,\"y\":2},\"d\":4,\"c\":[3]}\n"
    );
    assert!(stdout.contains("a: 9"), "{stdout}");
    Ok(())
}

/// A destructuring loop pattern is succinctly's own (`--jq-extensions`; yq has none), and folds
/// the same document list: two documents give two seeds, each folding both.
#[test]
fn a_destructuring_pattern_folds_the_document_list_2427() -> Result<()> {
    let (stdout, stderr, code) = crate::run_yq_stdin_with_stderr(
        ". as [$a] ireduce (0; . + $a)",
        "[1, 2]\n---\n[3]\n",
        &["--eval-all", "--jq-extensions", "-o", "json", "-I0"],
    )?;
    assert_eq!(code, 0, "{stderr}");
    assert_eq!(stdout, "4\n4\n");
    Ok(())
}
