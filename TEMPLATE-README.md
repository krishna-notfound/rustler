# Project Context Template

A project-agnostic starter for spec-driven work with Claude Code. Drop it into a new repo so every
session starts with the same verified knowledge.

## What is inside

```text
CLAUDE.md                      Loaded every session: read order, source-of-truth rules,
                               anti-hallucination rules, parallel-work rules, commands
context/
  AGENTS.md                    Framework-version warning + read order
  project-overview.md          Product, roles, flows, scope, business rules, success criteria
  architecture-context.md      Stack, boundaries, data model, data access, invariants, risks
  ui-context.md                Theme tokens, typography, layout, required UI states
  code-standards.md            Implementation rules and definition of done
  ai-workflow-rules.md         How work is scoped, split, parallelised, verified
  progress-tracker.md          Current state, open questions, decisions, deferred work
features/
  README.md                    Feature index + status
  _feature-template.md         Copy to features/fNN_<about_it>.md for each feature
```

## How to use for a new project

1. Unzip into the new repo root (you get `CLAUDE.md`, `context/`, `features/`).
2. Replace every `{{PLACEHOLDER}}` in `CLAUDE.md` and `context/*.md`. Delete sections that do not apply.
3. Keep `context/AGENTS.md` only if the framework moves fast; otherwise delete its warning block.
4. Create one `features/fNN_<about_it>.md` per feature **before** writing any code.
5. Start every Claude Code session normally — `CLAUDE.md` tells it what to read.
6. Keep `context/progress-tracker.md` truthful after every meaningful change.

Search for `{{` to find unfilled placeholders.
