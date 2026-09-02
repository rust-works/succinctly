//! Expression AST for jq-like queries.

#[cfg(not(test))]
use alloc::boxed::Box;
#[cfg(not(test))]
use alloc::collections::BTreeMap;
#[cfg(not(test))]
use alloc::rc::Rc;
#[cfg(not(test))]
use alloc::string::String;
#[cfg(not(test))]
use alloc::vec::Vec;
#[cfg(test)]
use std::collections::BTreeMap;
#[cfg(test)]
use std::rc::Rc;

use super::value::{NumberRepr, OwnedValue};

/// A jq expression representing a query path.
#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    /// Identity: `.`
    Identity,

    /// Field access: `.foo`
    Field(String),

    /// Array index access: `.[0]` or `.[-1]`
    Index(i64),

    /// Array index access whose *original number* has to survive into
    /// `path()` output: `.[2.0]`, `.[1.7]`, `.[1e10]`, `.[2.00]`.
    ///
    /// jq appends the resolved key **verbatim** as the path component, so a
    /// number that still carries its own spelling keeps it —
    /// `path(.[2.0])` is `[2.0]` and `path(.[1e10])` is `[1E+10]`, not
    /// `[2]`/`[10000000000]` (#1088). [`Expr::Index`]'s bare `i64` has
    /// nowhere to put that, so a float-spelled key folds to this instead.
    ///
    /// `idx` is exactly the `i64` [`Expr::Index`] would have carried (the
    /// same truncation-toward-zero `numeric_key_to_index` applies), and
    /// every step that *navigates* — reading, `setpath`, `del` — uses it
    /// and behaves identically. Only the rendering of the component
    /// differs, which is why nearly every match site pairs the two arms:
    /// `Expr::Index(idx) | Expr::IndexNumber { idx, .. }`.
    ///
    /// An integer-*spelled* key never reaches here: `2` renders the same
    /// whether it goes out as `OwnedValue::Int` or as its own literal, so
    /// it keeps folding to [`Expr::Index`] and the hot `.foo.bar[0]` path
    /// is untouched. Nor does a negated one — see [`NumberKey`].
    IndexNumber {
        /// The index every navigation step actually uses.
        idx: i64,
        /// The number the component is reported as.
        key: NumberKey,
    },

    /// Array slice: `.[2:5]` or `.[2:]` or `.[:5]`
    Slice {
        start: Option<i64>,
        end: Option<i64>,
    },

    /// Array slice whose *original number* has to survive into `path()`
    /// output for at least one bound: `.[1.5:3.5]`, `.[1.0:3]`, `.[:3.00]`.
    ///
    /// The slice-bound sibling of [`Expr::IndexNumber`] (#1326, following
    /// on from #1088): jq appends each resolved bound **verbatim** as the
    /// path component, so a bound that still carries its own spelling keeps
    /// it -- `path(.[1.5:3.5])` is `[{"start":1.5,"end":3.5}]`, not
    /// `[{"start":1,"end":4}]`. [`Expr::Slice`]'s bare `Option<i64>` pair has
    /// nowhere to put that, so a slice with at least one float-spelled bound
    /// folds to this instead.
    ///
    /// `start`/`end` are exactly the `Option<i64>` pair [`Expr::Slice`]
    /// would have carried (the same bound-folding `fold_slice_bound`
    /// applies to each), and every step that *navigates* — reading,
    /// `setpath`, `del` — uses them and behaves identically. Only the
    /// rendering of the path component differs, which is why nearly every
    /// match site pairs the two arms:
    /// `Expr::Slice { start, end } | Expr::SliceNumber { start, end, .. }`.
    ///
    /// A bound is only ever carried here when *its own* `NumberKey` is
    /// `Some` — a slice with one float-spelled bound and one absent/plain
    /// bound (`.[1.5:]`, `.[1.5:3]`) still reaches here with the other
    /// key field `None`, not a synthesized one. A slice whose bounds are
    /// *both* integer-spelled or absent never reaches here at all: it keeps
    /// folding to plain [`Expr::Slice`], and the hot `.foo.bar[1:3]` path is
    /// untouched. Nor does a negated bound (`.[-1.5:]`) — see [`NumberKey`].
    SliceNumber {
        /// The start bound every navigation step actually uses.
        start: Option<i64>,
        /// The end bound every navigation step actually uses.
        end: Option<i64>,
        /// The number the start bound is reported as, if it kept its own
        /// spelling.
        start_key: Option<NumberKey>,
        /// The number the end bound is reported as, if it kept its own
        /// spelling.
        end_key: Option<NumberKey>,
    },

    /// Iterate all elements: `.[]`
    Iterate,

    /// Index by a computed key: `.[$k]`, `.[.k]`, `.[1,2]`, `E[K]`.
    ///
    /// jq compiles `E[K]` as `K as $k | E | .[$k]`, so `K` is evaluated against
    /// the input to the *whole* postfix chain, not against the value at `E`. On
    /// `{"k":"x","a":{"k":"y","x":"HIT-x","y":"HIT-y"}}`, `.a[.k]` is `"HIT-x"`
    /// (the root's `.k`) while `.a | .[.k]` is `"HIT-y"`. [`Expr::Pipe`] cannot
    /// express that difference — both lower to the same flat chain — so this
    /// variant carries its own `target` and nests instead of flattening.
    ///
    /// Constant keys never reach here: the parser folds `.["a"]` to
    /// [`Expr::Field`] and `.[0]` to [`Expr::Index`], so the hot `.foo.bar[0]`
    /// path and every existing `Field`/`Index` match site are untouched.
    ///
    /// Two deliberate divergences from jq. An array-valued key errors with
    /// `Cannot index array with array` rather than performing jq's
    /// indices-of-subarray search (`[10,20,30] | .[[20]]` → `[1]`). And a
    /// NaN key, which reads as null in both, has no path component here — jq's
    /// `path(.[nan])` is `[null]`, a path its own `setpath` then rejects, so
    /// this errors at the source instead.
    ///
    /// A float key is *not* on that list any more: it keeps its own
    /// spelling in `path()` output, via [`Expr::IndexNumber`] (#1088).
    IndexExpr {
        /// The value being indexed — the postfix chain so far.
        target: Box<Self>,
        /// The key expression, evaluated against this node's own input.
        key: Box<Self>,
    },

    /// Slice by a computed bound: `.[$a:$b]`, `.[.k:2]`, `E[S:T]` where at
    /// least one bound isn't an integer literal.
    ///
    /// jq compiles `E[S:T]` as `S as $s | T as $t | E | .[$s:$t]`, the same
    /// desugaring [`Expr::IndexExpr`] documents for `E[K]` — `S`/`T` are
    /// evaluated against *this node's* input, not `E`'s output, which is why
    /// this variant carries its own `target` instead of flattening into
    /// [`Expr::Pipe`]. `S` is evaluated outer, `T` middle, `E` inner.
    ///
    /// A bound that fully folds to a constant never reaches here: the parser
    /// keeps producing a static [`Expr::Slice`]/[`Expr::SliceNumber`]
    /// whenever *both* present bounds are constant (#1326), so the existing
    /// fast paths and every match site pairing those two are untouched.
    SliceExpr {
        /// The value being sliced — the postfix chain so far.
        target: Box<Self>,
        /// The start bound, evaluated against this node's own input.
        start: Option<Box<Self>>,
        /// The end bound, evaluated against this node's own input.
        end: Option<Box<Self>>,
    },

    /// Optional access: `.foo?` - returns null instead of error if missing
    Optional(Box<Self>),

    /// Chained expressions: `.foo.bar[0]`
    /// Each element is applied in sequence to the result of the previous.
    Pipe(Vec<Self>),

    /// Comma operator: `.foo, .bar` - outputs from both expressions
    Comma(Vec<Self>),

    /// Array construction: `[.foo, .bar]` or `[.items[]]`
    /// Collects all outputs from the inner expression into an array.
    Array(Box<Self>),

    /// Object construction: `{foo: .bar, baz: .qux}`
    /// Each entry is (key_expr, value_expr). Key can be literal or dynamic.
    Object(Vec<ObjectEntry>),

    /// Literal value (for object keys, constructed values, etc.)
    Literal(Literal),

    /// Recursive descent: `..`
    /// Recursively descends into all values.
    RecursiveDescent,

    /// Parenthesized expression (for grouping)
    /// This is mostly handled by the parser, but we keep it for clarity.
    Paren(Box<Self>),

    /// Arithmetic operation: `.a + .b`, `.a - .b`, `.a * .b`, `.a / .b`, `.a % .b`
    Arithmetic {
        op: ArithOp,
        left: Box<Self>,
        right: Box<Self>,
    },

    /// Unary minus: `-expr` (#1100). A dedicated single-child variant,
    /// matching this AST's own convention for unary operations (`Paren`,
    /// `Optional`, etc. above) -- unlike the `ArithOp::Negate`
    /// approach it replaces, which reused `Expr::Arithmetic`'s binary shape
    /// with an always-unused dummy `left` operand purely to borrow its
    /// existing fan-out (cartesian-product) machinery. That reuse worked
    /// correctly but re-evaluated the dummy operand once per output the
    /// real operand produced (`map(-.)` over N elements: N wasted dummy
    /// evaluations); this variant evaluates its one real operand directly,
    /// mapping `arith_negate` over each output instead of routing through
    /// the binary fan-out path at all.
    Negate(Box<Self>),

    /// Comparison operation: `.a == .b`, `.a != .b`, `.a < .b`, etc.
    Compare {
        op: CompareOp,
        left: Box<Self>,
        right: Box<Self>,
    },

    /// Boolean AND: `.a and .b`
    And(Box<Self>, Box<Self>),

    /// Boolean OR: `.a or .b`
    Or(Box<Self>, Box<Self>),

    /// Boolean NOT: `not` (unary, applied via pipe)
    Not,

    /// Alternative operator: `.foo // "default"`
    /// Returns left if truthy, otherwise right.
    Alternative(Box<Self>, Box<Self>),

    /// If-then-else conditional: `if .foo then .bar else .baz end`
    /// elif is desugared to nested If during parsing.
    If {
        cond: Box<Self>,
        then_branch: Box<Self>,
        else_branch: Box<Self>,
    },

    /// Try-catch error handling: `try .foo catch "default"`
    /// If catch is None, errors are silently suppressed (outputs nothing).
    Try {
        expr: Box<Self>,
        catch: Option<Box<Self>>,
    },

    /// Error raising: `error` or `error("message")`
    /// Raises an error that can be caught by try-catch.
    Error(Option<Box<Self>>),

    /// Builtin function call: `type`, `length`, `keys`, etc.
    Builtin(Builtin),

    /// String interpolation: `"Hello \(.name)"`
    /// Contains a sequence of literal parts and expression parts.
    StringInterpolation(Vec<StringPart>),

    /// Format string: `@json`, `@text`, `@uri`, etc.
    Format(FormatType),

    // Phase 8: Variables and Advanced Control Flow
    /// Variable binding: `.foo as $x | .bar + $x`
    As {
        /// Expression to evaluate and bind
        expr: Box<Self>,
        /// Variable name (without the $)
        var: String,
        /// Body expression where the variable is in scope
        body: Box<Self>,
    },

    /// Variable reference: `$x`
    Var(String),

    /// A frozen variable snapshot, synthesized only by variable
    /// substitution (`substitute_var_tracked`/`substitute_var_impl` in
    /// `eval.rs`) to replace an `Expr::Var` reference whose bind source
    /// was, at bind time, a pure passthrough of `.` -- never produced by
    /// the parser. `resolve_node`'s own arm for this variant decides
    /// path-trackability lazily, by comparing this snapshot against the
    /// ambient value it holds at the point of use (#844).
    ///
    /// `Rc`, not `Box`: a `$var` bound once outside a loop and referenced
    /// inside it gets re-embedded into a fresh substituted `Expr` tree on
    /// every loop iteration (`substitute_var_impl`'s ordinary AST-rebuild),
    /// and every one of those rebuilds clones this node. A `Box` clone
    /// deep-copies the whole snapshot -- O(document size) per iteration,
    /// measured as a genuine multi-second regression on a "bind the root
    /// once, loop, dereference it" filter (#844 review). `Rc::clone` is an
    /// O(1) refcount bump, so the snapshot is allocated once at the outer
    /// binding and shared, not repeatedly re-copied, by every inner
    /// iteration that re-embeds it.
    TrackedVar(Rc<OwnedValue>),

    /// Location reference: `$__loc__`
    /// Returns `{"file": "<stdin>", "line": N}` where N is the 1-based line number
    /// in the jq filter source where `$__loc__` appears.
    Loc {
        /// 1-based line number in the jq source
        line: usize,
    },

    /// Environment variables: `$ENV`
    /// Returns an object containing all environment variables.
    Env,

    /// Reduce: `reduce .[] as $x (0; . + $x)`, or `reduce .[] as {a: $a} (0; . + $a)`
    /// with a full destructuring pattern (#1201), or with `?//`-separated
    /// alternatives (`reduce .[] as [$a] ?// {a:$a} (0; . + $a)`, #1365) --
    /// always at least one pattern, `patterns.len() == 1` for the common
    /// non-`?//` case (mirroring `Expr::AsPattern`'s own always-a-`Vec`
    /// shape, since neither node has an analogous bare-`$var` fast path
    /// the way `Expr::As` is to `Expr::AsPattern`).
    Reduce {
        /// Input expression (what to iterate over)
        input: Box<Self>,
        /// Binding pattern alternatives for each element, tried in order
        /// (a bare `$var` is `Pattern::Var`)
        patterns: Vec<Pattern>,
        /// Initial accumulator value
        init: Box<Self>,
        /// Update expression (has access to accumulator via . and the pattern's bound variables)
        update: Box<Self>,
    },

    /// Foreach: `foreach .[] as $x (0; . + 1)` or `foreach .[] as $x (0; . + 1; .)`,
    /// or with a full destructuring pattern in place of `$x` (#1201), or
    /// `?//`-separated alternatives (#1365) -- same always-a-`Vec` shape
    /// as `Reduce` above.
    Foreach {
        /// Input expression
        input: Box<Self>,
        /// Binding pattern alternatives for each element, tried in order
        patterns: Vec<Pattern>,
        /// Initial accumulator value
        init: Box<Self>,
        /// Update expression
        update: Box<Self>,
        /// Extract expression (optional, defaults to identity)
        extract: Option<Box<Self>>,
    },

    /// Limit: `limit(n; expr)` - take first n outputs
    Limit {
        /// Number of outputs to take
        n: Box<Self>,
        /// Expression to limit
        expr: Box<Self>,
    },

    /// First with expression: `first(expr)` - first output of expr
    FirstExpr(Box<Self>),

    /// Last with expression: `last(expr)` - last output of expr
    LastExpr(Box<Self>),

    /// Nth with expression: `nth(n; expr)` - nth output of expr
    NthExpr { n: Box<Self>, expr: Box<Self> },

    /// Until: `until(cond; update)` - loop until condition is true
    Until { cond: Box<Self>, update: Box<Self> },

    /// While: `while(cond; update)` - loop while condition is true
    While { cond: Box<Self>, update: Box<Self> },

    /// Repeat: `repeat(expr)` - infinite repetition
    Repeat(Box<Self>),

    /// Range: `range(n)` or `range(a;b)` or `range(a;b;step)`
    Range {
        from: Box<Self>,
        to: Option<Box<Self>>,
        step: Option<Box<Self>>,
    },

    /// Label for non-local control flow: `label $name | expr`
    /// Establishes a scope that can be exited early with `break $name`
    Label {
        /// Label name (without the $)
        name: String,
        /// Body expression
        body: Box<Self>,
    },

    /// Break from a labeled scope: `break $name`
    /// Exits the nearest enclosing `label $name` scope
    Break(String),

    // Phase 9: Variables & Definitions
    /// Destructuring variable binding: `. as {name: $n, age: $a} | ...`
    /// or `. as [$first, $second] | ...`, optionally with `?//`-separated
    /// alternatives (`. as [$a] ?// {$a} | ...`): the first alternative
    /// whose pattern matches *and* whose body doesn't error is used: a
    /// pattern-match failure or a body-evaluation error (not `break`/
    /// `halt`/empty output) falls through to the next alternative, except
    /// on the last one, where either failure propagates normally. A single
    /// pattern (the common case) is just a one-element `patterns`.
    AsPattern {
        /// Expression to evaluate and destructure
        expr: Box<Self>,
        /// Pattern alternatives to try in order (`?//`-separated); usually
        /// just one
        patterns: Vec<Pattern>,
        /// Body expression where the variables are in scope
        body: Box<Self>,
    },

    /// Function definition: `def name: body;` or `def name(params): body;`
    /// The function is in scope for the `then` expression.
    FuncDef {
        /// Function name
        name: String,
        /// Parameter names (empty for no-arg functions)
        params: Vec<String>,
        /// Function body
        body: Box<Self>,
        /// Expression where this function is in scope
        then: Box<Self>,
        /// `then`, with this def's own calls installed -- computed on first
        /// evaluation of this node and reused afterwards (#2094). See
        /// [`FuncDefBound`]'s own doc comment for why this needs a different
        /// cache shape than `DefCall`'s own `bound` field. Every
        /// substitution pass that rebuilds this node's `body`/`then`/`params`
        /// must reset this to `FuncDefBound::default()`; a pass that merely
        /// clones the node through unchanged (nothing here was substituted
        /// into) may carry the cached value along.
        bound: FuncDefBound,
    },

    /// Function call: `name` or `name(args)`
    FuncCall {
        /// Function name
        name: String,
        /// Arguments (empty for no-arg calls)
        args: Vec<Self>,
    },

    /// A sub-expression that has already had every enclosing substitution
    /// applied to it, and must therefore be treated as **opaque** by every
    /// later substitution pass (#1371).
    ///
    /// Created when a user-defined function call binds its arguments: the
    /// argument expression is captured as-is, behind an [`Rc`], instead of
    /// being cloned into the callee's body. Two properties follow, and both
    /// are load-bearing:
    ///
    /// - **No growth with recursion depth.** `def sum_to(n): … sum_to(n-1) …`
    ///   used to inline the caller's own argument tree at every level, so the
    ///   argument at depth `d` had `O(d)` nodes and the tree being walked kept
    ///   growing — `O(d²)` work and native stack. Sharing makes each level add
    ///   `O(1)` nodes over a pointer to the level before, so the same
    ///   recursion is linear.
    /// - **Hygiene.** A substitution that skips this node cannot reach inside
    ///   an argument, so a binder in the callee (`3 as $x`, a `reduce`
    ///   pattern) can never capture a caller variable the argument mentions
    ///   (#2077).
    ///
    /// The contents are always fully substituted at construction time: the
    /// node is built while *evaluating* a call, and everything that could
    /// substitute into it lexically encloses that call and has therefore
    /// already run. Evaluation is transparent — a `Shared` evaluates exactly
    /// as its inner expression does.
    Shared(Rc<Self>),

    /// A call to a user-defined function, bound to its definition but **not
    /// yet substituted** (#1371).
    ///
    /// Installed in place of an [`Expr::FuncCall`] when the enclosing
    /// [`Expr::FuncDef`] is evaluated; the substitution of `args` into
    /// `def.body` happens later, when evaluation actually reaches this node.
    /// That split is the whole fix:
    ///
    /// - **Recursion terminates on its own.** Static substitution cannot see
    ///   that a runtime condition (`n == 0`) will eventually hold, so it
    ///   unrolls a self-recursive body forever and has to be cut off by a
    ///   guard. Substituting one level per evaluation instead lets the base
    ///   case simply not be evaluated — which is also why a branching body
    ///   (`fib`) works at all now, rather than exhausting a budget at every
    ///   depth including zero.
    /// - **`args` stay ordinary caller-scope code until the call runs.** They
    ///   are still reachable by an enclosing `as`-substitution, which an
    ///   argument captured at install time would not be — the call site can
    ///   sit inside a binder that has not been evaluated yet.
    DefCall {
        /// The definition this call resolves to, shared so that binding one
        /// more level of recursion costs a pointer clone, not a body clone.
        def: Rc<FuncDefData>,
        /// Argument expressions, in the caller's scope and unsubstituted.
        args: Vec<Self>,
        /// How many native evaluation frames are already live above this
        /// node, accumulated as the installer walks: one per structural level
        /// it descends, carried across each call so it sums over the whole
        /// live recursion. Carried in the node so the guard needs no ambient
        /// state and stays deterministic under `no_std`.
        ///
        /// A plain call *count* would not do. It is not the number of calls
        /// that exhausts the native stack but the frames each one holds live:
        /// measured at 256 MB, a body whose recursive call sits directly in a
        /// pipe survives ~57,500 levels, while one that wraps the same call in
        /// 40 array constructors — which stay live across it — dies between
        /// 4,000 and 8,000. Charging the structural nesting at each call site
        /// tracks that difference; a count cannot.
        frames: u32,
        /// The substituted body, remembered after the first evaluation of
        /// this node.
        bound: BoundBody,
    },

    /// Namespaced function call: `module::func` or `module::func(args)`
    NamespacedCall {
        /// Module namespace
        namespace: String,
        /// Function name
        name: String,
        /// Arguments (empty for no-arg calls)
        args: Vec<Self>,
    },

    // Assignment operators
    /// Simple assignment: `.a = value`
    /// Sets the path to the value and returns the modified input.
    Assign {
        /// Path expression (left side)
        path: Box<Self>,
        /// Value expression (right side)
        value: Box<Self>,
    },

    /// Update assignment: `.a |= f`
    /// Applies filter f to the value at path and updates it.
    Update {
        /// Path expression (left side)
        path: Box<Self>,
        /// Filter expression (right side)
        filter: Box<Self>,
    },

    /// Compound assignment: `.a += value`, `.a -= value`, etc.
    /// Equivalent to `.a |= . op value`
    CompoundAssign {
        /// Assignment operator type
        op: AssignOp,
        /// Path expression (left side)
        path: Box<Self>,
        /// Value expression (right side)
        value: Box<Self>,
    },

    /// Alternative assignment: `.a //= value`
    /// Sets path to value only if current value is null or false.
    AlternativeAssign {
        /// Path expression (left side)
        path: Box<Self>,
        /// Default value expression (right side)
        value: Box<Self>,
    },
}

