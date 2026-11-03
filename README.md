<p align="center">
  <img src="www/public/images/soli-logo.svg" alt="Soli Logo" width="120" height="120" />
</p>

<h1 align="center">Soli</h1>

<p align="center">
  <strong>A Ruby-flavoured language and a full-stack MVC framework, written in Rust.</strong>
</p>

<p align="center">
  <a href="#features">Features</a> •
  <a href="#quick-start">Quick Start</a> •
  <a href="#examples">Examples</a> •
  <a href="#documentation">Documentation</a> •
  <a href="#contributing">Contributing</a>
</p>

<p align="center">
  <img src="https://img.shields.io/badge/license-MIT-blue.svg" alt="License" />
  <img src="https://img.shields.io/badge/rust-stable-orange.svg" alt="Rust stable" />
  <img src="https://img.shields.io/badge/JSON-561k%20req%2Fs-green.svg" alt="561k req/s on the JSON benchmark" />
</p>

---

## Why Soli?

Soli brings Rails' expressiveness to a single Rust binary. You write terse, Ruby-like code with
optional type annotations; the binary runs it on a bytecode VM, serves it, tests it, lints it
and ships it. There is no `node_modules`, no separate app server and no build step to wire up.

```ruby
# app/controllers/posts_controller.sl — models are auto-loaded
class PostsController < Controller
  # GET /posts — no render call: posts/index renders with @posts
  def index
    @posts = Post.where({ published: true }).order("created_at", "desc").limit(10).all
  end

  # POST /posts
  def create
    @post = Post.create(permit(params, {"title": true, "body": true}))
    return render("new", {}, {"status": 422}) if @post._errors

    redirect("/posts/#{@post.id}")
  end
end
```

## Features

### Performance
- **Bytecode VM in production** for controllers, middleware, models and views
- **549k req/s** on a JSON response and **547k req/s** on a rendered 50-row page, with 16
  hardware threads — **12.7x Rails** on the JSON row, 3% behind Kemal (compiled Crystal). Full
  methodology, every stack and every caveat: [`www/docs/benchmarks.md`](www/docs/benchmarks.md)
- **Response cache** for pages that depend only on what they render, and **`grouped`** to ship
  several database reads in one round trip

### Data
- **SoliDB** by default (native driver), plus **PostgreSQL**, **MySQL**, **SQLite** and
  **Cloudflare D1** adapters — as document stores or against an existing SQL schema
- ORM with validations, callbacks, associations and `includes`, scopes, state machines,
  soft delete, transactions, `after_commit`, domain events and optimistic locking
- Migrations and seeds (`soli db:migrate`, `soli db:seed`), several named connections, and
  **one database per tenant** (`switch_tenant`, `with_tenant`, `--tenant`)
- Value objects with invariants, and `DomainError` mapped to a 422

### Web
- ERB-style views (`.html.slv`) with layouts, partials, components and `content_for`
- A Rails-style **form builder** (`form_with`), Rack-style nested params and `permit`
- **Live View** with DOM-aware morphing, streams and nested components; WebSockets
- htmx and Alpine bundled, instant navigation, Tailwind CSS v4 compiled with no npm
- OpenAPI documents generated from routes and doc comments
- Background jobs and cron, mailer, file attachments (disk, S3, R2), PDF rendering, i18n

### Security
- CSRF protection (Origin gate plus per-session tokens), escaped-by-default templates
- Sessions with in-memory, encrypted-cookie, disk, SoliDB and SoliKV drivers; signed and
  encrypted cookies
- Generators for session auth with policies, OAuth sign-in and an OpenID Connect provider
- `soli lint` flags unfiltered mass assignment, among others

### Tooling
- `soli serve --dev`: hot reload, error pages with variable inspection, a dev bar with
  queries, N+1 detection and flame graphs
- Built-in error tracking and slow-query pages at `/__soli/*` in production
- `soli test` (specs, E2E requests, browser tests, coverage), `soli lint`, `soli fmt`,
  `soli check`, and a language server (`soli lsp`)
