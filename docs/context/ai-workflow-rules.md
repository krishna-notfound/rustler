# Development Workflow

## Approach

Build incrementally with a spec-driven workflow. The approved design spec and plan define what to build; the
feature files (`docs/features/fNN_*.md`) define each unit of work; `docs/progress-log.md` is the record of what
is actually built and tested. Implement against these documents — do not infer or invent behavior.

## Order of Work

1. Brainstorm and decide. Record decisions in `docs/progress-log.md` (dated) and, if they change architecture or scope, in `docs/context/`.
2. Write the feature file `docs/features/fNN_<name>.md` **before any code** for that feature.
3. Write the plan and tasks (in the feature file or the superpowers plan).
4. Implement, test-first where practical.
5. Verify: run the commands and record the real result.
6. Update the feature file *Status Log*, `docs/features/README.md` and `docs/progress-log.md`.

## Scoping Rules

- One feature unit or subsystem at a time.
- Prefer small, verifiable increments over large speculative changes.
- Do not combine unrelated crates in one implementation step.
- Time-boxed work follows the "5-HOUR SCOPE" section of the plan, which overrides the slice ordering.

## When To Split Work

Split a step if it combines:

- Engine work and Rust command work (they share the JSON-RPC contract; change `PROTOCOL.md` first)
- Unrelated crates or command groups
- Behavior not clearly defined in the feature file or the design spec

If a change cannot be verified end to end quickly, the scope is too broad — split it.

## Handling Missing Requirements

- Do not invent product behavior.
- Ambiguous requirement → resolve it in the design spec or `docs/00-vision-and-requirements.md` first.
- Missing requirement → add it to *Open Questions* in `docs/progress-log.md` and continue with work that is not blocked.

## Parallel Work

- One git worktree and branch per agent; disjoint files, listed as `Owned files` in each feature file.
- Freeze shared contracts first (core models, registry, `PROTOCOL.md`). Agents consume them, never change them.
- Agents report real command output; the orchestrator reviews before merging.
- Model tiering (plan execution notes): contracts, render compiler and end-to-end work by the strongest model;
  feature crates by a mid-tier model; docs and feature files by the cheapest model; all reviewed.
- Agent commits end with the `Co-Authored-By` trailer the user's rules specify; do not push.

## Frozen Foundation

Do not change these unless the task explicitly requires it, and record the decision first:

- `crates/rustler-core` models, ids, errors and events
- `crates/rustler-registry` (`Command`, `typed()`, envelope)
- `engine/PROTOCOL.md` method payloads

Put project-specific logic in the crate that owns the responsibility, not in shared contracts.

## Keeping Docs In Sync

All project documentation lives in `docs/`. Update it after every merge: the progress log, the feature file, the
feature index, and the matching context file when architecture, storage, standards or scope change.
Progress state must reflect the **actual** state, not the intended state.

## Before Moving To The Next Unit

1. The current unit works end to end within its defined scope.
2. No invariant in `architecture-context.md` was violated.
3. Verification commands were run and their real results seen.
4. `docs/progress-log.md` and the feature file *Status Log* reflect the completed work.
