//! #5306 (`INDEX`): `INDEX(.name)` over an array already held as a value no longer rebuilds a temporary document
//! from it, and reads a plain `.name` of an object row directly. Every row is a live jq 1.7.1 capture (stdout,
//! error message, exit code), both routes.

use anyhow::Result;

use crate::assert_path_rows_both_routes_3749;

#[test]
fn test_5306_index_over_an_owned_array_and_a_plain_field_key() -> Result<()> {
    assert_path_rows_both_routes_3749(&[
        ("null", "[{\"id\":1,\"n\":\"a\"},{\"id\":2,\"n\":\"b\"},{\"id\":1,\"n\":\"c\"}] | INDEX(.id)", "{\"1\":{\"id\":1,\"n\":\"c\"},\"2\":{\"id\":2,\"n\":\"b\"}}\n", "", 0),
        ("null", "[{\"id\":\"x\"},{\"id\":2.5},{\"id\":null},{\"id\":true},{\"id\":[1]},{\"id\":{\"a\":1}}] | INDEX(.id)", "{\"x\":{\"id\":\"x\"},\"2.5\":{\"id\":2.5},\"null\":{\"id\":null},\"true\":{\"id\":true},\"[1]\":{\"id\":[1]},\"{\\\"a\\\":1}\":{\"id\":{\"a\":1}}}\n", "", 0),
        ("null", "[{\"a\":1},{\"id\":3},{}] | INDEX(.id)", "{\"null\":{},\"3\":{\"id\":3}}\n", "", 0),
        ("null", "[null, {\"id\":1}] | INDEX(.id)", "{\"null\":null,\"1\":{\"id\":1}}\n", "", 0),
        ("null", "[] | INDEX(.id)", "{}\n", "", 0),
        ("null", "[{\"id\":1}] | INDEX(.id.x)", "", "Cannot index number with string \"x\"", 5),
        ("null", "{\"p\":{\"id\":1},\"q\":{\"id\":2}} | INDEX(.id)", "{\"1\":{\"id\":1},\"2\":{\"id\":2}}\n", "", 0),
        ("null", "[{\"id\":1},2] | INDEX(.id)", "", "Cannot index number with string \"id\"", 5),
        ("null", "[{\"id\":1},\"s\"] | INDEX(.id)", "", "Cannot index string with string \"id\"", 5),
        ("null", "[[1],[2]] | INDEX(.id)", "", "Cannot index array with string \"id\"", 5),
        ("null", "[{\"id\":1,\"w\":{\"k\":[1,2]}}] | INDEX(.id)", "{\"1\":{\"id\":1,\"w\":{\"k\":[1,2]}}}\n", "", 0),
        ("null", "[{\"id\":-0},{\"id\":1e3},{\"id\":1.0}] | INDEX(.id)", "{\"-0\":{\"id\":-0},\"1E+3\":{\"id\":1E+3},\"1.0\":{\"id\":1.0}}\n", "", 0),
        ("null", "[{\"id\":1},{\"id\":2}] | INDEX(.[]; .id)", "{\"1\":{\"id\":1},\"2\":{\"id\":2}}\n", "", 0),
        ("null", "5 | INDEX(.id)", "", "Cannot iterate over number (5)", 5),
        ("null", "null | INDEX(.id)", "", "Cannot iterate over null (null)", 5),
        ("null", "[{\"id\":\"b\"},{\"id\":\"a\"}] | INDEX(.id) | keys_unsorted", "[\"b\",\"a\"]\n", "", 0),
        ("null", "[{\"id\":1,\"v\":1},{\"id\":1,\"v\":2}] | INDEX(.id) | .[\"1\"].v", "2\n", "", 0),
    ])
}