/// A [`Expr::DefCall`]'s substituted body, computed on first evaluation and
/// reused afterwards (#1371).
///
/// Binding a call is a pure function of the node — substitute its `args` into
/// a copy of `def.body`, then install the definition over the result — so for
/// a given node the answer never changes and can be computed once. Without
/// this, moving substitution from "once, before evaluation" to "at the call"
/// charges it *per call*: a `def` used inside `.[]` over 200,000 elements
/// re-substituted its body 200,000 times, measured at +8% for one call per
/// element and +27% for three chained ones. With it, a repeated call site
/// pays once, and a recursion still substitutes per level because each level
/// installs its own fresh nodes.
///
/// Cloned along with the node it belongs to, which is safe wherever `args`
/// come along unchanged. **The two substitution passes rebuild `args` and so
/// must reset this** — they construct a fresh `BoundBody` rather than
/// carrying the old node's, since a cache keyed on arguments that just
/// changed is exactly a stale one.
///
/// Write-once by design (see [`Self::get_or_try_init`]/[`Self::get_or_init`]
/// below) — safe *only* because `DefCall.frames` is a plain field, frozen
/// once by whichever `install_def_calls` pass built this specific node, and
/// never re-read from ambient state at call time. [`Expr::FuncDef`]'s own
/// cache (#2094) needs a different shape for exactly that reason: see
/// [`FuncDefBound`]'s own doc comment.
#[derive(Clone, Default)]
pub struct BoundBody(core::cell::OnceCell<Rc<Expr>>);

