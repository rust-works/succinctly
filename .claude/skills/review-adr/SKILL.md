---
name: review-adr
description: Review Architecture Decision Records (ADRs) for structural accuracy, project conformance, and software engineering best practice. Works with existing project ADRs and with proposed ADRs from issues or other sources. Triggers on terms like "review ADR", "check ADR", "evaluate ADR", "ADR review".
argument-hint: [adr-number|all|issue]
allowed-tools: Read, Grep, Glob, Bash(git log *)
---

# ADR Review Skill

Review one or more Architecture Decision Records (ADRs) across four dimensions:
structural accuracy, project conformance, decision quality, and implementation quality.

## Step 0: Determine What to Review

Parse `$ARGUMENTS` to determine the review mode:

| Argument                             | Action                                                           |
|--------------------------------------|------------------------------------------------------------------|
| A number (`5`, `05`, `0005`)         | Review `docs/adrs/adr-NNNN.md` in the current project            |
| A filename or path                   | Review that specific ADR file                                    |
| `all`                                | Review every `adr-*.md` in `docs/adrs/`                          |
| `issue` or a GitHub issue URL        | Review a proposed ADR pasted or fetched from the issue           |
| No argument or an ADR in the chat    | Ask the user what to review, or use the ADR already in context   |

If the argument is a GitHub issue URL, use `gh issue view <number> --repo <owner/repo>` to
fetch the issue body, then extract the ADR text from it.

## Step 1: Load the ADR Standard

Read the project's foundational ADR to understand the expected format and conventions:

1. Look for `docs/adrs/adr-0000.md` in the current working directory.
2. If found, read it — it is the project's authoritative definition of ADR structure,
   purpose, naming, and storage conventions.
3. If not found, fall back to the Michael Nygard baseline:
   - **Title** — short noun phrase, `ADR-NNNN: <title>`
   - **Status** — one of Proposed / Accepted / Deprecated / Superseded
   - **Context** — the forces at play; why a decision was needed
   - **Decision** — active voice ("We will ..."); the response to those forces
   - **Consequences** — positive, negative, and neutral outcomes
   - Stored in version control alongside the code; one decision per record; 1–2 pages

## Step 2: Load the ADR(s) to Review

**Existing ADRs:**
- Read the target ADR file(s) using the Read tool.
- For `all` mode, use `Glob("docs/adrs/adr-*.md")` then read each file.

**Proposed / issue ADRs:**
- If the ADR text is already in the conversation, use it directly.
- If the user passed a GitHub issue URL or number, fetch the issue with `gh issue view`.
- Otherwise, ask the user to paste the ADR content.

## Step 3: Gather Project Context

Collect supporting context needed for the project-conformance and implementation checks.

**For existing ADRs** (code is available):
1. Note every file path, module, crate, function, command, or configuration key mentioned
   in the ADR.
2. Read those source files with the Read tool to verify the implementation.
3. Use Grep to locate code patterns or identifiers referenced in the ADR.
4. Skim other ADRs in `docs/adrs/` for cross-ADR consistency; read any that are closely
   related to the ADR under review.

**For proposed ADRs** (no implementation yet):
- If inside a project, check whether referenced paths, patterns, or conventions exist.
- Clearly note in the review what cannot be verified without an implementation.

## Step 4: Apply the Four Review Dimensions

Evaluate the ADR against all four dimensions. Record findings for each before writing the
report.

---

### Dimension 1: Structural Accuracy

Conformance to the format defined in ADR-0000 (or the Michael Nygard baseline):

- **Title** — Present as H1? Format `ADR-NNNN: <short noun phrase>`? Concise and
  descriptive? Does the number match the filename (for existing ADRs)?
- **Status** — Present as H2? Valid status value? Appropriate emoji if the project uses
  them (check ADR-0000 and other ADRs for the convention)?
- **Context** — Present as H2? Explains the forces at play, not just background noise?
  Makes clear *why* a decision was needed at this time?
- **Decision** — Present as H2? Written in active voice ("We will ...")? Specific and
  unambiguous? Covers a single decision, not several rolled together?
- **Consequences** — Present as H2? Covers positive, negative, and neutral outcomes?
  Honest about drawbacks and risks, not just the upsides?
- **Length** — Roughly one to two pages? Enough detail without padding or sprawl?
- **Scope** — One decision per record? Not conflating unrelated choices?
- **File** — Named `adr-NNNN.md` with a zero-padded four-digit number (for existing ADRs)?
  Stored in the project's designated ADR directory?

---

### Dimension 2: Project Conformance

Whether the ADR accurately reflects the actual project:

- **Accuracy** — Do stated file paths, module names, functions, crates, commands, and
  configuration keys actually exist in the codebase?
- **Implementation match** — Does the described design correspond to what the code actually
  does? Are there gaps between the ADR and the implementation?
- **Cross-ADR consistency** — Does this ADR contradict or silently overlap with any other
  ADR? If it supersedes an earlier decision, is that relationship explicitly noted?
- **Terminology** — Does it use the project's established terminology and naming conventions
  consistently with other ADRs and project documentation?
- **Scope fit** — Is the decision scope appropriate to the project's scale and architecture?
  (Not over-engineering for a small tool; not under-specifying a genuinely complex area.)
- **Freshness** — For existing ADRs: does the content still accurately describe the current
  state of the project, or has it drifted from the codebase since it was written?

---

### Dimension 3: Decision Quality

Whether the decision itself reflects software engineering best practice:

