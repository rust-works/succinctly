//! #7734: an environment entry with no `=` (only a raw `execve` makes one) is read as jq reads it.
//!
//! jq has two readings of `environ`, which agree on every entry with a `=` (split at the first one,
//! #7487) and differ on one without. The `env` builtin (`f_env`) sets the whole string as a name
//! with the value `null`, in place, so `A=1` then `A` is `{"A":null}`. `$ENV` (`make_env`, built
//! once at compile time) deletes the name instead, so the same environment's `$ENV` is `{}`, and an
//! `A=3` after the bare `A` adds it again at the end. `succinctly jq` skipped such an entry in both.
//!
//! Every row was captured from jq 1.7.1 (`/usr/bin/jq`) and jq 1.8.2 (`/opt/homebrew/bin/jq`) on
//! macOS arm64, which agree on all of them, under the same environment made by a raw `execve`; jq
//! 1.7.1 on Linux x86_64 gives the same. A name that is not UTF-8 is substituted as #6564 does. Each
//! whole-object program drops the entry an instrumented (`cargo llvm-cov`) binary adds to itself, as
//! `i7487.rs` does (`OWN`).

#![cfg(unix)]
#![allow(unsafe_code)] // `posix_spawn` with a hand-made environment: `std::process` writes `NAME=value` only

use std::ffi::CString;

use anyhow::{ensure, Result};

/// Drops the entries a `cargo llvm-cov` binary adds to its own environment (see `i7487.rs`).
const OWN: &str = "def own: with_entries(select(.key | startswith(\"__LLVM_PROFILE_\") | not)); ";

/// Run `succinctly jq -n -c <OWN><program>` with exactly `environ` as its environment, returning
/// `(stdout, stderr, exit status)`.
fn run_with_environ(environ: &[&[u8]], program: &str) -> Result<(String, String, i32)> {
    let dir = tempfile::tempdir()?;
    let out = CString::new(dir.path().join("out").display().to_string())?;
    let err = CString::new(dir.path().join("err").display().to_string())?;
    let argv: Vec<CString> = [
        env!("CARGO_BIN_EXE_succinctly"),
        "jq",
        "-n",
        "-c",
        &format!("{OWN}{program}"),
    ]
    .iter()
    .map(|word| CString::new(*word))
    .collect::<Result<_, _>>()?;
    let envp: Vec<CString> = environ
        .iter()
        .map(|entry| CString::new(entry.to_vec()))
        .collect::<Result<_, _>>()?;
    let mut argv_ptrs: Vec<*mut libc::c_char> =
        argv.iter().map(|w| w.as_ptr().cast_mut()).collect();
    argv_ptrs.push(std::ptr::null_mut());
    let mut envp_ptrs: Vec<*mut libc::c_char> =
        envp.iter().map(|e| e.as_ptr().cast_mut()).collect();
    envp_ptrs.push(std::ptr::null_mut());
    let mut pid: libc::pid_t = 0;
    let mut status: libc::c_int = 0;
    // SAFETY: every pointer handed to the `posix_spawn` family points into a `CString` or a
    // null-terminated pointer array that outlives the calls, and the file-actions object is
    // initialised before use and destroyed after.
    unsafe {
        let mut actions: libc::posix_spawn_file_actions_t = std::mem::zeroed();
        ensure!(
            libc::posix_spawn_file_actions_init(&mut actions) == 0,
            "file actions"
        );
        let flags = libc::O_WRONLY | libc::O_CREAT | libc::O_TRUNC;
        ensure!(
            libc::posix_spawn_file_actions_addopen(
                &mut actions,
                0,
                c"/dev/null".as_ptr(),
                libc::O_RDONLY,
                0
            ) == 0
        );
        ensure!(
            libc::posix_spawn_file_actions_addopen(&mut actions, 1, out.as_ptr(), flags, 0o600)
                == 0
        );
        ensure!(
            libc::posix_spawn_file_actions_addopen(&mut actions, 2, err.as_ptr(), flags, 0o600)
                == 0
        );
        let spawned = libc::posix_spawn(
            &mut pid,
            argv_ptrs[0],
            &actions,
            std::ptr::null(),
            argv_ptrs.as_ptr(),
            envp_ptrs.as_ptr(),
        );
        libc::posix_spawn_file_actions_destroy(&mut actions);
        ensure!(spawned == 0, "posix_spawn failed: {spawned}");
        ensure!(libc::waitpid(pid, &mut status, 0) == pid, "waitpid");
    }
    ensure!(libc::WIFEXITED(status), "did not exit: {status}");
    Ok((
        String::from_utf8(std::fs::read(dir.path().join("out"))?)?,
        String::from_utf8(std::fs::read(dir.path().join("err"))?)?,
        libc::WEXITSTATUS(status),
    ))
}

