# 237 Errors, All False: Teaching `soli check` the Runtime It Checks

On a 247-file application, `soli check` reported **237 errors**. Every one of them
was false. The code ran. The server served it, the test suite passed, and the type
checker said none of it was valid.

What a team does next is predictable, and this one did it: they took `soli check`
out of their verification gate and wrote a line in their README saying so. That was
the honest thing to write. A checker that fails on working code is not a strict
checker, it's a broken one. Once people learn to scroll past its output, they will
also scroll past the one real error hiding in it.

This post covers how the checker got into that state and why the fix was to remove
a list rather than patch it. It also covers two smaller lessons from the same week
about when people can trust a tool's advice.

<figure style="margin:1.5rem auto;max-width:1024px;">
  <img src="/images/blog/soli-check-false-positives.svg" width="1024" height="576" alt="Two panels. Left, before: a hand-kept list of known globals sits beside the runtime and drifts from it; names like pdf_merge, Image, router_eui, middleware and describe are missing, so soli check reports 237 errors on a 247-file application, all false. Right, after: register_builtins fills a throwaway environment, six Soli preludes are parsed for their declarations, and the checker reads the result; the same application reports 0 false errors. Footer: parity baseline type-checker section 312 to 62 names; soli new demo --eui 176 to 13 errors; the pass over 247 files takes 0.4 seconds." style="display:block;width:100%;height:auto;border-radius:12px;border:1px solid #30363d;background:#0b0d0f;">
  <figcaption style="text-align:center;color:#8b949e;font-size:0.875rem;margin-top:0.5rem;">A second list drifts from the runtime by construction. Ask the runtime instead.</figcaption>
</figure>

## Where the false errors came from

The 237 errors fell into five families. The first was by far the largest:
**140 of the 237 named something the interpreter installs.** These were
`describe` and `res_body` from the test DSL, `middleware` from the routing DSL,
`RateLimiter`, `Mailer`, and `form_with`. Every one of them is a real global at
run time, and the checker reported each as `Undefined variable`.

The reason is structural. Soli's builtins are installed into one `Environment` at
startup by `register_builtins`. Two other tools also need to know about each one:

- the **type checker** (`src/types/environment.rs`), or `soli check` fails the
  program with `Undefined variable '<name>'`;
- the **linter** (`WELL_KNOWN_GLOBALS` in `src/lint/rules/scope.rs`), or
  `smell/undefined-local` flags every call site. `let` is optional in Soli, so the
  rule can't tell a builtin from a typo.

So the runtime had one list and the tooling kept two more by hand. Adding a
builtin meant remembering three places. Nothing failed when you forgot two of
them. The builtin worked, your program ran, and the only thing that refused it
was a tool you might not run for days.

## The same bug, three times in one cycle

The commit that finally pinned this down (`ec900770`) is titled *"a builtin the
runtime has and the tooling refuses, for the third time"*. That's an accurate
count:

1. **Ten `pdf_*` builtins.** `pdf_render`, `pdf_response` and the two Factur-X
   entry points were in the linter's list. `pdf_from_markdown`,
   `pdf_layout_map`, `pdf_fill`, `pdf_merge`, `pdf_pages`, `pdf_stamp`,
   `pdf_sign`, `pdf_verify`, `pdf_extract_facturx` and `pdf_attachments` were
   not. `smell/undefined-local` flagged every call to them. On the checker side,
   the PDF block listed thirteen builtins and left out `pdf_response`.
2. **`Image`, `File` and `Trusted`.** `register_builtins` installs all three
   unconditionally. None of them was in the type checker's engine-embedded class
   list, so any script that touched them failed with `Undefined variable 'Image'`
   unless it ran with `--no-type-check`. That list existed to solve exactly this
   problem, and it had missed three classes.
3. **The EUI builtins.** `router_eui`, `eui_render`, `eui?`, `eui_wake`,
   `eui_notify`, `eui_asset`, `eui_font`, `eui_icon`, `eui_name`, `eui_stats`
   and `eui_capabilities` were registered at run time and declared to neither
   tool. The failing application was one that `soli new --eui` had generated. So
   the framework's own generator produced code that its own checker rejected.
   `sse` and `stream` had the same problem. `next` was known to the linter but
   not the checker, and `permit` the other way round. The linter was also
   missing about fifty ordinary builtins that controllers call all the time:
   `cache`, `forbidden`, `json_stringify`, `password_hash`, `md5`, `url_encode`,
   `slurp`, `broadcast`, `csrf_token`, and the `x25519` family.

