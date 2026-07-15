# Code Graph (Graph RAG for agents)

`soli graph build` extracts a **graph of your project's source code** — files,
classes, models, controllers, methods, functions, routes and views, plus the
relationships between them — and stores it in SolidB. Every node's text is
embedded, so AI agents (and your own tools) can retrieve the right code by
**semantic search** and then **traverse relationships** from there. It's a graph
RAG index of your own codebase.

Unlike `Model.rag`, which indexes your *application data*, this points the same
vector + graph machinery at the *code itself*.

## Building the graph

```bash
soli graph build            # build the graph for the app in the current folder
soli graph build path/to/app
```

By default every node is embedded (vector index included). Configure the
embedding provider with the standard variables (OpenAI by default):

| Variable | Default |
|----------|---------|
| `SOLI_EMBEDDING_API_KEY` | *(required to embed)* |
| `SOLI_EMBEDDING_URL` | `https://api.openai.com/v1/embeddings` |
| `SOLI_EMBEDDING_MODEL` | `text-embedding-3-small` |

The graph connects to the **same SolidB the app's Models use** (`SOLIDB_HOST`,
`SOLIDB_DATABASE`, `SOLIDB_USERNAME`/`SOLIDB_PASSWORD` or `SOLIDB_JWT`), loaded
from your `.env` just like `soli db:seed`.

### Flags

| Flag | Effect |
|------|--------|
| `--no-embed` | Structural graph only — skip embeddings and the vector index (fully offline, no API key). |
| `--database NAME` | Write to a specific database instead of `SOLIDB_DATABASE`. |
| `--dry-run` | Print the whole graph as JSON to stdout — writes nothing to SolidB and calls no embedding API. Great for inspection and CI. |

```bash
# Inspect what would be built, without a database or API key:
soli graph build --dry-run | jq '.nodes | length, .edges | length'
```

The build is a **clean rebuild**: the `soli_graph_nodes` and `soli_graph_edges`
collections are dropped and recreated each run, so the graph always matches the
current source. Re-run it whenever the code changes.

## Schema

Two SolidB collections, namespaced so they never clash with your app data.

### `soli_graph_nodes`

One document per code entity. `_key` is a sanitized, stable identifier; the
human-readable identity is `kind` + `qualified_name`.

| Field | Meaning |
|-------|---------|
| `kind` | `file`, `class`, `model`, `controller`, `method`, `function`, `route`, `view`, `enum`, `interface`, `external` |
| `name` | short name (`authenticate`, `User`, `GET /login`) |
| `qualified_name` | `User#authenticate`, `posts#index`, `sessions/new` |
| `file`, `line` | source location |
| `signature` | readable signature (`def authenticate(password: String) -> Bool`) |
| `superclass`, `role` | class base; MVC role (`model`, `controller`, …) |
| `doc` | leading `#` / `//` comment |
| `text` | the text that was embedded (kind, name, signature, doc, snippet) |
| `embedding` | vector (omitted with `--no-embed`) |

### `soli_graph_edges`

One document per relationship, as a SolidB edge (`_from` / `_to` reference
`soli_graph_nodes/<key>`).

| `edge_kind` | From → To |
|-------------|-----------|
| `defines` | file → class/function, class → method (containment) |
| `inherits` | class → superclass (an `external` stub for framework bases like `Model`) |
| `implements` | class → interface |
| `imports` | file → local file |
| `calls` | method/function → the function or class-method it calls |
| `instantiates` | method/function → class it constructs (`new X()`) |
| `renders` | controller action → view it renders |
| `routes_to` | route → controller action |
| `relates` | model → model (`has_many` / `belongs_to` / `edge`; the DSL name is in `relation`) |

Call-graph resolution is **precision-first**: class-method calls
(`User.find(...)`), `this.method(...)`, and unambiguous bare function calls are
linked; instance calls on a variable (`user.save()`) are not inferred (no type
inference), so an edge is never invented.

## Querying the graph (for agents)

Agents query SolidB directly. Two moves cover most needs: **find** nodes (by
name, kind, or semantic similarity), then **traverse** from them.

> **Traversal starts** use `soli_graph_nodes/<_key>`, not `n._id` (SolidB's
> `_id` carries a `db:` prefix that won't match edge endpoints). Build the start
> with `CONCAT("soli_graph_nodes/", n._key)`. Traversals support the
> `FOR v, e IN …` (vertex + edge) form.

### Find code

```aql
// All controllers
FOR n IN soli_graph_nodes FILTER n.kind == "controller" RETURN n.name

// A specific route
FOR n IN soli_graph_nodes
  FILTER n.kind == "route" AND n.qualified_name == "sessions#create"
  RETURN n
```

Semantic search (needs embeddings) uses the `node_vec` vector index — from Soli,
model the collection and call `similar` / `graph_rag`:

```soli
class CodeNode < Model
  collection "soli_graph_nodes"
  vector_index "embedding", dimension: 1536, metric: "cosine"
end

# "Where is authentication handled?" → the most relevant code nodes
let hits = CodeNode.similar("authentication and login", field: "embedding", k: 8)
```

### Traverse relationships

```aql
// What handles this route, and what does it reach? (route -> action -> calls/renders)
FOR v, e IN 1..3 OUTBOUND "soli_graph_nodes/route:GET_:login" soli_graph_edges
  RETURN { via: e.edge_kind, kind: v.kind, name: v.qualified_name }

// Which actions render this view? (reverse edges)
FOR v, e IN 1..1 INBOUND "soli_graph_nodes/view:posts:index" soli_graph_edges
  FILTER e.edge_kind == "renders"
  RETURN v.qualified_name

// What does a controller action call?
FOR v, e IN 1..2 OUTBOUND "soli_graph_nodes/method:PostsController.create" soli_graph_edges
  FILTER e.edge_kind IN ["calls", "instantiates"]
  RETURN v.qualified_name

// Every model and what it inherits / relates to
FOR n IN soli_graph_nodes FILTER n.kind == "model"
  FOR v, e IN 1..1 OUTBOUND CONCAT("soli_graph_nodes/", n._key) soli_graph_edges
    FILTER e.edge_kind IN ["inherits", "relates"]
    RETURN { model: n.name, rel: e.edge_kind, to: v.name }
```

Semantic **seed → traverse** (graph RAG) combines both: `similar()` finds the
relevant nodes, then an OUTBOUND/INBOUND traversal expands to their callers,
callees, routes and views — exactly the context an agent needs to make a change.

## Notes

- v1 is a one-shot build — re-run it to refresh; there's no file-watch yet.
- `.slv` views become `view` nodes (their content is embedded too), so
  "find the view that renders X" is a semantic query.
- The graph is a **superset**: it works on any Soli codebase, but MVC roles
  (models, controllers, routes, views) get first-class node kinds.
