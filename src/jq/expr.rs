//! Expression AST for jq-like queries.

#[cfg(not(test))]
use alloc::boxed::Box;
#[cfg(not(test))]
use alloc::string::String;
#[cfg(not(test))]
use alloc::vec::Vec;

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
    /// `first` - first element (.[0])
    First,
    /// `last` - last element (.[-1])
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
