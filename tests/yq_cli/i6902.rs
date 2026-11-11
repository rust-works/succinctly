//! #6902: a field write through a target that iterates a path the document does not have and then
//! selects (`(.zz[] | select(.n == 1)).x = 5`) adds the `zz: []` yq creates, instead of raising
//! `Cannot iterate over null`. A stream of Kubernetes manifests fails on every document that lacks
//! the path otherwise. `=`, `|=` and the arithmetic operators all share the shape. Every row is
//! captured from yq v4.53.3; the rows over a path that is there, or that matches, are the controls
//! that were right before.

use anyhow::Result;

use crate::{run_yq_stdin, run_yq_stdin_with_stderr};

/// `(document, filter, output)`.
const ROWS: &[(&str, &str, &str)] = &[
    (
        "kind: Service\nspec:\n  type: x\n  ports:\n    - port: 80\n",
        "(.zz[] | select(.n == 1)).x = 5",
        "kind: Service\nspec:\n  type: x\n  ports:\n    - port: 80\nzz: []\n",
    ),
    (
        "kind: Service\nspec:\n  type: x\n  ports:\n    - port: 80\n",
        "(.zz[] | select(.n == 1)).x |= 5",
        "kind: Service\nspec:\n  type: x\n  ports:\n    - port: 80\nzz: []\n",
    ),
    (
        "kind: Service\nspec:\n  type: x\n  ports:\n    - port: 80\n",
        "(.zz[] | select(.n == 1)).x += 5",
        "kind: Service\nspec:\n  type: x\n  ports:\n    - port: 80\nzz: []\n",
    ),
    (
        "kind: Service\nspec:\n  type: x\n  ports:\n    - port: 80\n",
        "(.zz[] | select(.n == 1)).x -= 1",
        "kind: Service\nspec:\n  type: x\n  ports:\n    - port: 80\nzz: []\n",
    ),
    (
        "kind: Service\nspec:\n  type: x\n  ports:\n    - port: 80\n",
        "(.spec.zz[] | select(.n == 1)).x |= 5",
        "kind: Service\nspec:\n  type: x\n  ports:\n    - port: 80\n  zz: []\n",
    ),
    (
        "kind: Service\nspec:\n  type: x\n  ports:\n    - port: 80\n",
        "(.spec.zz[] | select(.n == 1)).x = 5",
        "kind: Service\nspec:\n  type: x\n  ports:\n    - port: 80\n  zz: []\n",
    ),
    (
        "kind: Service\nspec:\n  type: x\n  ports:\n    - port: 80\n",
        "(.zz[] | select(.n == 1) | .x) = 5",
        "kind: Service\nspec:\n  type: x\n  ports:\n    - port: 80\nzz: []\n",
    ),
    (
        "kind: Service\nspec:\n  type: x\n  ports:\n    - port: 80\n",
        "(.zz[] | select(.n == 1)).x.y = 5",
        "kind: Service\nspec:\n  type: x\n  ports:\n    - port: 80\nzz: []\n",
    ),
    (
        "kind: Service\nspec:\n  type: x\n  ports:\n    - port: 80\n",
        "(.zz[] | select(.n == 1)).x[1:2] = 5",
        "kind: Service\nspec:\n  type: x\n  ports:\n    - port: 80\nzz: []\n",
    ),
    (
        "kind: Service\nspec:\n  type: x\n  ports:\n    - port: 80\n",
        "(.a.b[].c[] | select(.n == 1)).x = 5",
        "kind: Service\nspec:\n  type: x\n  ports:\n    - port: 80\na:\n  b: []\n",
    ),
    (
        "kind: Service\nspec:\n  type: x\n  ports:\n    - port: 80\n",
        "(.zz | .[] | select(.n == 1)).x |= 5",
        "kind: Service\nspec:\n  type: x\n  ports:\n    - port: 80\nzz: []\n",
    ),
    (
        "kind: Service\nspec:\n  type: x\n  ports:\n    - port: 80\n",
        "(.zz[] | select(.n == 1)).x |= 5, .kind",
        "kind: Service\nspec:\n  type: x\n  ports:\n    - port: 80\nzz: []\nService\n",
    ),
    (
        "kind: Service\nspec:\n  type: x\n  ports:\n    - port: 80\n",
        "(.spec.ports[] | select(.port == 81)).x |= 5",
        "kind: Service\nspec:\n  type: x\n  ports:\n    - port: 80\n",
    ),
    (
        "kind: Service\nspec:\n  type: x\n  ports:\n    - port: 80\n",
        "(.spec.ports[] | select(.port == 80)).x |= 5",
        "kind: Service\nspec:\n  type: x\n  ports:\n    - port: 80\n      x: 5\n",
    ),
    (
        "kind: Service\nspec:\n  type: x\n  ports:\n    - port: 80\n",
        "(.spec.ports[] | select(.port == 80)).x = 5",
        "kind: Service\nspec:\n  type: x\n  ports:\n    - port: 80\n      x: 5\n",
    ),
    (
        "zz: ~\nk: 1\n",
        "(.zz[] | select(.n == 1)).x = 5",
        "zz: []\nk: 1\n",
    ),
    (
        "zz: ~\nk: 1\n",
        "(.zz[] | select(.n == 1)).x |= 5",
        "zz: []\nk: 1\n",
    ),
    (
        "zz:\n  - n: 1\n  - n: 2\nk: 1\n",
        "(.zz[] | select(.n == 1)).x = 5",
        "zz:\n  - n: 1\n    x: 5\n  - n: 2\nk: 1\n",
    ),
    (
        "zz:\n  - n: 1\n  - n: 2\nk: 1\n",
        "(.zz[] | select(.n == 3)).x |= 5",
        "zz:\n  - n: 1\n  - n: 2\nk: 1\n",
    ),
];

#[test]
fn test_write_through_select_over_a_missing_path_creates_it_6902() -> Result<()> {
    for (doc, filter, want) in ROWS {
        let (stdout, code) = run_yq_stdin(filter, doc, &[])?;
        assert_eq!(code, 0, "exit code of {filter:?}");
        assert_eq!(&stdout, want, "{filter:?} over {doc:?}");
    }
    Ok(())
}

/// What still raises, pinned so a change to it is a decision: a comma among the targets (the `null`
/// a missing key reads as can stand for a sibling's write), and an iteration after the select
/// (`.x[]`, where yq creates `x: []` under each node the select passed). Each row is `(document,
/// filter, yq's stdout)`.
#[test]
fn test_the_comma_and_iterate_after_select_forms_stay_recorded_6902() -> Result<()> {
    let svc = "kind: Service\nspec:\n  type: x\n";
    let rows: &[(&str, &str, &str)] = &[
        (
            svc,
            "(.zz[] | select(.n == 1), .kind).x = 5",
            "kind: Service\nspec:\n  type: x\nzz: []\n",
        ),
        (
            svc,
            "(.zz[] | select(.n == 1)).x[] = 5",
            "kind: Service\nspec:\n  type: x\nzz: []\n",
        ),
    ];
    for (doc, filter, yq) in rows {
        let (stdout, stderr, code) = run_yq_stdin_with_stderr(filter, doc, &[])?;
        assert_eq!((stdout.as_str(), code), ("", 1), "{filter:?}: {stderr}");
        assert!(
            stderr.contains("Cannot iterate over null"),
            "{filter:?}: {stderr}"
        );
        assert_ne!(
            stdout, *yq,
            "{filter:?} now matches yq: drop it from the limitations"
        );
    }
    Ok(())
}
