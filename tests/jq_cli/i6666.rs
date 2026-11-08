//! #6666: a loop over an array the program built reads it by index or by a narrow slice through
//! the element index, not by walking it per read. The answers are jq 1.7.1's (captured from it);
//! the timing test is a floor under the mechanism's own pin (`jq::eval::tests`): 40,000 reads
//! of a 40,000-element array took 40 s before, and 200,000 now take about 0.1 s.

use anyhow::Result;

use crate::run_jq_null;

#[test]
fn test_jq_index_and_slice_loops_over_a_built_array_match_jq_6666() -> Result<()> {
    for (filter, want) in [
        (
            "[range(100)] | [range(0;100;7) as $i | .[$i]] | length",
            "15\n",
        ),
        (
            "[range(100)] | [range(-105;105;30) as $i | .[$i]]",
            "[null,25,55,85,15,45,75]\n",
        ),
        (
            "[range(100)] | [range(-105;105;70) as $i | .[$i:$i+3]]",
            "[[],[65,66,67],[35,36,37]]\n",
        ),
        (
            "[range(100)] | [range(0;300) as $i | .[3:7] | add] | add",
            "5400\n",
        ),
        (
            "[range(100)] | [range(0;300) as $i | .[-3:] | add] | add",
            "88200\n",
        ),
        (
            "[range(100)] | [range(0;300) as $i | .[0:] | length] | add",
            "30000\n",
        ),
        (
            "[range(100)] | [range(0;300) as $i | .[60:50] | length] | add",
            "0\n",
        ),
        (
            "[range(100)] | [range(0;300) as $i | .[1.5:3.5]] | .[0]",
            "[1,2,3]\n",
        ),
        (
            "[nan, 1] as $a | [range(0;300) as $i | ($a | .[0:] | .) == $a] | .[0]",
            "true\n",
        ),
        ("[range(100)] | [range(0;100) as $i | .[$i]] == .", "true\n"),
        // `length` and `has` in the loop, and a wide object's keys (the memoized length and key
        // index, not a walk per read).
        ("[range(100)] | [range(0;100;7) as $i | length] | add", "1500\n"),
        ("[range(100)] | [range(0;100;9) as $i | .[length - 1 - $i]]", "[99,90,81,72,63,54,45,36,27,18,9,0]\n"),
        ("[range(100)] | [range(-3;104;9) as $i | has($i)] | map(select(.)) | length", "11\n"),
        ("[range(100)|{key:tostring,value:.}] | from_entries | [range(0;110;11) as $i | .[$i|tostring]]", "[0,11,22,33,44,55,66,77,88,99]\n"),
        ("[range(100)|{key:tostring,value:.}] | from_entries | [range(0;110;11) as $i | has($i|tostring)] | map(select(.)) | length", "10\n"),
    ] {
        let (stdout, code) = run_jq_null(filter, &["-c"])?;
        assert_eq!((stdout.as_str(), code), (want, 0), "{filter}");
    }
    Ok(())
}

#[test]
fn test_jq_index_loop_over_a_wide_built_array_is_not_quadratic_6666() -> Result<()> {
    // 200,000 reads of a 200,000-element array took minutes before. The element-index counters in
    // `jq::eval::tests` pin the mechanism; this only fails a loop that has gone quadratic again,
    // by a bound far above any runner's speed.
    for (filter, want) in [
        (
            "[range(200000)] | [range(0; length; 1) as $i | .[$i]] | length",
            "200000\n",
        ),
        (
            "[range(200000)] | [range(0; length; 1) as $i | .[$i:$i+2]] | length",
            "200000\n",
        ),
        // Streaming: the loop emits per read, with no collecting `[ ]` around it.
        (
            "[range(200000)] | [range(0; length; 1) as $i | .[$i]] | last",
            "199999\n",
        ),
        (
            "[range(200000)] | last(range(0; length; 1) as $i | .[$i])",
            "199999\n",
        ),
        (
            "[range(200000)] | last(range(0; length; 1) as $i | .[$i:$i+2][0])",
            "199999\n",
        ),
        (
            "[range(100000)|{key:tostring,value:.}] | from_entries | last(range(0; 100000) as $i | .[$i|tostring])",
            "99999\n",
        ),
        (
            "[range(200000)] | [range(0; length; 2) as $i | length] | add",
            "20000000000\n",
        ),
    ] {
        let started = std::time::Instant::now();
        let (stdout, code) = run_jq_null(filter, &["-c"])?;
        assert_eq!((stdout.as_str(), code), (want, 0), "{filter}");
        assert!(
            started.elapsed() < std::time::Duration::from_secs(60),
            "{filter} took {:?}: the loop reads the array per iteration again",
            started.elapsed()
        );
    }
    Ok(())
}
