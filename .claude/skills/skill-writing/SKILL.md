---
name: skill-writing
description: Best practices for writing and structuring Claude Code skills. Use when creating new skills, improving existing skills, or asking how to structure a SKILL.md file.
---

# Writing Effective Claude Code Skills

## Core Principle: Progressive Disclosure

The most important concept for building skills is **progressive disclosure** — show just enough information to help Claude decide what to do next, then reveal more details as needed.

- Keep `SKILL.md` under 500 lines
- Move detailed reference material to separate files
- Reference supporting files so Claude knows when to load them

## Skill File Structure

Every skill needs a `SKILL.md` file with two parts:

```yaml
---
name: my-skill-name
description: What this skill does and when to use it
---

# Skill Title

[Instructions Claude follows when the skill is invoked]

## Examples
## Guidelines
```

### Directory Structure

```
my-skill/
├── SKILL.md           # Main instructions (required)
├── reference.md       # Detailed docs (loaded when needed)
├── examples/          # Example outputs
└── scripts/           # Executable scripts
```

## Frontmatter Reference

| Field                      | Purpose                                                    |
|----------------------------|------------------------------------------------------------|
| `name`                     | Slash command name (lowercase, hyphens, max 64 chars)      |
| `description`              | **Recommended** - helps Claude decide when to use skill    |
| `argument-hint`            | Autocomplete hint, e.g., `[issue-number]`                  |
| `disable-model-invocation` | `true` = only user can invoke (for deploy, commit, etc.)   |
| `user-invocable`           | `false` = hide from `/` menu (background knowledge)        |
| `allowed-tools`            | Tools Claude can use without permission when skill active  |
| `model`                    | Override model for this skill                              |
| `context`                  | `fork` = run in isolated subagent                          |
| `agent`                    | Subagent type when `context: fork` (`Explore`, `Plan`)     |

## Types of Skill Content

### Reference Skills (Knowledge)
Add conventions, patterns, or domain knowledge Claude applies to current work:

```yaml
---
name: api-conventions
description: API design patterns for this codebase
---

When writing API endpoints:
- Use RESTful naming conventions
- Return consistent error formats
```

### Task Skills (Actions)
Step-by-step instructions for specific actions. Usually user-invoked only:

```yaml
---
name: deploy
description: Deploy the application to production
context: fork
disable-model-invocation: true
---

Deploy the application:
1. Run the test suite
2. Build the application
3. Push to deployment target
```

## String Substitutions

| Variable               | Description                                      |
|------------------------|--------------------------------------------------|
| `$ARGUMENTS`           | All arguments passed to skill                    |
| `$ARGUMENTS[N]` / `$N` | Specific argument by 0-based index               |
| `${CLAUDE_SESSION_ID}` | Current session ID                               |
| `!`command``           | Shell command output (runs before skill loads)   |

## Invocation Control Matrix

| Frontmatter                      | User | Claude | When to Use                        |
|----------------------------------|------|--------|------------------------------------|
| (default)                        | Yes  | Yes    | General-purpose skills             |
| `disable-model-invocation: true` | Yes  | No     | Side effects (deploy, commit)      |
| `user-invocable: false`          | No   | Yes    | Background knowledge               |

## Best Practices

### DO
- Use active, directive language ("Run tests", not "Tests should be run")
- Break complex tasks into numbered sequential steps
- Include concrete examples showing expected format
- Specify output formats when consistency matters
- Test skills with both direct invocation and natural language

### DON'T
- Put reference content in task skills (use `context: fork` instead)
- Create skills for one-off tasks (just ask Claude directly)
- Exceed 500 lines in SKILL.md (use supporting files)
- Use `disable-model-invocation` for reference skills

## When to Create a Skill

Create a skill when:
- You type the same prompt repeatedly across conversations
- Team knowledge should be codified and shared
- A workflow has specific steps that must be followed
- You want Claude to automatically apply certain patterns

Use CLAUDE.md instead when:
- Instructions apply to the entire project always
- Context is needed for every conversation
- Settings are project-wide conventions

## Skill Locations

| Location   | Path                                 | Scope                  |
|------------|--------------------------------------|------------------------|
| Personal   | `~/.claude/skills/<name>/SKILL.md`   | All your projects      |
| Project    | `.claude/skills/<name>/SKILL.md`     | This project only      |
| Enterprise | Managed settings                     | All org users          |

## Example: Well-Structured Skill

```yaml
---
name: review-pr
description: Review a pull request for code quality, security, and best practices
argument-hint: [pr-number]
context: fork
agent: Explore
allowed-tools: Bash(gh *)
---

# Pull Request Review

Review PR #$ARGUMENTS for:

## 1. Code Quality
- Readability and maintainability
- Consistent naming conventions
- Appropriate abstractions

## 2. Security
- Input validation
- Authentication/authorization
- No secrets in code

## 3. Testing
- Adequate test coverage
- Edge cases handled

## Context
- PR diff: !`gh pr diff $ARGUMENTS`
- PR description: !`gh pr view $ARGUMENTS`

Provide actionable feedback with specific line references.
```

## Additional Resources

For the official specification, see [Agent Skills](https://agentskills.io).
For Claude Code documentation, see [Extend Claude with skills](https://code.claude.com/docs/en/skills).
