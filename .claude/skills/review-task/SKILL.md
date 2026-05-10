---
name: review-task
description: Review the uncommitted fix in this worktree against tasks/todo/<slug>.md, then commit (or stop with a Rejected verdict). Use after /do-task — the orchestrator runs us in the same worktree to verify and commit the change.
---

# review-task

You are running inside the same worktree where `/do-task` just implemented a fix. The orchestrator's DB row tracks the task's lifecycle; your job is the **code review** of the diff and, if approved, the **commit**.

## Inputs

- **Argument**: the task slug. The spec lives at `tasks/todo/<slug>.md`. If no argument is passed, infer the slug from the single spec md present.
- **`CLAUDE.md`** describes the project's conventions, verification loop, and (often) commit-message style.

Do **not** move, rename, or delete the spec md. The orchestrator owns it; leave it where it is — but **never commit it** (see Step 5).

## Step 1 — Read the spec

Read `tasks/todo/<slug>.md` in full: Severity, Location, Issue, proposed Fix.

## Step 2 — Inspect repo state

In parallel:
- `git status`
- `git log --oneline -10` (to match the project's commit-message style)
- `git diff` and `git diff --cached`

The fix is normally **uncommitted** in the working tree (that's how `/do-task` leaves it). If `git status` shows nothing changed, the verdict will likely be **Approved-no-fix-needed** — continue and let Step 4 decide.

## Step 3 — Review

Read the current state of the files referenced in the spec's Location section. For each:

1. **Verify the change addresses the described issue.** The proposed Fix in the spec is a *suggestion* — accept any equivalent implementation, but reject changes that paper over the issue (e.g. catching and ignoring an exception instead of fixing the root cause).
2. **Check completeness.** If the spec lists multiple call sites, verify every one was patched. Do **not** broaden scope — only verify what the spec explicitly described.
3. **Look for regressions.** Read the diff for the touched files in full. Flag: new unsafe paths, swallowed errors, broadened privileges, removed validation, weakened types.
4. **Check docs (user-facing changes).** If the project's `CLAUDE.md` describes a documentation policy, verify every surface it names was updated.
5. **Run static checks** as defined in `CLAUDE.md` (lint / typecheck / format / test / coverage). If they fail, report and stop — do not commit broken code.

Report findings as a short markdown bullet list:
- **Verdict:** Approved / Approved-with-notes / Approved-no-fix-needed / Approved-with-followups / Rejected
- **Coverage:** what was checked
- **Findings:** issues (each: severity, file:line, what's wrong, suggested follow-up)

## Step 4 — Decision

- **Rejected** — Do **not** commit. Report what's missing and stop. The orchestrator will mark the row failed.
- **Approved-with-notes** — Notes are non-blocking. Continue to Step 5.
- **Approved-no-fix-needed** — Review concluded the spec is invalid, describes intended behavior, or is won't-fix. Continue to Step 5; if there is no diff, stop without committing — the orchestrator detects "no commit produced" and treats it as a successful no-code-change close.
- **Approved-with-followups** — Work is deferred. Before Step 5, drop one `tasks/todo/<NEW-slug>.md` per follow-up. Mirror the original's structure (Severity, Location, Issue, proposed Fix). Continue to Step 5.

    **Reality check before creating any follow-up.** For every symbol you name in a follow-up's title or Issue (function, method, file path, doc id), `grep` the worktree to confirm it exists OR explicitly frame the follow-up as a feature task ("Add `<X>`...", with rationale). Don't materialize a follow-up referencing a name you can't find. If unsure, drop the follow-up and note the uncertainty in the verdict.
- **Approved** — Continue to Step 5.

## Step 5 — Commit

Match the project's commit style — read the recent `git log` output and any guidance in `CLAUDE.md`.

Stage everything that belongs to this task:
- the implementation diff
- any new `tasks/todo/<NEW-slug>.md` follow-up files

`git add -A` will sweep up the orchestrator-seeded spec md too, which we do **not** want in the commit. Always run, immediately after staging:

```bash
git restore --staged tasks/todo/<slug>.md
```

(Substitute the actual slug.) This drops the spec from the index while keeping the file on disk. Verify with `git status` that `tasks/todo/<slug>.md` shows as untracked again before committing.

Use a HEREDOC so the message formats correctly. Pick a subject that matches the verdict:
- **Approved** / **Approved-with-notes** — `fix(...)` / `feat(...)` describing the change.
- **Approved-no-fix-needed** — `chore(<scope>): close <slug> (won't fix)` or `(invalid)`; the body must explain the decision.
- **Approved-with-followups** — `chore(<scope>): close <slug> (deferred)`; the body must list the new `tasks/todo/...` files.

```bash
git commit -m "$(cat <<'EOF'
fix(<scope>): <subject>

Closes <slug>.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

After commit, run `git status` to confirm the working tree is clean (the spec md will still appear excluded; that's expected).

## Constraints

- **Never** commit if the verdict is Rejected.
- **Never** skip hooks (`--no-verify`) or bypass signing. If a pre-commit hook fails, fix the underlying issue and create a new commit.
- **Never** push. The orchestrator handles push + PR after this skill returns.
- **Never** `git mv` the spec between `tasks/{todo,inprogress,review,done}/`. The DB tracks status; folder-shuffling is the old file-based queue.
- **Do not** edit the spec md content. If review surfaces follow-ups, write new `tasks/todo/<slug>.md` files instead.
- Process exactly **one** task per invocation.
- If `--allow-empty` feels tempting, stop and re-read Step 4 — an Approved-no-fix-needed with no diff means **no commit**, not an empty commit.
