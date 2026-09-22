---
name: skill-writing
description: Write or improve Codex skills for this repository. Use when creating or editing a SKILL.md file or deciding whether guidance belongs in a skill.
---

# Writing Codex Skills

## Choose the right home

- Put instructions that apply to every task in `AGENTS.md`.
- Put task-specific workflows and reference material in `.agents/skills/<name>/SKILL.md`.
- Keep each skill focused on one job. Move detailed examples or data to supporting files in the same skill directory.

## Required structure

Create a directory with a `SKILL.md` file. Include `name` and `description` in YAML frontmatter:

```yaml
---
name: example-skill
description: What this skill does and when Codex should use it.
---

# Example Skill

Follow these steps when the task matches the description.
```

Write the description so Codex can select the skill from the user's request. State both the task and the trigger. Write instructions as concrete actions with clear inputs and outputs.

## Optional files

```text
example-skill/
├── SKILL.md
├── references/
├── scripts/
├── assets/
└── agents/openai.yaml
```

Use `agents/openai.yaml` for optional display metadata, invocation policy, and tool dependencies. To require explicit invocation, set `policy.allow_implicit_invocation: false` there.

## Check the result

1. Confirm the manifest is named exactly `SKILL.md` and has both required frontmatter fields.
2. Check all referenced files and links.
3. Try a prompt that should trigger the skill and one that should not.
4. Keep examples aligned with tools and paths that actually exist in this repository.

See the [official Codex skill documentation](https://developers.openai.com/codex/skills) for current fields and discovery paths.
