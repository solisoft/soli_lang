# Linting

Static analysis that catches style issues and code smells without executing
your code.

```bash
soli lint                  # every .sl file under the current directory
soli lint src/             # a directory
soli lint app/main.sl      # one file
```

Exit code `0` when clean, `1` when anything was found.

Each issue is one line — path, line, column, rule, message:

```text
app/main.sl:12:5 - [naming/snake-case] variable 'myVar' should use snake_case
app/main.sl:30:9 - [smell/unreachable-code] unreachable code after return statement

2 issue(s) found in 1 file(s)
```

The one-line form is the default, so CI output you grep does not change. Two
flags add to it:

```bash
soli lint --frames          # a code frame (source line, caret, hint) under each issue
soli lint --format json     # one JSON document instead, for editors and CI
soli explain smell/undefined-local   # what a rule wants, and how to silence it
```

```text
app/main.sl:3:9 - [smell/undefined-local] variable 'totl' is read but never assigned in this scope (did you mean 'total'?)
 --> app/main.sl:3:9
  |
3 |   print(totl)
  |         ^^^^
  |
  = help: did you mean 'total'?
  = note: `soli explain smell/undefined-local`
```

The JSON shape is described in [`error-codes.md`](error-codes.md#machine-readable-output).

## The rules

### Naming

| Rule | What it wants |
|---|---|
| `naming/snake-case` | variables, functions, methods and parameters in `snake_case` |
| `naming/pascal-case` | classes and interfaces in `PascalCase` |

```soli
# Bad
let myVar = 10
def processData end
class my_class end

# Good
let my_var = 10
def process_data end
class MyClass end
```

### Style

| Rule | What it wants |
|---|---|
| `style/empty-block` | a block should do something, or not exist |
| `style/line-length` | at most 120 characters |
| `style/redundant-model-import` | no `import "../models/*.sl"` inside `app/controllers/` — models are auto-loaded |

### Code smells

| Rule | What it catches |
|---|---|
| `smell/unreachable-code` | code after `return` |
| `smell/empty-catch` | a `catch` that swallows the error silently |
| `smell/duplicate-methods` | two methods of one class sharing a name — an instance `def kind_label` and a `static def kind_label` are distinct and pass |
| `smell/duplicate-global` | two files of an app declaring the same top-level class, module, `def` or `const`: the one loaded last would replace the other, so `soli serve` refuses to boot on it too. Controllers' top-level `def`s (function-style actions) are exempt |
| `smell/deep-nesting` | more than four levels |
| `smell/undefined-local` | a bare name never assigned in this scope |
| `smell/dangerous-server-builtin` | an injection sink reached from a request layer |
| `smell/closure-cycle` | a closure stored onto `this` — the instance and the closure keep each other alive |
| `smell/shadowed-builtin` | a local named after a builtin, in a scope that also calls the builtin |

`smell/undefined-local` exists because `let` is optional: `x = 1` declares `x`,
so a typo like `optsx.push(...)` fails only at run time. The rule finds it
statically.

`smell/shadowed-builtin` flags a local — a bare assignment, or a parameter
annotated with a value type (`options: Hash`) — named after a global the runtime
installs, when the same scope also calls that global. The call then reaches the
local, not the builtin; and the tree-walking engine reads a bare assignment to
an existing global as rebinding it, so the builtin is gone for the rest of the
program:

```soli
def register(path)
  options = "a#b"          # flagged: options() is a builtin...
  options(path, options)   # ...and this calls the String
end
```

Only a scope that uses the builtin is reported — a local `post`, `json` or
`stream` in an action that never calls `post(...)` is ordinary Soli. Not
reported either: `let options = …` (the documented way to shadow a function on
purpose), a lambda assigned to the name, an unannotated parameter the body
calls (a callback), and a name the app itself declares. Blocks belong to the
enclosing scope, and the file's top level is a scope too. `soli check` names
the builtin in the same situation: assigning a value over one says so and
suggests renaming, and calling a `let`-shadowed name reports that the local
shadows the builtin (`` `options` is a local String here, shadowing the builtin
options(); rename it ``).

`smell/closure-cycle` flags a closure assigned onto the instance —
`this.x = fn(...) ...`, `@x = |y| ...`, `this.handlers["k"] = fn ...`. The closure
captures the method's environment, which holds `this`; stored on `this`, it
makes a reference cycle, so neither the instance nor anything it holds is ever
freed. In a long-lived worker that is a leak per object. Store a method **name**
and dispatch on it, or pass the closure in per call:

```soli
# Leaks: the instance holds a closure that holds the instance
this.on_save = fn(record) { this.log(record) }

# Instead: keep the name, call the method when needed
this.on_save = "log"
```

The rule only watches instance code. Where `this` is the class — a
`static { … }` block, a `static def`, class-level statements — a closure stored
on it is held by a class that lives as long as the process, so the documented
controller hooks (`this.before_action = fn(req) { … }`,
`this.after_action = fn(req, response) { … }`) are not flagged. The body of such
a lambda is instance code again: a closure it stores on `this` is flagged.

`smell/dangerous-server-builtin` fires on `db_query_raw`, `Trusted.*`,
`System.shell` / `System.shell_sync`, and backtick command substitution, when
called from `app/controllers/`, `app/middleware/` or `app/views/`. These are
powerful primitives that become injection or traversal sinks once fed
request-controlled data, and the diagnostic names the safe form for each:

- `db_query_raw` → a parameterised `@sdbql{ … #{value} … }` block, or
  `Model.where("x = #{v}", { "v": v })`
- `Trusted.*` → the jailed `File.*` (`read` / `write` / `exists`), which keeps
  every operation under the app root
- `System.shell` / backticks → `System.run(["prog", "arg1", …])` with an argv
  array, which never invokes a shell

Models, migrations and tests are out of scope: those layers legitimately use
these APIs against operator-controlled data.

```soli
def example
  return 42
  print("never reached")   # smell/unreachable-code
end

# Bad — the error vanishes
try
  risky()
catch e
end

# Good — at least say something
try
  risky()
catch e
  print("Error: " + str(e))
end
```

### Idioms

Correct code that reads better with a builtin.

| Rule | Instead of | Write |
|---|---|---|
| `idiom/nil-comparison` | `user == nil` / `user != nil` (or `null`) | `user.nil?` / `!user.nil?` |
| `idiom/prefer-blank` | `name == ""` | `name.blank?` (covers nil too) |
| `idiom/prefer-includes` | three or more `==` on one value | `["up", "late"].includes?(status)` |
| `idiom/prefer-to-s` | `record.title ?? ""` | `record.title.to_s` |
| `idiom/manual-find-guard` | a nil-check after `Model.find` | nothing — `find` raises |

`idiom/prefer-includes` costs nothing at run time. Where code runs on the
bytecode VM (production `soli serve`, scripts), the list of literals in
`["up", "late"].includes?(status)` is built once, when the code is compiled:
each evaluation only compares `status` with its elements, which makes the
rewrite faster than the `==` chain it replaces.

`idiom/prefer-to-s` is about honesty as much as brevity. Coalescing to an empty
string means "render this, and render nothing when it is nil", which `.to_s`
says in one call. It also fixes a type that changes with the data: `count ?? ""`
is a number when there is one and a string when there is not. A *real* fallback
(`name ?? "Guest"`) is left alone — it says something `.to_s` cannot.

`idiom/manual-find-guard` is about dead code. `Model.find` raises
`RecordNotFound` on a miss, which the request handler turns into a 404, so the
guard after it never runs:

```soli
# Bad — dead code: .find already raised
post = Post.find(id)
return not_found() if post.nil?

# Good
post = Post.find(id)

# When you want nil on a miss, ask for it
post = Post.find_by("slug", slug)
```

The rule only looks at `.find` called on a class name (`Post.find`,
`Admin::User.find`). Every other `find` returns nil on a miss — `Array#find`,
`Hash#find`, `XmlNode#find`, and a query chain's `.find { … }`, which runs over
the loaded rows — so a guard after `doc.find("item")`, `@doc.find(…)` or
`xs.find { |x| … }` is live code and is left alone.

### Security

`security/unfiltered-mass-assignment` — `Model.create(params)`, `.update` and
`.create_many` in `app/controllers/` or `app/services/` persist every posted
key, and Soli is schemaless, so "every key" means every key an attacker sends.
Whitelist first. It does not fire on a hash literal, or on `permit` /
`_permit_params`.

```soli
# Bad
Post.create(params)
Post.create(req["json"])

# Good
Post.create(permit(params, {"title": true, "body": true}))
Post.create(this._permit_params(params))
```

### Components

`component/props` — a `props(...)` declaration must use string literals, with no
duplicates. Missing or unknown props are checked at render time under `--dev`,
not here.

```soli
props("title", "title")   # duplicate
props("title", x)         # not a string literal
props("title", "value")   # good
```

### OpenAPI doc comments

`docs/openapi` — in `app/controllers/`, a mistake in an action's
[OpenAPI doc comments](openapi.md#documenting-an-action): an unknown tag, a
`@response` without a status code, a `@param` without a name, or a `@body` /
`@response` that starts like JSON but does not parse. The spec tolerates all of
them, so without the rule a typo just disappears.

```soli
# @returns 200            # unknown tag — @response
# @response ok Fine       # needs a status code
# @body {"title": }       # not JSON
def create
```

## Locale files are skipped

Translation tables are data that happens to be written in Soli — long sentences
in a hash literal, one key per line. Style rules over them produce hundreds of
`style/line-length` hits nobody will act on, drowning the real findings.
Walking a directory skips:

- anything under a directory named `locales/` (the `config/locales/` convention);
- a file whose stem is `locale_<tag>` or `<tag>_locale`, where the tag looks
  like a locale code — `locale_fr.sl`, `locale_zh-Hans.sl`, `pt_BR_locale.sl`.

The tag check is narrow on purpose, so helpers *about* locales keep being
linted: `locale_helper.sl` and `locale_switcher.sl` are code, not data. Skipped
files are counted in the summary, and naming one explicitly always lints it:

```bash
soli lint app/helpers/
# No issues found. (9 locale files skipped)

soli lint app/helpers/locale_fr.sl   # the escape hatch
```

## Suppressing a warning

When a finding is a known false positive or a deliberate exception, say so
inline.

```soli
# soli-lint-disable-next-line smell/dangerous-server-builtin
if Trusted.is_dir(wt_path)
  ...
end

Trusted.read(p)  # soli-lint-disable-line smell/dangerous-server-builtin
```

Block form, for several adjacent exceptions:

```soli
# soli-lint-disable smell/dangerous-server-builtin
exists = Trusted.is_dir(path)
data   = Trusted.read(path)
# soli-lint-enable smell/dangerous-server-builtin
```

- Omit the rule name to suppress every rule (`# soli-lint-disable`); pass a
  comma-separated list to scope to several.
- An `enable` for a specific rule re-enables only that rule, even when the
  preceding `disable` was blanket.
- A block `disable` with no matching `enable` runs to the end of the file.
- Prefer naming the exact rule, so unrelated warnings still surface.

## Type checking: `soli check`

Where `soli lint` catches style and smells, `soli check` runs Soli's optional
type system over your code **without executing it** — for CI, or a pre-commit
hook. It resolves imports and reports each mismatch with a `file:line:column`,
exiting non-zero when any are found.

```bash
soli check                 # the current project
soli check app/models      # a directory
soli check app/models/user.sl
soli check --format json   # one JSON document, for editors and CI
```

Every type error of a file is reported, each on the line it always printed and
with a code frame under it — the source line, a caret, the "did you mean"
suggestion and the error code:

```text
app/models/user.sl: Type error: Undefined variable 'ponits' (did you mean 'points'?) at 12:5
  --> app/models/user.sl:12:5
   |
12 |     ponits + bonus
   |     ^^^^^^
   |
   = help: did you mean 'points'?
   = note: error[E0301] undefined-variable — `soli explain E0301`

1 error(s) in 1 of 38 file(s)
```

The codes are listed in [`error-codes.md`](error-codes.md).

### A directory is checked as one namespace

A running server loads `app/controllers`, `app/models`, `app/services`,
`app/policies`, `app/middleware`, `app/mailers`, `config` and `stdlib` into
**one** environment, so a helper declared in one file is callable from its
neighbours with no import. Given a *directory*, `soli check` reads the top-level
declarations of all of them first, and checks each file knowing what its
neighbours declare.

Two consequences:

- **A project declaration overrides a builtin of the same name**, because that
  is what happens at run time. A project defining its own three-argument
  `input` is checked against that one.
- **A name declared without `let` counts.** `MAX_WIDTH = 280` at the top of a
  file declares `MAX_WIDTH` for the whole project, exactly as `const` would.

A target *inside* an application — `soli check app`, `soli check
app/controllers`, `soli check app/models/user.sl` — reads the auto-loaded set of
the application's root (the nearest directory up holding `soli.toml` or
`config/routes.sl`), and still checks only the files it names. `soli check app`
used to look for `app/app/models` and report every model as undefined.

A neighbour is known by its declaration, not only its name:

- **A `def` is checked for arity** — at least its parameters without a
  default, at most all of them — and for the types of its annotated
  parameters, wherever it is called from. `report_title("x", "y")` against a
  sibling's `def report_title(name)` is `Wrong number of arguments: expected
  1, got 2`.
- **A class is known by its members.** `Pricing.new()` (or `new Pricing()`)
  is a `Pricing`, so `pricing.totl(3)` is `Cannot access member 'totl' on type
  'Pricing' (did you mean 'total'?)`, whichever file declares `Pricing`.
- **A parent in another file is resolved.** `class PostPolicy <
  ApplicationPolicy` inherits the sibling's methods, and a typo in one of them
  is reported instead of answered with `Any`.

What stays open: a `Model` subclass's fields (the schema is the database's),
a class whose ancestry leaves the application (`< Controller`), and a name two
files declare — which one wins depends on load order, so it is only known to
exist. An instance variable a method or the constructor assigns (`@total =
…`) is a field of the class, declared or not, as on both engines. The class
of an `app/jobs/*_job.sl` file has the job facade (`perform_later`,
`perform_now`, `perform_in`, `perform_at`, `schedule_cron`) the loader gives
it, and a static method read on its class (`Feed._text(…)`) is not refused
for being `private`: neither engine enforces it there.

A loose file or directory with no application around it keeps the narrower
per-file view.

`render` and `redirect` stay unknown to the checker on purpose: they only mean
anything inside a request, and `serve` does not type-check.

### Strictness levels

How much `soli check` reports is set per file, per run or per project:

| Level | Reports |
|---|---|
| `off` | nothing — the file is not checked |
| `loose` | only what certainly fails at run time: an undefined name or type (`E0301`–`E0303`), an unknown member (`E0304`), calling what is not a function (`E0402`), a wrong number of arguments (`E0403`) |
| `standard` | everything the checker knows — the default |
| `strict` | `standard`, and every parameter and return type must be written (`E0404`) |

```soli
# soli: strict
def total(prices: Array) -> Int   # `def total(prices)` is two E0404 errors
  prices.length
end
```

- **`# soli: <level>`** among the comment lines at the top of a file sets that
  file's level, and wins over everything else. `soli script.sl` honours it
  too: `# soli: off` runs a script unchecked, as `--no-type-check` does for
  one run.
- **`soli check --level <level>`** sets it for the run.
- **`level` under `[check]` in `soli.toml`** sets it for the application, when
  no flag is given:

  ```toml
  [check]
  level = "strict"
  ```

At `strict`, a `&block` parameter and a constructor's return need no type, and
a `def` that returns nothing says `-> Void`.

### Views

`soli check` reads the `.slv` views too (`--no-views` leaves them out; a view
named on the command line is always checked). The code of a view's tags is
checked as one program, with the template's line numbers, against what a
rendering sees: the view helpers (`h`, `t`, `link_to`, `partial`,
`form_with`, `public_path`…), the `def`s of `app/helpers/`, the classes of
`app/components/`, and the application's models and services.

```erb
<%= monye(post.cents) %>        <%# Undefined variable 'monye' (did you mean 'money'?) %>
<%= money(post.cents, "EUR") %> <%# Wrong number of arguments: expected 1, got 2 %>
<%= Invoice.new.totl %>         <%# Cannot access member 'totl' on type 'Invoice' %>
```

What a view cannot know is left open: `@title` is whatever the controller
set, and a bare name it reads — `post`, `items` — may be a local the caller
passed (a partial's, a component's), so neither is an error. A name it
*calls* must exist somewhere. A controller's own top-level `def`s
(function-style actions) are not view helpers and are not seen. Two tags on
one source line are read one line apart, so an error after them may be
reported a line low.

In any file of an application, an undefined `*_path` or `*_url` is a route
helper (`post_path(@post)`, `posts_url()`): the router names them when
`config/routes.sl` runs, which the checker does not do.

### What else a file sees, by where it lives

The checker gives a file the names the runtime gives it:

| File | Also in scope |
|---|---|
| `app/`, `config/`, `stdlib/`, a spec | the request scope (`req`, `params`, `session`, `render`, `redirect`…) |
| `db/migrations/` | the runner's prelude: `_db` (with `_db_host`, `_db_name`) and `MigrationDb` |
| a module a sibling script imports and opens an `eui_window` with | the EUI widget catalogue (`button`, `row`, `canvas`…) |
| a spec (`tests/`, `*_spec.sl`) | the test DSL (`describe`, `test`, `expect`…) |

A file of `app/helpers/` does **not** get the view-only helpers
(`datetime_format`, `substring`, `t`, `paginate`…): a helper runs in the
application's environment, and calling one of them there is a 500 ("Cannot
call non-function value"), so `soli check` reports it. Use the methods
(`DateTime.from_unix(ts).format("%Y")`, `text.substring(0, 3)`).

### What it does not report

`soli check` is a gate you can trust only if it does not cry wolf, so code that
runs is not refused for being Soli:

- **Any value is a condition.** `if xs.length`, `if hash`, `while node`,
  `flag ? a : b` are truthiness tests (`false`, `nil`, `0`, `""`, `[]` and `{}`
  are falsy), whatever the type.
- **A local holds whatever was last put in it.** Reassigning an unannotated
  local with another type widens it — `total = 0` then `total = total * 1.5`
  makes it a `Float`, a String later given a Hash becomes `Any` — instead of an
  error. A binding whose type you wrote down still refuses: `let x: Int = 1`
  then `x = "s"`, or a typed parameter, is a mismatch. So is a bare
  `helper = 5` where `helper` is a builtin or a `def` of the same file: both
  engines rebind the function rather than make a local, so the next
  `helper()` fails. Rename the local, or write `let helper = 5` to shadow it.
- **`T?` takes `nil`.** `def f(x: Int?)` accepts `f(nil)` and `f(2)`, and
  `x = nil` or `return nil` into an `Int?` slot is fine. A value read from it
  is used as the `T` (there is no narrowing after `x.nil?` yet).
- **A union takes any of its members.** `def f(x: Int | String)` accepts
  `f(1)` and `f("a")`, refuses `f(1.5)`; a union *value* goes only where every
  member fits (`let n: Int = int_or_string` is refused). A value read from a
  union is `Any` until the checker narrows, so its methods are not checked.
- **One mistake does not hide the rest.** Each statement of a `def`, a method,
  a block or a `describe` body is checked even after an earlier one failed, so
  every error of a file is reported — and a name the failing line would have
  bound (`x = broken(…)`) does not then read as undefined on every later line.

The in-repo corpus — the docs site, `examples/`, and what `soli new`,
`soli new --eui`, `soli generate auth` (with and without `--passkeys`) and
`soli generate scaffold` write — checks with **zero** errors, and a test keeps
it there (`tests/check_corpus_test.rs`, a ratchet that may only go down).

## Editor integration

The VS Code / Cursor extension starts `soli lsp`: real-time linting (the
path-aware rules included), hover documentation, autocomplete,
go-to-definition, find-references. When the server cannot run, it falls back
to `soli lint` on open and save.

```bash
cd editors/vscode
npm ci && npm run package    # then install soli-language.vsix
```

| Setting | Default |
|---|---|
| `soli.lsp.enabled` | `true` |
| `soli.lsp.path` | `""` (`PATH`, then `~/.local/bin`, `~/.cargo/bin`, `/usr/local/bin`) |
| `soli.lint.enable` | `true` (fallback only) |
| `soli.lint.onSave` | `true` (fallback only) |

For an editor that takes an LSP server directly:

```lua
require('lspconfig').soli.setup({
  cmd = {"soli", "lsp"},
  filetypes = {"soli"},
  root_dir = lspconfig.util.root_pattern("soli.toml", ".git"),
})
```

## See also

- [`editor-integration.md`](editor-integration.md) — full editor setup
- [`formatting.md`](formatting.md) — `soli fmt`
- Rendered page: `/docs/development-tools/linting`
