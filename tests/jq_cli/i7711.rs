//! #7711: a syntax error found at the end of a program is reported where jq reports it, at the
//! start of its lexer's last match (the trailing whitespace run, or else the last comment or
//! token), so a program file or module ending in a newline names the line of its last token and
//! echoes that line, where it used to name the empty line after it. The message wording stays
//! succinctly's own (`docs/compliance/jq/limitations.md`); the line, the echoed line and its
//! padding are jq's.
//!
//! Every row was captured from jq 1.7.1 (`/usr/bin/jq`, jq-1.7.1-apple); jq 1.8.2 names the same
//! line in each (with its column and caret, not followed here). [`SWEEP`] is a sample of a
//! 3,000-program sweep (programs of upstream's `jq.test` cut short, re-spaced and given trailing
//! blank lines and comments) where the line or the padding differed before the fix.

use crate::run_jq_full;
use anyhow::Result;

/// (program text, jq 1.7.1's line, its echoed line with its padding), each run as `-n PROGRAM`.
const SWEEP: &[(&str, usize, &str)] = &[
    ("try\npick(last)\n\ncatch .  | \n", 4, "catch .  |           "),
    ("[while(.<100;\n\n", 1, "[while(.<100;             "),
    ("-.\n  |\n  tojson ==\t\n# x\n", 4, "# x   "),
    ("(add\t/\n\nlength) as\n$m\n|\n\nmap((. - # c\n$m) as\n$d | # c\n$d *\n\n$d) |\n  add\n\n/\n\nlength\n\n|\n  \n", 19, "| "),
    ("(13911860366432393\t== # c\n13911860366432392) |\t. == # c\nif have_decnum # c\nthen false\nelse\ntrue\nend  +\n\n", 7, "end  +      "),
    ("to_entries\t +\n\n", 1, "to_entries\t +             "),
    ("[(index(\",\",\"|\"), rindex(\",\",\"|\")),\tindices\n\n\n", 1, "[(index(\",\",\"|\"), rindex(\",\",\"|\")),\tindices                                           "),
    ("1\n+\t2\n  *\t2\n\n+\n\n10 /\n\n", 7, "10 /    "),
    ("IN(range(10;20); \n\n", 1, "IN(range(10;20);                 "),
    ("(.[]\n|\n\nsele\t\n ", 4, "sele\t    "),
    ("map(has\n\n\n", 1, "map(has       "),
    ("try\n\n\n", 1, "try   "),
    ("try\n\n((ma\n", 3, "((ma    "),
    ("[.[]|split(\",\n# x\n", 1, "[.[]|split(\",            "),
    ("[1,2,empty,3,empty,4]\t +\n\n", 1, "[1,2,empty,3,empty,4]\t +                        "),
    ("[{\"foo\":42} == {\"foo\":42},{\"foo\":42} !=\n  {\"foo\":42}, # c\n{\"foo\":42}\n!=\t{\"ba\t\n ", 4, "!=\t{\"ba\t     "),
    ("try\n\npath(.a # c\n| map(select(.b\n==\n  0)) | .[]) # c\ncatch\n.  | \n", 8, ".  |     "),
    ("{foo: # c\n{baz:\n12, # c\nblap:\n{bar:\t13}},\n  bar: 14}\n\n|\n  contains({bar: 14,\nfoo: {blap: {bar:\t14\n# x\n", 11, "# x   "),
    ("[ran\t\n ", 1, "[ran\t    "),
    ("[first(range(.)),\nlast(range(.))] # c\n[1,\n 2,\n\n\n", 4, " 2,   "),
    ("del(.), del(empty), del((.foo,.bar,.baz)\n\n|\n  .[2,3,0]),\n  del(.foo[0],\n\n.bar[0], .foo, # c\n.\n# x\n", 9, "# x   "),
    ("map(.[1] as $needle |\n  .[0]\n  | contains($needle))\t | \n", 3, "  | contains($needle))\t |                          "),
    ("JOIN({\"0\":[0,\"abc\"],\"1\n# x", 1, "JOIN({\"0\":[0,\"abc\"],\"1                     "),
    ("{foo: {baz:\n\n12, blap:\n\n{bar: 13}},\n  bar: 14} |\n  contains({bar:\n14,\tfoo:\n ", 8, "14,\tfoo:        "),
    ("[[3],[4],[5],6]\n  | # c\n.[]\n\nas\t{a:$a} # c\n?// {\n# x\n", 7, "# x   "),
    ("map(delpaths(\n\n", 1, "map(delpaths(             "),
    ("[contains(\"b\\u0000c\"),\ncontains(\"b\\u0000cd\"), contains(\"b\n\n\n", 2, "contains(\"b\\u0000cd\"), contains(\"b                                 "),
    ("IN(range(5;20);\n\nrange(10))\t[1,\n 2,\n\n\n", 4, " 2,   "),
    ("-.\n  | tojson ==\n  if\n\nhave_decnum\n\nthen\n\n\"-13911860\n\n\n", 9, "\"-13911860 "),
    ("{\"k\":\n{\t\n ", 2, "{\t "),
    ("[.[]|(.a, .a)?\n\n\n", 1, "[.[]|(.a, .a)?              "),
    ("[16 /\n  4\n  /\n2,\n\n16\t/ 4\n  * 2,\t16 # c\n-\n\n\t\n ", 8, "- "),
    ("try\tpick(last) catch # c\n.\n\n[1,\n 2,\n\n\n", 5, " 2,   "),
    ("[[1,2,3]\n==\t[1,2,3],\t[1,2,3]\t!= # c\n[1,2,3], [1,2,3]\n\n== [4,5,6\n# x\n", 6, "# x   "),
    ("[range(0;-5;-1)\n", 1, "[range(0;-5;-1)               "),
    ("[any,al\n\n\n", 1, "[any,al       "),
    ("def\tx(a;b):\n\na # c\nas\n$a\n\n|\tb\n\nas\n  $b\t|\t$a\n  +\n  $b; # c\ndef y($a;$b): $a\n\n+ $b;\n  def\tcheck(a;b):\n[x(a;b)] # c\n== # c\n[y(a;b)];\n\ncheck(.[];.[]*2)  | \n", 21, "check(.[];.[]*2)  |                    "),
    ("[. * (\n\n\n", 1, "[. * (      "),
    ("[skip(0,2,\n# x\n", 2, "# x   "),
    ("1 as\t$foreach |\t2 as $and | 3 as\n$or\n  | # c\n{ # c\n$foreach,\n\n$and, $or, a\t} [1,\n 2,\n\n\n", 8, " 2,   "),
    ("[range(10)]\n  |\n  .[1.2:3.5]\n\n |", 5, " | "),
    ("[.[]|join(\"a\" # tail", 1, "[.[]|join(\"a\" # tail              "),
    (".[]\t|\n\n[.[0] and # c\n.[1],\t.[0] or .[1]]\n\n,", 6, ","),
    ("implode|explode  |", 1, "implode|explode  |                 "),
    (".\tas\n  [$i]\n  |\n\n.\nas\n  $i\t|\n \n# x", 9, "# x"),
    ("[(i # tail", 1, "[(i # tail    "),
    ("[{\"foo\":[1,2,{\"bar\":18},\"world\"]} ==\t{\"foo\":[1,2,{\"bar\":18},\"world\"]},{\"foo\":[1,2,{\"bar\":18},\"world\"]}\t== {\"foo\":[1,2,\n# x", 2, "# x"),
    ("import\n  \"shadow1\" # c\nas f; import\n  \"shadow2\" as\n  f;\n\nimport\n\"shadow1\"\n  as #\n# x", 10, "# x"),
    ("[.[]|values]\n\n\n,", 4, ","),
    ("indices([1,2", 1, "indices([1,2           "),
    ("setpath([range(10000)\t|\n0]", 2, "0] "),
    ("[.[]|[.,1]|until(.[0]\t<\n1;\n  [.[0] # \n# x", 4, "# x"),
    ("[.[]|startswit", 1, "[.[]|startswit     "),
    ("try\n(0/0) catch", 2, "(0/0) catch      "),
    ("[.[]", 1, "[.[]   "),
    ("[if false # ", 1, "[if false #           "),
    ("[limit(5,7;\n\nrange(9))] \n,", 4, ","),
    ("1\n+ tonumber +\n\n(\"10\" |\ttonumbe", 4, "(\"10\" |\ttonumbe        "),
    ("def # c\nf(a;b;c;d;e;f): # c\n[a+1,b,c,d,e,f];\nf(.[0];.[1];.[0];.[0];.[0];.[0]) \n,", 5, ","),
    (".\n\n|", 3, "|"),
];

