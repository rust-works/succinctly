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

    /// Returns true if this is the identity expression.
    pub fn is_identity(&self) -> bool {
        matches!(self, Expr::Identity)
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
}
