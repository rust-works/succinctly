//! #7094: yq runs `with(PATH; UPDATE)` with each node as the context of `UPDATE`, and a binding
//! there (`SRC as $v | WRITES`) binds each value of `SRC` in turn, every one of whose writes lands on
//! the same node: the last value's write wins and `+=` accumulates. A `|=` body that is such a
//! binding does the same. Every answer was captured from yq v4.53.3.

use anyhow::Result;

use crate::run_yq_stdin;

/// (input, filter, yq's stdout).
const ROWS: &[(&str, &str, &str)] = &[
    (
        "k: {x: [1, 2]}\n",
        "with(.k; .x[] as $v | .y = $v)",
        "k: {x: [1, 2], y: 2}\n",
    ),
    (
        "k: {x: [1, 2]}\n",
        "with(.k; .x[] as $v | .y += $v)",
        "k: {x: [1, 2], y: 3}\n",
    ),
    (
        "k: {x: [1, 2]}\n",
        "with(.k; .x[] as $v | .y = $v, .w = 1)",
        "k: {x: [1, 2], y: 2, w: 1}\n",
    ),
    (
        "k: {x: [1, 2]}\n",
        "with(.k; .x[] as $v | .y += $v, .w = 1)",
        "k: {x: [1, 2], y: 3, w: 1}\n",
    ),
    (
        "k: {x: [1, 2]}\n",
        "with(.k; .x[] as $v | .y |= . + $v)",
        "k: {x: [1, 2], y: 3}\n",
    ),
    (
        "k: {x: [1, 2]}\n",
        "with(.k; .x[] as $v | .y = $v | .z += 1)",
        "k: {x: [1, 2], y: 2, z: 2}\n",
    ),
    (
        "k: {x: [1, 2]}\n",
        "with(.k; .x[] as $v | .x[] as $w | .y += [[$v, $w]])",
        "k: {x: [1, 2], y: [[1, 1], [1, 2], [2, 1], [2, 2]]}\n",
    ),
    (
        "k: {x: [1, 2]}\n",
        "with(.k; .x[] as $v | .y += $v | .x[] as $w | .z += $w)",
        "k: {x: [1, 2], y: 3, z: 6}\n",
    ),
    (
        "k: {x: [1, 2]}\n",
        "with(.k; .x[] as $v | (.y += $v | .x[] as $w | .z += $w))",
        "k: {x: [1, 2], y: 3, z: 6}\n",
    ),
    (
        "k: {x: [1, 2, 3]}\n",
        "with(.k; .x[] as $v | select($v > 1) | .y += $v)",
        "k: {x: [1, 2, 3], y: 5}\n",
    ),
    (
        "k: {x: [1, 2, 3]}\n",
        "with(.k; .x[] as $v | select($v > 5) | .y += $v)",
        "k: {x: [1, 2, 3]}\n",
    ),
    (
        "k: {x: [a, b]}\n",
        "with(.k; .x[] as $v | .[$v] = 1)",
        "k: {x: [a, b], a: 1, b: 1}\n",
    ),
    (
        "k: {x: [a, b]}\n",
        "with(.k; .x[] as $v | del(.[$v]))",
        "k: {x: [a, b]}\n",
    ),
    (
        "k: {x: [a, b], a: 1, b: 2, c: 3}\n",
        "with(.k; .x[] as $v | del(.[$v]))",
        "k: {x: [a, b], c: 3}\n",
    ),
    (
        "k: {x: [1, 2], e: []}\n",
        "with(.k; .e[] as $v | .y = $v)",
        "k: {x: [1, 2], e: [], y: null}\n",
    ),
    (
        "k: {x: [], z: 1}\n",
        "with(.k; .x[] as $v | .[$v] = 1)",
        "k: {x: 1, z: 1}\n",
    ),
    (
        "k: {x: [1, 2]}\n",
        "with(.k; .nope as $v | .y = ($v | tostring))",
        "k: {x: [1, 2], y: null}\n",
    ),
    (
        "k: {x: [1, 2]}\n",
        "with(.k; (.x[0], .nope) as $v | .y += [($v | tostring)])",
        "k: {x: [1, 2], y: [\"1\"]}\n",
    ),
    (
        "k: {x: [1, 2]}\n",
        "with(.k; (.nope, .x[0]) as $v | .y += [($v | tostring)])",
        "k: {x: [1, 2], y: [\"1\"]}\n",
    ),
    (
        "k: {x: [1, 2]}\n",
        "with(.k; (.nope, .x[]) as $v | .y += [$v])",
        "k: {x: [1, 2], y: [1, 2]}\n",
    ),
    (
        "k: {x: [1, 2]}\n",
        "with(.k; .x as $v | .y = $v)",
        "k: {x: [1, 2], y: [1, 2]}\n",
    ),
    (
        "k: {x: [1, 2]}\n",
        "with(.k; .x[0] as $v | .y = $v)",
        "k: {x: [1, 2], y: 1}\n",
    ),
    (
        "k: {x: [1, 2]}\n",
        "with(.k; (1, 2, 3) as $v | .y += $v)",
        "k: {x: [1, 2], y: 6}\n",
    ),
    (
        "k: {x: [1, 2]}\n",
        "with(.k; (.x[] | . * 10) as $v | .y += [$v])",
        "k: {x: [1, 2], y: [10, 20]}\n",
    ),
    (
        "k: {x: [1, 2]}\n",
        "with(.k; (.x[] | . + 10) as $v | .x[1] = 9 | .y += [$v])",
        "k: {x: [1, 9], y: [11, 12]}\n",
    ),
    (
        "k: {x: [1, 2]}\n",
        "with(.k; .x[] as $v | del(.x[0]) | .y += [$v])",
        "k: {x: [], y: [1, 2]}\n",
    ),
    (
        "k: {x: [1, 2]}\n",
        "with(.k; .x[] as $v | .x = [7, 8] | .y += [$v])",
        "k: {x: [7, 8], y: [1, 2]}\n",
    ),
    (
        "k: {x: [1, 2]}\n",
        "with(.k; .x[] as $v | .x[0] = 9 | .y += [$v])",
        "k: {x: [9, 2], y: [1, 2]}\n",
    ),
    (
        "k: # c\n  x: [1, 2] # lc\n  y: 0 # yc\n",
        "with(.k; .x[] as $v | .y += $v)",
        "k: # c\n  x: [1, 2] # lc\n  y: 3 # yc\n",
    ),
    (
        "# head\nk: {x: [1, 2]} # kc\nz: 1 # zc\n",
        "with(.k; .x[] as $v | .y += $v)",
        "# head\nk: {x: [1, 2], y: 3} # kc\nz: 1 # zc\n",
    ),
    (
        "k: [{x: [1, 2]}, {x: [3]}]\n",
        "with(.k[]; .x[] as $v | .y += $v)",
        "k: [{x: [1, 2], y: 3}, {x: [3], y: 3}]\n",
    ),
    (
        "k: {x: [1, 2]}\n---\nk: {x: [3, 4, 5]}\n",
        "with(.k; .x[] as $v | .y += $v)",
        "k: {x: [1, 2], y: 3}\n---\nk: {x: [3, 4, 5], y: 12}\n",
    ),
    (
        "k: {x: [1, 2]}\n",
        ".k |= (.x[] as $v | .y += $v)",
        "k: {x: [1, 2], y: 3}\n",
    ),
    (
        "k: {x: [1, 2]}\n",
        ".k |= (.x[] as $v | .y = $v, .w = 1)",
        "k: {x: [1, 2], y: 2, w: 1}\n",
    ),
    (
        "k: {x: [1, 2]}\n",
        ".k |= (.x[] as $v | .y += $v, .w = 1)",
        "k: {x: [1, 2], y: 3, w: 1}\n",
    ),
    (
        "k: {x: [1, 2]}\n",
        ".k |= (.x[] as $v | (.y += $v, .w = 1))",
        "k: {x: [1, 2], y: 3, w: 1}\n",
    ),
    (
        "k: {x: [1, 2]}\n",
        ".k |= ((.x[] as $v | .y += $v), .z = 1)",
        "k: {x: [1, 2], y: 3, z: 1}\n",
    ),
    (
        "k: {x: [1, 2]}\n",
        ".k |= (.x[] as $v | .y += $v | .w = 1)",
        "k: {x: [1, 2], y: 3, w: 1}\n",
    ),
    (
        "k: {x: [1, 2, 3]}\n",
        ".k |= (.x[] as $v | select($v > 1) | .y += $v)",
        "k: {x: [1, 2, 3], y: 5}\n",
    ),
    (
        "k: {x: [1, 2, 3]}\n",
        ".k |= (.x[] as $v | select($v > 5) | .y += $v)",
        "k: {x: [1, 2, 3]}\n",
    ),
    (
        "k: {x: [1, 2, 3]}\n",
        ".k |= (.x[] as $v | select($v < 2) | .y += $v)",
        "k: {x: [1, 2, 3], y: 1}\n",
    ),
    (
        "k: {x: [1, 2, 3]}\n",
        ".k |= (.x[] as $v | .y += $v | .x[] as $w | .z += $w)",
        "k: {x: [1, 2, 3], y: 6, z: 18}\n",
    ),
    (
        "k: {x: [1, 2, 3]}\n",
        ".k |= (.x[] as $v | (.y += $v, 7))",
        "k: {x: [1, 2, 3], y: 6}\n",
    ),
    (
        "k: {x: [1, 2, 3]}\n",
        ".k |= ((.x[] as $v | .y += $v), 7)",
        "k: {x: [1, 2, 3], y: 6}\n",
    ),
    (
        "k: {x: [1, 2, 3]}\n",
        ".k |= (.x[] as $v | .y += $v | $v)",
        "k: 1\n",
    ),
    ("k: {x: [1, 2, 3]}\n", ".k |= (.x[] as $v | $v)", "k: 1\n"),
    (
        "k: {x: [1, 2, 3]}\n",
        ".k |= (.x[] as $v | .)",
        "k: {x: [1, 2, 3]}\n",
    ),
    (
        "k: {x: [1, 2, 3]}\n",
        ".k |= (.x[] as $v | select($v > 1))",
        "k: {x: [1, 2, 3]}\n",
    ),
    (
        "k: {x: [1, 2, 3]}\n",
        ".k.z |= (.x[] as $v | .)",
        "k: {x: [1, 2, 3], z: null}\n",
    ),
    (
        "x: [1, 2]\n",
        "with(.k; (.x[] as $v | .y += $v) | .z = 2)",
        "x: [1, 2]\nk:\n  y: null\n  z: 2\n",
    ),
];

