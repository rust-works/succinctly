---
name: update-adr-inventory
description: Update the ADR inventory table in docs/adrs/README.md by scanning all adr-*.md files, extracting their title, status, and date. Use when a new ADR is added or an existing ADR's status or title has changed.
allowed-tools: Read, Edit, Glob, Grep, Bash(git log *)
---

# Update ADR Inventory

Rebuild the inventory table in `docs/adrs/README.md` so it reflects the current
set of ADR files and their metadata.

## Critical: Minimise diff noise

**Only touch rows that actually changed.** When adding a new ADR, append a row
matching the existing column widths — do NOT reformat the header, separator, or
unchanged data rows. When updating an ADR's status or title, edit only that
row's cell content and pad to match the existing column width. The goal is a
minimal, reviewable git diff.

## Procedure

1. **Read the current inventory.** Read `docs/adrs/README.md` and parse the
   existing table to understand current column widths and which ADRs are already
   listed (with their status and title).

2. **Find all ADR files.** Glob for `docs/adrs/adr-*.md` (exclude `README.md`).
   Sort results in natural numeric order by ADR number.

3. **Extract metadata from each ADR file.** For every file, read the first ~10
   lines and extract:
   - **ADR number and title** from the level-1 heading: `# ADR-NNNN: Title`
   - **Status** from the `## Status` section (the first non-empty line after
     `## Status`)

4. **Determine the date.** Use git to get the author date of the commit that
   first introduced the file:
   ```
   git log --diff-filter=A --follow --format="%as" -- <file> | tail -1
   ```
   If the file is untracked (no git history), use today's date.

5. **Map status text to the table format.** Normalise the raw status string
   (strip any existing emoji) and apply the project's status legend:

   | Raw (case-insensitive) | Table display    |
   |------------------------|------------------|
   | Proposed               | 🟡 Proposed      |
   | Accepted               | ✅ Accepted      |
   | Deprecated             | ❌ Deprecated    |
   | Superseded             | 🔄 Superseded   |

   If the status does not match any of the above, use the raw text as-is.

6. **Compare and apply minimal edits.**
   - **New ADRs:** Append a row, padding each cell to match the existing column
     widths (right-pad with spaces). Do not alter the header or separator rows
     unless the new content exceeds an existing column width.
   - **Changed status/title:** Edit only the affected cell(s) in the existing
     row, adjusting padding to maintain alignment.
   - **Removed ADRs:** Delete only that row.
   - **No changes:** Do nothing — do not rewrite the table.

   Only widen columns (header, separator, and all data rows) if new content
   exceeds the current column width. Never shrink columns.

7. **Report what changed.** Tell the user which ADRs were added, removed, or
   updated compared to the previous inventory, or confirm that the inventory
   was already up to date.
