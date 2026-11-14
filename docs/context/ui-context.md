# UI Context

## Theme

{{Light only / dark only / both. One sentence describing the visual language.}}

All colors are CSS custom properties defined in `{{globals.css}}` and mapped to Tailwind tokens.
Components use tokens only — no hardcoded hex values and no raw palette classes (e.g. `zinc-*`).

| Role | CSS Variable | Value |
| ---- | ------------ | ----- |
| Page background | `--{{...}}` | `{{#hex}}` |
| Surface | `--{{...}}` | `{{#hex}}` |
| Primary text | `--{{...}}` | `{{#hex}}` |
| Muted text | `--{{...}}` | `{{#hex}}` |
| Border | `--{{...}}` | `{{#hex}}` |
| Brand | `--{{...}}` | `{{#hex}}` |
| Accent | `--{{...}}` | `{{#hex}}` |
| Error / Success / Warning | `--{{...}}` | `{{#hex}}` |

Tailwind utility names for the tokens: `{{bg-..., text-..., border-...}}`.

## Typography

| Role | Font | CSS Variable |
| ---- | ---- | ------------ |
| UI text | {{font}} | `{{--font-...}}` |
| Mono | {{font}} | `{{--font-...}}` |

Scale: {{headings / body / caption sizes and weights}}

## Spacing, Radius, Shadow

| Context | Class / Value |
| ------- | ------------- |
| Inline / small | {{rounded-xl}} |
| Cards | {{rounded-2xl}} |
| Modals | {{rounded-3xl}} |
| Shadow | {{soft shadow definition}} |

## Component Library

{{shadcn/ui or other}}. Components live in `{{components/ui/}}` and are **protected** — do not edit
them; compose or wrap them in app-level components.

## Layout Patterns

- {{Shell per area: public, app, admin ...}}
- {{Navigation pattern, mobile behaviour}}
- Breakpoints: {{values}}

## Required UI States

Every data-driven view implements: loading skeleton, empty state, error state, success feedback,
and confirmation for destructive actions.

## Accessibility

- {{Keyboard navigation, focus rings, contrast target, label requirements}}

## Icons

{{Icon library and sizing rules.}}

## Content / Copy Rules

- {{Tone, headline, terminology, anything the UI must say verbatim}}
