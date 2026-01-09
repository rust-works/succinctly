# hw-mquery: Monadic Query DSL

## Project Overview

**Repository:** https://github.com/haskell-works/hw-mquery
**Author:** John Ky
**License:** BSD-3-Clause
**Version:** 0.2.1.1
**Tested GHC Versions:** 9.2.2, 9.0.2, 8.10.7, 8.8.4, 8.6.5

### Purpose

hw-mquery provides a **monadic domain-specific language (DSL) for querying structured data**. It is designed to work with JSON documents and other hierarchical data structures, enabling interactive exploration and extraction of data using a fluent, composable query syntax.

The library is part of the haskell-works ecosystem and is designed to integrate with:
- `hw-json` - High-performance JSON parsing using succinct data structures
- Lens-based data access patterns
- Pretty-printing with ANSI terminal colors for REPL-style exploration

### Key Features

1. **Monadic Query Composition**: Queries are built using standard monadic operators (`>>=`, `>=>`, `do`-notation)
2. **Difference Lists for Efficiency**: Results are collected in `DList` for O(1) concatenation
3. **Lens Integration**: Custom operators for seamless lens-based field access
4. **Pretty Printing**: Colorized terminal output for interactive exploration
5. **Pagination and Filtering**: Built-in support for `limit`, `skip`, `page`, `sorted`, and `satisfying`

---

## Code Structure

```
hw-mquery/
├── src/HaskellWorks/Data/
│   ├── MQuery.hs              # Core MQuery monad and query combinators
│   ├── MQuery/
│   │   ├── Ann.hs             # Annotations for styled output
│   │   ├── AtLeastSize.hs     # Efficient size-checking typeclass
│   │   ├── Entry.hs           # Key-value entry type
│   │   ├── Micro.hs           # Compact pretty-printing wrapper
│   │   ├── Mini.hs            # Medium-sized pretty-printing wrapper
│   │   ├── Row.hs             # Row-based output formatting
│   │   ├── Shows.hs           # Show-based string building
│   │   ├── Style.hs           # ANSI terminal styling
│   │   └── ToBool.hs          # Boolean coercion typeclass
├── app/Main.hs                # Example application (placeholder)
├── test/
│   ├── Spec.hs                # HSpec test discovery
│   └── HaskellWorks/Data/
│       ├── MQuerySpec.hs      # Core MQuery tests
│       └── Model/             # Test data models
│           ├── Type.hs        # Mount/Storage ADTs
│           ├── Lens.hs        # Template Haskell lens generation
│           └── Example.hs     # Example mount configurations
└── doctest/DoctestDriver.hs   # Doctest integration
```

---

## Key Modules Analysis

### 1. MQuery.hs - Core Monad

The heart of the library is the `MQuery` newtype:

```haskell
newtype MQuery a = MQuery (DL.DList a)
```

**Derived Instances:**
- `Functor` - Transform results
- `Applicative` - Combine independent queries
- `Monad` - Chain dependent queries
- `Alternative` / `MonadPlus` - Represent failure/choice
- `Foldable` - Iterate over results
- `Semigroup` / `Monoid` - Combine result sets

**Core Query Combinators:**

| Function     | Signature                              | Purpose                                  |
|--------------|----------------------------------------|------------------------------------------|
| `satisfying` | `(a -> Bool) -> a -> MQuery a`         | Filter by predicate                      |
| `having`     | `(a -> MQuery b) -> a -> MQuery a`     | Filter by existence of sub-query results |
| `select`     | `a -> (a -> b) -> MQuery a`            | Filter by ToBool result                  |
| `valueOf`    | `Eq a => a -> a -> MQuery a`           | Exact value match                        |
| `key`        | `Entry k v -> MQuery k`                | Extract entry key                        |
| `value`      | `Entry k v -> MQuery v`                | Extract entry value                      |
| `limit`      | `Int -> MQuery a -> MQuery a`          | Take first n results                     |
| `skip`       | `Int -> MQuery a -> MQuery a`          | Drop first n results                     |
| `page`       | `Int -> Int -> MQuery a -> MQuery a`   | Pagination (size, page number)           |
| `sorted`     | `Ord a => MQuery a -> MQuery a`        | Sort results                             |
| `onList`     | `([a] -> [a]) -> MQuery a -> MQuery a` | Apply list transformation                |
| `count`      | `MQuery a -> MQuery Int`               | Count results                            |
| `aggregate`  | `([a] -> b) -> MQuery a -> MQuery b`   | Aggregate results                        |
| `uniq`       | `Eq a => [a] -> [a]`                   | Remove consecutive duplicates            |

