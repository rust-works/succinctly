# hw-aeson Analysis

## Project Overview

**Repository**: [https://github.com/haskell-works/hw-aeson](https://github.com/haskell-works/hw-aeson)

**Author**: John Ky

**License**: BSD-3-Clause

**Version**: 0.1.8.0

**Purpose**: hw-aeson is a Haskell library providing convenience functions and utilities for working with the popular [aeson](https://hackage.haskell.org/package/aeson) JSON library. It serves two primary functions:

1. **Convenience utilities** for JSON serialization/deserialization with cleaner syntax
2. **Compatibility layer** for different aeson versions (particularly the breaking changes in aeson 2.0)

The library is tested across multiple GHC versions (8.6.5 through 9.4.4) and aeson versions (1.5.x through 2.1.x), demonstrating its role as a compatibility shim.

---

## Code Structure

```
hw-aeson/
├── src/
│   └── HaskellWorks/
│       └── Data/
│           ├── Aeson.hs                    # Main module - convenience functions
│           └── Aeson/
│               ├── Compat.hs               # Key type compatibility
│               └── Compat/
│                   ├── Map.hs              # Version-switching module
│                   ├── Map/
│                   │   ├── V1.hs           # HashMap-based KeyMap (aeson < 2.0)
│                   │   └── V2.hs           # KeyMap-based KeyMap (aeson >= 2.0)
├── test/
│   ├── Spec.hs                             # Test discovery
│   └── HaskellWorks/
│       └── Data/
│           └── AesonSpec.hs                # Unit tests
├── doctest/
│   └── DoctestDriver.hs                    # Doctest runner
└── hw-aeson.cabal                          # Package configuration
```

### Module Dependency Graph

```
HaskellWorks.Data.Aeson
    └── HaskellWorks.Data.Aeson.Compat
            └── HaskellWorks.Data.Aeson.Compat.Map
                    ├── V1 (aeson < 2.0: HashMap Text v)
                    └── V2 (aeson >= 2.0: KeyMap v)
```

---

## Key Modules Analysis

### 1. HaskellWorks.Data.Aeson (Main Module)

**File**: `/src/HaskellWorks/Data/Aeson.hs`

This is the primary public interface providing convenience functions for JSON serialization.

#### Key Types and Functions

**JsonEndo** - A newtype wrapper for building lists of key-value pairs:
```haskell
newtype JsonEndo a = JsonEndo
  { unJsonEndo :: [a] -> [a]
  }
  deriving (Semigroup, Monoid) via (Endo [a])
```

This uses the endomorphism pattern (difference lists) for efficient list concatenation.

**(.?=)** - Optional field operator:
```haskell
(.?=) :: (KeyValue p, ToJSON v, Monoid p) => Key -> Maybe v -> p
```
Renders `Nothing` values as missing fields rather than `null` in JSON output.

**(.!=)** - Lower precedence assignment operator:
```haskell
(.!=) :: (KeyValue kv, ToJSON v) => Key -> v -> kv
```
Same as `.=` but with lower precedence for better integration with lens operations.

**objectWithoutNulls** - Filters null values from JSON objects:
```haskell
objectWithoutNulls :: [Pair] -> Value
objectWithoutNulls = object . Prelude.filter (not . isNull . snd)
```

**ToJsonKeyValues** - Type class for defining JSON serialization via key-value pairs:
```haskell
class ToJsonKeyValues a where
  toJsonKeyValues :: (KeyValue kv, Monoid kv) => a -> [kv]
```

**WithJsonKeyValues** - Newtype for deriving ToJSON via DerivingVia:
```haskell
newtype WithJsonKeyValues a = WithJsonKeyValues { unWithJsonKeyValues :: a }

instance ToJsonKeyValues a => ToJSON (WithJsonKeyValues a) where
  toJSON = objectEndo . toJsonKeyValues . unWithJsonKeyValues
  toEncoding = pairs . mconcat . toJsonKeyValues . unWithJsonKeyValues
```

### 2. HaskellWorks.Data.Aeson.Compat

**File**: `/src/HaskellWorks/Data/Aeson/Compat.hs`

Provides compatibility for the `Key` type which changed in aeson 2.0:

```haskell
-- aeson >= 2.0.0
type Key = J.Key

-- aeson < 2.0.0
type Key = Text
```

Key conversion functions:
- `textToKey :: Text -> Key`
- `keyToText :: Key -> Text`
- `stringToKey :: String -> Key`
- `keyToString :: Key -> String`
- `shortTextToKey :: ShortText -> Key`
- `keyToShortText :: Key -> ShortText`

### 3. HaskellWorks.Data.Aeson.Compat.Map

**File**: `/src/HaskellWorks/Data/Aeson/Compat/Map.hs`

Version-switching module using CPP:

```haskell
#if MIN_VERSION_aeson(2,0,0)
import HaskellWorks.Data.Aeson.Compat.Map.V2 as JM
#else
import HaskellWorks.Data.Aeson.Compat.Map.V1 as JM
#endif
```

### 4. Map.V1 and Map.V2

These modules provide a unified interface for JSON object key-value maps.

**V1** (aeson < 2.0): Uses `HashMap Text v` from `unordered-containers`

**V2** (aeson >= 2.0): Uses aeson's native `KeyMap v`

Both export the same 40+ functions:
- Basic operations: `null`, `lookup`, `size`, `member`, `empty`, `singleton`, `insert`, `delete`
- Set operations: `union`, `unionWith`, `intersection`, `difference`
- Folds: `foldr`, `foldl`, `foldrWithKey`, `foldlWithKey`, `foldMapWithKey`
- Mapping: `map`, `mapWithKey`, `traverseWithKey`
- Filtering: `filter`, `filterWithKey`, `mapMaybe`, `mapMaybeWithKey`
- Conversion: `toList`, `fromList`, `toMap`, `fromMap`, `toHashMap`, `fromHashMap`

---

## Haskell Language Features Leveraged

### 1. DerivingVia (GHC Extension)

```haskell
{-# LANGUAGE DerivingVia #-}

newtype JsonEndo a = JsonEndo { unJsonEndo :: [a] -> [a] }
  deriving (Semigroup, Monoid) via (Endo [a])
```

Allows deriving instances by specifying a type with compatible structure. Here, `JsonEndo` derives `Semigroup` and `Monoid` by using the `Endo [a]` representation.

### 2. CPP Preprocessor (Conditional Compilation)

```haskell
{-# LANGUAGE CPP #-}

#if MIN_VERSION_aeson(2,0,0)
type Key = J.Key
#else
type Key = Text
#endif
```

Used extensively for version compatibility across different aeson releases.

### 3. Type Classes and Instances

Custom type classes like `ToJsonKeyValues` and instance derivation via newtypes.

### 4. Operator Definitions with Custom Fixity

```haskell
infixr 7 .?=
infixr 7 .!=
```

Custom operators with specified associativity and precedence.

### 5. Newtype Pattern

Used for zero-cost abstractions:
- `JsonEndo` for difference list encoding
- `WithJsonKeyValues` for derivation mechanism

### 6. Higher-Kinded Types

```haskell
alterF :: Functor f => (Maybe v -> f (Maybe v)) -> Key -> KeyMap v -> f (KeyMap v)
traverseWithKey :: Applicative f => (Key -> v1 -> f v2) -> KeyMap v1 -> f (KeyMap v2)
```

Functions parameterized over functors and applicatives.

### 7. Type Applications (TypeApplications Extension)

```haskell
let _ = JM.lookup @Int
```

Used in tests for explicit type instantiation.

### 8. OverloadedStrings

Allows string literals to represent `Text`, `Key`, or other `IsString` types.

---

## Computer Science Techniques

### 1. Endomorphism / Difference Lists

The `JsonEndo` type uses the endomorphism pattern for O(1) list concatenation:

```haskell
newtype JsonEndo a = JsonEndo { unJsonEndo :: [a] -> [a] }
```

Instead of building a list `[a, b, c]`, it builds a function composition `(a:) . (b:) . (c:)`. When applied to `[]`, it produces the list efficiently. This avoids the O(n) cost of left-associated list concatenation.

**Monoid instance**:
- `mempty = id`
- `mappend = (.)`

### 2. Adapter Pattern

The Compat modules implement the Adapter design pattern, providing a unified interface over two different underlying implementations (HashMap vs KeyMap).

### 3. Versioned API Abstraction

The V1/V2 module structure provides version-specific implementations behind a stable API, allowing client code to work across breaking library changes.

### 4. Typeclass-Based Polymorphism

The `KeyValue` typeclass allows functions to work polymorphically over both `Pair` (for building JSON objects) and `Series` (for building encoded JSON).

### 5. Phantom Type Elimination via Newtypes

`WithJsonKeyValues` wraps values purely for type-level dispatch without runtime overhead.

---

## Dependencies

| Package              | Version Range   | Purpose                 |
|----------------------|-----------------|-------------------------|
| base                 | >= 4.11, < 5    | Standard library        |
| aeson                | >= 1.4, < 2.2   | JSON library            |
| bytestring           | >= 0.10, < 0.12 | Binary data             |
| containers           | >= 0.6, < 0.7   | Map data structure      |
| hashable             | >= 1.3, < 1.5   | Hashing                 |
| text                 | >= 1.2, < 3     | Text handling           |
| text-short           | >= 0.1.3, < 0.2 | Short text optimization |
| unordered-containers | >= 0.2, < 0.3   | HashMap                 |

---

## Rust Porting Considerations

### Feasibility: HIGH

This library is well-suited for porting to Rust. The concepts translate naturally:

### Feature Mapping

| Haskell Feature                     | Rust Equivalent                                                   |
|-------------------------------------|-------------------------------------------------------------------|
| `newtype JsonEndo a`                | `struct JsonEndo<A>(Vec<A>)` or use `Vec::with_capacity` + `push` |
| `DerivingVia`                       | Trait implementations or procedural macros                        |
| Type classes (`ToJSON`, `KeyValue`) | Traits (`Serialize`, custom traits)                               |
| CPP conditionals                    | Cargo features with `#[cfg(feature = "...")]`                     |
| `Maybe v`                           | `Option<T>`                                                       |
| `HashMap Text v`                    | `HashMap<String, V>` or `serde_json::Map`                         |
| Higher-kinded types                 | Trait objects or GATs (limited)                                   |

### Rust Implementation Approach

#### 1. Core Types

```rust
use serde::{Serialize, Serializer};
use serde_json::{Map, Value};

// Optional field helper
pub fn to_value_maybe<T: Serialize>(value: &Option<T>) -> Option<Value> {
    value.as_ref().map(|v| serde_json::to_value(v).unwrap())
}

// Object without nulls
pub fn object_without_nulls(pairs: Vec<(&str, Value)>) -> Value {
    let map: Map<String, Value> = pairs
        .into_iter()
        .filter(|(_, v)| !v.is_null())
        .map(|(k, v)| (k.to_string(), v))
        .collect();
    Value::Object(map)
}
```

#### 2. Builder Pattern (Replaces JsonEndo)

```rust
pub struct JsonObjectBuilder {
    pairs: Vec<(String, Value)>,
}

impl JsonObjectBuilder {
    pub fn new() -> Self {
        Self { pairs: Vec::new() }
    }

    pub fn field<T: Serialize>(mut self, key: &str, value: T) -> Self {
        self.pairs.push((key.to_string(), serde_json::to_value(value).unwrap()));
        self
    }

    pub fn optional_field<T: Serialize>(mut self, key: &str, value: Option<T>) -> Self {
        if let Some(v) = value {
            self.pairs.push((key.to_string(), serde_json::to_value(v).unwrap()));
        }
        self
    }

    pub fn build(self) -> Value {
        Value::Object(self.pairs.into_iter().collect())
    }
}
```

#### 3. Derive Macro Approach

```rust
use serde::Serialize;

// With serde's skip_serializing_if
#[derive(Serialize)]
struct MyType {
    mandatory: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    optional: Option<i32>,
}
```

### Key Differences

1. **No Endomorphism Needed**: Rust's `Vec::push` is amortized O(1), so the difference list technique is unnecessary.

2. **Feature Flags vs CPP**: Cargo features provide cleaner conditional compilation:
   ```toml
   [features]
   aeson-compat-v1 = []
   aeson-compat-v2 = []
   ```

3. **Serde Integration**: serde already provides most of hw-aeson's functionality:
   - `#[serde(skip_serializing_if = "Option::is_none")]` replaces `.?=`
   - `#[serde(flatten)]` for nested serialization
   - Custom serializers for complex cases

4. **No Orphan Instances**: Rust's coherence rules prevent orphan implementations, requiring wrapper types or trait extension patterns.

### Recommended Rust Crate Structure

```rust
// lib.rs
pub mod compat;  // If cross-version compatibility is needed

pub use serde_json::{Map, Value};

/// Extension trait for JSON object building
pub trait JsonExt {
    fn without_nulls(self) -> Self;
}

impl JsonExt for Value {
    fn without_nulls(self) -> Self {
        match self {
            Value::Object(map) => {
                Value::Object(
                    map.into_iter()
                        .filter(|(_, v)| !v.is_null())
                        .collect()
                )
            }
            other => other,
        }
    }
}
```

### Potential Rust Crate: `serde-json-ext`

A Rust port could provide:
1. `JsonObjectBuilder` - fluent API for building JSON objects
2. `without_nulls()` extension method
3. Optional field helpers compatible with serde
4. Type-safe key helpers if working with typed JSON schemas

### Challenges

1. **Type-Level Programming**: Haskell's type classes are more expressive than Rust traits in some ways. The `DerivingVia` pattern would require procedural macros.

2. **Higher-Kinded Types**: Functions like `traverseWithKey` with `Applicative f` constraints don't translate directly. Use async/iterators or specific types.

3. **Minimal Value**: Much of hw-aeson's functionality exists in serde already. A Rust port would primarily add convenience methods rather than new capabilities.

---

## Conclusion

hw-aeson is a focused utility library that:
1. Provides ergonomic shortcuts for common JSON patterns
2. Smooths over aeson version differences

For Rust, most functionality is already covered by serde's attribute system. A port would be straightforward but should focus on gaps in serde's ergonomics rather than direct translation. The endomorphism pattern is unnecessary in Rust, and the compatibility layer would only matter if porting code between different serde_json major versions.

**Porting Priority**: LOW to MEDIUM - The concepts are simple and serde provides native solutions for most use cases. Only port if specific convenience methods are frequently needed across a Rust codebase.
