# Code Standards

## General

- Keep modules small and single-purpose.
- Fix root causes — do not layer workarounds.
- Do not mix unrelated concerns in one component or route.
- Respect the boundaries defined in `architecture-context.md`.
- No dead code, no commented-out code, no TODOs for core functionality.
- Match the style, naming, and comment density of surrounding code.

## Language ({{e.g. TypeScript}})

- {{Strict mode required.}}
- {{Avoid `any`; use explicit types.}}
- {{Validate unknown external input at system boundaries.}}

## Framework ({{e.g. Next.js}})

- {{Default to server components; mark client components only when interactivity needs it.}}
- {{Route handler / action rules.}}

## Styling

- Tokens only (see `ui-context.md`). No hardcoded colors.
- Radius, spacing and shadow follow the scales in `ui-context.md`.
- Protected components (`{{PROTECTED_PATHS}}`) are never edited.

## Data and State

- {{Where data access lives and how components reach it.}}
- {{Server vs client state rules.}}

## Naming and File Organization

- Name files after the responsibility they contain, not the technology.
- {{Folder responsibilities.}}

## Testing

- {{What must have tests, tools, where tests live, naming.}}
- A feature is not done until its tests pass **and** the result was observed, not assumed.

## Definition of Done (per unit of work)

1. Works end to end within the feature file's scope.
2. {{Lint}}, {{type-check}}, {{tests}}, {{build}} all pass — real output seen.
3. No invariant in `architecture-context.md` violated.
4. Feature file *Status* and `progress-tracker.md` updated.