**Lens Integration Operators:**

```haskell
(/^.)  :: Monad m => s -> Getting a s a -> m a
(/^..) :: (Monad m, Foldable t, Monoid (m a)) => s -> Getting (t a) s (t a) -> m a
(>>^.) :: Monad m => m a -> Getting b a b -> m b
(>>^..) :: (Monad m, Foldable t, Monoid (m a), Monoid (m b)) => m a -> Getting (t b) a (t b) -> m b
```

These operators combine monadic binding with lens access, enabling queries like:
```haskell
q >>^. L.readOnly      -- Extract readOnly field from each result
q >>^.. L.options      -- Flatten all options into result set
```

### 2. Entry.hs - Key-Value Pairs

```haskell
data Entry k v = Entry k v
```

A simple key-value pair type used for representing object entries during JSON traversal.

### 3. ToBool.hs - Boolean Coercion

```haskell
class ToBool a where
  toBool :: a -> Bool

instance ToBool Bool where toBool = id
instance ToBool [a] where toBool = not . null
instance ToBool (DL.DList a) where toBool = toBool . DL.toList
```

Enables JavaScript-like "truthiness" checks - empty lists are falsy, non-empty are truthy.

### 4. AtLeastSize.hs - Efficient Size Checking

```haskell
class AtLeastSize a where
  atLeastSize :: a -> Int -> Bool

instance AtLeastSize [a] where
  atLeastSize _      0 = True
  atLeastSize (_:as) n | n > 0 = atLeastSize as (n - 1)
  atLeastSize _      _ = False
```

Efficiently checks if a list has at least N elements without traversing the entire list. Used for truncated pretty-printing.

### 5. Micro.hs and Mini.hs - Pretty Printing Wrappers

**Micro** - Ultra-compact representation:
- Shows up to 10 elements in inline format
- Adds `..` suffix for truncated results

**Mini** - Slightly larger format:
- Also limits to 10 elements
- Uses indentation for readability

### 6. Style.hs - Terminal Styling

A complete ANSI terminal styling system:

```haskell
data SetStyle = SetStyle
  { ansiReset            :: Bool
  , ansiForeground       :: Maybe (ColorIntensity, Color)
  , ansiBackground       :: Maybe (ColorIntensity, Color)
  , ansiConsoleIntensity :: Maybe ConsoleIntensity
  , ansiItalics          :: Maybe Italicized
  , ansiUnderlining      :: Maybe Underlining
  }

data Color = Black | Red | Green | Yellow | Blue | Magenta | Cyan | White
```

Provides composable styling with `Semigroup`/`Monoid` instances.

### 7. Row.hs and Ann.hs - Output Formatting

- `Row` wraps results with a maximum character width
- `Ann` (annotation) type for pretty-printer styling
- Output prefixed with bold yellow `==>` markers

---

## Haskell Language Features Leveraged

### 1. Newtype Deriving and Standalone Deriving

```haskell
newtype MQuery a = MQuery (DL.DList a)

deriving instance Functor     MQuery
deriving instance Applicative MQuery
deriving instance Monad       MQuery
deriving instance Alternative MQuery
deriving instance MonadPlus   MQuery
deriving instance Foldable    MQuery
```

