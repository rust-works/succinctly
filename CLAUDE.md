# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## AI Scratch Directory

When working on this project, use `.ai/scratch/` for temporary files:

- **Location**: `.ai/scratch/` in the repository root
- **Purpose**: Store intermediate work, drafts, analysis notes, generated data
- **Ignored by git**: The `.ai/` directory is in `.gitignore`

**Usage examples**:
- Draft documentation before finalizing
- Store benchmark results for comparison
- Keep notes about investigation/debugging sessions
- Save generated test data temporarily

Create the directory if it doesn't exist:
```bash
mkdir -p .ai/scratch
```
