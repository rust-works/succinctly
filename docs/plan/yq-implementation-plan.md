# yq Implementation Plan for succinctly

## Executive Summary

This plan describes implementing Mike Farah's yq query language in the succinctly crate. The implementation prioritizes:

1. **Correctness**: Byte-for-byte compatibility with yq tool output
2. **Performance**: Consume source bytes and write computed bytes directly to output where possible, avoiding temporary string allocations
3. **Testability**: Test-first approach with comprehensive conformance tests

## Key Decisions

| Decision | Choice | Rationale |
|----------|--------|-----------|
| Parser approach | Write completely new parser in `src/yq/` | yq syntax differs significantly from jq (`ireduce`, no `if-then-else`, YAML operators) |
| Output strategy | yq-compatible output, not source preservation | Output must match yq tool; consume source bytes efficiently but serialize to yq's normalized format |
| Error messages | May differ from yq | Must convey same information or better, but exact wording can differ |
| First iteration scope | Exclude `load()`, `env()`, `strenv()`, `envsubst()`, `eval()` | Security/complexity concerns; may add later |

## Current YamlCursor Capabilities

Investigation of `src/yaml/light.rs` and `src/yaml/index.rs` reveals:

| Feature | Status | Location |
|---------|--------|----------|
| Anchors | ✅ Supported | `YamlIndex.anchors: BTreeMap<String, usize>` |
| Aliases | ✅ Supported | `YamlIndex.aliases: BTreeMap<usize, usize>` |
| Alias resolution | ✅ Supported | `YamlIndex.resolve_alias()`, `get_alias_target()` |
| Anchor lookup | ✅ Supported | `YamlIndex.get_anchor_bp_pos()` |
| Text position | ✅ Supported | `YamlCursor.text_position()`, `text_end_position()` |
| Navigation | ✅ Supported | `first_child()`, `next_sibling()`, `parent()` |
| Type detection | ✅ Supported | `is_container()`, `is_sequence_at_bp()` |
| Comments | ❌ Not tracked | Would require parser changes |
| Style info | ❌ Not tracked | Would require parser changes |
| Tag info | ❌ Not tracked | Would require parser changes |

**Implication**: YAML-specific operators that require comments, style, or tag metadata will need enhancements to the YAML parser/index in a future iteration.

## Architecture Overview

### Current State

The existing `succinctly yq` implementation:
- Uses a jq-compatible parser in `src/jq/parser.rs`
- Evaluates against `YamlIndex` semi-indexed YAML
- Converts to intermediate `OwnedValue` for output

### Target State

The new implementation will:
- Have a completely new yq parser in `src/yq/parser.rs`
- Support efficient cursor-based evaluation with minimal allocations
- Produce yq-compatible output format
- Pass all YQ-* conformance tests

## Module Structure

```
src/
├── yq/
│   ├── mod.rs              # Public API
│   ├── lexer.rs            # Tokenizer
│   ├── parser.rs           # Expression parser
│   ├── ast.rs              # AST node types
│   ├── eval.rs             # Core evaluator
│   ├── functions.rs        # Built-in functions
│   ├── output.rs           # Output formatting
│   └── tests/
│       ├── mod.rs
│       ├── parser_tests.rs
│       ├── eval_tests.rs
│       └── conformance_tests.rs
```

## Phase 0: Investigate Cursor Enhancements for yq Compatibility

Before implementing the yq query language, investigate what enhancements to `YamlIndex` and `YamlCursor` are needed for full yq compatibility.

### 0.1 Required Capabilities Assessment

Evaluate each yq operator that requires metadata not currently tracked:

| Operator | Required Data | Current Status | Enhancement Needed |
|----------|---------------|----------------|-------------------|
| `line_comment` (read) | Comment text after value | ❌ Not tracked | Track comment byte ranges |
| `line_comment` (write) | Ability to set comment | ❌ N/A | Output layer only |
| `head_comment` (read) | Comment text above node | ❌ Not tracked | Track comment byte ranges |
| `foot_comment` (read) | Comment text below node | ❌ Not tracked | Track comment byte ranges |
| `style` (read) | Quote style, flow/block | ⚠️ Partial | Can detect from text position |
| `style` (write) | Output formatting control | ❌ N/A | Output layer only |
| `tag` (read) | YAML tag (!!str, etc.) | ✅ Derivable | From value type |
| `tag` (write) | Custom tag assignment | ❌ Not tracked | Track explicit tags |
| `anchor` (read) | Anchor name | ✅ Supported | Already in `YamlIndex.anchors` |
| `anchor` (write) | Set anchor name | ❌ N/A | Output layer only |

### 0.2 Parser Enhancement Options

Investigate the cost/benefit of tracking additional metadata in the YAML parser:

**Option A: Minimal (First Iteration)**
- No parser changes
- Support `tag`/`kind`/`type` (read) via value type derivation
- Support `style` (read) via text position inspection
- Support `anchor`/`alias` via existing tracking
- Defer comment operators entirely

**Option B: Comment Tracking**
- Add `comments: Vec<CommentInfo>` to `YamlIndex`
- Track byte ranges for head/line/foot comments during parsing
- Moderate parser complexity increase
- Enables full comment operator support

**Option C: Full Metadata**
- Track comments, explicit tags, and style indicators
- Significant parser complexity increase
- Enables complete yq feature parity

### 0.3 Investigation Tasks

1. **Measure comment frequency**: Sample real-world YAML files to determine how often comments appear and whether tracking them is worthwhile
2. **Prototype comment tracking**: Add minimal comment tracking to parser, measure performance impact
3. **Review yq usage patterns**: Analyze common yq queries to determine which metadata operators are frequently used
4. **Decision point**: Choose Option A, B, or C based on findings

### 0.4 Investigation Findings

#### Comment Frequency Analysis

Sampled real-world YAML files to measure comment usage:

| Source | Files | Total Lines | Comment Lines | Ratio |
|--------|-------|-------------|---------------|-------|
| succinctly CI workflows | 3 | 422 | 3 | 0.7% |
| CoreDNS K8s manifest | 1 | ~200 | 5 | 2.5% |
| nginx-ingress Helm values | 1 | ~1000 | 145 | 14.5% |
| GitLab CI configuration | 1 | ~2000 | 94 | 4.7% |
| Ansible lamp_simple playbook | 1 | ~30 | 1 | 3.3% |
| Bitnami PostgreSQL Helm values | 1 | ~5000 | 1847 | 37% |

