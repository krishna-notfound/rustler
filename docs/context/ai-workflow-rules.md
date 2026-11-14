# Development Workflow

## Approach

Build incrementally with a spec-driven workflow. Context files define what to build and how;
`docs/features/` files define each unit of work; `progress-tracker.md` defines current state.
Implement against these specs — do not infer or invent behavior.

## Order of Work

1. Brainstorm and decide (record decisions in `progress-tracker.md` → Architecture Decisions).
2. Write the feature file `docs/features/fNN_<name>.md` **before any code** for that feature.
3. Write the plan/tasks. 4. Implement. 5. Verify. 6. Update docs and tracker.

## Scoping Rules

- One feature unit or subsystem at a time.
- Prefer small, verifiable increments over large speculative changes.
- Do not combine unrelated system boundaries in one implementation step.

## When To Split Work

Split a step if it combines:

- UI changes and data/storage changes
- Unrelated routes or modules
- Behavior not clearly defined in the context or feature files

If a change cannot be verified end to end quickly, the scope is too broad — split it.

## Handling Missing Requirements

- Do not invent product behavior.
- Ambiguous requirement → resolve it in the relevant context file before implementing.
- Missing requirement → add it to *Open Questions* in `progress-tracker.md` and continue with
  work that is not blocked.

## Parallel Work

- Use one git worktree + branch per agent; assign disjoint files (`Owned files` in the feature file).
- Freeze shared contracts (types, interfaces, shells, tokens) first; agents consume, never change.
- Agents report real command output; the orchestrator reviews before merging.
- Choose the model per task: security-critical or contract work by the orchestrator, repetitive
  UI/CRUD by cheaper agents, always reviewed.

## Protected Foundation Components

Do not modify generated third-party components (`{{PROTECTED_PATHS}}`) unless the task explicitly
requires it. Put project-specific styling and logic in app-level components.

## Keeping Docs In Sync

Update the matching context file whenever work changes architecture, storage, standards, or
scope. Progress state must reflect the **actual** state, not the intended state.

## Before Moving To The Next Unit

1. The current unit works end to end within its defined scope.
2. No invariant defined in `architecture-context.md` was violated.
3. Verification commands were run and their real results seen.
4. `progress-tracker.md` and the feature file *Status* reflect the completed work.