/// (the whole environment, the program after `OWN`, jq's output).
type Row = (&'static [&'static [u8]], &'static str, &'static str);

const ROWS: &[Row] = &[
    // bare_only
    (&[b"A", b"Y=1"], "$ENV | own", "{\"Y\":\"1\"}"),
    (&[b"A", b"Y=1"], "env | own", "{\"A\":null,\"Y\":\"1\"}"),
    (&[b"A", b"Y=1"], "$ENV.A", "null"),
    (&[b"A", b"Y=1"], "env.A", "null"),
    (
        &[b"A", b"Y=1"],
        "env | own | keys_unsorted",
        "[\"A\",\"Y\"]",
    ),
    (&[b"A", b"Y=1"], "$ENV | own | keys_unsorted", "[\"Y\"]"),
    // several_bare
    (&[b"A", b"B", b"Y=1", b"C"], "$ENV | own", "{\"Y\":\"1\"}"),
    (
        &[b"A", b"B", b"Y=1", b"C"],
        "env | own",
        "{\"A\":null,\"B\":null,\"Y\":\"1\",\"C\":null}",
    ),
    (&[b"A", b"B", b"Y=1", b"C"], "$ENV.A", "null"),
    (&[b"A", b"B", b"Y=1", b"C"], "env.A", "null"),
    (
        &[b"A", b"B", b"Y=1", b"C"],
        "env | own | keys_unsorted",
        "[\"A\",\"B\",\"Y\",\"C\"]",
    ),
    (
        &[b"A", b"B", b"Y=1", b"C"],
        "$ENV | own | keys_unsorted",
        "[\"Y\"]",
    ),
    // bare_after_assigned
    (&[b"A=1", b"A"], "$ENV | own", "{}"),
    (&[b"A=1", b"A"], "env | own", "{\"A\":null}"),
    (&[b"A=1", b"A"], "$ENV.A", "null"),
    (&[b"A=1", b"A"], "env.A", "null"),
    (&[b"A=1", b"A"], "env | own | keys_unsorted", "[\"A\"]"),
    (&[b"A=1", b"A"], "$ENV | own | keys_unsorted", "[]"),
    // assigned_after_bare
    (&[b"A", b"A=1"], "$ENV | own", "{\"A\":\"1\"}"),
    (&[b"A", b"A=1"], "env | own", "{\"A\":\"1\"}"),
    (&[b"A", b"A=1"], "$ENV.A", "\"1\""),
    (&[b"A", b"A=1"], "env.A", "\"1\""),
    (&[b"A", b"A=1"], "env | own | keys_unsorted", "[\"A\"]"),
    (&[b"A", b"A=1"], "$ENV | own | keys_unsorted", "[\"A\"]"),
    // bare_mid
    (
        &[b"A=1", b"B=2", b"A", b"C=3"],
        "$ENV | own",
        "{\"B\":\"2\",\"C\":\"3\"}",
    ),
    (
        &[b"A=1", b"B=2", b"A", b"C=3"],
        "env | own",
        "{\"A\":null,\"B\":\"2\",\"C\":\"3\"}",
    ),
    (&[b"A=1", b"B=2", b"A", b"C=3"], "$ENV.A", "null"),
    (&[b"A=1", b"B=2", b"A", b"C=3"], "env.A", "null"),
    (
        &[b"A=1", b"B=2", b"A", b"C=3"],
        "env | own | keys_unsorted",
        "[\"A\",\"B\",\"C\"]",
    ),
    (
        &[b"A=1", b"B=2", b"A", b"C=3"],
        "$ENV | own | keys_unsorted",
        "[\"B\",\"C\"]",
    ),
    // bare_then_reassign
    (
        &[b"A=1", b"B=2", b"A", b"A=3"],
        "$ENV | own",
        "{\"B\":\"2\",\"A\":\"3\"}",
    ),
    (
        &[b"A=1", b"B=2", b"A", b"A=3"],
        "env | own",
        "{\"A\":\"3\",\"B\":\"2\"}",
    ),
    (&[b"A=1", b"B=2", b"A", b"A=3"], "$ENV.A", "\"3\""),
    (&[b"A=1", b"B=2", b"A", b"A=3"], "env.A", "\"3\""),
    (
        &[b"A=1", b"B=2", b"A", b"A=3"],
        "env | own | keys_unsorted",
        "[\"A\",\"B\"]",
    ),
    (
        &[b"A=1", b"B=2", b"A", b"A=3"],
        "$ENV | own | keys_unsorted",
        "[\"B\",\"A\"]",
    ),
    // bare_first_then_set_order
    (
        &[b"A", b"B=2", b"A=3"],
        "$ENV | own",
        "{\"B\":\"2\",\"A\":\"3\"}",
    ),
    (
        &[b"A", b"B=2", b"A=3"],
        "env | own",
        "{\"A\":\"3\",\"B\":\"2\"}",
    ),
    (&[b"A", b"B=2", b"A=3"], "$ENV.A", "\"3\""),
    (&[b"A", b"B=2", b"A=3"], "env.A", "\"3\""),
    (
        &[b"A", b"B=2", b"A=3"],
        "env | own | keys_unsorted",
        "[\"A\",\"B\"]",
    ),
    (
        &[b"A", b"B=2", b"A=3"],
        "$ENV | own | keys_unsorted",
        "[\"B\",\"A\"]",
    ),
    // empty_name
    (&[b"=v", b"Y=1"], "$ENV | own", "{\"\":\"v\",\"Y\":\"1\"}"),
    (&[b"=v", b"Y=1"], "env | own", "{\"\":\"v\",\"Y\":\"1\"}"),
    (&[b"=v", b"Y=1"], "$ENV.A", "null"),
    (&[b"=v", b"Y=1"], "env.A", "null"),
    (
        &[b"=v", b"Y=1"],
        "env | own | keys_unsorted",
        "[\"\",\"Y\"]",
    ),
    (
        &[b"=v", b"Y=1"],
        "$ENV | own | keys_unsorted",
        "[\"\",\"Y\"]",
    ),
    // eq_only
    (&[b"=", b"Y=1"], "$ENV | own", "{\"\":\"\",\"Y\":\"1\"}"),
    (&[b"=", b"Y=1"], "env | own", "{\"\":\"\",\"Y\":\"1\"}"),
    (&[b"=", b"Y=1"], "$ENV.A", "null"),
    (&[b"=", b"Y=1"], "env.A", "null"),
    (&[b"=", b"Y=1"], "env | own | keys_unsorted", "[\"\",\"Y\"]"),
    (
        &[b"=", b"Y=1"],
        "$ENV | own | keys_unsorted",
        "[\"\",\"Y\"]",
    ),
    // empty_entry
    (&[b"", b"Y=1"], "$ENV | own", "{\"Y\":\"1\"}"),
    (&[b"", b"Y=1"], "env | own", "{\"\":null,\"Y\":\"1\"}"),
    (&[b"", b"Y=1"], "$ENV.A", "null"),
    (&[b"", b"Y=1"], "env.A", "null"),
    (&[b"", b"Y=1"], "env | own | keys_unsorted", "[\"\",\"Y\"]"),
    (&[b"", b"Y=1"], "$ENV | own | keys_unsorted", "[\"Y\"]"),
    // empty_entry_after
    (&[b"Y=1", b""], "$ENV | own", "{\"Y\":\"1\"}"),
    (&[b"Y=1", b""], "env | own", "{\"Y\":\"1\",\"\":null}"),
    (&[b"Y=1", b""], "$ENV.A", "null"),
    (&[b"Y=1", b""], "env.A", "null"),
    (&[b"Y=1", b""], "env | own | keys_unsorted", "[\"Y\",\"\"]"),
    (&[b"Y=1", b""], "$ENV | own | keys_unsorted", "[\"Y\"]"),
    // dup
    (&[b"X=1", b"X=2"], "$ENV | own", "{\"X\":\"2\"}"),
    (&[b"X=1", b"X=2"], "env | own", "{\"X\":\"2\"}"),
    (&[b"X=1", b"X=2"], "$ENV.A", "null"),
    (&[b"X=1", b"X=2"], "env.A", "null"),
    (&[b"X=1", b"X=2"], "env | own | keys_unsorted", "[\"X\"]"),
    (&[b"X=1", b"X=2"], "$ENV | own | keys_unsorted", "[\"X\"]"),
    // value_with_eq
    (&[b"X=a=b"], "$ENV | own", "{\"X\":\"a=b\"}"),
    (&[b"X=a=b"], "env | own", "{\"X\":\"a=b\"}"),
    (&[b"X=a=b"], "$ENV.A", "null"),
    (&[b"X=a=b"], "env.A", "null"),
    (&[b"X=a=b"], "env | own | keys_unsorted", "[\"X\"]"),
    (&[b"X=a=b"], "$ENV | own | keys_unsorted", "[\"X\"]"),
    // empty_value
    (&[b"X="], "$ENV | own", "{\"X\":\"\"}"),
    (&[b"X="], "env | own", "{\"X\":\"\"}"),
    (&[b"X="], "$ENV.A", "null"),
    (&[b"X="], "env.A", "null"),
    (&[b"X="], "env | own | keys_unsorted", "[\"X\"]"),
    (&[b"X="], "$ENV | own | keys_unsorted", "[\"X\"]"),
    // bare_nonutf8
    (&[b"\xffA", b"Y=1"], "$ENV | own", "{\"Y\":\"1\"}"),
    (
        &[b"\xffA", b"Y=1"],
        "env | own",
        "{\"�A\":null,\"Y\":\"1\"}",
    ),
    (&[b"\xffA", b"Y=1"], "$ENV.A", "null"),
    (&[b"\xffA", b"Y=1"], "env.A", "null"),
    (
        &[b"\xffA", b"Y=1"],
        "env | own | keys_unsorted",
        "[\"�A\",\"Y\"]",
    ),
    (&[b"\xffA", b"Y=1"], "$ENV | own | keys_unsorted", "[\"Y\"]"),
    // nonutf8_assigned
    (&[b"N\xff=v\xfe"], "$ENV | own", "{\"N�\":\"v�\"}"),
    (&[b"N\xff=v\xfe"], "env | own", "{\"N�\":\"v�\"}"),
    (&[b"N\xff=v\xfe"], "$ENV.A", "null"),
    (&[b"N\xff=v\xfe"], "env.A", "null"),
    (&[b"N\xff=v\xfe"], "env | own | keys_unsorted", "[\"N�\"]"),
    (&[b"N\xff=v\xfe"], "$ENV | own | keys_unsorted", "[\"N�\"]"),
    // bare_dup
    (&[b"A", b"A"], "$ENV | own", "{}"),
    (&[b"A", b"A"], "env | own", "{\"A\":null}"),
    (&[b"A", b"A"], "$ENV.A", "null"),
    (&[b"A", b"A"], "env.A", "null"),
    (&[b"A", b"A"], "env | own | keys_unsorted", "[\"A\"]"),
    (&[b"A", b"A"], "$ENV | own | keys_unsorted", "[]"),
    // bare_after_empty_name
    (&[b"=v", b""], "$ENV | own", "{}"),
    (&[b"=v", b""], "env | own", "{\"\":null}"),
    (&[b"=v", b""], "$ENV.A", "null"),
    (&[b"=v", b""], "env.A", "null"),
    (&[b"=v", b""], "env | own | keys_unsorted", "[\"\"]"),
    (&[b"=v", b""], "$ENV | own | keys_unsorted", "[]"),
    // empty_name_dup
    (&[b"=1", b"=2"], "$ENV | own", "{\"\":\"2\"}"),
    (&[b"=1", b"=2"], "env | own", "{\"\":\"2\"}"),
    (&[b"=1", b"=2"], "$ENV.A", "null"),
    (&[b"=1", b"=2"], "env.A", "null"),
    (&[b"=1", b"=2"], "env | own | keys_unsorted", "[\"\"]"),
    (&[b"=1", b"=2"], "$ENV | own | keys_unsorted", "[\"\"]"),
];

#[test]
fn an_environment_entry_with_no_equals_sign_is_read_as_jq_reads_it_7734() -> Result<()> {
    for &(environ, program, want) in ROWS {
        let got = run_with_environ(environ, program)?;
        assert_eq!(
            (got.0.as_str(), got.1.as_str(), got.2),
            (&format!("{want}\n")[..], "", 0),
            "{environ:?}: {program}"
        );
    }
    Ok(())
}
