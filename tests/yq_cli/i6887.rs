//! #6887: a document `.inf`/`.nan` used as a number, and `to_json`/`@json` of a non-finite float,
//! fail as yq does instead of computing or printing `null`.
//!
//! yq reads each operand's text with Go's `strconv.ParseFloat`, which has no `.inf`/`.nan`
//! spelling, so `+ - * / %`, `< <= > >=` and `min`/`max` over two numbers one of which is such a
//! scalar raise `strconv.ParseFloat: parsing ".inf": invalid syntax`, in every output format, naming
//! the left operand when both fail. `to_json`, `to_json(N)`, `tojson` and `@json` raise the JSON
//! encoder's marshal error, wrapped once per node from the number up to the value they were given
//! (#6867 does the same for `-o=json`). `==`, `!=`, `//`, `length`, a string or `null` on the other
//! side and a computed infinity are unchanged.
//!
//! Every row was captured from yq v4.53.3 (`yq -n ...` rows excepted: the expectation is the same
//! text from the document form), as `(filter, input, args, stdout, first stderr line, exit code)`.

use crate::run_yq_stdin_with_stderr;

/// `(filter, input, args, stdout, first stderr line, exit code)`.
type Row = (
    &'static str,
    &'static str,
    &'static [&'static str],
    &'static str,
    &'static str,
    i32,
);

