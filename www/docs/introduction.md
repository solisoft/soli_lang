# Introduction to Soli

Soli is a dynamically-typed, high-performance web framework and programming language written in Rust. It combines Ruby-like expressiveness with production-grade performance — 170,000+ requests/second and sub-millisecond response times on a single server, backed by a bytecode VM.

This guide gives you a full tour of what Soli can do, then gets you from zero to a running app in three commands.

## What is Soli?

Soli is two things in one:

- **A language** — dynamically typed, with *optional* type annotations you can add where they help (documentation, IDE support, runtime validation). No compile step gates execution, but `soli check` will static type-check the parts you've annotated.
- **A web framework** — a batteries-included MVC stack (routing, ORM, views, jobs, mailer, WebSockets, auth) built on top of it, following convention over configuration.

| | |
|---|---|
| **170,000+ req/s** | Single-server throughput on the built-in HTTP server |
| **Sub-millisecond** | Typical response times, thanks to the bytecode VM |
| **3 commands** | From install to a running app — no `npm install` required |

## Feature Tour

### Language

- **Dynamic typing, optional annotations** — write fast, annotate where it pays off.
- **Classes, inheritance & interfaces** — familiar OOP with `class Foo < Bar`.
- **Pattern matching** — `match` with guards and destructuring on arrays/hashes.
- **Enums** — first-class enumerated types.
- **Closures & lambdas** — `fn(x) { ... }` and `|x| { ... }` pipe syntax.
- **Pipeline operator (`|>`)** — chain data transformations left to right.
- **Error handling** — `try/catch/finally` plus postfix `rescue` for one-line fallbacks.
- **Modern conveniences** — string interpolation (`#{}`), nullish coalescing (`??`), safe navigation (`&.`), spread/rest operators, default & named parameters, comprehensions, raw/multiline strings, symbols, percent literals (`%w`, `%i`, `%n`).
- **Metaprogramming** — `define_method`, `method_missing`, and friends.

→ [Full language reference](/docs/language)

### Web Framework

- **Convention-over-configuration autoloading** — drop files in `app/controllers`, `app/models`, `app/services`, `app/policies`, `app/jobs`, `app/mailers` and Soli finds them.
- **Declarative routing** — RESTful `resources()`, nested resources, namespaces, and named route helpers (`posts_path`, `edit_post_path(post)`).
- **ERB-style views** (`.html.slv`) — layouts, partials, and helpers.
- **Middleware pipeline** — request/response interceptors for auth, CORS, logging, and more.

→ [Routing](/docs/core-concepts/routing) · [Controllers](/docs/core-concepts/controllers) · [Views](/docs/core-concepts/views) · [Middleware](/docs/core-concepts/middleware)

### Data & Persistence

- **SoliDB** — a built-in document database queried with AQL.
- **Active-Record-style Model API** — `find`, `where`, `order`, `limit`, `create`, `update`, `delete`.
- **Relationships** — `belongs_to`, `has_many`, and has-and-belongs-to-many.
- **Validations & callbacks**, **migrations**, **state machines**, and query scopes.

→ [Models](/docs/database/models) · [Query Builder](/docs/database/query-builder) · [Relationships](/docs/database/relationships) · [Migrations](/docs/database/migrations)

### Realtime

- **WebSockets** — first-class support for bidirectional connections.
- **Live View** — server-rendered reactive components (`soli-click`, `soli-submit`, and friends), no client-side JavaScript required.
- **Streaming / SSE** — push updates to the browser as they happen.

→ [WebSockets](/docs/core-concepts/websockets) · [Live View](/docs/core-concepts/liveview) · [Streaming](/docs/core-concepts/streaming)

### Batteries Included

- **Auth & security** — JWT stateless auth, Pundit-style policy-based authorization, CSRF protection, XSS sanitization, Argon2 password hashing, secure cookies.
- **Sessions** — four pluggable backends: in-memory, disk, SoliDB, or SoliKV.
- **Background jobs & cron** — `perform_later`, `perform_in`, `perform_at`, and scheduled jobs.
- **Mailer** — HTML and text emails, attachments, SMTP delivery.
- **Document generation** — PDF rendering, including Factur-X/EN16931 e-invoicing.
- **i18n** — multi-locale support out of the box.

