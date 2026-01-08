# hw-json: Succinct JSON Parser for Haskell

## Project Overview

**Repository:** [haskell-works/hw-json](https://github.com/haskell-works/hw-json)
**Author:** John Ky
**License:** BSD-3-Clause
**Version:** 1.3.3.0

hw-json is a memory-efficient JSON parsing library that uses **succinct data structures** to enable traversal of large JSON strings with minimal memory overhead. Unlike traditional JSON parsers that build a complete in-memory tree representation (often requiring 2-10x the original file size in memory), hw-json creates compact auxiliary indices that enable navigation and value extraction directly from the original byte buffer.

### Key Value Proposition

The library demonstrates dramatic memory improvements over traditional parsers:

| Parser          | File Size  | Memory Usage |
|-----------------|------------|--------------|
| Scala/Argonaut  | 78 MB JSON | ~2 GB        |
| Haskell/Aeson   | 78 MB JSON | ~928 MB      |
| Haskell/hw-json | 78 MB JSON | ~495 MB      |

The fundamental insight is that hw-json avoids building a traditional AST. Instead, it maintains:
1. The original JSON bytes (memory-mapped)
2. An **Interest Bits (IB)** index - marks positions of "interesting" characters (structural delimiters)
3. A **Balanced Parentheses (BP)** index - encodes the tree structure

---

## Code Structure and Key Modules

### Source Tree Organization

```
src/HaskellWorks/Data/Json/
├── DecodeError.hs          -- Simple error type wrapper
├── FromValue.hs            -- Type class for extracting values
├── Internal/
│   ├── CharLike.hs         -- Character classification predicates
│   ├── Doc.hs              -- Pretty-printing utilities
│   ├── Index.hs            -- JsonIndex extraction (strict)
│   ├── PartialIndex.hs     -- JsonPartialIndex extraction (lazy)
│   ├── Slurp.hs            -- Extracting strings and numbers
│   ├── Standard/
│   │   ├── Cursor/Token.hs -- Token extraction at cursor
│   │   └── Token/Tokenize.hs -- JSON tokenizer
│   ├── Token.hs            -- Re-exports
│   ├── Token/Types.hs      -- JsonToken ADT
│   ├── Value.hs            -- String parsing with escape handling
│   └── Word64.hs           -- 64-bit word patterns for SIMD
├── LightJson.hs            -- Lazy cursor-based JSON (GC-friendly)
├── PartialValue.hs         -- Lazy JSON values with inline errors
├── Query.hs                -- Query DSL for LightJson
├── Simple/Value.hs         -- Simple backend value types
├── Standard/Load/Partial.hs -- Loading functions with CsPoppy
└── Value.hs                -- Traditional JsonValue type
```

### Module Responsibilities

#### Core Data Types

**JsonValue** (`Value.hs`) - Traditional JSON ADT:
```haskell
data JsonValue
  = JsonString Text
  | JsonNumber Double
  | JsonObject [(Text, JsonValue)]
  | JsonArray [JsonValue]
  | JsonBool Bool
  | JsonNull
```

**JsonPartialValue** (`PartialValue.hs`) - Lazy JSON with error nodes:
```haskell
data JsonPartialValue
  = JsonPartialString Text
  | JsonPartialNumber Double
  | JsonPartialObject [(Text, JsonPartialValue)]
  | JsonPartialArray [JsonPartialValue]
  | JsonPartialBool Bool
  | JsonPartialNull
  | JsonPartialError Text  -- Errors can appear anywhere in the tree
```

**LightJson** (`LightJson.hs`) - Cursor-based lazy JSON:
```haskell
data LightJson c
  = LightJsonString Text
  | LightJsonNumber BS.ByteString  -- Unparsed number bytes
  | LightJsonObject [(Text, c)]    -- Cursors, not values!
  | LightJsonArray [c]
  | LightJsonBool Bool
  | LightJsonNull
  | LightJsonError Text
```

The key insight with `LightJson` is that arrays and objects contain **cursors** (type `c`), not parsed values. This enables:
- Lazy evaluation: only parse what you access
- Better GC: dropping a cursor reference allows the subtree to be collected
- Memory efficiency: siblings are not retained when navigating

#### Index Types

**JsonIndex** (`Internal/Index.hs`) - Strict index pointing to raw bytes:
```haskell
data JsonIndex
  = JsonIndexString BS.ByteString  -- Points to raw string bytes
  | JsonIndexNumber BS.ByteString  -- Points to raw number bytes
  | JsonIndexObject [(BS.ByteString, JsonIndex)]
  | JsonIndexArray [JsonIndex]
  | JsonIndexBool Bool
  | JsonIndexNull
```

**JsonPartialIndex** (`Internal/PartialIndex.hs`) - Same but with error support for lazy parsing.

---

## Haskell Language Features Leveraged

### 1. Type Classes for Polymorphic Cursor Types

The library uses type classes to abstract over different index implementations:

```haskell
class JsonPartialValueAt a where
  jsonPartialJsonValueAt :: a -> JsonPartialValue

class JsonPartialIndexAt a where
  jsonPartialIndexAt :: a -> JsonPartialIndex

class LightJsonAt a where
  lightJsonAt :: a -> LightJson a
```

This allows the same algorithms to work with different cursor implementations (simple vs. standard, slow vs. fast).

### 2. Flexible Instances with Multi-Parameter Type Classes

```haskell
instance (BP.BalancedParens w, Rank0 w, Rank1 w, Select1 v, TestBit w)
         => JsonPartialIndexAt (GenericCursor BS.ByteString v w) where
  jsonPartialIndexAt k = ...
```

The constraint `(BP.BalancedParens w, Rank0 w, Rank1 w, Select1 v, TestBit w)` expresses that the cursor works with any types that support the required rank/select operations.

### 3. Lazy Evaluation for Streaming

Haskell's lazy evaluation is crucial for the library's efficiency. The `JsonPartialValue` type can be partially evaluated:

```haskell
-- Only first 10 items are evaluated
take 10 (arrayItems partialValue)
```

### 4. Bang Patterns for Strict Evaluation Points

```haskell
let !ibip = JCF.simdToIbBp jsonBs  -- Force index construction
let !c    = JCF.fromBsIbBp jsonBs ibip  -- Force cursor creation
```

Strategic strictness ensures indices are fully computed before traversal.

### 5. Monadic Navigation with Kleisli Composition

```haskell
let fc = TC.firstChild
let ns = TC.nextSibling

-- Navigate to grandchild's sibling
(fc >=> fc >=> ns) cursor
```

The `>=>` (Kleisli composition) operator chains `Maybe`-returning navigation operations elegantly.

### 6. MQuery Monad for Query DSL

```haskell
newtype MQuery a = MQuery { mQuery :: DList a }

-- Query example
q >>= item >>= entry >>= named "acquisition" >>= entry >>= named "price_amount"
```

Uses difference lists (`DList`) for efficient list concatenation during query execution.

---

## Computer Science Techniques

### 1. Semi-Indexing (Primary Technique)

The library implements the technique from:
> "Semi-Indexing Semi-Structured Data in Tiny Space" (CIKM 2010)

**Key Insight:** Instead of building a full index of all positions, create a minimal index that:
1. Marks "interesting" positions (structural delimiters: `{`, `}`, `[`, `]`, `:`, `,`)
2. Encodes tree structure as balanced parentheses

This reduces index size from O(n) pointers to O(n) bits.

### 2. Balanced Parentheses Encoding

Tree structure is encoded as a sequence of balanced parentheses:

```
JSON: {"a": [1, 2]}
Tree:  { "a" [ 1   2 ] }
BP:    ( (   ( (   ( ) ) )

Where:
- ( = open (entering a node)
- ) = close (leaving a node)
```

The balanced parentheses allow O(1) navigation operations using:
- **findClose**: Find matching closing paren (skip subtree)
- **findOpen**: Find matching opening paren (go to parent)
- **enclose**: Find enclosing parens (parent node)

### 3. Rank and Select Operations

These are the fundamental succinct data structure operations:

**Rank1(i)**: Count 1-bits in positions [0, i]
**Select1(j)**: Find position of j-th 1-bit

Used to:
- Map between "interesting" positions and cursor positions
- Navigate the balanced parentheses structure

The library supports multiple implementations:
- **BitShown**: Simple but slow (useful for debugging)
- **CsPoppy**: Cache-sensitive popcount index (fast)
- **RangeMin**: Range-minimum query support for findClose/findOpen

### 4. CsPoppy Index Structure

CsPoppy (Cache-Sensitive Poppy) provides O(1) rank and select with:
- L0 blocks: 2^16 bits (8KB), stores cumulative popcount
- L1 blocks: 512 bits, stores offset within L0
- L2 blocks: 64 bits, use hardware popcount

Memory overhead: ~3.125% of original bitvector.

### 5. Interest Bits (IB) Index

Marks positions of structural characters:
```
JSON: {"a": 1}
IB:   1000110
      ^  ^ ^^
      {  : 1}
```

Generated by scanning the JSON and toggling a "in string" flag to ignore quoted content.

### 6. SIMD Acceleration

On x86_64, the library uses SIMD instructions for:
- Parallel character classification (finding `"`, `\`, `{`, etc.)
- Bulk popcount operations
- String escape handling

Dependencies on x86_64:
- `hw-json-simd`: SIMD-accelerated index construction
- `hw-simd`: Low-level SIMD primitives

### 7. Memory Mapping

```haskell
!jsonBs <- BS.mmap "corpus/bench/hospitalisation.json"
```

Files are memory-mapped, avoiding copies:
- The kernel manages page faults
- Only accessed pages are loaded
- Multiple processes can share read-only mappings

---

## Related Libraries (hw-* Ecosystem)

hw-json depends on several related libraries:

| Library                   | Purpose                                     |
|---------------------------|---------------------------------------------|
| `hw-prim`                 | Primitive operations (Drop, Position, etc.) |
| `hw-bits`                 | Bit manipulation utilities                  |
| `hw-rankselect`           | Rank/Select implementations (CsPoppy)       |
| `hw-rankselect-base`      | Rank/Select type classes                    |
| `hw-balancedparens`       | Balanced parentheses data structures        |
| `hw-json-simple-cursor`   | Simple backend cursor                       |
| `hw-json-standard-cursor` | Standard backend cursor                     |
| `hw-json-simd`            | SIMD-accelerated operations (x86_64)        |
| `hw-mquery`               | Query DSL                                   |
| `hw-parser`               | Parser combinators                          |

---

## Rust Porting Considerations

### Feasibility Assessment: HIGH

The core algorithms are portable. Main challenges are:

### 1. Type System Differences

**Haskell approach:**
```haskell
class (BP.BalancedParens w, Rank0 w, Rank1 w, Select1 v, TestBit w)
      => JsonPartialIndexAt (GenericCursor BS.ByteString v w)
```

**Rust equivalent:**
```rust
trait JsonPartialIndexAt {
    fn json_partial_index_at(&self) -> JsonPartialIndex;
}

impl<V, W> JsonPartialIndexAt for GenericCursor<V, W>
where
    V: Select1,
    W: BalancedParens + Rank0 + Rank1 + TestBit,
{
    // ...
}
```

Rust traits work well for this pattern.

### 2. Lazy Evaluation

Haskell's lazy evaluation is crucial for the partial value types. In Rust:

**Option 1: Iterator-based laziness**
```rust
struct JsonArray<'a> {
    cursor: Cursor<'a>,
}

impl<'a> JsonArray<'a> {
    fn iter(&self) -> impl Iterator<Item = JsonValue<'a>> + 'a {
        // Navigate siblings lazily
    }
}
```

**Option 2: Explicit thunks**
```rust
enum LazyJsonValue<'a> {
    Evaluated(Box<JsonValue>),
    Thunk(Box<dyn FnOnce() -> JsonValue + 'a>),
}
```

### 3. Memory Management

Haskell's GC handles cursor lifetime automatically. In Rust:
- Use lifetimes to tie cursors to the source buffer
- Consider `Arc<[u8]>` for shared ownership
- Memory-mapped files with `memmap2` crate

### 4. SIMD Implementation

Rust has excellent SIMD support:
- `std::arch` for platform-specific intrinsics
- `packed_simd` or `wide` crates for portable SIMD
- simd-json already demonstrates this approach

### Comparison with Existing Rust JSON Libraries

#### simd-json

simd-json also uses SIMD for JSON parsing but with different goals:
- **simd-json**: Builds a full DOM, focuses on parse speed
- **hw-json**: Minimal indexing, focuses on memory efficiency

simd-json creates a "tape" of events plus string buffer, while hw-json keeps the original bytes and tiny indices.

**Potential hybrid approach:**
Combine simd-json's SIMD indexing with hw-json's navigation strategy.

#### serde_json

Traditional recursive descent parser. Full DOM construction. Best for small-medium documents where simplicity matters.

#### simdjson (C++ via FFI)

Google's simdjson shares the semi-indexing philosophy:
- Creates "structural index" (similar to Interest Bits)
- Supports lazy/on-demand parsing
- Highly optimized for modern CPUs

### Recommended Rust Implementation Strategy

1. **Start with the data structures:**
   - Port `BitShown` for simple bitvector with rank/select
   - Port `CsPoppy` for O(1) rank/select
   - Port `SimpleBalancedParens` and `RangeMin`

2. **Implement the cursor:**
   - Generic over rank/select implementations
   - Navigation: firstChild, nextSibling, parent
   - Use lifetimes for zero-copy access to source

3. **Add value extraction:**
   - JsonIndex equivalent for strict extraction
   - Iterator-based API for lazy traversal

4. **SIMD optimization:**
   - Use simd-json's indexing as reference
   - Target AVX2 initially, with fallback
   - Consider ARM NEON for M1/M2 Macs

### Key Advantages of Rust Port

1. **No GC overhead**: Predictable performance
2. **Zero-cost abstractions**: Traits compile to static dispatch
3. **SIMD ergonomics**: Better than Haskell's FFI approach
4. **Memory safety**: Lifetimes prevent dangling cursor bugs
5. **Ecosystem**: Integration with serde, tokio, etc.

### Potential Challenges

1. **Lazy evaluation**: Requires explicit iterator design
2. **Higher-kinded types**: Some patterns need workarounds
3. **Testing**: Property-based testing less mature than QuickCheck
4. **Documentation**: Less formal documentation tradition

---

## References

1. [Semi-Indexing Semi-Structured Data in Tiny Space](http://www.di.unipi.it/~ottavian/files/semi_index_cikm.pdf) - CIKM 2010
2. [Succinct Data Structures talk by Edward Kmett](https://www.youtube.com/watch?v=uA0Z7_4J7u8)
3. [Typed Tagless Final Interpreters](http://okmij.org/ftp/tagless-final/course/lecture.pdf)
4. [simd-json: Parsing gigabytes of JSON per second](https://branchfree.org/2019/02/25/paper-parsing-gigabytes-of-json-per-second/)
5. [Rank/Select Data Structures](https://www.cs.cmu.edu/~dga/papers/zhou-sea2013.pdf)

---

## Appendix: Example Usage

### Basic Navigation

```haskell
import qualified HaskellWorks.Data.Json.Standard.Cursor.Fast as JCF
import qualified HaskellWorks.Data.TreeCursor as TC

-- Load and create cursor
jsonBs <- BS.mmap "large.json"
let ibip = JCF.simdToIbBp jsonBs
let cursor = JCF.fromBsIbBp jsonBs ibip

-- Navigate: first child's sibling
let maybeSecond = (TC.firstChild >=> TC.nextSibling) cursor
```

### Query DSL

```haskell
import HaskellWorks.Data.Json.PartialValue
import HaskellWorks.Data.MQuery

cursor <- loadPartial "data.json"
let json = jsonPartialJsonValueAt cursor
let q = MQuery (DL.singleton json)

-- Find all "price_amount" values where currency is USD
putPretty $ q >>= (item
              >=> entry
              >=> named "acquisition"
              >=> having (entry >=> named "currency" >=> valueOf "USD")
              >=> entry
              >=> named "price_amount")
```

### Creating Pre-computed Indices

```bash
# Create index files for faster subsequent loads
hw-json create-index -i large.json -b standard -m simd

# Produces:
#   large.json.ib.idx  (Interest Bits)
#   large.json.bp.idx  (Balanced Parentheses)
```
