//! #7260: `-sR` builds its one string from the first input's own buffer and appends the rest, so a
//! single input is moved rather than copied. The joined text, file order and empty files must come
//! out as jq 1.7.1 prints them. Every row was captured from jq 1.7.1.

use anyhow::Result;

use crate::run_jq_full;

#[test]
fn raw_slurp_joins_inputs_in_order_7260() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let path = |name: &str| dir.path().join(name).to_string_lossy().into_owned();
    let (one, two, empty) = (path("one.txt"), path("two.txt"), path("empty.txt"));
    std::fs::write(&one, "a\nb\n")?;
    std::fs::write(&two, "c")?;
    std::fs::write(&empty, "")?;
    for (files, expected) in [
        (vec![&one], "[\"a\\nb\\n\",4]\n"),
        (vec![&empty], "[\"\",0]\n"),
        (vec![&one, &empty, &two], "[\"a\\nb\\nc\",5]\n"),
        (vec![&two, &one], "[\"ca\\nb\\n\",5]\n"),
        (vec![&empty, &one], "[\"a\\nb\\n\",4]\n"),
    ] {
        let mut args = vec!["-sRc", "[., length]"];
        args.extend(files.iter().map(|f| f.as_str()));
        let (out, err, code) = run_jq_full(&args, None)?;
        assert_eq!(
            (out.as_str(), err.as_str(), code),
            (expected, "", 0),
            "{files:?}"
        );
    }
    Ok(())
}
