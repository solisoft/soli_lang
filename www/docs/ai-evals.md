# Agents on Soli — methodology

Public table: [`/ai`](/ai). Source of numbers: `www/data/ai_evals.json`.
Corpus and harness: `evals/` and `scripts/evals/run.py`.

This page is the contract. The `/ai` table is only allowed to show numbers
that this process produced.

## What is being compared

The first board freezes **three coding CLIs**, each with one default model:

| Runner | CLI | Default model |
|---|---|---|
| `claude` | `claude -p` (Claude Code) | `sonnet` (`SOLI_EVALS_CLAUDE_MODEL`) |
| `opencode` | `opencode run` | `opencode/deepseek-v4-flash-free` |
| `grok` | `grok --prompt-file` (Grok Build) | `grok-4.6` |

Same fixture, same prompts, same hidden tests. The table id is the
**runner** (claude-code / opencode-deepseek / grok-build), not a raw
API model name.

This is not Cursor vs Claude Code vs Codex as products. Later boards
can swap the model under one CLI.

Not compared: Soli vs Rails on the same score. Rails’ [Agents on Rails](https://rubyonrails.org/2026/8/12/llm-benchmarking-project)
uses Writebook and a different grader. Different corpus, different
language.

## Corpus (Stage 1)

Twelve one-capability tasks on a tiny MVC fixture (`evals/app`), not the
docs site:

| Slug | Capability |
|---|---|
| `hello-world` | harness smoke |
| `scaffold-resource` | `resources` + controller |
| `hash-where` | portable `{ "total": { "gt": 10 } }` |
| `column-sti` | subclass + `type` |
| `job-enqueue` | `NotifyJob.perform_later` |
| `unless-guard` | block `unless` |
| `csrf-webhook` | `skip_csrf` on the webhook only |
| `form-permit` | `permit` / mass-assignment |
| `validation` | `validates` |
| `attachment` | `has_one_attached` |
| `test-spec` | `describe` / `assert_eq` (not `assert_equal`) |
| `llm-stream` | `sse` + `out.llm_stream` |

Each task has a visible `prompt.md`, hidden tests the model never sees,
and `expect.md` identifiers for **API recall**.

Stage 2 (not this cycle): multi-step features across model + job + view.

## Grading

For each run:

1. Copy `evals/app` to a fresh worktree.
2. Apply the prompt with one of the frozen CLIs (`claude -p`, `opencode run`, `grok --prompt-file`). Hidden tests are not in the tree yet.
3. Copy `evals/tasks/<slug>/hidden/` into that tree.
4. Run `soli lint` and `soli test hidden`.
5. Scan the diff for the identifiers in `expect.md`.

Pass = lint green **and** hidden tests green. Failures and refusals are
fails.

**3 runs per model.** Published:

- **accuracy** — passes / (tasks × 3)
- **speed** — median wall time (seconds)
- **tokens** — mean input+output
- **cost** — mean USD from a published price table
- **API recall** — mean hits / expected identifiers

## Cost

A full refresh is about 5 models × 12 tasks × 3 ≈ 180 runs. At
$0.50–$2 per run that is roughly **$90–$360**. That is why this is
**not** on every PR.

## Reproduce

```bash
python3 scripts/evals/run.py --list
python3 scripts/evals/run.py --grade-fixture
python3 scripts/evals/run.py --dry-run --models claude,opencode,grok --tasks hello-world --runs 1
# smoke (1 task × 1 run × 3 runners)
python3 scripts/evals/run.py --models claude,opencode,grok --tasks hello-world --runs 1 \
  --out /tmp/ai_evals_smoke.json --keep
# full board (12 × 3 × 3) — costs real credits
python3 scripts/evals/run.py --models claude,opencode,grok --out www/data/ai_evals.json
```

A paid refresh writes `www/data/ai_evals.json` and that file is
committed. Until `models` is non-empty, `/ai` shows an empty state —
never placeholder scores. Token/cost stay `null` when a CLI does not
report them (do not fill them from a guessed price table).

Harness id: `soli-evals/0.2`.