**Key Observations**:
- CI/CD workflow files (GitHub Actions, GitLab CI): 0-5% comments
- Kubernetes manifests: 2-5% comments
- Helm chart values.yaml files: 15-40% comments (heavily documented configuration)
- Ansible playbooks: 1-5% comments

**Conclusion**: Comment frequency varies dramatically by use case. Helm values files are extremely comment-heavy, while CI workflows and K8s manifests have minimal comments.

#### yq Usage Pattern Analysis

Reviewed GitHub issues, StackOverflow questions, and yq documentation to assess metadata operator usage:

| Operator | Usage Frequency | Common Use Cases |
|----------|-----------------|------------------|
| `style` (read/write) | **High** | Format control, quote forcing, flow vs block |
| `tag` (read) | Medium | Type checking, custom tag detection |
| `tag` (write) | Low | Rarely used outside edge cases |
| `head_comment` (read/write) | Medium | Documentation extraction/injection |
| `line_comment` (read/write) | Medium | Inline annotation handling |
| `foot_comment` (read/write) | Low | Trailing comment manipulation |
| `anchor`/`alias` | Medium | Template processing, reference handling |

**Key Observations**:
- Style operators are heavily used for output formatting control
- Comment operators have significant GitHub issue/discussion activity (complex edge cases)
- The go-yaml parser underlying yq has known issues with comment attachment

#### Style Detection Feasibility

Examined existing `YamlString` enum in `src/yaml/light.rs`:

```rust
pub enum YamlString<'a> {
    DoubleQuoted { text, start },     // → style = "double"
    SingleQuoted { text, start },     // → style = "single"
    Unquoted { text, start, end, base_indent }, // → style = ""
    BlockLiteral { text, indicator_pos, chomping, explicit_indent },  // → style = "literal"
    BlockFolded { text, indicator_pos, chomping, explicit_indent },   // → style = "folded"
}
```

**Conclusion**: String style is **already fully derivable** from `YamlValue::String(YamlString::*)` variant. No parser changes needed for read access.

For container style (flow vs block), detection is possible by checking the first byte at `text_position()`:
- `{` → flow mapping
- `[` → flow sequence
- Otherwise → block style

#### Succinct Data Structure Considerations

Investigated whether additional succinct indexes could improve metadata tracking:

**Current Index Structures**:
- `ib: Vec<u64>` - Interest bits (structural positions)
- `bp: BalancedParens<W>` - Tree structure with O(1) navigation
- `ty: Vec<u64>` - Type bits (mapping vs sequence)
- `bp_to_text: Vec<u32>` - BP position to text offset
- `bp_to_text_end: Vec<u32>` - End positions for scalars
- `seq_items: Vec<u64>` - Sequence item wrapper markers
- `containers: Vec<u64>` - Container position markers

**Potential New Index Structures for Comments**:

| Structure | Purpose | Space Overhead | Implementation Complexity |
|-----------|---------|----------------|---------------------------|
| `comment_positions: Vec<u32>` | Track comment start positions | O(n) where n = comment count | Low |
| `comment_ends: Vec<u32>` | Track comment end positions | O(n) | Low |
| `node_to_comment: Vec<u32>` | Map BP nodes to their comments | O(nodes) | Medium |
| `comment_type: Vec<u2>` | head/line/foot classification | O(n) bits | Low |

**Analysis**: Comments are sparse in most YAML files (typically <5% of lines). A simple `Vec<CommentInfo>` would be more efficient than a bitvector approach, since bitvectors optimize for dense data.

**Potential Enhancement for Tags**:

| Structure | Purpose | Space Overhead |
|-----------|---------|----------------|
| `explicit_tags: BTreeMap<usize, String>` | Store explicit tags (!!str, !custom) | O(tagged nodes) |

Most nodes don't have explicit tags, so a sparse map is appropriate.

### 0.5 Recommendation

**Final Decision: Option A (Minimal) with Style Detection**

Based on the investigation findings:

1. **Style operators are fully supportable without parser changes**
   - String style derivable from `YamlString` variant
   - Container style (flow/block) derivable from first byte inspection
   - No new index structures needed

2. **Comment operators should be deferred**
   - Complex edge cases in comment attachment (known issues in go-yaml)
   - Low-frequency usage in most YAML files (except Helm values)
   - Would require new `comments` index structure
   - Recommend deferring to Phase 7+ if user demand materializes

3. **Tag operators partially supportable**
   - Basic tags (!!str, !!int, !!bool, etc.) derivable from value type
   - Custom explicit tags would require new `explicit_tags` index
   - Recommend supporting basic tag detection, deferring custom tags

4. **Anchor/alias operators already fully supported**
   - Existing `anchors` and `aliases` BTreeMaps in `YamlIndex`
   - `resolve_alias()`, `get_anchor_bp_pos()` methods available

**Summary Table**:

| Operator | Phase 1-6 Support | Notes |
|----------|-------------------|-------|
| `style` (read) | ✅ Full | Derivable from `YamlString` variant |
| `style` (write) | ✅ Full | Output layer only |
| `tag` (read, basic) | ✅ Full | Derivable from value type |
| `tag` (read, custom) | ❌ Deferred | Requires parser changes |
| `tag` (write) | ✅ Full | Output layer only |
| `anchor` (read) | ✅ Full | Already supported |
| `alias` (read) | ✅ Full | Already supported |
| `head_comment` | ❌ Deferred | Requires new index structure |
| `line_comment` | ❌ Deferred | Requires new index structure |
| `foot_comment` | ❌ Deferred | Requires new index structure |

Proceed to Phase 1 with this scoping.

---

## Appendix A: Optimized Comment/Tag Index Designs

This appendix explores design options for adding comment and tag tracking while minimizing memory usage and maximizing performance, applying lessons from the optimization documentation.

### A.1 Design Constraints

Based on Phase 0 findings:
- **Comment sparsity**: 0.7%-37% of lines contain comments (typically <5% for CI/K8s)
- **Tag sparsity**: Custom explicit tags are rare (most nodes use implicit typing)
- **Access pattern**: Random access by BP position during query evaluation
- **Memory budget**: Target <10% overhead for index structures

Key optimization lessons to apply:
- "Simpler is often faster" - Lightweight DSV index beat 3-level BitVec by 5-9x
- Sparse data → sparse structures (maps/vectors vs bitvectors)
- Cache locality matters more than asymptotic complexity
- Pack auxiliary data to reduce memory traffic

