//! Scaffold module for generating new Soli MVC applications.
//!
//! Provides functionality for `soli new app_name` command.

use std::fs::{self, File};
use std::io::Write;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

/// A field definition parsed from scaffold arguments
#[derive(Debug, Clone)]
struct FieldDefinition {
    name: String,
    field_type: String,
}

impl FieldDefinition {
    fn parse(field_str: &str) -> Option<Self> {
        let parts: Vec<&str> = field_str.split(':').collect();
        match parts.as_slice() {
            [name, field_type] => Some(Self {
                name: name.to_string(),
                field_type: field_type.to_string(),
            }),
            _ => None,
        }
    }

    fn to_snake_case(&self) -> String {
        let mut result = String::new();
        for (i, c) in self.name.chars().enumerate() {
            if c.is_uppercase() {
                if i > 0 {
                    result.push('_');
                }
                result.push(c.to_ascii_lowercase());
            } else {
                result.push(c);
            }
        }
        result
    }

    fn to_title_case(&self) -> String {
        let snake = self.to_snake_case();
        snake
            .split('_')
            .map(|word| {
                let mut chars = word.chars();
                match chars.next() {
                    Some(c) => c.to_ascii_uppercase().to_string() + chars.as_str(),
                    None => String::new(),
                }
            })
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// Create scaffold for a resource (model, controller, views)
pub fn create_scaffold(folder: &str, name: &str) -> Result<(), String> {
    create_scaffold_with_fields(folder, name, &[])
}

pub fn create_scaffold_with_fields(
    folder: &str,
    name: &str,
    fields: &[String],
) -> Result<(), String> {
    let app_path = Path::new(folder);

    if !app_path.exists() {
        return Err(format!("Directory '{}' does not exist", folder));
    }

    if !app_path.is_dir() {
        return Err(format!("'{}' is not a directory", folder));
    }

    let parsed_fields: Vec<FieldDefinition> = fields
        .iter()
        .filter_map(|f| FieldDefinition::parse(f))
        .collect();

    // Ensure directory structure exists
    ensure_directory_structure(app_path)?;

    // Create model
    create_model(app_path, name, &parsed_fields)?;

    // Create controller
    create_controller(app_path, name)?;

    // Create views (index, show, new, edit)
    create_views(app_path, name, &parsed_fields)?;

    // Create form partial (shared by new/edit)
    create_form_partial(app_path, name, &parsed_fields)?;

    // Create migration
    create_migration(app_path, name, &parsed_fields)?;

    // Create tests
    create_tests(app_path, name)?;

    // Add routes
    add_routes(app_path, name)?;

    Ok(())
}

fn create_directories(app_path: &Path) -> Result<(), String> {
    let dirs = [
        "",
        "app",
        "app/controllers",
        "app/models",
        "app/views",
        "app/views/home",
        "app/views/layouts",
        "config",
        "db",
        "db/migrations",
        "public",
        "public/css",
        "public/js",
        "public/images",
        "tests",
    ];

    for dir in dirs {
        let path = app_path.join(dir);
        fs::create_dir_all(&path)
            .map_err(|e| format!("Failed to create directory '{}': {}", path.display(), e))?;
    }

    Ok(())
}

fn write_file(path: &Path, content: &str) -> Result<(), String> {
    let mut file =
        File::create(path).map_err(|e| format!("Failed to create '{}': {}", path.display(), e))?;
    file.write_all(content.as_bytes())
        .map_err(|e| format!("Failed to write to '{}': {}", path.display(), e))?;
    Ok(())
}

fn create_routes_file(app_path: &Path) -> Result<(), String> {
    let content = r#"// Routes configuration
// Define your application routes here

// Home page
get("/", "home#index");

// Health check endpoint
get("/health", "home#health");

print("Routes loaded!");
"#;
    write_file(&app_path.join("config/routes.soli"), content)
}

fn create_home_controller(app_path: &Path) -> Result<(), String> {
    let content = r#"// Home controller - handles the root routes

class HomeController extends Controller {
    // GET /
    fn index(req: Any) -> Any {
        return render("home/index", {
            "title": "Welcome"
        });
    }

    // GET /health
    fn health(req: Any) -> Any {
        return {
            "status": 200,
            "headers": {"Content-Type": "application/json"},
            "body": "{\"status\":\"ok\"}"
        };
    }
}
"#;
    write_file(
        &app_path.join("app/controllers/home_controller.soli"),
        content,
    )
}

fn create_layout(app_path: &Path) -> Result<(), String> {
    let content = r##"<!DOCTYPE html>
<html lang="en" class="h-full">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title><%= title %> - Soli App</title>

    <!-- Tailwind CSS via CDN -->
    <script src="https://cdn.tailwindcss.com"></script>

    <!-- Custom styles -->
    <link rel="stylesheet" href="<%= public_path("css/app.css") %>">

    <script>
        tailwind.config = {
            theme: {
                extend: {
                    fontFamily: {
                        sans: ['Inter', 'system-ui', 'sans-serif'],
                        mono: ['JetBrains Mono', 'monospace'],
                    },
                }
            }
        }
    </script>

    <!-- Google Fonts -->
    <link rel="preconnect" href="https://fonts.googleapis.com">
    <link rel="preconnect" href="https://fonts.gstatic.com" crossorigin>
    <link href="https://fonts.googleapis.com/css2?family=Inter:wght@300;400;500;600;700;800&family=JetBrains+Mono:wght@400;500&display=swap" rel="stylesheet">
</head>
<body class="min-h-full bg-slate-950 text-white font-sans antialiased">
    <%= yield %>
</body>
</html>
"##;
    write_file(
        &app_path.join("app/views/layouts/application.html.erb"),
        content,
    )
}

fn create_index_view(app_path: &Path) -> Result<(), String> {
    let content = r##"<div class="min-h-screen relative overflow-hidden">
    <!-- Animated background gradient -->
    <div class="absolute inset-0 bg-gradient-to-br from-slate-950 via-slate-900 to-slate-950"></div>
    <div class="absolute inset-0 bg-[radial-gradient(ellipse_80%_50%_at_50%_-20%,rgba(99,102,241,0.15),transparent)]"></div>
    <div class="absolute inset-0 bg-[radial-gradient(ellipse_60%_40%_at_100%_100%,rgba(168,85,247,0.1),transparent)]"></div>

    <!-- Grid pattern overlay -->
    <div class="absolute inset-0 bg-[linear-gradient(rgba(255,255,255,0.02)_1px,transparent_1px),linear-gradient(90deg,rgba(255,255,255,0.02)_1px,transparent_1px)] bg-[size:64px_64px]"></div>

    <!-- Content -->
    <div class="relative z-10 flex flex-col items-center justify-center min-h-screen px-4">
        <!-- Logo -->
        <div class="mb-8 relative group">
            <div class="absolute -inset-4 bg-gradient-to-r from-indigo-500 via-purple-500 to-pink-500 rounded-3xl blur-2xl opacity-20 group-hover:opacity-30 transition-opacity duration-500"></div>
            <div class="relative w-24 h-24 rounded-2xl bg-gradient-to-br from-indigo-500 via-purple-500 to-pink-500 flex items-center justify-center shadow-2xl shadow-indigo-500/25">
                <span class="text-5xl font-bold text-white">S</span>
            </div>
        </div>

        <!-- Title -->
        <h1 class="text-5xl md:text-7xl font-bold text-center mb-4 tracking-tight">
            <span class="bg-gradient-to-r from-white via-slate-200 to-slate-400 bg-clip-text text-transparent">
                Welcome Aboard
            </span>
        </h1>

        <p class="text-xl md:text-2xl text-slate-400 text-center mb-12 max-w-2xl">
            Your Soli MVC application is ready to go.
        </p>

        <!-- Status badge -->
        <div class="flex items-center gap-2 px-4 py-2 rounded-full bg-emerald-500/10 border border-emerald-500/20 mb-12">
            <div class="w-2 h-2 rounded-full bg-emerald-400 animate-pulse"></div>
            <span class="text-emerald-400 text-sm font-medium">Server running</span>
        </div>

        <!-- Quick Start Cards -->
        <div class="grid grid-cols-1 md:grid-cols-3 gap-6 max-w-4xl w-full mb-16">
            <!-- Documentation -->
            <a href="https://solilang.com/docs" target="_blank" class="group relative p-6 rounded-2xl bg-white/5 border border-white/10 hover:border-indigo-500/50 transition-all duration-300 hover:shadow-lg hover:shadow-indigo-500/10">
                <div class="absolute inset-0 bg-gradient-to-br from-indigo-500/10 to-transparent opacity-0 group-hover:opacity-100 transition-opacity rounded-2xl"></div>
                <div class="relative">
                    <div class="w-12 h-12 rounded-xl bg-indigo-500/10 flex items-center justify-center mb-4 group-hover:scale-110 transition-transform">
                        <svg class="w-6 h-6 text-indigo-400" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M12 6.253v13m0-13C10.832 5.477 9.246 5 7.5 5S4.168 5.477 3 6.253v13C4.168 18.477 5.754 18 7.5 18s3.332.477 4.5 1.253m0-13C13.168 5.477 14.754 5 16.5 5c1.747 0 3.332.477 4.5 1.253v13C19.832 18.477 18.247 18 16.5 18c-1.746 0-3.332.477-4.5 1.253" />
                        </svg>
                    </div>
                    <h3 class="text-lg font-semibold text-white mb-2">Documentation</h3>
                    <p class="text-slate-400 text-sm">Learn about controllers, models, views, and more.</p>
                </div>
            </a>

            <!-- Tailwind CSS -->
            <a href="https://tailwindcss.com/docs" target="_blank" class="group relative p-6 rounded-2xl bg-white/5 border border-white/10 hover:border-cyan-500/50 transition-all duration-300 hover:shadow-lg hover:shadow-cyan-500/10">
                <div class="absolute inset-0 bg-gradient-to-br from-cyan-500/10 to-transparent opacity-0 group-hover:opacity-100 transition-opacity rounded-2xl"></div>
                <div class="relative">
                    <div class="w-12 h-12 rounded-xl bg-cyan-500/10 flex items-center justify-center mb-4 group-hover:scale-110 transition-transform">
                        <svg class="w-6 h-6 text-cyan-400" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M7 21a4 4 0 01-4-4V5a2 2 0 012-2h4a2 2 0 012 2v12a4 4 0 01-4 4zm0 0h12a2 2 0 002-2v-4a2 2 0 00-2-2h-2.343M11 7.343l1.657-1.657a2 2 0 012.828 0l2.829 2.829a2 2 0 010 2.828l-8.486 8.485M7 17h.01" />
                        </svg>
                    </div>
                    <h3 class="text-lg font-semibold text-white mb-2">Tailwind CSS</h3>
                    <p class="text-slate-400 text-sm">Pre-configured with Tailwind for rapid UI development.</p>
                </div>
            </a>

            <!-- GitHub -->
            <a href="https://github.com/solisoft/soli_lang" target="_blank" class="group relative p-6 rounded-2xl bg-white/5 border border-white/10 hover:border-purple-500/50 transition-all duration-300 hover:shadow-lg hover:shadow-purple-500/10">
                <div class="absolute inset-0 bg-gradient-to-br from-purple-500/10 to-transparent opacity-0 group-hover:opacity-100 transition-opacity rounded-2xl"></div>
                <div class="relative">
                    <div class="w-12 h-12 rounded-xl bg-purple-500/10 flex items-center justify-center mb-4 group-hover:scale-110 transition-transform">
                        <svg class="w-6 h-6 text-purple-400" fill="currentColor" viewBox="0 0 24 24">
                            <path fill-rule="evenodd" d="M12 2C6.477 2 2 6.484 2 12.017c0 4.425 2.865 8.18 6.839 9.504.5.092.682-.217.682-.483 0-.237-.008-.868-.013-1.703-2.782.605-3.369-1.343-3.369-1.343-.454-1.158-1.11-1.466-1.11-1.466-.908-.62.069-.608.069-.608 1.003.07 1.531 1.032 1.531 1.032.892 1.53 2.341 1.088 2.91.832.092-.647.35-1.088.636-1.338-2.22-.253-4.555-1.113-4.555-4.951 0-1.093.39-1.988 1.029-2.688-.103-.253-.446-1.272.098-2.65 0 0 .84-.27 2.75 1.026A9.564 9.564 0 0112 6.844c.85.004 1.705.115 2.504.337 1.909-1.296 2.747-1.027 2.747-1.027.546 1.379.202 2.398.1 2.651.64.7 1.028 1.595 1.028 2.688 0 3.848-2.339 4.695-4.566 4.943.359.309.678.92.678 1.855 0 1.338-.012 2.419-.012 2.747 0 .268.18.58.688.482A10.019 10.019 0 0022 12.017C22 6.484 17.522 2 12 2z" clip-rule="evenodd" />
                        </svg>
                    </div>
                    <h3 class="text-lg font-semibold text-white mb-2">GitHub</h3>
                    <p class="text-slate-400 text-sm">Star us on GitHub and join the community.</p>
                </div>
            </a>
        </div>

        <!-- Quick Start Code -->
        <div class="max-w-2xl w-full">
            <h2 class="text-lg font-semibold text-white mb-4 flex items-center gap-2">
                <svg class="w-5 h-5 text-indigo-400" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                    <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M13 10V3L4 14h7v7l9-11h-7z" />
                </svg>
                Quick Start
            </h2>
            <div class="rounded-xl bg-slate-900/80 border border-white/10 overflow-hidden">
                <div class="flex items-center gap-2 px-4 py-3 bg-slate-800/50 border-b border-white/5">
                    <div class="w-3 h-3 rounded-full bg-red-500/80"></div>
                    <div class="w-3 h-3 rounded-full bg-yellow-500/80"></div>
                    <div class="w-3 h-3 rounded-full bg-green-500/80"></div>
                    <span class="ml-2 text-xs text-slate-500 font-mono">Terminal</span>
                </div>
                <div class="p-4 font-mono text-sm">
                    <div class="text-slate-500"># Edit your routes</div>
                    <div class="text-emerald-400">vim config/routes.soli</div>
                    <div class="mt-3 text-slate-500"># Create a new controller</div>
                    <div class="text-emerald-400">vim app/controllers/posts_controller.soli</div>
                    <div class="mt-3 text-slate-500"># Create a view</div>
                    <div class="text-emerald-400">vim app/views/posts/index.html.erb</div>
                    <div class="mt-3 text-slate-500"># Restart with hot reload</div>
                    <div class="text-emerald-400">soli serve . --dev</div>
                </div>
            </div>
        </div>

        <!-- Footer -->
        <div class="mt-16 text-center">
            <p class="text-slate-500 text-sm">
                Built with
                <span class="text-pink-400">&hearts;</span>
                using
                <a href="https://solilang.com" class="text-indigo-400 hover:text-indigo-300 transition-colors">Soli</a>
            </p>
        </div>
    </div>
</div>
"##;
    write_file(&app_path.join("app/views/home/index.html.erb"), content)
}

fn create_css_file(app_path: &Path) -> Result<(), String> {
    let content = r#"/* Custom application styles */
/* Tailwind CSS is loaded via CDN in the layout */

/* Add your custom styles here */

/* Example: Custom animations */
@keyframes float {
    0%, 100% { transform: translateY(0); }
    50% { transform: translateY(-10px); }
}

.animate-float {
    animation: float 3s ease-in-out infinite;
}

/* Example: Custom scrollbar */
::-webkit-scrollbar {
    width: 8px;
    height: 8px;
}

::-webkit-scrollbar-track {
    background: transparent;
}

::-webkit-scrollbar-thumb {
    background: rgba(99, 102, 241, 0.3);
    border-radius: 4px;
}

::-webkit-scrollbar-thumb:hover {
    background: rgba(99, 102, 241, 0.5);
}
"#;
    write_file(&app_path.join("public/css/app.css"), content)
}

fn create_env_file(app_path: &Path) -> Result<(), String> {
    let content = r#"# Database Configuration
# These variables are used by soli db:migrate commands

SOLIDB_HOST=http://localhost:6745
SOLIDB_DATABASE=default
SOLIDB_USERNAME=admin
SOLIDB_PASSWORD=admin

# Application Settings
# APP_ENV=development
# APP_SECRET=your-secret-key-here
"#;
    write_file(&app_path.join(".env"), content)
}

fn create_gitignore(app_path: &Path) -> Result<(), String> {
    let content = r#"# Soli MVC
soli.pid
soli.log

# Dependencies
node_modules/

# Build artifacts
/target/
*.o
*.so

# IDE
.idea/
.vscode/
*.swp
*.swo
*~

# OS
.DS_Store
Thumbs.db

# Environment
.env
.env.local
.env.*.local

# Logs
*.log
logs/

# Coverage
coverage/
.coverage
"#;
    write_file(&app_path.join(".gitignore"), content)
}

fn create_readme(app_path: &Path, name: &str) -> Result<(), String> {
    let content = format!(
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
│   └── routes.soli      # Route definitions
├── db/
│   └── migrations/      # Database migrations
├── public/              # Static assets
│   ├── css/
│   ├── js/
│   └── images/
└── tests/               # Test files
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
    );
    write_file(&app_path.join("README.md"), &content)
}

/// Print success message after creating an app
pub fn print_success_message(name: &str) {
    println!();
    println!(
        "  \x1b[32m\x1b[1mSuccess!\x1b[0m Created \x1b[1m{}\x1b[0m",
        name
    );
    println!();
    println!("  \x1b[2mGet started:\x1b[0m");
    println!();
    println!("    \x1b[36mcd {}\x1b[0m", name);
    println!("    \x1b[36msoli serve . --dev\x1b[0m");
    println!();
    println!("  \x1b[2mThen open\x1b[0m \x1b[4mhttp://localhost:3000\x1b[0m");
    println!();
}

/// Create a new Soli MVC application
pub fn create_app(name: &str) -> Result<(), String> {
    let app_path = Path::new(name);

    if app_path.exists() {
        return Err(format!("Directory '{}' already exists", name));
    }

    // Create directory structure
    create_directories(app_path)?;

    // Create files
    create_routes_file(app_path)?;
    create_home_controller(app_path)?;
    create_layout(app_path)?;
    create_index_view(app_path)?;
    create_css_file(app_path)?;
    create_env_file(app_path)?;
    create_gitignore(app_path)?;
    create_readme(app_path, name)?;

    Ok(())
}

fn ensure_directory_structure(app_path: &Path) -> Result<(), String> {
    let dirs = [
        "app/models",
        "app/controllers",
        "app/views",
        "tests",
        "tests/models",
        "tests/controllers",
        "config",
        "db/migrations",
    ];

    for dir in dirs {
        let path = app_path.join(dir);
        if !path.exists() {
            fs::create_dir_all(&path)
                .map_err(|e| format!("Failed to create directory '{}': {}", path.display(), e))?;
        }
    }

    Ok(())
}

fn create_model(app_path: &Path, name: &str, fields: &[FieldDefinition]) -> Result<(), String> {
    let model_name = to_pascal_case(name);
    let collection_name = to_snake_case_plural(name);

    let validations = fields
        .iter()
        .filter(|f| {
            matches!(
                f.field_type.as_str(),
                "string" | "text" | "email" | "password" | "url"
            )
        })
        .map(|f| {
            format!(
                "validates(\"{}\", {{ \"presence\": true }})",
                f.to_snake_case()
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    let field_comments = fields
        .iter()
        .map(|f| format!("        // {} ({})", f.to_snake_case(), f.field_type))
        .collect::<Vec<_>>()
        .join("\n");

    let content = format!(
        r#"// {model_name} model - auto-generated scaffold
// Collection: {collection_name}

class {model_name} extends Model {{
    static {{
        // Fields
{field_comments}

        // Validations
{validations}
    }}

    // Callbacks
    before_save("normalize_fields")
}}
"#,
        model_name = model_name,
        collection_name = collection_name,
        field_comments = if field_comments.is_empty() {
            "        // (no additional fields)".to_string()
        } else {
            field_comments
        },
        validations = if validations.is_empty() {
            "        // (no validations defined)".to_string()
        } else {
            format!("        {}", validations.replace("\n", "\n        "))
        }
    );

    let model_path = app_path
        .join("app/models")
        .join(format!("{}_model.soli", to_snake_case(name)));
    write_file(&model_path, &content)?;
    Ok(())
}

fn create_controller(app_path: &Path, name: &str) -> Result<(), String> {
    let controller_name = to_pascal_case(name) + "Controller";
    let resource_name = to_snake_case_plural(name);
    let model_name = to_pascal_case(name);

    let content = format!(
        r#"// {} controller - auto-generated scaffold

class {controller_name} extends Controller {{
    static {{
        this.layout = "application";
    }}

    // GET /{resource}
    fn index(req: Any) -> Any {{
        let {model_var}s = {model_name}.all();
        return render("{resource}/index", {{
            "{model_var}s": {model_var}s,
            "title": "{controller_name}"
        }});
    }}

    // GET /{resource}/:id
    fn show(req: Any) -> Any {{
        let id = req.params["id"];
        let {model_var} = {model_name}.find(id);
        if {model_var} == null {{
            return error(404, "{model_name} not found");
        }}
        return render("{resource}/show", {{
            "{model_var}": {model_var},
            "title": "View {model_name}"
        }});
    }}

    // GET /{resource}/new
    fn new(req: Any) -> Any {{
        return render("{resource}/new", {{
            "{model_var}": {{}},
            "title": "New {model_name}"
        }});
    }}

    // GET /{resource}/:id/edit
    fn edit(req: Any) -> Any {{
        let id = req.params["id"];
        let {model_var} = {model_name}.find(id);
        if {model_var} == null {{
            return error(404, "{model_name} not found");
        }}
        return render("{resource}/edit", {{
            "{model_var}": {model_var},
            "title": "Edit {model_name}"
        }});
    }}

    // POST /{resource}
    fn create(req: Any) -> Any {{
        let result = {model_name}.create(req.params);
        if result["valid"] == true {{
            return redirect("/{resource}");
        }}
        return render("{resource}/new", {{
            "{model_var}": result,
            "title": "New {model_name}"
        }});
    }}

    // PATCH/PUT /{resource}/:id
    fn update(req: Any) -> Any {{
        let id = req.params["id"];
        {model_name}.update(id, req.params);
        return redirect("/{resource}");
    }}

    // DELETE /{resource}/:id
    fn delete(req: Any) -> Any {{
        let id = req.params["id"];
        {model_name}.delete(id);
        return redirect("/{resource}");
    }}
}}
"#,
        controller_name = controller_name,
        resource = resource_name,
        model_name = model_name,
        model_var = to_singular(name)
    );

    let controller_path = app_path
        .join("app/controllers")
        .join(format!("{}_controller.soli", to_snake_case(name)));
    write_file(&controller_path, &content)?;
    Ok(())
}

fn create_views(app_path: &Path, name: &str, fields: &[FieldDefinition]) -> Result<(), String> {
    let resource_name = to_snake_case_plural(name);
    let model_var = to_snake_case(name);

    // Create view directory
    let view_dir = app_path.join("app/views").join(&resource_name);
    fs::create_dir_all(&view_dir)
        .map_err(|e| format!("Failed to create directory '{}': {}", view_dir.display(), e))?;

    // Create index view
    create_resource_index_view(&view_dir, &resource_name, &model_var, fields)?;

    // Create show view
    create_show_view(&view_dir, &resource_name, &model_var, fields)?;

    // Create new view
    create_form_view(&view_dir, &resource_name, &model_var, "new")?;

    // Create edit view
    create_form_view(&view_dir, &resource_name, &model_var, "edit")?;

    Ok(())
}

fn create_resource_index_view(
    view_dir: &Path,
    resource_name: &str,
    model_var: &str,
    fields: &[FieldDefinition],
) -> Result<(), String> {
    let title = to_title_case(resource_name);
    let model_title = to_title_case(model_var);

    let table_headers = fields
        .iter()
        .map(|f| {
            format!(
                r#"                    <th class="px-6 py-3 text-left text-xs font-medium text-slate-300 uppercase tracking-wider">{}</th>"#,
                f.to_title_case()
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    let table_cells = fields
        .iter()
        .map(|f| {
            format!(
                r#"                    <td class="px-6 py-4 whitespace-nowrap text-white"><%= {model_var}["{field_name}"] %></td>"#,
                model_var = model_var,
                field_name = f.to_snake_case()
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    let content = format!(
        r#"<div class="p-6">
    <div class="flex justify-between items-center mb-6">
        <h1 class="text-2xl font-bold">{title}</h1>
        <a href="/{resource}/new" class="bg-indigo-600 hover:bg-indigo-700 text-white px-4 py-2 rounded-lg transition-colors">
            New {model_title}
        </a>
    </div>

    <div class="bg-slate-800 rounded-xl overflow-hidden">
        <table class="w-full">
            <thead class="bg-slate-700">
                <tr>
                    <th class="px-6 py-3 text-left text-xs font-medium text-slate-300 uppercase tracking-wider">ID</th>
{table_headers}
                    <th class="px-6 py-3 text-left text-xs font-medium text-slate-300 uppercase tracking-wider">Actions</th>
                </tr>
            </thead>
            <tbody class="divide-y divide-slate-700">
                <% if {model_var}s.empty? %>
                <tr>
                    <td colspan="{colspan}" class="px-6 py-8 text-center text-slate-400">
                        No {resource} found. <a href="/{resource}/new" class="text-indigo-400 hover:text-indigo-300">Create one?</a>
                    </td>
                </tr>
                <% end %>
                <% {model_var}s.each(fn({model_var}) %>
                <tr class="hover:bg-slate-700/50 transition-colors">
                    <td class="px-6 py-4 whitespace-nowrap text-slate-300"><%= {model_var}["id"] %></td>
{table_cells}
                    <td class="px-6 py-4 whitespace-nowrap">
                        <div class="flex gap-2">
                            <a href="/{resource}/<%= {model_var}["id"] %>" class="text-indigo-400 hover:text-indigo-300">Show</a>
                            <a href="/{resource}/<%= {model_var}["id"] %>/edit" class="text-yellow-400 hover:text-yellow-300">Edit</a>
                            <form action="/{resource}/<%= {model_var}["id"] %>" method="POST" class="inline">
                                <input type="hidden" name="_method" value="DELETE">
                                <button type="submit" class="text-red-400 hover:text-red-300" onclick="return confirm('Are you sure?')">Delete</button>
                            </form>
                        </div>
                    </td>
                </tr>
                <% end %>
            </tbody>
        </table>
    </div>
</div>
"#,
        title = title,
        resource = resource_name,
        model_title = model_title,
        model_var = model_var,
        table_headers = if table_headers.is_empty() {
            "".to_string()
        } else {
            table_headers
        },
        table_cells = if table_cells.is_empty() {
            "".to_string()
        } else {
            table_cells
        },
        colspan = 2 + fields.len()
    );

    write_file(&view_dir.join("index.html.erb"), &content)?;
    println!("  Created: {}/index.html.erb", view_dir.display());
    Ok(())
}

fn create_show_view(
    view_dir: &Path,
    resource_name: &str,
    model_var: &str,
    fields: &[FieldDefinition],
) -> Result<(), String> {
    let resource_title = to_title_case(resource_name);
    let model_title = to_title_case(model_var);

    let detail_rows = fields
        .iter()
        .map(|f| {
            format!(
                r#"                <div>
                    <dt class="text-sm font-medium text-slate-400">{field_title}</dt>
                    <dd class="mt-1 text-sm text-white"><%= {model_var}["{field_name}"] %></dd>
                </div>"#,
                model_var = model_var,
                field_title = f.to_title_case(),
                field_name = f.to_snake_case()
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    let content = format!(
        r#"<div class="p-6">
    <div class="mb-6">
        <a href="/{resource}" class="text-indigo-400 hover:text-indigo-300">&larr; Back to {resource_title}</a>
    </div>

    <div class="bg-slate-800 rounded-xl overflow-hidden">
        <div class="px-6 py-4 border-b border-slate-700 flex justify-between items-center">
            <h1 class="text-xl font-bold">{model_title} Details</h1>
            <div class="flex gap-2">
                <a href="/{resource}/<%= {model_var}["id"] %>/edit" class="bg-yellow-600 hover:bg-yellow-700 text-white px-3 py-1 rounded transition-colors">Edit</a>
                <form action="/{resource}/<%= {model_var}["id"] %>" method="POST" class="inline">
                    <input type="hidden" name="_method" value="DELETE">
                    <button type="submit" class="bg-red-600 hover:bg-red-700 text-white px-3 py-1 rounded transition-colors" onclick="return confirm('Are you sure?')">Delete</button>
                </form>
            </div>
        </div>
        <div class="p-6">
            <dl class="grid grid-cols-1 gap-x-4 gap-y-6 sm:grid-cols-2">
                <div>
                    <dt class="text-sm font-medium text-slate-400">ID</dt>
                    <dd class="mt-1 text-sm text-white"><%= {model_var}["id"] %></dd>
                </div>
{detail_rows}
            </dl>
        </div>
    </div>
</div>
"#,
        resource = resource_name,
        resource_title = resource_title,
        model_var = model_var,
        model_title = model_title,
        detail_rows = if detail_rows.is_empty() {
            "".to_string()
        } else {
            detail_rows
        }
    );

    write_file(&view_dir.join("show.html.erb"), &content)?;
    println!("  Created: {}/show.html.erb", view_dir.display());
    Ok(())
}

fn create_form_view(
    view_dir: &Path,
    resource_name: &str,
    model_var: &str,
    action: &str,
) -> Result<(), String> {
    let title = if action == "new" {
        format!("New {}", to_title_case(model_var))
    } else {
        format!("Edit {}", to_title_case(model_var))
    };

    let submit_text = if action == "new" { "Create" } else { "Update" };
    let form_action = if action == "new" {
        format!("/{}", resource_name)
    } else {
        format!("/{}/<%= {}[\"id\"] %>", resource_name, model_var)
    };
    let method = if action == "new" { "POST" } else { "PUT" };

    let content = format!(
        r#"<div class="p-6">
    <div class="mb-6">
        <a href="/{resource}" class="text-indigo-400 hover:text-indigo-300">&larr; Back to {resource_title}</a>
    </div>

    <div class="max-w-2xl">
        <h1 class="text-2xl font-bold mb-6">{title}</h1>

        <form action="{form_action}" method="POST" class="space-y-6">
            <input type="hidden" name="_method" value="{method}">
            <%= render("{resource}/_form", {{ "{model_var}": {model_var} }}) %>
        </form>
    </div>
</div>
"#,
        resource = resource_name,
        resource_title = to_title_case(resource_name),
        model_var = model_var,
        title = title,
        form_action = form_action,
        method = method
    );

    let filename = format!("{}.html.erb", action);
    write_file(&view_dir.join(&filename), &content)?;
    println!("  Created: {}/{}", view_dir.display(), filename);
    Ok(())
}

fn create_form_partial(
    app_path: &Path,
    name: &str,
    fields: &[FieldDefinition],
) -> Result<(), String> {
    let resource_name = to_snake_case_plural(name);
    let model_var = to_snake_case(name);
    let model_title = to_title_case(&model_var);

    let view_dir = app_path.join("app/views").join(&resource_name);

    let field_inputs = fields
        .iter()
        .map(|f| {
            let label = f.to_title_case();
            let field_name = f.to_snake_case();
            let input_type = match f.field_type.as_str() {
                "email" => "email",
                "password" => "password",
                "text" | "string" | "url" => "text",
                "number" | "integer" | "float" => "number",
                "boolean" | "bool" => "checkbox",
                "date" => "date",
                "datetime" => "datetime-local",
                _ => "text",
            };
            let placeholder = format!("Enter {}", label.to_ascii_lowercase());

            if input_type == "checkbox" {
                format!(
                    r#"            <div class="flex items-center">
                <input type="checkbox" id="{field_name}" name="{field_name}" value="true"
                    class="h-4 w-4 text-indigo-600 focus:ring-indigo-500 border-slate-600 rounded bg-slate-700"
                    <% if {model_var}["{field_name}"] == true %>checked<% end %>>
                <label for="{field_name}" class="ml-2 block text-sm text-slate-300">{label}</label>
            </div>"#
                )
            } else {
                format!(
                    r#"            <div>
                <label for="{field_name}" class="block text-sm font-medium text-slate-300 mb-2">{label}</label>
                <input type="{input_type}" id="{field_name}" name="{field_name}" value="<%= {model_var}["{field_name}"] %>"
                    class="w-full px-4 py-2 bg-slate-700 border border-slate-600 rounded-lg text-white focus:outline-none focus:ring-2 focus:ring-indigo-500 focus:border-transparent"
                    placeholder="{placeholder}">
            </div>"#,
                    field_name = field_name,
                    input_type = input_type,
                    label = label,
                    placeholder = placeholder,
                    model_var = model_var
                )
            }
        })
        .collect::<Vec<_>>()
        .join("\n");

    let content = format!(
        r#"<% if {model_var}["valid"] == false %>
<div class="bg-red-500/10 border border-red-500/20 rounded-lg p-4 mb-6">
    <h3 class="text-red-400 font-medium mb-2">Errors:</h3>
    <ul class="list-disc list-inside text-red-300 text-sm">
        <% {model_var}["errors"].each(fn(error)) %>
        <li><%= error["message"] %></li>
        <% end %>
    </ul>
</div>
<% end %>

{field_inputs}

<div class="flex gap-4">
    <button type="submit" class="bg-indigo-600 hover:bg-indigo-700 text-white px-6 py-2 rounded-lg transition-colors">
        Submit {model_title}
    </button>
    <a href="/{resource}" class="bg-slate-600 hover:bg-slate-700 text-white px-6 py-2 rounded-lg transition-colors text-center">
        Cancel
    </a>
</div>
"#,
        model_var = model_var,
        resource = resource_name,
        model_title = model_title,
        field_inputs = if field_inputs.is_empty() {
            r#"            <div>
                <label for="name" class="block text-sm font-medium text-slate-300 mb-2">Name</label>
                <input type="text" id="name" name="name" value="<%= {model_var}["name"] %>"
                    class="w-full px-4 py-2 bg-slate-700 border border-slate-600 rounded-lg text-white focus:outline-none focus:ring-2 focus:ring-indigo-500 focus:border-transparent"
                    placeholder="Enter name">
            </div>"#.replace("{model_var}", &model_var)
        } else {
            field_inputs
        }
    );

    let partial_path = view_dir.join("_form.html.erb");
    write_file(&partial_path, &content)?;
    println!("  Created: {}/_form.html.erb", view_dir.display());

    Ok(())
}

fn create_tests(app_path: &Path, name: &str) -> Result<(), String> {
    let snake_name = to_snake_case(name);
    let model_name = to_pascal_case(name);
    let collection_name = to_snake_case_plural(name);

    // Create tests directory structure
    let tests_dir = app_path.join("tests");
    let controllers_dir = tests_dir.join("controllers");
    let models_dir = tests_dir.join("models");

    if !controllers_dir.exists() {
        fs::create_dir_all(&controllers_dir).map_err(|e| {
            format!(
                "Failed to create directory '{}': {}",
                controllers_dir.display(),
                e
            )
        })?;
    }
    if !models_dir.exists() {
        fs::create_dir_all(&models_dir).map_err(|e| {
            format!(
                "Failed to create directory '{}': {}",
                models_dir.display(),
                e
            )
        })?;
    }

    // Create model test
    let model_test_content = format!(
        r#"// {} model tests - auto-generated scaffold

describe("{}Model", fn() {{
    before_each(fn() {{
        // Setup code - runs before each test
    }})

    after_each(fn() {{
        // Cleanup code - runs after each test
    }})

    test("should have correct collection name", fn() {{
        // The model should derive collection name from class name
        assert_true(true, "Collection name should be derived correctly")
    }})

    test("should create valid record", fn() {{
        let data = {{
            "name": "Test {}"
        }};
        let result = {model_name}.create(data);
        assert_true(result["valid"], "Create should return valid: true");
        assert_not_null(result["record"], "Create should return a record");
    }})

    test("should find record by id", fn() {{
        let result = {model_name}.find("test-id");
        assert_true(true, "Find should work without errors");
    }})

    test("should return all records", fn() {{
        let results = {model_name}.all();
        assert_true(true, "All should return array of records");
    }})

    test("should validate presence of name", fn() {{
        let data = {{}};
        let result = {model_name}.create(data);
        assert_false(result["valid"], "Create should fail without name");
    }})
}})
"#,
        model_name,
        model_name = model_name,
        collection_name = collection_name
    );

    let model_test_path = models_dir.join(format!("{}_test.soli", snake_name));
    write_file(&model_test_path, &model_test_content)?;
    println!("  Created: {}", model_test_path.display());

    // Create controller test
    let controller_test_content = format!(
        r#"// {}Controller tests - auto-generated scaffold

describe("{}Controller", fn() {{
    before_each(fn() {{
        // Setup code - runs before each test
    }})

    after_each(fn() {{
        // Cleanup code - runs after each test
    }})

    test("index action should return list", fn() {{
        let req = {{
            "params": {{}},
            "session": {{}}
        }};
        // Controller action would be tested here
        assert_true(true, "Index action should render view");
    }})

    test("show action should return single record", fn() {{
        let req = {{
            "params": {{ "id": "test-id" }},
            "session": {{}}
        }};
        assert_true(true, "Show action should render view with record");
    }})

    test("new action should render new form", fn() {{
        let req = {{
            "params": {{}},
            "session": {{}}
        }};
        assert_true(true, "New action should render form");
    }})

    test("edit action should render edit form", fn() {{
        let req = {{
            "params": {{ "id": "test-id" }},
            "session": {{}}
        }};
        assert_true(true, "Edit action should render form with record");
    }})

    test("create action should redirect on success", fn() {{
        let req = {{
            "params": {{ "name": "Test {}" }},
            "session": {{}}
        }};
        assert_true(true, "Create action should redirect");
    }})

    test("update action should redirect on success", fn() {{
        let req = {{
            "params": {{ "id": "test-id", "name": "Updated" }},
            "session": {{}}
        }};
        assert_true(true, "Update action should redirect");
    }})

    test("delete action should redirect", fn() {{
        let req = {{
            "params": {{ "id": "test-id" }},
            "session": {{}}
        }};
        assert_true(true, "Delete action should redirect");
    }})

    test("should have correct routes defined", fn() {{
        assert_true(true, "Routes should be defined in config/routes.soli");
    }})
}})
"#,
        controller_name = to_pascal_case(name) + "Controller",
        model_name = model_name,
        collection_name = collection_name
    );

    let controller_test_path = controllers_dir.join(format!("{}_controller_test.soli", snake_name));
    write_file(&controller_test_path, &controller_test_content)?;
    println!("  Created: {}", controller_test_path.display());

    Ok(())
}

fn create_migration(app_path: &Path, name: &str, fields: &[FieldDefinition]) -> Result<(), String> {
    let collection_name = to_snake_case_plural(name);
    let migration_name = format!("create_{}", collection_name);

    // Generate timestamp for migration filename
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| format!("Failed to get timestamp: {}", e))?
        .as_secs();

    let filename = format!("{}{}_{}.soli", timestamp, migration_name, timestamp);
    let migrations_dir = app_path.join("db/migrations");
    let migration_path = migrations_dir.join(&filename);

    // Create migrations directory if it doesn't exist
    fs::create_dir_all(&migrations_dir)
        .map_err(|e| format!("Failed to create migrations directory: {}", e))?;

    // Create indexes for unique fields
    let unique_indexes: Vec<String> = fields
        .iter()
        .filter(|f| matches!(f.field_type.as_str(), "email" | "password"))
        .map(|f| {
            format!(
                r#"    db.create_index("{collection}", "idx_{field_name}", ["{field_name}"], {{ "unique": true }});"#,
                collection = collection_name,
                field_name = f.to_snake_case()
            )
        })
        .collect();

    let content = format!(
        r#"// Migration: {migration_name}
// Generated by: soli generate scaffold {name}

fn up(db: Any) -> Any {{
    // Create collection for {model_name}
    db.create_collection("{collection}");

    // Create indexes
{indexes}
}}

fn down(db: Any) -> Any {{
    // Drop indexes
    <% db.list_indexes("{collection}").each(fn(idx) {{
        db.drop_index("{collection}", idx["name"]);
    }}) %>

    // Drop collection
    db.drop_collection("{collection}");
}}
"#,
        migration_name = migration_name,
        name = name,
        model_name = to_pascal_case(name),
        collection = collection_name,
        indexes = if unique_indexes.is_empty() {
            "    // No indexes defined".to_string()
        } else {
            unique_indexes.join("\n")
        }
    );

    write_file(&migration_path, &content)?;

    Ok(())
}

fn add_routes(app_path: &Path, name: &str) -> Result<(), String> {
    let resource_name = to_snake_case_plural(name);
    let routes_file = app_path.join("config/routes.soli");

    let new_routes = format!(
        r#"

// {name} resource routes
get("/{resource}", "{resource}#index")
get("/{resource}/new", "{resource}#new")
post("/{resource}", "{resource}#create")
get("/{resource}/:id", "{resource}#show")
get("/{resource}/:id/edit", "{resource}#edit")
put("/{resource}/:id", "{resource}#update")
delete("/{resource}/:id", "{resource}#delete")
"#,
        name = name,
        resource = resource_name
    );

    if routes_file.exists() {
        let mut content = std::fs::read_to_string(&routes_file)
            .map_err(|e| format!("Failed to read routes file: {}", e))?;
        content.push_str(&new_routes);
        std::fs::write(&routes_file, content)
            .map_err(|e| format!("Failed to write routes file: {}", e))?;
        println!("  Updated: {}/config/routes.soli", app_path.display());
    } else {
        write_file(&routes_file, &new_routes)?;
    }

    Ok(())
}

fn to_pascal_case(s: &str) -> String {
    let mut result = String::new();
    let mut capitalize_next = true;
    for c in s.chars() {
        if c == '_' || c == '-' {
            capitalize_next = true;
        } else if capitalize_next {
            result.push(c.to_ascii_uppercase());
            capitalize_next = false;
        } else {
            result.push(c);
        }
    }
    result
}

fn to_snake_case(s: &str) -> String {
    let mut result = String::new();
    for (i, c) in s.chars().enumerate() {
        if c.is_uppercase() {
            if i > 0 {
                result.push('_');
            }
            result.push(c.to_ascii_lowercase());
        } else {
            result.push(c);
        }
    }
    result
}

fn to_singular(s: &str) -> String {
    let snake = to_snake_case(s);
    // Remove trailing 's' if it exists
    if snake.ends_with('s') && snake.len() > 1 {
        snake[..snake.len() - 1].to_string()
    } else {
        snake
    }
}

fn to_snake_case_plural(s: &str) -> String {
    let snake = to_snake_case(s);
    // Don't add 's' if it already ends with 's' to avoid "userss"
    if snake.ends_with('s') {
        snake
    } else {
        snake + "s"
    }
}

fn to_title_case(s: &str) -> String {
    let snake = to_snake_case(s);
    snake
        .split('_')
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(c) => c.to_ascii_uppercase().to_string() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Print success message after creating a scaffold
pub fn print_scaffold_success_message(name: &str) {
    println!();
    println!(
        "  \x1b[32m\x1b[1mSuccess!\x1b[0m Created scaffold for \x1b[1m{}\x1b[0m",
        name
    );
    println!();
    println!("  \x1b[2mGenerated files:\x1b[0m");
    println!();
    println!(
        "    \x1b[36mapp/models/{}_model.soli\x1b[0m",
        to_snake_case(name)
    );
    println!(
        "    \x1b[36mapp/controllers/{}_controller.soli\x1b[0m",
        to_snake_case(name)
    );
    println!(
        "    \x1b[36mapp/views/{}/index.html.erb\x1b[0m",
        to_snake_case_plural(name)
    );
    println!(
        "    \x1b[36mapp/views/{}/show.html.erb\x1b[0m",
        to_snake_case_plural(name)
    );
    println!(
        "    \x1b[36mapp/views/{}/new.html.erb\x1b[0m",
        to_snake_case_plural(name)
    );
    println!(
        "    \x1b[36mapp/views/{}/edit.html.erb\x1b[0m",
        to_snake_case_plural(name)
    );
    println!(
        "    \x1b[36mapp/views/{}/_form.html.erb\x1b[0m",
        to_snake_case_plural(name)
    );
    println!();
    println!("  \x1b[2mTest files:\x1b[0m");
    println!();
    println!(
        "    \x1b[36mtests/models/{}_test.soli\x1b[0m",
        to_snake_case(name)
    );
    println!(
        "    \x1b[36mtests/controllers/{}_controller_test.soli\x1b[0m",
        to_snake_case(name)
    );
    println!();
    println!("  \x1b[2mRoutes added to:\x1b[0m \x1b[36mconfig/routes.soli\x1b[0m");
    println!();
}