/// The located line of the first report on `stderr` and the echoed line after it.
fn first_report(stderr: &str) -> Option<(String, String)> {
    let mut lines = stderr.split('\n');
    while let Some(line) = lines.next() {
        if let Some((_, at)) = line.rsplit_once(" at ") {
            if line.starts_with("jq: error: ") && at.ends_with(':') {
                return Some((at.to_string(), lines.next().unwrap_or("").to_string()));
            }
        }
    }
    None
}

#[test]
fn a_syntax_error_at_the_end_is_on_the_line_of_the_last_token_7711() -> Result<()> {
    for &(program, line, echo) in SWEEP {
        let (_, stderr, code) = run_jq_full(&["-n", program], None)?;
        assert_eq!(
            (first_report(&stderr), code),
            (
                Some((format!("<top-level>, line {line}:"), echo.to_string())),
                3
            ),
            "{program:?}"
        );
    }
    Ok(())
}

/// A program file, a module and `modulemeta` read the same way (the issue's own rows).
#[test]
fn a_program_file_or_module_ending_in_a_newline_7711() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let root = std::fs::canonicalize(dir.path())?;
    std::fs::write(root.join("q.jq"), "[1,\n 2,\n\n\n")?;
    std::fs::write(root.join("eof.jq"), "def f: 1 +\n")?;
    std::fs::write(root.join("str.jq"), "def f: 1;\n\"abc\n\n")?;
    std::fs::write(root.join("cmt.jq"), "def f: 1 + # c\n\n")?;
    let run = |args: &[&str]| -> Result<(String, Option<(String, String)>)> {
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_succinctly"))
            .arg("jq")
            .args(["-L", ".", "-n"])
            .args(args)
            .current_dir(&root)
            .env("HOME", &root)
            .output()?;
        let stderr =
            String::from_utf8_lossy(&output.stderr).replace(&*root.to_string_lossy(), "<R>");
        Ok((
            String::from_utf8_lossy(&output.stdout).into_owned(),
            first_report(&stderr),
        ))
    };
    let at = |place: &str, echo: &str| Some((place.to_string(), echo.to_string()));
    assert_eq!(
        run(&["-f", "q.jq"])?,
        (String::new(), at("<top-level>, line 2:", " 2,   "))
    );
    assert_eq!(
        run(&[r#"import "eof" as e; 1"#])?,
        (
            String::new(),
            at("<R>/eof.jq, line 1:", "def f: 1 +          ")
        )
    );
    assert_eq!(
        run(&[r#"include "str"; 1"#])?,
        (String::new(), at("<R>/str.jq, line 2:", "\"abc "))
    );
    // jq 1.7.1 matches a comment without its newline: the whitespace run after it starts on its
    // line. jq 1.8.2 (Tcl-style multiline comments, jqlang/jq#2989) ends the comment with the
    // newline and names line 2; see limitations.md.
    assert_eq!(
        run(&[r#"include "cmt"; 1"#])?,
        (
            String::new(),
            at("<R>/cmt.jq, line 1:", "def f: 1 + # c              ")
        )
    );
    // `modulemeta` prints its load failure and goes on, as jq does.
    assert_eq!(
        run(&[r#""str" | modulemeta"#])?,
        ("null\n".to_string(), at("<R>/str.jq, line 2:", "\"abc "))
    );
    Ok(())
}