### A.2 Design Option 1: Sparse BTreeMap (Simple, Cache-Friendly)

Store comment/tag metadata only for nodes that have them.

```rust
/// Comment attached to a YAML node
pub struct CommentInfo {
    /// BP position of the node this comment belongs to
    node_bp: u32,
    /// Comment type (head/line/foot)
    kind: CommentKind,
    /// Byte range in source text [start, end)
    start: u32,
    end: u32,
}

pub enum CommentKind {
    Head = 0,  // Above node
    Line = 1,  // Same line as node
    Foot = 2,  // Below node
}

pub struct YamlIndex<W = Vec<u64>> {
    // ... existing fields ...

    /// Sparse comment tracking - only nodes with comments have entries
    /// Key: BP position, Value: indices into comments vec
    comment_map: BTreeMap<u32, SmallVec<[u16; 2]>>,
    /// All comments stored contiguously (cache-friendly iteration)
    comments: Vec<CommentInfo>,

    /// Explicit tags - only nodes with custom tags
    /// Key: BP position, Value: tag string (e.g., "!custom", "!!python/object")
    explicit_tags: BTreeMap<u32, String>,
}
```

**Memory analysis** (1MB YAML with 5% comment lines):
- ~500 comments × 12 bytes = 6 KB comment data
- ~500 BTreeMap entries × ~40 bytes = 20 KB overhead
- Total: ~26 KB = **2.6% of source size**

**Performance characteristics**:
- Lookup: O(log n) where n = nodes with comments (sparse = fast)
- Build: O(c log c) where c = comment count
- Cache: Comments stored contiguously, good for iteration

**Pros**: Simple, proven pattern (matches anchors/aliases design), minimal memory for sparse data
**Cons**: BTreeMap has pointer-chasing overhead

### A.3 Design Option 2: Interleaved Position Array (Sorted, Binary Search)

Store comment positions sorted by BP position for binary search lookup.

```rust
/// Packed comment entry (16 bytes, cache-line aligned with padding)
#[repr(C)]
pub struct PackedComment {
    node_bp: u32,      // BP position of owning node
    kind: u8,          // CommentKind as u8
    _pad: [u8; 3],     // Align to 4 bytes
    start: u32,        // Byte offset in source
    end: u32,          // End byte offset
}

pub struct YamlIndex<W = Vec<u64>> {
    // ... existing fields ...

    /// Comments sorted by node_bp for binary search
    /// 16 bytes per comment, no pointer chasing
    comments: Vec<PackedComment>,

    /// Index: For each superblock (every 64 BP positions),
    /// store offset into comments array
    /// Enables O(1) jump to approximate location, then short linear scan
    comment_superblock_idx: Vec<u32>,

    /// Explicit tags sorted by BP position
    tags: Vec<(u32, String)>,
}
```

**Lookup algorithm**:
```rust
fn get_comments(&self, bp_pos: u32) -> &[PackedComment] {
    // O(1) superblock lookup
    let superblock = bp_pos / 64;
    let start = self.comment_superblock_idx[superblock] as usize;
    let end = self.comment_superblock_idx[superblock + 1] as usize;

    // Binary search within superblock range
    let slice = &self.comments[start..end];
    let idx = slice.partition_point(|c| c.node_bp < bp_pos);

    // Return contiguous slice of matching comments
    let mut end_idx = idx;
    while end_idx < slice.len() && slice[end_idx].node_bp == bp_pos {
        end_idx += 1;
    }
    &slice[idx..end_idx]
}
```

**Memory analysis** (1MB YAML with 5% comment lines):
- ~500 comments × 16 bytes = 8 KB comment data
- ~(1MB/64) superblock entries × 4 bytes = ~64 KB index
- Total: ~72 KB = **7.2% overhead**

**Performance characteristics**:
- Lookup: O(1) superblock + O(log k) where k = comments in superblock (typically <4)
- Build: O(c) single pass during parsing
- Cache: Contiguous array, excellent prefetching

**Pros**: No pointer chasing, cache-friendly layout, O(1) approximate lookup
**Cons**: Higher fixed overhead for superblock index

### A.4 Design Option 3: Bit-Packed Compact Representation

Minimize memory by bit-packing and using position deltas.

```rust
/// Ultra-compact comment storage using delta encoding
/// Assumes comments are usually near their owning node in text order
pub struct CompactComments {
    /// Packed entries: each comment uses ~6 bytes average
    /// Format per entry:
    ///   - Delta from previous node_bp (varint, typically 1-2 bytes)
    ///   - Kind (2 bits) + length high bits (6 bits) packed in 1 byte
    ///   - Text offset delta (varint, typically 2-3 bytes)
    data: Vec<u8>,

    /// Sparse index for random access
    /// Entry i = byte offset in data for comments near BP position i*256
    sparse_idx: Vec<u32>,
}

/// Explicit tags using string interning
pub struct CompactTags {
    /// Interned tag strings (most YAML uses few unique tags)
    tag_pool: Vec<String>,
    /// (BP position, tag pool index) pairs, sorted by BP
    entries: Vec<(u32, u16)>,
}
```

**Memory analysis** (1MB YAML with 5% comment lines):
- ~500 comments × ~6 bytes average = 3 KB
- Sparse index: ~4 KB
- Total: ~7 KB = **0.7% overhead**

**Performance characteristics**:
- Lookup: O(1) sparse index + O(k) sequential decode
- Build: More complex delta encoding
- Cache: Very compact, but decode cost

**Pros**: Minimal memory footprint
**Cons**: Complex implementation, decode overhead on access

### A.5 Design Option 4: Parallel Bitvector + Position Array (For Dense Comments)

For Helm-style files with 30%+ comments, bitvector becomes efficient.

```rust
pub struct DenseComments<W = Vec<u64>> {
    /// Bitvector: 1 if BP position has any comment
    has_comment: W,
    /// Cumulative popcount for O(1) rank on has_comment
    has_comment_rank: Vec<u32>,

    /// Dense array: comments[rank(bp_pos)] gives comment info
    /// Only allocated for positions with comments
    comments: Vec<PackedComment>,
}
```

**Memory analysis** (1MB YAML with 37% comments like Helm values):
- Bitvector: ~2KB per 16K BP positions
- Rank index: ~3% of bitvector
- Comments: 37% × nodes × 16 bytes

**Performance characteristics**:
- Lookup: O(1) rank + O(1) array access
- Best for: Files with >20% comments
- Cache: Bitvector is very cache-friendly for existence check

