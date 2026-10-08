# The Interpreter in Every Closure

Production Soli runs controller actions, middleware and, since the last change,
`before_action` hooks on its bytecode VM. Once hooks were moved, we went looking for
whatever else production still ran on the tree-walking interpreter, and timed each
one. A model with four custom validators took 5.5 ms to fail one `create`. A
`scope` cost about half a millisecond. A method added to `String` cost a quarter of
one per call.

The interpreter wasn't what made them slow. A validator such as
`value != "bad"` takes well under a microsecond to evaluate on either engine. What
took the time came before it: every one of these calls built a whole new
interpreter, registering every builtin again, and then didn't use it.

This post covers where those 270 µs went, the two constructs that paid them twice,
the fix (a different constructor, at a dozen call sites), and a leak we found while
measuring: in production, `render_json` sent fields that `as_json` was written to
keep out.

<figure style="margin:1.5rem auto;max-width:1024px;">
  <img src="/images/blog/the-interpreter-in-every-closure.svg" width="1024" height="576" alt="Before: each call of a validator closure went through Interpreter::default(), which built a new interpreter and registered every builtin again, about 270 microseconds, to run a body that takes under a microsecond in its closure's own environment. Now the body runs there directly, with no registry. CPU per request: validators 5.49 ms to 55 µs (x100), model scopes 8.67 ms to 58 µs (x150), a method added to String 2.74 ms to 22 µs (x122), perform_now 1.41 ms to 20 µs (x72), mailer actions 2.93 ms to 55 µs (x53), scopes calling app code 1.22 ms to 25 µs (x49)." style="display:block;width:100%;height:auto;border-radius:12px;border:1px solid #30363d;background:#0b0d0f;">
  <figcaption style="text-align:center;color:#8b949e;font-size:0.875rem;margin-top:0.5rem;">Most of what these calls cost was spent before the closure ran a single line.</figcaption>
</figure>

## Six constructs, one bench app

We built one app with a route per construct. Each route repeats the construct a few
times, so that it accounts for most of the request. Nothing here is exotic; this is
code from the guides.

**Custom validators.** All five creates fail on `title`, so nothing is written and
the route measures validation alone:

```soli
# app/models/gadget.sl
class Gadget < Model
  validates("title", {"presence": true})
  validates("name", {"custom": fn(value) { value != "bad" }})
  validates("code", {"custom": fn(value, record) { record["name"] != value }})
  validates("flag", {"custom": fn(value) { value != "off" }})
  validates("nickname", {"min_length": 2, "if": fn(record) { record["strict"] == true }})
end
```

```soli
# GET /validate
def validate(req)
  errors = 0
  (0..5).each do |i|
    gadget = Gadget.create({"name": "n#{i}", "code": "c", "flag": "on", "strict": true, "nickname": "ab"})
    errors = errors + gadget._errors.length
  end
  render_json({"errors": errors})
end
```

**Scopes.** Six chains of three scopes, built into a query with `to_query`, so no
database round-trip is involved:

```soli
class Article < Model
  scope("published", fn() { this.where("status = @s", {"s": "published"}) })
  scope("recent", fn() { this.order("created_at", "desc") })
  scope("by_author", fn(author) { this.where("author = @a", {"a": author}) })
end

Article.published.recent.by_author("a#{i}").to_query
```

**A tenant scope** that calls application code and reads a constant, the way a
multi-tenant app restricts every query:

```soli
# app/models/tenant_functions.sl
const TENANT_PREFIX = "acme"

def current_tenant()
  TENANT_PREFIX + "-eu"
end

def normalize_tag(tag)
  tag.downcase.trim
end
```

```soli
# app/models/thing.sl
module Tenanted
  included do
    scope("for_tenant", fn() { this.where({"tenant_id": current_tenant()}) })
    scope("tagged", fn(tag) { this.where({"tag": normalize_tag(tag)}) })
  end
end

class Thing < Model
  include Tenanted
end

Thing.for_tenant.tagged("  Rust ").to_query
```

**A method added to a built-in type**, called ten times:

```soli
String.define_method("shout", fn() { this + "!!!" })

"x#{i}".shout
```

**A job run inline** and **a mailer action**, five of each:

```soli
class EchoJob
  static def perform(args: Hash)
    args["n"] * 2
  end
end

class PingMailer < Mailer
  def ping(email)
    this.mail(to: email, subject: "Ping", html: "<p>ping</p>")
  end
end

EchoJob.perform_now({"n": i})
PingMailer.ping("u#{i}@example.com")
```

On 2.18.3, 16 workers, CPU per request:

| Route | CPU per request | Requests per second |
|---|---:|---:|
| `render_json` of a hash (the floor) | 15.8 µs | 762k |
| 5 creates, four closure validators each | 5,488 µs | 2.9k |
| 6 chains of 3 scopes | 8,671 µs | 1.8k |
| 2 tenant scopes | 1,224 µs | 13k |
| 10 calls of `String#shout` | 2,737 µs | 5.8k |
| 5 `perform_now` | 1,412 µs | 11k |
| 5 mailer messages | 2,931 µs | 5.4k |

The numbers line up too neatly to be a coincidence. The validator route makes 20
closure calls (four per create, five creates), and 20 × 270 µs is 5.4 ms. The
`String#shout` route makes ten calls, at 270 µs each. The `perform_now` route makes
five. The mailer route makes ten calls for five messages, which we come back to
below. Whatever a call did, it cost about 270 µs, and the body that ran was always
small.

## Where 270 µs went

All of these closures are tree-walker functions. `soli serve` loads models, jobs and
mailers on the interpreter at boot, so a `fn` written in a model body is an
interpreter closure, holding a pointer to the environment it was defined in. When
the VM, or a native builtin like `create`, needs to call one, it asks the
interpreter to.

Each of those call sites did the same thing:

```rust
// src/interpreter/builtins/model/validation.rs — before
fn run_validator_body(func: &Function, env: Environment, what: &str) -> Result<Value, String> {
    let mut interp = Interpreter::default();
    // … bind the arguments into `env`, then:
    interp.execute_block(&func.body, env)
}
```

`Interpreter::default()` is the constructor that a script run starts with. It
creates a global environment and registers every builtin into it: `print`, `len`,
the `HTTP` and `Crypto` modules, `Model`, the test DSL, and the rest. That is the
whole registry, built from nothing, at about 270 µs.

Then `execute_block(body, env)` swaps in the environment it is handed and runs the
body there. That environment is the closure's own, and its parent chain already
reaches a global scope with every builtin in it. The registry built a moment earlier
was never looked at, and it was dropped when the call returned.

The fix is a constructor that skips the registry:

```rust
/// An interpreter for running one bound body with `execute_block(body, env)`:
/// a method bound to a receiver, a block, a validator. `execute_block` runs
/// the body in the environment it is handed, whose chain is the body's
/// closure and already sees every builtin, so this one registers none.
pub(crate) fn for_bound_body() -> Self {
    Self::with_environment(Rc::new(RefCell::new(Environment::new())))
}
```

`run_validator_body` now calls `Interpreter::for_bound_body()`. So do the other
call sites that bind a closure to a receiver and run it: a method added with
`define_method` and called on a string, array or hash; an instance's and a class's
`method_missing`; a static method taken as a value; a `class_eval` block; the
`perform` behind `perform_now`; and a mailer action. A block such as
`xs.map { |x| … }` never paid this: in an action it is compiled with the action and
runs on the VM, and on the interpreter it runs in the interpreter already running
the code.

Nothing changes in what a body can see. It still resolves names through its
closure, so `this`, `@field`, your functions, constants and every builtin are
there, as before.

## Scopes paid twice

The registry fix took the scope route from 8.7 ms to 629 µs. That was a ×14
gain, but still 35 µs per scope call, against about 2 µs for every other construct.

A scope goes through one more step. The VM looks `Article.published` up through
the interpreter's member access, so it built an interpreter to do the lookup. That
interpreter was made with `for_vm_fragment`, which copies the VM's entire global
table into it, every builtin and every name the app defines. The comment said why: *a scope closure
is user code, and a tenant scope calls `Current` or an app helper*.

That is true of the scope's body, but the body doesn't run in the interpreter that
looks it up. It runs bound to the query builder, in its own closure's environment,
exactly like a validator. The lookup itself reads only the class. With the copy
gone, the route went from 629 µs to 58 µs. Six chains of three scopes now cost
about 42 µs on top of the floor, or 2.3 µs per scope.

The tenant scope is the case the copy existed for, so it got its own test. The
end-to-end app's model now has two scopes like the ones above, and the server runs
in strict mode, which fails the request if the VM hands it back to the interpreter.
The query comes back with `acme-eu` and `rust` in it, on the VM.

Writing that test caught us out in a different way. We first put
`current_tenant()` in `app/helpers/`, and the scope raised *Undefined variable*, on
2.18.3 as much as now. Helpers are **view** helpers: templates see them; models and
controllers don't. Functions that a model needs belong in `app/models/` (or in a
file the model imports), as in the example above.

## Mailers paid twice too

The mailer route cost 590 µs per message, twice the others. `PingMailer.ping(...)`
isn't a method that exists on the class. It reaches the `Mailer` base class's
`method_missing`, which then invokes the `ping` action. Both steps called a bound
body, and each one built its own interpreter. Both now use `for_bound_body`, and a
message costs about 8 µs to build.

## A leak found on the way: `render_json` and `as_json`

