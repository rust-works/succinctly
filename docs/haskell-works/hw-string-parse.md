# hw-string-parse Analysis

## Project Overview

**Repository:** [haskell-works/hw-string-parse](https://github.com/haskell-works/hw-string-parse)

**Version:** 0.0.0.5

**License:** BSD-3-Clause

**Author:** John Ky

**Description:** A simple parser combinator library for strings in Haskell.

### Purpose

`hw-string-parse` is a minimalist parser combinator library that provides the foundational building blocks for parsing strings. It implements the classic monadic parser combinator pattern, allowing users to compose small, simple parsers into more complex ones through Haskell's powerful type class abstractions.

The library is designed to be:
- **Simple:** A single module with ~60 lines of core implementation
- **Educational:** Demonstrates fundamental parser combinator concepts
- **Composable:** Leverages Haskell's `Functor`, `Applicative`, `Monad`, `Alternative`, and `MonadPlus` type classes

---

## Code Structure

```
hw-string-parse/
├── src/
│   └── HaskellWorks/
│       └── Data/
│           └── String/
│               └── Parse.hs      # Main and only source module
├── test/
│   └── Spec.hs                   # Test entry point (hspec-discover)
├── doctest/
│   └── DoctestDriver.hs          # Doctest configuration
├── hw-string-parse.cabal         # Build configuration
├── README.md                     # Project description
└── project.sh                    # Build helper script
```

### Key Module: `HaskellWorks.Data.String.Parse`

This is the sole module in the library, containing all parser combinator functionality.

---

## Core Implementation Details

### The Parser Type

```haskell
newtype Parser a = Parser { parse :: String -> [(a, String)] }
```

The parser is a newtype wrapper around a function that takes a `String` and returns a list of possible parse results. Each result is a tuple containing:
- The parsed value of type `a`
- The remaining unconsumed input `String`

This design follows the "list of successes" pattern, where:
- An empty list `[]` represents parse failure
- A single-element list represents a unique successful parse
- Multiple elements represent ambiguous parses

### Core Functions

| Function    | Type                                      | Purpose                                                |
|-------------|-------------------------------------------|--------------------------------------------------------|
| `runParser` | `Parser a -> String -> a`                 | Execute a parser, expecting complete input consumption |
| `item`      | `Parser Char`                             | Consume and return a single character                  |
| `unit`      | `a -> Parser a`                           | Wrap a value in a parser (monadic return)              |
| `bind`      | `Parser a -> (a -> Parser b) -> Parser b` | Sequential composition (monadic bind)                  |
| `failure`   | `Parser a`                                | A parser that always fails                             |
| `combine`   | `Parser a -> Parser a -> Parser a`        | Combine results from two parsers                       |
| `option`    | `Parser a -> Parser a -> Parser a`        | Try first parser, fallback to second                   |
| `char`      | `Char -> Parser Char`                     | Parse a specific character                             |

### Type Class Instances

The `Parser` type implements five key type classes:

1. **Functor** - Transform parsed values:
   ```haskell
   fmap f (Parser cs) = Parser (\s -> [(f a, b) | (a, b) <- cs s])
   ```

2. **Applicative** - Apply parsed functions to parsed values:
   ```haskell
   (Parser cs1) <*> (Parser cs2) = Parser (\s -> [(f a, s2) | (f, s1) <- cs1 s, (a, s2) <- cs2 s1])
   ```

3. **Monad** - Sequential composition with result dependency:
   ```haskell
   (>>=) = bind
   ```

4. **Alternative** - Choice between parsers:
   ```haskell
   empty = mzero
   (<|>) = option
   ```

5. **MonadPlus** - Combines Monad and Alternative:
   ```haskell
   mzero = failure
   mplus = combine
   ```

---

## Haskell Language Features Leveraged

### 1. Newtype Wrapper
```haskell
newtype Parser a = Parser { parse :: String -> [(a, String)] }
```
- Zero-cost abstraction at runtime
- Record syntax provides automatic accessor function `parse`
- Allows defining type class instances separate from the underlying function type

### 2. Type Classes and Instances
The library demonstrates the classic functional programming pattern of defining data types and their algebraic properties through type class instances.

### 3. List Comprehensions
```haskell
[(f a, b) | (a, b) <- cs s]
```
Elegant syntax for transforming and filtering list elements.

### 4. Pattern Matching
```haskell
case s of
  []     -> []
  (c:cs) -> [(c,cs)]
```
Destructuring strings into head and tail.

### 5. Higher-Order Functions
```haskell
concatMap (\(a, s') -> parse (f a) s') $ parse p s
```
Functions that take and return functions, enabling composition.

### 6. Do-Notation (Monadic Syntax Sugar)
```haskell
char c = do
  d <- item
  if c == d then return c else failure
```
Readable sequential composition of monadic operations.

### 7. Lambda Expressions
```haskell
Parser (\s -> [(a,s)])
```
Anonymous functions for concise inline definitions.

---

## Computer Science Techniques Used

### 1. Parser Combinators

The library implements the classic **monadic parser combinator** pattern, first popularized by:
- Hutton & Meijer's "Monadic Parser Combinators" (1996)
- Wadler's work on functional parsers

**Key characteristics:**
- Parsers are first-class values
- Complex parsers built by combining simple ones
- Backtracking through the list-of-successes model

### 2. List of Successes Technique

The return type `[(a, String)]` represents:
- **Non-deterministic parsing:** Multiple valid parse interpretations
- **Backtracking:** On failure, try alternative parses
- **Ambiguity handling:** Return all possible parses

This is a form of **non-deterministic computation** modeled as a list monad.

### 3. Algebraic Data Types and Abstract Algebra

The type class instances form algebraic structures:
- **Monoid-like behavior:** `failure` is the identity, `combine` is the operation
- **Alternative functor:** Combines applicative effects with choice
- **Monad laws:** Left identity, right identity, associativity

### 4. Lazy Evaluation

Haskell's lazy evaluation enables:
- **Efficient backtracking:** Unevaluated alternatives don't cost anything
- **Infinite parse possibilities:** Only computed as needed
- **Short-circuit failure:** Stop on first success with `option`

### 5. Continuation-Passing Style (Implicit)

The parser structure `String -> [(a, String)]` is related to CPS:
- The remaining string is the "continuation" of parsing
- Each step passes the remaining input to the next parser

---

## Rust Porting Considerations

### Feasibility Assessment: **HIGH**

This library is an excellent candidate for Rust porting due to:
1. Small, self-contained codebase (~60 lines)
2. Well-defined algebraic structure
3. No external dependencies (pure computation)
4. Clear type signatures

### Rust Feature Mapping

#### 1. The Parser Type

**Haskell:**
```haskell
newtype Parser a = Parser { parse :: String -> [(a, String)] }
```

**Rust equivalent:**
```rust
pub struct Parser<A, F>
where
    F: Fn(&str) -> Vec<(A, &str)>,
{
    parse: F,
}

// Or using dynamic dispatch:
pub struct Parser<A> {
    parse: Box<dyn Fn(&str) -> Vec<(A, &str)>>,
}

// Or with an associated type trait:
pub trait Parser {
    type Output;
    fn parse<'a>(&self, input: &'a str) -> Vec<(Self::Output, &'a str)>;
}
```

#### 2. Type Classes to Traits

| Haskell       | Rust                           | Notes                              |
|---------------|--------------------------------|------------------------------------|
| `Functor`     | `map()` method or custom trait | Rust doesn't have HKTs, use method |
| `Applicative` | Custom trait + methods         | No direct equivalent               |
| `Monad`       | `and_then()` / `flat_map()`    | Similar to `Iterator::flat_map`    |
| `Alternative` | `or()` method or custom trait  | Similar to `Option::or`            |
| `MonadPlus`   | Combined trait                 | Manual implementation              |

#### 3. Key Function Translations

**item (parse single char):**
```rust
fn item<'a>() -> impl Fn(&'a str) -> Vec<(char, &'a str)> {
    |s: &str| {
        s.chars().next()
            .map(|c| vec![(c, &s[c.len_utf8()..])])
            .unwrap_or_else(Vec::new)
    }
}
```

**unit (return/pure):**
```rust
fn unit<'a, A: Clone>(a: A) -> impl Fn(&'a str) -> Vec<(A, &'a str)> {
    move |s: &str| vec![(a.clone(), s)]
}
```

**bind (monadic bind):**
```rust
fn bind<'a, A, B, P, F, Q>(p: P, f: F) -> impl Fn(&'a str) -> Vec<(B, &'a str)>
where
    P: Fn(&'a str) -> Vec<(A, &'a str)>,
    F: Fn(A) -> Q,
    Q: Fn(&'a str) -> Vec<(B, &'a str)>,
{
    move |s: &str| {
        p(s).into_iter()
            .flat_map(|(a, s_prime)| f(a)(s_prime))
            .collect()
    }
}
```

### Challenges and Solutions

#### Challenge 1: No Higher-Kinded Types (HKTs)
Rust lacks HKTs, so we cannot define generic `Functor` or `Monad` traits.

**Solution:** Use method-based API or define parser-specific traits:
```rust
trait ParserExt<A>: Sized {
    fn map<B, F: Fn(A) -> B>(self, f: F) -> MappedParser<Self, F>;
    fn and_then<B, F: Fn(A) -> P, P: Parser<Output = B>>(self, f: F) -> FlatMappedParser<Self, F>;
    fn or<P: Parser<Output = A>>(self, other: P) -> OrParser<Self, P>;
}
```

#### Challenge 2: Ownership and Lifetimes
Haskell's GC allows free sharing; Rust requires explicit ownership.

**Solution:** Use `&str` slices with lifetimes, or `Clone` for owned strings:
```rust
// Borrowing approach (zero-copy, efficient)
fn parse<'a>(&self, input: &'a str) -> Vec<(A, &'a str)>;

// Owning approach (simpler but allocates)
fn parse(&self, input: String) -> Vec<(A, String)>;
```

#### Challenge 3: Closures and Function Composition
Rust closures have unique types, making composition tricky.

**Solution:** Use `Box<dyn Fn>` for dynamic dispatch or generics with trait bounds:
```rust
// Dynamic dispatch (easier, slight runtime cost)
type ParseFn<A> = Box<dyn Fn(&str) -> Vec<(A, &str)>>;

// Static dispatch (zero-cost, complex types)
fn combine<A, P1, P2>(p1: P1, p2: P2) -> impl Fn(&str) -> Vec<(A, &str)>
where
    P1: Fn(&str) -> Vec<(A, &str)>,
    P2: Fn(&str) -> Vec<(A, &str)>,
```

### Recommended Rust Architecture

```rust
// Core parser trait
pub trait Parser<'a> {
    type Output;
    fn parse(&self, input: &'a str) -> Vec<(Self::Output, &'a str)>;
}

// Combinator methods via extension trait
pub trait ParserCombinators<'a>: Parser<'a> + Sized {
    fn map<B, F: Fn(Self::Output) -> B>(self, f: F) -> Map<Self, F> { ... }
    fn and_then<B, P: Parser<'a, Output = B>, F: Fn(Self::Output) -> P>(self, f: F) -> AndThen<Self, F> { ... }
    fn or<P: Parser<'a, Output = Self::Output>>(self, other: P) -> Or<Self, P> { ... }
}

// Basic parsers
pub fn item<'a>() -> Item { Item }
pub fn char<'a>(c: char) -> Char { Char(c) }
pub fn pure<'a, A: Clone>(a: A) -> Pure<A> { Pure(a) }
pub fn fail<'a, A>() -> Fail<A> { Fail(PhantomData) }
```

### Existing Rust Parser Combinator Libraries

For reference, these libraries solve similar problems:
- **nom:** Zero-copy, byte-oriented, widely used
- **combine:** Closest to Haskell-style combinators
- **chumsky:** Modern, error-recovery focused
- **pest:** PEG-based, declarative

### Porting Effort Estimate

| Component        | Effort | Notes                                |
|------------------|--------|--------------------------------------|
| Core Parser type | Low    | Straightforward translation          |
| Functor (map)    | Low    | Simple method                        |
| Applicative      | Medium | Requires careful lifetime handling   |
| Monad (bind)     | Medium | flat_map style                       |
| Alternative (or) | Low    | Simple fallback                      |
| Type inference   | Medium | May need more explicit types         |
| Tests            | Low    | Property-based testing with proptest |

**Total estimated effort:** 1-2 hours for a basic port, 4-8 hours for a polished, idiomatic Rust library with full documentation and tests.

---

## Summary

`hw-string-parse` is a minimal but elegant implementation of the monadic parser combinator pattern in Haskell. It demonstrates how powerful abstractions can be built from simple primitives through type class composition.

**Key takeaways:**
1. The "list of successes" technique provides backtracking and ambiguity handling
2. Type class instances (`Functor`, `Applicative`, `Monad`, `Alternative`) enable compositional parsing
3. The library is highly portable to Rust with some adaptations for ownership and lack of HKTs
4. Existing Rust parser combinator libraries like `combine` provide similar functionality with more features

**Rust porting verdict:** Highly feasible. The main adaptation is replacing type classes with trait methods and handling ownership/lifetimes explicitly. The small size and clear structure make this an excellent learning exercise for understanding both Haskell and Rust approaches to parser combinators.
