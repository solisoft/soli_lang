# BUG-001: `soli serve` loader rejects multi-statement controller / model bodies that parse fine standalone

Severity: Medium — blocks any non-trivial controller that uses early-returns or
multi-statement bodies in a fresh `soli new` app.

## Summary

In a freshly scaffolded `soli new <app>` project, any controller method (or
free-standing model `fn`) whose body contains more than one statement is
rejected by the `soli serve` loader at request time with:

```
Parse error at line N: Unexpected token 'EOF', expected identifier at 1:3 at X:Y
```

The same `.sl` file parses cleanly when run directly via `soli path/to/file.sl`
(it errors only at runtime on undefined builtins like `render`/`getenv`, never
during parsing). The error is therefore in the loader path
(`src/serve/app_loader.rs::load_controller` / `load_models`), not in the
parser proper.

The error position `at 1:3 at X:Y` is invariant across edits and stays at
`1:3` even when the offending file is changed. `X:Y` shifts based on file
content but consistently lands on the **second statement** of any multi-line
function body.

## Repro (confirmed with Soli 1.0.3 on `soli new`)

```bash
soli new buggy && cd buggy
```

1. **`config/routes.sl`** — add:
   ```soli
   get("/foo", "foo#show")
   ```

2. **`app/controllers/foo_controller.sl`** — class with a multi-statement `def`:
   ```soli
   class FooController < Controller
       def show(req)
           let name = req["params"]["name"]
           return render("foo/show", {"name": name})
       end
   end
   ```

3. **`app/views/foo/show.html.slv`**:
   ```erb
   <p>Hello <%= name %></p>
   ```

4. `soli serve . --dev` and visit `http://localhost:5011/foo`.

**Expected:** 200 OK, "Hello".
**Actual:** 500 with the cryptic parse error in the dev breakpoint page.

Standalone-parse check that proves the file is syntactically valid:
```
$ soli app/controllers/foo_controller.sl
Error: Type error: Undefined variable 'render' at 3:16
```
i.e. parsing succeeds and only `render` is unbound at the script level.

The same shape with **a single-expression body** works:
```soli
class FooController < Controller
    def show(req)
        return render("foo/show", {"name": req["params"]["name"]})
    end
end
```

## Other shapes that fail under `soli serve` but parse standalone

- Free-standing `fn name(req)` with `let x = ...` then `return ...`
- `fn name(args)` with `if cond ... end` then `return ...`
- Class-method `def name(req)` with inner `if cond ... end` early-return — the
  inner `end` appears to close the method instead of the conditional.

In every case `soli <file.sl>` parses cleanly; only `soli serve` fails.

## Working baseline

`lang/www/app/controllers/blog_controller.sl::show(req)` is multi-statement
with `if cond ... end ... let ... render(...)` and works fine when served via
the lang/www project. The discrepancy between "works under www/" and "fails
under a freshly scaffolded `soli new`" suggests a divergence in the loader
path the new scaffold takes vs. what the in-tree www/ project uses (perhaps
class-based vs. function-based controller registration in
`load_controller` / `derive_routes_from_controller`).

## Suspected location

- `src/serve/app_loader.rs::load_controller` — calls `execute_file` on the
  controller; the parse error is surfaced from there.
- `src/serve/router.rs::extract_function_names` — regex-based scan of `fn `
  declarations; class methods (`def name(req)`) are invisible to it, so
  class-based controllers may go down a different code path that double-parses
  or wraps the source.
- The `at 1:3 at X:Y` position format hints at a span emitted from a generated
  wrapper around the user's source.

## Acceptance criteria

- Multi-statement bodies in controller methods (`def`/`fn`) and free-standing
  model `fn`s load cleanly under `soli serve` in a fresh `soli new` app.
- The "Hello" repro above renders 200.
- Error messages from the loader path point at user-supplied line numbers in
  the offending file, not at a generated wrapper.

## Workspace context

Triggered while building `~/workspace/soli/task-orchestrator/`. **Root cause:**
the view template used `fn` as a loop variable (`<% for fn in columns %>`).
`fn` is a reserved keyword, so the ERB compilation surfaced as a parser path
that errored with the cryptic `expected identifier at 1:3 at X:Y` at
evaluation time — but the controller-relative line numbers in the error and
the fact that the offending code was *in a view*, not the controller, made
this nearly impossible to track down. The bug is therefore the diagnostic,
not the strict-keyword check: any keyword used as a template loop variable
should produce an error that points at the view file and line.

## Companion bug — `Trusted.glob` matches against the wrong string

While debugging the above I also hit `Trusted.glob("/abs/path/*")` returning
an empty array even though the directory has children. Looking at
`src/interpreter/builtins/file.rs` (the `File`/`Trusted` `glob` registration
around L775):

```rust
let pattern = Pattern::new(&pattern_str)        // "/abs/path/*"
    .map_err(...)?;
let path = Path::new(&pattern_str);
let dir_str = path.parent()...                   // "/abs/path"
let entries = fs::read_dir(&resolved_dir)...;
let matches = entries
    .filter(|p| {
        let name = p.file_name()...              // "crm"
        pattern.matches(&name)                   // matches "crm" against full pattern "/abs/path/*"
    })
```

The full pattern string is matched against the *basename* of each entry,
which never succeeds for any pattern containing a `/`. The fix is to either
match `entry.path()` against the full pattern, or strip the directory prefix
out of the pattern before constructing it (so `Pattern::new("*").matches("crm")`
returns true).

Workaround used in the orchestrator: shell out via
`System.run_sync(["ls", "-1", path])` and parse stdout. This worked but means
the orchestrator can't rely on `Trusted.glob` for cross-repo listing.
