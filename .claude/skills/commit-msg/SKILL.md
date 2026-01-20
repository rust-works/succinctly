---
name: commit-msg
description: Analyze git commits and generate better commit messages based on actual changes. Use when amending commit messages, reviewing commit quality, or writing commits. Triggers on terms like "commit message", "amend commit", "fix commits", "reword commits".
user-invocable: true
---

# Commit Message Skill

Generates and improves commit messages following conventional commits format.

## Conventional Commits Format

```
<type>(<scope>): <description>

[optional body]

[optional footer]
```

## Types

| Type       | Use For                                               |
|------------|-------------------------------------------------------|
| `feat`     | New feature                                           |
| `fix`      | Bug fix                                               |
| `docs`     | Documentation only                                    |
| `style`    | Formatting, no code change                            |
| `refactor` | Code change that neither fixes a bug nor adds feature |
| `perf`     | Performance improvement                               |
| `test`     | Adding or updating tests                              |
| `chore`    | Maintenance tasks                                     |
| `ci`       | CI/CD changes                                         |
| `build`    | Build system changes                                  |

## Scopes for This Project

| Scope  | Description                          |
|--------|--------------------------------------|
| bits   | BitVec, rank/select, popcount        |
| bp     | Balanced parentheses                 |
| json   | JSON semi-indexing                   |
| jq     | jq query language                    |
| simd   | SIMD implementations                 |
| cli    | CLI tool                             |
| bench  | Benchmarks                           |
| docs   | Documentation                        |

## Examples

### Feature Addition

```
feat(json): add PFSM table-driven parser

Implements parallel finite state machine approach from hw-json-simd.
Achieves 950 MiB/s throughput on x86_64 (AMD Zen 4), 40-77% faster than scalar.
```

### Bug Fix

```
fix(bp): correct find_close for edge case at word boundary

Fixes #42
```

### Performance Improvement

```
perf(popcount): add AVX-512 VPOPCNTDQ implementation

5.2x faster than scalar for large bitvectors.
Requires Intel Ice Lake+ or AMD Zen 4+.
```

### Refactoring

```
refactor(jq): replace deprecated last() with next_back()

Removes dead code and simplifies iterator handling.
```

## Instructions

1. Read the staged changes with `git diff --cached`
2. Understand what changed and why
3. Choose appropriate type and scope
4. Write a concise subject line (50 chars ideal, 72 max)
5. Add body if changes need explanation
6. Include footer for breaking changes or issue references

## Commit Message Rules

1. **Subject line**: Imperative mood ("add" not "added")
2. **No period** at end of subject line
3. **Blank line** between subject and body
4. **Body**: Explain what and why, not how
5. **Footer**: Reference issues, note breaking changes
