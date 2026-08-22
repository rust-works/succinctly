//! Structural traversal of a parsed jq filter.
//!
//! One exhaustive definition of "what sub-expressions does this node own",
//! shared by every caller that needs to ask "does this tree mention X
//! anywhere" (#1309).
//!
//! Before this module there were two hand-written answers to that question and
//! they disagreed. `jq_runner`'s `input`/`inputs`/`input_line_number` gate was
//! a substring scan of the *raw filter source*, so a call inside a
//! `-L`/`import`/`include`-loaded module body — which only exists in the tree
//! after `ModuleLoader::process_program` inlines it — was invisible, and the
//! filter silently took a path that never seeds the input queue.
//! `yq_runner::contains_split_doc` did walk the tree, exhaustively over `Expr`
//! but with a `_ => false` wildcard over `Builtin`, so it silently treated
//! twelve sub-expression-carrying builtins as leaves. Both are the failure
//! mode a single definition exists to prevent.
//!
//! The guard against a third divergence is [`builtin_kids`]: it has no
//! wildcard arm, so adding a `Builtin` variant is a compile error here until
//! its sub-expressions are declared, and every caller picks the fix up at once.

use super::{Builtin, Expr, ObjectKey, StringPart};

/// The sub-expressions a [`Builtin`] owns, in source order.
///
/// Four shapes because that is all `Builtin` actually uses: of its 207
/// variants, 125 carry no sub-expression, 60 carry one, 20 carry two and 2
/// carry three.
#[derive(Debug, Clone, Copy)]
pub enum BuiltinKids<'a> {
    /// A builtin with no sub-expression — `length`, `keys`, `inputs`, and the
    /// 122 others, including the two that carry a non-expression payload
    /// (`$ENV`-style `EnvObject(String)` and `StrEnv(String)`).
    None,
    /// One sub-expression — `map(f)`, `select(f)`, `first(f)`, …
    One(&'a Expr),
    /// Two sub-expressions — `limit(n; f)`, `sub(re; s)`, `any(gen; cond)`, …
    Two(&'a Expr, &'a Expr),
    /// Three sub-expressions — `sub(re; s; flags)` and `gsub(re; s; flags)`.
    Three(&'a Expr, &'a Expr, &'a Expr),
}

/// The sub-expressions `builtin` owns.
///
/// **Deliberately has no wildcard arm.** Every one of `Builtin`'s variants is
/// named, so adding a variant fails to compile here until its sub-expressions
/// are declared — which is what keeps [`any_subexpr`] and everything built on
/// it honest as the builtin set grows. `yq_runner::contains_split_doc` ended
/// in `_ => false` and, for its whole life before #1309, silently reported "no
/// `split_doc` in here" for `any(split_doc; .)`, `all(...)`, `IN`/`INDEX`,
/// `fromstream`, `truncate_stream`, `at_offset` and `at_position`.
///
/// Resist the temptation to shorten this with a catch-all: the cost of the
/// long match is paid once, by whoever adds a variant; the cost of a wildcard
/// is paid silently and repeatedly by everyone downstream.
pub fn builtin_kids(builtin: &Builtin) -> BuiltinKids<'_> {
    match builtin {
        // --- No sub-expression (125) ---------------------------------------
        Builtin::Type
        | Builtin::IsNull
        | Builtin::IsBoolean
        | Builtin::IsNumber
        | Builtin::IsString
        | Builtin::IsArray
        | Builtin::IsObject
        | Builtin::Values
        | Builtin::Nulls
        | Builtin::Booleans
        | Builtin::Numbers
        | Builtin::Strings
        | Builtin::Arrays
        | Builtin::Objects
        | Builtin::Iterables
        | Builtin::Scalars
        | Builtin::Length
        | Builtin::Utf8ByteLength
        | Builtin::Keys
        | Builtin::KeysUnsorted
        | Builtin::Empty
        | Builtin::Add
        | Builtin::Any
        | Builtin::All
        | Builtin::Min
        | Builtin::Max
        | Builtin::AsciiDowncase
        | Builtin::AsciiUpcase
        | Builtin::First
        | Builtin::Last
        | Builtin::Reverse
        | Builtin::Flatten
        | Builtin::Unique
        | Builtin::Sort
        | Builtin::ToEntries
        | Builtin::FromEntries
        | Builtin::ToString
        | Builtin::ToNumber
        | Builtin::ToJson
        | Builtin::FromJson
        | Builtin::Explode
        | Builtin::Implode
        | Builtin::ToJsonStream
        | Builtin::FromJsonStream
        | Builtin::ToStream
        | Builtin::Recurse
        | Builtin::PathNoArg
        | Builtin::Parent
        | Builtin::Paths
        | Builtin::LeafPaths
        | Builtin::Floor
        | Builtin::Ceil
        | Builtin::Round
        | Builtin::Sqrt
        | Builtin::Fabs
        | Builtin::Log
        | Builtin::Log10
        | Builtin::Log2
        | Builtin::Exp
        | Builtin::Exp10
        | Builtin::Exp2
        | Builtin::Sin
        | Builtin::Cos
        | Builtin::Tan
        | Builtin::Asin
        | Builtin::Acos
        | Builtin::Atan
        | Builtin::Sinh
        | Builtin::Cosh
        | Builtin::Tanh
        | Builtin::Asinh
        | Builtin::Acosh
        | Builtin::Atanh
        | Builtin::Infinite
        | Builtin::Nan
        | Builtin::IsInfinite
        | Builtin::IsNan
        | Builtin::IsNormal
        | Builtin::IsFinite
        | Builtin::Debug
        | Builtin::Halt
        | Builtin::Stderr
        | Builtin::HaltError
        | Builtin::Env
        | Builtin::EnvObject(_)
        | Builtin::StrEnv(_)
        | Builtin::NullLit
        | Builtin::Trim
        | Builtin::Ltrim
        | Builtin::Rtrim
        | Builtin::Transpose
        | Builtin::Tag
        | Builtin::Anchor
        | Builtin::Style
        | Builtin::Kind
        | Builtin::Key
        | Builtin::Line
        | Builtin::Column
        | Builtin::DocumentIndex
        | Builtin::LineComment
        | Builtin::FileIndex
        | Builtin::Shuffle
        | Builtin::Pivot
        | Builtin::SplitDoc
        | Builtin::Now
        | Builtin::Input
        | Builtin::Inputs
        | Builtin::InputLineNumber
        | Builtin::Abs
        | Builtin::Builtins
        | Builtin::Normals
        | Builtin::Finites
        | Builtin::RecurseDown
        | Builtin::Gmtime
        | Builtin::Localtime
        | Builtin::Mktime
        | Builtin::Todate
        | Builtin::Fromdate
        | Builtin::Todateiso8601
        | Builtin::Fromdateiso8601
        | Builtin::Combinations
        | Builtin::Trunc
        | Builtin::ToBoolean
        | Builtin::FromUnix
        | Builtin::ToUnix => BuiltinKids::None,

        // --- One sub-expression (60) ---------------------------------------
        Builtin::Has(e)
        | Builtin::In(e)
        | Builtin::UpperIn(e)
        | Builtin::Select(e)
        | Builtin::Map(e)
        | Builtin::MapValues(e)
        | Builtin::AnyF(e)
        | Builtin::AllF(e)
        | Builtin::MinBy(e)
        | Builtin::MaxBy(e)
        | Builtin::Ltrimstr(e)
        | Builtin::Rtrimstr(e)
        | Builtin::Startswith(e)
        | Builtin::Endswith(e)
        | Builtin::Split(e)
        | Builtin::Join(e)
        | Builtin::Contains(e)
        | Builtin::Inside(e)
        | Builtin::Nth(e)
        | Builtin::FlattenDepth(e)
        | Builtin::GroupBy(e)
        | Builtin::UniqueBy(e)
        | Builtin::SortBy(e)
        | Builtin::WithEntries(e)
        | Builtin::Test(e)
        | Builtin::Indices(e)
        | Builtin::Index(e)
        | Builtin::Rindex(e)
        | Builtin::UpperIndex(e)
        | Builtin::FromStream(e)
        | Builtin::TruncateStream(e)
        | Builtin::GetPath(e)
        | Builtin::RecurseF(e)
        | Builtin::Walk(e)
        | Builtin::IsValid(e)
        | Builtin::Path(e)
        | Builtin::ParentN(e)
        | Builtin::PathsFilter(e)
        | Builtin::DelPaths(e)
        | Builtin::DebugMsg(e)
        | Builtin::HaltErrorCode(e)
        | Builtin::EnvVar(e)
        | Builtin::BSearch(e)
        | Builtin::ModuleMeta(e)
        | Builtin::Pick(e)
        | Builtin::Omit(e)
        | Builtin::Del(e)
        | Builtin::FirstStream(e)
        | Builtin::LastStream(e)
        | Builtin::IsEmpty(e)
        | Builtin::Strftime(e)
        | Builtin::Strptime(e)
        | Builtin::Match(e)
        | Builtin::Capture(e)
        | Builtin::Scan(e)
        | Builtin::Splits(e)
        | Builtin::CombinationsN(e)
        | Builtin::Tz(e)
        | Builtin::Load(e)
        | Builtin::AtOffset(e) => BuiltinKids::One(e),

        // --- Two sub-expressions (20) --------------------------------------
        Builtin::UpperInSrc(a, b)
        | Builtin::AnyCond(a, b)
        | Builtin::AllCond(a, b)
        | Builtin::UpperIndexStream(a, b)
        | Builtin::RecurseCond(a, b)
        | Builtin::SetPath(a, b)
        | Builtin::Pow(a, b)
        | Builtin::Atan2(a, b)
        | Builtin::Limit(a, b)
        | Builtin::NthStream(a, b)
        | Builtin::TestFlags(a, b)
        | Builtin::MatchFlags(a, b)
        | Builtin::CaptureFlags(a, b)
        | Builtin::Sub(a, b)
        | Builtin::Gsub(a, b)
        | Builtin::ScanFlags(a, b)
        | Builtin::SplitRegex(a, b)
        | Builtin::SplitsFlags(a, b)
        | Builtin::Skip(a, b)
        | Builtin::AtPosition(a, b) => BuiltinKids::Two(a, b),

        // --- Three sub-expressions (2) -------------------------------------
        Builtin::SubFlags(a, b, c) | Builtin::GsubFlags(a, b, c) => BuiltinKids::Three(a, b, c),
    }
}

