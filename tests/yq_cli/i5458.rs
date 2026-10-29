//! #5458: yq's `array_to_map`: an array becomes a map keyed by index with its null elements
//! skipped, a map keeps its non-null entries, anything else is `{}`. Every row was captured from
//! yq v4.53.3.

use anyhow::Result;

use crate::run_yq_stdin;

const DOC: &str = "b: [x, y, z]\nc: {d: 2}\nl: [{k: 1}, null, [2]]\ne: []\na: &x [1, 2]\nal: *x\nm: &y {k: 1, n: ~}\nma: *y\ns: &z 5\nsa: *z\nf: [false, 0, \"\", null]\n";

#[test]
fn test_array_to_map_matches_yq_5458() -> Result<()> {
    for (filter, want) in [
        (
            ".b | array_to_map",
            "{\"0\":\"x\",\"1\":\"y\",\"2\":\"z\"}\n",
        ),
        (".l | array_to_map", "{\"0\":{\"k\":1},\"2\":[2]}\n"),
        (".e | array_to_map", "{}\n"),
        (".c | array_to_map", "{\"d\":2}\n"),
        (".b[0] | array_to_map", "{}\n"),
        (".z | array_to_map", "{}\n"),
        // An alias has no tag of its own; yq resolves it.
        (".al | array_to_map", "{\"0\":1,\"1\":2}\n"),
        (".sa | array_to_map", "{}\n"),
        // A map keeps its non-null entries, through an alias too.
        (".m | array_to_map", "{\"k\":1}\n"),
        (".ma | array_to_map", "{\"k\":1}\n"),
        // False, 0 and "" are not null.
        (".f | array_to_map", "{\"0\":false,\"1\":0,\"2\":\"\"}\n"),
        (".b | array_to_map | length", "3\n"),
        (".b | array_to_map | .0", "\"x\"\n"),
        (".b | array_to_map | tag", "\"!!map\"\n"),
        ("[.b | array_to_map | .[]]", "[\"x\",\"y\",\"z\"]\n"),
    ] {
        let (stdout, code) = run_yq_stdin(filter, DOC, &["-o=json", "-I=0"])?;
        assert_eq!((stdout.as_str(), code), (want, 0), "`{filter}`");
    }
    // The whole document is a map, so its keys come back.
    let (stdout, code) = run_yq_stdin("array_to_map | keys | length", DOC, &["-o=json", "-I=0"])?;
    assert_eq!((stdout.as_str(), code), ("11\n", 0));
    // YAML output of scalars agrees too.
    let (stdout, code) = run_yq_stdin(".b | array_to_map", DOC, &[])?;
    assert_eq!((stdout.as_str(), code), ("0: x\n1: y\n2: z\n", 0));
    Ok(())
}
