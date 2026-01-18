//! Expression AST for jq-like queries.

#[cfg(not(test))]
use alloc::boxed::Box;
#[cfg(not(test))]
use alloc::collections::BTreeMap;
#[cfg(not(test))]
use alloc::string::String;
#[cfg(not(test))]
use alloc::vec::Vec;
#[cfg(test)]
use std::collections::BTreeMap;

/// A jq expression representing a query path.
#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    /// Identity: `.`
    Identity,

    /// Field access: `.foo`
    Field(String),

    /// Array index access: `.[0]` or `.[-1]`
    Index(i64),

    /// Array slice: `.[2:5]` or `.[2:]` or `.[:5]`
    Slice {
        start: Option<i64>,
        end: Option<i64>,
    },

    /// Iterate all elements: `.[]`
    Iterate,

    /// Optional access: `.foo?` - returns null instead of error if missing
    Optional(Box<Expr>),

    /// Chained expressions: `.foo.bar[0]`
    /// Each element is applied in sequence to the result of the previous.
    Pipe(Vec<Expr>),

    /// Comma operator: `.foo, .bar` - outputs from both expressions
    Comma(Vec<Expr>),

    /// Array construction: `[.foo, .bar]` or `[.items[]]`
    /// Collects all outputs from the inner expression into an array.
    Array(Box<Expr>),

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
    Paren(Box<Expr>),

    /// Arithmetic operation: `.a + .b`, `.a - .b`, `.a * .b`, `.a / .b`, `.a % .b`
    Arithmetic {
        op: ArithOp,
        left: Box<Expr>,
        right: Box<Expr>,
    },

    /// Comparison operation: `.a == .b`, `.a != .b`, `.a < .b`, etc.
    Compare {
        op: CompareOp,
        left: Box<Expr>,
        right: Box<Expr>,
    },

    /// Boolean AND: `.a and .b`
    And(Box<Expr>, Box<Expr>),

    /// Boolean OR: `.a or .b`
    Or(Box<Expr>, Box<Expr>),

    /// Boolean NOT: `not` (unary, applied via pipe)
    Not,

    /// Alternative operator: `.foo // "default"`
    /// Returns left if truthy, otherwise right.
    Alternative(Box<Expr>, Box<Expr>),

    /// If-then-else conditional: `if .foo then .bar else .baz end`
    /// elif is desugared to nested If during parsing.
    If {
        cond: Box<Expr>,
        then_branch: Box<Expr>,
        else_branch: Box<Expr>,
    },

    /// Try-catch error handling: `try .foo catch "default"`
    /// If catch is None, errors are silently suppressed (outputs nothing).
    Try {
        expr: Box<Expr>,
        catch: Option<Box<Expr>>,
    },

    /// Error raising: `error` or `error("message")`
    /// Raises an error that can be caught by try-catch.
    Error(Option<Box<Expr>>),

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
        expr: Box<Expr>,
        /// Variable name (without the $)
        var: String,
        /// Body expression where the variable is in scope
        body: Box<Expr>,
    },

    /// Variable reference: `$x`
    Var(String),

    /// Reduce: `reduce .[] as $x (0; . + $x)`
    Reduce {
        /// Input expression (what to iterate over)
        input: Box<Expr>,
        /// Variable name for each element
        var: String,
        /// Initial accumulator value
        init: Box<Expr>,
        /// Update expression (has access to accumulator via . and element via $var)
        update: Box<Expr>,
    },

    /// Foreach: `foreach .[] as $x (0; . + 1)` or `foreach .[] as $x (0; . + 1; .)`
    Foreach {
        /// Input expression
        input: Box<Expr>,
        /// Variable name for each element
        var: String,
        /// Initial accumulator value
        init: Box<Expr>,
        /// Update expression
        update: Box<Expr>,
        /// Extract expression (optional, defaults to identity)
        extract: Option<Box<Expr>>,
    },

    /// Limit: `limit(n; expr)` - take first n outputs
    Limit {
        /// Number of outputs to take
        n: Box<Expr>,
        /// Expression to limit
        expr: Box<Expr>,
    },

    /// First with expression: `first(expr)` - first output of expr
    FirstExpr(Box<Expr>),

    /// Last with expression: `last(expr)` - last output of expr
    LastExpr(Box<Expr>),

    /// Nth with expression: `nth(n; expr)` - nth output of expr
    NthExpr { n: Box<Expr>, expr: Box<Expr> },

    /// Until: `until(cond; update)` - loop until condition is true
    Until { cond: Box<Expr>, update: Box<Expr> },

    /// While: `while(cond; update)` - loop while condition is true
    While { cond: Box<Expr>, update: Box<Expr> },

    /// Repeat: `repeat(expr)` - infinite repetition
    Repeat(Box<Expr>),

    /// Range: `range(n)` or `range(a;b)` or `range(a;b;step)`
    Range {
        from: Box<Expr>,
        to: Option<Box<Expr>>,
        step: Option<Box<Expr>>,
    },

    // Phase 9: Variables & Definitions
    /// Destructuring variable binding: `. as {name: $n, age: $a} | ...`
    /// or `. as [$first, $second] | ...`
    AsPattern {
        /// Expression to evaluate and destructure
        expr: Box<Expr>,
        /// Pattern to match against
        pattern: Pattern,
        /// Body expression where the variables are in scope
        body: Box<Expr>,
    },

    /// Function definition: `def name: body;` or `def name(params): body;`
    /// The function is in scope for the `then` expression.
    FuncDef {
        /// Function name
        name: String,
        /// Parameter names (empty for no-arg functions)
        params: Vec<String>,
        /// Function body
        body: Box<Expr>,
        /// Expression where this function is in scope
        then: Box<Expr>,
    },

    /// Function call: `name` or `name(args)`
    FuncCall {
        /// Function name
        name: String,
        /// Arguments (empty for no-arg calls)
        args: Vec<Expr>,
    },

    /// Namespaced function call: `module::func` or `module::func(args)`
    NamespacedCall {
        /// Module namespace
        namespace: String,
        /// Function name
        name: String,
        /// Arguments (empty for no-arg calls)
        args: Vec<Expr>,
    },

    // Assignment operators
    /// Simple assignment: `.a = value`
    /// Sets the path to the value and returns the modified input.
    Assign {
        /// Path expression (left side)
        path: Box<Expr>,
        /// Value expression (right side)
        value: Box<Expr>,
    },

    /// Update assignment: `.a |= f`
    /// Applies filter f to the value at path and updates it.
    Update {
        /// Path expression (left side)
        path: Box<Expr>,
        /// Filter expression (right side)
        filter: Box<Expr>,
    },

    /// Compound assignment: `.a += value`, `.a -= value`, etc.
    /// Equivalent to `.a |= . op value`
    CompoundAssign {
        /// Assignment operator type
        op: AssignOp,
        /// Path expression (left side)
        path: Box<Expr>,
        /// Value expression (right side)
        value: Box<Expr>,
    },

    /// Alternative assignment: `.a //= value`
    /// Sets path to value only if current value is null or false.
    AlternativeAssign {
        /// Path expression (left side)
        path: Box<Expr>,
        /// Default value expression (right side)
        value: Box<Expr>,
    },
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
        Program {
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
        Program {
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
    Array(Vec<MetaValue>),
    /// Nested object
    Object(BTreeMap<String, MetaValue>),
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
#[derive(Debug, Clone, PartialEq)]
pub enum Pattern {
    /// Simple variable: `$x`
    Var(String),
    /// Object pattern: `{name: $n, age: $a}`
    Object(Vec<PatternEntry>),
    /// Array pattern: `[$first, $second]`
    Array(Vec<Pattern>),
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
    /// `all` - true if all elements are truthy
    All,
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
    /// `tojsonstream` - convert to JSON stream format
    ToJsonStream,
    /// `fromjsonstream` - convert from JSON stream format
    FromJsonStream,
    /// `getpath(path)` - get value at path
    GetPath(Box<Expr>),

    // Phase 7: Regular Expression Functions (requires "regex" feature)
    /// `match(re)` or `match(re; flags)` - return match object
    #[cfg(feature = "regex")]
    Match(Box<Expr>, Option<String>),
    /// `capture(re)` - return named captures as object
    #[cfg(feature = "regex")]
    Capture(Box<Expr>),
    /// `scan(re)` - find all matches
    #[cfg(feature = "regex")]
    Scan(Box<Expr>),
    /// `splits(re)` - split by regex (iterator version)
    #[cfg(feature = "regex")]
    Splits(Box<Expr>),
    /// `sub(re; replacement)` - replace first match
    #[cfg(feature = "regex")]
    Sub(Box<Expr>, Box<Expr>),
    /// `gsub(re; replacement)` - replace all matches
    #[cfg(feature = "regex")]
    Gsub(Box<Expr>, Box<Expr>),
    /// `test(re; flags)` - test with flags (extends basic test)
    #[cfg(feature = "regex")]
    TestWithFlags(Box<Expr>, String),

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
    /// `modulemeta(name)` - get module metadata (stub for compatibility)
    ModuleMeta(Box<Expr>),
    /// `pick(keys)` - select only specified keys from object/array (yq)
    Pick(Box<Expr>),

    // Phase 11: Path manipulation
    /// `del(path)` - delete value at path
    Del(Box<Expr>),
}

/// Arithmetic operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArithOp {
    /// Addition: `+`
    Add,
    /// Subtraction: `-`
    Sub,
    /// Multiplication: `*`
    Mul,
    /// Division: `/`
    Div,
    /// Modulo: `%`
    Mod,
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
    /// Multiplication assignment: `*=`
    Mul,
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

/// Literal values that can appear in jq expressions.
#[derive(Debug, Clone, PartialEq)]
pub enum Literal {
    /// null
    Null,
    /// true or false
    Bool(bool),
    /// Integer number
    Int(i64),
    /// Floating-point number
    Float(f64),
    /// String literal
    String(String),
}

impl Expr {
    /// Create an identity expression.
    pub fn identity() -> Self {
        Expr::Identity
    }

    /// Create a field access expression.
    pub fn field(name: impl Into<String>) -> Self {
        Expr::Field(name.into())
    }

    /// Create an index expression.
    pub fn index(i: i64) -> Self {
        Expr::Index(i)
    }

    /// Create an iterate expression.
    pub fn iterate() -> Self {
        Expr::Iterate
    }

    /// Create a slice expression.
    pub fn slice(start: Option<i64>, end: Option<i64>) -> Self {
        Expr::Slice { start, end }
    }

    /// Make this expression optional.
    pub fn optional(self) -> Self {
        Expr::Optional(Box::new(self))
    }

    /// Chain multiple expressions together.
    pub fn pipe(exprs: Vec<Expr>) -> Self {
        if exprs.len() == 1 {
            exprs.into_iter().next().unwrap()
        } else {
            Expr::Pipe(exprs)
        }
    }

    /// Create a comma expression (multiple outputs).
    pub fn comma(exprs: Vec<Expr>) -> Self {
        if exprs.len() == 1 {
            exprs.into_iter().next().unwrap()
        } else {
            Expr::Comma(exprs)
        }
    }

    /// Create an array construction expression.
    pub fn array(inner: Expr) -> Self {
        Expr::Array(Box::new(inner))
    }

    /// Create an object construction expression.
    pub fn object(entries: Vec<ObjectEntry>) -> Self {
        Expr::Object(entries)
    }

    /// Create a literal expression.
    pub fn literal(lit: Literal) -> Self {
        Expr::Literal(lit)
    }

    /// Create a recursive descent expression.
    pub fn recursive_descent() -> Self {
        Expr::RecursiveDescent
    }

    /// Create a parenthesized expression.
    pub fn paren(inner: Expr) -> Self {
        Expr::Paren(Box::new(inner))
    }

    /// Create an arithmetic expression.
    pub fn arithmetic(op: ArithOp, left: Expr, right: Expr) -> Self {
        Expr::Arithmetic {
            op,
            left: Box::new(left),
            right: Box::new(right),
        }
    }

    /// Create a comparison expression.
    pub fn compare(op: CompareOp, left: Expr, right: Expr) -> Self {
        Expr::Compare {
            op,
            left: Box::new(left),
            right: Box::new(right),
        }
    }

    /// Create an AND expression.
    pub fn and(left: Expr, right: Expr) -> Self {
        Expr::And(Box::new(left), Box::new(right))
    }

    /// Create an OR expression.
    pub fn or(left: Expr, right: Expr) -> Self {
        Expr::Or(Box::new(left), Box::new(right))
    }

    /// Create a NOT expression.
    pub fn not() -> Self {
        Expr::Not
    }

    /// Create an alternative expression.
    pub fn alternative(left: Expr, right: Expr) -> Self {
        Expr::Alternative(Box::new(left), Box::new(right))
    }

    /// Create an if-then-else expression.
    pub fn if_then_else(cond: Expr, then_branch: Expr, else_branch: Expr) -> Self {
        Expr::If {
            cond: Box::new(cond),
            then_branch: Box::new(then_branch),
            else_branch: Box::new(else_branch),
        }
    }

    /// Create a try expression.
    pub fn try_expr(expr: Expr, catch: Option<Expr>) -> Self {
        Expr::Try {
            expr: Box::new(expr),
            catch: catch.map(Box::new),
        }
    }

    /// Create an error expression.
    pub fn error(msg: Option<Expr>) -> Self {
        Expr::Error(msg.map(Box::new))
    }

    /// Create a builtin function expression.
    pub fn builtin(b: Builtin) -> Self {
        Expr::Builtin(b)
    }

    /// Returns true if this is the identity expression.
    pub fn is_identity(&self) -> bool {
        matches!(self, Expr::Identity)
    }
}

impl ObjectEntry {
    /// Create a new object entry with a literal key.
    pub fn new(key: impl Into<String>, value: Expr) -> Self {
        ObjectEntry {
            key: ObjectKey::Literal(key.into()),
            value,
        }
    }

    /// Create a new object entry with a dynamic key.
    pub fn dynamic(key_expr: Expr, value: Expr) -> Self {
        ObjectEntry {
            key: ObjectKey::Expr(Box::new(key_expr)),
            value,
        }
    }
}

impl Literal {
    /// Create a null literal.
    pub fn null() -> Self {
        Literal::Null
    }

    /// Create a boolean literal.
    pub fn bool(b: bool) -> Self {
        Literal::Bool(b)
    }

    /// Create an integer literal.
    pub fn int(n: i64) -> Self {
        Literal::Int(n)
    }

    /// Create a float literal.
    pub fn float(f: f64) -> Self {
        Literal::Float(f)
    }

    /// Create a string literal.
    pub fn string(s: impl Into<String>) -> Self {
        Literal::String(s.into())
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
}