impl BoundBody {
    /// The bound body, computing it with a fallible `bind` on first call.
    ///
    /// A failure (the recursion-depth guard) is deliberately **not** cached:
    /// it depends on nothing this node owns that could change, but leaving it
    /// uncached keeps the cache holding only successful, reusable results and
    /// costs nothing — a node that failed the guard is not evaluated again.
    pub fn get_or_try_init<E>(
        &self,
        bind: impl FnOnce() -> Result<Rc<Expr>, E>,
    ) -> Result<&Rc<Expr>, E> {
        if let Some(cached) = self.0.get() {
            return Ok(cached);
        }
        let bound = bind()?;
        Ok(self.0.get_or_init(|| bound))
    }
}

/// Two `DefCall`s are equal when their definition, arguments and frame count
/// are — whether either has been evaluated yet is derived state, not identity.
impl PartialEq for BoundBody {
    fn eq(&self, _other: &Self) -> bool {
        true
    }
}

/// Deliberately opaque: printing the cached body would make a node's `Debug`
/// output depend on whether it had been evaluated, which is the shape that
/// made `assert_eq!` on `{:?}` unreliable for YAML's own cursor cache.
impl core::fmt::Debug for BoundBody {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("BoundBody")
    }
}

/// An [`Expr::FuncDef`]'s "`then`, with this def's own calls installed" cache
/// (#2094).
///
/// Computed on first evaluation and reused afterwards -- unlike
/// [`BoundBody`], keyed additionally by the ambient frame depth
/// ([`super::eval::ambient_frame_depth`]) it was computed at, and
/// recomputed (not just left stale) whenever that depth differs from what
/// produced the cached answer.
///
/// The naive `OnceCell`-style write-once cache `BoundBody` uses is unsound
/// here: installing a `def` bakes the *current* ambient depth into every
/// nested `DefCall`'s own `frames` field (`bind_def`'s doc comment covers
/// why that's normally safe to treat as constant across repeat evaluations
/// of one node). But `Expr::Shared` sharing an `Rc<Expr>` unchanged across
/// substitution passes (by design, for #1371/#2077/#2096 hygiene) means the
/// *same* `FuncDef` node -- same `Rc`, same cache cell -- can genuinely be
/// reached at two different real depths without ever being rebuilt: pass a
/// self-recursive `def` as a function-valued argument, evaluate it once
/// directly (shallow) and once inside a deeper recursive call (e.g. `def
/// each(n; f): if n <= 0 then empty else (f, each(n - 1; f)) end`). A
/// write-once cache would freeze the shallow reach's `frames` baseline and
/// silently reuse it for the deep reach too, undercounting how close that
/// deep call actually is to `MAX_EVAL_FRAMES` -- exactly the unbounded
/// native-recursion gap #1098/#1016 exist to close.
///
/// Falling back to recomputing whenever the depth moves keeps the *common*
/// case (a `def` inline in a loop body, reached repeatedly at one constant
/// depth) fully cached, and only pays install cost again in the rarer
/// cross-depth-reuse shape above -- trading a little of that shape's own
/// performance for keeping the recursion guard sound in all cases.
#[derive(Clone, Default)]
pub struct FuncDefBound(core::cell::RefCell<Option<(u32, Rc<Expr>)>>);

impl FuncDefBound {
    /// The installed body for the given ambient `depth`, computing it fresh
    /// via `bind` if nothing is cached yet or the cached answer was computed
    /// at a different depth.
    pub fn get_or_init_at(&self, depth: u32, bind: impl FnOnce() -> Rc<Expr>) -> Rc<Expr> {
        if let Some((cached_depth, tree)) = self.0.borrow().as_ref() {
            if *cached_depth == depth {
                return Rc::clone(tree);
            }
        }
        let tree = bind();
        *self.0.borrow_mut() = Some((depth, Rc::clone(&tree)));
        tree
    }
}

/// Two `FuncDef`s are equal when their name/params/body/then are — whether
/// either has been evaluated yet, or at what depth, is derived state, not
/// identity (mirrors [`BoundBody`]'s own `PartialEq`).
impl PartialEq for FuncDefBound {
    fn eq(&self, _other: &Self) -> bool {
        true
    }
}

/// Deliberately opaque, for the same reason as [`BoundBody`]'s own `Debug`.
impl core::fmt::Debug for FuncDefBound {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("FuncDefBound")
    }
}

/// The three parts of a `def` that [`Expr::DefCall`] carries from the
/// definition to each of its call sites (#1371).
///
/// Split out of [`Expr::FuncDef`]'s inline fields so one `Rc` can hold all
/// three: re-installing the definition over each newly substituted body is
/// the innermost step of every recursive call, and cloning the definition
/// there -- rather than bumping one refcount -- would put a per-level copy
/// back into the path this design exists to keep flat.
#[derive(Debug, Clone, PartialEq)]
pub struct FuncDefData {
    /// Function name.
    pub name: String,
    /// Parameter names (empty for a no-argument definition).
    pub params: Vec<String>,
    /// Function body.
    pub body: Expr,
}

/// A complete jq program including module directives and the main expression.
#[derive(Debug, Clone, PartialEq)]
pub struct Program {
    /// Optional module metadata declaration
    pub module: Option<ModuleMeta>,
    /// Import directives
    pub imports: Vec<Import>,
    /// Include directives
    pub includes: Vec<Include>,
    /// The main expression (function definitions followed by the filter)
    pub expr: Expr,
}

impl Default for Program {
    fn default() -> Self {
        Self {
            module: None,
            imports: Vec::new(),
            includes: Vec::new(),
            expr: Expr::Identity,
        }
    }
}

impl Program {
    /// Create a program from just an expression (no module directives).
    pub fn from_expr(expr: Expr) -> Self {
        Self {
            module: None,
            imports: Vec::new(),
            includes: Vec::new(),
            expr,
        }
    }
}

/// Module metadata declaration: `module { ... };`
#[derive(Debug, Clone, PartialEq)]
pub struct ModuleMeta {
    /// Metadata key-value pairs
    pub metadata: BTreeMap<String, MetaValue>,
}

/// Values allowed in module metadata.
#[derive(Debug, Clone, PartialEq)]
pub enum MetaValue {
    /// String value
    String(String),
    /// Number value
    Number(f64),
    /// Boolean value
    Bool(bool),
    /// Array of values
    Array(Vec<Self>),
    /// Nested object
    Object(BTreeMap<String, Self>),
}

/// Import directive: `import "path" as name;` or `import "path" as name { meta };`
#[derive(Debug, Clone, PartialEq)]
pub struct Import {
    /// The module path (relative, without .jq extension)
    pub path: String,
    /// The namespace alias
    pub alias: String,
    /// Optional metadata overrides
    pub metadata: Option<BTreeMap<String, MetaValue>>,
}

