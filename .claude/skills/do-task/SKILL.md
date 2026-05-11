---
name: do-task
description: Implement the fix described in tasks/todo/<slug>.md, run the project's verification loop, and stop without committing. Use when the orchestrator dispatches a task into a fresh worktree and asks you to do the work.
---

# do-task

You are running inside a fresh git worktree that the task-orchestrator just created. The DB row owns the task's lifecycle (queued → inprogress → review → done) — your job is the **implementation**, nothing more.

## Inputs

- **Argument**: the task slug. The spec lives at `tasks/todo/<slug>.md`. If no argument is passed, list `tasks/todo/*.md`, pick the alphabetically-first entry, and infer the slug from its filename.
- **`CLAUDE.md`** at the worktree root (and per-directory) describes the project's conventions and verification loop. Read it before editing.

Do **not** move, rename, or delete the spec md. The orchestrator excludes it from git via `.git/info/exclude`; leave it where it is.

## Step 1 — Read the spec

Read `tasks/todo/<slug>.md` in full. It should describe Severity, Location (file:line refs), the Issue, and a proposed Fix.

## Step 2 — Inspect repo state

In parallel:
- `git status` — expect exactly one untracked file: `tasks/todo/<slug>.md` (the spec the orchestrator just seeded). Anything else untracked or modified means something is wrong; stop and report.
- `git log --oneline -5` — to match the project's commit-message style later.

Read every file referenced in the spec's Location section before editing.

## Step 3 — Implement

1. Apply the proposed Fix, or an equivalent that addresses the root cause. The Fix in the spec is a *suggestion* — deviate when there's a better approach, but never paper over the issue (e.g. swallowing an error instead of fixing it).
2. **Cover every call site listed in the spec.** Patch all of them. Do **not** search for similar patterns elsewhere — only fix what the spec explicitly describes.
3. **Stay in scope.** No drive-by refactors, renames, or fixes to neighboring issues. If you spot a separate problem, drop a `tasks/todo/<NEW-slug>.md` for it (the orchestrator will pick it up next ingest) and continue with the original task.
4. **Docs.** If the change is user-facing (new API, config flag, behavior change) and `CLAUDE.md` describes a documentation policy, follow it — update every surface the policy names in the same change.

## Step 4 — Verify

Run the project's verification loop **as defined in `CLAUDE.md`**. If `CLAUDE.md` lists explicit commands (lint / test / coverage / format), run those. If it doesn't, fall back to the obvious-for-the-stack equivalents — but read `CLAUDE.md` first.

If verification fails, fix the root cause. **Never** suppress warnings, skip hooks (`--no-verify`), bypass signing, or weaken assertions to make checks pass.

If a check fails on a pre-existing issue clearly unrelated to your change, note it in your summary and continue — but be conservative about that judgement.

## Step 5 — Stop

Leave all implementation changes **uncommitted** in the working tree. The `review-task` skill, which the orchestrator runs immediately after, is responsible for reviewing the diff and creating the commit. Pre-committing forces an amend or revert if review rejects the fix.

End with a short summary to the user:
- Which task (slug).
- What was changed (file list).
- What was verified (commands run + outcome).
- Any follow-ups dropped as new `tasks/todo/<slug>.md` files.

## Constraints

- Process exactly **one** task per invocation.
- **Never** use `pkill`, `kill`, `killall`, or any process-killing command.
- **Never** commit, push, or skip hooks.
- **Never** edit the spec md content. It's the historical record. Surface follow-ups via new `tasks/todo/<slug>.md` files instead.
- **Never** `git mv` the spec between `tasks/todo/`, `tasks/inprogress/`, `tasks/review/`, etc. The DB tracks status; folder-shuffling is the old file-based queue.
- If verification fails and you can't fix it within the scope of the spec, revert your changes (`git restore .`) and report — don't leave a half-done task in the worktree.
- If the user passed a slug that doesn't have a matching `tasks/todo/<slug>.md`, stop and list what *is* there. Don't silently pick a different one.