const ROWS: &[Row] = &[
    (".a + 1", "a: .inf\nb: -.inf\nc: .nan\nd: 2\ni: 0x10\ns: x\nf: 1.5\nj: 7\n", &[], "", "Error: strconv.ParseFloat: parsing \".inf\": invalid syntax", 1),
    (".a - 1", "a: .inf\nb: -.inf\nc: .nan\nd: 2\ni: 0x10\ns: x\nf: 1.5\nj: 7\n", &[], "", "Error: strconv.ParseFloat: parsing \".inf\": invalid syntax", 1),
    (".a * 2", "a: .inf\nb: -.inf\nc: .nan\nd: 2\ni: 0x10\ns: x\nf: 1.5\nj: 7\n", &[], "", "Error: strconv.ParseFloat: parsing \".inf\": invalid syntax", 1),
    (".a / 2", "a: .inf\nb: -.inf\nc: .nan\nd: 2\ni: 0x10\ns: x\nf: 1.5\nj: 7\n", &[], "", "Error: strconv.ParseFloat: parsing \".inf\": invalid syntax", 1),
    (".a % 2", "a: .inf\nb: -.inf\nc: .nan\nd: 2\ni: 0x10\ns: x\nf: 1.5\nj: 7\n", &[], "", "Error: strconv.ParseFloat: parsing \".inf\": invalid syntax", 1),
    ("1 + .a", "a: .inf\nb: -.inf\nc: .nan\nd: 2\ni: 0x10\ns: x\nf: 1.5\nj: 7\n", &[], "", "Error: strconv.ParseFloat: parsing \".inf\": invalid syntax", 1),
    ("2 * .a", "a: .inf\nb: -.inf\nc: .nan\nd: 2\ni: 0x10\ns: x\nf: 1.5\nj: 7\n", &[], "", "Error: strconv.ParseFloat: parsing \".inf\": invalid syntax", 1),
    (".d / .a", "a: .inf\nb: -.inf\nc: .nan\nd: 2\ni: 0x10\ns: x\nf: 1.5\nj: 7\n", &[], "", "Error: strconv.ParseFloat: parsing \".inf\": invalid syntax", 1),
    (".d % .a", "a: .inf\nb: -.inf\nc: .nan\nd: 2\ni: 0x10\ns: x\nf: 1.5\nj: 7\n", &[], "", "Error: strconv.ParseFloat: parsing \".inf\": invalid syntax", 1),
    (".b + 1", "a: .inf\nb: -.inf\nc: .nan\nd: 2\ni: 0x10\ns: x\nf: 1.5\nj: 7\n", &[], "", "Error: strconv.ParseFloat: parsing \"-.inf\": invalid syntax", 1),
    (".c * 2", "a: .inf\nb: -.inf\nc: .nan\nd: 2\ni: 0x10\ns: x\nf: 1.5\nj: 7\n", &[], "", "Error: strconv.ParseFloat: parsing \".nan\": invalid syntax", 1),
    (".a + .b", "a: .inf\nb: -.inf\nc: .nan\nd: 2\ni: 0x10\ns: x\nf: 1.5\nj: 7\n", &[], "", "Error: strconv.ParseFloat: parsing \".inf\": invalid syntax", 1),
    (".b + .a", "a: .inf\nb: -.inf\nc: .nan\nd: 2\ni: 0x10\ns: x\nf: 1.5\nj: 7\n", &[], "", "Error: strconv.ParseFloat: parsing \"-.inf\": invalid syntax", 1),
    (".a + .d", "a: .inf\nb: -.inf\nc: .nan\nd: 2\ni: 0x10\ns: x\nf: 1.5\nj: 7\n", &[], "", "Error: strconv.ParseFloat: parsing \".inf\": invalid syntax", 1),
    (".d + .a", "a: .inf\nb: -.inf\nc: .nan\nd: 2\ni: 0x10\ns: x\nf: 1.5\nj: 7\n", &[], "", "Error: strconv.ParseFloat: parsing \".inf\": invalid syntax", 1),
    (".a + .i", "a: .inf\nb: -.inf\nc: .nan\nd: 2\ni: 0x10\ns: x\nf: 1.5\nj: 7\n", &[], "", "Error: strconv.ParseFloat: parsing \".inf\": invalid syntax", 1),
    (".i + .a", "a: .inf\nb: -.inf\nc: .nan\nd: 2\ni: 0x10\ns: x\nf: 1.5\nj: 7\n", &[], "", "Error: strconv.ParseFloat: parsing \"0x10\": invalid syntax", 1),
    (".a * 0", "a: .inf\nb: -.inf\nc: .nan\nd: 2\ni: 0x10\ns: x\nf: 1.5\nj: 7\n", &[], "", "Error: strconv.ParseFloat: parsing \".inf\": invalid syntax", 1),
    (".a + 1.5", "a: .inf\nb: -.inf\nc: .nan\nd: 2\ni: 0x10\ns: x\nf: 1.5\nj: 7\n", &[], "", "Error: strconv.ParseFloat: parsing \".inf\": invalid syntax", 1),
    (".a + 1", "a: .Inf\n", &[], "", "Error: strconv.ParseFloat: parsing \".Inf\": invalid syntax", 1),
    (".a + 1", "a: +.INF\n", &[], "", "Error: strconv.ParseFloat: parsing \"+.INF\": invalid syntax", 1),
    (".a + 1", "a: -.Inf\n", &[], "", "Error: strconv.ParseFloat: parsing \"-.Inf\": invalid syntax", 1),
    (".a + 1", "a: .NaN\n", &[], "", "Error: strconv.ParseFloat: parsing \".NaN\": invalid syntax", 1),
    (".a + 1", "a: .NAN\n", &[], "", "Error: strconv.ParseFloat: parsing \".NAN\": invalid syntax", 1),
    (".a + 1", "a: +.inf\n", &[], "", "Error: strconv.ParseFloat: parsing \"+.inf\": invalid syntax", 1),
    (".a + 1", "a: !!float .inf\n", &[], "", "Error: strconv.ParseFloat: parsing \".inf\": invalid syntax", 1),
    (".a += 1", "a: .inf\nb: -.inf\nc: .nan\nd: 2\ni: 0x10\ns: x\nf: 1.5\nj: 7\n", &[], "", "Error: strconv.ParseFloat: parsing \".inf\": invalid syntax", 1),
    (".d += .a", "a: .inf\nb: -.inf\nc: .nan\nd: 2\ni: 0x10\ns: x\nf: 1.5\nj: 7\n", &[], "", "Error: strconv.ParseFloat: parsing \".inf\": invalid syntax", 1),
    (".a |= . + 1", "a: .inf\nb: -.inf\nc: .nan\nd: 2\ni: 0x10\ns: x\nf: 1.5\nj: 7\n", &[], "", "Error: strconv.ParseFloat: parsing \".inf\": invalid syntax", 1),
    (".d |= . + .a", "a: .inf\nb: -.inf\nc: .nan\nd: 2\ni: 0x10\ns: x\nf: 1.5\nj: 7\n", &[], "a: .inf\nb: -.inf\nc: .nan\nd: 2\ni: 0x10\ns: x\nf: 1.5\nj: 7\n", "", 0),
    (".d *= .a", "a: .inf\nb: -.inf\nc: .nan\nd: 2\ni: 0x10\ns: x\nf: 1.5\nj: 7\n", &[], "", "Error: strconv.ParseFloat: parsing \".inf\": invalid syntax", 1),
    (".a -= .d", "a: .inf\nb: -.inf\nc: .nan\nd: 2\ni: 0x10\ns: x\nf: 1.5\nj: 7\n", &[], "", "Error: strconv.ParseFloat: parsing \".inf\": invalid syntax", 1),
    (".a > 1", "a: .inf\nb: -.inf\nc: .nan\nd: 2\ni: 0x10\ns: x\nf: 1.5\nj: 7\n", &[], "", "Error: strconv.ParseFloat: parsing \".inf\": invalid syntax", 1),
    (".a < .d", "a: .inf\nb: -.inf\nc: .nan\nd: 2\ni: 0x10\ns: x\nf: 1.5\nj: 7\n", &[], "", "Error: strconv.ParseFloat: parsing \".inf\": invalid syntax", 1),
    (".d >= .a", "a: .inf\nb: -.inf\nc: .nan\nd: 2\ni: 0x10\ns: x\nf: 1.5\nj: 7\n", &[], "", "Error: strconv.ParseFloat: parsing \".inf\": invalid syntax", 1),
    (".a <= .a", "a: .inf\nb: -.inf\nc: .nan\nd: 2\ni: 0x10\ns: x\nf: 1.5\nj: 7\n", &[], "", "Error: strconv.ParseFloat: parsing \".inf\": invalid syntax", 1),
    (".a > .i", "a: .inf\nb: -.inf\nc: .nan\nd: 2\ni: 0x10\ns: x\nf: 1.5\nj: 7\n", &[], "", "Error: strconv.ParseFloat: parsing \".inf\": invalid syntax", 1),
    (".i > .a", "a: .inf\nb: -.inf\nc: .nan\nd: 2\ni: 0x10\ns: x\nf: 1.5\nj: 7\n", &[], "", "Error: strconv.ParseFloat: parsing \"0x10\": invalid syntax", 1),
    (".c < 1", "a: .inf\nb: -.inf\nc: .nan\nd: 2\ni: 0x10\ns: x\nf: 1.5\nj: 7\n", &[], "", "Error: strconv.ParseFloat: parsing \".nan\": invalid syntax", 1),
    ("[.a, .d] | max", "a: .inf\nb: -.inf\nc: .nan\nd: 2\ni: 0x10\ns: x\nf: 1.5\nj: 7\n", &[], "", "Error: strconv.ParseFloat: parsing \".inf\": invalid syntax", 1),
    ("[.d, .a] | min", "a: .inf\nb: -.inf\nc: .nan\nd: 2\ni: 0x10\ns: x\nf: 1.5\nj: 7\n", &[], "", "Error: strconv.ParseFloat: parsing \".inf\": invalid syntax", 1),
    ("[.d, .d, .b] | max", "a: .inf\nb: -.inf\nc: .nan\nd: 2\ni: 0x10\ns: x\nf: 1.5\nj: 7\n", &[], "", "Error: strconv.ParseFloat: parsing \"-.inf\": invalid syntax", 1),
    ("[.a, .c] | min", "a: .inf\nb: -.inf\nc: .nan\nd: 2\ni: 0x10\ns: x\nf: 1.5\nj: 7\n", &[], "", "Error: strconv.ParseFloat: parsing \".nan\": invalid syntax", 1),
    ("[.f, .i] | max", "a: .inf\nb: -.inf\nc: .nan\nd: 2\ni: 0x10\ns: x\nf: 1.5\nj: 7\n", &[], "", "Error: strconv.ParseFloat: parsing \"0x10\": invalid syntax", 1),
    ("[.i, .f] | max", "a: .inf\nb: -.inf\nc: .nan\nd: 2\ni: 0x10\ns: x\nf: 1.5\nj: 7\n", &[], "", "Error: strconv.ParseFloat: parsing \"0x10\": invalid syntax", 1),
    ("[.f, .i] | min", "a: .inf\nb: -.inf\nc: .nan\nd: 2\ni: 0x10\ns: x\nf: 1.5\nj: 7\n", &[], "", "Error: strconv.ParseFloat: parsing \"0x10\": invalid syntax", 1),
    ("[.i, .j] | max", "a: .inf\nb: -.inf\nc: .nan\nd: 2\ni: 0x10\ns: x\nf: 1.5\nj: 7\n", &[], "0x10\n", "", 0),
    ("[.j, .i] | min", "a: .inf\nb: -.inf\nc: .nan\nd: 2\ni: 0x10\ns: x\nf: 1.5\nj: 7\n", &[], "7\n", "", 0),
    ("[.f, .j] | max", "a: .inf\nb: -.inf\nc: .nan\nd: 2\ni: 0x10\ns: x\nf: 1.5\nj: 7\n", &[], "7\n", "", 0),
    ("[.a, .i] | max", "a: .inf\nb: -.inf\nc: .nan\nd: 2\ni: 0x10\ns: x\nf: 1.5\nj: 7\n", &[], "", "Error: strconv.ParseFloat: parsing \"0x10\": invalid syntax", 1),
    ("[.a, .i] | min", "a: .inf\nb: -.inf\nc: .nan\nd: 2\ni: 0x10\ns: x\nf: 1.5\nj: 7\n", &[], "", "Error: strconv.ParseFloat: parsing \"0x10\": invalid syntax", 1),
    ("[.i, .a] | max", "a: .inf\nb: -.inf\nc: .nan\nd: 2\ni: 0x10\ns: x\nf: 1.5\nj: 7\n", &[], "", "Error: strconv.ParseFloat: parsing \".inf\": invalid syntax", 1),
    (".a == 1", "a: .inf\nb: -.inf\nc: .nan\nd: 2\ni: 0x10\ns: x\nf: 1.5\nj: 7\n", &[], "false\n", "", 0),
    (".a != 1", "a: .inf\nb: -.inf\nc: .nan\nd: 2\ni: 0x10\ns: x\nf: 1.5\nj: 7\n", &[], "true\n", "", 0),
    (".a // 1", "a: .inf\nb: -.inf\nc: .nan\nd: 2\ni: 0x10\ns: x\nf: 1.5\nj: 7\n", &[], ".inf\n", "", 0),
    (".a | length", "a: .inf\nb: -.inf\nc: .nan\nd: 2\ni: 0x10\ns: x\nf: 1.5\nj: 7\n", &[], "4\n", "", 0),
    (".a + \"x\"", "a: .inf\nb: -.inf\nc: .nan\nd: 2\ni: 0x10\ns: x\nf: 1.5\nj: 7\n", &[], ".infx\n", "", 0),
    ("\"x\" + .a", "a: .inf\nb: -.inf\nc: .nan\nd: 2\ni: 0x10\ns: x\nf: 1.5\nj: 7\n", &[], "x.inf\n", "", 0),
    (".a + \"\"", "a: .inf\nb: -.inf\nc: .nan\nd: 2\ni: 0x10\ns: x\nf: 1.5\nj: 7\n", &[], ".inf\n", "", 0),
    ("[.a] | max", "a: .inf\nb: -.inf\nc: .nan\nd: 2\ni: 0x10\ns: x\nf: 1.5\nj: 7\n", &[], ".inf\n", "", 0),
    ("[.a] | min", "a: .inf\nb: -.inf\nc: .nan\nd: 2\ni: 0x10\ns: x\nf: 1.5\nj: 7\n", &[], ".inf\n", "", 0),
    (".d + 1", "a: .inf\nb: -.inf\nc: .nan\nd: 2\ni: 0x10\ns: x\nf: 1.5\nj: 7\n", &[], "3\n", "", 0),
    (".d > 1", "a: .inf\nb: -.inf\nc: .nan\nd: 2\ni: 0x10\ns: x\nf: 1.5\nj: 7\n", &[], "true\n", "", 0),
    (".a == .a", "a: .inf\nb: -.inf\nc: .nan\nd: 2\ni: 0x10\ns: x\nf: 1.5\nj: 7\n", &[], "true\n", "", 0),
    (".a | tostring", "a: .inf\nb: -.inf\nc: .nan\nd: 2\ni: 0x10\ns: x\nf: 1.5\nj: 7\n", &[], ".inf\n", "", 0),
    (".a | tag", "a: .inf\nb: -.inf\nc: .nan\nd: 2\ni: 0x10\ns: x\nf: 1.5\nj: 7\n", &[], "!!float\n", "", 0),
    ("[.a] | .[0] == .[0]", "a: .inf\nb: -.inf\nc: .nan\nd: 2\ni: 0x10\ns: x\nf: 1.5\nj: 7\n", &[], "true\n", "", 0),
    (".s + .a", "a: .inf\nb: -.inf\nc: .nan\nd: 2\ni: 0x10\ns: x\nf: 1.5\nj: 7\n", &[], "x.inf\n", "", 0),
    (".a | to_json", "a: .inf\nb: -.inf\nc: .nan\nd: 2\ni: 0x10\ns: x\nf: 1.5\nj: 7\n", &[], "", "Error: json: error calling MarshalJSON for type *yqlib.CandidateNode: strconv.ParseFloat: parsing \".inf\": invalid syntax", 1),
    (".c | to_json", "a: .inf\nb: -.inf\nc: .nan\nd: 2\ni: 0x10\ns: x\nf: 1.5\nj: 7\n", &[], "", "Error: json: error calling MarshalJSON for type *yqlib.CandidateNode: strconv.ParseFloat: parsing \".nan\": invalid syntax", 1),
    ("[.a] | to_json", "a: .inf\nb: -.inf\nc: .nan\nd: 2\ni: 0x10\ns: x\nf: 1.5\nj: 7\n", &[], "", "Error: json: error calling MarshalJSON for type *yqlib.CandidateNode: json: error calling MarshalJSON for type *yqlib.CandidateNode: strconv.ParseFloat: parsing \".inf\": invalid syntax", 1),
    ("{\"k\": [.a]} | to_json", "a: .inf\nb: -.inf\nc: .nan\nd: 2\ni: 0x10\ns: x\nf: 1.5\nj: 7\n", &[], "", "Error: json: error calling MarshalJSON for type *yqlib.CandidateNode: json: error calling MarshalJSON for type *yqlib.CandidateNode: json: error calling MarshalJSON for type *yqlib.CandidateNode: strconv.ParseFloat: parsing \".inf\": invalid syntax", 1),
    (".a | to_json(0)", "a: .inf\nb: -.inf\nc: .nan\nd: 2\ni: 0x10\ns: x\nf: 1.5\nj: 7\n", &[], "", "Error: json: error calling MarshalJSON for type *yqlib.CandidateNode: strconv.ParseFloat: parsing \".inf\": invalid syntax", 1),
    (".a | to_json(4)", "a: .inf\nb: -.inf\nc: .nan\nd: 2\ni: 0x10\ns: x\nf: 1.5\nj: 7\n", &[], "", "Error: json: error calling MarshalJSON for type *yqlib.CandidateNode: strconv.ParseFloat: parsing \".inf\": invalid syntax", 1),
    (".a | @json", "a: .inf\nb: -.inf\nc: .nan\nd: 2\ni: 0x10\ns: x\nf: 1.5\nj: 7\n", &[], "", "Error: json: error calling MarshalJSON for type *yqlib.CandidateNode: strconv.ParseFloat: parsing \".inf\": invalid syntax", 1),
    ("[.a] | @json", "a: .inf\nb: -.inf\nc: .nan\nd: 2\ni: 0x10\ns: x\nf: 1.5\nj: 7\n", &[], "", "Error: json: error calling MarshalJSON for type *yqlib.CandidateNode: json: error calling MarshalJSON for type *yqlib.CandidateNode: strconv.ParseFloat: parsing \".inf\": invalid syntax", 1),
    (".a | tojson", "a: .inf\nb: -.inf\nc: .nan\nd: 2\ni: 0x10\ns: x\nf: 1.5\nj: 7\n", &[], "", "Error: json: error calling MarshalJSON for type *yqlib.CandidateNode: strconv.ParseFloat: parsing \".inf\": invalid syntax", 1),
    (".d | to_json", "a: .inf\nb: -.inf\nc: .nan\nd: 2\ni: 0x10\ns: x\nf: 1.5\nj: 7\n", &[], "2\n\n", "", 0),
    ("[.d, .s] | to_json", "a: .inf\nb: -.inf\nc: .nan\nd: 2\ni: 0x10\ns: x\nf: 1.5\nj: 7\n", &[], "[\n  2,\n  \"x\"\n]\n\n", "", 0),
    ("{\"k\": .a} | @json", "a: .inf\nb: -.inf\nc: .nan\nd: 2\ni: 0x10\ns: x\nf: 1.5\nj: 7\n", &[], "", "Error: json: error calling MarshalJSON for type *yqlib.CandidateNode: json: error calling MarshalJSON for type *yqlib.CandidateNode: strconv.ParseFloat: parsing \".inf\": invalid syntax", 1),
    ("[.a / 0] | @json", "a: 1\n", &[], "", "Error: json: error calling MarshalJSON for type *yqlib.CandidateNode: json: error calling MarshalJSON for type *yqlib.CandidateNode: json: unsupported value: +Inf", 1),
    (".a / 0 | to_json", "a: 1\n", &[], "", "Error: json: error calling MarshalJSON for type *yqlib.CandidateNode: json: unsupported value: +Inf", 1),
    ("[.a / 0] | to_json", "a: 1\n", &[], "", "Error: json: error calling MarshalJSON for type *yqlib.CandidateNode: json: error calling MarshalJSON for type *yqlib.CandidateNode: json: unsupported value: +Inf", 1),
    ("{\"k\": [0 / 0]} | to_json", "a: 1\n", &[], "", "Error: json: error calling MarshalJSON for type *yqlib.CandidateNode: json: error calling MarshalJSON for type *yqlib.CandidateNode: json: error calling MarshalJSON for type *yqlib.CandidateNode: json: unsupported value: NaN", 1),
    ("[.a, (.a / 0)] | @json", "a: 1\n", &[], "", "Error: json: error calling MarshalJSON for type *yqlib.CandidateNode: json: error calling MarshalJSON for type *yqlib.CandidateNode: json: unsupported value: +Inf", 1),
    (".a * 2", "a: .inf\nb: -.inf\nc: .nan\nd: 2\ni: 0x10\ns: x\nf: 1.5\nj: 7\n", &["-o=json"], "", "Error: strconv.ParseFloat: parsing \".inf\": invalid syntax", 1),
    (".a + 1", "a: .inf\nb: -.inf\nc: .nan\nd: 2\ni: 0x10\ns: x\nf: 1.5\nj: 7\n", &["-o=json", "-I0"], "", "Error: strconv.ParseFloat: parsing \".inf\": invalid syntax", 1),
    (".a | to_json", "a: .inf\nb: -.inf\nc: .nan\nd: 2\ni: 0x10\ns: x\nf: 1.5\nj: 7\n", &["-o=json"], "", "Error: json: error calling MarshalJSON for type *yqlib.CandidateNode: strconv.ParseFloat: parsing \".inf\": invalid syntax", 1),
    (".a + 1", "a: .inf\nb: -.inf\nc: .nan\nd: 2\ni: 0x10\ns: x\nf: 1.5\nj: 7\n", &["-o=yaml"], "", "Error: strconv.ParseFloat: parsing \".inf\": invalid syntax", 1),
    (".a + 1", "a: .inf\nb: -.inf\nc: .nan\nd: 2\ni: 0x10\ns: x\nf: 1.5\nj: 7\n", &["-r"], "", "Error: strconv.ParseFloat: parsing \".inf\": invalid syntax", 1),
    ("(.a + 1)", "a: .inf\nb: -.inf\nc: .nan\nd: 2\ni: 0x10\ns: x\nf: 1.5\nj: 7\n", &["-P"], "", "Error: strconv.ParseFloat: parsing \".inf\": invalid syntax", 1),
    ("{\"b\": (1 / 0), \"a\": [(0 / 0)]}", "a: .inf\nb: -.inf\nc: .nan\nd: 2\ni: 0x10\ns: x\nf: 1.5\nj: 7\n", &["-o=json"], "", "Error: json: error calling MarshalJSON for type *yqlib.CandidateNode: json: error calling MarshalJSON for type *yqlib.CandidateNode: json: unsupported value: +Inf", 1),
];