/// Include directive: `include "path";` or `include "path" { meta };`
#[derive(Debug, Clone, PartialEq)]
pub struct Include {
    /// The module path (relative, without .jq extension)
    pub path: String,
    /// Optional metadata overrides
    pub metadata: Option<BTreeMap<String, MetaValue>>,
}

/// A pattern for destructuring variable binding.
///
/// Exclusively constructed by the parser's `parse_pattern` (`src/jq/parser.rs`),
/// which caps recursion at `MAX_PATTERN_DEPTH` (256) and returns a clean parse
/// error past that rather than overflowing the stack (#1240). Every recursive
/// `Pattern`-walking function elsewhere (`extract_pattern_bindings`,
/// `collect_pattern_var_names`, `pattern_binds_var` in `src/jq/eval.rs`) is
/// therefore transitively bounded by that same limit too, and deliberately
/// carries no depth counter of its own -- there is no other construction path
/// that could hand any of them a deeper tree to walk.
#[derive(Debug, Clone, PartialEq)]
pub enum Pattern {
    /// Simple variable: `$x`
    Var(String),
    /// Object pattern: `{name: $n, age: $a}`
    Object(Vec<PatternEntry>),
    /// Array pattern: `[$first, $second]`
    Array(Vec<Self>),
}

/// An entry in an object destructuring pattern.
#[derive(Debug, Clone, PartialEq)]
pub struct PatternEntry {
    /// The key to match (always a string literal in patterns)
    pub key: String,
    /// The pattern to bind the value to
    pub pattern: Pattern,
}

/// A part of a string interpolation expression.
#[derive(Debug, Clone, PartialEq)]
pub enum StringPart {
    /// Literal string content
    Literal(String),
    /// Expression to be evaluated and converted to string
    Expr(Box<Expr>),
}

/// Format string types for @format expressions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FormatType {
    /// @text - convert to string (same as tostring)
    Text,
    /// @json - format as JSON
    Json,
    /// @uri - URI encode
    Uri,
    /// @csv - CSV format (for arrays)
    Csv,
    /// @tsv - TSV format (for arrays)
    Tsv,
    /// @dsv(delimiter) - Generic DSV format with custom delimiter
    Dsv(String),
    /// @base64 - base64 encode
    Base64,
    /// @base64d - base64 decode
    Base64d,
    /// @html - HTML entity escape
    Html,
    /// @sh - shell quote
    Sh,
    /// @urid - URI decode (percent decoding)
    Urid,
    /// @yaml - format as YAML string (yq)
    Yaml,
    /// @props - Java properties format (yq)
    Props,
}

/// Builtin functions supported by jq.
#[derive(Debug, Clone, PartialEq)]
pub enum Builtin {
    // Type functions
    /// `type` - returns the type name as a string
    Type,
    /// `isnull` - returns true if null
    IsNull,
    /// `isboolean` - returns true if boolean
    IsBoolean,
    /// `isnumber` - returns true if number
    IsNumber,
    /// `isstring` - returns true if string
    IsString,
    /// `isarray` - returns true if array
    IsArray,
    /// `isobject` - returns true if object
    IsObject,

    // Type filter functions (select by type)
    /// `values` - select non-null values (equivalent to `select(. != null)`)
    Values,
    /// `nulls` - select only null values
    Nulls,
    /// `booleans` - select only boolean values
    Booleans,
    /// `numbers` - select only number values
    Numbers,
    /// `strings` - select only string values
    Strings,
    /// `arrays` - select only array values
    Arrays,
    /// `objects` - select only object values
    Objects,
    /// `iterables` - select arrays and objects
    Iterables,
    /// `scalars` - select non-iterables (null, bool, number, string)
    Scalars,

    // Length & Keys functions
    /// `length` - string/array/object length
    Length,
    /// `utf8bytelength` - byte length of string
    Utf8ByteLength,
    /// `keys` - sorted object keys or array indices
    Keys,
    /// `keys_unsorted` - object keys in original order
    KeysUnsorted,
    /// `has(key)` - check if object/array has key/index
    Has(Box<Expr>),
    /// `in(obj)` - check if key exists in object
    In(Box<Expr>),
    /// `IN(s)` - true if any output of s equals the current value
    UpperIn(Box<Expr>),
    /// `IN(src; s)` - true if any output of src equals any output of s
    UpperInSrc(Box<Expr>, Box<Expr>),

    // Selection & Filtering
    /// `select(condition)` - output input only if condition is truthy
    Select(Box<Expr>),
    /// `empty` - output nothing
    Empty,

    // Map & Iteration
    /// `map(f)` - apply f to each element: [.[] | f]
    Map(Box<Expr>),
    /// `map_values(f)` - apply f to each object value
    MapValues(Box<Expr>),

    // Reduction
    /// `add` - sum/concatenate array elements
    Add,
    /// `any` - true if any element is truthy
    Any,
    /// `any(cond)` - true if cond is truthy for any element of `.[]`
    AnyF(Box<Expr>),
    /// `any(gen; cond)` - true if cond is truthy for any output of gen
    AnyCond(Box<Expr>, Box<Expr>),
    /// `all` - true if all elements are truthy
    All,
    /// `all(cond)` - true if cond is truthy for every element of `.[]`
    AllF(Box<Expr>),
    /// `all(gen; cond)` - true if cond is truthy for every output of gen
    AllCond(Box<Expr>, Box<Expr>),
    /// `min` - minimum element
    Min,
    /// `max` - maximum element
    Max,
    /// `min_by(f)` - minimum element by key
    MinBy(Box<Expr>),
    /// `max_by(f)` - maximum element by key
    MaxBy(Box<Expr>),

    // Phase 5: String Functions
    /// `ascii_downcase` - lowercase ASCII characters
    AsciiDowncase,
    /// `ascii_upcase` - uppercase ASCII characters
    AsciiUpcase,
    /// `ltrimstr(s)` - remove prefix s
    Ltrimstr(Box<Expr>),
    /// `rtrimstr(s)` - remove suffix s
    Rtrimstr(Box<Expr>),
    /// `startswith(s)` - check if string starts with s
    Startswith(Box<Expr>),
    /// `endswith(s)` - check if string ends with s
    Endswith(Box<Expr>),
    /// `split(s)` - split string by separator
    Split(Box<Expr>),
    /// `join(s)` - join array elements with separator
    Join(Box<Expr>),
    /// `contains(b)` - check if input contains b
    Contains(Box<Expr>),
    /// `inside(b)` - check if input is inside b
    Inside(Box<Expr>),

    // Phase 5: Array Functions
    /// `first` - first element (`.[0]`)
    First,
    /// `last` - last element (`.[−1]`)
    Last,
    /// `nth(n)` - nth element
    Nth(Box<Expr>),
    /// `reverse` - reverse array
    Reverse,
    /// `flatten` - flatten nested arrays (1 level)
    Flatten,
    /// `flatten(depth)` - flatten to specific depth
    FlattenDepth(Box<Expr>),
    /// `group_by(f)` - group by key function
    GroupBy(Box<Expr>),
    /// `unique` - remove duplicates
    Unique,
    /// `unique_by(f)` - remove duplicates by key
    UniqueBy(Box<Expr>),
    /// `sort` - sort array
    Sort,
    /// `sort_by(f)` - sort by key function
    SortBy(Box<Expr>),

    // Phase 5: Object Functions
    /// `to_entries` - {k:v} → [{key:k, value:v}]
    ToEntries,
    /// `from_entries` - [{key:k, value:v}] → {k:v}
    FromEntries,
    /// `with_entries(f)` - to_entries | map(f) | from_entries
    WithEntries(Box<Expr>),

    // Phase 6: Type Conversions
    /// `tostring` - convert to string
    ToString,
    /// `tonumber` - convert to number
    ToNumber,
    /// `tojson` - convert value to JSON string
    ToJson,
    /// `fromjson` - parse JSON string to value
    FromJson,

    // Phase 6: Additional String Functions
    /// `explode` - string to array of codepoints
    Explode,
    /// `implode` - array of codepoints to string
    Implode,
    /// `test(re)` - test if regex matches (basic string contains for now)
    Test(Box<Expr>),
    /// `indices(s)` - array of indices where s occurs
    Indices(Box<Expr>),
    /// `index(s)` - first index of s, or null
    Index(Box<Expr>),
    /// `rindex(s)` - last index of s, or null
    Rindex(Box<Expr>),
    /// `INDEX(idx_expr)` - build an object keyed by idx_expr from `.[]`
    UpperIndex(Box<Expr>),
    /// `INDEX(stream; idx_expr)` - build an object keyed by idx_expr from stream
    UpperIndexStream(Box<Expr>, Box<Expr>),
    /// `tojsonstream` - convert to JSON stream format
    ToJsonStream,
    /// `fromjsonstream` - convert from JSON stream format
    FromJsonStream,
    /// `tostream` - jq-compatible stream of `[path,value]` / `[path]` events
    ToStream,
    /// `fromstream(f)` - reconstruct values from a stream of events produced by `f`
    FromStream(Box<Expr>),
    /// `truncate_stream(f)` - drop the leading `.` path components from `f`'s stream events
    TruncateStream(Box<Expr>),
    /// `getpath(path)` - get value at path
    GetPath(Box<Expr>),

    // Phase 8: Advanced Control Flow Builtins
    /// `recurse` - recursively apply .[] (same as recurse(.[];true))
    Recurse,
    /// `recurse(f)` - recursively apply f
    RecurseF(Box<Expr>),
    /// `recurse(f; cond)` - recurse while condition holds
    RecurseCond(Box<Expr>, Box<Expr>),
    /// `walk(f)` - apply f to all values bottom-up
    Walk(Box<Expr>),
    /// `isvalid(expr)` - true if expr produces at least one output without error
    IsValid(Box<Expr>),

    // Phase 10: Path Expressions
    /// `path(expr)` - return the path to values selected by expr
    Path(Box<Expr>),
    /// `path` (no-arg, yq) - return the current traversal path
    /// Used as `.a.b | path` to get `["a", "b"]`
    PathNoArg,
    /// `parent` (no-arg, yq) - return the parent node of the current position
    /// Used as `.a.b | parent` to get the value at `.a`
    Parent,
    /// `parent(n)` (yq) - return the nth parent node (0 = self, 1 = parent, etc.)
    ParentN(Box<Expr>),
    /// `paths` - all paths to values (excluding empty paths)
    Paths,
    /// `paths(filter)` - paths to values matching filter
    PathsFilter(Box<Expr>),
    /// `leaf_paths` - paths to scalar (non-container) values
    LeafPaths,
    /// `setpath(path; value)` - set value at path (returns modified copy)
    SetPath(Box<Expr>, Box<Expr>),
    /// `delpaths(paths)` - delete paths from value
    DelPaths(Box<Expr>),

