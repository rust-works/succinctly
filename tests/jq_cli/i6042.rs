//! #6042: a `-0` written as a program literal keeps its sign when path mode takes it as the register
//! value (`path(error)`, `pick(error)`, `del(empty)`). The literal is `Int(0)` spelled `-0`, and the
//! path-mode literal arm used to rebuild it as a plain `Int(0)`. Every row is what `/usr/bin/jq` 1.7.1
//! printed for the same program.

use anyhow::Result;

use crate::run_jq_full;

/// `-nc PROGRAM`: stdout of a run that must succeed.
fn out(program: &str) -> Result<String> {
    let (stdout, _stderr, code) = run_jq_full(&["-nc", program], None)?;
    assert_eq!(code, 0, "`{program}` failed");
    Ok(stdout.trim().to_string())
}

#[test]
fn a_negative_zero_literal_keeps_its_sign_through_path_mode() -> Result<()> {
    let rows: &[(&str, &str)] = &[
        ("-0 | del(empty)", "-0"),
        ("[-0|try path(error) catch .]", "[-0]"),
        ("[-0|try pick(error) catch .]", "[-0]"),
        ("[-0|try error catch .]", "[-0]"),
        // The other spellings of the same negation.
        ("[-00|try path(error) catch .]", "[-0]"),
        ("[- 0|try path(error) catch .]", "[-0]"),
        ("[-(0)|try path(error) catch .]", "[-0]"),
        ("[-0e0|try path(error) catch .]", "[-0]"),
        ("-00 | del(empty)", "-0"),
        // Neighbours that must not change: a read `-0` float, a computed zero, and negatives that
        // are plain integers.
        ("[-0.0|try path(error) catch .]", "[-0]"),
        ("[0|-.]|try path(.[0]|error) catch .", "-0"),
        ("[-1,-0,-2]|try path(error) catch .", "[-1,-0,-2]"),
        ("[0,-1]|try path(error) catch .", "[0,-1]"),
        ("[-0|path(.)]", "[[]]"),
    ];
    for (program, want) in rows {
        assert_eq!(out(program)?, *want, "`{program}`");
    }
    Ok(())
}
