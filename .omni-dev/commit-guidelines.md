# omni-dev Commit Guidelines

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

Required. Use scopes defined in `.omni-dev/scopes.yaml`:

- `ci` - CI/CD pipelines and GitHub Actions workflows
- `claude` - AI client implementation and integration
- `cli` - Command-line interface and argument parsing
- `git` - Git operations and repository analysis
- `data` - Data structures and serialization
- `docs` - Documentation and planning
- `api` - External API integrations
- `workflows` - GitHub Actions workflow files

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
feat(claude): add contextual intelligence for commit message improvement

Implements Phase 3 of the twiddle command enhancement with multi-layer
context discovery including project conventions, branch analysis, and
work pattern detection.

- Add project context discovery from .omni-dev/ configuration
- Implement branch naming pattern analysis
- Add work pattern detection across commit ranges
- Enhance Claude prompting with contextual intelligence

Closes #12
```

### Breaking change
```
feat(api)!: change amendment response format to YAML

BREAKING CHANGE: The amendment API now returns YAML instead of JSON.
Update clients to use a YAML parser for response handling.
```