### A.6 Recommended Design: Adaptive Sparse (Option 1 + Enhancements)

Based on the optimization lessons and typical usage patterns:

```rust
/// Optimized comment tracking for typical YAML (sparse comments)
pub struct CommentIndex {
    /// Primary storage: sorted vector of comment entries
    /// Better cache locality than BTreeMap for iteration
    entries: Vec<CommentEntry>,

    /// For files with >100 comments: sparse lookup acceleration
    /// Maps BP position ranges to entry indices
    /// Only built when entries.len() > 100
    sparse_idx: Option<Vec<u32>>,
}

#[repr(C)]
pub struct CommentEntry {
    node_bp: u32,    // Owning node's BP position
    start: u32,      // Byte offset in source
    end: u32,        // End byte offset
    kind: u8,        // CommentKind
    _pad: [u8; 3],   // Keep 16-byte alignment
}

/// Tag tracking (extremely sparse)
pub struct TagIndex {
    /// Simple sorted vector - custom tags are rare
    /// (bp_position, tag_string)
    entries: Vec<(u32, String)>,
}
```

**Key design decisions**:

1. **Sorted Vec over BTreeMap**:
   - No pointer chasing, better cache locality
   - Binary search is O(log n) same as BTreeMap
   - Contiguous memory for prefetching

2. **Optional sparse index**:
   - Only built for files with many comments
   - Avoids overhead for typical sparse case
   - Enables O(1) approximate lookup when needed

3. **16-byte aligned entries**:
   - Fits 4 entries per cache line
   - No cross-line access for single entry

4. **Separate tag index**:
   - Tags are even sparser than comments
   - Simple vector sufficient for typical usage

**Memory overhead summary**:

| Scenario | Comments | Tags | Total Overhead |
|----------|----------|------|----------------|
| CI workflow (0.7% comments) | ~0.1% | ~0% | **0.1%** |
| K8s manifest (2.5% comments) | ~0.4% | ~0% | **0.4%** |
| Helm values (37% comments) | ~6% | ~0.1% | **6.1%** |

### A.7 Implementation Considerations

**Parser integration**:
```rust
// During parsing, collect comments with minimal overhead
struct Parser {
    // ... existing fields ...
    pending_comments: Vec<CommentEntry>,
}

impl Parser {
    fn parse_comment(&mut self, start: usize) {
        let end = self.skip_to_newline();
        // Defer node_bp assignment until we know which node owns this comment
        self.pending_comments.push(CommentEntry {
            node_bp: 0,  // Will be filled in
            start: start as u32,
            end: end as u32,
            kind: self.determine_comment_kind(),
            _pad: [0; 3],
        });
    }

    fn finalize_node(&mut self, bp_pos: usize) {
        // Assign pending comments to this node
        for comment in self.pending_comments.drain(..) {
            // ... assignment logic ...
        }
    }
}
```

**Cursor API extension**:
```rust
impl<'a, W: AsRef<[u64]>> YamlCursor<'a, W> {
    /// Get head comments (comments above this node)
    pub fn head_comments(&self) -> impl Iterator<Item = &str> {
        self.index.comments.get_for_node(self.bp_pos, CommentKind::Head)
            .map(|c| &self.text[c.start as usize..c.end as usize])
    }

    /// Get line comment (comment on same line)
    pub fn line_comment(&self) -> Option<&str> {
        self.index.comments.get_for_node(self.bp_pos, CommentKind::Line)
            .next()
            .map(|c| &self.text[c.start as usize..c.end as usize])
    }

    /// Get explicit tag if present
    pub fn explicit_tag(&self) -> Option<&str> {
        self.index.tags.get(self.bp_pos)
    }
}
```

### A.8 Conclusion

The recommended **Adaptive Sparse** design (Option 1 enhanced) provides:

- **Minimal memory**: 0.1-6% overhead depending on comment density
- **O(log n) lookup**: Binary search on sorted vector
- **Cache-friendly**: Contiguous storage, no pointer chasing
- **Simple implementation**: Leverages proven patterns from anchors/aliases
- **Adaptive scaling**: Optional sparse index for comment-heavy files

This design follows the key optimization principle: "Simpler is often faster." The lightweight sorted vector approach mirrors the successful DSV lightweight index pattern, which outperformed the theoretically optimal 3-level BitVec by 5-9x.

---

## Phase 1: Test Infrastructure (Week 1)

### 1.1 Create Test Framework

```rust
// tests/yq_conformance.rs

/// Test case structure matching YQ-* codes
#[derive(Debug)]
struct YqTestCase {
    code: &'static str,
    description: &'static str,
    input: &'static str,
    expression: &'static str,
    expected: Expected,
}

enum Expected {
    /// Single output value
    Value(&'static str),
    /// Multiple output values (separated by newlines)
    MultiValue(Vec<&'static str>),
    /// Error expected
    Error,
    /// Empty output
    Empty,
}
```

### 1.2 Test Data Files

Create public domain test data files:
- `tests/fixtures/yq/gutenberg_names.yaml`
- `tests/fixtures/yq/planets.yaml`
- `tests/fixtures/yq/elements.yaml`
- `tests/fixtures/yq/anchors.yaml`
- `tests/fixtures/yq/multidoc.yaml`

### 1.3 Conformance Test Runner

```rust
/// Run test and compare against real yq tool
fn run_conformance_test(test: &YqTestCase) -> Result<(), TestFailure> {
    // 1. Run real yq
    let yq_output = Command::new("yq")
        .arg(&test.expression)
        .stdin(test.input)
        .output()?;

    // 2. Run succinctly yq
    let our_output = succinctly_yq(test.input, test.expression)?;

    // 3. Compare
    assert_eq!(yq_output.stdout, our_output);
}
```

## Phase 2: Lexer and Parser (Week 2-3)

### 2.1 Token Types

```rust
#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    // Literals
    Null,
    True,
    False,
    Integer(i64),
    Float(f64),
    String(String),

    // Identifiers and variables
    Ident(String),
    Variable(String),     // $name

    // Path operators
    Dot,                  // .
    DotDot,               // ..
    DotDotDot,            // ...

    // Brackets
    LBracket,             // [
    RBracket,             // ]
    LBrace,               // {
    RBrace,               // }
    LParen,               // (
    RParen,               // )

    // Operators
    Pipe,                 // |
    Comma,                // ,
    Colon,                // :
    Question,             // ?

    // Comparison
    Eq,                   // ==
    Ne,                   // !=
    Lt,                   // <
    Le,                   // <=
    Gt,                   // >
    Ge,                   // >=

    // Arithmetic
    Plus,                 // +
    Minus,                // -
    Star,                 // *
    Slash,                // /
    Percent,              // %

    // Assignment
    Assign,               // =
    PipeAssign,           // |=
    PlusAssign,           // +=
    MinusAssign,          // -=
    StarAssign,           // *=
    SlashAssign,          // /=

    // Alternative
    SlashSlash,           // //

    // Keywords
    And,
    Or,
    Not,
    As,
    Ireduce,

    // Format functions
    Format(String),       // @json, @base64, etc.

    // Special
    Eof,
}
```