The library leverages GHC's `GeneralizedNewtypeDeriving` and `StandaloneDeriving` to automatically derive all standard typeclass instances from the underlying `DList`.

### 2. Type Families

```haskell
class IsPredicate a where
  type ArgOf a
  toPredicate :: ArgOf a -> (a -> Bool)
```

Uses associated type families to create type-level relationships for predicate construction.

### 3. Flexible Instances and Contexts

```haskell
{-# LANGUAGE FlexibleInstances #-}
{-# LANGUAGE FlexibleContexts  #-}

instance Pretty (Micro a) => Pretty (Mini [a]) where ...
instance Pretty a => Pretty (Micro (DL.DList a)) where ...
```

Allows instance definitions with complex type expressions.

### 4. Functional Dependencies

```haskell
{-# LANGUAGE FunctionalDependencies #-}
{-# LANGUAGE MultiParamTypeClasses  #-}
```

Used in lens generation for type inference.

### 5. Template Haskell

```haskell
makeFields ''Storage
makeFields ''Mount
```

Automatic lens generation from record types.

### 6. Operator Sections and Composition

The DSL heavily uses:
- Kleisli composition (`>=>`)
- Reverse function application (`&`)
- Custom infix operators (`>>^.`, `>>^..`, `/^.`, `/^..`)

### 7. OverloadedStrings

```haskell
pretty (Entry k v) = pretty k <> ": " <> pretty v
```

String literals are polymorphic `Doc` types.

---

## Computer Science Techniques

### 1. Difference Lists (DList)

The core data structure uses difference lists for O(1) append operations:

```haskell
newtype MQuery a = MQuery (DL.DList a)
```

**Why DLists?**
- Standard Haskell lists have O(n) concatenation
- DLists represent lists as function composition: `[1,2,3]` becomes `\xs -> 1:2:3:xs`
- Concatenation becomes O(1) function composition
- Critical for query operations that aggregate many results

### 2. Monadic Query Composition

The library implements a **List Monad with efficient concatenation**:

```haskell
instance Monad MQuery where
  MQuery xs >>= f = MQuery $ DL.concat $ map (mQuery . f) $ DL.toList xs
```

This enables:
- Non-deterministic computation (multiple results)
- Filtering through `guard` / `Alternative`
- Chained navigation through nested structures

### 3. Domain-Specific Language (DSL) Design

The API follows DSL design principles:
- **Fluent interface**: Method chaining with `&` operator
- **Composable primitives**: Small functions that combine well
- **Monadic embedding**: Uses Haskell's `do`-notation as syntax

Example query from README:
```haskell
q >>= item >>= entry >>= named "name" & limit 10
```

### 4. Lazy Evaluation for Pagination

Operations like `limit` and `page` leverage Haskell's laziness:

```haskell
limit n (MQuery xs) = MQuery ((DL.fromList . take n . DL.toList) xs)
```

Only the first `n` elements are computed, even if the source is infinite or expensive.

### 5. Typeclass-Based Polymorphism

The `ToBool` and `AtLeastSize` typeclasses provide:
- Ad-hoc polymorphism for boolean coercion
- Short-circuit evaluation for size checks
- Extensibility for new types

### 6. Pretty Printer Algebra

Uses the `prettyprinter` library's algebraic document representation:
- `<>` for horizontal concatenation
- `vcat` for vertical layout
- `indent` for nested formatting
- Annotations for deferred styling decisions

---

## Rust Porting Considerations

### Feasibility Assessment

**Overall Feasibility: HIGH**

The core concepts translate well to Rust, though the implementation strategy differs significantly.

### Direct Translations

