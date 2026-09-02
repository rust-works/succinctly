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

use alloc::boxed::Box;

#[cfg(test)]
use super::FuncDefBound;
use super::{Builtin, Expr, ObjectKey, StringPart};

/// The sub-expressions a [`Builtin`] owns, in source order.
///
/// Four shapes because that is all `Builtin` actually uses: of its 207
/// variants, 126 carry no sub-expression, 59 carry one, 20 carry two and 2
/// carry three.
#[derive(Debug, Clone, Copy)]
pub enum BuiltinKids<'a> {
    /// A builtin with no sub-expression — `length`, `keys`, `inputs`, and the
    /// 123 others, including the two that carry a non-expression payload
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
        // --- No sub-expression (126) ---------------------------------------
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
        | Builtin::ModuleMeta
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

        // --- One sub-expression (59) ---------------------------------------
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

/// The mapping twin of [`builtin_kids`]: reconstructs `builtin` with each of
/// its sub-expressions replaced by `f`'s answer for it, in the same source
/// order `builtin_kids` reports them in.
///
/// **Deliberately has no wildcard arm, for the same reason `builtin_kids`
/// doesn't** — a new `Builtin` variant is a compile error here until its
/// sub-expressions (if any) are declared, keeping this function honest
/// alongside `builtin_kids` as the set grows, rather than silently treating
/// a new variant as a leaf. `rewrite_namespaced_calls`
/// (`src/bin/succinctly/jq_runner.rs`) is exactly this shape's motivating
/// caller (#1505): before it existed, that function's own `Builtin`
/// traversal ended in `Expr::Builtin(_) => expr`, so a namespaced call
/// inside any of the 82 sub-expression-carrying builtins (`map`, `select`,
/// `limit`, `sub`, ...) was never rewritten from `Expr::NamespacedCall` to
/// `Expr::FuncCall`, and evaluation failed with "module not loaded".
///
/// `&mut dyn FnMut`, not a generic `F`, matching [`any_subexpr`]'s own
/// reasoning: this recurses, so a generic parameter would monomorphise the
/// whole traversal per call site. Borrowing, not consuming, `builtin` --
/// matching `builtin_kids`'s own convention -- costs one clone per
/// sub-expression at `rewrite_namespaced_calls`'s call site specifically,
/// because that caller's own transform (`Expr -> Expr`, not `&Expr ->
/// Expr`) needs an owned value to hand back into itself (bounded by the
/// parser's own `MAX_EXPR_DEPTH`, since it runs once per parsed program,
/// not per document); a consuming signature would avoid that at the cost of
/// breaking symmetry with `builtin_kids`, considered and not taken (#1526
/// review of a similar tradeoff elsewhere reached the same call for a
/// comparable one-time, depth-bounded traversal). The three `eval.rs`
/// callers added by #1506 (below) pay no such clone: their transforms are
/// already `&Expr -> Expr`, the same shape this function hands them.
///
/// #1506 consolidated this function's three former siblings in `eval.rs`
/// (`substitute_var_in_builtin`, `expand_func_calls_in_builtin`,
/// `substitute_func_param_in_builtin`) onto this one, after confirming each
/// was pure structural recursion -- every arm forwarding its sub-expression
/// unchanged to one fixed transform, with no per-variant special-casing --
/// so no behavior moved in the process, only the traversal itself. Unlike
/// `rewrite_namespaced_calls` (parse-time, once per program),
/// `substitute_var_in_builtin` and `expand_func_calls_in_builtin` sit on the
/// AST-substitution path this codebase uses in place of a runtime variable
/// environment (see #1371), so they can re-run per binding evaluation --
/// potentially many times per document, not just once at parse time.
/// `builtin_kids` remains the sole surviving hand-maintained accessor
/// alongside this one; nothing else duplicates the variant-to-sub-expression
/// mapping now.
///
/// **The dispatch cost this now carries, not just the clone cost above**
/// (#1506 review): every sub-expression visit goes through `f`'s vtable
/// call, and the no-sub-expression bucket's shared `builtin.clone()` arm
/// compiles (confirmed via release-build disassembly, not source-level
/// guessing) to a tail call into the derived `<Builtin as Clone>::clone` --
/// its own ~200-arm switch re-deciding the variant a second time -- rather
/// than the old hand-written code's zero-cost direct materialization
/// (`Builtin::Type => Builtin::Type`) for each of the 126 fieldless
/// variants. Neither cost is new to this function; #1506 is what newly
/// exposes both to genuinely hot per-element callers instead of only
/// `rewrite_namespaced_calls`'s one-shot parse-time one. Measured
/// end-to-end rather than assumed either way, per this file's own
/// benchmarking-discipline precedent: an interleaved A/B (pre-#1506 binary
/// vs. post) over `.[] as $x | $x | length | ... | length` chains of 20/50/
/// 100 fieldless-builtin substitutions across 150k-500k elements -- built to
/// maximize exposure to exactly this path -- showed no consistent,
/// reproducible delta (each configuration's sign flipped between runs,
/// magnitude ~1-2%, on a heavily loaded shared machine rather than one of
/// this project's dedicated benchmark nodes). If a real regression on this
/// path ever does surface under properly isolated measurement, the fix is
/// narrow: give the no-sub-expression bucket its own per-variant arms
/// (`Builtin::Type => Builtin::Type`, ...) instead of the shared
/// `builtin.clone()`, restoring the old zero-cost path without touching the
/// sub-expression-carrying arms at all.
pub fn map_builtin_subexprs(builtin: &Builtin, f: &mut dyn FnMut(&Expr) -> Expr) -> Builtin {
    match builtin {
        // --- No sub-expression (126) ---------------------------------------
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
        | Builtin::ModuleMeta
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
        | Builtin::ToUnix => builtin.clone(),

        // --- One sub-expression (59) ---------------------------------------
        Builtin::Has(e) => Builtin::Has(Box::new(f(e))),
        Builtin::In(e) => Builtin::In(Box::new(f(e))),
        Builtin::UpperIn(e) => Builtin::UpperIn(Box::new(f(e))),
        Builtin::Select(e) => Builtin::Select(Box::new(f(e))),
        Builtin::Map(e) => Builtin::Map(Box::new(f(e))),
        Builtin::MapValues(e) => Builtin::MapValues(Box::new(f(e))),
        Builtin::AnyF(e) => Builtin::AnyF(Box::new(f(e))),
        Builtin::AllF(e) => Builtin::AllF(Box::new(f(e))),
        Builtin::MinBy(e) => Builtin::MinBy(Box::new(f(e))),
        Builtin::MaxBy(e) => Builtin::MaxBy(Box::new(f(e))),
        Builtin::Ltrimstr(e) => Builtin::Ltrimstr(Box::new(f(e))),
        Builtin::Rtrimstr(e) => Builtin::Rtrimstr(Box::new(f(e))),
        Builtin::Startswith(e) => Builtin::Startswith(Box::new(f(e))),
        Builtin::Endswith(e) => Builtin::Endswith(Box::new(f(e))),
        Builtin::Split(e) => Builtin::Split(Box::new(f(e))),
        Builtin::Join(e) => Builtin::Join(Box::new(f(e))),
        Builtin::Contains(e) => Builtin::Contains(Box::new(f(e))),
        Builtin::Inside(e) => Builtin::Inside(Box::new(f(e))),
        Builtin::Nth(e) => Builtin::Nth(Box::new(f(e))),
        Builtin::FlattenDepth(e) => Builtin::FlattenDepth(Box::new(f(e))),
        Builtin::GroupBy(e) => Builtin::GroupBy(Box::new(f(e))),
        Builtin::UniqueBy(e) => Builtin::UniqueBy(Box::new(f(e))),
        Builtin::SortBy(e) => Builtin::SortBy(Box::new(f(e))),
        Builtin::WithEntries(e) => Builtin::WithEntries(Box::new(f(e))),
        Builtin::Test(e) => Builtin::Test(Box::new(f(e))),
        Builtin::Indices(e) => Builtin::Indices(Box::new(f(e))),
        Builtin::Index(e) => Builtin::Index(Box::new(f(e))),
        Builtin::Rindex(e) => Builtin::Rindex(Box::new(f(e))),
        Builtin::UpperIndex(e) => Builtin::UpperIndex(Box::new(f(e))),
        Builtin::FromStream(e) => Builtin::FromStream(Box::new(f(e))),
        Builtin::TruncateStream(e) => Builtin::TruncateStream(Box::new(f(e))),
        Builtin::GetPath(e) => Builtin::GetPath(Box::new(f(e))),
        Builtin::RecurseF(e) => Builtin::RecurseF(Box::new(f(e))),
        Builtin::Walk(e) => Builtin::Walk(Box::new(f(e))),
        Builtin::IsValid(e) => Builtin::IsValid(Box::new(f(e))),
        Builtin::Path(e) => Builtin::Path(Box::new(f(e))),
        Builtin::ParentN(e) => Builtin::ParentN(Box::new(f(e))),
        Builtin::PathsFilter(e) => Builtin::PathsFilter(Box::new(f(e))),
        Builtin::DelPaths(e) => Builtin::DelPaths(Box::new(f(e))),
        Builtin::DebugMsg(e) => Builtin::DebugMsg(Box::new(f(e))),
        Builtin::HaltErrorCode(e) => Builtin::HaltErrorCode(Box::new(f(e))),
        Builtin::EnvVar(e) => Builtin::EnvVar(Box::new(f(e))),
        Builtin::BSearch(e) => Builtin::BSearch(Box::new(f(e))),
        Builtin::Pick(e) => Builtin::Pick(Box::new(f(e))),
        Builtin::Omit(e) => Builtin::Omit(Box::new(f(e))),
        Builtin::Del(e) => Builtin::Del(Box::new(f(e))),
        Builtin::FirstStream(e) => Builtin::FirstStream(Box::new(f(e))),
        Builtin::LastStream(e) => Builtin::LastStream(Box::new(f(e))),
        Builtin::IsEmpty(e) => Builtin::IsEmpty(Box::new(f(e))),
        Builtin::Strftime(e) => Builtin::Strftime(Box::new(f(e))),
        Builtin::Strptime(e) => Builtin::Strptime(Box::new(f(e))),
        Builtin::Match(e) => Builtin::Match(Box::new(f(e))),
        Builtin::Capture(e) => Builtin::Capture(Box::new(f(e))),
        Builtin::Scan(e) => Builtin::Scan(Box::new(f(e))),
        Builtin::Splits(e) => Builtin::Splits(Box::new(f(e))),
        Builtin::CombinationsN(e) => Builtin::CombinationsN(Box::new(f(e))),
        Builtin::Tz(e) => Builtin::Tz(Box::new(f(e))),
        Builtin::Load(e) => Builtin::Load(Box::new(f(e))),
        Builtin::AtOffset(e) => Builtin::AtOffset(Box::new(f(e))),

        // --- Two sub-expressions (20) --------------------------------------
        Builtin::UpperInSrc(a, b) => Builtin::UpperInSrc(Box::new(f(a)), Box::new(f(b))),
        Builtin::AnyCond(a, b) => Builtin::AnyCond(Box::new(f(a)), Box::new(f(b))),
        Builtin::AllCond(a, b) => Builtin::AllCond(Box::new(f(a)), Box::new(f(b))),
        Builtin::UpperIndexStream(a, b) => {
            Builtin::UpperIndexStream(Box::new(f(a)), Box::new(f(b)))
        }
        Builtin::RecurseCond(a, b) => Builtin::RecurseCond(Box::new(f(a)), Box::new(f(b))),
        Builtin::SetPath(a, b) => Builtin::SetPath(Box::new(f(a)), Box::new(f(b))),
        Builtin::Pow(a, b) => Builtin::Pow(Box::new(f(a)), Box::new(f(b))),
        Builtin::Atan2(a, b) => Builtin::Atan2(Box::new(f(a)), Box::new(f(b))),
        Builtin::Limit(a, b) => Builtin::Limit(Box::new(f(a)), Box::new(f(b))),
        Builtin::NthStream(a, b) => Builtin::NthStream(Box::new(f(a)), Box::new(f(b))),
        Builtin::TestFlags(a, b) => Builtin::TestFlags(Box::new(f(a)), Box::new(f(b))),
        Builtin::MatchFlags(a, b) => Builtin::MatchFlags(Box::new(f(a)), Box::new(f(b))),
        Builtin::CaptureFlags(a, b) => Builtin::CaptureFlags(Box::new(f(a)), Box::new(f(b))),
        Builtin::Sub(a, b) => Builtin::Sub(Box::new(f(a)), Box::new(f(b))),
        Builtin::Gsub(a, b) => Builtin::Gsub(Box::new(f(a)), Box::new(f(b))),
        Builtin::ScanFlags(a, b) => Builtin::ScanFlags(Box::new(f(a)), Box::new(f(b))),
        Builtin::SplitRegex(a, b) => Builtin::SplitRegex(Box::new(f(a)), Box::new(f(b))),
        Builtin::SplitsFlags(a, b) => Builtin::SplitsFlags(Box::new(f(a)), Box::new(f(b))),
        Builtin::Skip(a, b) => Builtin::Skip(Box::new(f(a)), Box::new(f(b))),
        Builtin::AtPosition(a, b) => Builtin::AtPosition(Box::new(f(a)), Box::new(f(b))),

        // --- Three sub-expressions (2) -------------------------------------
        Builtin::SubFlags(a, b, c) => {
            Builtin::SubFlags(Box::new(f(a)), Box::new(f(b)), Box::new(f(c)))
        }
        Builtin::GsubFlags(a, b, c) => {
            Builtin::GsubFlags(Box::new(f(a)), Box::new(f(b)), Box::new(f(c)))
        }
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
        // #1371: both variants only ever exist mid-evaluation, never in a
        // freshly parsed program -- but this function answers "does this tree
        // mention X anywhere" for callers that do run against evaluation-time
        // trees, so both descend. A `Shared` holds a real argument
        // expression, and a `DefCall` holds both the definition it resolved
        // to and that call's arguments; treating either as a leaf would
        // reintroduce exactly the "silently reported no match" bug this
        // module exists to prevent.
        Expr::Shared(inner) => any_subexpr(inner, pred),
        Expr::DefCall { def, args, .. } => {
            any_subexpr(&def.body, pred) || args.iter().any(|a| any_subexpr(a, pred))
        }
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

/// Whether `expr` references a builtin whose answer comes from the *cursor*
/// rather than from the value, anywhere (#1504).
///
/// These eight are the ones `eval_generic.rs` answers from `Option<V::Cursor>`
/// and `eval.rs` cannot answer at all: `line`/`column`/`document_index`/
/// `anchor`/`style`/`line_comment` are fixed-default stubs there, and
/// `at_offset`/`at_position` are unconditional "requires document cursor
/// context" errors. Any bridge from `eval_generic.rs` into `eval.rs` therefore
/// silently downgrades or breaks them, which is what this predicate exists to
/// let a caller avoid.
///
/// Re-indexing cannot rescue them: `eval_each_owned` rebuilds its index from
/// `to_json_for_reindex`'s *re-serialized* text, so byte offsets and
/// line/column on that document describe the re-serialization, not the file
/// the user passed. A cursor-aware `eval.rs` would answer confidently and
/// wrongly; declining the bridge is the only answer that stays true.
///
/// Same expanded-program requirement as [`uses_input_builtins`] — a call
/// reachable only through an imported module body still counts.
pub fn uses_cursor_metadata_builtins(expr: &Expr) -> bool {
    contains_builtin(expr, |b| {
        matches!(
            b,
            Builtin::Line
                | Builtin::Column
                | Builtin::DocumentIndex
                | Builtin::Anchor
                | Builtin::Style
                | Builtin::LineComment
                | Builtin::AtOffset(_)
                | Builtin::AtPosition(_, _)
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

    fn uses_cursor_meta(filter: &str) -> bool {
        uses_cursor_metadata_builtins(&parse(filter).expect("filter should parse"))
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
            bound: FuncDefBound::default(),
        };
        assert!(uses_input_builtins(&expanded));
    }

    #[test]
    fn detects_every_cursor_metadata_builtin() {
        // The six `eval.rs` answers from a fixed default...
        assert!(uses_cursor_meta("line"));
        assert!(uses_cursor_meta("column"));
        assert!(uses_cursor_meta("document_index"));
        assert!(uses_cursor_meta("anchor"));
        assert!(uses_cursor_meta("style"));
        assert!(uses_cursor_meta("line_comment"));
        // ...and the two it rejects outright.
        assert!(uses_cursor_meta("at_offset(0)"));
        assert!(uses_cursor_meta("at_position(1; 1)"));
    }

    #[test]
    fn cursor_metadata_check_reaches_nested_and_argument_positions() {
        assert!(uses_cursor_meta("inputs | line"));
        assert!(uses_cursor_meta("(., at_offset(0))"));
        assert!(uses_cursor_meta("first(line)"));
        assert!(uses_cursor_meta("[at_position(1; 1)]"));
        assert!(uses_cursor_meta("def f: line; f"));
        assert!(uses_cursor_meta("at_offset(line)")); // nested in an argument
    }

    #[test]
    fn cursor_metadata_check_does_not_fire_on_lookalikes() {
        assert!(!uses_cursor_meta("."));
        assert!(!uses_cursor_meta(".line"));
        assert!(!uses_cursor_meta(".column"));
        assert!(!uses_cursor_meta("\"line\""));
        assert!(!uses_cursor_meta("inputs | input_line_number"));
        assert!(!uses_cursor_meta("{line: 1}"));
    }

    /// The two predicates are independent: #1504's carve-out is the
    /// *conjunction* of them, so neither may imply the other.
    #[test]
    fn input_and_cursor_metadata_checks_are_independent() {
        assert!(uses_inputs("inputs") && !uses_cursor_meta("inputs"));
        assert!(uses_cursor_meta("line") && !uses_inputs("line"));
        assert!(uses_inputs("inputs, line") && uses_cursor_meta("inputs, line"));
        assert!(!uses_inputs(".a") && !uses_cursor_meta(".a"));
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

    /// #1371: unlike `resolve::check` (which runs once on the freshly parsed
    /// program and so can never actually see an `Expr::DefCall`),
    /// `any_subexpr` answers callers -- `streams_unbounded`, `has_navigation`
    /// and others -- that run *during* evaluation, after `def` calls have
    /// been installed. Both halves of the arm's `||` need their own case: a
    /// predicate matching only inside the resolved definition's body, and one
    /// matching only inside the call's own (unsubstituted) arguments, so a
    /// regression that dropped either operand would still fail here even
    /// though the other made the whole expression true.
    #[test]
    fn any_subexpr_descends_into_defcall_body_and_args() {
        use alloc::rc::Rc;

        let marker = || Expr::Var("marker".into());
        let mut is_marker = |e: &Expr| matches!(e, Expr::Var(name) if name == "marker");

        let in_body = Expr::DefCall {
            def: Rc::new(crate::jq::FuncDefData {
                name: "f".into(),
                params: Vec::new(),
                body: marker(),
            }),
            args: vec![Expr::Identity],
            frames: 0,
            bound: crate::jq::BoundBody::default(),
        };
        assert!(any_subexpr(&in_body, &mut is_marker));

        let in_args = Expr::DefCall {
            def: Rc::new(crate::jq::FuncDefData {
                name: "f".into(),
                params: Vec::new(),
                body: Expr::Identity,
            }),
            args: vec![marker()],
            frames: 0,
            bound: crate::jq::BoundBody::default(),
        };
        assert!(any_subexpr(&in_args, &mut is_marker));

        let neither = Expr::DefCall {
            def: Rc::new(crate::jq::FuncDefData {
                name: "f".into(),
                params: Vec::new(),
                body: Expr::Identity,
            }),
            args: vec![Expr::Identity],
            frames: 0,
            bound: crate::jq::BoundBody::default(),
        };
        assert!(!any_subexpr(&neither, &mut is_marker));
    }

    fn find_builtin(filter: &str) -> Builtin {
        let expr = parse(filter).expect("filter should parse");
        let mut found = None;
        any_subexpr(&expr, &mut |e| {
            if let Expr::Builtin(b) = e {
                found = Some(b.clone());
                true
            } else {
                false
            }
        });
        found.unwrap_or_else(|| panic!("no Builtin found in {filter:?}"))
    }

    /// #1506 review: no test exercised `map_builtin_subexprs` directly, or
    /// cross-checked it for field *order* rather than just field *count* --
    /// a future two/three-field variant with its fields transcribed in the
    /// wrong order here would still compile (exhaustiveness only checks
    /// variant coverage, not per-arm argument order) and would misbehave
    /// identically across every one of `map_builtin_subexprs`'s three
    /// `eval.rs` callers at once, instead of being caught by one of three
    /// previously-independent hand-written matches disagreeing.
    ///
    /// Every case below is real filter syntax whose fields already hold
    /// distinct values (`pow(2; 3)`, not `pow(2; 2)`) -- so round-tripping
    /// through `map_builtin_subexprs` with an identity `f` (`|e| e.clone()`)
    /// and asserting full equality against the original genuinely catches a
    /// swap: a buggy `Pow(f(b), f(a))` on `pow(2; 3)` would reconstruct
    /// `Pow(3, 2)`, which fails `assert_eq!` against the original `Pow(2,
    /// 3)`. A marker that only encodes *call order* (tried first, reverted)
    /// does not: placement tracks call order too, so `f(b)` called first
    /// still lands wherever `f`'s first result is written, silently passing
    /// even when the fields themselves are transposed.
    #[test]
    fn map_builtin_subexprs_preserves_field_order() {
        let two_field_cases = [
            r#"sub("a"; "b")"#,   // Sub(re, repl)
            "pow(2; 3)",          // Pow(a, b)
            "atan2(2; 3)",        // Atan2(a, b)
            "at_position(1; 2)",  // AtPosition(line, col)
            r#"test("a"; "g")"#,  // TestFlags(re, flags)
            "any(inputs; . > 1)", // AnyCond(source, cond)
            "INDEX(inputs; .)",   // UpperIndexStream(source, key)
            "IN(inputs; .)",      // UpperInSrc(source, target)
            "setpath([0]; 1)",    // SetPath(path, value)
        ];
        for filter in two_field_cases {
            let builtin = find_builtin(filter);
            let mut calls = 0usize;
            let result = map_builtin_subexprs(&builtin, &mut |e| {
                calls += 1;
                e.clone()
            });
            assert_eq!(calls, 2, "expected exactly 2 sub-expressions in {filter:?}");
            assert_eq!(
                result, builtin,
                "field order/identity not preserved for {filter:?}"
            );
        }

        let three_field_cases = [
            r#"sub("a"; "b"; "g")"#,  // SubFlags(re, repl, flags)
            r#"gsub("a"; "b"; "g")"#, // GsubFlags(re, repl, flags)
        ];
        for filter in three_field_cases {
            let builtin = find_builtin(filter);
            let mut calls = 0usize;
            let result = map_builtin_subexprs(&builtin, &mut |e| {
                calls += 1;
                e.clone()
            });
            assert_eq!(calls, 3, "expected exactly 3 sub-expressions in {filter:?}");
            assert_eq!(
                result, builtin,
                "field order/identity not preserved for {filter:?}"
            );
        }
    }
}
