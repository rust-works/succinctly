//! #6472: a directory given as an input file opens in jq, and its first read fails.
//!
//! Every row was captured from jq 1.7.1 (`/usr/bin/jq`, macOS). `fopen` succeeds on a directory
//! and `fgets` fails with `EISDIR`, so the file is a source with no bytes, and jq prints
//! `jq: error: Is a directory` when its reader moves on to the next file: once per directory that
//! a file follows, and not at all for the last. Its main loop ends the run after the next value
//! once a read has failed, as it does after a file that would not open (#5472), and the exit status
//! is 2 whichever way it went.

use anyhow::Result;

use crate::run_jq_full;

/// A temp dir holding `nums.json` (`1 2 3`), `b.json` (`4`) and a directory `adir`.
struct Files {
    _dir: tempfile::TempDir,
    nums: String,
    b: String,
    adir: String,
    gone: String,
}

impl Files {
    fn new() -> Result<Self> {
        let dir = tempfile::tempdir()?;
        let path = |name: &str| dir.path().join(name).to_string_lossy().into_owned();
        let (nums, b, adir) = (path("nums.json"), path("b.json"), path("adir"));
        std::fs::write(&nums, "1\n2\n3\n")?;
        std::fs::write(&b, "4\n")?;
        std::fs::create_dir(&adir)?;
        Ok(Self {
            gone: path("gone.json"),
            nums,
            b,
            adir,
            _dir: dir,
        })
    }
}

/// Runs `args`, which name the files as `{nums}`, `{b}`, `{adir}` and `{gone}`: the temp dir's
/// paths go in, and come back out of the stderr, whose open errors name the path.
fn run(f: &Files, args: &[&str]) -> Result<(String, String, i32)> {
    let sub = |text: &str, back: bool| {
        let mut text = text.to_owned();
        for (name, path) in [
            ("nums", &f.nums),
            ("b", &f.b),
            ("adir", &f.adir),
            ("gone", &f.gone),
        ] {
            let name = format!("{{{name}}}");
            text = if back {
                text.replace(path.as_str(), &name)
            } else {
                text.replace(&name, path)
            };
        }
        text
    };
    let args: Vec<String> = args.iter().map(|a| sub(a, false)).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let (stdout, stderr, code) = run_jq_full(&args, Some(""))?;
    Ok((stdout, sub(&stderr, true), code))
}

/// A directory opens and its first read fails: jq says `Is a directory` when its reader moves on to
/// a later file, so once per directory that a file follows, before that file, and not for the last.
/// Its main loop stops after the next value once a read has failed.
#[test]
fn a_directory_is_a_source_whose_read_fails_6472() -> Result<()> {
    let f = Files::new()?;
    #[allow(clippy::type_complexity)]
    let rows: &[(&[&str], &str, &str, i32)] = &[
        (&["-c", ".", "{adir}"], "", "", 2),
        (&["-c", ".", "{adir}", "{nums}"], "1\n", "jq: error: Is a directory\n", 2),
        (&["-c", ".", "{nums}", "{adir}"], "1\n2\n3\n", "", 2),
        (&["-c", ".", "{adir}", "{nums}", "{adir}"], "1\n", "jq: error: Is a directory\n", 2),
        (&["-c", ".", "{adir}", "{adir}", "{nums}"], "1\n", "jq: error: Is a directory\njq: error: Is a directory\n", 2),
        (&["-c", ".", "{adir}", "{gone}", "{nums}"], "1\n", "jq: error: Is a directory\njq: error: Could not open file {gone}: No such file or directory\n", 2),
        (&["-c", ".", "{gone}", "{adir}", "{nums}"], "1\n", "jq: error: Could not open file {gone}: No such file or directory\njq: error: Is a directory\n", 2),
        (&["-nc", "input", "{adir}"], "", "jq: error (at {adir}:0): break\n", 2),
        (&["-nc", "[inputs]", "{adir}", "{nums}"], "[1,2,3]\n", "jq: error: Is a directory\n", 2),
        (&["-nc", "[input_filename, input]", "{adir}", "{nums}"], "[null,1]\n", "jq: error: Is a directory\n", 2),
        (&["-c", "-s", ".", "{adir}"], "[]\n", "", 2),
        (&["-c", "-s", ".", "{adir}", "{nums}"], "[1,2,3]\n", "jq: error: Is a directory\n", 2),
        (&["-c", "-R", ".", "{adir}", "{b}"], "\"4\"\n", "jq: error: Is a directory\n", 2),
        (&["-nc", "--stream", "[inputs]", "{adir}", "{b}"], "[[[],4]]\n", "jq: error: Is a directory\n", 2),
        (&["-nc", "--stream", ".", "{adir}"], "null\n", "", 0),
        (&["-c", "--seq", ".", "{adir}", "{b}"], "", "jq: error: Is a directory\njq: ignoring parse error: Unfinished abandoned text at EOF at line 2, column 0\n", 2),
        (&["-c", ".", "{nums}", "{adir}", "{b}"], "1\n2\n3\n4\n", "jq: error: Is a directory\n", 2),
    ];
    for (args, stdout, stderr, code) in rows {
        let got = run(&f, args)?;
        assert_eq!(
            (got.0.as_str(), got.1.as_str(), got.2),
            (*stdout, *stderr, *code),
            "{args:?}"
        );
    }
    Ok(())
}
