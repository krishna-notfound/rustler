# Architecture Context

## Stack

| Layer | Technology | Role |
| ----- | ---------- | ---- |
| Framework | {{e.g. Next.js + TypeScript}} | {{role}} |
| UI | {{e.g. Tailwind + shadcn/ui}} | {{role}} |
| Auth | {{provider}} | {{role}} |
| Database | {{db}} | {{role}} |
| Hosting | {{host}} | {{role}} |

Verified versions (fill from `package.json`, never from memory): {{name@version, ...}}

## System Boundaries

> One line per folder/module: what it owns and what it must not do.

- `{{path}}` — {{responsibility}}

## Data Model

> Entities, key fields, relationships, constraints. Source of truth for naming.

| Entity | Key fields | Relations | Notes |
| ------ | ---------- | --------- | ----- |
| {{Entity}} | {{fields}} | {{relations}} | {{notes}} |

## Data Access Layer

- How the UI reads/writes data: {{interface / service / API}}
- Current implementation: {{e.g. demo data in `lib/data/demo/`}}
- Planned implementation: {{e.g. Supabase}}
- Rule: components import only from {{interface path}}, never from the implementation.

## Auth and Authorization Model

- Identity provider: {{provider}}
- Roles and where they are stored: {{where}}
- Where authorization is enforced: {{every mutation boundary / RLS / middleware}}

## Storage Model

- {{What goes in the database}}
- {{What goes elsewhere}}

## Invariants

> Rules that must never be broken. Check these before finishing any unit of work.

1. {{Invariant}}

## Known Risks

| Risk | Impact | Mitigation |
| ---- | ------ | ---------- |
| {{Risk}} | {{impact}} | {{mitigation}} |