### 2.2 AST Types

```rust
#[derive(Debug, Clone)]
pub enum Expr {
    // Literals
    Null,
    Bool(bool),
    Integer(i64),
    Float(f64),
    String(String),

    // Path expressions
    Identity,                           // .
    RecurseDescent,                     // ..
    RecurseDescentAll,                  // ...
    Field(String),                      // .field
    BracketField(Box<Expr>),            // .["field"] or .[expr]
    Index(i64),                         // .[n]
    Slice { start: Option<i64>, end: Option<i64> },
    MultiIndex(Vec<Expr>),              // .[0, 2, 4]
    Iterate,                            // .[]
    Optional(Box<Expr>),                // expr?

    // Variables
    Variable(String),                   // $var

    // Operations
    Pipe(Vec<Expr>),
    Union(Vec<Expr>),                   // expr, expr

    // Binary operators
    Add(Box<Expr>, Box<Expr>),
    Sub(Box<Expr>, Box<Expr>),
    Mul(Box<Expr>, Box<Expr>),
    Div(Box<Expr>, Box<Expr>),
    Mod(Box<Expr>, Box<Expr>),

    Eq(Box<Expr>, Box<Expr>),
    Ne(Box<Expr>, Box<Expr>),
    Lt(Box<Expr>, Box<Expr>),
    Le(Box<Expr>, Box<Expr>),
    Gt(Box<Expr>, Box<Expr>),
    Ge(Box<Expr>, Box<Expr>),

    And(Box<Expr>, Box<Expr>),
    Or(Box<Expr>, Box<Expr>),
    Not(Box<Expr>),

    Alternative(Box<Expr>, Box<Expr>),  // //

    // Assignment
    Assign { path: Box<Expr>, value: Box<Expr> },
    UpdateAssign { path: Box<Expr>, update: Box<Expr> },

    // Constructors
    Array(Vec<Expr>),
    Object(Vec<ObjectEntry>),

    // Control flow
    Select(Box<Expr>),
    Map(Box<Expr>),
    MapValues(Box<Expr>),

    // Variables and reduce
    VarBind { name: String, value: Box<Expr>, body: Box<Expr> },
    Reduce {
        iter: Box<Expr>,
        var: String,
        init: Box<Expr>,
        update: Box<Expr>
    },

    // Function calls
    Call { name: String, args: Vec<Expr> },

    // Format functions
    Format(String),                     // @json, @base64, etc.

    // YAML-specific
    Anchor,
    Alias,
    LineComment,
    HeadComment,
    FootComment,
    Style,
    Tag,
    Kind,
    Explode(Box<Expr>),
}

#[derive(Debug, Clone)]
pub struct ObjectEntry {
    pub key: ObjectKey,
    pub value: Expr,
}

#[derive(Debug, Clone)]
pub enum ObjectKey {
    Literal(String),
    Expr(Expr),
}
```

### 2.3 Parser Implementation

Key parsing functions:

```rust
impl Parser {
    pub fn parse(&mut self) -> Result<Expr, ParseError> {
        self.parse_pipe()
    }

    fn parse_pipe(&mut self) -> Result<Expr, ParseError> {
        let mut exprs = vec![self.parse_union()?];
        while self.match_token(Token::Pipe) {
            exprs.push(self.parse_union()?);
        }
        Ok(if exprs.len() == 1 { exprs.pop().unwrap() } else { Expr::Pipe(exprs) })
    }

    fn parse_union(&mut self) -> Result<Expr, ParseError> {
        let mut exprs = vec![self.parse_alternative()?];
        while self.match_token(Token::Comma) {
            exprs.push(self.parse_alternative()?);
        }
        Ok(if exprs.len() == 1 { exprs.pop().unwrap() } else { Expr::Union(exprs) })
    }

    // ... precedence climbing for operators

    fn parse_postfix(&mut self) -> Result<Expr, ParseError> {
        let mut expr = self.parse_primary()?;
        loop {
            expr = match self.peek() {
                Token::Dot => self.parse_field_access(expr)?,
                Token::LBracket => self.parse_bracket_access(expr)?,
                Token::Question => { self.advance(); Expr::Optional(Box::new(expr)) }
                Token::LParen => self.parse_call(expr)?,
                _ => break,
            };
        }
        Ok(expr)
    }

    fn parse_bracket_access(&mut self, base: Expr) -> Result<Expr, ParseError> {
        self.expect(Token::LBracket)?;

        // Handle .[expr] - could be index, slice, or field
        if self.check(Token::RBracket) {
            // .[] - iterate
            self.advance();
            return Ok(Expr::Pipe(vec![base, Expr::Iterate]));
        }

        // Check for slice starting with :
        if self.check(Token::Colon) {
            // .[:n] or .[:]
            return self.parse_slice(base, None);
        }

        // Parse first expression
        let first = self.parse_expr()?;

        match self.peek() {
            Token::RBracket => {
                // .[n] or .["field"]
                self.advance();
                Ok(Expr::Pipe(vec![base, Expr::BracketField(Box::new(first))]))
            }
            Token::Colon => {
                // .[n:] or .[n:m]
                self.parse_slice(base, Some(first))
            }
            Token::Comma => {
                // .[n, m, ...]
                self.parse_multi_index(base, first)
            }
            _ => Err(ParseError::unexpected(self.peek())),
        }
    }
}
```

## Phase 3: Evaluator Design (Week 4-5)

### 3.1 Value Type

The evaluator works with a `Value` type that can reference cursor positions:

