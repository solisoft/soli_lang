# Soli Projects Are Designed for AI Coding Agents

Most web frameworks were never designed with AI coding agents in mind.

When you point Claude Code, Cursor, or Aider at a typical Rails, Laravel, or Next.js codebase, the agent immediately struggles with:

- Dozens of ways to do the same thing
- Massive, conflicting context from years of accumulated libraries and patterns
- No reliable way to know whether the change it just made is safe
- Having to ask the human "should I run the tests now?" every single time

The result is slow, brittle, high-supervision collaboration. The agent produces code that looks plausible but frequently violates local conventions, breaks tests, or introduces subtle bugs that only surface in production.

Soli takes the opposite approach.

From the very first `soli new myapp`, the project is deliberately engineered to be an excellent environment for AI agents. The same constraints that make Soli feel "opinionated" to humans make it dramatically more reliable for agents.

## What Ships on Day One

Every new Soli project includes a complete, layered agent support system:

| Artifact | Purpose |
|----------|---------|
| `CLAUDE.md` (root) | The master agent guide — verification loop, footgun cheatsheet, common recipes, project structure |
| `AGENTS.md` | Tool-agnostic pointer so Aider, Copilot CLI, Codex, etc. find the right instructions |
| `app/controllers/CLAUDE.md` | Controller-specific rules (auto-loaded models, named route helpers, thin controllers) |
| `app/models/CLAUDE.md` | Model rules (never override CRUD, safe `where` forms, when to use `@sdbql{}`) |
| `app/views/CLAUDE.md` | View rules (escaping, `locals`, partial conventions, indentation) |
| `app/middleware/CLAUDE.md` | Middleware ordering, `// order:` and `// scope_only:` directives |
| `tests/CLAUDE.md` | BDD DSL, HTTP client helpers, coverage requirements |
| `db/migrations/CLAUDE.md` | Migration naming and `up`/`down` contract |
| `.claude/settings.json` | Pre-approved safe `soli` subcommands (no per-prompt approval tax) |
| `.claude/commands/` | Custom slash commands: `/soli-verify`, `/soli-test`, `/soli-resource` |
| `docs/` | Full copy of the Soli documentation, so agents never need an internet round-trip |

Claude Code automatically discovers and loads the nearest `CLAUDE.md` files as the agent navigates directories. Other tools read the root files.

This is not a thin "here's a prompt" layer. It is a complete, enforceable contract between the human team and any AI collaborator.

## The Verification Loop (The Non-Negotiable Rule)

The single most important piece of the system is the **verification loop**, documented at the top of every `CLAUDE.md`:

```bash
soli lint <files-you-changed>           # naming, smells, undefined-locals
soli test tests/<the-relevant-spec>.sl  # narrow, fast feedback
soli test --coverage --coverage-min 90  # full sweep
soli serve . --dev && open the browser  # if UI or routes changed
```

The `/soli-verify` slash command bundles the first three steps.

The rule is simple and absolute:

> If any step fails, **fix the root cause**. Never weaken the assertion, lower the coverage gate, or use `--no-verify`.

This single rule prevents the most common failure mode of AI-assisted development: the agent slowly accumulating technical debt and broken tests while the human is distracted.

Because the loop is fast (full test suite on a fresh project is usually under a second), agents can run it after almost every meaningful change without destroying flow.

## Slash Commands That Actually Matter

The `.claude/commands/` directory contains three particularly useful custom commands:

### `/soli-verify`

Runs the full pre-merge check (lint + targeted test + full coverage gate). This is the one agents are instructed to run before claiming any task is complete.

### `/soli-resource <name>`

The killer feature for agents.

Running `/soli-resource post` executes:

1. `soli generate model post`
2. `soli generate migration create_posts`
3. `soli generate controller posts`
4. Adds `resources("posts")` to `config/routes.sl`
5. Stubs the standard views and a controller spec

It then **pauses** after step 4 and tells the agent: "Let the human fill in the fields, validations, and associations on the model before continuing."

This is the kind of judgment call that agents are bad at making on their own. By baking the pause into the command, we get the best of both worlds: massive boilerplate reduction + human oversight at the highest-leverage moment.

### `/soli-test [path]`

Fast, narrow feedback without having to remember the exact test command.

## Why the Constraints Help Agents

Soli's "minimal by design" philosophy is not just for humans.

A small, coherent surface area means:

- Far fewer ways to do any given thing
- Strong, consistent naming conventions that are actually enforced by `soli lint`
- One canonical way to define routes, models, validations, and templates
- No invisible magic from 17 transitive dependencies

When an agent proposes a change, the probability that it violates "how we do things here" is dramatically lower than in a large, multi-paradigm, historically layered codebase.

The same strictness that occasionally frustrates experienced humans (especially those coming from more permissive frameworks) is exactly what makes the agent reliable.

## Bundled Documentation

One particularly nice detail: `soli new` copies the entire `www/docs/` tree (the full Soli documentation) into the new project's `docs/` directory.

This means an agent working on a plane, in a restricted network, or inside a large monorepo can still look up exact API details, migration patterns, or testing helpers without hallucinating or requiring an external call.

The per-directory `CLAUDE.md` files even contain relative links into this local copy.

## Real-World Effect

Teams using this setup report that agents can reliably:

- Add new RESTful resources end-to-end with minimal supervision
- Implement background jobs and their tests correctly on the first try
- Navigate the model layer without accidentally bypassing validations or soft-delete scopes
- Produce pull requests that pass the full verification loop without human intervention

The human's job shifts from "constant code review and correction" to "define the shape of the change and review the high-level approach."

## Adding This to an Existing Project

If you have an older Soli app and want the agent support:

1. Run `soli new tmp-agent-kit` somewhere
2. Copy the AI files across (preserving paths)
3. Merge the "For AI agents" and "Footgun cheatsheet" sections into your existing root `CLAUDE.md`
4. Run `/soli-verify`

The files are deliberately designed to be incrementally adoptable.

## The Bigger Bet

Soli is betting that the future of software development is **tight human + AI collaboration** inside small, coherent, well-constrained systems — not increasingly complex frameworks that require ever-larger teams of humans (and increasingly expensive AI) just to keep the lights on.

The AI agent scaffolding is one of the most concrete expressions of that bet.

If you want to see what it actually feels like, the fastest way is:

```bash
soli new my-ai-experiment
cd my-ai-experiment
# open in Claude Code, Cursor, or your agent of choice
# try: /soli-resource product
```

Then watch what happens when the agent has a clear contract, fast feedback, and a language that was built to be understood rather than accumulated.

---

The image at the top of this post shows a realistic view of what a well-scaffolded Soli project looks like to an AI agent: clear context, safe commands, and an explicit verification contract. All of it ships by default.