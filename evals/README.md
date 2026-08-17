# Agents on Soli (evals)

Atomic coding tasks used to grade **models** on real Soli work. Results
are published on [`/ai`](https://solisoft.net/ai) from
`www/data/ai_evals.json`.

This is **not** a Cursor-vs-Claude-Code table. The harness is frozen; the
variable is the model.

## Layout

```
evals/app/                 fixture MVC app (the starting worktree)
evals/tasks/<slug>/
  prompt.md                what the model is asked (visible)
  expect.md                identifiers that count as an API hit
  hidden/                  tests the model never sees
scripts/evals/run.py       copy → (optional agent) → lint + test → JSON
```

## Grade locally (no paid run)

```bash
# lint + test the fixture as-is (no model)
python3 scripts/evals/run.py --grade-fixture

# after you apply a task by hand in a copy of evals/app:
python3 scripts/evals/run.py --grade --task hello-world --workdir /tmp/soli-eval
```

## Paid board refresh

Frozen runners: **Claude Code** (`claude -p`), **OpenCode + DeepSeek**,
**Grok Build**. Each run is a fresh copy of `evals/app`.

```bash
python3 scripts/evals/run.py --list
python3 scripts/evals/run.py --dry-run --models claude,opencode,grok --tasks hello-world --runs 1
python3 scripts/evals/run.py --models claude,opencode,grok --tasks hello-world --runs 1 --keep
python3 scripts/evals/run.py --models claude,opencode,grok --out www/data/ai_evals.json
```

Commit the JSON. Do **not** invent scores. Do **not** run this on every PR.

See `www/docs/ai-evals.md`.
