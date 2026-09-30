---
description: Scaffold a full RESTful resource (model + migration + controller + views + route + spec)
argument-hint: <singular-name>   e.g. /soli-resource post
---

Resource name: `$1` (singular). Plural form for routes/views is `${1}s` — adjust manually for irregular plurals (person → people).

Run, in order:

1. `soli generate model $1`
2. `soli generate migration create_${1}s`
3. `soli generate controller ${1}s`
4. Edit `config/routes.sl`: add `resources("${1}s")` if it isn't already there.
5. Stub `app/views/${1}s/{index,show,new,edit}.html.slv` if the generator didn't.
6. Stub `tests/${1}s_controller_spec.sl` with `index` / `create` / `update` / `destroy` cases.
7. Run `/soli-verify` and fix any failures.

**Pause after step 4** — the user fills in fields and validations on the model before continuing to migrations and views.
