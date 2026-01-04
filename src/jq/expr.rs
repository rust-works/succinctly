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
