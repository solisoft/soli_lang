# no_manual_404_after_find

`Model.find(id)` raises `RecordNotFound` when the id doesn't exist, and the
request handler turns that into a `404` automatically. So a manual
`if record.nil? { return 404 }` guard right after `.find` is dead code — it can
never run, because `.find` already threw. (When you genuinely want "or nil",
use `find_by` / `first_by` instead.)

The stub adds that dead guard. Rewrite `stub.sl` so it:

- removes the unreachable nil-check after `.find`
- keeps the same behavior for the found-record case (the tests must still pass)

`soli lint` flags a nil-check on the result of `.find(...)` as
`idiom/manual-find-guard`, so a clean controller reports zero issues.
