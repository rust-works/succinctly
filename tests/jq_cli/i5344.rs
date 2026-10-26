//! #5344: the metadata of a `module` header or an `import`/`include` directive is an ordinary
//! expression that jq folds to a constant object, not an object literal. Expected values captured
//! live against jq 1.7.1 (`/usr/bin/jq`): `modulemeta` prints the folded metadata, a number
//! literal keeps its spelling, and an expression that does not fold is `Module metadata must be
//! constant` (or `must be an object`) at the position of the expression, whatever the directive.

use anyhow::Result;

use crate::run_jq_in_dir;

/// `modulemeta` of a module `lib/t.jq` holding `header` and `def z: 1;`.
fn modulemeta(header: &str) -> Result<(String, String, i32)> {
    let dir = tempfile::tempdir()?;
    std::fs::create_dir(dir.path().join("lib"))?;
    std::fs::write(dir.path().join("lib/m.jq"), "def a: 1;")?;
    std::fs::write(
        dir.path().join("lib/t.jq"),
        format!("{header}\ndef z: 1;\n"),
    )?;
    run_jq_in_dir(
        dir.path(),
        &["-c", "-L", "lib", "modulemeta"],
        Some("\"t\""),
    )
}

#[test]
#[allow(clippy::literal_string_with_formatting_args)] // jq object literals read like format arguments
fn test_module_metadata_folds_constant_expressions_5344() -> Result<()> {
    for (header, want) in [
        // Arithmetic and comparison over constants, in jq's own kind rules.
        ("module {a:(1+2)};", r#"{"a":3,"#),
        ("module {a:[1,(2+3)]};", r#"{"a":[1,5],"#),
        (r#"module {a:("x"+"y")};"#, r#"{"a":"xy","#),
        (r#"module {a:("x"*2)};"#, r#"{"a":"xx","#),
        ("module {a:(10/4)};", r#"{"a":2.5,"#),
        ("module {a:(10%4)};", r#"{"a":2,"#),
        ("module {a:(1<2)};", r#"{"a":true,"#),
        ("module {a:(1==1)};", r#"{"a":true,"#),
        (r#"module {a:("a"<"b")};"#, r#"{"a":true,"#),
        ("module {a:(null+1)};", r#"{"a":1,"#),
        ("module {a:([1]+[2])};", r#"{"a":[1,2],"#),
        (r#"module {a:({"x":1}+{"y":2})};"#, r#"{"a":{"x":1,"y":2},"#),
        // The object itself may be computed, or parenthesised.
        ("module ({a:1});", r#"{"a":1,"#),
        ("module ({a:1}+{b:2});", r#"{"a":1,"b":2,"#),
        ("module {a:1}+{b:2};", r#"{"a":1,"b":2,"#),
        // Keys: a keyword, a quoted string, a parenthesised constant string; the last of a
        // repeated key wins; a trailing comma is fine.
        (
            "module {if:1, then:2, def:3, import:4};",
            r#"{"if":1,"then":2,"def":3,"import":4,"#,
        ),
        (r#"module {"a b":1};"#, r#"{"a b":1,"#),
        (r#"module {("a"):1};"#, r#"{"a":1,"#),
        ("module {a:1,a:2};", r#"{"a":2,"#),
        ("module {a:1,};", r#"{"a":1,"#),
        ("module {};", r#"{"deps""#),
        // A format with nothing to interpolate is a plain string.
        (r#"module {a:@base64 "x"};"#, r#"{"a":"x","#),
        (
            "module {a:{b:{c:[1,2,{d:null}]}}};",
            r#"{"a":{"b":{"c":[1,2,{"d":null}]}},"#,
        ),
        // A number literal keeps the spelling it was written with; a computed one does not.
        ("module {a:1.0};", r#"{"a":1.0,"#),
        ("module {a:1.50};", r#"{"a":1.50,"#),
        (
            "module {a:100000000000000000001};",
            r#"{"a":100000000000000000001,"#,
        ),
        ("module {a:0.1e2};", r#"{"a":1E+1,"#),
        ("module {a:[1.0,{b:2.50}]};", r#"{"a":[1.0,{"b":2.50}],"#),
        (
            "module {a:true, b:false, c:null};",
            r#"{"a":true,"b":false,"c":null,"#,
        ),
        // A `-` in an exponent is part of the number, not a unary minus.
        ("module {a:1e-5};", r#"{"a":0.00001,"#),
        ("module {a:1E-5, b:2.5e+3};", r#"{"a":0.00001,"b":2.5E+3,"#),
        // A `-` that subtracts, or sits in a string or a comment, is not a unary minus.
        ("module {a:(5-3)};", r#"{"a":2,"#),
        ("module {a:(5 - 3)};", r#"{"a":2,"#),
        (r#"module {"a-b":"-1"};"#, r#"{"a-b":"-1","#),
        ("module {a:1 # -1\n};", r#"{"a":1,"#),
    ] {
        let (stdout, stderr, code) = modulemeta(header)?;
        assert!(
            stdout.starts_with(want) && code == 0,
            "{header}: wanted {want} first, got {stdout:?} ({stderr})"
        );
    }
    Ok(())
}

#[test]
#[allow(clippy::literal_string_with_formatting_args)] // jq object literals read like format arguments
fn test_module_metadata_that_does_not_fold_is_refused_5344() -> Result<()> {
    for header in [
        // A unary minus is a call in jq, not a constant: no negative metadata number.
        "module {a:-1};",
        "module {a:-(1)};",
        "module {a:-(1+1)};",
        "module {a:-0};",
        // An operator that fails, or one jq does not fold.
        r#"module {a:(1+"x")};"#,
        "module {a:(1 and 2)};",
        "module {a:(1|2)};",
        // Anything that is not a literal.
        r#"module {a:"x\(1)"};"#,
        "module {a:.};",
        "module {a:(1,2)};",
        r#"module {a:("x"|ascii_upcase)};"#,
        "module {a:$ENV};",
        "module {a:[.]};",
        "module {a:{b:.}};",
        "module {a};",
        "module {a:empty};",
    ] {
        let (stdout, stderr, code) = modulemeta(header)?;
        assert_eq!((stdout.as_str(), code), ("null\n", 0), "{header}: {stderr}");
        assert!(
            stderr.starts_with("jq: error: Module metadata must be constant at ")
                && stderr.contains("t.jq, line 1:"),
            "{header}: {stderr}"
        );
    }
    for header in ["module 1;", "module [1];", r#"module "x";"#, "module null;"] {
        let (stdout, stderr, code) = modulemeta(header)?;
        assert_eq!((stdout.as_str(), code), ("null\n", 0), "{header}: {stderr}");
        assert!(
            stderr.starts_with("jq: error: Module metadata must be an object at ")
                && stderr.contains("t.jq, line 1:"),
            "{header}: {stderr}"
        );
    }
    Ok(())
}

#[test]
#[allow(clippy::literal_string_with_formatting_args)] // jq object literals read like format arguments
fn test_directive_metadata_is_a_constant_expression_too_5344() -> Result<()> {
    for (header, want) in [
        (
            r#"import "m" as m {search:(1+2)};"#,
            r#"{"deps":[{"search":3,"as":"m","is_data":false,"relpath":"m"}],"#,
        ),
        (
            r#"import "m" as m ({search:"x"});"#,
            r#"{"deps":[{"search":"x","as":"m","is_data":false,"relpath":"m"}],"#,
        ),
        (
            r#"import "m" as m {search:["a","b"]}+{other:1};"#,
            r#"{"deps":[{"search":["a","b"],"other":1,"as":"m","is_data":false,"relpath":"m"}],"#,
        ),
        (
            r#"include "m" {a:(1+2)};"#,
            r#"{"deps":[{"a":3,"is_data":false,"relpath":"m"}],"#,
        ),
    ] {
        let (stdout, stderr, code) = modulemeta(header)?;
        assert!(
            stdout.starts_with(want) && code == 0,
            "{header}: wanted {want} first, got {stdout:?} ({stderr})"
        );
    }
    for (header, message) in [
        (r#"import "m" as m 1;"#, "must be an object"),
        (r#"include "m" [1];"#, "must be an object"),
        (r#"import "m" as m {a:.};"#, "must be constant"),
    ] {
        let (stdout, stderr, code) = modulemeta(header)?;
        assert_eq!((stdout.as_str(), code), ("null\n", 0), "{header}: {stderr}");
        assert!(
            stderr.starts_with(&format!("jq: error: Module metadata {message} at ")),
            "{header}: {stderr}"
        );
    }
    Ok(())
}

/// The folded metadata is what the loader reads: a `search` computed by the fold finds its
/// directory (#5338), and the program's own directive takes the same expressions.
#[test]
#[allow(clippy::literal_string_with_formatting_args)] // jq object literals read like format arguments
fn test_program_directive_metadata_folds_for_the_loader_5344() -> Result<()> {
    let dir = tempfile::tempdir()?;
    for sub in ["L1", "L2"] {
        std::fs::create_dir(dir.path().join(sub))?;
        std::fs::write(
            dir.path().join(sub).join("m.jq"),
            format!("def a: \"{sub}\";"),
        )?;
    }
    for (filter, want) in [
        (r#"import "m" as m {search:("./L"+"1")}; m::a"#, "\"L1\"\n"),
        (
            r#"import "m" as m ({search:["./L2","./L1"]}); m::a"#,
            "\"L2\"\n",
        ),
    ] {
        let (stdout, stderr, code) = run_jq_in_dir(dir.path(), &["-nc", filter], None)?;
        assert_eq!((stdout.as_str(), code), (want, 0), "{filter}: {stderr}");
    }
    for (filter, message) in [
        (
            r#"import "m" as m [1]; m::a"#,
            "jq: error: Module metadata must be an object at ",
        ),
        (
            r#"import "m" as m {search:-1}; m::a"#,
            "jq: error: Module metadata must be constant at ",
        ),
    ] {
        let (stdout, stderr, code) = run_jq_in_dir(dir.path(), &["-nc", filter], None)?;
        assert_eq!((stdout.as_str(), code), ("", 3), "{filter}: {stderr}");
        assert!(stderr.starts_with(message), "{filter}: {stderr}");
    }
    Ok(())
}