/// Recorded differences (`docs/compliance/yq/limitations.md`): the values are the source's as the
/// node stood before the first write, where yq copies each bound node when its turn comes, so a
/// write to a node a later value is bound to is seen in yq and not here; and a bound value loses its
/// comment on the way, as `[.[]]` does (#5429). (input, filter, succinctly's stdout; yq's is in the
/// comment.)
const RESIDUAL_ROWS: &[(&str, &str, &str)] = &[
    // yq: "k: {x: [21, 22], y: [1, 12]}\n"
    (
        "k: {x: [1, 2]}\n",
        "with(.k; .x[] as $v | .x[] |= . + 10 | .y += [$v])",
        "k: {x: [21, 22], y: [1, 2]}\n",
    ),
    // yq: "k: {x: [1, 1]}\n"
    (
        "k: {x: [1, 2]}\n",
        "with(.k; .x[] as $v | .x[] = $v)",
        "k: {x: [2, 2]}\n",
    ),
    // yq: "k: {x: [1, 9], y: [1, 9]}\n"
    (
        "k: {x: [1, 2]}\n",
        "with(.k; .x[] as $v | .y += [$v] | .x[1] = 9)",
        "k: {x: [1, 9], y: [1, 2]}\n",
    ),
    // yq: "k:\n  x:\n    - \"1\"\n    - \"2\" # two\n  y: \"2\" # two\n"
    (
        "k:\n  x:\n    - \"1\"\n    - \"2\" # two\n",
        "with(.k; .x[] as $v | .y = $v)",
        "k:\n  x:\n    - \"1\"\n    - \"2\" # two\n  y: \"2\"\n",
    ),
];

