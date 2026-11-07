//! #6543: a block scalar header on a line of its own, left of the dash or key it would be the
//! value of, is refused as yq refuses it (`did not find expected` ...), where a chain of bare dashes
//! (`- - -`), a dash with only properties (`- - &x`) and the key or dash after an explicit `?` or
//! `:` (`? x` / `: - k:`, `- ? k:`, a bare `- ?`) took it as an item or a key's empty value. The
//! header at the owner's own column or to its right is still its value. Every row was captured
//! from yq v4.53.3: the refused rows are the whole grid of those lines above a `|` or `>-` header
//! at columns 0 to 6 that yq refuses. The wording of the refusal follows the container yq stops in
//! and is not pinned; the exit status and the empty stdout are.

use anyhow::Result;

use crate::{run_yq_split, run_yq_stdin};

const REFUSED: &[&str] = &[
    "- -\n|\n  t\n",
    "- -\n |\n   t\n",
    "- -\n>-\n  t\n",
    "- -\n >-\n   t\n",
    "- - -\n|\n  t\n",
    "- - -\n |\n   t\n",
    "- - -\n  |\n    t\n",
    "- - -\n   |\n     t\n",
    "- - -\n>-\n  t\n",
    "- - -\n >-\n   t\n",
    "- - -\n  >-\n    t\n",
    "- - -\n   >-\n     t\n",
    "- - &x\n|\n  t\n",
    "- - &x\n |\n   t\n",
    "- - &x\n>-\n  t\n",
    "- - &x\n >-\n   t\n",
    "-   -\n|\n  t\n",
    "-   -\n |\n   t\n",
    "-   -\n  |\n    t\n",
    "-   -\n   |\n     t\n",
    "-   -\n>-\n  t\n",
    "-   -\n >-\n   t\n",
    "-   -\n  >-\n    t\n",
    "-   -\n   >-\n     t\n",
    "- - # c\n|\n  t\n",
    "- - # c\n |\n   t\n",
    "- - # c\n>-\n  t\n",
    "- - # c\n >-\n   t\n",
    "? x\n: - k:\n|\n  t\n",
    "? x\n: - k:\n |\n   t\n",
    "? x\n: - k:\n  |\n    t\n",
    "? x\n: - k:\n   |\n     t\n",
    "? x\n: - k:\n>-\n  t\n",
    "? x\n: - k:\n >-\n   t\n",
    "? x\n: - k:\n  >-\n    t\n",
    "? x\n: - k:\n   >-\n     t\n",
    "? x\n: k:\n|\n  t\n",
    "? x\n: k:\n |\n   t\n",
    "? x\n: k:\n>-\n  t\n",
    "? x\n: k:\n >-\n   t\n",
    "? x\n: - -\n|\n  t\n",
    "? x\n: - -\n |\n   t\n",
    "? x\n: - -\n  |\n    t\n",
    "? x\n: - -\n   |\n     t\n",
    "? x\n: - -\n>-\n  t\n",
    "? x\n: - -\n >-\n   t\n",
    "? x\n: - -\n  >-\n    t\n",
    "? x\n: - -\n   >-\n     t\n",
    "- ? x\n  : k:\n|\n  t\n",
    "- ? x\n  : k:\n |\n   t\n",
    "- ? x\n  : k:\n  |\n    t\n",
    "- ? x\n  : k:\n   |\n     t\n",
    "- ? x\n  : k:\n>-\n  t\n",
    "- ? x\n  : k:\n >-\n   t\n",
    "- ? x\n  : k:\n  >-\n    t\n",
    "- ? x\n  : k:\n   >-\n     t\n",
    "- ? x\n  : -\n|\n  t\n",
    "- ? x\n  : -\n |\n   t\n",
    "- ? x\n  : -\n  |\n    t\n",
    "- ? x\n  : -\n   |\n     t\n",
    "- ? x\n  : -\n>-\n  t\n",
    "- ? x\n  : -\n >-\n   t\n",
    "- ? x\n  : -\n  >-\n    t\n",
    "- ? x\n  : -\n   >-\n     t\n",
    "? k:\n|\n  t\n",
    "? k:\n |\n   t\n",
    "? k:\n>-\n  t\n",
    "? k:\n >-\n   t\n",
    "- ? k:\n|\n  t\n",
    "- ? k:\n |\n   t\n",
    "- ? k:\n  |\n    t\n",
    "- ? k:\n   |\n     t\n",
    "- ? k:\n>-\n  t\n",
    "- ? k:\n >-\n   t\n",
    "- ? k:\n  >-\n    t\n",
    "- ? k:\n   >-\n     t\n",
    "a:\n  ? k:\n|\n  t\n",
    "a:\n  ? k:\n |\n   t\n",
    "a:\n  ? k:\n  |\n    t\n",
    "a:\n  ? k:\n   |\n     t\n",
    "a:\n  ? k:\n>-\n  t\n",
    "a:\n  ? k:\n >-\n   t\n",
    "a:\n  ? k:\n  >-\n    t\n",
    "a:\n  ? k:\n   >-\n     t\n",
    "? - k:\n|\n  t\n",
    "? - k:\n |\n   t\n",
    "? - k:\n  |\n    t\n",
    "? - k:\n   |\n     t\n",
    "? - k:\n>-\n  t\n",
    "? - k:\n >-\n   t\n",
    "? - k:\n  >-\n    t\n",
    "? - k:\n   >-\n     t\n",
    "- ?\n|\n  t\n",
    "- ?\n |\n   t\n",
    "- ?\n>-\n  t\n",
    "- ?\n >-\n   t\n",
];

