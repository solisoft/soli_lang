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
| `smell/duplicate-methods` | two methods of one class sharing a name |
| `smell/deep-nesting` | more than four levels |
| `smell/undefined-local` | a bare name never assigned in this scope |
| `smell/dangerous-server-builtin` | an injection sink reached from a request layer |
| `smell/closure-cycle` | a closure stored onto `this` — the instance and the closure keep each other alive |

`smell/undefined-local` exists because `let` is optional: `x = 1` declares `x`,
so a typo like `optsx.push(...)` fails only at run time. The rule finds it
statically.

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
| `idiom/nil-comparison` | `user == null` | `user.nil?` / `user.present?` |
| `idiom/prefer-blank` | `name == ""` | `name.blank?` (covers nil too) |
| `idiom/prefer-includes` | three or more `==` on one value | `["up", "late"].includes?(status)` |
| `idiom/prefer-to-s` | `record.title ?? ""` | `record.title.to_s` |
| `idiom/manual-find-guard` | a nil-check after `Model.find` | nothing — `find` raises |

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
```

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

Naming a single file keeps the narrower per-file view: a loose script has no
project around it. A neighbour's *type* is not inferred, only its existence —
what it is would mean checking the file that declares it, so its calls are
unconstrained rather than wrongly constrained.

`render` and `redirect` stay unknown to the checker on purpose: they only mean
anything inside a request, and `serve` does not type-check.

## Editor integration

The VS Code / Cursor extension speaks LSP: real-time linting, hover
documentation, autocomplete, go-to-definition, find-references.

```bash
cd editors/vscode
vsce package    # then install the generated .vsix
```

| Setting | Default |
|---|---|
| `soli.lsp.enable` | `true` |
| `soli.lsp.executablePath` | `"soli"` |
| `soli.lint.enable` | `true` |
| `soli.lint.onSave` | `true` |

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
