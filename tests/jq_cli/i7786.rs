//! #7786: a chain of defs in which each def calls the one below it twice took time exponential in the
//! chain's length before any of it ran, where jq 1.7.1 answers at once: the analyses run before a
//! `reduce`, a `map`, an update or an `as` body walked a called def's body at every call, so the
//! chain was walked as a tree of 2^N calls. Every row is 60 defs deep (2^60 calls as a tree), and
//! each was captured from jq 1.7.1; jq 1.8.2 answers the same, but aborts on the module program
//! (an assertion in its `expand_call_arglist`).

#![cfg(unix)]

use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// `succinctly jq -nc <args>`'s stdout and exit status, or `None` when it has not ended within
/// `limit` (it is then killed). The rows below end in milliseconds; the limit only keeps a
/// regression from holding the suite.
fn run_within(args: &[&str], limit: Duration) -> Option<(String, i32)> {
    let mut child = Command::new(env!("CARGO_BIN_EXE_succinctly"))
        .arg("jq")
        .arg("-nc")
        .args(args)
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawned");
    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().expect("polled") {
            break status;
        }
        if started.elapsed() > limit {
            child.kill().expect("killed");
            child.wait().expect("reaped");
            return None;
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    let mut out = String::new();
    child
        .stdout
        .take()
        .expect("a pipe")
        .read_to_string(&mut out)
        .expect("read");
    Some((out, status.code().unwrap_or(-1)))
}

const LIMIT: Duration = Duration::from_secs(60);

/// The positions a call to the chain was walked from, `G` standing for the call, and jq 1.7.1's
/// answer for each.
const POSITIONS: &[(&str, &str)] = &[
    ("reduce empty as $i (0; G)", "0\n"),
    ("[foreach empty as $i (0; G)]", "[]\n"),
    ("[] | map(G)", "[]\n"),
    ("[] | sort_by(G)", "[]\n"),
    ("[] | any(G)", "false\n"),
    ("[] | .[] |= G", "[]\n"),
    ("{} | with_entries(G)", "{}\n"),
    ("[empty as $x | G]", "[]\n"),
    (
        "def loop($n): if $n <= 0 then 0 else ([] | map(G)) as $x | loop($n - 1) end; loop(3)",
        "0\n",
    ),
    ("[1, 2] | map(if . > 5 then G else . end)", "[1,2]\n"),
];

/// `def g0: 1; def g1: g0 + g0; ... def g60: g59 + g59;`
fn doubling_chain(levels: usize) -> String {
    let mut defs = String::from("def g0: 1; ");
    for k in 1..=levels {
        defs.push_str(&format!("def g{k}: g{} + g{}; ", k - 1, k - 1));
    }
    defs
}

/// `def g0: 1; def g1: 1; def g2: g1 + g0; ... def g60: g59 + g58;`
fn fibonacci_chain(levels: usize) -> String {
    let mut defs = String::from("def g0: 1; def g1: 1; ");
    for k in 2..=levels {
        defs.push_str(&format!("def g{k}: g{} + g{}; ", k - 1, k - 2));
    }
    defs
}

fn assert_every_position(defs: &str, call: &str, extra: &[&str]) {
    for (position, want) in POSITIONS {
        let program = format!("{defs}{}", position.replace('G', call));
        let mut args = extra.to_vec();
        args.push(&program);
        let got = run_within(&args, LIMIT);
        assert_eq!(
            got,
            Some(((*want).to_string(), 0)),
            "{position} with {call}"
        );
    }
}

#[test]
fn a_doubling_chain_of_defs_is_walked_once_7786() {
    assert_every_position(&doubling_chain(60), "g60", &[]);
}

#[test]
fn a_fibonacci_chain_of_defs_is_walked_once_7786() {
    assert_every_position(&fibonacci_chain(60), "g60", &[]);
}

#[test]
fn a_chain_runs_its_low_defs_beside_the_unrun_top_7786() {
    // The chain still evaluates: the defs low enough to run answer as jq does.
    let program = format!(
        "{}[g10, ([] | map(g60)), ([1] | map(g3))]",
        doubling_chain(60)
    );
    assert_eq!(
        run_within(&[&program], LIMIT),
        Some(("[1024,[],[8]]\n".to_string(), 0))
    );
}

#[test]
fn a_layered_module_dag_is_walked_once_7786() {
    // 40 layers of three modules, each module's def adding the defs of the three modules below.
    let dir = tempfile::tempdir().expect("tempdir");
    let layers = 40;
    for k in 0..layers {
        for j in 0..3 {
            let source = if k == layers - 1 {
                format!("def f{k}_{j}: {j};\n")
            } else {
                let imports: String = (0..3)
                    .map(|i| format!("import \"m{}_{i}\" as n{i}; ", k + 1))
                    .collect();
                let sum: Vec<String> = (0..3).map(|i| format!("n{i}::f{}_{i}", k + 1)).collect();
                format!("{imports}def f{k}_{j}: {};\n", sum.join(" + "))
            };
            std::fs::write(dir.path().join(format!("m{k}_{j}.jq")), source).expect("write module");
        }
    }
    let search = dir.path().to_str().expect("a UTF-8 path");
    assert_every_position("import \"m0_0\" as m; ", "m::f0_0", &["-L", search]);
}
