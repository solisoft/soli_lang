//! Scaffold module for generating new Soli MVC applications.
//!
//! Provides functionality for `soli new app_name` command.

use std::fs::{self, File};
use std::io::Write;
use std::path::Path;

/// Create a new Soli MVC application with the given name.
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
    println!("  \x1b[32m\x1b[1mSuccess!\x1b[0m Created \x1b[1m{}\x1b[0m", name);
    println!();
    println!("  \x1b[2mGet started:\x1b[0m");
    println!();
    println!("    \x1b[36mcd {}\x1b[0m", name);
    println!("    \x1b[36msoli serve . --dev\x1b[0m");
    println!();
    println!("  \x1b[2mThen open\x1b[0m \x1b[4mhttp://localhost:3000\x1b[0m");
    println!();
}