→ [Authentication](/docs/security/authentication) · [Authorization](/docs/security/authorization) · [Sessions](/docs/security/sessions) · [Jobs](/docs/builtins/jobs) · [Mailer](/docs/builtins/mailer) · [PDF](/docs/builtins/pdf) · [i18n](/docs/core-concepts/i18n)

### Developer Tools

- **Hot reload** — edit and refresh, no restart.
- **Beautiful dev error pages** — with variable inspection.
- **Scaffold generator** — `soli generate scaffold` produces a complete resource in seconds.
- **Built-in package manager** — `soli add`, `soli install`, `soli publish` against a registry, configured via `soli.toml`.
- **Lint, format, type-check** — `soli lint`, `soli fmt`, `soli check`.
- **LSP + editor integration** — a language server and VS Code extension for autocomplete, diagnostics, and go-to-definition.
- **BDD testing framework** — `describe`/`test`/`before_each`, HTTP integration testing, and coverage reporting.

→ [Live Reload](/docs/development-tools/live-reload) · [Scaffold](/docs/development-tools/scaffold) · [Linting](/docs/development-tools/linting) · [Formatting](/docs/development-tools/formatting) · [Editor Integration](/docs/development-tools/editor-integration) · [Testing](/docs/testing)

## The MVC Pattern

Soli follows the **Model-View-Controller (MVC)** architectural pattern:

- **Model** (`app/models/`) — manages data, business logic, and database interactions.
- **View** (`app/views/`) — handles presentation and HTML rendering.
- **Controller** (`app/controllers/`) — orchestrates the flow between models and views.

## Quick Start

```bash
# Install the Soli CLI
curl -sSL https://raw.githubusercontent.com/solisoft/soli_lang/main/install.sh | sh

# Scaffold a new app
soli new my_app
cd my_app

# Start the dev server (auto-compiles Tailwind CSS, hot reload enabled)
soli serve . --dev
```

Your app is now running at `http://localhost:5011`. No `npm install` needed to get started — `--dev` mode compiles your Tailwind CSS for you.

## Project Structure

```
my_app/
├── app/
│   ├── controllers/       # Request handlers
│   ├── jobs/               # Background jobs
│   ├── middleware/         # Request interceptors
│   ├── views/              # HTML templates (.html.slv)
│   └── assets/
│       └── css/            # Tailwind source
├── config/
│   ├── routes.sl          # Route definitions
│   ├── application.sl     # App configuration
│   └── locales/            # i18n translation files
├── public/
│   └── css/                # Compiled CSS output
└── package.json            # Tailwind toolchain (only needed if customizing)
```

## Quick Example

**1. Define a route** (`config/routes.sl`):

```soli
get("/", "home#index")
get("/users/:id", "users#show")
```

**2. Create a controller** (`app/controllers/home_controller.sl`):

```soli
class HomeController < Controller
  def index
    message = "Welcome to Soli!"

    render("home/index", {
      "title": "Home",
      "message": message
    })
  end
end
```

**3. Build a view** (`app/views/home/index.html.slv`):

```erb
<h1><%= message %></h1>
<p>Start building something amazing.</p>
```

## The Package Manager

Soli ships with a built-in package manager, no separate tool needed:

```bash
soli init                       # create soli.toml in the current directory
soli add utils --path ../shared/utils
soli add soli-math --version 1.0.0
soli install                    # install everything from soli.toml
soli publish                    # publish your package to a registry
```

The manifest can also pin a minimum interpreter version with `soli_version = "1.16.0"` in `[package]`; `soli serve`/`test`/`run` then refuse to start on an older `soli`.

See [Modules & Packages](/docs/language/modules) for the full `soli.toml` reference.

## Design Philosophy

Soli favors convention over configuration. By following standard naming patterns, you write less glue code and focus on building features.

## Next Steps

- **[Installation](/docs/getting-started/installation)** — get the CLI installed and your first app running.
- **[Configuration](/docs/getting-started/configuration)** — environment variables for databases, sessions, jobs, and deployment.
- **[Routing](/docs/core-concepts/routing)** — define your application's URLs.
- **[Language Reference](/docs/language)** — the complete Soli syntax, types, and language features.
- **[Models](/docs/database/models)** — ORM-style data modeling with CRUD operations.
- **[Deploy](/docs/development-tools/deploy)** — ship to production with blue-green deployment.
