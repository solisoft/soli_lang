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

`/soli-resource post` runs `soli generate model post`, `soli generate migration create_posts`, `soli generate controller posts`, adds `resources("posts")` to `config/routes.sl`, and stubs the views and spec. It pauses after step 4 so you can fill in fields and validations on the model before continuing.

## Permissions

`.claude/settings.json` pre-allows the safe, read-only-or-sandboxed `soli` subcommands an agent uses constantly: `soli lint`, `soli test`, `soli serve`, `soli generate`, `soli db:migrate`, `soli run`. This removes the per-prompt approval tax without granting blanket access. Destructive things (`git push`, package mutations, anything outside the project) are deliberately left to require explicit approval.

## Migrating an existing project

To bring an older Soli project up to the new layout:

1. Run `soli new tmp-agent-kit` in a scratch directory.
2. Copy these files into your project, preserving paths:
   - `AGENTS.md`
   - `app/controllers/CLAUDE.md`, `app/models/CLAUDE.md`, `app/views/CLAUDE.md`, `app/middleware/CLAUDE.md`
   - `tests/CLAUDE.md`, `db/migrations/CLAUDE.md`
   - `.claude/settings.json`, `.claude/commands/`
3. Merge the new "For AI agents — read this first", "Footgun cheatsheet", and "Recipes" sections from `tmp-agent-kit/CLAUDE.md` into the top of your existing `CLAUDE.md`.
4. Delete `tmp-agent-kit/`.
5. Run `/soli-verify` to confirm nothing regresses.

## Customizing

The shipped files are starting points — edit them to fit project conventions. Common additions:

- **Project-specific recipes** in the root `CLAUDE.md` (e.g. "deploy: run `bin/deploy.sh`").
- **Stop hook** in `.claude/settings.local.json` (not the shipped `settings.json`) to auto-run `soli lint` after the agent stops editing.
- **Extra slash commands** under `.claude/commands/` — anything reusable enough to deserve a shortcut.

Don't store secrets or environment-specific paths in the shipped `.claude/settings.json` — that file is committed. Use `.claude/settings.local.json` (gitignored) for per-machine overrides.
