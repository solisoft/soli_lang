# A worker that waits for nobody

A Soli server in production starts two workers by default. Each one is a full
interpreter with your models, controllers and views loaded, and until now each
one served a request from its first line to its last. That is fine while a
request computes. It is not fine while a request waits: an action that calls a
payment API answering in half a second holds its worker for half a second, and
two such calls at once stall the whole application.

Soli now has an experimental answer, behind one environment variable:

```bash
SOLI_COROUTINES=1 soli serve . --port 5011
```

Each request runs on a coroutine of its own. When it waits on an outbound HTTP
call or a SoliDB query, the worker puts it aside and serves other requests;
when the answer arrives, the request picks up where it stopped. Your code does
not change. There is no `await`, no callback, no async variant of `HTTP.get`:

```soli
class CheckoutController < Controller
  # POST /checkout
  def create(req)
    order = Order.create(permit(params, {"cart": true}))
    charge = HTTP.post_json("https://payments.example.com/charges", {"amount": order.total})
    order.update({"charge_id": charge["id"]})
    redirect("/orders/#{order._key}")
  end
end
```

The `HTTP.post_json` line still returns the charge. While it is in flight, the
worker that runs this action is serving someone else.

This post explains how that works, what it took to make it safe, where it does
not apply, and what it buys, measured on two machines.

<figure style="margin:1.5rem auto;max-width:1024px;">
  <img src="/images/blog/a-worker-that-waits-for-nobody.svg" width="1024" height="576" alt="Two timelines of one worker. Before, requests A, B and C run one after another and each spends most of its time waiting on an API, so the worker sits idle for 1.5 seconds. With SOLI_COROUTINES=1, the worker starts A, B and C in turn while their API calls are in flight, and each resumes when its answer arrives, in 0.5 seconds. Below, measured on 2 workers: an API at 500 ms goes from 4 to 74 requests per second, SoliDB create, count and read from 1,180 to 3,120, an API call between two queries from 8 to 249." style="display:block;width:100%;height:auto;border-radius:12px;border:1px solid #30363d;background:#0b0d0f;">
  <figcaption style="text-align:center;color:#8b949e;font-size:0.875rem;margin-top:0.5rem;">The wait still happens. The worker just stops spending it.</figcaption>
</figure>

## Why not async/await

Node, Python and Rust put the waiting in the language: a function that may
wait is marked `async`, and every caller has to `await` it. That makes the
waits visible, which has value. It also splits every library in two and asks
you to think about colouring before you write a controller.

Soli's interpreter is synchronous, and so is every builtin. An `HTTP.get`
inside a `map` block inside a model callback is ordinary code. To suspend that
request we need to suspend everything under it — the Soli frames, the Rust
frames of the interpreter, the builtin in the middle of its call — and resume
all of it later.