    // Phase 10: Math Functions
    /// `floor` - floor of number
    Floor,
    /// `ceil` - ceiling of number
    Ceil,
    /// `round` - round to nearest integer
    Round,
    /// `sqrt` - square root
    Sqrt,
    /// `fabs` - absolute value
    Fabs,
    /// `log` - natural logarithm
    Log,
    /// `log10` - base-10 logarithm
    Log10,
    /// `log2` - base-2 logarithm
    Log2,
    /// `exp` - e^x
    Exp,
    /// `exp10` - 10^x
    Exp10,
    /// `exp2` - 2^x
    Exp2,
    /// `pow(x; y)` - x^y
    Pow(Box<Expr>, Box<Expr>),
    /// `sin` - sine
    Sin,
    /// `cos` - cosine
    Cos,
    /// `tan` - tangent
    Tan,
    /// `asin` - arc sine
    Asin,
    /// `acos` - arc cosine
    Acos,
    /// `atan` - arc tangent
    Atan,
    /// `atan(x; y)` - two-argument arc tangent
    Atan2(Box<Expr>, Box<Expr>),
    /// `sinh` - hyperbolic sine
    Sinh,
    /// `cosh` - hyperbolic cosine
    Cosh,
    /// `tanh` - hyperbolic tangent
    Tanh,
    /// `asinh` - inverse hyperbolic sine
    Asinh,
    /// `acosh` - inverse hyperbolic cosine
    Acosh,
    /// `atanh` - inverse hyperbolic tangent
    Atanh,

    // Phase 10: Number Classification & Constants
    /// `infinite` - positive infinity constant
    Infinite,
    /// `nan` - NaN constant
    Nan,
    /// `isinfinite` - true if value is infinite
    IsInfinite,
    /// `isnan` - true if value is NaN
    IsNan,
    /// `isnormal` - true if value is a normal number (not zero, infinite, NaN, or subnormal)
    IsNormal,
    /// `isfinite` - true if value is finite (not infinite or NaN)
    IsFinite,

    // Phase 10: Debug
    /// `debug` - output value to stderr, pass through unchanged
    Debug,
    /// `debug(msg)` - output message and value to stderr
    DebugMsg(Box<Expr>),

    // Process control (#791)
    /// `halt` - stop the interpreter immediately, exit code 0, no output
    Halt,
    /// `stderr` - print input in raw/compact mode to stderr (no trailing
    /// newline, not even for the passed-through value), pass through unchanged
    Stderr,
    /// `halt_error` - print input to stderr and exit (default code 5)
    HaltError,
    /// `halt_error(exit_code)` - print input to stderr and exit with the given code
    HaltErrorCode(Box<Expr>),

    // Phase 10: Environment
    /// `env` - object of all environment variables
    Env,
    /// `env.VAR` or `$ENV.VAR` - get environment variable (expression-based)
    EnvVar(Box<Expr>),
    /// `env(VAR_NAME)` - get environment variable by literal name (yq syntax)
    EnvObject(String),
    /// `strenv(VAR_NAME)` - get environment variable as string (yq syntax)
    StrEnv(String),

    // Phase 10: Null handling
    /// `null` - the null constant
    NullLit,

    // Phase 10: String functions
    /// `trim` - remove leading/trailing whitespace
    Trim,
    /// `ltrim` - remove leading whitespace
    Ltrim,
    /// `rtrim` - remove trailing whitespace
    Rtrim,

    // Phase 10: Array functions
    /// `transpose` - transpose array of arrays
    Transpose,
    /// `bsearch(x)` - binary search for x in sorted array
    BSearch(Box<Expr>),

    // Phase 10: Object functions
    /// `modulemeta` - get module metadata for the input module name (stub
    /// for compatibility). Real jq's builtin is arity 0, not arity 1 (#2035).
    ModuleMeta,
    /// `pick(keys)` - select only specified keys from object/array (yq)
    Pick(Box<Expr>),
    /// `omit(keys)` - remove specified keys from object/indices from array (yq)
    /// Inverse of `pick`: keeps all keys/indices except those specified.
    Omit(Box<Expr>),

    // YAML metadata functions (yq)
    /// `tag` - return YAML type tag (!!str, !!int, !!map, etc.)
    Tag,
    /// `anchor` - return anchor name if present, or empty string
    Anchor,
    /// `style` - return scalar style (double, single, literal, folded) or collection style (flow)
    Style,
    /// `kind` - return node kind: "scalar", "seq", or "map"
    Kind,
    /// `key` - return the current key when iterating (yq)
    Key,
    /// `line` - return the 1-based line number of the current node (yq)
    Line,
    /// `column` - return the 1-based column number of the current node (yq)
    Column,
    /// `document_index` / `di` - return the 0-indexed document position in multi-doc stream (yq)
    DocumentIndex,
    /// `line_comment` - return the trailing same-line comment text, or "" (yq, #710)
    LineComment,
    /// `file_index` / `fileIndex` / `fi` - return the 0-indexed origin file
    /// position within an `--eval-all` combined evaluation (yq/succinctly
    /// extension, #715). Resolves via the same `current_path`-derived
    /// mechanism as `key`, not `document_index`'s cursor mechanism, since
    /// only that path reaches the combined-array evaluation `--eval-all`
    /// requires. Outside `--eval-all` (or reachable through the same
    /// supported shapes `key`/`document_index` already document — plain
    /// navigation/`select`/comparisons, not `map`/`if`/literals/user
    /// functions), returns 0 -- the same "0 outside supported context"
    /// contract `document_index` already has today.
    FileIndex,
    /// `shuffle` - randomly shuffle array elements (yq)
    Shuffle,
    /// `pivot` - transpose arrays/objects (yq)
    /// For array of arrays: transposes rows/columns
    /// For array of objects: collects values by key
    Pivot,
    /// `split_doc` - marks outputs as separate YAML documents (yq)
    /// Each output from this operator should be printed with `---` separator.
    /// Semantically returns the input unchanged, but signals document boundary.
    SplitDoc,

    // Phase 11: Path manipulation
    /// `del(path)` - delete value at path
    Del(Box<Expr>),

    // Phase 12: Additional builtins
    /// `now` - current Unix timestamp
    Now,
    /// `input` - read and return the next input document, erroring once the
    /// input stream is exhausted -- real jq's own error text there is
    /// exactly `break` (confirmed live against jq 1.7.1), not the more
    /// descriptive "No more inputs" the C source's internal name suggests
    /// (#723)
    Input,
    /// `inputs` - generator yielding every remaining input document one at a
    /// time, stopping (not erroring) once exhausted (#723)
    Inputs,
    /// `input_line_number` - the line number of the most recently read
    /// input document (#723)
    InputLineNumber,
    /// `abs` - absolute value (alias for fabs)
    Abs,
    /// `builtins` - list all builtin function names
    Builtins,
    /// `normals` - select only normal numbers (not zero, infinite, NaN, or subnormal)
    Normals,
    /// `finites` - select only finite numbers (not infinite or NaN)
    Finites,

    // Phase 13: Iteration control
    /// `limit(n; expr)` - output at most n values from expr
    Limit(Box<Expr>, Box<Expr>),
    /// `first(expr)` - output only the first value from expr (sugar for limit(1; expr))
    /// Note: no-arg `first` uses `Builtin::First` from Phase 5
    FirstStream(Box<Expr>),
    /// `last(expr)` - output only the last value from expr
    /// Note: no-arg `last` uses `Builtin::Last` from Phase 5
    LastStream(Box<Expr>),
    /// `nth(n; expr)` - output only the nth value from expr (0-indexed)
    /// Note: no-arg `nth(n)` uses `Builtin::Nth` from Phase 5
    NthStream(Box<Expr>, Box<Expr>),
    /// `isempty(expr)` - returns true if expr produces no outputs
    IsEmpty(Box<Expr>),

    // Phase 14: Recursive traversal (extends Phase 8)
    /// `recurse_down` - recurse downward (alias for recurse)
    RecurseDown,

    // Phase 15: Date/Time functions
    /// `gmtime` - convert Unix timestamp to broken-down UTC time
    /// Returns [year, month(0-11), day(1-31), hour, minute, second, weekday(0-6), yearday(0-365)]
    Gmtime,
    /// `localtime` - convert Unix timestamp to broken-down local time
    /// Returns [year, month(0-11), day(1-31), hour, minute, second, weekday(0-6), yearday(0-365)]
    Localtime,
    /// `mktime` - convert broken-down time to Unix timestamp
    Mktime,
    /// `strftime(fmt)` - format broken-down time as string
    Strftime(Box<Expr>),
    /// `strptime(fmt)` - parse string to broken-down time
    Strptime(Box<Expr>),
    /// `todate` - convert Unix timestamp to ISO 8601 date string (alias for todateiso8601)
    Todate,
    /// `fromdate` - parse ISO 8601 date string to Unix timestamp (alias for fromdateiso8601)
    Fromdate,
    /// `todateiso8601` - convert Unix timestamp to ISO 8601 date string
    Todateiso8601,
    /// `fromdateiso8601` - parse ISO 8601 date string to Unix timestamp
    Fromdateiso8601,

