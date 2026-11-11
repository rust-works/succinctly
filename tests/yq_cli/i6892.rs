//! #6892: a `del()` whose only target iterates a path the document does not have and then selects
//! (`del(.zz[] | select(.n == 1))`) deletes nothing and leaves the document as it is, as yq does,
//! instead of raising `Cannot iterate over null`. The usual shape is a Kubernetes edit over a stream
//! of manifests, where most documents lack the path. Every row is captured from yq v4.53.3; the
//! rows over a path that is there are the controls that were right before.

use anyhow::Result;

use crate::{run_yq_stdin, run_yq_stdin_with_stderr};

/// `(document, filter, output)`.
const ROWS: &[(&str, &str, &str)] = &[
    ("kind: Service\nspec:\n  type: x\n  ports:\n    - port: 80\n", "del(.zz[] | select(.n == 1))", "kind: Service\nspec:\n  type: x\n  ports:\n    - port: 80\n"),
    ("kind: Service\nspec:\n  type: x\n  ports:\n    - port: 80\n", "del(.spec.template.spec.containers[] | select(.name == \"sidecar\"))", "kind: Service\nspec:\n  type: x\n  ports:\n    - port: 80\n"),
    ("kind: Service\nspec:\n  type: x\n  ports:\n    - port: 80\n", "del(.zz[] | select(true))", "kind: Service\nspec:\n  type: x\n  ports:\n    - port: 80\n"),
    ("kind: Service\nspec:\n  type: x\n  ports:\n    - port: 80\n", "del(.zz[] | select(.n) | .x)", "kind: Service\nspec:\n  type: x\n  ports:\n    - port: 80\n"),
    ("kind: Service\nspec:\n  type: x\n  ports:\n    - port: 80\n", "del(.zz.a[] | select(.n == 1))", "kind: Service\nspec:\n  type: x\n  ports:\n    - port: 80\n"),
    ("kind: Service\nspec:\n  type: x\n  ports:\n    - port: 80\n", "del(.zz | .[] | select(.n))", "kind: Service\nspec:\n  type: x\n  ports:\n    - port: 80\n"),
    ("kind: Service\nspec:\n  type: x\n  ports:\n    - port: 80\n", "del(.spec.type[] | select(.n == 1))", "kind: Service\nspec:\n  type: x\n  ports:\n    - port: 80\n"),
    ("kind: Service\nspec:\n  type: x\n  ports:\n    - port: 80\n", "del(.spec.ports[] | select(.port == 81))", "kind: Service\nspec:\n  type: x\n  ports:\n    - port: 80\n"),
    ("kind: Service\nspec:\n  type: x\n  ports:\n    - port: 80\n", "del(.spec.ports[] | select(.port == 80))", "kind: Service\nspec:\n  type: x\n  ports: []\n"),
    ("kind: Deployment\nspec:\n  template:\n    spec:\n      containers:\n        - name: web\n        - name: sidecar\n        - name: log\n", "del(.spec.template.spec.containers[] | select(.name == \"sidecar\"))", "kind: Deployment\nspec:\n  template:\n    spec:\n      containers:\n        - name: web\n        - name: log\n"),
    ("kind: Deployment\nspec:\n  template:\n    spec:\n      containers:\n        - name: web\n        - name: sidecar\n        - name: log\n", "del(.spec.template.spec.containers[] | select(.name == \"nope\"))", "kind: Deployment\nspec:\n  template:\n    spec:\n      containers:\n        - name: web\n        - name: sidecar\n        - name: log\n"),
    ("kind: Deployment\nspec:\n  template:\n    spec:\n      containers:\n        - name: web\n        - name: sidecar\n        - name: log\n", "del(.spec.template.spec.containers[] | select(.name == \"sidecar\" or .name == \"log\"))", "kind: Deployment\nspec:\n  template:\n    spec:\n      containers:\n        - name: web\n"),
    ("kind: Deployment\nspec:\n  template:\n    spec:\n      containers:\n        - name: web\n        - name: sidecar\n        - name: log\n", "del(.spec.template.spec.initContainers[] | select(.name == \"x\"))", "kind: Deployment\nspec:\n  template:\n    spec:\n      containers:\n        - name: web\n        - name: sidecar\n        - name: log\n"),
];

#[test]
fn test_del_through_select_over_a_missing_path_leaves_the_document_6892() -> Result<()> {
    for (doc, filter, want) in ROWS {
        let (stdout, code) = run_yq_stdin(filter, doc, &[])?;
        assert_eq!(code, 0, "exit code of {filter:?}");
        assert_eq!(&stdout, want, "{filter:?} over {doc:?}");
    }
    Ok(())
}

/// What still differs, pinned so a change to it is a decision (each yq answer is v4.53.3's): a
/// `del()` with a comma still raises for the sake of the placeholder rule `resolve_iterate_sink`
/// documents, and a field write through the same path (`(.zz[] | select(.n == 1)).x = 5`) raises
/// where yq adds `zz: []`; over a stored `null` the delete is a no-op here where yq turns the `null`
/// into `[]`. Each row is `(document, filter, this build's stdout, its exit code, yq's stdout)`.
#[test]
fn test_the_comma_write_and_stored_null_forms_stay_recorded_6892() -> Result<()> {
    let svc = "kind: Service\nspec:\n  type: x\n";
    let rows: &[(&str, &str, &str, i32, &str)] = &[
        (
            svc,
            "del(.zz[] | select(.n == 1), .kind)",
            "",
            1,
            "spec:\n  type: x\n",
        ),
        (
            svc,
            "(.zz[] | select(.n == 1)).x = 5",
            "",
            1,
            "kind: Service\nspec:\n  type: x\nzz: []\n",
        ),
        (
            "a: ~\nb: 1\n",
            "del(.a[] | select(.n == 1))",
            "a: ~\nb: 1\n",
            0,
            "a: []\nb: 1\n",
        ),
        ("~\n", "del(.[] | select(.n))", "~\n", 0, "[]\n"),
    ];
    for (doc, filter, ours, code, yq) in rows {
        let (stdout, stderr, got) = run_yq_stdin_with_stderr(filter, doc, &[])?;
        assert_eq!(
            (stdout.as_str(), got),
            (*ours, *code),
            "{filter:?} over {doc:?}"
        );
        if *code != 0 {
            assert!(
                stderr.contains("Cannot iterate over null"),
                "{filter:?}: {stderr}"
            );
        }
        assert_ne!(
            ours, yq,
            "{filter:?} now matches yq: drop it from the limitations"
        );
    }
    Ok(())
}
