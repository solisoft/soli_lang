---
description: Run the full pre-merge verification loop (lint + test + coverage)
---

Run, in order, and report any failure with the exact failing file:line:

1. `soli lint`
2. `soli test --coverage --coverage-min 90.0`

If lint fails: fix the root cause — don't suppress with comments or weaken rules. If coverage drops below 90%: write the missing test, don't lower the threshold.
