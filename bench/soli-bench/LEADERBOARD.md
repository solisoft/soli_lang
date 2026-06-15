# SoliBench Leaderboard

Deterministic scores from `grader.sh`. The **reference** row copies each
`solution.sl` verbatim and must always be 100% — it's the proof the suite
itself is green, not a model result. Add a row per model run below it.

Suite weights: `core` tasks total 15.0, `mvc` 8.5, `idiom` 8.5 (weighted score
is the weight-passed / weight-total across all 22 tasks).

| Model        | Date       | Overall | core         | mvc        | idiom       | Notes |
|--------------|------------|---------|--------------|------------|-------------|-------|
| reference    | 2026-06-15 | 100.0%  | 12/12 (100%) | 3/3 (100%) | 7/7 (100%)  | baseline — verbatim solutions |
| _your model_ | —          | —       | —            | —          | —           | run it and add a row |

## Reproducing

```bash
# SoliDB must be up for the mvc suite (see README "Requires SoliDB").
export SOLIDB_HOST=http://localhost:6745 SOLIDB_USERNAME=admin SOLIDB_PASSWORD=…

# Reference (should print 100%).
./bench/soli-bench/grader.sh --reference --json /tmp/ref.json

# A model run — point --solution-dir at the candidate tree the model produced
# (mirroring bench/soli-bench/<suite>/<task>/stub.sl).
./bench/soli-bench/grader.sh --model my-model --solution-dir runs/my-model --json /tmp/my-model.json
```

Update the table after each release with the exact `grader.sh` summary output.

## Where models tend to bleed points

- **idiom** is the Soli-fluency tell. A model that scores well on `core`/`mvc`
  but poorly on `idiom` knows the syntax but writes Soli like a Ruby/Python-ist
  (`== null` instead of `.nil?`, manual membership chains instead of
  `.includes?`, a dead nil-check after `Model.find`). Those are exactly the
  `idiom/*` lint rules.
- **mvc** exercises the real `Model` API (instance returns from `create`,
  `._key`, `.where(...).order(...).paginate(...)`, class `scope`). Models that
  hallucinate a Rails/ActiveRecord shape fail here.
