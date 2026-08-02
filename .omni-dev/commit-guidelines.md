# succinctly Commit Guidelines

This project follows conventional commit format with specific requirements.

## Severity Levels

| Severity | Sections                                                               |
|----------|------------------------------------------------------------------------|
| error    | Commit Format, Types, Scopes, Subject Line, Accuracy, Breaking Changes |
| warning  | Body Guidelines                                                        |
| info     | Subject Line Style                                                     |

## Commit Format

```
<type>(<scope>): <description>

[optional body]

[optional footer(s)]
```

## Types

Required. Must be one of:

| Type       | Use for                                               |
|------------|-------------------------------------------------------|
| `feat`     | New features or enhancements to existing features     |
| `fix`      | Bug fixes                                             |
| `docs`     | Documentation changes only                            |
| `refactor` | Code refactoring without behavior changes             |
| `chore`    | Maintenance tasks, dependency updates, config changes |
| `test`     | Test additions or modifications                       |
| `ci`       | CI/CD pipeline changes                                |
| `build`    | Build system or external dependency changes           |
| `perf`     | Performance improvements                              |
| `style`    | Code style changes (formatting, whitespace)           |

## Scopes

Required. `.omni-dev/scopes.yaml` is authoritative; this list mirrors it and must
be updated alongside it:

- `bitvec` - Bitvector data structure and rank/select operations
- `bp` - Balanced parentheses tree navigation
- `json` - JSON semi-indexing and parsing
- `yaml` - YAML semi-indexing and parsing
- `jq` - jq query language parser and evaluator
- `dsv` - DSV/CSV semi-indexing and parsing
- `simd` - SIMD optimizations (x86, AVX2, NEON)
- `core` - Core utilities (broadword, tables, binary ops, text/UTF-8 helpers)
- `cli` - Command-line interface tool
- `bench` - Benchmarks and performance testing
- `test` - Test suite and property tests
- `docs` - Documentation and optimization notes
- `ci` - CI/CD pipelines, GitHub Actions, and commit-lint configuration
- `build` - Build configuration and feature flags
- `scripts` - Developer and repository maintenance shell scripts

## Subject Line

- Keep under 72 characters total
- Use imperative mood: "add feature" not "added feature" or "adds feature"
- Be specific: avoid vague terms like "update", "fix stuff", "changes"

## Subject Line Style

- Use lowercase for the description
- No period at the end

## Accuracy

The commit message must accurately reflect the actual code changes:

- **Type must match changes**: Don't use `feat` for a bug fix, or `fix` for new functionality
- **Scope must match files**: The scope should reflect which area of code was modified
- **Description must be truthful**: Don't claim changes that weren't made
- **Mention significant changes**: If you add error handling, logging, or change behavior, mention it

## Body Guidelines

For significant changes (>50 lines or architectural changes), include a body:

- Explain what was changed and why
- Describe the approach taken
- Note any breaking changes or migration requirements
- Use bullet points for multiple related changes
- Reference issues in footer: `Closes #123` or `Fixes #456`

## Breaking Changes

For breaking changes:
- Add `!` after type/scope: `feat(api)!: change response format`
- Include `BREAKING CHANGE:` footer with migration instructions

## Examples

### Simple change
```
fix(cli): handle missing config file gracefully
```

### Feature with body
```
feat(yaml): stream YAML directly to JSON output

Eliminates the intermediate OwnedValue DOM by streaming directly from
the YAML cursor to JSON, avoiding a full in-memory materialization
pass before output.

- Add single-pass escape transcoding for string values
- Wire the streaming path into the yq CLI identity query
- Update benchmarks to reflect the new throughput

Closes #12
```

### Breaking change
```
feat(jq)!: change tostream event shape to match jq's [path,value] form

BREAKING CHANGE: tostream now emits jq's standard [path,value]/[path]
event pairs instead of the previous non-standard shape. Update callers
that pattern-match on tostream output.
```
