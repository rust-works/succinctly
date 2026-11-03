//! #6012 changes `delpaths` in jq mode only: yq refuses a path component that is not a `!!str` or
//! `!!int` before it deletes anything, so a slice descriptor over a string and a `null` key over a
//! `null` keep their wording. Both answers were captured from the same build before the change.

use anyhow::Result;

use crate::run_yq_stdin_with_stderr;

#[test]
fn yq_refuses_an_object_or_null_path_component_up_front() -> Result<()> {
    for (input, filter, tag) in [
        ("abc", r#"delpaths([[{"a":1},0]])"#, "!!map"),
        ("abc", r#"delpaths([[{"start":1,"end":2},0]])"#, "!!map"),
        ("[1,2]", r#"delpaths([[{"start":0,"end":1},0]])"#, "!!map"),
        ("null", "delpaths([[null,0]])", "!!null"),
    ] {
        let (out, err, code) = run_yq_stdin_with_stderr(filter, input, &["-o=json", "-I=0"])?;
        assert_eq!(out, "", "{filter} over {input}");
        assert_eq!(code, 1, "{filter} over {input}: {err}");
        assert!(
            err.contains(&format!(
                "DELPATHS: expected either a !!str or !!int in the path, found {tag} instead"
            )),
            "{filter} over {input}: {err}"
        );
    }
    Ok(())
}
