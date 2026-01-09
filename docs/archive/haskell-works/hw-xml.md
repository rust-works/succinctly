# hw-xml: Succinct XML Parser Analysis

## Project Overview

**Repository:** [haskell-works/hw-xml](https://github.com/haskell-works/hw-xml)
**Version:** 0.5.1.2
**License:** BSD-3-Clause
**Authors:** John Ky, Alexey Raga

### Purpose

`hw-xml` is a high-performance XML parsing library that leverages **succinct data structures** to enable traversal of large XML documents with minimal memory overhead. Unlike traditional XML parsers that build a full DOM tree in memory, hw-xml creates compact bit-vector indices that allow random access navigation of the XML structure while keeping the original document as the source of truth.

### Key Academic References

The implementation is based on two key papers:
1. **"Semi-Indexing Semi-Structured Data in Tiny Space"** (Ottaviano et al.) - The core technique for indexing XML/JSON with succinct structures
2. **"Space-Efficient, High-Performance Rank & Select Structures on Uncompressed Bit Sequences"** (Zhou et al.) - Efficient rank/select implementations

---

## Code Structure

```
hw-xml/
├── src/HaskellWorks/Data/Xml/
│   ├── Blank.hs                    # XML blanking (simplified version)
│   ├── CharLike.hs                 # Character classification typeclass
│   ├── Decode.hs                   # High-level decoding combinators
│   ├── DecodeError.hs              # Error types
│   ├── DecodeResult.hs             # Result monad for decoding
│   ├── Grammar.hs                  # XML grammar parsing (attoparsec)
│   ├── Index.hs                    # Re-exports
│   ├── Lens.hs                     # Lens utilities for Value type
│   ├── RawDecode.hs                # Raw value to typed value conversion
│   ├── RawValue.hs                 # Low-level parsed XML representation
│   ├── Token.hs                    # Token re-exports
│   ├── Token/
│   │   ├── Tokenize.hs             # Token parsing
│   │   └── Types.hs                # Token type definitions
│   ├── Type.hs                     # XML type definitions
│   ├── Value.hs                    # High-level XML value ADT
│   ├── Internal/
│   │   ├── BalancedParens.hs       # BP bit generation from blanked XML
│   │   ├── Blank.hs                # Core XML blanking (streaming)
│   │   ├── ByteString.hs           # ByteString utilities
│   │   ├── List.hs                 # Interest bit generation
│   │   ├── Show.hs                 # Show utilities
│   │   ├── Tables.hs               # Lookup tables for interesting chars
│   │   ├── ToIbBp64.hs             # Combined IB+BP generation
│   │   └── Words.hs                # Word8 character predicates
│   └── Succinct/
│       ├── Cursor.hs               # Cursor re-exports
│       ├── Index.hs                # XmlIndex type and navigation
│       └── Cursor/
│           ├── BalancedParens.hs   # BP construction from blanked XML
│           ├── BlankedXml.hs       # Blanked XML wrapper type
│           ├── Create.hs           # Cursor construction functions
│           ├── InterestBits.hs     # IB construction from blanked XML
│           ├── Internal.hs         # XmlCursor type definition
│           ├── Load.hs             # File loading utilities
│           ├── MMap.hs             # Memory-mapped file loading
│           ├── Token.hs            # Token extraction at cursor
│           └── Types.hs            # SlowCursor/FastCursor type aliases
├── app/                            # CLI application
│   ├── Main.hs
│   └── App/
│       ├── Commands/
│       │   ├── Count.hs
│       │   ├── CreateBlankedXml.hs
│       │   ├── CreateBpIndex.hs
│       │   ├── CreateIbIndex.hs
│       │   ├── CreateIndex.hs
│       │   └── Demo.hs             # Example usage
│       └── XPath/                  # XPath parsing support
├── test/                           # Test suite
└── bench/                          # Criterion benchmarks
```

---

## Key Modules and Their Purpose

### 1. Core Succinct Data Structure: `XmlCursor`

**File:** `/src/HaskellWorks/Data/Xml/Succinct/Cursor/Internal.hs`

```haskell
data XmlCursor t v w = XmlCursor
  { cursorText     :: !t    -- Original XML text (ByteString)
  , interests      :: !v    -- Interest bits (marks structural positions)
  , balancedParens :: !w    -- Balanced parentheses (encodes tree structure)
  , cursorRank     :: !Count -- Current position in BP sequence
  }
```

The cursor is parameterized over three types:
- `t`: The text storage (typically `ByteString`)
- `v`: Interest bits storage (e.g., `CsPoppy1` for fast rank/select)
- `w`: Balanced parentheses storage (e.g., `RangeMin2 CsPoppy1` for fast navigation)

**Two cursor variants are provided:**

```haskell
-- Simple version - slower but less memory
type SlowCursor = XmlCursor BS.ByteString
                    (BitShown (DVS.Vector Word64))
                    (SimpleBalancedParens (DVS.Vector Word64))

-- Optimized version - faster with additional index structures
type FastCursor = XmlCursor BS.ByteString
                    CsPoppy1
                    (RangeMin2 CsPoppy1)
```

### 2. XML Blanking: The First Pass

**File:** `/src/HaskellWorks/Data/Xml/Internal/Blank.hs`

The blanking phase transforms XML into a simplified representation where:
- Structural characters (`<`, `>`, `(`, `)`) are preserved
- Content is replaced with spaces
- Special markers are introduced:
  - `t` for text content start
  - `a` for attribute names
  - `v` for attribute values
  - `[` and `]` for meta/CDATA sections

**State Machine States:**
```haskell
data BlankState
  = InXml           -- Between elements
  | InTag           -- Inside opening tag name
  | InAttrList      -- Inside attribute list
  | InCloseTag      -- Inside closing tag
  | InClose         -- At /> or ?>
  | InBang Int      -- Inside <!...> with nesting depth
  | InString Char   -- Inside quoted string
  | InText          -- Inside text content
  | InMeta          -- Start of meta (<!...)
  | InCdataTag      -- Inside CDATA tag
  | InCdata Int     -- Inside CDATA content
  | InRem Int       -- Inside comment (<!-- -->)
  | InIdent         -- Inside identifier
```

**Example transformation:**
```
Input:  <element attr="value">text</element>
Blank:  <       (a    v     )>t   >
```

### 3. Interest Bits Generation

**File:** `/src/HaskellWorks/Data/Xml/Internal/List.hs`

Interest bits mark positions in the document where structural elements begin:
- `<` - Element start
- `(` - Attribute list start
- `[` - CDATA/Meta start
- `a` - Attribute name
- `v` - Attribute value
- `t` - Text content

The function `blankedXmlToInterestBits` converts blanked XML to a compressed bit vector where each bit indicates whether the corresponding position is "interesting."

### 4. Balanced Parentheses Generation

**File:** `/src/HaskellWorks/Data/Xml/Internal/BalancedParens.hs`

The balanced parentheses representation encodes the tree structure:
- `1` (open paren) for element/attribute starts
- `0` (close paren) for element/attribute ends

**Character to BP mapping:**
```haskell
balancedParensOf :: Word8 -> MiniBP
balancedParensOf c = case c of
    '<'  -> MiniT   -- Open (1)
    '>'  -> MiniF   -- Close (0)
    '['  -> MiniT   -- Open
    ']'  -> MiniF   -- Close
    '('  -> MiniT   -- Open
    ')'  -> MiniF   -- Close
    't'  -> MiniTF  -- Open then Close (10)
    'a'  -> MiniTF  -- Open then Close
    'v'  -> MiniTF  -- Open then Close
    _    -> MiniN   -- No bit
```

### 5. Tree Navigation via TreeCursor

**File:** `/src/HaskellWorks/Data/Xml/Succinct/Cursor/Internal.hs`

The `TreeCursor` typeclass provides navigation primitives:

```haskell
instance (BP.BalancedParens u, Rank1 u, Rank0 u) => TreeCursor (XmlCursor t v u) where
  firstChild  :: XmlCursor t v u -> Maybe (XmlCursor t v u)
  nextSibling :: XmlCursor t v u -> Maybe (XmlCursor t v u)
  parent      :: XmlCursor t v u -> Maybe (XmlCursor t v u)
  depth       :: XmlCursor t v u -> Maybe Count
  subtreeSize :: XmlCursor t v u -> Maybe Count
```

These operations use the balanced parentheses structure:
- `firstChild`: Find matching open paren after current position
- `nextSibling`: Skip to after matching close paren
- `parent`: Find enclosing open paren

### 6. XmlIndex: Extracting Structure at Cursor Position

**File:** `/src/HaskellWorks/Data/Xml/Succinct/Index.hs`

```haskell
data XmlIndex
  = XmlIndexDocument [XmlIndex]
  | XmlIndexElement Text [XmlIndex]
  | XmlIndexCData BS.ByteString
  | XmlIndexComment BS.ByteString
  | XmlIndexMeta Text [XmlIndex]
  | XmlIndexAttrList [XmlIndex]
  | XmlIndexValue BS.ByteString
  | XmlIndexAttrName BS.ByteString
  | XmlIndexAttrValue BS.ByteString
  | XmlIndexError Text
```

The `xmlIndexAt` function lazily extracts the XML structure at the current cursor position, recursively building children only when accessed.

### 7. Value Types and Decoding

**Files:** `/src/HaskellWorks/Data/Xml/Value.hs`, `/src/HaskellWorks/Data/Xml/Decode.hs`

High-level decoded XML representation:
```haskell
data Value
  = XmlDocument { _childNodes :: [Value] }
  | XmlText { _textValue :: Text }
  | XmlElement { _name :: Text, _attributes :: [(Text, Text)], _childNodes :: [Value] }
  | XmlCData { _cdata :: Text }
  | XmlComment { _comment :: Text }
  | XmlMeta { _name :: Text, _childNodes :: [Value] }
  | XmlError { _errorMessage :: Text }
```

Query combinators:
```haskell
(@>) :: Value -> Text -> DecodeResult Text     -- Get attribute
(/>) :: Value -> Text -> DecodeResult Value    -- Get child element
(/>>) :: Value -> Text -> DecodeResult [Value] -- Get all matching children
```

---

## Haskell Language Features Leveraged

### 1. Type Classes and Polymorphism

- **Multi-parameter type classes**: `FromBlankedXml`, `Parser t Word8`
- **Flexible instances**: Different backing stores (Word8/16/32/64 vectors)
- **Type families** (via dependencies): `hw-rankselect`, `hw-balancedparens`

### 2. Lazy Evaluation

The XML structure is built lazily - children are only parsed when accessed:
```haskell
mapValuesFrom s = L.unfoldr (fmap (getIndexAt s &&& nextSibling))
```

### 3. Bang Patterns and Strictness

Careful use of strictness annotations for performance:
```haskell
data XmlCursor t v w = XmlCursor
  { cursorText     :: !t
  , interests      :: !v
  , balancedParens :: !w
  , cursorRank     :: !Count
  }
```

### 4. GHC Pragmas

- `{-# INLINE #-}` for critical functions
- `{-# INLINABLE #-}` for polymorphic functions
- `-msse4.2` for SIMD optimization (popcount)

### 5. Lens Library Integration

Template Haskell for generating lenses:
```haskell
makeClassy ''Value
makePrisms ''Value
```

### 6. Monad Transformers

- `ResourceT` for resource management (mmap)
- `DecodeResult` as a custom monad for error handling

### 7. Attoparsec for Parsing

Incremental parsing with backtracking for XML grammar.

---

## Computer Science Techniques

### 1. Succinct Data Structures

**Definition:** Data structures that use space close to the information-theoretic minimum while supporting efficient queries.

**In hw-xml:**
- **Interest Bits (IB)**: A bitvector marking structural positions
- **Balanced Parentheses (BP)**: Encodes tree structure in 2n bits for n nodes

### 2. Rank and Select Operations

**Rank(i):** Count of 1-bits up to position i
**Select(i):** Position of the i-th 1-bit

These operations are used to:
- Map between original text positions and structural positions
- Navigate the tree structure efficiently

**Implementation:** Uses `CsPoppy1` from hw-rankselect for O(1) rank/select with ~3% overhead.

### 3. Balanced Parentheses for Tree Navigation

The BP representation enables O(1) tree operations:
- **findclose(i)**: Find matching close paren (for subtree skip)
- **enclose(i)**: Find enclosing open paren (for parent)
- **firstchild**: Position i+1 if BP[i+1]=1

**Implementation:** `RangeMin2` structure for O(1) operations.

### 4. Semi-Indexing

Rather than fully parsing the document, the library creates a lightweight index that:
- Preserves the original document
- Allows random access to any element
- Supports lazy/streaming processing

### 5. Memory Mapping

For large files, uses `mmap` to:
- Avoid loading entire file into memory
- Let the OS handle paging
- Enable processing of files larger than RAM

---

## Rust Porting Considerations

### Feasibility: HIGH

The techniques used are language-agnostic and well-suited to Rust:

### Advantages in Rust

1. **Memory safety without GC**: Zero-copy parsing with lifetimes
2. **SIMD**: Native SIMD intrinsics for popcount, rank/select
3. **Performance**: No lazy evaluation overhead for strict operations
4. **Ecosystem**: Strong byte-processing libraries (bytes, memmap2)

### Key Porting Challenges

1. **Lazy Evaluation Simulation**
   - Haskell's laziness allows natural tree construction without full materialization
   - Rust needs explicit iterators/lazy types (e.g., `impl Iterator`)

2. **Type Class Translation**
   - Haskell: `TreeCursor`, `FromBlankedXml`, `RawDecode`
   - Rust: Traits with associated types

3. **Streaming/Chunked Processing**
   - The Haskell version handles chunks naturally via lazy lists
   - Rust needs explicit streaming (async streams, iterators)

4. **Polymorphic Index Types**
   - Haskell easily parameterizes over Word8/16/32/64
   - Rust: Use generics with trait bounds or type aliases

### Rust Architecture Recommendations

```rust
/// Core cursor type
pub struct XmlCursor<'a, IB, BP> {
    text: &'a [u8],
    interests: IB,
    balanced_parens: BP,
    rank: u64,
}

/// Interest bits trait
pub trait InterestBits {
    fn rank1(&self, pos: u64) -> u64;
    fn select1(&self, idx: u64) -> u64;
}

/// Balanced parentheses trait
pub trait BalancedParens {
    fn first_child(&self, pos: u64) -> Option<u64>;
    fn next_sibling(&self, pos: u64) -> Option<u64>;
    fn parent(&self, pos: u64) -> Option<u64>;
    fn findclose(&self, pos: u64) -> Option<u64>;
}

/// Tree cursor navigation
pub trait TreeCursor {
    fn first_child(&self) -> Option<Self> where Self: Sized;
    fn next_sibling(&self) -> Option<Self> where Self: Sized;
    fn parent(&self) -> Option<Self> where Self: Sized;
}
```

### Existing Rust Libraries to Leverage

1. **succinct** crate: Basic succinct structures (limited)
2. **bio** crate: Bioinformatics has rank/select implementations
3. **vers-vecs**: Succinct bitvectors
4. **rsdict**: Rank/select dictionary
5. **fid**: Fully indexable dictionary

### Comparison to Existing Rust XML Libraries

| Library             | Approach  | Memory | Random Access | Streaming |
|---------------------|-----------|--------|---------------|-----------|
| quick-xml           | SAX-style | Low    | No            | Yes       |
| roxmltree           | DOM       | High   | Yes           | No        |
| xmltree             | DOM       | High   | Yes           | No        |
| **hw-xml (ported)** | Succinct  | Low    | Yes           | Yes       |

### Recommended Implementation Order

1. **Phase 1: Core Bit Operations**
   - Implement or wrap rank/select bitvector
   - Implement balanced parentheses with RangeMin

2. **Phase 2: XML Blanking**
   - Port the state machine (straightforward)
   - Implement streaming chunk processing

3. **Phase 3: Index Generation**
   - Interest bits from blanked XML
   - Balanced parentheses from blanked XML

4. **Phase 4: Cursor and Navigation**
   - XmlCursor type
   - TreeCursor trait implementation

5. **Phase 5: Value Extraction**
   - XmlIndex enum
   - Value extraction at cursor position

6. **Phase 6: High-Level API**
   - Query combinators
   - Ergonomic builder patterns

### Performance Expectations

Based on the Haskell benchmarks and Rust's performance characteristics:
- **Index creation**: ~100-200 MB/s (similar to Haskell with SIMD)
- **Navigation**: O(1) for all tree operations
- **Memory**: ~6-10% of document size for indices (vs. 5-10x for DOM)

---

## Example Usage (from Demo.hs)

```haskell
-- Load XML with fast cursor
cursor <- loadFastCursor "data/catalog.xml"

-- Skip XML declaration to root element
case nextSibling cursor of
  Just rootCursor -> do
    -- Extract structure at cursor
    let rootValue = rawValueAt (xmlIndexAt rootCursor)

    -- Decode to typed value
    case decodeCatalog (rawDecode rootValue) of
      DecodeOk catalog -> print catalog
      DecodeFailed msg -> print msg
```

The key insight is that `xmlIndexAt` lazily builds the structure - only the parts you access are actually parsed from the original document.

---

## Conclusion

hw-xml demonstrates a sophisticated application of succinct data structures to XML parsing. The approach offers:

1. **Memory efficiency**: Indices are ~10% of document size
2. **Random access**: O(1) navigation to any node
3. **Streaming support**: Process documents larger than RAM
4. **Lazy extraction**: Only parse what you need

Porting to Rust is highly feasible and could result in even better performance due to:
- Zero-cost abstractions
- Native SIMD support
- No GC pauses
- Explicit memory layout control

The main engineering challenges are replicating the lazy tree construction and ensuring the streaming chunk processing works correctly across API boundaries.