- **Problem definition** — Is the problem clearly articulated? Does the Context section
  explain *why* a decision is needed, with enough specificity to justify the ADR?
- **Alternatives** — Were meaningful alternatives evaluated? Are the trade-off analyses
  fair and balanced, not strawmen? Does the chosen option emerge naturally from the analysis?
- **Justification** — Is the chosen option clearly motivated by the context? Or does it feel
  arbitrary, or like it was chosen first and the rationale retrofitted?
- **Scope** — Is the decision appropriately scoped? Not solving a problem that doesn't exist;
  not ignoring real complexity; not deferring decisions that should be made now.
- **Reversibility** — Is the permanence of the decision appropriate? Are the risks of getting
  it wrong acknowledged? Is there a path to revisiting it if circumstances change?
- **SE principles** — Does the decision align with recognised software engineering principles
  where applicable: separation of concerns, fail-fast, defence in depth, least surprise,
  single responsibility, DRY, YAGNI, etc.? Any clear violations that should be flagged?
- **Evolution** — Does the ADR acknowledge known future decisions it may require, or areas
  where the design is intentionally left open?

---

### Dimension 4: Implementation Quality

For **existing ADRs**: whether the referenced code or processes satisfy SE best practice
within the ADR's scope.

- **Correctness** — Does the implementation actually do what the ADR claims?
- **Safety** — Are edge cases, error paths, and failure modes handled? Does the code match
  the ADR's safety or reliability claims?
- **Maintainability** — Is the code readable and structured for future modification? Is
  logic confined to the modules the ADR identifies?
- **Testability** — Is the approach testable? Are tests present where the ADR implies they
  should be? Do tests cover the failure modes the ADR acknowledges?
- **Performance** — If the ADR makes performance claims or states that overhead is
  negligible, are those claims credible given the implementation?
- **Security** — Does the implementation introduce risks not acknowledged by the ADR?
- **SE principles** — Does the code follow good practices: single responsibility, defensive
  programming, appropriate abstraction level, no unnecessary coupling?

For **proposed ADRs** (no implementation yet): evaluate whether the described approach is
likely to result in high-quality implementation, and flag foreseeable pitfalls, complexity
risks, or SE concerns that the proposer should address before accepting.

---

## Step 5: Produce the Review Report

Write the review in the following format. Be specific and actionable — vague feedback is
not useful. If a dimension has no issues, say so briefly and move on.

```
## ADR Review: [ADR-NNNN or "Proposed ADR"]

**Subject**: [The title, or a one-line summary of what the ADR decides]
**Source**: [File path, or "Issue #N", or "Proposed — no implementation yet"]

### Overall Verdict

[✅ Pass | ⚠️ Minor Issues | ❌ Major Issues]

[1–3 sentences summarising the most important findings and the overall quality of the ADR.]

---

### Dimension 1: Structural Accuracy — [✅ | ⚠️ | ❌]

[Findings. State clearly what is correct. Call out every structural problem with a specific,
actionable correction. Quote the problematic text where helpful.]

### Dimension 2: Project Conformance — [✅ | ⚠️ | ❌]

[Findings. Name specific inaccuracies, path mismatches, cross-ADR conflicts, or terminology
drift. For proposed ADRs, note what could not be verified and why.]

### Dimension 3: Decision Quality — [✅ | ⚠️ | ❌]

[Findings. Identify missing alternatives, unjustified choices, scope problems, or SE
principle concerns. Be direct about weak reasoning without being dismissive.]

### Dimension 4: Implementation Quality — [✅ | ⚠️ | ❌ | N/A — Proposed]

[Findings. For existing ADRs: identify gaps between ADR claims and the actual code, or
code-quality issues within the ADR's scope. For proposed ADRs: flag design risks,
complexity traps, or feasibility concerns.]

---

### Recommendations

[Numbered list of actionable improvements, most important first. Omit this section entirely
if there are no significant issues.]
```

---

## Branch Naming

When creating a branch to work on an ADR — whether implementing a proposed ADR from an
issue, amending an existing ADR, or adding a new one — the branch name must begin with
`adr/`. Examples:

- `adr/0014-template-resolution` — adding a new ADR
- `adr/issue-42-config-layering` — working on a proposed ADR from an issue
- `adr/amend-0005-walk-up-discovery` — amending an existing ADR

If the user asks to create a branch or start work on an ADR, apply this prefix automatically.

---

## Reviewing All ADRs (`all` mode)

When reviewing the entire ADR inventory:

1. Review each ADR individually using the four dimensions above.
2. After individual reviews, add a cross-cutting analysis covering:
   - **Coverage gaps** — Are major architectural areas of the project undocumented? Name
     specific areas that warrant an ADR but lack one.
   - **Cross-ADR consistency** — Contradictions, overlapping scope, or silently superseded
     decisions that have not been marked Superseded/Deprecated.
   - **Hygiene** — ADRs whose status no longer reflects reality; ADRs that have drifted from
     the current codebase.
3. Open the report with a summary table before the individual reviews:

```markdown
| ADR      | Title                        | Overall | Dim 1 | Dim 2 | Dim 3 | Dim 4 |
|----------|------------------------------|---------|-------|-------|-------|-------|
| ADR-0001 | YAML as Primary Data Format  | ✅      | ✅    | ✅    | ✅    | ✅    |
| ADR-0005 | Hierarchical Config ...      | ⚠️      | ✅    | ⚠️    | ✅    | ✅    |
```
