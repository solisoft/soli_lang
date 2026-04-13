# Engines

Engines are mini-applications that can be mounted at specific URL paths. Each engine has its own controllers, models, views, routes, and migrations—completely isolated from the main application.

## Why Engines?

Engines are useful for:

- **Modular architecture** — Build self-contained features that can be reused across projects
- **Plugin systems** — Ship features as packages with their own models and controllers
- **Microservices-lite** — Mount at URL paths with their own routing prefix
- **Code organization** — Separate concerns into distinct directories

## Engine Structure

```
engines/shop/
├── engine.sl              # Engine manifest
├── app/
│   ├── controllers/
│   ├── models/
│   ├── views/
│   └── helpers/
├── config/
│   └── routes.sl         # Engine-specific routes
└── db/
    └── migrations/       # Engine-specific migrations
```

## Engine Manifest

Each engine has a manifest file `engine.sl`:

```soli
engine "shop" {
    version: "1.0.0",
    dependencies: []
}
```

## Routes

Engine routes are relative to their mount point. If an engine is mounted at `/shop`, a route `get("/products", "shop#products")` will respond to `GET /shop/products`.

```soli
get("/", "shop#index")
get("/products", "shop#products")
get("/products/:id", "shop#product_detail")
```

## Mounting Engines

Define engines in `config/engines.sl`:

```soli
mount "shop", at: "/shop"
mount "blog", at: "/blog"
```

When mounted:
- Requests to `/shop/*` route to engine controllers
- Views are resolved from `engines/shop/app/views/`
- Models and migrations are namespaced to the engine

## Template Engine Integration

The template engine automatically resolves views from both the main app and mounted engines. When you call `render("shop/index", data)`:

1. First checks `app/views/shop/index.html.slv`
2. Then checks `engines/shop/app/views/shop/index.html.slv`

This allows engines to override or extend views while maintaining their own view directory structure.

## Creating Engines

Use the scaffold generator:

```bash
soli engine create shop
```

This creates the directory structure and starter files in `engines/shop/`.

## File Resolution

| File Type | Main App | Engine |
|-----------|----------|--------|
| Controllers | `app/controllers/` | `engines/<name>/app/controllers/` |
| Models | `app/models/` | `engines/<name>/app/models/` |
| Views | `app/views/` | `engines/<name>/app/views/` |
| Routes | `config/routes.sl` | `engines/<name>/config/routes.sl` |
| Migrations | `db/migrations/` | `engines/<name>/db/migrations/` |