    // Phase 16: Regex functions
    /// `test(re; flags)` - test if regex matches with flags
    /// Flags: "i" (case insensitive), "x" (extended), "s" (single-line), "m" (multi-line), "g" (global)
    TestFlags(Box<Expr>, Box<Expr>),
    /// `match(re)` - find first regex match, returning {offset, length, string, captures}
    Match(Box<Expr>),
    /// `match(re; flags)` - find regex match(es) with flags
    MatchFlags(Box<Expr>, Box<Expr>),
    /// `capture(re)` - capture named groups from first match, returning {name: value, ...}
    Capture(Box<Expr>),
    /// `capture(re; flags)` - capture named groups with flags
    CaptureFlags(Box<Expr>, Box<Expr>),
    /// `sub(re; replacement)` - replace first match
    Sub(Box<Expr>, Box<Expr>),
    /// `sub(re; replacement; flags)` - replace first match with flags
    SubFlags(Box<Expr>, Box<Expr>, Box<Expr>),
    /// `gsub(re; replacement)` - replace all matches
    Gsub(Box<Expr>, Box<Expr>),
    /// `gsub(re; replacement; flags)` - replace all matches with flags
    GsubFlags(Box<Expr>, Box<Expr>, Box<Expr>),
    /// `scan(re)` - find all matches, outputting each as a stream
    Scan(Box<Expr>),
    /// `scan(re; flags)` - find all matches with flags
    ScanFlags(Box<Expr>, Box<Expr>),
    /// `split(re; flags)` - split string by regex with flags
    SplitRegex(Box<Expr>, Box<Expr>),
    /// `splits(re)` - split string by regex, outputting as stream
    Splits(Box<Expr>),
    /// `splitsFlags(re; flags)` - split string by regex with flags, outputting as stream
    SplitsFlags(Box<Expr>, Box<Expr>),

    // Phase 17: Combinations
    /// `combinations` - generate all combinations from array of arrays
    ///
    /// Input: `[[1,2], [3,4]]` -> outputs `[1,3]`, `[1,4]`, `[2,3]`, `[2,4]`
    Combinations,
    /// `combinations(n)` - generate n-way combinations (Cartesian product with itself n times)
    ///
    /// Input with n=2: `[1,2]` -> outputs `[1,1]`, `[1,2]`, `[2,1]`, `[2,2]`
    CombinationsN(Box<Expr>),

    // Phase 18: Additional math functions
    /// `trunc` - truncate toward zero (remove fractional part)
    /// 2.7 -> 2, -2.7 -> -2
    Trunc,

    // Phase 19: Type conversion
    /// `toboolean` - convert to boolean
    /// Accepts: true, false, "true", "false"
    /// Errors on other types
    ToBoolean,

    // Phase 20: Iteration control extension
    /// `skip(n; expr)` - skip first n outputs from expr
    /// Outputs all remaining values after skipping the first n
    Skip(Box<Expr>, Box<Expr>),

    // Phase 21: Extended Date/Time functions (yq)
    /// `from_unix` - convert Unix epoch to ISO 8601 date string
    /// Input: 1705766400 -> "2024-01-20T16:00:00Z"
    FromUnix,
    /// `to_unix` - convert ISO 8601 date string to Unix epoch
    /// Input: "2024-01-20T16:00:00Z" -> 1705766400
    ToUnix,
    /// `tz(zone)` - convert Unix timestamp to datetime in specified timezone
    /// Input: now | tz("America/New_York") -> "2024-01-20T11:00:00-05:00"
    /// Supported zones: "UTC", "local", or IANA timezone names
    Tz(Box<Expr>),

    // Phase 22: File operations (yq)
    /// `load(file)` - load external YAML/JSON file and return its parsed content
    /// Input: load("config.yaml") -> {parsed content}
    /// Supports both YAML and JSON files (auto-detected by extension)
    Load(Box<Expr>),

    // Phase 23: Position-based navigation (succinctly extension)
    /// `at_offset(n)` - jump to node at byte offset n (0-indexed)
    /// Returns the value at the specified byte offset in the document.
    /// This is a succinctly-specific extension not available in standard jq.
    AtOffset(Box<Expr>),
    /// `at_position(line; col)` - jump to node at line/column (1-indexed)
    /// Returns the value at the specified line and column in the document.
    /// This is a succinctly-specific extension not available in standard jq.
    AtPosition(Box<Expr>, Box<Expr>),
}

/// Arithmetic operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArithOp {
    /// Addition: `+`
    Add,
    /// Subtraction: `-`
    Sub,
    /// Multiplication: `*`, or yq's merge operator with optional flag suffixes
    /// (`*+`, `*?`, `*n`, `*d`, `*c`, combinable, e.g. `*+d`).
    Mul(MergeFlags),
    /// Division: `/`
    Div,
    /// Modulo: `%`
    Mod,
}

/// yq merge-flag suffixes on the `*`/`*=` merge operator (e.g. `*+d`, `*=nd`).
/// Combinable and order-independent — each flag is an independent switch.
///
/// `clobber_tags` (`c`) is parsed and carried here but currently has no
/// observable effect: succinctly's YAML parser rejects custom tags outright,
/// so there is no tag data to clobber or preserve either way (issue #713).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MergeFlags {
    /// `+` — append arrays instead of merging/replacing by index.
    pub append_arrays: bool,
    /// `?` — only update fields/indices that already exist; never create new ones.
    pub only_existing: bool,
    /// `n` — only write fields/indices that don't already exist (or are `null`).
    pub only_new: bool,
    /// `d` — deep-merge arrays: treat them like objects, merging by index.
    pub deep_merge_arrays: bool,
    /// `c` — clobber custom tags (default preserves the left side's tag). No-op today.
    pub clobber_tags: bool,
}

/// Comparison operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompareOp {
    /// Equal: `==`
    Eq,
    /// Not equal: `!=`
    Ne,
    /// Less than: `<`
    Lt,
    /// Less than or equal: `<=`
    Le,
    /// Greater than: `>`
    Gt,
    /// Greater than or equal: `>=`
    Ge,
}

/// Compound assignment operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssignOp {
    /// Addition assignment: `+=`
    Add,
    /// Subtraction assignment: `-=`
    Sub,
    /// Multiplication/merge assignment: `*=`, with optional yq flag suffixes
    /// after the `=` (`*=+`, `*=?`, `*=n`, `*=d`, `*=c`, combinable, e.g. `*=+d`).
    Mul(MergeFlags),
    /// Division assignment: `/=`
    Div,
    /// Modulo assignment: `%=`
    Mod,
}

/// An entry in an object construction expression.
#[derive(Debug, Clone, PartialEq)]
pub struct ObjectEntry {
    /// The key expression. Can be a literal string or a dynamic expression.
    pub key: ObjectKey,
    /// The value expression.
    pub value: Expr,
}

/// Object key in construction - either literal or dynamic.
#[derive(Debug, Clone, PartialEq)]
pub enum ObjectKey {
    /// Literal string key: `{foo: .bar}`
    Literal(String),
    /// Dynamic key from expression: `{(.name): .value}`
    Expr(Box<Expr>),
}

/// The number an [`Expr::IndexNumber`] component is reported as by `path()`.
///
/// jq's rule for a numeric path component is that there is no rule: the
/// resolved key value is appended unchanged, so whatever spelling it still
/// carries is what comes back out (#1088). These are the two shapes a
/// *non-integer-spelled* key can be in by the time it reaches a component.
///
/// There is deliberately no `Int` arm. A plain integer renders identically
/// whether it goes out as `OwnedValue::Int` or as its own literal text, so
/// an integer key keeps folding to [`Expr::Index`] and nothing on the hot
/// navigation path changes shape.
///
/// The apparent "negative float indices collapse to integers" asymmetry
/// jq shows (`path(.[-1.0])` is `[-1]`, not `[-1.0]`) is *not* modelled
/// here, because it is not a rule about indices at all: jq's unary minus
/// destroys number-literal preservation, so `-1.0` in filter source is
/// already the computed double `-1` before any indexing happens — and jq
/// prints a whole double without a decimal point. A negative float that
/// arrives from *data* keeps its spelling in both
/// (`{"i":-1.00} | .i as $x | path(.a[$x])` is `["a",-1.00]`). The parser's
/// own negative-literal fold reproduces that by dropping to
/// [`Float`](Self::Float) whenever it negates.
#[derive(Debug, Clone, PartialEq)]
pub enum NumberKey {
    /// A computed float, with no source spelling left to preserve — the
    /// jq-compatible rendering of the `f64` is the component. Covers both
    /// a runtime `OwnedValue::Float` key and any literal the parser had to
    /// negate.
    Float(f64),
    /// A float-spelled literal that still carries its own source text,
    /// mirroring `OwnedValue::NumberLiteral`: `2.0`, `2.00`, `1e10`.
    ///
    /// The value is a bare `f64` rather than a [`NumberRepr`] because an
    /// `Int` repr cannot occur here by construction: an integer-spelled
    /// literal renders identically to its own `i64`, so it folds to
    /// [`Expr::Index`] and never reaches this type. Spelling that
    /// invariant into the field is what keeps [`Self::value`] total
    /// instead of carrying an arm nothing can reach.
    Literal(f64, Box<str>),
}

impl NumberKey {
    /// The `f64` this key denotes, spelling discarded.
    ///
    /// Used by the parser's negation fold, which is precisely the operation
    /// that discards the spelling — see this type's own doc comment.
    pub fn value(&self) -> f64 {
        match self {
            Self::Float(f) | Self::Literal(f, _) => *f,
        }
    }
}

/// Literal values that can appear in jq expressions.
#[derive(Debug, Clone, PartialEq)]
pub enum Literal {
    /// null
    Null,
    /// true or false
    Bool(bool),
    /// A number literal written directly in filter source text, keeping its
    /// original spelling (e.g. `1.500`, `1e2`) the same way document-parsed
    /// numbers do via `OwnedValue::NumberLiteral` (#1035) -- jq always
    /// echoes a number literal's own source text back out rather than a
    /// freshly-formatted `f64`/`i64` rendering, and `parse`'s tokenizer
    /// already has that text on hand for free. `Int`/`Float` below remain
    /// for internally-synthesized literals (e.g. desugaring) that have no
    /// source text to preserve.
    ///
    /// The `NumberRepr` is the same value `parse`'s tokenizer already
    /// computed (via `parse_i64_or_f64`) to decide whether this spelling
    /// even qualifies for `NumberLiteral` in the first place (#1062) --
    /// carrying it through means `literal_to_owned`/`eval_single` (via
    /// `From<Literal> for OwnedValue`) clone a pre-parsed `Copy` value on
    /// every evaluation of this AST node instead of re-running
    /// `i64`/`f64::from_str`. `JqValue::from_literal` (`lazy.rs`)
    /// deliberately does *not* benefit -- see its own doc comment for why
    /// its laziness means reading `repr` here at all would be premature,
    /// same shape as `OwnedValue::NumberLiteral(NumberRepr, Box<str>)`
    /// already uses on the read side.
    NumberLiteral(NumberRepr, String),
    /// Integer number
    Int(i64),
    /// Floating-point number
    Float(f64),
    /// String literal
    String(String),
}

