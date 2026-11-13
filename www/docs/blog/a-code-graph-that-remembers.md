# A code graph that remembers

`soli graph build` turns an app's source into a graph in SoliDB: files,
classes, methods, routes and views, with the calls, renders and redirects
between them. A coding agent asks `soli graph query "…"` and gets back the
relevant code with its neighbours, instead of grepping. Every node's text is
embedded, so the search works on meaning, not only on names.

The graph had three gaps. It did not know the database: which method reads
`orders`, and what writes `refunds`. It missed links that are plain to anyone
reading the code, such as the view an action renders without naming it, or the
`@orders` it hands that view. And it forgot everything an agent worked out:
the next session started from zero, even when the last one had spent ten
minutes finding where refunds really happen.

The graph on `main` fixes all three. Everything below ran on a small shop app
against a throwaway SoliDB.

<figure style="margin:1.5rem auto;max-width:1024px;">
  <img src="/images/blog/a-code-graph-that-remembers.svg" width="1024" height="576" alt="The shop app's code graph. OrdersController#index reads orders (where, count), renders orders/index implicitly and assigns @orders and @total to it. OrdersController#refund reads orders (find) and calls RefundService#perform, which writes refunds (INSERT) and calls Order#mark_refunded, which writes orders (save). OrdersController#report reads orders and refunds through raw SDBQL. A note anchored on RefundService#perform, Order#mark_refunded and refunds says that a refund is RefundService.perform, and is marked stale after Order#mark_refunded changed." style="display:block;width:100%;height:auto;border-radius:12px;border:1px solid #30363d;background:#0b0d0f;">
  <figcaption style="text-align:center;color:#8b949e;font-size:0.875rem;margin-top:0.5rem;">The shop app's graph: collections, the edges into them, and a note an agent left.</figcaption>
</figure>

## The app

Two models, a service and a controller:

```soli
class Order < Model
  belongs_to("customer")

  # Marks the order refunded; the money moves in RefundService.
  def mark_refunded
    @status = "refunded"
    @save
  end
end

class RefundService
  # Pays the refund back, then records it.
  static def perform(order: Order)
    @sdbql{ INSERT { order: #{order._key}, at: DATE_NOW() } INTO refunds }
    order.mark_refunded
  end
end

class OrdersController < Controller
  # GET /orders
  def index(req)
    @orders = Order.where({"status": "paid"}).all
    @total = Order.count
  end

  # POST /orders/:id/refund
  def refund(req)
    order = Order.find(params["id"])
    RefundService.perform(order)
    redirect("/orders")
  end

  # GET /orders/report
  def report(req)
    @rows = @sdbql{ FOR o IN orders FOR r IN refunds FILTER r.order == o._key RETURN o }
    render("orders/index")
  end
end
```

And the view, `app/views/orders/index.html.slv`, reads `@total` and `@orders`.

## The database, in the graph

Each model now has a `collection` node, named as the ORM names it (`Order` →
`orders`). An STI subclass points at its root's collection. The code that
touches a collection gets a `reads` or `writes` edge to it, and the edge's
`relation` records the operation. These are the edges `soli graph build`
extracts from the app above:

```
Order                   - stored_in        -> orders
OrdersController#index  - reads where      -> orders
OrdersController#index  - reads count      -> orders
OrdersController#refund - reads find       -> orders
OrdersController#report - reads FOR        -> orders
OrdersController#report - reads FOR        -> refunds
Order#mark_refunded     - writes save      -> orders
RefundService#perform   - writes INSERT    -> refunds
```

Three sources feed them:

- **The ORM.** `Order.where`, `Order.find` and `Order.count` read; `create`,
  `update_all` and `delete` write. A write on a record (`order.update(…)`,
  `@save` inside the model) writes the model's collection.
- **Raw SDBQL**, in `@sdbql{ … }` blocks and in strings passed to
  `db.query(…)`. `FOR x IN orders` and a traversal's edge collection are reads;
  `INSERT … INTO`, `UPDATE`, `REPLACE`, `REMOVE` and `UPSERT … IN` are writes.
  Names bound by `LET` or `FOR`, and bind parameters such as `@@coll`, are not
  collections.
- **`Order.count` without parentheses.** Soli calls a zero-argument method with
  no `()`, and the graph now counts that as a call.

So "what writes `refunds`?" is one traversal from `collection:refunds`, and the
query shows it directly (trimmed):

```
$ soli graph query "who writes refunds"
  2. collection  refunds
       ← reads FOR          method:OrdersController#report
       ← writes INSERT      method:RefundService#perform
```

## What the code says, now in the graph

Four links a reader sees at a glance, and the graph used to miss:

```
OrdersController#index - renders implicit -> orders/index
OrdersController#index - assigns @orders  -> orders/index
OrdersController#index - assigns @total   -> orders/index
RefundService#perform  - calls            -> Order#mark_refunded
```