Earlier in the changelog there are more of these. At one point 25
general-purpose builtins, including `sha256`, `password_verify`, `html_escape`
and `sleep`, were rejected by the checker. Before that it was `Base64`, `Hex`,
`RsaKey`, `X509` and the JWT functions. The commit message explains why this
kept happening: *"each time a user found it, because a builtin that works and only
the tooling refuses makes no noise anywhere a test would hear it."*

## Step one: make the drift fail the build

`ec900770` didn't fix the lists. It made drift in them a test failure.
`tests/builtin_registration_parity_test.rs` asks the runtime what it holds. It
registers the builtins into a fresh environment and reads back the bindings.
Then it checks every global against both lists. It uses the runtime environment
instead of grepping the registration sites because grepping misses anything
registered in a loop.

A plain "every builtin must be known" assertion would have been false, so the
test works as a **ratchet**, like the repo's `scripts/unwrap_baseline.txt`.
Many names are legitimately unknown to the checker. `describe`, `it` and
`assert_*` are only valid inside a spec. `has_many` and `before_save` are only
valid inside a model body. If the checker accepted those everywhere, `soli check`
would pass a spec-style call in a script that can't run it. So the known gaps are
recorded in `tests/builtin_registration_baseline.txt`, and the test fails in two
cases:

- a registered name that's missing from both the tool and the baseline, so a new
  builtin can't reach `main` unless someone decides in a commit which kind it is;
- a baseline name that has since become known, so the file can only shrink.

That stops the bleeding, but the lists are still hand-written. The next step was
to delete one of them.

## Step two: stop keeping the list

`0b309313` replaced the checker's hand-kept list with
`src/types/runtime_globals.rs`. The module's doc comment states the problem
plainly. *"A second hand-written list would drift the same way,"* it says, and
it notes that the linter's `WELL_KNOWN_GLOBALS` was already missing `middleware`
and `RateLimiter`. So the checker now derives the list:

1. **Register the builtins into a throwaway environment and read it back.**
   `register_builtins(&mut env, false)` runs, then `env.get_all_bindings()`
   returns what's there. If a binding is a class, the checker records its native
   static and instance methods too. That's why `I18n.cache_table` now resolves
   (it has been real since the day it was added to the interpreter, but the
   checker's hand-written model of `I18n` didn't include it), while
   `I18n.tarnslate` is still an error.
2. **Parse the preludes the runtime runs as Soli source.** Some of the global
   namespace isn't written in Rust. The routing verbs, `Mailer` and `Message`,
   the form builder (`form_with`, `csrf_field`, `button_to`), the upload helpers
   and `Retry` are all Soli code the server evaluates at boot. The checker lexes
   and parses those six sources and collects their top-level functions and
   classes. It doesn't copy their names into a list.
3. **Keep the names that already have precise types.** `len`, `str`, `HTTP` and
   the rest of the hand-modelled surface keep their signatures. The derived set
   only fills gaps and never overwrites a real type.

All of this runs once per process behind a `OnceLock`. According to the
changelog, the pass over the 247-file application takes 0.4 s.

### Two namespaces the runtime keeps separate

Seeding everything into every file would have created the opposite bug: the
checker would accept code that fails at run time. The runtime actually has more
than one namespace, and the checker follows it.

- **The test DSL.** `register_builtins` takes a flag. A served application does
  *not* get `describe`, `test`, `expect` or `as_guest`. So a `describe(...)` in a
  controller is a real error. The checker computes the test DSL as the
  *difference* between the two registration modes, so no one maintains that list
  either. It seeds those names only for files that `soli test` would run.
- **The request scope.** `req`, `params`, `session`, `cookies` and
  `current_user` exist only while a request is being handled. The server injects
  them into the handler's scope right before the call. This is the one list in
  the module that is written by hand, because no table exists to derive it from.
  `render` and `redirect` belong here too. They are registered builtins, but a
  standalone script that calls them can't work, and that decision is pinned by a
  test. These names are seeded for files under `app/`, `config/` or `stdlib/`,
  and nowhere else. A loose script still gets `Undefined variable 'render'`.