- Scaffolds, `soli routes`, a REPL, and a project-local pinned interpreter version

### Deploy
- `soli build` produces an app bundle or one standalone executable
- `soli deploy`, a Docker image (`ghcr.io/solisoft/soli_lang`), and `soli update` in place
- **Cloudflare Workers**: `soli new my_app --edge`, with D1, Durable Objects, R2, Queues and
  Hyperdrive
- Native shells for Android, iOS, Linux and Windows (`soli generate client`), and EUI windows

## Quick Start

### Installation

```bash
# Quick install (latest release binary)
curl -sSL https://raw.githubusercontent.com/solisoft/soli_lang/main/install.sh | sh
```

This installs to `~/.local/bin`. When run as **root** (e.g. `... | sudo sh`, or in a Docker
build) it installs globally to `/usr/local/bin` so every user can run `soli`. Pass `--system`
to force a global install as a non-root user, or `--user` to force a per-user install as root.

Already installed? `soli update` upgrades in place — use `sudo soli update` if Soli lives in a
root-owned directory like `/usr/local/bin`.

With Docker:

```bash
docker run --rm ghcr.io/solisoft/soli_lang:latest --version
```

From source:

```bash
git clone https://github.com/solisoft/soli_lang.git
cd soli_lang
cargo build --release
export PATH="$PATH:$(pwd)/target/release"
```

### Create a New Project

```bash
soli new my_app
cd my_app
soli serve . --dev
```

