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

Required. See [`.omni-dev/scopes.yaml`](scopes.yaml) for the canonical, enforced
list — do not duplicate it here. `omni-dev` also silently accepts a handful of
ecosystem-default scopes not listed there (for this repo: `cargo`, `lib`); run
`omni-dev git commit message lint --verbose` to see the full resolved set.

## Subject Line

- Keep under 100 characters total (enforced by `.omni-dev/commit-rules.yaml`'s
  `subject_max_len`; `omni-dev git commit message lint` is the source of truth if
  this drifts)
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
