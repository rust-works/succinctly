//! #7664: a `--slurpfile` file or a data import that ends at one of jq's 4,096-byte read boundaries
//! is read as any other file is: its last value is kept and a file that is not JSON is refused.
//! jq 1.7.1 and 1.8.2 never hand `jv_load_file`'s parser the final flag there, so they drop the
//! value and read the garbage as an empty stream (upstream fix jqlang/jq#3686, open). Matching
//! would discard data and accept corrupt input, so the divergence is deliberate and recorded in
//! `docs/compliance/jq/limitations.md`; this test keeps a later fidelity change from adopting it.
//!
//! Each row also carries jq's answer (`--slurpfile`, captured from jq 1.7.1, `/usr/bin/jq`; jq
//! 1.8.2 and the x86_64 Linux 1.7.1 agree): `JQ_DROPS` marks the rows where jq answers `[]`.

use anyhow::Result;

/// `JQ_DROPS`: jq answers `[]` for the file.
const JQ_DROPS: bool = true;

/// (spaces, tail, jq answers `[]`, succinctly's `$d` or `None` for a refusal).
const ROWS: &[(usize, &[u8], bool, Option<&str>)] = &[
    (4094, b"1", false, Some("[1]")),
    (4095, b"1", JQ_DROPS, Some("[1]")),
    (4096, b"1", false, Some("[1]")),
    (8191, b"1", JQ_DROPS, Some("[1]")),
    (4094, b"[", false, None),
    (4095, b"[", JQ_DROPS, None),
    (4096, b"[", false, None),
    (8191, b"[", JQ_DROPS, None),
    (4095, b"true", false, Some("[true]")),
    (4092, b"true", JQ_DROPS, Some("[true]")),
];

fn run(dir: &std::path::Path, args: &[&str]) -> Result<(String, String, Option<i32>)> {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_succinctly"))
        .arg("jq")
        .args(args)
        .current_dir(dir)
        .env("LC_ALL", "C")
        .env_remove("JQ_LIBRARY_PATH")
        .output()?;
    Ok((
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    ))
}

/// Check one file through `--slurpfile` and through a data import.
fn check(dir: &std::path::Path, bytes: &[u8], want: Option<&str>, label: &str) -> Result<()> {
    std::fs::write(dir.join("d.json"), bytes)?;
    let slurp = run(dir, &["-nc", "--slurpfile", "d", "d.json", "$d"])?;
    let import = run(dir, &["-nc", "-L", ".", "import \"d\" as $d; $d"])?;
    match want {
        Some(value) => {
            let want = (format!("{value}\n"), String::new(), Some(0));
            assert_eq!(slurp, want, "--slurpfile, {label}");
            assert_eq!(import, want, "import, {label}");
        }
        None => {
            assert_eq!((slurp.0.as_str(), slurp.2), ("", Some(2)), "{label}");
            assert!(
                slurp
                    .1
                    .starts_with("jq: Bad JSON in --slurpfile d d.json: "),
                "--slurpfile, {label}: {}",
                slurp.1
            );
            assert_eq!((import.0.as_str(), import.2), ("", Some(3)), "{label}");
            assert!(
                import.1.starts_with("jq: error loading data file ")
                    && import.1.ends_with("\n\njq: 1 compile error\n"),
                "import, {label}: {}",
                import.1
            );
        }
    }
    Ok(())
}

#[test]
fn a_file_that_ends_at_a_read_boundary_is_read_as_any_other_7664() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let mut dropped = 0;
    for &(spaces, tail, jq_drops, want) in ROWS {
        let mut bytes = vec![b' '; spaces];
        bytes.extend_from_slice(tail);
        let label = format!(
            "{} bytes ending {:?}",
            bytes.len(),
            String::from_utf8_lossy(tail)
        );
        check(dir.path(), &bytes, want, &label)?;
        dropped += usize::from(jq_drops);
    }
    assert_eq!(dropped, 5, "the boundary rows");
    Ok(())
}

/// The file the divergence was found with: 4,095 `a` bytes and `\xe2\x82!`, whose last read jq
/// extends by one byte to finish the UTF-8 sequence it ends inside (jq answers `[]`).
#[test]
fn the_reported_file_is_refused_7664() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let mut bytes = vec![b'a'; 4095];
    bytes.extend_from_slice(b"\xe2\x82!");
    check(dir.path(), &bytes, None, "4,095 a bytes and \\xe2\\x82!")
}