```rust
/// A value that may reference original document via cursor
#[derive(Debug, Clone)]
pub enum Value<'a> {
    /// Null value
    Null,

    /// Boolean
    Bool(bool),

    /// Integer
    Integer(i64),

    /// Floating point
    Float(f64),

    /// String - may be borrowed from source or owned
    String(Cow<'a, str>),

    /// Array - may reference cursor positions or be constructed
    Array(ArrayValue<'a>),

    /// Map - may reference cursor positions or be constructed
    Map(MapValue<'a>),

    /// Reference to original document via cursor
    /// This is the key to zero-copy output!
    Cursor(YamlCursor<'a>),
}

/// Array that may reference cursors or contain computed values
#[derive(Debug, Clone)]
pub enum ArrayValue<'a> {
    /// Reference to array in original document
    Cursor(YamlCursor<'a>),
    /// Constructed array of values
    Owned(Vec<Value<'a>>),
}

/// Map that may reference cursors or contain computed values
#[derive(Debug, Clone)]
pub enum MapValue<'a> {
    /// Reference to map in original document
    Cursor(YamlCursor<'a>),
    /// Constructed map
    Owned(Vec<(Cow<'a, str>, Value<'a>)>),
}
```

### 3.2 Evaluation Context

```rust
pub struct EvalContext<'a> {
    /// Current document
    doc: &'a YamlIndex,

    /// Variable bindings
    vars: HashMap<String, Value<'a>>,

    /// Output collector
    output: OutputCollector<'a>,
}

/// Collects output values for formatting
pub struct OutputCollector<'a> {
    values: Vec<Value<'a>>,
}
```

### 3.3 Evaluator Core

```rust
impl<'a> Evaluator<'a> {
    /// Evaluate expression against a value, producing zero or more results
    pub fn eval(&mut self, expr: &Expr, input: Value<'a>) -> Result<Vec<Value<'a>>, EvalError> {
        match expr {
            Expr::Identity => Ok(vec![input]),

            Expr::Field(name) => self.eval_field(input, name),

            Expr::BracketField(key_expr) => {
                let keys = self.eval(key_expr, input.clone())?;
                let mut results = Vec::new();
                for key in keys {
                    results.extend(self.eval_bracket_access(input.clone(), key)?);
                }
                Ok(results)
            }

            Expr::Iterate => self.eval_iterate(input),

            Expr::Pipe(exprs) => {
                let mut current = vec![input];
                for expr in exprs {
                    let mut next = Vec::new();
                    for val in current {
                        next.extend(self.eval(expr, val)?);
                    }
                    current = next;
                }
                Ok(current)
            }

            Expr::Union(exprs) => {
                let mut results = Vec::new();
                for expr in exprs {
                    results.extend(self.eval(expr, input.clone())?);
                }
                Ok(results)
            }

            Expr::Select(cond) => {
                let cond_results = self.eval(cond, input.clone())?;
                for cond_val in cond_results {
                    if cond_val.is_truthy() {
                        return Ok(vec![input]);
                    }
                }
                Ok(vec![])
            }

            // ... etc
        }
    }

    /// Evaluate field access - KEY OPTIMIZATION POINT
    fn eval_field(&mut self, input: Value<'a>, name: &str) -> Result<Vec<Value<'a>>, EvalError> {
        match input {
            Value::Cursor(cursor) => {
                // Direct cursor navigation - no allocation!
                if let Some(child) = cursor.get_field(name) {
                    Ok(vec![Value::Cursor(child)])
                } else {
                    Ok(vec![Value::Null])
                }
            }
            Value::Map(MapValue::Cursor(cursor)) => {
                if let Some(child) = cursor.get_field(name) {
                    Ok(vec![Value::Cursor(child)])
                } else {
                    Ok(vec![Value::Null])
                }
            }
            Value::Map(MapValue::Owned(entries)) => {
                for (k, v) in entries {
                    if k == name {
                        return Ok(vec![v]);
                    }
                }
                Ok(vec![Value::Null])
            }
            _ => Ok(vec![Value::Null]),
        }
    }

    /// Evaluate iteration - returns cursor references where possible
    fn eval_iterate(&mut self, input: Value<'a>) -> Result<Vec<Value<'a>>, EvalError> {
        match input {
            Value::Cursor(cursor) => {
                let mut results = Vec::new();
                for child in cursor.children() {
                    results.push(Value::Cursor(child));
                }
                Ok(results)
            }
            Value::Array(ArrayValue::Cursor(cursor)) => {
                let mut results = Vec::new();
                for child in cursor.children() {
                    results.push(Value::Cursor(child));
                }
                Ok(results)
            }
            Value::Array(ArrayValue::Owned(arr)) => Ok(arr),
            Value::Map(MapValue::Cursor(cursor)) => {
                let mut results = Vec::new();
                for (_, child) in cursor.entries() {
                    results.push(Value::Cursor(child));
                }
                Ok(results)
            }
            Value::Map(MapValue::Owned(entries)) => {
                Ok(entries.into_iter().map(|(_, v)| v).collect())
            }
            _ => Err(EvalError::CannotIterate),
        }
    }
}
```

## Phase 4: Output Formatting (Week 6)

### 4.1 Efficient Cursor-Based Output

The key performance optimization: when outputting a `Value::Cursor`, consume source bytes directly without intermediate string allocation. The output must match yq's normalized format (not preserve source formatting).

```rust
pub struct YamlWriter<W: Write> {
    writer: W,
    indent: usize,
    text: &[u8],  // Source document for efficient string access
}

impl<W: Write> YamlWriter<W> {
    /// Write a value to output in yq-compatible format
    ///
    /// For cursor-backed values, reads source bytes directly and writes
    /// to output buffer without intermediate String allocation.
    pub fn write_value(&mut self, value: &Value, doc: &YamlIndex) -> io::Result<()> {
        match value {
            Value::Cursor(cursor) => {
                // Efficient path: serialize cursor content directly to output
                // without building intermediate String
                self.write_cursor_value(cursor, doc)
            }
            Value::Null => write!(self.writer, "null"),
            Value::Bool(b) => write!(self.writer, "{}", b),
            Value::Integer(n) => write!(self.writer, "{}", n),
            Value::Float(f) => self.write_float(*f),
            Value::String(s) => self.write_yaml_string(s),
            Value::Array(arr) => self.write_array(arr, doc),
            Value::Map(map) => self.write_map(map, doc),
        }
    }

    /// Serialize cursor value directly to output in yq format.
    ///
    /// Consumes source bytes and writes computed output bytes directly,
    /// avoiding temporary string allocations where possible.
    fn write_cursor_value(&mut self, cursor: &YamlCursor, doc: &YamlIndex) -> io::Result<()> {
        match cursor.value() {
            YamlValue::Null => write!(self.writer, "null"),
            YamlValue::String(s) => {
                // Decode string and write in yq's preferred format
                // Uses cursor's byte range to read source, writes directly to output
                self.write_yaml_string_from_cursor(&s)
            }
            YamlValue::Mapping(fields) => self.write_mapping_from_cursor(fields),
            YamlValue::Sequence(elements) => self.write_sequence_from_cursor(elements),
            YamlValue::Error(msg) => Err(io::Error::new(io::ErrorKind::InvalidData, msg)),
        }
    }

    /// Write array, using cursors where possible
    fn write_array(&mut self, arr: &ArrayValue, doc: &YamlIndex) -> io::Result<()> {
        match arr {
            ArrayValue::Cursor(cursor) => self.write_cursor_value(cursor, doc),
            ArrayValue::Owned(values) => {
                for (i, val) in values.iter().enumerate() {
                    if i > 0 {
                        self.write_newline()?;
                    }
                    write!(self.writer, "- ")?;
                    self.write_value(val, doc)?;
                }
                Ok(())
            }
        }
    }
}
```

