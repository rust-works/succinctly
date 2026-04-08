---
name: update-adr-inventory
description: Update the ADR inventory table in docs/adrs/README.md by scanning all adr-*.md files, extracting their title, status, and date. Use when a new ADR is added or an existing ADR's status or title has changed.
---

# Update ADR Inventory Skill

This skill updates the inventory table in `docs/adrs/README.md` to reflect the current set of ADR files.

## Process

1. **Scan** `docs/adrs/` for all files matching `adr-*.md`
2. **Extract** from each file:
   - **ADR number** from the filename (e.g., `adr-0003.md` -> `ADR-0003`)
   - **Title** from the `# ADR-NNNN: <Title>` heading on line 1
   - **Status** from the `## Status` section (line after the heading, e.g., `✅ Accepted`)
3. **Date**: Use the git log to find the commit date when the file was first added:
   ```bash
   git log --diff-filter=A --format=%as -- docs/adrs/adr-NNNN.md
   ```
   If the file is untracked (not yet committed), use today's date.
4. **Rebuild** the inventory table in `docs/adrs/README.md`, sorted by ADR number ascending
5. **Format** the table using the markdown-tables skill for consistent column widths

## Table Format

The inventory table uses this structure:

```markdown
## Inventory

| ADR                      | Status      | Date       | Title                             |
|--------------------------|-------------|------------|-----------------------------------|
| [ADR-0000](adr-0000.md)  | ✅ Accepted | 2026-02-10 | Use Architecture Decision Records  |
```

### Column Details

| Column | Source                              | Example                             |
|--------|-------------------------------------|-------------------------------------|
| ADR    | Filename, linked: `[ADR-NNNN](...)` | `[ADR-0000](adr-0000.md)`           |
| Status | Status section with emoji prefix    | `✅ Accepted`                        |
| Date   | Git first-commit date or today      | `2026-02-10`                        |
| Title  | H1 heading after `ADR-NNNN: `       | `Use Architecture Decision Records` |

## Instructions

1. Read all `adr-*.md` files in `docs/adrs/`
2. For each file, extract the title from line 1 and status from the Status section
3. Look up the git creation date for each file
4. Replace the inventory table section in `docs/adrs/README.md` with the updated table
5. Ensure the table is formatted with consistent column widths