#[test]
fn a_document_non_finite_scalar_fails_as_yq_does_where_it_is_read_as_a_number_6887() {
    for (filter, input, args, stdout, stderr, code) in ROWS {
        let (out, err, status) =
            run_yq_stdin_with_stderr(filter, input, args).expect("run succinctly yq");
        assert_eq!(out, *stdout, "{filter} {args:?}: stdout");
        assert_eq!(
            err.lines().next().unwrap_or(""),
            *stderr,
            "{filter} {args:?}: stderr"
        );
        assert_eq!(status, *code, "{filter} {args:?}: exit code");
    }
}

/// jq mode is untouched: `nan` and `infinite` are numbers there, and `to_json`-style text is jq's.
#[test]
fn jq_mode_still_computes_with_a_non_finite_number_6887() {
    let (out, _, code) = crate::run_jq_stdin_with_stderr("[infinite, 1] | max", "null", &[])
        .expect("run succinctly jq");
    assert_eq!(code, 0);
    assert!(out.contains("1.7976931348623157e+308"), "{out}");
    let (out, _, code) = crate::run_jq_stdin_with_stderr("[infinite] | tojson", "null", &[])
        .expect("run succinctly jq");
    assert_eq!(code, 0);
    assert!(out.contains("1.7976931348623157e+308"), "{out}");
}