That is what a **stackful coroutine** does. Each request gets a stack of its
own (we use the [`corosensei`](https://crates.io/crates/corosensei) crate), and
switching requests means switching stacks: a few registers saved, a few
restored. The interpreter does not know it happened.

## Who drives the waiting

The network work of an outbound call is a Rust future: send the request, wait
for the answer, read the body. Before, the worker drove that future to the end
with `block_on`, sitting idle in the middle.

Now, when a request on a coroutine reaches that point, the coroutine polls the
future itself, on the worker thread. If the answer is not there yet, it saves
its request's state, hands control back to the worker, and sleeps. The future
carries a waker: when the network answers, the waker sends the coroutine's id
to its worker, which resumes it, and the coroutine polls again.

Polling on the worker thread, rather than handing the future to another
thread, turned out to be the decision that made the rest simple. A future that
holds Soli values — `get_json` parses into them, `HTTP.stream` keeps them
between lines — cannot move to another thread, and did not have to be
rewritten. The dev bar's HTTP log, which the call writes as it completes, still
lands on the right request.

SoliDB queries work the same way, with one more step. Each worker used to run
its database I/O on a small runtime of its own, which only moves while someone
blocks on it. A suspended query would never progress. A worker running
coroutines therefore sends all its SoliDB traffic through the server's shared
runtime, which never stops, and the native driver keeps a few connections per
worker instead of one, since each waiting request holds its own.

## A request's state is more than its stack

A stack is not all a request owns. A worker serves one request at a time, so
over the years a lot of per-request state settled into thread-locals: the
session id, the cookies to set, the flash, the open transaction, the
`grouped` batch, the locale, the database tenant, the dev logs, the deadline,
half the template engine's bookkeeping. Each was reset between two requests
and nobody had to think about it.

Interleave two requests on one thread and every one of those becomes a leak:
request B would set request A's cookies.

We counted them: **95 variables in 59 files**. Rather than hunt them one by
one at every switch, we gave them a name. Request state is now declared with a
`request_local!` macro instead of `thread_local!`. The macro keeps the same
API, so nothing that reads them changed, and each variable registers itself
on its thread. At a switch, the scheduler takes every registered value out,
which leaves the initial values, and puts the next request's back in.

The globals a handler reads (`req`, `params`, `cookies`, and the same names in
the view helpers' environment) are saved and restored the same way.

A test now keeps this from regressing. It lists every `thread_local!` in the
source and fails unless it is either a `request_local!` or written, with its
reason, in a list of things shared by the whole thread (caches, registries,
pools). A new piece of request state cannot slip in as a plain thread-local.

One lesson from the migration: the first version also turned the recursion
guards of formatting and equality into request locals, and string
interpolation got 14 to 20 % slower. Those guards are read on every `#{}` and
every `==`, and they are back at zero after each call, so they never need
saving. They stayed plain thread-locals, and the scheduler simply refuses to
suspend while one is above zero.

## When a request does not suspend

Some waits are not safe to step away from, and there the call blocks exactly
as before:

- **inside a transaction**, whose connection and open state belong to it;
- **inside a `grouped` block**, which collects reads to send in one round trip;
- **during a render**, and during a `stream` block, which holds its worker by
  design;
- **while a value is being formatted, compared or serialized**.

Three more limits:

- **The SQL adapters still block.** Postgres, MySQL and SQLite go through
  synchronous Rust crates; making them suspend means changing drivers, which
  is its own project.
- **It is off under `--dev`**, where hot reload and the dev bar assume one
  request at a time.
- **Concurrency is capped:** `SOLI_WORKER_CONCURRENCY` (32 by default) is how
  many requests one worker runs at once.

And one thing changes for application code: two requests on the same worker
can now interleave at those waits. Request state stays each request's own, but
a class-level static field read and written around an `HTTP.get` can see
another request's write in between, as in Node. If you keep a counter in a
static field, it is no longer per request.

## The numbers

We measured on the Ryzen 9 9950X box used for the [benchmarks
page](/docs/getting-started/benchmarks): 2 production workers (the default),
the server and the load generator pinned to cores of their own, a fresh
SoliDB for every configuration, and every response checked to carry its own
`params`, session, cookie and API answer.

| Route | v2.28.11 | Coroutines on | Change |
|---|---:|---:|---:|
| Calls an API that answers in 500 ms | 4 req/s | 74 req/s | ×18 |
| Creates a row, counts, reads it back (SoliDB) | 1,180 req/s | 3,120 req/s | ×2.6 |
| An API call between two queries | 8 req/s | 249 req/s | ×31 |
| Three reads of one document (SoliDB) | 35,800 req/s | 65,500 req/s | ×1.8 |

Across roughly 30,000 checked responses, not one carried another request's
state.

The 74 req/s is the load generator's ceiling, not the server's: 40 concurrent
clients waiting 500 ms each cannot ask for more than 80 a second. With 200
clients, the same route reached 116 req/s, against the 128 that 2 workers × 32
coroutines waiting half a second each allow.

The last row was a surprise. Reads that take a fraction of a millisecond still
leave the worker idle for that fraction, and with coroutines it starts the
next request meanwhile.

With the variable off, the build is as fast as v2.28.11 on every route we
measured.

## What a coroutine costs

Coroutines are not free. On a route that only renders JSON, at 567,000
requests per second, the first version was 5.4 % slower: about 100 ns per
request for a stack from the pool, two switches and the bookkeeping, which
shows when the whole request takes 1.8 µs. A template page lost 2.6 %; a
route that touches the database cannot tell.

So the scheduler now learns which routes wait. A route that has never reached
a wait runs directly on the worker, as before, and pays nothing. The first
time it waits on the network, that one call blocks as it used to, and the
route's later requests run on a coroutine. Routes are grouped by method and
path with the ids masked, so `/orders/123/pay` and `/orders/456/pay` learn
together.

The same two-worker bench, rerun with `dist` builds and the learning
scheduler:

| Route | v2.28.11 | Coroutines on |
|---|---:|---:|
| JSON only | 566,000 req/s | 585,000 req/s |
| Template page | 186,000 req/s | 189,000 req/s |
| Three reads of one document (SoliDB) | 36,700 req/s | 68,700 req/s |
| API at 500 ms | 4 req/s | 68 req/s |
| Create, count, read back (SoliDB) | 1,190 req/s | 3,030 req/s |
| An API call between two queries | 8 req/s | 226 req/s |

The routes that only compute no longer pay anything. The ones that wait keep
their gains, a little below the first run: on each worker, the first request
of each route blocks once before the route moves to coroutines, about half a
second on a five-second run, and nothing on a server that stays up.

## Rerunning the benchmarks page

The [benchmarks page](/docs/getting-started/benchmarks) measures a different
situation: 16 workers on 16 threads, a load generator at 200 connections, and
a SoliDB on the same machine that answers in about 9 µs. We reran its Soli
read rows with its own harness and its own 50 rows, on the same box, with
`dist` builds (fat LTO), over two rounds:

| Row | v2.28.11 | Coroutines off | Coroutines on |
|---|---:|---:|---:|
| `/json` | 511k / 521k | 519k / 545k | 516k / 544k |
| `/template` | 509k / 517k | 515k / 542k | 515k / 541k |
| `/db` (50 rows) | 274k / 273k | 271k / 291k | 276k / 302k |
| `/db-template` | 270k / 268k | 270k / 286k | 266k / 300k |

Every response was a 200, and `/db` sent the same 2,268 bytes as every stack
on that page. (The page's `/db` figure is 222k; it was measured with SoliDB
2.0.5, these with 2.3.1, so they do not replace it.)

Nothing here moves by more than the run-to-run noise, and that is the point.
Sixteen workers on sixteen threads are already busy all the time, and a
database that answers in 9 µs leaves almost nothing to recover. Coroutines do
not make computing faster; they stop a worker from idling. They pay where a
request waits, behind a slow API, a remote database, or a small worker pool,
and cost nothing measurable where it does not.

## Trying it

```bash
SOLI_COROUTINES=1 SOLI_WORKER_CONCURRENCY=32 soli serve . --port 5011
```

It is experimental: off by default, and documented in
[Configuration](/docs/getting-started/configuration). It pays most where an
app spends its time waiting on other services, which, for most web apps, is
most of the time. If an action of yours behaves differently with it on, that
is a bug we want to hear about.