The parity baseline shows the effect. Its `[type-checker]` section dropped from
**312 names to 62** in that commit. A second test now requires every remaining
name to fall under one of the two stated reasons, test DSL or request scope. That
keeps the section from turning back into a place where someone silences a missed
builtin by adding a line.

## Reading a project the way the server loads it

Builtins were one source of false errors. The other was the checker's view of the
project itself.

At run time, a server loads `app/controllers`, `app/models`, `app/services`,
`app/policies`, `app/middleware`, `app/mailers`, `config` and `stdlib` into **one
environment**. A helper declared in one file can be called from any of the
others without an import. `soli check` read one file at a time, so every one of
those calls was `Undefined variable`. A freshly generated `soli new demo --eui`,
with nothing edited, reported **176 errors**. 137 of them named five functions
declared in a sibling file of the same widget catalogue.

`9f92669c` changed this. When you give `soli check` a directory, it first
collects the top-level declarations from those auto-loaded directories and then
checks each file with those names known. Four details mattered, and the commit
says each was *"found by running it rather than by reading"*:

- **Neighbour names are declared as `Any`.** The checker knows the name will
  exist. Knowing its type would mean checking the declaring file first, which is
  a whole-project pass. `Any` removes the false error without making up a type.
- **A project declaration overrides a builtin with the same name**, because
  that's what happens at run time. The catalogue defines its own three-argument
  `input`, and the one-argument builtin's signature had been rejecting all nine
  call sites.
- **A bare top-level assignment declares a name.** `let` is optional, so
  `DROPDOWN_MAX_PX = 280` declares that name just as `const` would. The name
  collector only handled `let` and `const`. The linter uses the same collector,
  so it had the same gap.
- **A ternary accepts the same conditions an `if` does.** The ternary had been
  stricter: it required a `Bool`, so `flag ? "a" : "b"` failed in places where
  `if flag { … }` passed. That accounted for 31 of the 52 errors left after the
  namespace fix.

**176 went down to 13.** The commit accounts for the remaining thirteen. Five
are DSL that is deliberately not type-checked, or not yet. Eight are
`Void`/`Null` inference problems. `0b309313` then fixed four more cases where
the checker disagreed with the language. A method without a return annotation
was typed `Void`, so any use of its result was an error. A class whose parent
was in another file looked like a root class. `let x = nil` locked `x` to
`Null`. Writing a new key into a hash was checked against the value type of the
literal that built it.

## Before and after, on a scratch project

Here is a four-file project that uses each of these patterns: a routing DSL
verb, a helper in a service file, a forward-declared `nil`, a hash that picks up
a string key, and a spec.

```soli
# config/routes.sl
middleware("authenticate", fn() {
    get("/invoices/:id", "invoices#show")
})
```

```soli
# app/services/pricing.sl
LOYALTY_RATE = 2

def discount_for(customer) {
    return 0 if customer.nil?
    customer["loyalty"] * LOYALTY_RATE
}
```

```soli
# app/controllers/invoices_controller.sl
def show(req) {
    total = 200 - discount_for(req["customer"])

    let last_payment = nil
    last_payment = {"due_on": "2026-10-01"} if total > 0

    headers = {"timeout": 10}
    headers["Authorization"] = "Bearer " + getenv("BILLING_TOKEN").to_s

    render("invoices/show", {"total": total, "last_payment": last_payment})
}
```

```soli
# tests/pricing_spec.sl
describe("discount_for", fn() {
    test("no customer, no discount", fn() {
        assert_eq(discount_for(nil), 0)
    })
})
```

Here is the output from a v1.29.0 build:

```
$ soli check .
./tests/pricing_spec.sl: Type error: Undefined variable 'describe' at 1:1
./config/routes.sl: Type error: Undefined variable 'middleware' at 1:1
./app/controllers/invoices_controller.sl: Type error: Undefined variable 'discount_for' at 2:19

3 error(s) in 3 of 4 file(s)
```

Three errors doesn't sound like much, but the checker stops at the first error
in each file, so each one hid everything below it. The runtime-globals module
says the same about specs: an undefined `describe` meant *"its body — every
`test`, every assertion — went unchecked behind it."* When I removed the first
error from the controller one line at a time, the same build reported
`expected Null, found Hash<String, String>` on the `last_payment` reassignment,
`expected Int, found String` on the `Authorization` write, and
`Undefined variable 'render'`. Every one of those lines runs fine.