/// Whether `pred` holds for `expr` itself or for any expression nested
/// anywhere inside it.
///
/// Pre-order and short-circuiting: `pred` sees a node before its children, and
/// the walk stops at the first `true`. `pred` therefore does not have to be
/// pure, but it must not rely on visiting every node.
///
/// Exhaustive at both levels — over `Expr` here and over `Builtin` via
/// [`builtin_kids`] — with no wildcard arm at either, so a new variant of
/// either enum is a compile error rather than a silent "no".
///
/// `&mut dyn FnMut` rather than a generic `F`: this recurses, so a generic
/// parameter would monomorphise the whole traversal per call site for a
/// predicate that is called once per node either way. Same reasoning
/// `eval_each`'s sink already uses.
pub fn any_subexpr(expr: &Expr, pred: &mut dyn FnMut(&Expr) -> bool) -> bool {
    if pred(expr) {
        return true;
    }

    match expr {
        // Leaves: nothing nested to descend into.
        Expr::Identity
        | Expr::Field(_)
        | Expr::Index(_)
        | Expr::IndexNumber { .. }
        | Expr::Slice { .. }
        | Expr::SliceNumber { .. }
        | Expr::Iterate
        | Expr::Literal(_)
        | Expr::RecursiveDescent
        | Expr::Not
        | Expr::Format(_)
        | Expr::Var(_)
        | Expr::TrackedVar(_)
        | Expr::Loc { .. }
        | Expr::Env
        | Expr::Break(_) => false,

        Expr::Optional(inner)
        | Expr::Array(inner)
        | Expr::Paren(inner)
        | Expr::Negate(inner)
        | Expr::FirstExpr(inner)
        | Expr::LastExpr(inner)
        | Expr::Repeat(inner)
        | Expr::Label { body: inner, .. } => any_subexpr(inner, pred),

        Expr::Error(inner) => inner.as_deref().is_some_and(|e| any_subexpr(e, pred)),

        Expr::Arithmetic { left, right, .. }
        | Expr::Compare { left, right, .. }
        | Expr::And(left, right)
        | Expr::Or(left, right)
        | Expr::Alternative(left, right)
        | Expr::IndexExpr {
            target: left,
            key: right,
        }
        | Expr::Limit {
            n: left,
            expr: right,
        }
        | Expr::NthExpr {
            n: left,
            expr: right,
        }
        | Expr::Until {
            cond: left,
            update: right,
        }
        | Expr::While {
            cond: left,
            update: right,
        }
        | Expr::As {
            expr: left,
            body: right,
            ..
        }
        // `patterns` holds only destructuring names, never an `Expr`, so
        // `Reduce`/`Foreach`/`AsPattern` need no descent into them.
        | Expr::AsPattern {
            expr: left,
            body: right,
            ..
        }
        | Expr::FuncDef {
            body: left,
            then: right,
            ..
        }
        | Expr::Assign {
            path: left,
            value: right,
        }
        | Expr::Update {
            path: left,
            filter: right,
        }
        | Expr::CompoundAssign {
            path: left,
            value: right,
            ..
        }
        | Expr::AlternativeAssign {
            path: left,
            value: right,
        } => any_subexpr(left, pred) || any_subexpr(right, pred),

        Expr::Try { expr, catch } => {
            any_subexpr(expr, pred) || catch.as_deref().is_some_and(|c| any_subexpr(c, pred))
        }

        Expr::If {
            cond,
            then_branch,
            else_branch,
        } => {
            any_subexpr(cond, pred)
                || any_subexpr(then_branch, pred)
                || any_subexpr(else_branch, pred)
        }

        Expr::SliceExpr { target, start, end } => {
            any_subexpr(target, pred)
                || start.as_deref().is_some_and(|e| any_subexpr(e, pred))
                || end.as_deref().is_some_and(|e| any_subexpr(e, pred))
        }

        Expr::Range { from, to, step } => {
            any_subexpr(from, pred)
                || to.as_deref().is_some_and(|e| any_subexpr(e, pred))
                || step.as_deref().is_some_and(|e| any_subexpr(e, pred))
        }

        Expr::Reduce {
            input, init, update, ..
        } => any_subexpr(input, pred) || any_subexpr(init, pred) || any_subexpr(update, pred),

        Expr::Foreach {
            input,
            init,
            update,
            extract,
            ..
        } => {
            any_subexpr(input, pred)
                || any_subexpr(init, pred)
                || any_subexpr(update, pred)
                || extract.as_deref().is_some_and(|e| any_subexpr(e, pred))
        }

        Expr::Pipe(exprs) | Expr::Comma(exprs) => exprs.iter().any(|e| any_subexpr(e, pred)),

        Expr::FuncCall { args, .. } | Expr::NamespacedCall { args, .. } => {
            args.iter().any(|e| any_subexpr(e, pred))
        }

        Expr::Object(entries) => entries.iter().any(|entry| {
            matches!(&entry.key, ObjectKey::Expr(k) if any_subexpr(k, pred))
                || any_subexpr(&entry.value, pred)
        }),

        Expr::StringInterpolation(parts) => parts
            .iter()
            .any(|part| matches!(part, StringPart::Expr(e) if any_subexpr(e, pred))),

        Expr::Builtin(builtin) => match builtin_kids(builtin) {
            BuiltinKids::None => false,
            BuiltinKids::One(a) => any_subexpr(a, pred),
            BuiltinKids::Two(a, b) => any_subexpr(a, pred) || any_subexpr(b, pred),
            BuiltinKids::Three(a, b, c) => {
                any_subexpr(a, pred) || any_subexpr(b, pred) || any_subexpr(c, pred)
            }
        },
    }
}

