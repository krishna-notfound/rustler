# {{PROJECT_NAME}} — Claude Code Instructions

> {{ONE_LINE_PRODUCT_DESCRIPTION}}

This file is loaded into every session. It exists so each session starts with the same
verified knowledge and does not guess. **Read it fully, then follow the session start protocol.**

## 1. Session Start Protocol (do this first, every session)

1. Read the context files **in this order** (stop only when you have read all of them):
   1. `docs/context/project-overview.md` — what the product is, goals, scope, success criteria
   2. `docs/context/architecture-context.md` — stack, boundaries, data model, invariants
   3. `docs/context/ui-context.md` — theme, tokens, typography, component conventions
   4. `docs/context/code-standards.md` — implementation rules
   5. `docs/context/ai-workflow-rules.md` — how work is scoped, split, verified, delivered
   6. `docs/context/progress-tracker.md` — **current state**, what is done, what is next, open questions
2. Open `docs/features/README.md` (index + status). Then read **only** the feature file(s) for the
   task at hand (`docs/features/fNN_<name>.md`).
3. If `AGENTS.md` exists, obey it too (it contains framework-version warnings).
4. State in one or two sentences what you understand the current task and phase to be
   before you change anything.

## 2. Source of Truth Hierarchy

When two sources disagree, the higher one wins:

1. The user's latest explicit instruction in this session
2. `docs/features/fNN_*.md` for the feature being built
3. `docs/context/*.md`
4. Code that already exists in the repo
5. Your own memory or training data (**lowest — never overrides the above**)

## 3. Anti-Hallucination Rules

- **Never invent** product behavior, API shapes, table/column names, env var names, package
  versions, or library APIs. If it is not in the docs/context/feature files or verified in the repo, it
  is unknown.
- **Verify before you cite**: before naming a file, function, route, component, or flag, confirm it
  exists (Glob/Grep/Read). Quote paths as `path/to/file.ts:line`.
- **Verify library APIs against the installed version**: read `node_modules/<pkg>/` docs or types.
  Framework majors change fast; training data is stale.
- **Unknown or ambiguous → write it down, do not guess**: add it to *Open Questions* in
  `docs/context/progress-tracker.md`, ask the user if it blocks, otherwise continue with other work.
- **Say what you did not verify.** "Built and lint passes" is not "works". Run it, look at it,
  report the real result. Failing output is reported, not hidden.
- **No fake functionality.** No buttons, numbers, or states that look real but do nothing.
  Demo data must live in one clearly labelled place (see `ui-context.md` / `architecture-context.md`).
- **Do not expand scope.** Build what the active feature file says, nothing adjacent.

## 4. Working Rules

- Spec-driven: **no code for a feature until `docs/features/fNN_*.md` exists and is filled in.**
- One feature unit at a time; small, verifiable increments (see `docs/context/ai-workflow-rules.md`).
- Protected foundation components (`{{PROTECTED_PATHS}}`) are not edited unless the task says so.
- After each meaningful change: update `docs/context/progress-tracker.md` and the feature file's
  *Status* section. If architecture/scope/standards changed, update the matching context file
  **before** continuing.
- Commit only when the user asks (or when the plan says a task ends in a commit).

## 5. Parallel Work (worktrees + sub-agents)

- Each parallel agent gets its **own git worktree and branch** and owns a **disjoint set of
  files** listed in its feature file (`Owned files`). Agents never edit files they do not own.
- Shared contracts (types, data-access interfaces, layout shells, design tokens) are written and
  frozen **before** agents start. An agent that needs a contract change stops and reports it.
- Every agent reports: files created/changed, commands run + real results, open questions.
- The orchestrator reviews and merges; nothing merges without lint, type-check and build passing.

## 6. Commands

| Purpose | Command |
| ------- | ------- |
| Install | `{{INSTALL_CMD}}` |
| Dev server | `{{DEV_CMD}}` |
| Lint | `{{LINT_CMD}}` |
| Type check | `{{TYPECHECK_CMD}}` |
| Test | `{{TEST_CMD}}` |
| Build | `{{BUILD_CMD}}` |

## 7. Repository Map

```text
{{REPO_TREE}}
```

## 8. Project Snapshot

- **Stack:** {{STACK_SUMMARY}}
- **Current phase:** see `docs/context/progress-tracker.md`
- **Deferred / out of scope right now:** {{DEFERRED}}

## 9. Do Not

- Do not commit `.env*` files or secrets. Do not expose server-only keys to the browser.
- Do not change the stack, add major dependencies, or restructure folders without recording the
  decision in `progress-tracker.md` → *Architecture Decisions* first.
- Do not leave TODOs for core functionality.
