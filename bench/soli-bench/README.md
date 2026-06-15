# SoliBench

A deterministic eval suite for measuring how well a coding model (or human) writes
**Soli**. The grader uses `soli test` and `soli lint` as ground truth — no human
judgment required — so it's safe to wire into release gates and CI.

## Why this exists

Soli is a small language. The natural corpus is small. Pre-trained models
extrapolate from Ruby/Python/Lua and routinely get the *style* wrong even when
the syntax is close. SoliBench localizes the gap into three suites so we can see
where the model wins and where it bleeds points:

| Suite   | What it tests                                                         | Grader signal                       |
|---------|-----------------------------------------------------------------------|-------------------------------------|
| `core`  | Language-level tasks: arrays, hashes, lambdas, pipelines, pattern matching, postfix if/unless/rescue, `??`, `\|\|=` | `soli test` pass count              |
| `mvc`   | Full MVC tasks: model + controller + route + spec, scaffolded from a prompt | `soli test` pass count              |
| `idiom` | Anti-pattern traps: rewrite a stub so `soli lint` reports zero errors | `soli lint` error count (0 = pass)  |

A model that scores well on `core`/`mvc` but poorly on `idiom` knows Soli but
writes it like a Ruby-ist. A model that scores well on `idiom` but poorly on
`core` knows the rules but can't apply them. Claude Code (with the repo
context) is the bar to beat.

## Layout

```
bench/soli-bench/
├── core/<task>/           # core: one .sl per task + a test spec + a prompt
├── mvc/<task>/            # mvc: a small app skeleton with TODO markers
├── idiom/<task>/          # idiom: a stub that triggers a known lint rule
├── grader.sh              # the deterministic grader
├── lib/                   # bash helpers used by grader.sh
├── tasks.json             # manifest (suite, name, path, weight, lint_rules)
├── LEADERBOARD.md         # results table — update per release
└── README.md              # this file
```

### Per-task layout

```
core/group_by_array/
├── prompt.md          # natural-language description of the task
├── solution.sl        # reference implementation
├── stub.sl            # starter file the model fills in (with TODO markers)
├── tests.sl           # pre-written tests the solution must pass
└── meta.json          # { "id", "suite", "name", "weight", "lint_rules": [...] }
```

The grader copies `solution.sl` (or, in a model run, the candidate file the
model produced) into a temp scratch dir together with `tests.sl`, runs
`soli test`, and counts pass/fail.

## Requires SoliDB (mvc suite only)

The `core` and `idiom` suites are self-contained. The `mvc` suite runs against
a live SoliDB, so export the connection before grading it:

```bash
export SOLIDB_HOST=http://localhost:6745
export SOLIDB_USERNAME=admin
export SOLIDB_PASSWORD=…
# SOLIDB_DATABASE defaults to `solibench_test` (an isolated bench DB) if unset.
```

`soli test` truncate-resets the bench database before each spec, so grading
never accumulates state. Without these vars the mvc tasks fail (the DB returns
401); `core` and `idiom` are unaffected.

## Running the grader

```bash
# Reference run — solution.sl is copied verbatim, all suites should pass.
./bench/soli-bench/grader.sh --reference

# A model run — point --solution-dir at a directory the model produced
# (mirroring bench/soli-bench/<suite>/<task>/stub.sl).
./bench/soli-bench/grader.sh --model minimax-m3 --solution-dir runs/m3-2026-06-14

# Filter to one suite.
./bench/soli-bench/grader.sh --suite core

# Filter to one task.
./bench/soli-bench/grader.sh --suite core --task group_by_array

# Write results to JSON (for diffing between runs).
./bench/soli-bench/grader.sh --json runs/$(date +%s).json
```

The grader exits 0 if every required task passes, 1 otherwise. CI should treat
exit 1 as a release blocker for the model being graded.

## Adding a new task

1. Pick the suite (`core` / `mvc` / `idiom`).
2. Make `bench/soli-bench/<suite>/<task>/` with `prompt.md`, `solution.sl`,
   `stub.sl`, `tests.sl`, and `meta.json`.
3. The `meta.json` shape:
   ```json
   {
     "id": "core.group_by_array",
     "suite": "core",
     "name": "group_by_array",
     "weight": 1.0,
     "tags": ["array", "higher-order"],
     "lint_rules": []
   }
   ```
   For `idiom` tasks, `lint_rules` is the list of rules the stub triggers (the
   `soli lint` error count must drop to 0 on the solution):
   ```json
   "lint_rules": ["smell/undefined-local", "style/redundant-model-import"]
   ```
4. Re-run `./bench/soli-bench/grader.sh --reference` to confirm the reference
   solution passes.

## Anti-leakage

SoliBench tasks are not pulled from `tests/language/*_spec.sl` or
`tests/builtins/*_spec.sl` directly — they're *inspired* by them but written
from scratch, so the model isn't just being graded on recall.

> _Planned:_ a corpus builder (`tools/training/build_corpus.sh`) to MinHash-dedup
> any training set against `bench/soli-bench/` fixtures and prevent overlap. Not
> built yet — keep new tasks original by hand for now.

## Leaderboard

See [LEADERBOARD.md](./LEADERBOARD.md). Update after every release with the
exact output of:

```bash
./bench/soli-bench/grader.sh --reference --json /tmp/ref.json
./bench/soli-bench/grader.sh --model claude-code --json /tmp/cc.json
```

and a short diff/summary.