Here is the current build on the same project:

```
$ soli check .
No type errors. Checked 4 file(s).
```

The checker still catches real errors. Add a service with a typo in a
builtin-class method:

```soli
# app/services/labels.sl
def invoice_label(total) {
    I18n.tarnslate("invoice.total") + ": " + str(total)
}
```

```
$ soli check .
./app/services/labels.sl: Type error: Cannot access member 'tarnslate' on type 'I18n' at 2:5

1 error(s) in 1 of 5 file(s)
```

After fixing that typo, a misspelled local (`str(totl)`) gives
`Undefined variable 'totl'`. The changelog lists the other checks that still
fail as they should: an `Int` annotation given a `String`, three arguments to a
two-parameter function, and an unknown member on a class whose whole ancestry is
in the file.

One caveat: project mode applies only to a directory that *is* the project.
`soli check app/controllers` looks for the auto-loaded directories under
`app/controllers`, doesn't find them, and reports `discount_for` as undefined
again. `soli check app/controllers/invoices_controller.sl` does the same on
purpose, because a single file has no project around it. To get the view the
server has, run it from the root.

## The same lesson, applied to advice

The same week produced two smaller fixes that follow the same principle: a tool
has to say what the language actually does.

**`||` returns an operand, not a boolean** (`79dd2bc2`). `a || b` evaluates to
`a` if `a` is truthy and to `b` otherwise, so `getenv("HOST") || "localhost"`
is a `String`. The checker typed every `||` as `Bool`. It also had no rule for
repeating a string with `*`. Here are both on the old build:

```soli
def banner(host: String) -> String {
    "-" * 40 + "\n" + host
}
host = getenv("HOST") || "localhost"
print(banner(host))
```

```
ops.sl: Type error: cannot perform arithmetic on String and Int at 2:8
ops.sl: Type error: Type mismatch: expected String, found Bool at 4:1
```

The current build passes it and prints forty dashes followed by `localhost`. The
`||` default is the idiom the project's own `CLAUDE.md` teaches, so the checker
had been rejecting a pattern the project recommends.

**An idiom rule has to be a pure rename, or say that it isn't** (`cef28a0c`).
`idiom/nil-comparison` used to suggest `.present?` as the replacement for
`!= null`. Those aren't two spellings of the same test:

```soli
nickname = ""
print(nickname != null)    # true
print(nickname.present?)   # false
```

In Soli, `.present?` returns false for the empty string, while `!= null` is true
for it. The rule presented a change in behaviour as a style cleanup. On one real
repository it would have touched 613 sites, and whether each rewrite was safe
depended on whether that value could ever be `""`. The rule now suggests
`!x.nil?`, which is exactly equivalent, so all 613 sites can be rewritten
mechanically without changing anything.

`idiom/prefer-blank` can't be made equivalent: `.blank?` treats nil as empty,
`.present?` treats nil as absent, and `== ""` does neither. So it now says so
instead of presenting the difference as a bonus:

```
[idiom/prefer-blank] consider `.blank?` over comparing to an empty string — but it
CHANGES the nil case: `.blank?` counts nil as empty and `.present?` counts nil as
absent, while `== ""` does neither. Rewrite only where nil and "" should mean the
same thing
```

Two new tests now check the *text* of these messages. Before that, all 82 lint
tests passed without any of them looking at what the linter actually suggests,
which is the part people apply without reading closely. `.present?` is often
what the author meant, but deciding that requires reading the surrounding code,
and the linter shouldn't make that call for them.

## What's still hand-written

The linter's list is still hand-kept. The `[linter]` section of the parity
baseline lists 219 names today. That list now fails the build when it drifts
instead of failing a user, but it's still a list, and the reasoning in this post
applies to it too. In `runtime_globals.rs`, the request scope is the one list that isn't
derived, because the runtime has no table of those names to read.

Everywhere else, the rule is to read the runtime's namespace from the runtime.
Every duplicate list is a claim about the runtime that nothing verifies, and it
drifts. Users notice it, one false error at a time, before any test does. After
enough false errors they stop running the tool, and then it catches nothing.