| Haskell Concept                          | Rust Equivalent                               |
|------------------------------------------|-----------------------------------------------|
| `newtype MQuery a = MQuery (DL.DList a)` | `struct MQuery<T>(Vec<T>)` or custom iterator |
| `Functor`                                | `Iterator::map()`                             |
| `Foldable`                               | `Iterator` trait                              |
| `Monoid`                                 | `Default + Extend` or custom trait            |
| `Pretty`                                 | `Display` or `Debug`                          |
| `ToBool`                                 | Custom trait or `Into<bool>`                  |

### Rust Implementation Strategies

#### 1. Iterator-Based Approach (Recommended)

Instead of collecting into `DList`, use Rust's lazy iterator chains:

```rust
pub struct MQuery<I> {
    inner: I,
}

impl<I: Iterator> MQuery<I> {
    pub fn satisfying<P>(self, predicate: P) -> MQuery<impl Iterator<Item = I::Item>>
    where
        P: Fn(&I::Item) -> bool,
    {
        MQuery { inner: self.inner.filter(predicate) }
    }

    pub fn limit(self, n: usize) -> MQuery<impl Iterator<Item = I::Item>> {
        MQuery { inner: self.inner.take(n) }
    }
}
```

**Advantages:**
- Zero-cost abstractions
- Lazy evaluation by default
- Composable without allocation

#### 2. Vec-Based Approach (Simpler)

For simpler semantics at the cost of eagerness:

```rust
#[derive(Clone)]
pub struct MQuery<T>(Vec<T>);

impl<T> MQuery<T> {
    pub fn bind<U, F>(self, f: F) -> MQuery<U>
    where
        F: Fn(T) -> MQuery<U>,
    {
        MQuery(self.0.into_iter().flat_map(|x| f(x).0).collect())
    }

    pub fn limit(mut self, n: usize) -> Self {
        self.0.truncate(n);
        self
    }
}
```

#### 3. Entry Type

Direct translation:

```rust
pub struct Entry<K, V> {
    pub key: K,
    pub value: V,
}

impl<K: Display, V: Display> Display for Entry<K, V> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.key, self.value)
    }
}
```

#### 4. ToBool Trait

```rust
pub trait ToBool {
    fn to_bool(&self) -> bool;
}

impl ToBool for bool {
    fn to_bool(&self) -> bool { *self }
}

impl<T> ToBool for Vec<T> {
    fn to_bool(&self) -> bool { !self.is_empty() }
}

impl<T> ToBool for &[T] {
    fn to_bool(&self) -> bool { !self.is_empty() }
}
```

#### 5. AtLeastSize Trait

```rust
pub trait AtLeastSize {
    fn at_least_size(&self, n: usize) -> bool;
}

impl<T> AtLeastSize for &[T] {
    fn at_least_size(&self, n: usize) -> bool {
        self.len() >= n
    }
}

// For iterators (lazy check)
impl<I: Iterator> AtLeastSize for I {
    fn at_least_size(mut self, n: usize) -> bool {
        self.nth(n.saturating_sub(1)).is_some()
    }
}
```

#### 6. Terminal Styling

Use the `colored` or `owo-colors` crate:

```rust
use colored::*;

fn format_row<T: Display>(value: &T) -> String {
    format!("{} {}", "==>".bold().yellow(), value)
}
```

Or for more control, use `termcolor` or build custom ANSI escape handling.

### Challenges and Considerations

#### 1. Monadic Chaining

Rust lacks `do`-notation. Options:
- Method chaining (`.and_then()`, `.flat_map()`)
- Macros for pseudo-do-notation
- The `try` blocks (nightly feature)

```rust
// Haskell: do { x <- q; y <- f x; return y }
// Rust:
q.into_iter()
    .flat_map(|x| f(x))
    .collect()
```

#### 2. Lens Integration

Rust doesn't have built-in lenses, but several options exist:
- Field access via closures: `|x| &x.field`
- The `lens-rs` crate
- Derive macros for getters

#### 3. Lazy vs Eager Evaluation

Haskell's laziness is pervasive. In Rust:
- Use iterators for lazy sequences
- Be explicit about when to collect
- Consider `once_cell` or `Lazy` for lazy values

