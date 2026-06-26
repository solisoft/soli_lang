//! Application template strings

/// Routes configuration template
pub const ROUTES_TEMPLATE: &str = include_str!("routes.sl");

/// `db/seeds.sl` starter template (run by `soli db:seed`)
pub const SEEDS_TEMPLATE: &str = include_str!("seeds.sl");

/// Application boot config (loaded by `soli serve` before routes)
pub const APPLICATION_CONFIG_TEMPLATE: &str = include_str!("application.sl");

/// Home controller template
pub const HOME_CONTROLLER_TEMPLATE: &str = include_str!("home_controller.sl");

/// Application layout template
pub const LAYOUT_TEMPLATE: &str = include_str!("application.html.slv");

/// Home index view template
pub const INDEX_VIEW_TEMPLATE: &str = include_str!("index.html.slv");

/// CSS file template
pub const CSS_TEMPLATE: &str = include_str!("app.css");

/// HTMx v1.9.12 — shipped in `public/js/htmx.min.js` of every new app.
pub const HTMX_JS: &str = include_str!("htmx.min.js");

/// Alpine.js v3.14.1 — shipped in `public/js/alpine.min.js` of every new app.
pub const ALPINE_JS: &str = include_str!("alpine.min.js");

/// Environment file template
pub const ENV_TEMPLATE: &str = include_str!("env.template");

/// Gitignore template
pub const GITIGNORE_TEMPLATE: &str = include_str!("gitignore.template");

/// CLAUDE.md template
pub const CLAUDE_MD_TEMPLATE: &str = include_str!("CLAUDE.md");

/// Application helper template
pub const APPLICATION_HELPER_TEMPLATE: &str = include_str!("application_helper.sl");

/// CORS middleware template
pub const CORS_MIDDLEWARE_TEMPLATE: &str = include_str!("cors.sl");

/// Auth middleware template
pub const AUTH_MIDDLEWARE_TEMPLATE: &str = include_str!("auth.sl");

/// FeatureFlags stdlib module template
pub const FEATURE_FLAGS_TEMPLATE: &str = include_str!("feature_flags.sl");

/// Generate package.json content
pub fn package_json(name: &str) -> String {
    format!(
        r#"{{
  "name": "{}",
  "version": "1.0.0",
  "description": "A Soli MVC application",
  "scripts": {{
    "build:css": "npx @tailwindcss/cli -i ./app/assets/css/application.css -o ./public/css/application.css",
    "watch:css": "npx @tailwindcss/cli -i ./app/assets/css/application.css -o ./public/css/application.css --watch"
  }},
  "devDependencies": {{
    "@tailwindcss/cli": "^4.3.1"
  }}
}}
"#,
        name
    )
}

/// Generate soli.toml content
pub fn soli_toml(name: &str) -> String {
    format!(
        r#"[package]
name = "{}"
version = "0.1.0"
main = "app.sl"
"#,
        name
    )
}

/// Generate README.md content
pub fn readme(name: &str) -> String {
    format!(
        r#"# {}

A Soli MVC application.

## Getting Started

### Development Server

Start the development server with hot reload:

```bash
soli serve . --dev
```

Your app will be available at [http://localhost:5011](http://localhost:5011)

### Production Server

Start the production server:

```bash
soli serve . --port 5011
```

Or run as a daemon:

```bash
soli serve . -d
```

## Project Structure

```
{}/
├── app/
│   ├── assets/
│   │   └── css/
│   │       └── application.css  # Source CSS with Tailwind directives
│   ├── controllers/     # Request handlers
│   ├── models/          # Data models
│   └── views/           # HTML templates
│       ├── home/        # Home page views
│       └── layouts/     # Layout templates
├── config/
│   └── routes.sl      # Route definitions
├── db/
│   ├── migrations/      # Database migrations
│   ├── seeds/           # Additional seed files (soli db:seed generate)
│   └── seeds.sl         # Database seeds (soli db:seed)
├── public/              # Static assets (compiled output)
│   ├── css/
│   │   └── application.css  # Compiled CSS (generated)
│   ├── js/
│   └── images/
├── tests/               # Test files
└── package.json         # npm dependencies (Tailwind config is CSS-first, in application.css)
```

## Database Migrations

Generate a new migration:

```bash
soli db:migrate generate create_users
```

Run pending migrations:

```bash
soli db:migrate up
```

Rollback last migration:

```bash
soli db:migrate down
```

Check migration status:

```bash
soli db:migrate status
```

## Database Seeds

Populate the database with sample or initial data. Edit `db/seeds.sl` (and add ordered
files under `db/seeds/`), then run:

```bash
soli db:seed
```

Seeds are not tracked and re-run every time, so keep them idempotent (guard inserts with
`first_by` / `find_by`). Generate an additional ordered seed file:

```bash
soli db:seed generate demo_users
```

## Documentation

- [Soli MVC Documentation](https://soli.solisoft.net/docs)
- [Soli Language Reference](https://soli.solisoft.net/docs/soli-language)
- [Authorization & Policies](https://soli.solisoft.test/docs/security/authorization)
- [Tailwind CSS](https://tailwindcss.com/docs)

## License

MIT
"#,
        name, name
    )
}
