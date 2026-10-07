---
description: Scaffold a full RESTful resource (model + migration + controller + views + route + spec)
argument-hint: <singular-name>   e.g. /soli-resource post
---

Resource name: `$1` (singular). Plural form for routes/views is `${1}s` — adjust manually for irregular plurals (person → people).

Prefer the full-resource generator first:

1. `soli generate scaffold $1` (optionally with field args, e.g. `title:string body:text`).
   This writes model, controller, views, migration, routes, and
   `tests/controllers/${1}_controller_spec.sl`.
2. `soli db:migrate up` once you have filled any migration details you care about.
3. Rewrite the controller E2E spec the scaffold writes: it checks status codes
   only, and its show/update/delete tests use id `1` without creating that
   record. Create the records each test needs, and test every action's
   refusals as well as its success — invalid input (422, the field in error,
   nothing written), an unknown id (404), a user who may not act (401/403) —
   asserting the stored state, not just the status. Add a model unit spec by
   hand for validations and raised errors (scaffold does not emit one). See
   `tests/CLAUDE.md` → "What a spec must prove".
4. Run `/soli-verify` and fix any failures.

If you need pieces one at a time instead of scaffold:

1. Write `app/models/$1.sl` (fields, validations, associations).
2. `soli db:migrate generate create_${1}s` → fill `up`/`down`.
3. Write `app/controllers/${1}s_controller.sl` (or match the singular-controller
   naming scaffold uses: `${1}_controller.sl` / `${1}Controller`).
4. Edit `config/routes.sl`: add `resources("${1}s")` if it isn't already there.
5. Stub `app/views/${1}s/{index,show,new,edit}.html.slv`.
6. Write `tests/${1}s_controller_spec.sl` (or `tests/controllers/...`): every
   action's success *and* its refusals — see `tests/CLAUDE.md` → "What a spec
   must prove".
7. Run `/soli-verify`.

**Pause after the model/migration exist** — the user fills in fields and
validations before you push on views and specs.
