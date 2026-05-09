---
name: review-task
description: Pick a task file from tasks/review/, perform a code review of the implemented fix, move the file to tasks/done/ if the fix is correct, and commit. Use when the user wants to verify a fix that has been implemented for one of the SEC-NNN (or any) task tickets and progress it to done.
---

# review-task

Workflow: pick a single task md from `tasks/review/`, verify the fix in the codebase, move the file to `tasks/done/`, and commit.

## Step 1 — Pick the task

- If the user passed an argument (e.g. `SEC-007` or `007` or a full filename), match it against `tasks/review/*.md` (case-insensitive prefix match). If no match, list the files in `tasks/review/` and stop.
- If no argument, list `tasks/review/` sorted alphabetically and pick the first file. If the directory is empty, tell the user and stop.
- Read the file. It should contain a Severity, Location (file:line refs), Issue description, and proposed Fix.

## Step 2 — Inspect repo state

Run in parallel:
- `git status` (see uncommitted changes)
- `git log --oneline -10` (recent commits — the fix may already be committed)
- `git diff` and `git diff --cached` (current working-tree and staged diffs)

Identify whether the fix is:
- **(A) committed** — find the commit(s) that touched the files in the task's Location section.
- **(B) uncommitted** — present in working tree / index.
- **(C) missing** — neither.

If **(C) missing**, do not stop here. The right outcome may still be to close the ticket — e.g. the report turned out to be invalid/won't-fix, or the work is being deferred via new follow-up tickets in `tasks/todo/`. Continue to Step 3 and let the verdict drive the decision.

## Step 3 — Code review

Read the current state of the files referenced in the task's Location section. For each location:

1. **Verify the change addresses the described issue.** The proposed Fix in the task md is a *suggestion* — accept any equivalent implementation, but reject changes that paper over the issue (e.g. catching an exception instead of fixing the root cause).
2. **Check completeness.** If the issue lists multiple call sites (e.g. SEC-007 lists every `ureq::*` call), verify every one was patched. Do NOT search for similar patterns elsewhere — only verify what's explicitly described in the task.
3. **Look for regressions.** Read the diff for the touched files in full. Flag: new unsafe blocks, new error handling that swallows errors silently, broadened privileges, removed validation, weaker types.
4. **Check docs (user-facing changes).** If the change adds or alters a builtin function, method, language feature, or config flag, verify all three docs surfaces are updated together:
    - `www/docs/*.md`
    - `www/app/views/docs/**/*.html.slv`
    - `www/public/js/search-index.json` — there must be an `entries[]` record for the new/changed API. Missing this entry is a blocking review finding: the docs page renders but search can't find it. Reject (or Approved-with-followups + new `tasks/todo/<TICKET>.md` to backfill the index) if it's missing.
5. **Check tests.** If `tests/` has a relevant existing test, verify it still passes (`cargo test <relevant>` if cheap, otherwise note that tests should be run). If the fix is non-trivial and there is no test, flag this as a review concern but do not block on it unless the issue is Critical/High.
6. **Run static checks** if practical: `cargo clippy --quiet -- -D warnings` and `cargo fmt --check` for Rust changes. If they fail, report and stop — do not commit broken code.

Report findings as a short markdown bullet list to the user, structured as:
- **Verdict:** Approved / Approved-with-notes / Approved-no-fix-needed / Approved-with-followups / Rejected
- **Coverage:** what was checked
- **Findings:** issues found (each: severity, file:line, what's wrong, suggested follow-up)

## Step 4 — Decision

- **Rejected** — Do NOT move the file. Do NOT commit. Tell the user what's missing and stop.
- **Approved-with-notes** — Ask the user whether to proceed (notes are non-blocking). If yes, continue to Step 5. If no, stop.
- **Approved-no-fix-needed** — Review concluded the report is invalid, describes intended behavior, or is won't-fix. Continue to Step 5; the commit message must record the reasoning.
- **Approved-with-followups** — The work is deferred. Before Step 5, create one `tasks/todo/<TICKET>.md` file per follow-up (use the next free SEC-/TICKET- number; mirror the original's structure: Severity, Location, Issue, proposed Fix). Continue to Step 5; the new files will be staged with the move.
- **Approved** — Continue to Step 5.

## Step 5 — Move the task file

Use `git mv` so the move is tracked as a rename:

```bash
git mv tasks/review/<filename>.md tasks/done/<filename>.md
```

## Step 6 — Commit

Match the repo's commit style (look at recent `git log` messages — usually `<type>(<scope>): <subject>` or `security: ...`). Use a HEREDOC so the message formats correctly.

Stage everything that belongs to this task and commit it together:
- the renamed task file (already tracked by `git mv`)
- any uncommitted fix in the working tree (case B from Step 2)
- any new `tasks/todo/<TICKET>.md` files created for follow-ups

If the fix was already committed in a prior commit (case A), or the verdict is **Approved-no-fix-needed** with no follow-ups, the commit will contain just the rename — that is a valid, non-empty change. Do not bail out as "nothing to commit" and **never** add `--allow-empty`. If `git status` shows no staged changes after `git add`, it means `git mv` never ran or was undone — investigate, don't paper over it.

Pick a commit subject that matches the verdict:
- **Approved** / **Approved-with-notes** — `fix(...)` / `feat(...)` style; subject describes the fix.
- **Approved-no-fix-needed** — `chore(<scope>): close <TICKET> (won't fix)` or `(invalid)`; the body must explain the decision.
- **Approved-with-followups** — `chore(<scope>): close <TICKET> (deferred)`; the body must list the new `tasks/todo/...` files.

```bash
git commit -m "$(cat <<'EOF'
fix(security): SEC-007 enforce SSRF re-validation on HTTP redirects

Closes tasks/review/SEC-007-ssrf-redirect-bypass.md.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

After commit, run `git status` to confirm the working tree is clean (or only contains unrelated changes).

## Constraints

- **Never** move the file if the review verdict is Rejected.
- **Never** skip hooks (`--no-verify`) or bypass signing. If a pre-commit hook fails, fix the underlying issue and create a new commit.
- **Never** push. Stop after the local commit.
- **Do not** modify the task md content during review. The md is the historical record of the issue; if the review surfaces new follow-up work, create a separate file in `tasks/todo/` (e.g. `SEC-007a-followup.md`) rather than editing the original.
- If the user passed an explicit task argument that doesn't exist in `tasks/review/`, stop and list the files that ARE there. Don't fall back to picking a different one.
- Process exactly **one** task per invocation. Do not loop through `review/`.
