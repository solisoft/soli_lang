# AI agents in Soli projects

`soli new` scaffolds a project that's ready for AI coding agents (Claude Code, Cursor, Aider, Copilot CLI, Codex CLI, etc.) from the first commit. This page describes what ships and how to make use of it.

## What you get

Every fresh `soli new myapp` project includes:

| Path | Purpose |
|---|---|
| `CLAUDE.md` | Root agent guide — verification loop, footgun cheatsheet, recipes, MVC reference |
| `AGENTS.md` | Tool-agnostic stub pointing other agents to `CLAUDE.md` |
| `app/controllers/CLAUDE.md` | Controller-specific rules (auto-loaded models, named route helpers, mass-assignment) |
| `app/models/CLAUDE.md` | Model rules (don't override CRUD; safe `where`/`@sdbql{}` query forms) |
| `app/views/CLAUDE.md` | View rules (`<%= %>` escaping, locals, helpers, indent) |
| `app/middleware/CLAUDE.md` | Middleware shape and directive comments |
| `tests/CLAUDE.md` | BDD DSL, controller HTTP client, coverage gate |
| `db/migrations/CLAUDE.md` | Migration naming and `up`/`down` requirements |
| `.claude/settings.json` | Permissions allowlist for safe `soli` subcommands |
| `.claude/commands/soli-verify.md` | `/soli-verify` slash command — lint + test + coverage |
| `.claude/commands/soli-test.md` | `/soli-test [path]` — run one spec or full suite |
| `.claude/commands/soli-resource.md` | `/soli-resource <name>` — scaffold a full RESTful resource |

The per-directory `CLAUDE.md` files are picked up automatically by Claude Code as the agent works in that directory; you don't need to import them. Other agents read the root `CLAUDE.md` (and `AGENTS.md` as a fallback).

## The verification loop

Every agent working in a Soli project should run, before reporting a task complete:

```bash
soli lint <files-you-changed>           # naming, smells, undefined-locals
soli test tests/<the-relevant-spec>.sl  # narrow, fast feedback
soli test --coverage --coverage-min 90  # full sweep before handing off
soli serve . --dev                      # if a UI/route changed, hit it in a browser
```

The `/soli-verify` slash command bundles `soli lint` + `soli test --coverage --coverage-min 90`. If any step fails, the rule is to fix the root cause — never weaken assertions, lower the coverage gate, or skip hooks.

## Slash commands

| Command | What it does |
|---|---|
| `/soli-verify` | Runs the full pre-merge check (lint + test with coverage gate) |
| `/soli-test [path\|all]` | Runs one spec for fast feedback or the full suite with coverage |
| `/soli-resource <singular>` | Scaffolds model + migration + controller + views + route + spec |

`/soli-resource post` prefers `soli generate scaffold post` (model, controller, views, migration, routes, and a controller E2E spec under `tests/controllers/`), then `soli db:migrate up`. If you need pieces one at a time instead: hand-write the model, `soli db:migrate generate create_posts`, hand-write the controller, add `resources("posts")`, and stub views/specs. It pauses after the model/migration exist so you can fill in fields and validations before continuing.

## Permissions

`.claude/settings.json` pre-allows the safe, read-only-or-sandboxed `soli` subcommands an agent uses constantly: `soli lint`, `soli test`, `soli serve`, `soli generate`, `soli db:migrate`, `soli run`. This removes the per-prompt approval tax without granting blanket access. Destructive things (`git push`, package mutations, anything outside the project) are deliberately left to require explicit approval.

## Keeping docs up to date (`soli update docs`)

Agent guides and the language reference under `docs/` are **embedded in the
`soli` binary** at compile time (from the soli_lang git tree). After you
upgrade soli, refresh an existing project so those files match the current
release:

```bash
# From the project root
soli update docs

# Or point at a project path
soli update docs ./myapp
```

This rewrites:

- Root `CLAUDE.md` and `AGENTS.md`
- Per-directory `CLAUDE.md` files under `app/`, `tests/`, and `db/migrations/`
- `.claude/settings.json` and `.claude/commands/*`
- The whole language reference tree under `docs/` (repo-internal
  `docs/**/CLAUDE.md` files are still skipped)

**Custom edits in those paths are replaced.** Keep project-specific recipes in
a separate file you own (or re-apply them after `update docs`).
`.claude/settings.local.json` is never touched.

Typical flow after a soli upgrade:

```bash
soli update                 # self-update the CLI (no args)
soli update docs            # refresh this project's agent guides + docs/
```

## Migrating an older project (first-time agent kit)

If the project was created before agent scaffolding existed, `soli update docs`
is enough — it creates any missing host directories and writes the full kit.
Then run `/soli-verify` (or `soli lint` + `soli test`) to confirm nothing
regresses.

## Customizing

The shipped files are starting points — edit them to fit project conventions,
knowing `soli update docs` will overwrite them. Common additions that survive
an update:

- **Project-specific notes** in a file you control (e.g. `docs/project.md` or
  `AGENTS.local.md`) and a one-line pointer from root `CLAUDE.md` that you
  re-add after updates.
- **Stop hook** in `.claude/settings.local.json` (not the shipped
  `settings.json`) to auto-run `soli lint` after the agent stops editing.
- **Extra slash commands** under `.claude/commands/` with **your own names** —
  only the three `soli-*.md` commands are rewritten.

Don't store secrets or environment-specific paths in the shipped
`.claude/settings.json` — that file is committed. Use
`.claude/settings.local.json` (gitignored) for per-machine overrides.
