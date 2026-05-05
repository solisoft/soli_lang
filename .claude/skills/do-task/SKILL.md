---
name: do-task
description: Pick a task file from tasks/todo/, move it to tasks/inprogress/, implement the fix, run verification, and move it to tasks/review/. Use when the user wants to start work on one of the SEC-NNN (or any) task tickets and progress it from todo through implementation to ready-for-review.
---

# do-task

Workflow: pick a single task md from `tasks/todo/`, move it to `tasks/inprogress/`, implement the fix, verify it builds, and move the file to `tasks/review/`. Pairs with `review-task` (which handles `review/` → `done/`).

## Step 1 — Pick the task

- If the user passed an argument (e.g. `SEC-007` or `007` or a full filename), match it against `tasks/todo/*.md` (case-insensitive prefix match). If no match, list the files in `tasks/todo/` and stop.
- If no argument, list `tasks/todo/` sorted alphabetically and pick the first file. If the directory is empty, tell the user and stop.
- Read the file. It should contain a Severity, Location (file:line refs), Issue description, and proposed Fix.

## Step 2 — Inspect repo state

Run in parallel:
- `git status` (working tree must be clean of unrelated edits before starting — if it isn't, stop and ask the user how to proceed)
- `git log --oneline -5` (commit-style reference)

If `tasks/inprogress/` already contains a file, stop and ask the user whether to resume that task or move it back to `todo/` first. Process **one** task at a time.

## Step 3 — Move to inprogress

Use `git mv` so the move is tracked as a rename:

```bash
git mv tasks/todo/<filename>.md tasks/inprogress/<filename>.md
```

## Step 4 — Implement the fix

1. Read every file referenced in the task's Location section in full before editing.
2. Apply the proposed Fix, or an equivalent implementation that addresses the root cause. The Fix in the task md is a *suggestion* — deviate when there's a better approach, but never paper over the issue (e.g. swallowing an error instead of fixing it).
3. **Cover every call site.** If the issue lists several locations, patch all of them. Grep for the same pattern elsewhere in the codebase and patch any other matches.
4. **Stay focused.** Don't refactor unrelated code, rename symbols, or fix neighboring issues — those belong in their own task. If you find a new issue, drop a `tasks/todo/<NEW>.md` for it instead of expanding scope.
5. **Docs.** If the change is user-facing (new builtin, config flag, behavior change), update both `www/docs/*.md` AND the matching `www/app/views/docs/**/*.html.slv` in the same change (per the project's documentation policy in `CLAUDE.md`).

## Step 5 — Verify

Run the relevant subset for the changes made:

- Rust changes: `cargo clippy --quiet -- -D warnings` and `cargo fmt --check`
- Tests: `cargo test <relevant>` if a targeted test exists; otherwise note in the summary that broader tests should be run.
- Soli changes: `soli test` if `tests/` covers the area.

If verification fails, fix the root cause. **Never** suppress warnings, skip hooks, or weaken assertions to make checks pass. If clippy fails on a pre-existing issue unrelated to your change, note it in your summary and continue.

## Step 6 — Move to review

```bash
git mv tasks/inprogress/<filename>.md tasks/review/<filename>.md
```

Leave the implementation changes **uncommitted** in the working tree. The `review-task` skill is responsible for reviewing the diff and committing — pre-committing would force an amend or revert if review rejects the fix.

End with a short summary to the user: which task, what was changed (file list), what was verified, and any follow-ups noted as new `todo/` files.

## Constraints

- Process exactly **one** task per invocation. Do not loop through `todo/`.
- **Never** skip hooks (`--no-verify`) or bypass signing.
- **Never** modify the task md content. The md is the historical record of the issue; if implementation surfaces follow-up work, create a separate `tasks/todo/<NEW>.md` rather than editing the original.
- **Never** commit. Leave changes staged-or-unstaged in the working tree for `review-task` to handle.
- If the user passed an explicit task argument that doesn't exist in `tasks/todo/`, stop and list the files that ARE there. Don't silently pick a different one.
- If verification fails and you can't fix it within the scope of this task, move the file **back** to `tasks/todo/` (`git mv`), revert your code changes, and report — don't leave a half-done task in `inprogress/`.