#[test]
fn a_binding_over_several_values_writes_each_value_7094() -> Result<()> {
    for (input, filter, expected) in ROWS.iter().chain(RESIDUAL_ROWS) {
        let (stdout, status) = run_yq_stdin(filter, input, &[])?;
        assert_eq!(
            (stdout.as_str(), status),
            (*expected, 0),
            "{filter} over {input:?}"
        );
    }
    Ok(())
}

/// The body of a binding is written once in the fold, so chained bindings over several values grow
/// the parsed filter linearly (a draft that copied the body for the one-value case doubled it at
/// each binding, and twenty took more than 3 GB to parse). One value per source keeps the run
/// itself to one pass per binding.
#[test]
fn chained_bindings_over_a_sequence_parse_in_bounded_memory_7094() -> Result<()> {
    let started = std::time::Instant::now();
    let bindings: Vec<String> = (0..20)
        .map(|i| format!(".x[] as $v{i} | .y += $v{i}"))
        .collect();
    let filter = format!("with(.k; {})", bindings.join(" | "));
    let (stdout, status) = run_yq_stdin(&filter, "k: {x: [1]}\n", &[])?;
    assert_eq!((stdout.as_str(), status), ("k: {x: [1], y: 20}\n", 0));
    assert!(started.elapsed() < std::time::Duration::from_secs(10));
    Ok(())
}