#[test]
fn yq_refuses_a_header_left_of_its_dash_chain_or_explicit_key_6543() -> Result<()> {
    for doc in REFUSED {
        let (stdout, status) = run_yq_stdin(".", doc, &["-o=json", "-I0"])?;
        assert_eq!((stdout.as_str(), status), ("", 1), "{doc:?}");
    }
    Ok(())
}

#[test]
fn yq_keeps_a_header_at_or_right_of_its_owner_6543() -> Result<()> {
    for (doc, expected) in [
        ("- -\n  |\n    t\n", "[[\"t\\n\"]]"),
        ("- - -\n    |\n      t\n", "[[[\"t\\n\"]]]"),
        ("- - &x\n  |\n    t\n", "[[\"t\\n\"]]"),
        (
            "? x\n: - k:\n    |\n      t\n",
            "{\"x\":[{\"k\":\"t\\n\"}]}",
        ),
        ("? k:\n  |\n    t\n", "{\"\":null}"),
        ("- ? k:\n    |\n      t\n", "[{\"\":null}]"),
        ("? x\n: k:\n  |\n    t\n", "{\"x\":{\"k\":\"t\\n\"}}"),
        (
            "- ? x\n  : k:\n    |\n      t\n",
            "[{\"x\":{\"k\":\"t\\n\"}}]",
        ),
    ] {
        let (stdout, status) = run_yq_stdin(".", doc, &["-o=json", "-I0"])?;
        assert_eq!((stdout.trim_end(), status), (expected, 0), "{doc:?}");
    }
    Ok(())
}

/// Rows the generic stray-line refusal never took: the refusal comes from the owner check
/// (`did not find expected ...`), not from the arms that read a line with no owner.
#[test]
fn yq_refuses_a_header_left_of_its_owner_by_the_owner_check_6543() -> Result<()> {
    for doc in [
        "- -\n|\n  t\n",
        "- - -\n  |\n    t\n",
        "- - &x\n|\n  t\n",
        "- - # c\n|\n  t\n",
        "? k:\n|\n  t\n",
        "? x\n: - k:\n  |\n    t\n",
    ] {
        let (_stdout, stderr, status) = run_yq_split(".", doc, &["-o=json", "-I0"])?;
        assert_eq!(status, 1, "{doc:?}");
        assert!(
            stderr.contains("did not find expected"),
            "{doc:?}: {stderr}"
        );
    }
    Ok(())
}
