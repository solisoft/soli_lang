# no_redundant_model_import

In a Soli MVC app every file under `app/models/` is auto-loaded by `soli serve`
and the REPL, so a controller never needs to `import` a model — the class is
already in scope. The explicit import is dead weight.

The stub is a controller that redundantly imports its model. Rewrite `stub.sl`
so it:

- removes the `import "../models/..."` line
- keeps the same behavior (the tests must still pass)

`soli lint` flags model imports inside `app/controllers/` as
`style/redundant-model-import`, so a clean controller reports zero issues.