- `index` names no view, so it renders `orders/index`. The graph now records
  that render, marked `implicit`. `report` names its view, so it gets a plain
  `renders` edge and no implicit one.
- For each `@x` an action sets and its view reads, there is an `assigns` edge.
  The view's reads come from the same scanner the language server uses to warn
  about an `@x` nothing sets. "Where does `@total` come from?" is now an edge.
- `perform(order: Order)` types its parameter, so `order.mark_refunded`
  resolves to `Order#mark_refunded`. This already worked for `let order:
  Order = …` and for `order = Order.find(…)`, and now it works for an annotated
  parameter too. The graph stays precision-first: an untyped receiver is left
  unlinked rather than guessed.
- A bare `method(…)` inside a class reaches the class's own method, or an
  inherited one, before any top-level function of that name, as at run time.
  `@method` looks through the ancestors too.

## Notes that outlive the session

The graph describes what the code *is*. What an agent works out about it is
another kind of knowledge. Which of two refund paths is the real one, why a
guard exists, which three files a vague question actually concerns: none of
that is in the source. Now it can go into the graph:

```
$ soli graph note "A refund is RefundService.perform: it inserts into refunds, then Order#mark_refunded saves the order. Nothing else writes refunds." \
    --on RefundService#perform,Order#mark_refunded,refunds \
    --question "how is an order refunded?"
  ✓ noted if42fffd1767f3b66 on RefundService#perform, Order#mark_refunded, refunds
```

`--on` takes the names a query prints, and a name the graph does not hold is
refused. The note goes into its own collection, `soli_graph_insights`, and is
embedded when an embedding key is set. A later query that matches it, or whose
results touch one of its anchors, returns it. If the search ranked an anchor
low, the note brings it in as a result (output trimmed to the relevant lines):

```
$ soli graph query "who writes refunds"
  Query "who writes refunds"  (keyword, 4 results)

  1. class  RefundService  app/services/refund_service.sl:1
  2. collection  refunds
       ← reads FOR          method:OrdersController#report
       ← writes INSERT      method:RefundService#perform

  3. method  Order#mark_refunded  app/models/order.sl:5
     (from a note)
       ← calls              method:RefundService#perform
       → writes save        collection:orders

  4. method  RefundService#perform  app/services/refund_service.sl:3
     (from a note)
       → calls              method:Order#mark_refunded
       ← calls              method:OrdersController#refund
       → writes INSERT      collection:refunds

  Notes
    if42fffd1767f3b66  A refund is RefundService.perform: it inserts into refunds, then Order#mark_refunded saves the order. Nothing else writes refunds.
      asked: how is an order refunded?
      on: RefundService#perform, Order#mark_refunded, refunds
```

`soli agent`, the MCP server that attaches a coding agent to `soli serve
--dev`, has the same thing as a `graph_note` tool, next to `graph`. An agent
can leave the note as soon as it has the answer, and the next one finds it.

### A note can go stale

A note is only as good as the code it describes. It stores a fingerprint of
each anchor's text: the signature, the doc comment and the source, but not the
line number, so code that only moved does not count. When an anchor changes,
the note says so. Add a line to `Order#mark_refunded` and rebuild:

```
$ soli graph notes
  1 note(s), 1 stale
    if42fffd1767f3b66  A refund is RefundService.perform: …  (stale: the code changed since)
      on: RefundService#perform, Order#mark_refunded, refunds
```

The note is still returned, marked: the agent reads it as a lead to check, not
a fact. `soli graph forget <key>` deletes it. A rebuild never touches notes,
even `soli graph build --fresh`. Node keys come from the code's names, so an
anchor survives every rebuild.

### Questions asked before

Every `soli graph query` is also logged in `soli_graph_queries`, with the code
it returned; the last 1,000 are kept. A later question that shares two of its
words lists those earlier questions and what each found:

```
$ soli graph query "order refund steps"
  …
  Asked before
    "refund an order"  → orders#refund, Order#mark_refunded
```

It is a cheap trail: a question that already led somewhere gives the next one
a starting point.

## Using it

```bash
soli graph build                 # incremental; --no-embed without an embedding key
soli graph query "…" --json      # results, insights, related_queries
soli graph note "…" --on A,B --question "…"
soli graph notes                 # stale ones marked
soli graph forget <key>
```

The graph needs SoliDB. Without an embedding key, queries and notes fall back
to keyword matching. That matching compares parts of names, so "publish" finds
`Post#publish` but "publishing" does not. The full reference, including the
schema of the two new collections and the JSON fields (`insights`,
`related_queries`, `via`, each edge's `relation`), is in
[Code graph](/docs/development-tools/graph). These features are on `main` and
will ship in the next release.
