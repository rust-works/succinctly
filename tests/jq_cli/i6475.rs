//! #6475: jq opens an input file when its reader has read the one before and asks for more, so
//! a program that stops reading earlier never meets a file that will not open: `jq -n input
//! nums.json missing` prints `1` and exits 0. Under `-n` the files are read as `input`/`inputs` get
//! to them, and each failure is said when a read reaches it (or finds the stream at its end), before
//! the read returns, and fails the run then.
//!
//! Every row was captured from jq 1.7.1 (`/usr/bin/jq`, macOS). A file whose last value is a bare
//! number with no newline after it (`one.json`) completes only at its end, which sends the reader on
//! to the next file, so a failure after it is said whatever the program reads.

use anyhow::Result;

use crate::run_jq_full;

/// A temp dir holding `nums.json` (`1 2 3` on three lines), `b.json` (`4`), `one.json` (`1`, no
/// newline), `arr.json` (`[1,2]`, no newline), `raw.txt` (`abc`, no newline), a directory `adir` and
/// the path of a file that is not there.
struct Files {
    _dir: tempfile::TempDir,
    paths: Vec<(&'static str, String)>,
}

impl Files {
    fn new() -> Result<Self> {
        let dir = tempfile::tempdir()?;
        let path = |name: &str| dir.path().join(name).to_string_lossy().into_owned();
        std::fs::write(path("nums.json"), "1\n2\n3\n")?;
        std::fs::write(path("b.json"), "4\n")?;
        std::fs::write(path("one.json"), "1")?;
        std::fs::write(path("arr.json"), "[1,2]")?;
        std::fs::write(path("raw.txt"), "abc")?;
        std::fs::create_dir(path("adir"))?;
        let paths = vec![
            ("nums", path("nums.json")),
            ("b", path("b.json")),
            ("one", path("one.json")),
            ("arr", path("arr.json")),
            ("raw", path("raw.txt")),
            ("adir", path("adir")),
            ("gone", path("gone.json")),
        ];
        Ok(Self { _dir: dir, paths })
    }

    /// `text` with `{name}` replaced by the file's path, or, with `back`, the paths by the names.
    fn sub(&self, text: &str, back: bool) -> String {
        let mut text = text.to_owned();
        for (name, path) in &self.paths {
            let name = format!("{{{name}}}");
            text = if back {
                text.replace(path.as_str(), &name)
            } else {
                text.replace(&name, path)
            };
        }
        text
    }
}

#[test]
fn a_file_the_program_never_reaches_is_not_reported_6475() -> Result<()> {
    let f = Files::new()?;
    #[allow(clippy::type_complexity)]
    let rows: &[(&[&str], &str, &str, i32)] = &[
        (&["-nc", "input", "{nums}", "{gone}"], "1\n", "", 0),
        (
            &["-nc", "[input_filename,input]", "{nums}", "{gone}", "{b}"],
            "[null,1]\n",
            "",
            0,
        ),
        (&["-nc", "input", "{nums}", "{adir}"], "1\n", "", 0),
        (
            &["-nc", "[inputs]", "{nums}", "{gone}"],
            "[1,2,3]\n",
            "jq: error: Could not open file {gone}: No such file or directory\n",
            2,
        ),
        (
            &["-nc", "[inputs]", "{gone}", "{nums}"],
            "[1,2,3]\n",
            "jq: error: Could not open file {gone}: No such file or directory\n",
            2,
        ),
        (
            &["-nc", "input", "{gone}", "{nums}"],
            "1\n",
            "jq: error: Could not open file {gone}: No such file or directory\n",
            2,
        ),
        (
            &["-nc", "[input,input,input]", "{nums}", "{gone}"],
            "[1,2,3]\n",
            "",
            0,
        ),
        (
            &[
                "-nc",
                "[input,input,input,input]",
                "{nums}",
                "{gone}",
                "{b}",
            ],
            "[1,2,3,4]\n",
            "jq: error: Could not open file {gone}: No such file or directory\n",
            2,
        ),
        (&["-nc", "first(inputs)", "{nums}", "{gone}"], "1\n", "", 0),
        (
            &["-nc", "[limit(4;inputs)]", "{nums}", "{gone}", "{b}"],
            "[1,2,3,4]\n",
            "jq: error: Could not open file {gone}: No such file or directory\n",
            2,
        ),
        (
            &["-nc", "if false then input else 1 end", "{nums}", "{gone}"],
            "1\n",
            "",
            0,
        ),
        (
            &["-nc", "input_line_number", "{nums}", "{gone}"],
            "0\n",
            "",
            0,
        ),
        (&["-nc", "input", "{arr}", "{gone}"], "[1,2]\n", "", 0),
        (
            &["-nc", "input", "{one}", "{gone}"],
            "1\n",
            "jq: error: Could not open file {gone}: No such file or directory\n",
            2,
        ),
        (
            &["-nc", "[input,input]", "{nums}", "{adir}", "{b}"],
            "[1,2]\n",
            "",
            0,
        ),
        (&["-ncR", "input", "{nums}", "{gone}"], "\"1\"\n", "", 0),
        (
            &["-ncR", "input", "{raw}", "{gone}"],
            "\"abc\"\n",
            "jq: error: Could not open file {gone}: No such file or directory\n",
            2,
        ),
        (
            &["-nc", "[inputs]|length", "{nums}", "{adir}", "{b}"],
            "4\n",
            "jq: error: Is a directory\n",
            2,
        ),
        (
            &["-nc", "[input,halt]", "{nums}", "{gone}", "{b}"],
            "",
            "",
            0,
        ),
    ];
    for (args, stdout, stderr, code) in rows {
        let args: Vec<String> = args.iter().map(|a| f.sub(a, false)).collect();
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        let (out, err, status) = run_jq_full(&args, Some(""))?;
        assert_eq!(
            (out.as_str(), f.sub(&err, true).as_str(), status),
            (*stdout, *stderr, *code),
            "{args:?}"
        );
    }
    Ok(())
}