Visit [http://localhost:5011](http://localhost:5011). The generated `.env` points at a SoliDB
server on `localhost:6745`; see [`www/docs/database.md`](www/docs/database.md) for it and
[`www/docs/multi-database.md`](www/docs/multi-database.md) for PostgreSQL, MySQL and SQLite.

### Generate a Resource

```bash
soli generate scaffold post title:string content:text published:boolean
soli db:migrate up
```

This creates the model, a controller with the CRUD actions, the views (index, show, new, edit
and a form partial), a migration, a controller spec, and the routes.

### Run a Script

Soli is a general-purpose language too — a file runs top to bottom:

```bash
soli hello.sl
```

## Project Structure

```
my_app/
├── app/
│   ├── assets/css/       # Tailwind entry point
│   ├── controllers/      # Request handlers
│   ├── helpers/          # View helpers
│   ├── middleware/       # Request filters
│   ├── models/           # Data models (+ concerns/)
│   └── views/            # .html.slv templates (+ layouts/)
├── config/
│   ├── routes.sl         # URL routing
│   └── application.sl    # Boot configuration
├── db/
│   ├── migrations/       # Database migrations
│   └── seeds.sl
├── public/               # Static assets
├── tests/                # Specs
├── .env                  # Database and app settings
├── soli.toml             # Package manifest and pinned Soli version
└── CLAUDE.md, AGENTS.md  # Guides for AI coding agents
```

## Examples

### Routing

```ruby
# config/routes.sl
get("/", "home#index")
get("/about", "home#about")

resources("posts")              # RESTful routes

namespace("api/v1", -> {
  get("/status", "api/v1/status#show")
})

cors("/api/*", {"origins": ["https://app.example.com"]})
```

### Models

```ruby
# app/models/post.sl
class Post < Model
  validates("title", {"presence": true, "min_length": 3})
  validates("body", {"presence": true})

  belongs_to("author")
  has_many("comments")

  def summary -> String
    @body.length > 140 ? @body.substring(0, 140) + "…" : @body
  end
end
```

`Post.find(id)` raises when the record does not exist and the request becomes a 404;
`Post.find_by("slug", slug)` returns `nil` instead.

### Views

```erb
<!-- app/views/posts/index.html.slv -->
<h1>Latest posts</h1>

<% for post in @posts %>
  <article>
    <h2><%= post.title %></h2>
    <p><%= post.summary %></p>
    <a href="/posts/<%= post.id %>">Read more</a>
  </article>
<% end %>
```

### Forms

```erb
<%- form_with(@post) do |f| -%>
  <%- f.error_summary() %>
  <%- f.label("title") %>
  <%- f.text_field("title") %>
  <%- f.text_area("body") %>
  <%- f.submit("Save") %>
<%- end -%>
```

### Live View

Reactive UIs without writing JavaScript:

```erb
<!-- app/views/live/counter.html.slv -->
<div class="counter">
  <h2><%= count %></h2>
  <button soli-click="decrement">-</button>
  <button soli-click="increment">+</button>
</div>
```

```ruby
# config/routes.sl
router_live("counter", "live#counter")
```

```ruby
# app/controllers/live_controller.sl
def counter(event_data)
  count = event_data["state"]["count"] || 0

  match event_data["event"] {
    "increment" => { "count": count + 1 },
    "decrement" => { "count": count - 1 },
    _ => { "count": count }
  }
end
```

Patches are DOM-aware: the client (served by the binary at `/live/client.js`) morphs nodes in
place with keyed reconciliation (`soli-key`, `soli-ignore`), so focus and client-side widget
state survive updates.

## Testing

```ruby
# tests/controllers/posts_controller_spec.sl
describe("PostsController") do
  before_each() do
    as_guest()
  end

  test("lists posts") do
    response = get("/posts")
    assert_eq(res_status(response), 200)
    assert_hash_has_key(assigns(), "posts")
  end

  test("creates a post when signed in") do
    login("user@example.com", "password")
    response = post("/posts", {"title": "New post", "body": "Content here"})
    assert_eq(res_status(response), 302)
  end
end
```

```bash
soli test                                   # every spec under tests/
soli test tests/controllers/posts_controller_spec.sl
soli test --coverage-min 90                 # raise the coverage floor (default 80%)
soli test --browser                         # browser specs
```

## Documentation

The documentation site is [`www/`](www/), itself a Soli app; its Markdown sources are in
[`www/docs/`](www/docs/). Start with:

- [Introduction](www/docs/introduction.md) and [Installation](www/docs/installation.md)
- [The Soli language](www/docs/soli-language.md)
- [Routing](www/docs/routing.md), [Controllers](www/docs/controllers.md),
  [Views](www/docs/views.md), [Forms](www/docs/forms.md)
- [Models](www/docs/models.md), [Migrations](www/docs/migrations.md),
  [Multiple databases](www/docs/multi-database.md)
- [Live View](www/docs/liveview.md), [Jobs](www/docs/jobs.md),
  [Authentication](www/docs/authentication.md)
- [Testing](www/docs/testing.md), [Deploy](www/docs/deploy.md),
  [Cloudflare Workers](www/docs/edge.md)
- [Benchmarks](www/docs/benchmarks.md)

## Editor Support

`soli lsp` is a language server over stdio — diagnostics, formatting and more — for any editor
with an LSP client; see [`www/docs/editor-integration.md`](www/docs/editor-integration.md).
Syntax highlighting extensions live in [`editors/vscode`](editors/vscode) and
[`editors/nova`](editors/nova).

## Contributing

Contributions are welcome. Before opening a pull request, run the CI gate locally:

```bash
cargo fmt
cargo clippy --all-targets -- -D warnings
cargo test
soli test tests/
```

1. Fork the repository
2. Create a feature branch (`git checkout -b feature/amazing-feature`)
3. Commit your changes, with the docs that describe them
4. Push the branch and open a Pull Request

## License

This project is licensed under the MIT License — see the [LICENSE](LICENSE) file for details.

## Acknowledgments

- Inspired by Ruby on Rails, Phoenix and Laravel
- Built with Rust for performance and safety
- Stores documents in [SoliDB](https://github.com/solisoft/solidb) by default

---

<p align="center">
  Built with love by <a href="https://github.com/solisoft">solisoft</a>
</p>
