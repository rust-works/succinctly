//! #5554 (group 3): a value built past the tree ceiling (384 levels) by a program is printed the
//! way jq prints it -- 256 levels, then `<skipped: too deep>` and the closers -- where the output
//! check used to refuse it before a byte was written. Every expected string was captured from jq
//! 1.7.1 (`/usr/bin/jq`); the shape is the same at 300 and 1,000 levels.
//!
//! Not covered here, and still refused: a deep value consumed by another construct (`[., 1]`, a
//! comma after it), which fails inside the evaluator and not at the output.

use anyhow::Result;

use crate::run_jq_full;

/// jq's print of a deep array: 257 levels of `[`, the marker, 257 closers.
fn deep_array() -> String {
    format!("{}<skipped: too deep>{}", "[".repeat(257), "]".repeat(257))
}

fn deep_object_chain() -> String {
    format!(
        "{}<skipped: too deep>{}",
        "{\"a\":".repeat(257),
        "}".repeat(257)
    )
}

#[test]
fn a_deep_value_a_fold_builds_prints_as_jq_prints_it_5554() -> Result<()> {
    let want = format!("{}\n", deep_array());
    for (args, input) in [
        (vec!["-nc", "reduce range(1000) as $i ([]; .[0] = .)"], None),
        (vec!["-nc", "reduce range(300) as $i ([]; .[0] = .)"], None),
        // A document input takes the lazy route.
        (
            vec!["-c", "reduce range(1000) as $i ([]; .[0] = .)"],
            Some("1"),
        ),
        (
            vec!["-nc", "-S", "reduce range(1000) as $i ([]; .[0] = .)"],
            None,
        ),
        (
            vec!["-n", "-r", "-c", "reduce range(1000) as $i ([]; .[0] = .)"],
            None,
        ),
        (
            vec!["-nc", "-e", "reduce range(1000) as $i ([]; .[0] = .)"],
            None,
        ),
        (
            vec!["-c", "--slurp", "reduce range(1000) as $i ([]; .[0] = .)"],
            Some("1"),
        ),
    ] {
        let (stdout, stderr, code) = run_jq_full(&args, input)?;
        assert_eq!(
            (stdout.as_str(), code),
            (want.as_str(), 0),
            "{args:?}: {stderr}"
        );
    }
    Ok(())
}

#[test]
fn a_deep_value_among_other_outputs_prints_and_the_run_carries_on_5554() -> Result<()> {
    // Was: `1`, then exit 5 for the middle value, then `7` (`test_eager_over_depth_value...`).
    let want = format!("1\n{}\n7\n", deep_array());
    for (args, input) in [
        (vec!["-nc", "1, setpath([range(400)|0]; 1), 7"], None),
        (vec!["-nc", "-S", "1, setpath([range(400)|0]; 1), 7"], None),
        (
            vec!["-c", "--slurp", "1, setpath([range(400)|0]; 1), 7"],
            Some("null"),
        ),
        (vec!["-c", "1, setpath([range(400)|0]; 1), 7"], Some("null")),
    ] {
        let (stdout, stderr, code) = run_jq_full(&args, input)?;
        assert_eq!(
            (stdout.as_str(), code),
            (want.as_str(), 0),
            "{args:?}: {stderr}"
        );
    }
    Ok(())
}

#[test]
fn a_deep_value_inside_a_container_prints_where_it_sits_5554() -> Result<()> {
    let nested = format!("{}<skipped: too deep>{}", "[".repeat(256), "]".repeat(256));
    for (program, want) in [
        (
            "{a: setpath([range(500)|0]; 1), b: 2}",
            format!("{{\"a\":{nested},\"b\":2}}\n"),
        ),
        ("[setpath([range(400)|0]; 1), 2]", format!("[{nested},2]\n")),
        (
            "reduce range(1000) as $i ({}; .a = .)",
            format!("{}\n", deep_object_chain()),
        ),
    ] {
        for input in [None, Some("null")] {
            let args: Vec<&str> = if input.is_none() {
                vec!["-nc", program]
            } else {
                vec!["-c", program]
            };
            let (stdout, stderr, code) = run_jq_full(&args, input)?;
            assert_eq!(
                (stdout.as_str(), code),
                (want.as_str(), 0),
                "{program} over {input:?}: {stderr}"
            );
        }
    }
    Ok(())
}

/// A value read down to its length or kind still answers, and the one printed whole is unchanged.
#[test]
fn a_shallow_value_and_a_deep_values_length_are_unchanged_5554() -> Result<()> {
    let (stdout, _, code) = run_jq_full(
        &["-nc", "reduce range(1000) as $i ([]; .[0] = .) | length"],
        None,
    )?;
    assert_eq!((stdout.as_str(), code), ("1\n", 0));
    let (stdout, _, code) = run_jq_full(&["-nc", "reduce range(3) as $i ([]; .[0] = .)"], None)?;
    assert_eq!((stdout.as_str(), code), ("[[[[]]]]\n", 0));
    Ok(())
}
