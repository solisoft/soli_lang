//! Application template strings

/// Routes configuration template
pub const ROUTES_TEMPLATE: &str = r#"// Routes configuration
// Define your application routes here

// Home page
get("/", "home#index");

// Health check endpoint
get("/health", "home#health");

print("Routes loaded!");
"#;

/// Home controller template
pub const HOME_CONTROLLER_TEMPLATE: &str = r#"// Home controller - handles the root routes

class HomeController extends Controller {
    // GET /
    fn index(req) {
        return render("home/index", {
            "title": "Welcome"
        });
    }

    // GET /health
    fn health(req) {
        return {
            "status": 200,
            "headers": {"Content-Type": "application/json"},
            "body": "{\"status\":\"ok\"}"
        };
    }
}
"#;

/// Application layout template
pub const LAYOUT_TEMPLATE: &str = r##"<!DOCTYPE html>
<html lang="en" class="h-full">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title><%= title %> - Soli App</title>

    <!-- Tailwind CSS (compiled) -->
    <link rel="stylesheet" href="<%= public_path("css/output.css") %>">

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

/// Home index view template
pub const INDEX_VIEW_TEMPLATE: &str = r##"<div class="min-h-screen relative overflow-hidden">
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
                    <div class="text-emerald-400">vim config/routes.sl</div>
                    <div class="mt-3 text-slate-500"># Create a new controller</div>
                    <div class="text-emerald-400">vim app/controllers/posts_controller.sl</div>
                    <div class="mt-3 text-slate-500"># Create a view</div>
                    <div class="text-emerald-400">vim app/views/posts/index.html.slv</div>
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

/// CSS file template
pub const CSS_TEMPLATE: &str = r#"/* Tailwind CSS directives */
@tailwind base;
@tailwind components;
@tailwind utilities;

/* Custom application styles */

/* Custom animations */
@keyframes float {
    0%, 100% { transform: translateY(0); }
    50% { transform: translateY(-10px); }
}

.animate-float {
    animation: float 3s ease-in-out infinite;
}

/* Custom scrollbar */
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

/// Environment file template
pub const ENV_TEMPLATE: &str = r#"# Database Configuration
# These variables are used by soli db:migrate commands

SOLIDB_HOST=http://localhost:6745
SOLIDB_DATABASE=default
SOLIDB_USERNAME=admin
SOLIDB_PASSWORD=admin

# Application Settings
# APP_ENV=development
# APP_SECRET=your-secret-key-here
"#;

/// Gitignore template
pub const GITIGNORE_TEMPLATE: &str = r#"# Dependencies
node_modules/

# Build artifacts
/target/
*.o
*.so

# Process files
*.pid

# Logs
*.log
logs/

# Environment
.env
.env.local
.env.*.local

# IDE
.idea/
.vscode/
*.swp
*.swo
*~

# OS
.DS_Store
Thumbs.db

# Coverage
coverage/
.coverage
"#;

/// Application helper template
pub const APPLICATION_HELPER_TEMPLATE: &str = r#"// Application-wide view helpers

// Truncate text to a maximum length with ellipsis
fn truncate_text(text: String, length: Int, suffix: String) -> String {
    if len(text) <= length {
        return text;
    }
    return substring(text, 0, length - len(suffix)) + suffix;
}

// Capitalize first letter of a string
fn capitalize(text: String) -> String {
    if len(text) == 0 {
        return text;
    }
    return upcase(substring(text, 0, 1)) + substring(text, 1, len(text));
}

// Generate an HTML link
fn link_to(text: String, url: String) -> String {
    return "<a href=\"" + html_escape(url) + "\">" + html_escape(text) + "</a>";
}

// Generate an HTML link with CSS class
fn link_to_class(text: String, url: String, css_class: String) -> String {
    return "<a href=\"" + html_escape(url) + "\" class=\"" + html_escape(css_class) + "\">" + html_escape(text) + "</a>";
}

// Pluralize a word based on count
fn pluralize(count: Int, singular: String, plural: String) -> String {
    if count == 1 {
        return str(count) + " " + singular;
    }
    return str(count) + " " + plural;
}

// Simple pluralize (adds 's')
fn pluralize_simple(count: Int, word: String) -> String {
    if count == 1 {
        return str(count) + " " + word;
    }
    return str(count) + " " + word + "s";
}
"#;

/// CORS middleware template
pub const CORS_MIDDLEWARE_TEMPLATE: &str = r#"// ============================================================================
// CORS Middleware (Global)
// ============================================================================
//
// This middleware adds CORS headers to all responses.
// It runs for ALL requests automatically.
//
// Configuration:
// - `// order: N` - Execution order (lower runs first)
// - `// global_only: true` - Runs for all requests, cannot be scoped
//
// ============================================================================

// order: 5
// global_only: true

fn add_cors_headers(req: Any) -> Any {
    // Add CORS headers to the request context
    // These will be included in the response
    return {
        "continue": true,
        "request": req
    };
}
"#;

/// Auth middleware template
pub const AUTH_MIDDLEWARE_TEMPLATE: &str = r#"// ============================================================================
// Authentication Middleware (Scope-Only)
// ============================================================================
//
// This middleware checks for authentication.
// It only runs when explicitly scoped to routes.
//
// Usage in routes.sl:
//   middleware("authenticate", -> {
//       get("/admin", "admin#index");
//       get("/admin/settings", "admin#settings");
//   });
//
// Configuration:
// - `// order: N` - Execution order (lower runs first)
// - `// scope_only: true` - Only runs when explicitly scoped
//
// ============================================================================

// order: 20
// scope_only: true