### 4.2 Output Modes

```rust
pub enum OutputFormat {
    Yaml { indent: usize },
    Json { indent: usize },
    JsonCompact,
    Props,
    Csv,
    Tsv,
    Raw,  // -r flag
}

pub struct OutputOptions {
    pub format: OutputFormat,
    pub multi_doc_separator: bool,  // Output --- between docs
}
```

## Phase 5: Built-in Functions (Week 7-8)

### 5.1 Function Registry

```rust
pub type BuiltinFn = fn(&mut Evaluator, Vec<Value>) -> Result<Vec<Value>, EvalError>;

pub struct FunctionRegistry {
    functions: HashMap<&'static str, BuiltinFn>,
}

impl FunctionRegistry {
    pub fn new() -> Self {
        let mut reg = Self { functions: HashMap::new() };

        // Collection functions
        reg.register("length", builtin_length);
        reg.register("keys", builtin_keys);
        reg.register("values", builtin_values);
        reg.register("has", builtin_has);
        reg.register("contains", builtin_contains);

        // Array functions
        reg.register("first", builtin_first);
        reg.register("sort", builtin_sort);
        reg.register("sort_by", builtin_sort_by);
        reg.register("reverse", builtin_reverse);
        reg.register("unique", builtin_unique);
        reg.register("unique_by", builtin_unique_by);
        reg.register("flatten", builtin_flatten);
        reg.register("group_by", builtin_group_by);
        reg.register("min", builtin_min);
        reg.register("max", builtin_max);
        reg.register("add", builtin_add);
        reg.register("any", builtin_any);
        reg.register("all", builtin_all);
        reg.register("any_c", builtin_any_c);
        reg.register("all_c", builtin_all_c);
        reg.register("map", builtin_map);
        reg.register("shuffle", builtin_shuffle);

        // Map functions
        reg.register("to_entries", builtin_to_entries);
        reg.register("from_entries", builtin_from_entries);
        reg.register("with_entries", builtin_with_entries);
        reg.register("pick", builtin_pick);
        reg.register("omit", builtin_omit);
        reg.register("sort_keys", builtin_sort_keys);
        reg.register("map_values", builtin_map_values);
        reg.register("pivot", builtin_pivot);

        // String functions
        reg.register("split", builtin_split);
        reg.register("join", builtin_join);
        reg.register("trim", builtin_trim);
        reg.register("ltrim", builtin_ltrim);
        reg.register("rtrim", builtin_rtrim);
        reg.register("upcase", builtin_upcase);
        reg.register("downcase", builtin_downcase);
        reg.register("test", builtin_test);
        reg.register("match", builtin_match);
        reg.register("capture", builtin_capture);
        reg.register("sub", builtin_sub);

        // Type functions
        reg.register("type", builtin_type);
        reg.register("tag", builtin_tag);
        reg.register("kind", builtin_kind);
        reg.register("to_string", builtin_to_string);
        reg.register("to_number", builtin_to_number);

        // Path functions
        reg.register("path", builtin_path);
        reg.register("setpath", builtin_setpath);
        reg.register("delpaths", builtin_delpaths);
        reg.register("parent", builtin_parent);
        reg.register("parents", builtin_parents);
        reg.register("key", builtin_key);

        // Control flow
        reg.register("select", builtin_select);
        reg.register("empty", builtin_empty);
        reg.register("error", builtin_error);
        reg.register("del", builtin_del);

        // YAML-specific
        reg.register("anchor", builtin_anchor);
        reg.register("alias", builtin_alias);
        reg.register("explode", builtin_explode);
        reg.register("line_comment", builtin_line_comment);
        reg.register("head_comment", builtin_head_comment);
        reg.register("foot_comment", builtin_foot_comment);
        reg.register("style", builtin_style);
        reg.register("document_index", builtin_document_index);
        reg.register("split_doc", builtin_split_doc);

        reg
    }
}
```

### 5.2 Example Function Implementations

```rust
fn builtin_length(eval: &mut Evaluator, args: Vec<Value>) -> Result<Vec<Value>, EvalError> {
    let input = args.into_iter().next().ok_or(EvalError::MissingArg)?;
    let len = match input {
        Value::Null => 0,
        Value::String(s) => s.chars().count() as i64,
        Value::Array(ArrayValue::Cursor(c)) => c.len() as i64,
        Value::Array(ArrayValue::Owned(a)) => a.len() as i64,
        Value::Map(MapValue::Cursor(c)) => c.entry_count() as i64,
        Value::Map(MapValue::Owned(m)) => m.len() as i64,
        _ => return Err(EvalError::TypeError("length")),
    };
    Ok(vec![Value::Integer(len)])
}

fn builtin_keys(eval: &mut Evaluator, args: Vec<Value>) -> Result<Vec<Value>, EvalError> {
    let input = args.into_iter().next().ok_or(EvalError::MissingArg)?;
    match input {
        Value::Map(MapValue::Cursor(c)) => {
            let keys: Vec<Value> = c.keys()
                .map(|k| Value::String(Cow::Borrowed(k)))
                .collect();
            Ok(vec![Value::Array(ArrayValue::Owned(keys))])
        }
        Value::Map(MapValue::Owned(m)) => {
            let keys: Vec<Value> = m.iter()
                .map(|(k, _)| Value::String(k.clone()))
                .collect();
            Ok(vec![Value::Array(ArrayValue::Owned(keys))])
        }
        Value::Array(ArrayValue::Cursor(c)) => {
            let keys: Vec<Value> = (0..c.len())
                .map(|i| Value::Integer(i as i64))
                .collect();
            Ok(vec![Value::Array(ArrayValue::Owned(keys))])
        }
        Value::Array(ArrayValue::Owned(a)) => {
            let keys: Vec<Value> = (0..a.len())
                .map(|i| Value::Integer(i as i64))
                .collect();
            Ok(vec![Value::Array(ArrayValue::Owned(keys))])
        }
        _ => Err(EvalError::TypeError("keys")),
    }
}

fn builtin_sort(eval: &mut Evaluator, args: Vec<Value>) -> Result<Vec<Value>, EvalError> {
    let input = args.into_iter().next().ok_or(EvalError::MissingArg)?;
    let mut arr = match input {
        Value::Array(ArrayValue::Cursor(c)) => c.to_vec(),  // Materialize cursor
        Value::Array(ArrayValue::Owned(a)) => a,
        _ => return Err(EvalError::TypeError("sort")),
    };

    arr.sort_by(|a, b| a.cmp_values(b));
    Ok(vec![Value::Array(ArrayValue::Owned(arr))])
}
```

