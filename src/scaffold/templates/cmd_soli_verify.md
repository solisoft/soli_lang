---
description: Run the full pre-merge verification loop (fmt + lint + test + coverage)
---

Run, in order, and report any failure with the exact failing file:line:

1. `soli fmt`
2. `soli lint`
3. `soli test --coverage --coverage-min 90.0`

`fmt` goes first: it rewrites layout in place, so running it after lint would mean re-linting anyway. Report which files it reformatted — that's part of the diff you're handing off.

If lint fails: fix the root cause — don't suppress with comments or weaken rules. If coverage drops below 90%: write the missing test, don't lower the threshold.
