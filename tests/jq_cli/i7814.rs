//! #7814: a `--rawfile`, `-f` program, raw data import or module file that is UTF-8 throughout
//! becomes its text without a second copy of the file; one with a byte that is not UTF-8 keeps
//! #7436's answers, now reached from the read that holds the first such byte. These rows pin the
//! answers of the files that take the two shortcuts: multi-megabyte files, a bad byte hundreds of
//! reads in, a program with a shebang line, a module read whole.
//!
//! Every expectation was captured from jq 1.7.1 (`/usr/bin/jq`); jq 1.8.2 agrees on every row.

use anyhow::Result;

fn run(dir: &std::path::Path, args: &[&str]) -> Result<(String, String, i32)> {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_succinctly"))
        .arg("jq")
        .args(args)
        .current_dir(dir)
        .env("LC_ALL", "C")
        .output()?;
    Ok((
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code().unwrap_or(-1),
    ))
}

/// 3,000,000 bytes of one- to four-byte characters, so that reads end inside characters.
fn mixed_text() -> Vec<u8> {
    let unit = "ab\u{e9}\u{20ac}\u{1f600} ".as_bytes();
    unit.repeat(3_000_000 / unit.len())
}

#[test]
fn a_large_rawfile_that_is_utf8_is_the_files_text_7814() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let text = mixed_text();
    std::fs::write(dir.path().join("big.txt"), &text)?;
    // The same text as one JSON string, read by the parser rather than raw.
    let mut json = b"\"".to_vec();
    json.extend_from_slice(&text);
    json.push(b'"');
    std::fs::write(dir.path().join("big.json"), json)?;
    let got = run(
        dir.path(),
        &[
            "-nc",
            "--rawfile",
            "s",
            "big.txt",
            "--slurpfile",
            "d",
            "big.json",
            "[$d[0] == $s, ($s | length, utf8bytelength), ($s[4094:4100] | explode)]",
        ],
    )?;
    assert_eq!(
        got,
        (
            "[true,1500000,3000000,[233,8364,128512,32,97,98]]\n".to_string(),
            String::new(),
            0
        )
    );
    Ok(())
}

#[test]
fn a_bad_byte_hundreds_of_reads_into_a_rawfile_reads_as_jq_reads_it_7814() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let mut text = mixed_text();
    // A lead byte cut by the end of the 300th read, and a bad byte further on.
    text[4096 * 300 - 1] = 0xc3;
    text[1_500_000] = 0xff;
    std::fs::write(dir.path().join("bad.txt"), &text)?;
    let got = run(
        dir.path(),
        &[
            "-nc",
            "--rawfile",
            "s",
            "bad.txt",
            "$s | [length, utf8bytelength, (explode | indices(65533)), (explode | .[409300:409304], .[1090695:1090703])]",
        ],
    )?;
    assert_eq!(
        got,
        (
            "[1500000,3000004,[614399,750000],[128512,32,97,98],[8364,128512,32,97,98,233,8364,128512]]\n"
                .to_string(),
            String::new(),
            0
        )
    );

    // Four-byte characters to the first bad sequence, which is past the first read.
    let emoji = "\u{1f600}".as_bytes();
    let mut text = emoji.repeat(1500);
    text.extend_from_slice(b"\xe1\x41");
    text.extend_from_slice(&emoji.repeat(1500));
    text.extend_from_slice(b"\xffz");
    std::fs::write(dir.path().join("emoji.txt"), &text)?;
    let got = run(
        dir.path(),
        &[
            "-nc",
            "--rawfile",
            "s",
            "emoji.txt",
            "$s | [length, (explode | indices(65533)), (explode | .[1498:1504], .[-4:])]",
        ],
    )?;
    assert_eq!(
        got,
        (
            "[3004,[1500,3002],[128512,128512,65533,65,128512,128512],[128512,128512,65533,122]]\n"
                .to_string(),
            String::new(),
            0
        )
    );
    Ok(())
}

#[test]
fn a_program_file_loses_its_shebang_line_7814() -> Result<()> {
    let dir = tempfile::tempdir()?;
    std::fs::write(
        dir.path().join("prog.jq"),
        b"#!/usr/bin/jq -f\n\"\xc3\xa9\" | [., length]",
    )?;
    let got = run(dir.path(), &["-nc", "-f", "prog.jq"])?;
    assert_eq!(got, ("[\"\u{e9}\",1]\n".to_string(), String::new(), 0));
    Ok(())
}

#[test]
fn a_raw_data_import_and_a_module_read_whole_7814() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let lib = dir.path().join("lib");
    std::fs::create_dir(&lib)?;
    std::fs::write(lib.join("r.json"), mixed_text())?;
    std::fs::write(
        lib.join("m.jq"),
        b"def f: \"\xc3\xa9 ok\"; def g: \"\xff x\";",
    )?;
    let got = run(
        dir.path(),
        &[
            "-nc",
            "-L",
            "lib",
            "--rawfile",
            "s",
            "lib/r.json",
            "import \"r\" as $r {raw: true}; [$r == $s, ($r | length)]",
        ],
    )?;
    assert_eq!(got, ("[true,1500000]\n".to_string(), String::new(), 0));
    let got = run(
        dir.path(),
        &["-nc", "-L", "lib", "import \"m\" as m; [m::f, m::g]"],
    )?;
    assert_eq!(
        got,
        (
            "[\"\u{e9} ok\",\"\u{fffd} x\"]\n".to_string(),
            String::new(),
            0
        )
    );
    Ok(())
}
