# Markdown Formatting Guidelines

## Tables

**ALWAYS use fixed-width columns for markdown tables** to ensure readability in plain text editors and during code review.

### ✅ Good Example (Fixed Width)

```markdown
| Parser     | Throughput | vs Baseline | Notes                    |
|------------|------------|-------------|--------------------------|
| sonic-rs   | 723 MiB/s  | 4.06x       | Fastest overall          |
| succinctly | 539 MiB/s  | 3.03x       | 2nd fastest              |
| serde_json | 178 MiB/s  | baseline    | Standard library         |
| simd-json  | 117 MiB/s  | 0.66x       | Unexpectedly slow on AMD |
```

**Key features:**
- Separator row has **no spaces**, only pipes and dashes: `|------------|`
- Data rows have **spaces around content**: `| sonic-rs   |`
- All columns padded to same width (e.g., "Parser" column is 10 chars wide)

### ❌ Bad Example (Variable Width)

```markdown
| Parser     | Throughput | vs Baseline | Notes                    |
|------------|------------|-------------|--------------------------|
| sonic-rs   | 723 MiB/s  | 4.06x       | Fastest overall          |
| succinctly | 539 MiB/s  | 3.03x       | 2nd fastest              |
| serde_json | 178 MiB/s  | baseline    | Standard library         |
| simd-json  | 117 MiB/s  | 0.66x       | Unexpectedly slow on AMD |
```

**Problems:**
- Inconsistent column widths
- Hard to read in plain text
- Diffs are noisy when content changes

### Column Width Rules

1. **Measure the widest content** in each column (including header)
2. **Pad all cells** to match the widest cell in that column
3. **Use spaces for alignment**, not tabs
4. **Align text left**, numbers can be right-aligned for decimals
5. **Leave one space** after `|` and before `|` for readability in data rows
6. **Separator row has NO spaces** - format as `|------|---------|` with dashes matching column widths
7. **Empty cells use single dash** "-" padded with spaces: `| -          |`

### Example: Calculating Column Widths

For this data:
- Column 1: "succinctly" (10 chars) is longest
- Column 2: "Throughput" (10 chars) is longest
- Column 3: "vs Baseline" (11 chars) is longest
- Column 4: "Unexpectedly slow on AMD" (24 chars) is longest

So the template becomes:
```
| 10 chars | 10 chars | 11 chars | 24 chars |
```

### Tools and Automation

When creating tables manually:
1. Draft the table with correct content
2. Identify the widest entry in each column
3. Pad all cells to match
4. Verify alignment with a monospace font

For large tables, consider:
- Using a markdown table formatter/plugin
- Writing a small script to auto-format
- Using editor plugins that align tables on save

### Complex Tables

For tables with many columns or very long content:

```markdown
| Name       | Type   | Size    | Description                          |
|------------|--------|---------|--------------------------------------|
| ib         | Vec    | ~N bits | Interest bits marking structural     |
|            |        |         | characters and value starts          |
| bp         | Vec    | ~N bits | Balanced parens encoding tree        |
|            |        |         | structure                            |
| ib_rank    | Vec    | ~3% N   | Rank directory for O(1) rank1        |
| bp_rank    | Vec    | ~3% N   | Rank directory for BP bitvector      |
```

Note: For very wide tables, consider:
- Splitting into multiple smaller tables
- Using a different format (bullet lists, nested structure)
- Rotating the table (rows become columns)

## Why Fixed-Width Tables Matter

1. **Readability in Git Diffs**: Variable-width tables are hard to review in diff views
2. **Text Editor Alignment**: Most code reviews happen in monospace fonts
3. **Terminal Display**: Tables render correctly in terminal viewers
4. **Professional Appearance**: Shows attention to detail
5. **Easier Maintenance**: Clear structure makes updates simpler

## Other Markdown Guidelines

### Code Blocks

Always specify the language for syntax highlighting:

```markdown
\`\`\`rust
fn main() {
    println!("Hello, world!");
}
\`\`\`
```

Not:
```markdown
\`\`\`
fn main() {
    println!("Hello, world!");
}
\`\`\`
```

### Headings

Use ATX-style headers (# ## ###) consistently, not Setext-style (underlines):

✅ Good:
```markdown
## Section Title
### Subsection
```

❌ Bad:
```markdown
Section Title
-------------
```

### Lists

Use consistent indentation (2 or 4 spaces for nested items):

```markdown
- Top level item
  - Nested item (2 spaces)
  - Another nested item
    - Deeply nested (4 spaces total)
```

### Links

Use reference-style links for repeated URLs:

```markdown
See [documentation][docs] for details. More info in [the guide][docs].

[docs]: https://example.com/docs
```

## Verification

Before committing markdown files:

1. ✅ All tables use fixed-width columns
2. ✅ Code blocks have language specifiers
3. ✅ Consistent heading style (ATX)
4. ✅ No trailing whitespace
5. ✅ Blank line before/after code blocks and tables
6. ✅ Consistent list indentation
