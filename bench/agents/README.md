# agent-bench

The same coding agent (Claude Code, headless) builds the same apps on different stacks;
a hidden HTTP acceptance suite grades each result. The question it answers: **on which
stack does an agent ship a correct app most often, fastest and cheapest?**

## Layout

| Path | What |
|---|---|
| `specs/<name>/SPEC.md` | What the agent is asked to build (copied into the project) |
| `specs/<name>/checks.py` | The hidden acceptance suite — plain functions over a `Tester` |
| `stacks/<name>.json` | How to scaffold a project, services it needs, skills it ships, one line of stack guidance |
| `prompt.md` | The prompt, identical for every stack apart from the stack name and that line |
| `reference/server.py` | Stdlib implementation of every spec; `validate` proves the suites are passable |
| `bench.py` | `validate`, `run`, `check` |
| `report.py` | Markdown summary of `results/*.jsonl` |
| `container/Dockerfile` | Image for containerised trials: Ruby + Rails, Node, git, SQLite |

## Use

```bash
python3 bench.py validate                                   # suites vs. reference: must be all ok
python3 bench.py run --stacks soli --specs todo-api         # one smoke trial
python3 bench.py run --stacks all --specs all --trials 5 --jobs 3
python3 report.py results/<run>.jsonl
python3 bench.py check ~/.cache/agent-bench/runs/<run>/<trial> --spec todo-api   # re-grade one trial
```

### In containers (recommended)

```bash
python3 bench.py image --container podman        # or docker
python3 bench.py run --container podman --stacks all --specs all --trials 5 --jobs 3
```

Every command of a trial — setup, SoliDB, the agent, `run.sh` — runs in a throwaway
container of that image, with host networking and the trial directory mounted as `/work`.
No toolchain on the host beyond Python, and the agent's `bypassPermissions` stays inside
the container. `soli`, `solidb` and `claude` are the host's binaries, mounted read-only,
so every stack runs the same agent build. The agent authenticates with `ANTHROPIC_API_KEY`
when set, otherwise with a per-trial copy of `~/.claude/.credentials.json` that is deleted
when the trial ends.

Trial directories go under `~/.cache/agent-bench/runs/` (`AGENT_BENCH_WORK` overrides):
the scaffolded app, `agent.json` (Claude Code's result: cost, turns, tokens),
`setup.log`, `server.log`, and for Soli the trial's own `solidb-data/`.

## One trial

1. Scaffold with the stack's official generator (`soli new`, `rails new`, `create-next-app`),
   copy `SPEC.md` in, commit a git baseline.
2. For Soli, start a fresh SoliDB on a free port and point `.env` at it. Rails and Next.js
   use SQLite, which needs no service.
3. Run `claude -p` in the project with `--setting-sources project,local`: the user's own
   `~/.claude` (CLAUDE.md, skills, memory) is **not** loaded, so every stack gets only what
   it ships. For Soli that is what `soli new` writes (CLAUDE.md, `.claude/`) plus the
   `soli-lang` and `solidb` skills copied into the project. No web access unless `--web`.
4. Start the agent's `run.sh` on a new port and run the hidden checks, including a restart
   to prove persistence.
5. Record checks passed, cost, turns, tokens, wall time, and lines added (diff vs. the
   baseline, lockfiles excluded).

## Caveats

- Without `--container`, the agent runs with `bypassPermissions` on the host, confined to a
  scratch directory by instruction only, and each stack needs its toolchain installed.
- Soli is graded with the skills it ships; add a `soli-noskill` stack to measure what the
  skills are worth on their own.
- The specs are JSON APIs so one suite grades every stack. HTML/forms, realtime and
  background jobs need specs of their own (browser-driven checks).
- Next.js + Supabase needs Docker for a local Supabase; the Next.js stack uses SQLite for now.