While going through what production did differently from `--dev`, we compared the
two engines on a class that defines `as_json`:

```soli
class PriceTag
  name: String
  price: Float
  cost_price: Float

  new(name: String, price: Float, cost_price: Float)
    @name = name
    @price = price
    @cost_price = cost_price
  end

  def as_json
    {"name": @name, "price": @price}
  end
end

# GET /tag
def tag(req)
  render_json(new PriceTag("Lamp", 49.0, 21.5))
end
```

On 2.18.3:

```
soli serve . --dev   →  {"name":"Lamp","price":49.0}
soli serve .         →  {"name":"Lamp","price":49.0,"cost_price":21.5}
```

`as_json` exists to decide what leaves the server. The tree-walker calls it before
serializing an instance, and so the development server, tests and scripts all
behaved correctly. The VM passed the instance straight to the native
`render_json`, which serializes the fields. It does drop names that look secret
(`password*`, `*_token`, `*_digest` and the like), but nothing else, so a cost price,
an internal note or an email address went out in production and nowhere else.
Because every test ran on the interpreter, no test could catch it.

The VM now checks whether the class defines `as_json` (either as a compiled method
or as an interpreter method) and serializes what it returns. It costs nothing
measurable: 16.1 µs per request before, 16.3 µs now. An end-to-end test in the
server suite checks that a fixture's `as_json` drops its `token` field, on the VM.

This applies to model records as well. A `Note < Model` whose `as_json` returns
only the title sent `{"title":"t","internal_memo":"x"}` from a production server on
2.18.3, and `{"title":"t"}` under `--dev`. If an app on 2.18.3 or earlier renders
instances or records that rely on `as_json` to hide fields, those fields have been
in its production responses. Hashes are unaffected: they have no `as_json`, and
`render_json` sends what is in them.

## Results

Same app, same machine, the build before these changes against the build after,
CPU per request:

| Route | Before | After | Gain | Requests per second |
|---|---:|---:|---:|---:|
| Validators (5 creates) | 5,488 µs | 54.8 µs | ×100 | 2.9k → 257k |
| Scopes (6 chains of 3) | 8,671 µs | 57.7 µs | ×150 | 1.8k → 245k |
| Tenant scopes (2) | 1,224 µs | 24.8 µs | ×49 | 13k → 526k |
| `String#shout` (10 calls) | 2,737 µs | 22.5 µs | ×122 | 5.8k → 572k |
| `perform_now` (5) | 1,412 µs | 19.5 µs | ×72 | 11k → 650k |
| Mailer (5 messages) | 2,931 µs | 55.4 µs | ×53 | 5.4k → 253k |
| `render_json(instance)` | 16.1 µs | 16.3 µs | = | now sends `as_json` |
| `render_json(hash)` | 15.8 µs | 15.8 µs | = | 762k → 760k |

A real action doesn't run five creates and nothing else, so an application won't
see ×100 on a page. What it will see is that each of these constructs now costs
microseconds where it cost a quarter of a millisecond. A form that fails validation
on a model with six custom rules used to spend 1.6 ms on the rules alone. A page
that applies a tenant scope to each of its five queries spent 3 ms building them.

## How we measured

The bench ran on a Ryzen 9 9950X. The server had 16 hardware threads (CPU 0–7 and
16–23), the load generator (`oha`, 200 connections, 15 s per route after 5 s of
warm-up) had eight others, and SoliDB had the rest. Each start launches the app
fresh on one binary, and the two binaries alternate start by start, three starts
each. The figure we report is the median. CPU per request is the server's own
CPU time over the run, read from `/proc`, divided by the requests served. An
Express app on the same cores, measured at the start and end of the run, stayed
within 4% of its reference. The three starts of every route agreed to within 0.5%.

The test suites ran on both builds too: Soli's own Rust and spec suites, plus the
suites of six applications, about 8,700 tests. The new build failed the same tests
as the old one, apart from one application spec that failed once on the new build
and passed on every rerun, alone and in the full suite, on both builds.

## What is still on the list

- **Views.** A 50-row page costs 57 µs to render, the same on both builds. Templates
  run on the tree-walker, and this is the next large item.
- **Constructs that send a whole action back to the interpreter**: dynamic
  finders such as `find_by_email`, `find_each`, `respond_to`, `sse`. When the VM
  can't compile one of these, it hands the entire action to the interpreter, and
  the action stays there.
- **A `perform` compiled by the VM** (under `soli --vm`, not `soli serve`) still
  can't be called by `perform_now`.

## Do you need to change anything?

No. Upgrade, and the same code runs faster. If you avoided custom validators or
scopes on hot paths because they were slow, you can put them back. If you render
instances or records that rely on `as_json`, upgrade before anything else.

The changes are in the [changelog](/docs/getting-started/changelog#v2-18-4).