#### 4. Alternative/MonadPlus

For representing failure/choice:
- `Option<T>` for single optional result
- `Vec<T>` for multiple results
- Custom `MQuery<T>` that can be empty

### Recommended Rust Architecture

```rust
// Core types
pub struct MQuery<T>(Vec<T>);

pub struct Entry<K, V> {
    pub key: K,
    pub value: V,
}

// Traits
pub trait ToBool { fn to_bool(&self) -> bool; }
pub trait AtLeastSize { fn at_least_size(&self, n: usize) -> bool; }

// Query builder pattern
impl<T> MQuery<T> {
    pub fn new(items: Vec<T>) -> Self { MQuery(items) }
    pub fn singleton(item: T) -> Self { MQuery(vec![item]) }
    pub fn empty() -> Self { MQuery(Vec::new()) }

    pub fn bind<U, F: Fn(T) -> MQuery<U>>(self, f: F) -> MQuery<U> { ... }
    pub fn satisfying<P: Fn(&T) -> bool>(self, p: P) -> Self { ... }
    pub fn limit(self, n: usize) -> Self { ... }
    pub fn skip(self, n: usize) -> Self { ... }
    pub fn page(self, size: usize, page: usize) -> Self { ... }
    pub fn count(self) -> MQuery<usize> { ... }
}

// Entry operations
impl<K, V> MQuery<Entry<K, V>> {
    pub fn keys(self) -> MQuery<K> { ... }
    pub fn values(self) -> MQuery<V> { ... }
}

// Pretty printing
impl<T: Display> Display for MQuery<T> { ... }
```

### Dependencies for Rust Port

| Purpose          | Recommended Crate                       |
|------------------|-----------------------------------------|
| Terminal colors  | `colored`, `owo-colors`, or `termcolor` |
| Pretty printing  | `pretty` or custom                      |
| JSON integration | `serde_json`                            |
| Property testing | `proptest` or `quickcheck`              |

---

## Usage Examples

### From README - JSON Querying

```haskell
import HaskellWorks.Data.MQuery
import qualified Data.DList as DL

-- Load JSON and create query
let q = MQuery (DL.singleton json)

-- Get first 10 items
putPretty $ q >>= item & limit 10

-- Paginate: 10 items per page, page 1
putPretty $ q >>= item & page 10 1

-- Filter by key-value pair
putPretty $ q >>= item >>= hasKV "founded_year" (JsonPartialNumber 2005) & limit 10

-- Extract all entries
putPretty $ q >>= item >>= entry

-- Extract values for "name" key
putPretty $ q >>= item >>= entry >>= named "name" & limit 10

-- Get unique sorted keys
putPretty $ q >>= item >>= entry >>= key & limit 100 & onList (uniq . sort)
```

### From Tests - Lens Integration

```haskell
import HaskellWorks.Data.MQuery
import qualified Data.DList as DL

-- Create query from list of Mount records
let q = MQuery $ DL.fromList exampleMounts

-- Extract boolean field from each
q >>^. L.readOnly  -- [True, False, False, False]

-- Extract list field from each
q >>^. L.options   -- [[], ["nosuid", "noauto"], [], ["noexec"]]

-- Flatten all options into single result set
q >>^.. L.options  -- ["nosuid", "noauto", "noexec"]
```

---

## Summary

hw-mquery is a well-designed monadic query DSL that demonstrates:

1. **Elegant Haskell idioms**: Leverages typeclasses, newtypes, and monadic composition
2. **Performance considerations**: DList for efficient concatenation
3. **Practical design**: Integrates with lens ecosystem and pretty-printing
4. **REPL-friendly**: Colorized output for interactive exploration

The library is highly suitable for Rust porting, with the main adaptation being the shift from lazy evaluation to explicit iterator chains. The core query operations translate directly, and Rust's trait system can replicate the typeclass-based polymorphism.
