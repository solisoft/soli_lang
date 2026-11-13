# Soli 2.36: see what your code does

Until now, the way to find out what a Soli program was doing was `print`, or a
`debug()` call and the dev page it opens. 2.36 adds the tools you expect from a
mature language. There is a step debugger for scripts, specs and a running
server, and an allocation profiler. Types are checked across the application's
files and in its views. Errors point at the declaration as well as the call.
The editor finds references and renames across the whole app. Two more pieces
are for production: passkeys that also work in native apps and browser specs,
and presence that holds across several instances.

<figure style="margin:1.5rem auto;max-width:1024px;">
  <img src="/images/blog/soli-2-36.svg" width="1024" height="576" alt="Soli 2.36 in five areas: a step debugger (soli dap, VS Code, attaching to soli serve, conditional breakpoints and logpoints), types (unions, generics, strictness levels, soli check reading views), errors (a declared-here frame, did-you-mean on both engines, named errors on the dev page), the editor (references and rename across the app, view instance variables, a tree-sitter grammar), and production (passkeys with EdDSA, in native apps and in browser specs; presence across instances over SoliDB or SoliKV)." style="display:block;width:100%;height:auto;border-radius:12px;border:1px solid #30363d;background:#0b0d0f;">
  <figcaption style="text-align:center;color:#8b949e;font-size:0.875rem;margin-top:0.5rem;">What 2.36 adds, by area.</figcaption>
</figure>

## A step debugger

`soli dap` is a Debug Adapter Protocol server, so any editor with a DAP client
can drive it. The VS Code extension declares the `soli` debug type: Run and
Debug → "create a launch.json" offers "Debug this file", and there are snippets
for specs and for attaching to a server. Neovim (nvim-dap) and Helix need only a
few lines of configuration, given in the
[debugging guide](/docs/development-tools/debugging).

You get breakpoints, a stack, locals and globals you can expand (arrays, hashes,
object fields; names that look like secrets are redacted), step in, over and
out, and pause. `evaluate` runs in the paused frame, so `total = 0` really
changes the variable. While a debugger is attached, `debug()` acts as a
breakpoint.

A breakpoint can also carry:

- a **condition**, a Soli expression evaluated in the frame;
- a **hit count** (`5`, `>= 5`, `== 5`, `% 10`);
- a **log message**. A logpoint prints `{expression}`s to the debug console
  and does not stop.

It works on both engines. Pass `"engine": "vm"` and the VM stops on the same
statements with the same stack and locals; `dap_test` runs one scenario on both
engines and compares them.

It also debugs specs. With `"mode": "test"`, `program` is a spec file or a
directory, run by `soli test` with one worker. Breakpoints stop in the spec and
in the models, services and helpers it calls.

It also debugs a running server:

```bash
SOLI_DEV_REPL_SECRET=my-dev-token soli serve . --dev --inspect     # DAP on 127.0.0.1:9229
```

The editor attaches with the dev token. A breakpoint in a controller then holds
the request that reaches it. You read `params`, step through the action, and
`continue` sends the response. The handler timeout does not count the time
spent stopped.

`--inspect` refuses to start without `--dev` and binds loopback only, because
an attached debugger runs code in the server. When the editor disconnects, its
breakpoints are cleared and a stopped request resumes.

## Where the memory goes

`--profile` already showed where the time goes. `--profile=alloc` shows where
the bytes go, using the same table, folded stacks and speedscope output:

```soli
def build_rows(count)
  (0..count).map { |i| {"id": i, "title": "Post #{i}"} }
end

def render_rows(rows)
  rows.map { |row| "<li>#{row.title}</li>" }.join("")
end

20.times do
  render_rows(build_rows(5000))
end
```

```
$ soli --profile=alloc rows.sl
Profile: 43.7 MB allocated by Soli code over 27 ms wall time
   self KB  self%   total KB total%  function
   30027.0  67.1%    40535.2  90.6%  <lambda> (rows.sl)
    5817.8  13.0%    10843.9  24.2%  render_rows (rows.sl:5)
    4690.3  10.5%    29690.6  66.3%  build_rows (rows.sl:1)
    4219.9   9.4%    44755.1 100.0%  <main>
```

It counts bytes allocated, not bytes still live. The binary's allocator reports
every allocation, and the profiler charges the bytes to the Soli stack at its
next poll. When no allocation profile is running, that costs one atomic load per
allocation.

## Types that follow the whole application

Type annotations are optional in Soli. When you write them, 2.36 makes them
worth more.

**Unions and generics.**

```soli
class Box<T>
  value: T

  new(value: T)
    @value = value
  end

  def get -> T
    @value
  end
end

def first<T>(items: T[]) -> T
  items[0]
end

def label(id: Int | String | nil) -> String
  return "none" if id.nil?

  "##{id}"
end

box = Box.new(42)            # a Box<Int>
let name: String = box.get   # error[E0401] type-mismatch: expected String, found Int
```

The checker infers `T` from the constructor's or the call's arguments and
substitutes it into the members. At run time generics are erased: `Box<Int>`
checks that the value is a `Box`. Union return types are checked at run time on
both engines.

**Strictness levels.** Set `off`, `loose`, `standard` (the default) or
`strict` with `# soli: strict` at the top of a file, with `soli check --level`,
or with `level` under `[check]` in `soli.toml`. `loose` keeps only the errors
that would certainly fail at run time. `strict` also requires every parameter
and return type:

```
$ soli check strict.sl
strict.sl: Type error: Missing type annotation: parameter 'x' of double() at 2:12
 --> strict.sl:2:12
  |
2 | def double(x)
  |            ^
  |
  = note: error[E0404] missing-annotation — `soli explain E0404`
```

**Across files.** `soli check` used to know another file's top-level names
only, typed as `Any`. It now reads their declarations. A helper's `def` is
checked for arity and parameter types, a class for its members, and
`class PostPolicy < ApplicationPolicy` finds its parent in another file. Along
the way, `Klass.new(…)` returns an instance of `Klass` instead of `Any`, so
members are now checked on every instance. Before, none of them were.

**And the views.** `soli check` now reads every `.slv` too (`--no-views` skips
them). The code in a view's tags is checked with the template helpers, the
`def`s of `app/helpers/` and the components in scope. A misspelt helper is an
error before the page renders. `@x` and the locals passed to the view stay open.
On www's 199 views and on about 2,500 views from fifteen other applications, it
reports nothing.

## Errors that show both ends

When a call has the wrong number of arguments, the error now also points at the
declaration:

```
$ soli check arity.sl
arity.sl: Type error: Wrong number of arguments: expected 2, got 1 at 5:7
 --> arity.sl:5:7
  |
5 | print(total(3))
  |       ^^^^^
 ::: arity.sl:1:1
  |
1 | def total(price: Int, quantity: Int) -> Int
  | --- declared here
  |
  = note: error[E0403] wrong-arity — `soli explain E0403`
```

The VM now suggests local names. Bytecode slots have no names, so a misspelt
local under `--vm` used to get no "did you mean". The compiler now keeps each
function's local names for the error message, and both engines print the same
line:

```
Error: Runtime error: Undefined variable 'greting' (did you mean 'greeting'?) at 3:24
```

`soli test` draws the same code frame under a failing test. The dev error page
names the error class (`NameError`, `ZeroDivisionError`) and its stable code
(`error[E0301] undefined-variable`, with the `soli explain` text one click
away), and puts a caret under the column. Before, it labelled every handler
error as a "Breakpoint".

## The editor knows the whole app

- **References and rename** now work across `app/`, `config/` and `tests/`, and
  inside the `<% %>` tags of views. That covers a class, a top-level `def`, a
  helper, a constant, a method or a field. A local stays in its own file.
  Renaming a method that several classes declare is refused, and the error says
  how many classes declare it.
- **Hover** shows the `#` comment block written above a `def`, a class or a
  constant.
- **View instance variables.** In a view, an `@x` that no action rendering the
  view sets is a warning, because it reads `nil`. The LSP follows the actions
  that render the view, the methods they call, `before_action` hooks and parent
  controllers. Typing `@` completes the names those actions set. When the LSP
  cannot tell (layouts, partials, a computed `render`), it says nothing.
- **A tree-sitter grammar** in `editors/tree-sitter-soli/` provides highlights,
  locals, folds, indents and injections, plus queries for `.slv` views. Setup
  for Neovim, Helix and Zed is in its README. 359 of the repository's 373 `.sl`
  files and all 230 of its views parse without an error.

## Passkeys, further

- `WebAuthn` verifies Ed25519 credentials (EdDSA), and the default
  `pubKeyCredParams` are now `[-7, -8, -257]`.
- A `packed` self-attestation has its signature checked; a bad one fails the
  registration. The record says which attestation it got: `none`, `self` or
  `unverified`.
- `AppLinks.android(…, {"passkeys": true})` and `AppLinks.apple(…,
  {"passkeys": true})` let a native app use the site's passkeys.
  `AppLinks.android_origin(fingerprint)` returns the origin an Android app
  reports.
- Browser specs get a virtual authenticator, so you can test sign-in with a
  passkey end to end in Chrome:

```soli
authenticator = virtual_authenticator()
# … register, then sign in through the page …
credentials = virtual_credentials(authenticator)
```

See [WebAuthn](/docs/builtins/webauthn) and the browser testing guide.

## Realtime across instances

`SOLI_REALTIME_BUS=solikv` (also accepted as `redis` or `valkey`) carries
broadcasts between `soli serve` instances over `PUBLISH`/`SUBSCRIBE`, alongside
the existing `solidb` bus. On either bus, three things now work across
instances:

- **Presence.** `track`, `list_presence` and `presence_count` merge every
  node's users. Nodes resync every 10 seconds, and a node silent for 30 seconds
  is evicted, its users leaving.
- **`ws_send` and `ws_close`** reach a connection held by another instance.
- **`ws_count({"cluster": true})`** counts connections on every node.

See [WebSockets](/docs/core-concepts/websockets).

## Behaviour changes

Four changes can alter what existing code does:

- **`break` in a block stops the iteration**, as in Ruby. In
  `[1, 2, 3, 4].each do |n| break if n == 3 … end`, the block runs for 1 and 2
  only, on both engines.
- **`for item in hash` iterates the keys.** Before, it was refused.
- **One rule for where a trailing block binds.** A trailing block goes to
  `&block` if the method declares one. Otherwise it goes to the last parameter,
  if no argument filled it. Otherwise it is dropped. The two engines used to
  disagree.
- **A builtin's name assigned inside a function is a local**, so the builtin is
  no longer called by mistake there. A bare `Klass.new` builds an instance.

Beyond these, there are many fixes, most of them places where the VM and the
interpreter disagreed. The full list is in the
[changelog](/docs/getting-started/changelog#v2-36-0).
