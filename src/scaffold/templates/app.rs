//! Application template strings

/// Routes configuration template
pub const ROUTES_TEMPLATE: &str = include_str!("routes.sl");

/// Home controller template
pub const HOME_CONTROLLER_TEMPLATE: &str = include_str!("home_controller.sl");

/// Application layout template
pub const LAYOUT_TEMPLATE: &str = include_str!("application.html.slv");

/// Home index view template
pub const INDEX_VIEW_TEMPLATE: &str = include_str!("index.html.slv");

/// CSS file template
pub const CSS_TEMPLATE: &str = include_str!("app.css");

/// Environment file template
pub const ENV_TEMPLATE: &str = include_str!("env.template");

/// Gitignore template
pub const GITIGNORE_TEMPLATE: &str = include_str!("gitignore.template");

/// Application helper template
pub const APPLICATION_HELPER_TEMPLATE: &str = include_str!("application_helper.sl");

/// CORS middleware template
pub const CORS_MIDDLEWARE_TEMPLATE: &str = include_str!("cors.sl");

/// Auth middleware template
pub const AUTH_MIDDLEWARE_TEMPLATE: &str = include_str!("auth.sl");

/// Tailwind config template
pub const TAILWIND_CONFIG_TEMPLATE: &str = include_str!("tailwind.config.js");

/// Generate package.json content
pub fn package_json(name: &str) -> String {
    format!(
        r#"{{
  "name": "{}",
  "version": "1.0.0",
  "description": "A Soli MVC application",
  "scripts": {{
    "build:css": "npx tailwindcss -i ./public/css/app.css -o ./public/css/output.css",
    "watch:css": "npx tailwindcss -i ./public/css/app.css -o ./public/css/output.css --watch"
  }},
  "devDependencies": {{
    "tailwindcss": "^3.4.0"
  }}
}}
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

Your app will be available at [http://localhost:3000](http://localhost:3000)

### Production Server

Start the production server:

```bash
soli serve . --port 3000
```

Or run as a daemon:

```bash
soli serve . -d
```

## Project Structure

```
{}/
├── app/
│   ├── controllers/     # Request handlers
│   ├── models/          # Data models
│   └── views/           # HTML templates
│       ├── home/        # Home page views
│       └── layouts/     # Layout templates
├── config/
│   └── routes.sl      # Route definitions
├── db/
│   └── migrations/      # Database migrations
├── public/              # Static assets
│   ├── css/
│   │   ├── app.css      # Source CSS with Tailwind directives
│   │   └── output.css   # Compiled CSS (generated)
│   ├── js/
│   └── images/
├── tests/               # Test files
├── package.json         # npm dependencies
└── tailwind.config.js   # Tailwind configuration
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

## Documentation

- [Soli MVC Documentation](https://solilang.com/docs)
- [Soli Language Reference](https://solilang.com/docs/soli-language)
- [Tailwind CSS](https://tailwindcss.com/docs)

## License

MIT
"#,
        name, name
    )
}
