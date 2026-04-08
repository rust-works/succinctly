---
name: knowledge-map
description: Update the docs/ knowledge wiki when adding or changing data structures, modules, optimizations, SIMD paths, or paper references. Triggers on "knowledge map", "wiki", "update wiki", "add to wiki".
user-invocable: false
---

# Knowledge Map Maintenance

The knowledge wiki lives in `docs/` with `docs/index.md` as the entry point. Each core concept has its own page. When the codebase changes in ways that affect the wiki, update the relevant pages.

## Page Structure

Every concept page follows this structure:

1. **Breadcrumb**: `[Home](../../) > [Docs](../) > [Category](./) > PageName`
2. **What It Does** — one-paragraph summary
3. **How It Works** — key algorithms and data structures
4. **Depends On** — links to wiki pages this concept uses
5. **Used By** — links to wiki pages that use this concept
6. **Academic Papers** — citations with links where available
7. **Source & Docs** — links to implementation files and existing architecture/parsing docs

## When to Update

### New data structure or module

1. Create the page in the appropriate subdirectory (e.g. `docs/parsing/`, `docs/optimizations/`, `docs/reference/`) following the page structure above
2. Add a row to the Core Data Structures table in `docs/index.md`
3. Update Depends On / Used By sections on related pages (keep these bidirectional)
4. If the module has SIMD variants, add to `docs/optimizations/simd-strategy.md`

### New optimization

1. Add a row to the relevant concept page's optimization table (e.g. the Optimization Journey table in `docs/parsing/yaml-index.md`)
2. If it involves a new SIMD technique, update `docs/optimizations/simd-strategy.md`

### New academic paper reference

1. Add to the Academic Foundations table in `docs/index.md`
2. Add to the relevant concept page's Academic Papers section
3. If the URL is on `doi.org`, it's already excluded from link checking (returns 403 to bots)

### New SIMD path

1. Update the Platform Support table in `docs/optimizations/simd-strategy.md`
2. Update the Per-Module SIMD Usage section in `docs/optimizations/simd-strategy.md`
3. Update the SIMD Platform Support table in `docs/index.md` if a new platform is added

## Formatting Rules

- Tables must use fixed-width columns with consistent padding (invoke the `format-md-tables` skill)
- Use mermaid diagrams for flowcharts and dependency graphs, not ASCII art
- Use relative markdown links (not wikilinks)

## After Updating

1. Append an entry to `docs/log.md` with the date, what changed, and what sources were read
2. Verify links: `lychee '**/*.md'` (or let CI catch them)
3. Check that Depends On / Used By links are bidirectional