/// Whether `expr` mentions a given builtin anywhere in the tree.
///
/// The common case of [`any_subexpr`]: `contains_builtin(e, |b| matches!(b,
/// Builtin::SplitDoc))`.
pub fn contains_builtin(expr: &Expr, mut want: impl FnMut(&Builtin) -> bool) -> bool {
    any_subexpr(expr, &mut |e| matches!(e, Expr::Builtin(b) if want(b)))
}

/// Whether `expr` references `input`, `inputs` or `input_line_number`
/// anywhere (#1309).
///
/// Must be applied to the **expanded** program — the `Expr` that
/// `ModuleLoader::process_program` returns, with every `-L`/`import`/`include`
/// module body already inlined — not to the filter's source text. A call that
/// exists only inside an imported module's own function never appears in the
/// text the user typed, and the CLI's decision hangs on getting that right:
/// the path chosen when this returns `false` never seeds the input queue, so
/// an undetected call reports spurious exhaustion on every document rather
/// than reading the stream.
pub fn uses_input_builtins(expr: &Expr) -> bool {
    contains_builtin(expr, |b| {
        matches!(
            b,
            Builtin::Input | Builtin::Inputs | Builtin::InputLineNumber
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jq::{parse, parse_program};

    fn uses_inputs(filter: &str) -> bool {
        uses_input_builtins(&parse(filter).expect("filter should parse"))
    }

    fn has_split_doc(filter: &str) -> bool {
        let expr = parse(filter).expect("filter should parse");
        contains_builtin(&expr, |b| matches!(b, Builtin::SplitDoc))
    }

    #[test]
    fn detects_a_bare_input_builtin() {
        assert!(uses_inputs("input"));
        assert!(uses_inputs("inputs"));
        assert!(uses_inputs("input_line_number"));
    }

    #[test]
    fn does_not_fire_on_a_field_or_string_that_merely_spells_input() {
        // The substring heuristic this replaced (#1309) said `true` to every
        // one of these, costing the fast path and forcing a real read
        // under `-n`.
        assert!(!uses_inputs(".input"));
        assert!(!uses_inputs(".inputs"));
        assert!(!uses_inputs(r#""input""#));
        assert!(!uses_inputs(r#".["input_line_number"]"#));
        assert!(!uses_inputs(".a.b.c"));
    }

    #[test]
    fn descends_through_every_builtin_arity_class() {
        assert!(uses_inputs("map(input)")); // One
        assert!(uses_inputs("limit(1; inputs)")); // Two
        assert!(uses_inputs(r#"sub("a"; "b"; input)"#)); // Three
    }

    #[test]
    fn descends_through_the_twelve_builtins_the_old_yq_walk_missed() {
        // `contains_split_doc`'s `_ => false` arm treated all of these as
        // leaves (#1309). Each spelling below constructs one of the twelve.
        for filter in [
            "any(inputs; . > 1)",        // AnyCond
            "all(inputs; . > 1)",        // AllCond
            "any(input)",                // AnyF
            "all(input)",                // AllF
            "IN(inputs)",                // UpperIn
            "IN(inputs; .)",             // UpperInSrc
            "INDEX(input)",              // UpperIndex
            "INDEX(inputs; .)",          // UpperIndexStream
            "fromstream(inputs)",        // FromStream
            "truncate_stream(inputs)",   // TruncateStream
            "at_offset(input)",          // AtOffset
            "at_position(input; input)", // AtPosition
        ] {
            assert!(uses_inputs(filter), "should detect input in {filter}");
        }
    }

    #[test]
    fn descends_through_indirect_expr_carriers() {
        assert!(uses_inputs("{a: input}"));
        assert!(uses_inputs("{(input | tostring): 1}")); // ObjectKey::Expr
        assert!(uses_inputs(r#""x\(input)y""#)); // StringPart::Expr
        assert!(uses_inputs("def f: input; f")); // FuncDef body
        assert!(uses_inputs("def f: 1; input")); // FuncDef then
        assert!(uses_inputs("reduce inputs as $x (0; . + $x)"));
        assert!(uses_inputs("foreach inputs as $x (0; . + $x; .)"));
        assert!(uses_inputs("label $out | input"));
        assert!(uses_inputs("try input catch ."));
        assert!(uses_inputs("try . catch input"));
        assert!(uses_inputs("if . then input else . end"));
        assert!(uses_inputs(".[input:2]")); // SliceExpr bound
        assert!(uses_inputs(".[input]")); // IndexExpr key
        assert!(uses_inputs("range(0; input; 1)"));
        assert!(uses_inputs(".a = input"));
        assert!(uses_inputs(".a |= input"));
        assert!(uses_inputs("input as $x | $x"));
    }

    #[test]
    fn sees_an_input_call_only_a_module_body_spells_out() {
        // The #1309 repro in AST form: `filter_str` itself never contains the
        // substring "input", so the check this replaced returned `false`.
        let program =
            parse_program(r#"import "mylib" as m; ., m::readNext"#).expect("program should parse");
        assert!(!uses_input_builtins(&program.expr));

        // What `ModuleLoader::process_program` builds once `mylib.jq`'s
        // `def readNext: input;` is inlined.
        let expanded = Expr::FuncDef {
            name: "m::readNext".to_string(),
            params: Vec::new(),
            body: Box::new(Expr::Builtin(Builtin::Input)),
            then: Box::new(program.expr),
        };
        assert!(uses_input_builtins(&expanded));
    }

    #[test]
    fn contains_builtin_answers_for_other_builtins_too() {
        assert!(has_split_doc("split_doc"));
        assert!(has_split_doc("any(split_doc; .)"));
        assert!(!has_split_doc("."));
        assert!(!has_split_doc(".split_doc"));
    }

    #[test]
    fn any_subexpr_short_circuits_and_visits_the_root() {
        let expr = parse("1, 2, 3").expect("filter should parse");
        let mut seen = 0_usize;
        assert!(any_subexpr(&expr, &mut |_| {
            seen += 1;
            seen == 2
        }));
        // Root (`Comma`) then its first child, and no further.
        assert_eq!(seen, 2);
    }
}
