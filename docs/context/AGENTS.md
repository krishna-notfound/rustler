<!-- BEGIN:framework-agent-rules -->

# Framework version warning

{{FRAMEWORK_NAME}} {{FRAMEWORK_VERSION}} may have breaking changes — APIs, conventions, and file
structure may differ from your training data. Before writing code, read the relevant guide in
`{{FRAMEWORK_DOCS_PATH}}` (for Next.js: `node_modules/next/dist/docs/`). Heed deprecation notices.
Delete this block if the project has no fast-moving framework.

<!-- END:framework-agent-rules -->

## Application Building Context

Read the following files in order before implementing or making any architectural decision:

1. `docs/context/project-overview.md` — product definition, goals, features, and scope
2. `docs/context/architecture-context.md` — system structure, boundaries, storage model, and invariants
3. `docs/context/ui-context.md` — theme, colors, typography, layout, and component conventions
4. `docs/context/code-standards.md` — implementation rules and conventions
5. `docs/context/ai-workflow-rules.md` — development workflow, scoping rules, and delivery approach
6. `docs/context/progress-tracker.md` — current phase, completed work, open questions, and next steps
7. `docs/features/README.md`, then only the `docs/features/fNN_*.md` file(s) for the current task

Update `docs/context/progress-tracker.md` after each meaningful implementation change.

If implementation changes the architecture, scope, or standards documented in the context files,
update the relevant file before continuing.