fn authenticate(req: Any) -> Any {
    let headers = req["headers"];

    // Example: Check for API key in header
    let api_key = "";
    if has_key(headers, "X-Api-Key") {
        api_key = headers["X-Api-Key"];
    } else if has_key(headers, "x-api-key") {
        api_key = headers["x-api-key"];
    }

    // TODO: Replace with your authentication logic
    // For example, verify JWT token, check session, etc.
    if api_key == "" {
        return {
            "continue": false,
            "response": {
                "status": 401,
                "headers": {"Content-Type": "application/json"},
                "body": json_stringify({
                    "error": "Unauthorized",
                    "message": "Authentication required"
                })
            }
        };
    }

    // Authentication passed, continue to handler
    return {
        "continue": true,
        "request": req
    };
}
"#;

/// Tailwind config template
pub const TAILWIND_CONFIG_TEMPLATE: &str = r#"/** @type {import('tailwindcss').Config} */
module.exports = {
  content: [
    "./app/views/**/*.{html,erb}",
    "./public/js/**/*.js",
  ],
  theme: {
    extend: {
      fontFamily: {
        sans: ['Inter', 'system-ui', 'sans-serif'],
        mono: ['JetBrains Mono', 'monospace'],
      },
    },
  },
  plugins: [],
}
"#;

/// State machine stdlib template
pub const STATE_MACHINE_TEMPLATE: &str = r#"
export class StateMachine {
    states: Array;
    transitions: Array;
    _current_state: String;
    _history: Array;
    _last_transition: Hash;
    valid_events: Hash;
    context: Hash;

    new(initial_state: String, states: Array, transitions: Array) {
        this.states = states;
        this.transitions = transitions;
        this._current_state = initial_state;
        this._history = [];
        this._last_transition = null;
        this.context = {};
        
        this.valid_events = {};
        let t_idx = 0;
        while t_idx < len(transitions) {
            let transition = transitions[t_idx];
            let event = transition["event"];
            let sources = transition["from"];
            if type(sources) == "String" {
                sources = [sources];
            }
            if !has_key(this.valid_events, event) {
                this.valid_events[event] = sources;
            } else {
                let current = this.valid_events[event];
                let new_sources = [];
                let s_idx = 0;
                while s_idx < len(sources) {
                    let s = sources[s_idx];
                    let existing = current.find(fn(x) x == s);
                    if existing == null {
                        new_sources = [...new_sources, s];
                    }
                    s_idx = s_idx + 1;
                }
                if len(new_sources) > 0 {
                    this.valid_events[event] = [...current, ...new_sources];
                }
            }
            t_idx = t_idx + 1;
        }
    }

    fn current_state() -> String {
        return this._current_state;
    }

    fn is(state: String) -> Bool {
        return this._current_state == state;
    }

    fn is_in(states: Array) -> Bool {
        return states.find(fn(x) x == this._current_state) != null;
    }

    fn can(event: String) -> Bool {
        if !has_key(this.valid_events, event) {
            return false;
        }
        return this.valid_events[event].find(fn(x) x == this._current_state) != null;
    }

    fn available_events() -> Array {
        let all_events = keys(this.valid_events);
        let result = [];
        let e_idx = 0;
        while e_idx < len(all_events) {
            let event = all_events[e_idx];
            if this.valid_events[event].find(fn(x) x == this._current_state) != null {
                result = [...result, event];
            }
            e_idx = e_idx + 1;
        }
        return result;
    }

    fn transition(event: String) -> Hash {
        let idx = 0;
        while idx < len(this.transitions) {
            let transition = this.transitions[idx];
            if transition["event"] == event {
                let sources = transition["from"];
                let is_valid = false;
                if type(sources) == "String" {
                    if sources == this._current_state {
                        is_valid = true;
                    }
                } else {
                    if sources.find(fn(x) x == this._current_state) != null {
                        is_valid = true;
                    }
                }
                if is_valid {
                    let from_state = this._current_state;
                    let to_state = transition["to"];
                    this._current_state = to_state;
                    this._history = [...this._history, {
                        "from": from_state,
                        "to": to_state,
                        "event": event
                    }];
                    this._last_transition = {
                        "from": from_state,
                        "to": to_state,
                        "event": event
                    };
                    return {
                        "success": true,
                        "from": from_state,
                        "to": to_state,
                        "event": event
                    };
                }
            }
            idx = idx + 1;
        }
        
        return {
            "success": false,
            "error": "invalid_transition",
            "reason": "Cannot transition '" + event + "' from state '" + this._current_state + "'"
        };
    }

    fn set(key: String, value: Any) {
        this.context[key] = value;
    }

    fn get(key: String) -> Any {
        if has_key(this.context, key) {
            return this.context[key];
        }
        return null;
    }

    fn history() -> Array {
        return this._history;
    }

    fn last_transition() -> Hash {
        return this._last_transition;
    }
}

export fn create_state_machine(initial_state: String, states: Array, transitions: Array) -> StateMachine {
    return new StateMachine(initial_state, states, transitions);
}

export class StateMachineBuilder {
    initial_state: String;
    states: Array;
    transitions: Array;

    new() {
        this.initial_state = "";
        this.states = [];
        this.transitions = [];
    }

    fn initial(state: String) -> StateMachineBuilder {
        this.initial_state = state;
        return this;
    }

    fn states_list(states: Array) -> StateMachineBuilder {
        this.states = states;
        return this;
    }

    fn transition(event: String, from_state: Any, to: String) -> StateMachineBuilder {
        let sources = from_state;
        if type(from_state) == "String" {
            sources = [from_state];
        }
        this.transitions = [...this.transitions, {
            "event": event,
            "from": sources,
            "to": to
        }];
        return this;
    }

    fn build() -> StateMachine {
        return create_state_machine(this.initial_state, this.states, this.transitions);
    }
}

export fn state_machine() -> StateMachineBuilder {
    return new StateMachineBuilder();
}
"#;

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