impl Expr {
    /// The number this static array-index component reports itself as in
    /// `path()` output, if it is anything other than its own `i64`.
    ///
    /// Exists so the `Expr::Index(idx) | Expr::IndexNumber { idx, .. }`
    /// or-pattern stays a *single* arm at the three sites that render a
    /// path component: they bind `idx` from the pattern and ask for the
    /// spelling separately, instead of duplicating the body once per
    /// variant (#1088).
    pub(crate) fn index_number_key(&self) -> Option<&NumberKey> {
        match self {
            Self::IndexNumber { key, .. } => Some(key),
            _ => None,
        }
    }

    /// The numbers this static slice component's two bounds report
    /// themselves as in `path()` output, if either is anything other than
    /// its own `i64` -- the slice-bound sibling of [`Self::index_number_key`]
    /// (#1326), for the same reason: keeps the
    /// `Expr::Slice { start, end } | Expr::SliceNumber { start, end, .. }`
    /// or-pattern a single arm at the sites that render a path component.
    pub(crate) fn slice_number_keys(&self) -> (Option<&NumberKey>, Option<&NumberKey>) {
        match self {
            Self::SliceNumber {
                start_key, end_key, ..
            } => (start_key.as_ref(), end_key.as_ref()),
            _ => (None, None),
        }
    }

    /// Whether this is a slice path component, `Expr::Slice` or its
    /// float-bound sibling `Expr::SliceNumber` (#1326) -- one definition for
    /// the callers that only need "is this a slice", so a boolean-only check
    /// doesn't hand-copy the two-variant or-pattern a match arm that also
    /// needs `start`/`end` still has to spell out itself (CLAUDE.md:
    /// "duplicated predicates diverge silently", #106).
    pub(crate) fn is_slice(&self) -> bool {
        matches!(self, Self::Slice { .. } | Self::SliceNumber { .. })
    }

    /// Create an identity expression.
    pub fn identity() -> Self {
        Self::Identity
    }

    /// Create a field access expression.
    pub fn field(name: impl Into<String>) -> Self {
        Self::Field(name.into())
    }

    /// Create an index expression.
    pub fn index(i: i64) -> Self {
        Self::Index(i)
    }

    /// Create an iterate expression.
    pub fn iterate() -> Self {
        Self::Iterate
    }

    /// Create a computed-key index expression: `target[key]`.
    pub fn index_by(target: Self, key: Self) -> Self {
        Self::IndexExpr {
            target: Box::new(target),
            key: Box::new(key),
        }
    }

    /// Create a slice expression.
    pub fn slice(start: Option<i64>, end: Option<i64>) -> Self {
        Self::Slice { start, end }
    }

    /// Create a computed-bounds slice expression: `target[start:end]`.
    pub fn slice_by(target: Self, start: Option<Self>, end: Option<Self>) -> Self {
        Self::SliceExpr {
            target: Box::new(target),
            start: start.map(Box::new),
            end: end.map(Box::new),
        }
    }

    /// Make this expression optional.
    pub fn optional(self) -> Self {
        Self::Optional(Box::new(self))
    }

    /// Chain multiple expressions together.
    pub fn pipe(exprs: Vec<Self>) -> Self {
        if exprs.len() == 1 {
            exprs.into_iter().next().unwrap()
        } else {
            Self::Pipe(exprs)
        }
    }

    /// Create a comma expression (multiple outputs).
    pub fn comma(exprs: Vec<Self>) -> Self {
        if exprs.len() == 1 {
            exprs.into_iter().next().unwrap()
        } else {
            Self::Comma(exprs)
        }
    }

    /// Create an array construction expression.
    pub fn array(inner: Self) -> Self {
        Self::Array(Box::new(inner))
    }

    /// Create an object construction expression.
    pub fn object(entries: Vec<ObjectEntry>) -> Self {
        Self::Object(entries)
    }

    /// Create a literal expression.
    pub fn literal(lit: Literal) -> Self {
        Self::Literal(lit)
    }

    /// Create a recursive descent expression.
    pub fn recursive_descent() -> Self {
        Self::RecursiveDescent
    }

    /// Create a parenthesized expression.
    pub fn paren(inner: Self) -> Self {
        Self::Paren(Box::new(inner))
    }

    /// Create an arithmetic expression.
    pub fn arithmetic(op: ArithOp, left: Self, right: Self) -> Self {
        Self::Arithmetic {
            op,
            left: Box::new(left),
            right: Box::new(right),
        }
    }

    /// Create a comparison expression.
    pub fn compare(op: CompareOp, left: Self, right: Self) -> Self {
        Self::Compare {
            op,
            left: Box::new(left),
            right: Box::new(right),
        }
    }

    /// Create an AND expression.
    pub fn and(left: Self, right: Self) -> Self {
        Self::And(Box::new(left), Box::new(right))
    }

    /// Create an OR expression.
    pub fn or(left: Self, right: Self) -> Self {
        Self::Or(Box::new(left), Box::new(right))
    }

    /// Create a NOT expression.
    pub fn not() -> Self {
        Self::Not
    }

    /// Create an alternative expression.
    pub fn alternative(left: Self, right: Self) -> Self {
        Self::Alternative(Box::new(left), Box::new(right))
    }

    /// Create an if-then-else expression.
    pub fn if_then_else(cond: Self, then_branch: Self, else_branch: Self) -> Self {
        Self::If {
            cond: Box::new(cond),
            then_branch: Box::new(then_branch),
            else_branch: Box::new(else_branch),
        }
    }

    /// Create a try expression.
    pub fn try_expr(expr: Self, catch: Option<Self>) -> Self {
        Self::Try {
            expr: Box::new(expr),
            catch: catch.map(Box::new),
        }
    }

    /// Create an error expression.
    pub fn error(msg: Option<Self>) -> Self {
        Self::Error(msg.map(Box::new))
    }

    /// Create a builtin function expression.
    pub fn builtin(b: Builtin) -> Self {
        Self::Builtin(b)
    }

    /// Returns true if this is the identity expression.
    pub fn is_identity(&self) -> bool {
        matches!(self, Self::Identity)
    }
}

impl ObjectEntry {
    /// Create a new object entry with a literal key.
    pub fn new(key: impl Into<String>, value: Expr) -> Self {
        Self {
            key: ObjectKey::Literal(key.into()),
            value,
        }
    }

    /// Create a new object entry with a dynamic key.
    pub fn dynamic(key_expr: Expr, value: Expr) -> Self {
        Self {
            key: ObjectKey::Expr(Box::new(key_expr)),
            value,
        }
    }
}

impl Literal {
    /// Create a null literal.
    pub fn null() -> Self {
        Self::Null
    }

    /// Create a boolean literal.
    pub fn bool(b: bool) -> Self {
        Self::Bool(b)
    }

    /// Create an integer literal.
    pub fn int(n: i64) -> Self {
        Self::Int(n)
    }

    /// Create a float literal.
    pub fn float(f: f64) -> Self {
        Self::Float(f)
    }

    /// Create a string literal.
    pub fn string(s: impl Into<String>) -> Self {
        Self::String(s.into())
    }