## Phase 6: YAML-Specific Features (Week 9)

### 6.1 Anchor/Alias Support

```rust
fn builtin_anchor(eval: &mut Evaluator, args: Vec<Value>) -> Result<Vec<Value>, EvalError> {
    let input = args.into_iter().next().ok_or(EvalError::MissingArg)?;
    match input {
        Value::Cursor(c) => {
            if let Some(anchor) = c.anchor() {
                Ok(vec![Value::String(Cow::Borrowed(anchor))])
            } else {
                Ok(vec![Value::Null])
            }
        }
        _ => Ok(vec![Value::Null]),
    }
}

fn builtin_explode(eval: &mut Evaluator, args: Vec<Value>) -> Result<Vec<Value>, EvalError> {
    let input = args.into_iter().next().ok_or(EvalError::MissingArg)?;
    // Recursively resolve all aliases and remove anchor names
    Ok(vec![eval.explode_value(input)?])
}
```

### 6.2 Comment Support

```rust
fn builtin_line_comment(eval: &mut Evaluator, args: Vec<Value>) -> Result<Vec<Value>, EvalError> {
    let input = args.into_iter().next().ok_or(EvalError::MissingArg)?;
    match input {
        Value::Cursor(c) => {
            if let Some(comment) = c.line_comment() {
                Ok(vec![Value::String(Cow::Borrowed(comment))])
            } else {
                Ok(vec![Value::Null])
            }
        }
        _ => Ok(vec![Value::Null]),
    }
}
```

## Phase 7: Integration and CLI (Week 10)

### 7.1 Public API

```rust
// src/yq/mod.rs

pub use ast::Expr;
pub use parser::parse;
pub use eval::{Evaluator, Value};
pub use output::{OutputFormat, YamlWriter};

/// Parse and evaluate a yq expression
pub fn query<'a>(
    expression: &str,
    doc: &'a YamlIndex,
) -> Result<Vec<Value<'a>>, Error> {
    let expr = parse(expression)?;
    let mut eval = Evaluator::new(doc);
    eval.eval(&expr, Value::Cursor(doc.root()))
}

/// Parse, evaluate, and format output
pub fn query_to_string(
    expression: &str,
    doc: &YamlIndex,
    format: OutputFormat,
) -> Result<String, Error> {
    let values = query(expression, doc)?;
    let mut output = String::new();
    let mut writer = YamlWriter::new(&mut output, format);
    for value in values {
        writer.write_value(&value, doc)?;
        writer.write_separator()?;
    }
    Ok(output)
}
```

### 7.2 CLI Integration

```rust
// src/bin/succinctly.rs

fn run_yq(args: YqArgs) -> Result<()> {
    let content = fs::read_to_string(&args.file)?;
    let index = YamlIndex::from_str(&content)?;

    let format = match (args.output_format.as_deref(), args.raw) {
        (Some("json"), _) => OutputFormat::Json { indent: args.indent },
        (Some("props"), _) => OutputFormat::Props,
        (_, true) => OutputFormat::Raw,
        _ => OutputFormat::Yaml { indent: args.indent },
    };

    let output = yq::query_to_string(&args.expression, &index, format)?;
    print!("{}", output);
    Ok(())
}
```

## Implementation Order by Test Code

The implementation should proceed by making tests pass in this order:

### Batch 1: Core Path Expressions
```
YQ-TRV-001 through YQ-TRV-025
```

### Batch 2: Array Operations
```
YQ-ARR-001 through YQ-ARR-028
```

### Batch 3: Map Operations
```
YQ-MAP-001 through YQ-MAP-020
```

### Batch 4: String Operations
```
YQ-STR-001 through YQ-STR-020
```

### Batch 5: Numeric and Comparison
```
YQ-NUM-001 through YQ-NUM-012
YQ-CMP-001 through YQ-CMP-014
```

### Batch 6: Boolean and Control Flow
```
YQ-BOL-001 through YQ-BOL-010
YQ-CTL-001 through YQ-CTL-010
```

### Batch 7: Variables and Constructors
```
YQ-VAR-001 through YQ-VAR-007
YQ-CON-001 through YQ-CON-009
```

### Batch 8: Format Functions
```
YQ-FMT-001 through YQ-FMT-007
```

### Batch 9: YAML-Specific
```
YQ-YML-001 through YQ-YML-012
```

### Batch 10: Error Handling
```
YQ-ERR-001 through YQ-ERR-005
```

## Performance Targets

| Metric | Target |
|--------|--------|
| Parse 1KB expression | < 100 µs |
| Evaluate `.a.b.c` on 1MB YAML | < 1 ms |
| Full query `.users[].name` on 10MB | < 50 ms |
| Memory overhead vs input size | < 20% |

## Risk Mitigation

### Risk 1: Cursor invalidation
**Mitigation**: Cursor references are only valid during evaluation. Force materialization before mutation.

### Risk 2: yq version differences
**Mitigation**: Pin conformance tests to yq v4.x, document any known differences.

### Risk 3: Complex merge semantics
**Mitigation**: Implement merge flags (`*+`, `*d`, etc.) incrementally, with thorough testing.

## Success Criteria

1. All YQ-* tests pass
2. Byte-for-byte match with yq tool for supported features
3. Performance within 2x of existing jq implementation
4. Zero-copy output for identity queries (`.`)
