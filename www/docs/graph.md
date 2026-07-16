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
| `--no-embed` | Structural graph only — skip embeddings and the vector index (fully offline, no API key). Existing embeddings are preserved. |
| `--database NAME` | Write to a specific database instead of `SOLIDB_DATABASE`. |
| `--dry-run` | Print the whole graph as JSON to stdout — writes nothing to SolidB and calls no embedding API. Great for inspection and CI. |
| `--fresh` | Force a full clean rebuild (drop + recreate) instead of the default incremental sync. Use it after changing the embedding model, or to reset. |

```bash
# Inspect what would be built, without a database or API key:
soli graph build --dry-run | jq '.nodes | length, .edges | length'
```

The build is **incremental and non-destructive**. It hashes every source file
(MD5) into a manifest, so a re-run when nothing changed is a fast no-op
(`✓ code graph already up to date`). When something did change, it updates SolidB
**in place** — inserts new nodes/edges, updates changed ones, prunes removed ones
— rather than dropping the collections, so a concurrent reader never sees an
empty graph and **unchanged embeddings are reused** (only changed/new node text
is re-embedded). Pass `--fresh` to force a full clean rebuild. A progress bar
tracks the parse → embed → sync phases (an in-place bar on a TTY, sparse
percentage milestones otherwise) so a large project doesn't look frozen.

### Keeping it fresh automatically (dev)

When you run the dev server **with an embedding key configured**
(`SOLI_EMBEDDING_API_KEY`), the graph reindexes itself whenever you save a
`.sl`/`.slv` file — no flag needed, it just stays current while you work:

```bash
soli serve . --dev          # auto-reindex on (embedding key present)
```

It rides the dev file-watcher (debounced), runs on a background thread (off the
request path), and reuses the live route table rather than re-executing
`routes.sl`. Crucially it's **incremental on the expensive part**: it reuses the
existing embedding for every node whose text is unchanged and only re-embeds
what actually changed — so a one-file save costs a re-parse and a handful of
embeddings, not thousands, and your semantic layer stays intact.

Set `SOLI_GRAPH_WATCH=0` to turn it off, or `SOLI_GRAPH_WATCH=1` to force it on
even without an embedding key (structural-only reindex). The first reindex after
a server start does a full embed if there's no prior graph; after that it's
incremental.

## Any codebase (multi-language)

`soli graph` isn't limited to Soli apps — point it at **any repository** and pick
which files to index. The storage, embeddings, incremental sync, and
`soli graph query` are identical; only the extractor changes.

```bash
# index a Rails app: Ruby + templates
soli graph build /path/to/rails-app --ext rb,erb,slim

# or commit a .soligraph.toml and just run `soli graph build`
```

SolidB settings come from the project's `.env` (host, database, credentials),
exactly like a Soli app. A non-Soli repo won't have these, so add them:

```bash
# .env — required for `soli graph build` to reach SolidB
SOLIDB_HOST=http://localhost:6745      # required
SOLIDB_DATABASE=myapp_codegraph        # required (any db name; created on first write)
SOLIDB_USERNAME=admin                  # required for auth …
SOLIDB_PASSWORD=secret                 # … (or use SOLIDB_JWT / SOLIDB_API_KEY instead)

# optional — only needed to embed (semantic search). Without it, use --no-embed.
SOLI_EMBEDDING_API_KEY=sk-...
```

When you pass `--ext` (or a `.soligraph.toml` is present), the **generic
multi-language extractor** runs instead of the Soli one.

**Structural extraction (tree-sitter):** Ruby, Python, JavaScript/JSX,
TypeScript/TSX, Rust, and C# get real `class` / `module` / `method` / `function`
nodes plus `inherits`, `implements`, and `imports` edges. Every other extension
(`.erb`, `.slim`, `.md`, config, …) is **chunk-embedded** — split into windows
and embedded — so semantic search still covers it, just without structural
edges.

### `.soligraph.toml`

```toml
extensions  = ["rb", "erb", "slim"]   # what to index
exclude     = ["spec/", "db/migrate/"] # path substrings to skip
chunk_lines = 50                       # window for chunk-embedded files
```

Flags override the file: `--ext rb,py`, `--exclude spec/,tmp/`,
`--config path/to/config.toml`. Sensible directories are skipped by default
(`.git`, `node_modules`, `vendor`, `tmp`, `log`, `target`, `dist`, `build`,
`__pycache__`, dot-dirs).

**Note — call graph:** cross-file name resolution is best-effort (an
unambiguous name match only), so `inherits`/`imports` are reliable but a full
`calls` graph is not attempted for foreign languages. `--fresh`, `--dry-run`,
`--no-embed`, incremental MD5 skipping and non-destructive sync all work the
same as for Soli apps.

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

### The easy path: `soli graph query`

One command turns a natural-language task into the most relevant code **plus its
immediate relationships** — the graph-RAG payoff (semantic seed → graph
expansion) with no AQL to write:

```bash
soli graph query "where is authentication handled?"
soli graph query "refund flow" --json --limit 5 --hops 1
soli graph query "invoice validation" --path api/     # scope to one side of a mono-repo
```

It embeds the question, ANN-searches the vector index for seed nodes, and
expands each seed one hop over the graph (callers, callees, routes, views). If
the graph was built with `--no-embed` (no vector index) or no embedding key is
set, it falls back to a keyword-ranked scan, so the command always works.

`--json` emits a structured result an agent can parse directly:

```json
{
  "mode": "semantic",
  "query": "where is authentication handled?",
  "results": [
    {
      "score": 0.82,
      "kind": "method",
      "qualified_name": "SessionsController#create",
      "file": "app/controllers/sessions_controller.sl",
      "line": 12,
      "signature": "def create(req: Any) -> Any",
      "neighbors": [
        { "direction": "in",  "edge_kind": "routes_to", "kind": "route",  "name": "POST /login" },
        { "direction": "out", "edge_kind": "calls",     "kind": "method", "name": "User#authenticate" },
        { "direction": "out", "edge_kind": "renders",   "kind": "view",   "name": "sessions/new" }
      ]
    }
  ]
}
```

`--limit N` sets how many seed results to return (default 6); `--hops N` sets the
neighbour-expansion depth (default 1). `--path PREFIX` keeps only seeds whose
`file` starts with `PREFIX` (e.g. `--path api/` or `--path app/src/`), so an
agent can target one side of a mono-repo without post-filtering the JSON —
neighbours are unaffected. The semantic search over-fetches then filters (so an
out-of-path top ranking doesn't starve results), and the keyword fallback
filters server-side in AQL. The heavy `embedding`/`text` fields are never
included in the output.

### Raw queries

For anything the command doesn't cover, agents query SolidB directly. Two moves:
**find** nodes (by name, kind, or semantic similarity), then **traverse** from
them.

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

- Re-run `soli graph build` any time to refresh — it's incremental (hashes
  files, skips when unchanged, updates in place). In dev it auto-reindexes on
  save (see above), so you rarely need to run it by hand.
- `.slv` views become `view` nodes (their content is embedded too), so
  "find the view that renders X" is a semantic query.
- The graph is a **superset**: it works on any Soli codebase, but MVC roles
  (models, controllers, routes, views) get first-class node kinds.