    /// Create a source-text-preserving number literal (#1062), parsing
    /// `text` once here rather than leaving every call site to compute its
    /// own `NumberRepr`. Panics if `text` isn't RFC-8259-valid number
    /// syntax -- every real caller (the parser, `From<OwnedValue>`-style
    /// splices) already knows `text` parses, from having produced or
    /// validated it itself; this constructor exists for the many call
    /// sites (mostly tests) that only ever pass a literal they already
    /// know is valid.
    ///
    /// Gated on `is_valid_number`, not just `parse_i64_or_f64`: Rust's own
    /// `f64::from_str` accepts spellings like `"nan"`/`"inf"`/`"infinity"`
    /// that aren't valid JSON/jq number syntax at all -- without this
    /// check, this constructor would silently succeed on those instead of
    /// panicking as documented, unlike the real parser (which gates on the
    /// identical check before ever reaching `Literal::NumberLiteral`).
    #[track_caller]
    pub fn number_literal(text: impl Into<String>) -> Self {
        let text = text.into();
        // A direct `panic!` (not inside a closure) so `#[track_caller]`
        // actually blames the caller, not this function's own line --
        // `.unwrap_or_else(|| panic!(...))` would defeat it.
        if !crate::json::validate::is_valid_number(text.as_bytes()) {
            panic!("Literal::number_literal: {text:?} is not a valid number spelling");
        }
        // `is_valid_number` guarantees `parse_i64_or_f64` succeeds --
        // RFC-8259 number syntax always parses as `f64` at worst (extreme
        // magnitudes just become +/-infinity, never a parse failure).
        let repr = super::value::parse_i64_or_f64(&text)
            .expect("is_valid_number guarantees parse_i64_or_f64 succeeds");
        Self::NumberLiteral(repr, text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_expr_constructors() {
        assert_eq!(Expr::identity(), Expr::Identity);
        assert_eq!(Expr::field("foo"), Expr::Field("foo".into()));
        assert_eq!(Expr::index(0), Expr::Index(0));
        assert_eq!(Expr::iterate(), Expr::Iterate);
        assert_eq!(
            Expr::slice(Some(1), Some(3)),
            Expr::Slice {
                start: Some(1),
                end: Some(3)
            }
        );
    }

    /// #1827: pins the three shared predicates' own classification of each
    /// paired family's two members, so a future edit to `is_slice()`/
    /// `index_number_key()`/`slice_number_keys()` cannot silently start
    /// treating a plain/`*Number` pair differently. This alone does not
    /// catch a *third* future paired variant, or a dispatch site elsewhere
    /// that skips these predicates and pattern-matches the pair by hand
    /// (most sites do, via the `Expr::Index(idx) | Expr::IndexNumber {
    /// idx, .. }` or-pattern) — see
    /// `test_index_and_slice_number_siblings_behave_identically_1827` in
    /// `tests/jq_index_number_invariant_tests.rs` for the end-to-end
    /// behavioral check that covers those sites instead.
    #[test]
    fn test_index_and_slice_number_pairs_share_predicate_classification_1827() {
        let index = Expr::Index(1);
        let index_number = Expr::IndexNumber {
            idx: 1,
            key: NumberKey::Literal(1.0, "1.0".into()),
        };
        assert_eq!(index.index_number_key(), None);
        assert!(matches!(index_number.index_number_key(), Some(k) if k.value() == 1.0));
        assert!(!index.is_slice());
        assert!(!index_number.is_slice());

        let slice = Expr::Slice {
            start: Some(1),
            end: Some(3),
        };
        let slice_number = Expr::SliceNumber {
            start: Some(1),
            end: Some(3),
            start_key: Some(NumberKey::Literal(1.0, "1.0".into())),
            end_key: None,
        };
        assert_eq!(slice.slice_number_keys(), (None, None));
        let (start_key, end_key) = slice_number.slice_number_keys();
        assert!(matches!(start_key, Some(k) if k.value() == 1.0));
        assert_eq!(end_key, None);
        assert!(slice.is_slice());
        assert!(slice_number.is_slice());
    }

    #[test]
    fn test_pipe_simplification() {
        // Single element pipe simplifies to the element itself
        let single = Expr::pipe(vec![Expr::field("foo")]);
        assert_eq!(single, Expr::Field("foo".into()));

        // Multiple elements remain as pipe
        let multi = Expr::pipe(vec![Expr::field("foo"), Expr::field("bar")]);
        assert!(matches!(multi, Expr::Pipe(_)));
    }

    #[test]
    fn test_comma_simplification() {
        // Single element comma simplifies to the element itself
        let single = Expr::comma(vec![Expr::field("foo")]);
        assert_eq!(single, Expr::Field("foo".into()));

        // Multiple elements remain as comma
        let multi = Expr::comma(vec![Expr::field("foo"), Expr::field("bar")]);
        assert!(matches!(multi, Expr::Comma(_)));
    }

    #[test]
    fn test_array_construction() {
        let arr = Expr::array(Expr::iterate());
        assert!(matches!(arr, Expr::Array(_)));
    }

    #[test]
    fn test_object_construction() {
        let obj = Expr::object(vec![
            ObjectEntry::new("name", Expr::field("name")),
            ObjectEntry::dynamic(Expr::field("key"), Expr::field("value")),
        ]);
        assert!(matches!(obj, Expr::Object(_)));
    }

    #[test]
    fn test_literals() {
        assert_eq!(Expr::literal(Literal::null()), Expr::Literal(Literal::Null));
        assert_eq!(
            Expr::literal(Literal::bool(true)),
            Expr::Literal(Literal::Bool(true))
        );
        assert_eq!(
            Expr::literal(Literal::int(42)),
            Expr::Literal(Literal::Int(42))
        );
        assert_eq!(
            Expr::literal(Literal::string("hello")),
            Expr::Literal(Literal::String("hello".into()))
        );
    }

    #[test]
    fn test_literal_float() {
        assert_eq!(Literal::float(2.5), Literal::Float(2.5));
    }

    #[test]
    fn test_number_literal_carries_parsed_repr_1062() {
        assert_eq!(
            Literal::number_literal("1.500"),
            Literal::NumberLiteral(NumberRepr::Float(1.5), "1.500".to_string())
        );
        assert_eq!(
            Literal::number_literal("42"),
            Literal::NumberLiteral(NumberRepr::Int(42), "42".to_string())
        );
        assert_eq!(
            Literal::number_literal("1e2"),
            Literal::NumberLiteral(NumberRepr::Float(100.0), "1e2".to_string())
        );
    }

    #[test]
    #[should_panic(expected = "is not a valid number")]
    fn test_number_literal_panics_on_invalid_text_1062() {
        Literal::number_literal("not a number");
    }

    /// Rust's own `f64::from_str` accepts `"nan"`/`"inf"`/`"infinity"`, but
    /// none of those are valid JSON/jq number syntax -- `number_literal`
    /// must reject them too, not just gate on `parse_i64_or_f64` succeeding.
    #[test]
    #[should_panic(expected = "is not a valid number spelling")]
    fn test_number_literal_rejects_rust_only_float_spellings_1062() {
        Literal::number_literal("nan");
    }

    #[test]
    #[should_panic(expected = "is not a valid number spelling")]
    fn test_number_literal_rejects_infinity_spelling_1062() {
        Literal::number_literal("infinity");
    }

    #[test]
    fn test_optional_and_paren() {
        assert_eq!(
            Expr::field("x").optional(),
            Expr::Optional(Box::new(Expr::Field("x".into())))
        );
        assert_eq!(
            Expr::paren(Expr::identity()),
            Expr::Paren(Box::new(Expr::Identity))
        );
    }

    #[test]
    fn test_recursive_descent_and_not() {
        assert_eq!(Expr::recursive_descent(), Expr::RecursiveDescent);
        assert_eq!(Expr::not(), Expr::Not);
    }

    #[test]
    fn test_arithmetic_and_compare() {
        let one = || Expr::literal(Literal::int(1));
        let two = || Expr::literal(Literal::int(2));
        assert_eq!(
            Expr::arithmetic(ArithOp::Add, one(), two()),
            Expr::Arithmetic {
                op: ArithOp::Add,
                left: Box::new(Expr::Literal(Literal::Int(1))),
                right: Box::new(Expr::Literal(Literal::Int(2))),
            }
        );
        assert!(matches!(
            Expr::compare(CompareOp::Lt, one(), two()),
            Expr::Compare {
                op: CompareOp::Lt,
                ..
            }
        ));
    }

    #[test]
    fn test_boolean_and_alternative() {
        assert!(matches!(
            Expr::and(Expr::identity(), Expr::identity()),
            Expr::And(_, _)
        ));
        assert!(matches!(
            Expr::or(Expr::identity(), Expr::identity()),
            Expr::Or(_, _)
        ));
        assert!(matches!(
            Expr::alternative(Expr::identity(), Expr::literal(Literal::int(0))),
            Expr::Alternative(_, _)
        ));
    }

    #[test]
    fn test_if_try_error_builtin() {
        assert!(matches!(
            Expr::if_then_else(Expr::identity(), Expr::identity(), Expr::identity()),
            Expr::If { .. }
        ));
        assert!(matches!(
            Expr::try_expr(Expr::identity(), Some(Expr::identity())),
            Expr::Try { .. }
        ));
        assert!(matches!(Expr::error(None), Expr::Error(None)));
        assert_eq!(Expr::builtin(Builtin::Type), Expr::Builtin(Builtin::Type));
    }

    #[test]
    fn test_is_identity() {
        assert!(Expr::identity().is_identity());
        assert!(!Expr::field("x").is_identity());
    }

    #[test]
    fn test_program_from_expr() {
        let prog = Program::from_expr(Expr::identity());
        assert_eq!(prog.expr, Expr::Identity);
        assert!(prog.module.is_none());
        assert!(prog.imports.is_empty());
        assert!(prog.includes.is_empty());
    }

    /// #1371: `BoundBody` is derived state, so it must not participate in
    /// either of the two things `Expr` derives around it.
    ///
    /// **Equality** — two calls with the same definition, arguments and frame
    /// count are the same call, whether or not one of them has been evaluated
    /// yet. If the cache took part, a node would stop comparing equal to
    /// itself the moment it ran, which would silently change what every
    /// `assert_eq!` over an `Expr` in this crate means.
    ///
    /// **`Debug`** — the rendering must not change once the body is cached
    /// either. A cache that prints is the exact shape that made `assert_eq!`
    /// on `{:?}` unreliable for YAML's own sequential-cursor cache, where a
    /// shared index leaked its `Cell` into the formatted output.
    #[test]
    fn test_bound_body_is_invisible_to_eq_and_debug_1371() {
        let call = |bound| Expr::DefCall {
            def: Rc::new(FuncDefData {
                name: "f".into(),
                params: alloc_vec(["n"]),
                body: Expr::Identity,
            }),
            args: vec![Expr::Literal(Literal::Int(1))],
            frames: 3,
            bound,
        };
        let cold = call(BoundBody::default());

        // Populate the warm side's cache before it's ever wrapped in an
        // `Expr::DefCall`, so the two stay indistinguishable without needing
        // to destructure `bound` back out of `warm` afterwards.
        let warm_bound = BoundBody::default();
        let _ = warm_bound.get_or_try_init(|| Ok::<_, ()>(Rc::new(Expr::Identity)));
        let warm = call(warm_bound);

        assert_eq!(cold, warm, "the cache must not affect equality");
        assert_eq!(
            format!("{cold:?}"),
            format!("{warm:?}"),
            "the cache must not affect Debug output"
        );
    }

    /// Same invariant as [`test_bound_body_is_invisible_to_eq_and_debug_1371`],
    /// for [`FuncDefBound`] (#2094) -- and additionally covers that a
    /// *depth-mismatched* cache entry is just as invisible as an empty one,
    /// since `FuncDefBound` (unlike `BoundBody`) can hold a populated-but-
    /// stale entry rather than only empty-or-fresh.
    #[test]
    fn test_func_def_bound_is_invisible_to_eq_and_debug_2094() {
        let def = |bound| Expr::FuncDef {
            name: "f".into(),
            params: Vec::new(),
            body: Box::new(Expr::Identity),
            then: Box::new(Expr::Identity),
            bound,
        };
        let cold = def(FuncDefBound::default());

        let warm_bound = FuncDefBound::default();
        let _ = warm_bound.get_or_init_at(0, || Rc::new(Expr::Identity));
        let warm = def(warm_bound);

        let stale_bound = FuncDefBound::default();
        let _ = stale_bound.get_or_init_at(99, || Rc::new(Expr::Identity));
        let stale = def(stale_bound);

        for (label, other) in [("warm", &warm), ("stale", &stale)] {
            assert_eq!(cold, *other, "the cache must not affect equality ({label})");
            assert_eq!(
                format!("{cold:?}"),
                format!("{other:?}"),
                "the cache must not affect Debug output ({label})"
            );
        }
    }

    /// Helper: `Vec<String>` from string literals, without repeating the
    /// `to_string` dance at each call site.
    fn alloc_vec<const N: usize>(names: [&str; N]) -> Vec<String> {
        names.iter().map(|n| (*n).to_string()).collect()
    }
}
