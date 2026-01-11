---
name: markdown-tables
description: Formats markdown tables with fixed-width columns for consistent alignment. Use when creating tables, reformatting existing tables, or ensuring table readability. Triggers on terms like "format table", "align table", "fix table", "table columns", "markdown table".
---

# Markdown Table Formatting Skill

**Markdown tables MUST use fixed-width columns with space padding.** This is mandatory for readability in plain text editors and diffs.

## Correct vs Incorrect Format

### Correct: Fixed-width columns with padding

```markdown
| Size     | succinctly          | jq                  | Speedup    |
|----------|---------------------|---------------------|------------|
| **10KB** | 2.2 ms (4.3 MiB/s)  | 4.1 ms (2.3 MiB/s)  | **1.86x**  |
| **1MB**  | 24.5 ms (33 MiB/s)  | 44 ms (18 MiB/s)    | **1.79x**  |
```

### Incorrect: Unpadded columns

```markdown
| Size | succinctly | jq | Speedup |
|------|------------|-----|---------|
| **10KB** | 2.2 ms (4.3 MiB/s) | 4.1 ms (2.3 MiB/s) | **1.86x** |
```

## Column Width Rules

1. **Measure the widest content** in each column (including header)
2. **Pad all cells** to match the widest cell in that column
3. **Use spaces for alignment**, not tabs
4. **Align text left**, numbers can be right-aligned for decimals
5. **Leave one space** after `|` and before `|` for readability in data rows
6. **Separator row has NO spaces** - format as `|------|---------|` with dashes matching column widths
7. **Empty cells use single dash** "-" padded with spaces: `| -          |`

## Formatting Algorithm

```
For each column:
  1. width = max(len(cell) for cell in column)
  2. width = max(width, 3)  # Minimum 3 chars for separator

For each row:
  1. For each cell:
     - Pad content to column width
     - Add single space before and after content
  2. Join cells with '|' delimiter
  3. Add '|' at start and end
```

## Bold Text in Tables

When using bold (`**text**`) in table cells:

**Spaces must be OUTSIDE the bold markers**, not inside:

```markdown
<!-- CORRECT: Spaces outside ** -->
|   **59.6ms** |

<!-- WRONG: Spaces inside ** (markdown won't render bold) -->
| ** 59.6ms** |
```

This is important for right-aligned numeric columns where padding is added.

## Alignment Markers

| Alignment | Separator Format | Example        |
|-----------|------------------|----------------|
| Left      | `|----------|`   | Default        |
| Right     | `|---------:|`   | Numbers        |
| Center    | `|:--------:|`   | Headers        |

## Complex Tables

For tables with many columns or very long content:

```markdown
| Name       | Type   | Size    | Description                          |
|------------|--------|---------|--------------------------------------|
| ib         | Vec    | ~N bits | Interest bits marking structural     |
|            |        |         | characters and value starts          |
| bp         | Vec    | ~N bits | Balanced parens encoding tree        |
|            |        |         | structure                            |
```

Consider:
- Splitting into multiple smaller tables
- Using a different format (bullet lists, nested structure)
- Rotating the table (rows become columns)

## Why Fixed-Width Tables Matter

1. **Readability in Git Diffs**: Variable-width tables are hard to review in diff views
2. **Text Editor Alignment**: Most code reviews happen in monospace fonts
3. **Terminal Display**: Tables render correctly in terminal viewers
4. **Professional Appearance**: Shows attention to detail
5. **Easier Maintenance**: Clear structure makes updates simpler

## Instructions

When formatting tables:

1. **Read the table** - Identify all rows and columns
2. **Calculate widths** - Find max content width per column
3. **Format header** - Apply consistent width with padding
4. **Format separator** - Match column widths with dashes
5. **Format data rows** - Apply same widths to all cells
6. **Preserve alignment** - Keep `:` markers for right/center alignment
