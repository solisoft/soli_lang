use std::env;
use std::process;

pub const VERSION: &str = env!("CARGO_PKG_VERSION", "0.2.0");

pub enum Command {
    Run {
        file: String,
    },
    Eval {
        code: String,
    },
    Repl,
    New {
        name: String,
        template: Option<String>,
    },
    Generate {
        scaffold_name: String,
        fields: Vec<String>,
        folder: String,
    },
    /// `soli generate auth [folder]` — scaffold session-based authentication
    /// (User model + login/signup/logout) and a Pundit-style Policy layer.
    GenerateAuth {
        folder: String,
    },
    /// `soli generate mailer <Name> <action...> [folder]` — scaffold a mailer
    /// class (app/mailers/<name>_mailer.sl) and one HTML view per action.
    GenerateMailer {
        name: String,
        actions: Vec<String>,
        folder: String,
    },
    /// `soli generate component <name> [folder]` — scaffold a view component
    /// (app/views/components/<name>.html.slv).
    GenerateComponent {
        name: String,
        folder: String,
    },
    Serve {
        folder: String,
        port: u16,
        dev_mode: bool,
        workers: usize,
        daemonize: bool,
    },
    Test {
        paths: Vec<String>,
        /// `None` = user didn't pass `--jobs`; the runner picks a default
        /// once it knows whether the app needs a test server (apps with
        /// `app/controllers/` pay a per-worker subprocess-spawn cost that
        /// makes high parallelism a regression — see test_runner.rs).
        jobs: Option<usize>,
        /// Additional output format(s) beyond the console summary. Accepted
        /// via `--coverage=html`, `--coverage=json`, `--coverage=xml`.
        /// Empty = console only.
        coverage_formats: Vec<String>,
        coverage_min: Option<f64>,
        no_coverage: bool,
        /// List every uncovered executable line under the coverage summary.
        /// Opt-in via `--show-uncovered` — by default the report only shows
        /// the per-file percentages so the summary stays scannable.
        show_uncovered: bool,
        /// Fail any request spec whose response triggered an N+1 query
        /// pattern, without a per-test `assert_no_n_plus_one`. Opt-in via
        /// `--fail-on-n1`; a project-wide guard over the same detection the
        /// dev-bar N+1 badge uses.
        fail_on_n1: bool,
    },
    DbMigrate {
        action: DbMigrateAction,
        folder: String,
    },
    DbSeed {
        action: DbSeedAction,
        folder: String,
    },
    /// `soli db:indexes [folder]` — create any missing indexes declared with
    /// the class-body DSL (`index`, `vector_index`, `fulltext_index`,
    /// `geo_index`). Idempotent; the production counterpart of the dev-boot
    /// auto-sync.
    DbIndexes {
        folder: String,
    },
    /// `soli routes [folder]` — print the app's expanded route table
    /// (everything `config/routes.sl` + engines register) without starting
    /// the server.
    Routes {
        folder: String,
        /// Case-insensitive substring filter over method/path/handler/helper.
        grep: Option<String>,
        /// Emit a machine-readable JSON array instead of the table.
        json: bool,
    },
    /// `soli graph build [folder]` — extract a code graph (files, classes,
    /// methods, routes, views and their relationships) and store it in SolidB
    /// so agents can retrieve code by semantic search and graph traversal.
    Graph {
        folder: String,
        /// Skip embeddings + the vector index (structural graph only).
        no_embed: bool,
        /// Target database (default: SOLIDB_DATABASE).
        database: Option<String>,
        /// Print the graph as JSON instead of writing to SolidB.
        dry_run: bool,
        /// Force a full clean rebuild (drop + recreate) instead of the default
        /// incremental, hash-based, non-destructive sync.
        fresh: bool,
        /// Comma-separated file extensions to index (e.g. `rb,erb,slim`). When
        /// set (or a `.soligraph.toml` is present), the generic multi-language
        /// extractor is used instead of the Soli-app extractor.
        ext: Option<String>,
        /// Comma-separated path substrings to exclude.
        exclude: Option<String>,
        /// Path to the config file (default: `.soligraph.toml` in the folder).
        config: Option<String>,
    },
    /// `soli graph query "<question>" [folder]` — retrieve the code most
    /// relevant to a task (semantic seed + graph expansion), for agents.
    GraphQuery {
        question: String,
        folder: String,
        database: Option<String>,
        /// Number of seed results (default 6).
        limit: usize,
        /// Neighbour-expansion depth (default 1).
        hops: usize,
        /// Keep only results whose file starts with this path prefix
        /// (e.g. `api/` or `app/`). `None` = no path constraint.
        path: Option<String>,
        /// Comma-separated node kinds to keep (e.g. `method,controller`).
        kind: Option<String>,
        /// Emit JSON instead of the human-readable summary.
        json: bool,
    },
    Lint {
        paths: Vec<String>,
    },
    /// `soli check [paths...]` — static type-check without executing.
    Check {
        paths: Vec<String>,
    },
    Fmt {
        paths: Vec<String>,
        /// Don't rewrite — exit non-zero if any file isn't already formatted.
        check: bool,
        /// Read source from stdin, write formatted output to stdout.
        stdin: bool,
    },
    Init,
    Add {
        name: String,
        git: Option<String>,
        path: Option<String>,
        tag: Option<String>,
        branch: Option<String>,
        rev: Option<String>,
        version: Option<String>,
    },
    Remove {
        name: String,
    },
    Install,
    SelfUpdate,
    Update {
        name: Option<String>,
    },
    Login {
        registry: Option<String>,
        token: Option<String>,
    },
    Publish {
        registry: Option<String>,
    },
    Deploy {
        folder: Option<String>,
    },
    Engine {
        action: EngineAction,
    },
    /// Start the Soli LSP server on stdio. Used by editor plugins
    /// (Nova, VS Code, etc.) — not typically run interactively.
    Lsp,
    Build {
        folder: String,
        output: Option<String>,
        standalone: bool,
        /// Encrypt the bundle (AES-256-GCM, key from SOLI_BUNDLE_KEY or the
        /// key server at SOLI_BUNDLE_AUTH_URL).
        encrypt: bool,
        /// Replace `.sl` sources with serialized binary ASTs (implies
        /// `encrypt`) so no readable source ships in the bundle.
        protect: bool,
        /// Platform to embed for `--standalone` (release artifact name:
        /// linux-amd64, linux-arm64, darwin-amd64, darwin-arm64). None = host platform.
        target: Option<String>,
    },
    /// Package an app as a self-contained desktop application.
    DesktopBuild {
        folder: String,
        app_id: String,
        app_name: Option<String>,
        output: Option<String>,
        db_binary: String,
        seed: Option<String>,
        protect: bool,
        target: Option<String>,
    },
}

pub enum EngineAction {
    Create { name: String },
    DbMigrate { engine_name: Option<String> },
    DbRollback { engine_name: Option<String> },
}

pub enum DbMigrateAction {
    Up,
    Down,
    Status,
    Generate { name: String },
}

pub enum DbSeedAction {
    /// Run the project's seed scripts. When `file` is `Some`, run only that
    /// single seed file (resolved relative to the project folder) instead of
    /// the default `db/seeds.sl` then `db/seeds/*.sl` discovery.
    Run { file: Option<String> },
    /// Scaffold a new timestamped seed file under `db/seeds/`.
    Generate { name: String },
}

pub struct Options {
    pub command: Command,
    pub no_type_check: bool,
    pub use_vm: bool,
}

pub fn print_usage() {
    eprintln!("Soli {} - Solilang Interpreter", VERSION);
    eprintln!();
    eprintln!("Usage: soli [options] [script.sl]");
    eprintln!("       soli new <app_name>");
    eprintln!("       soli init");
    eprintln!("       soli add <name> --git <url> [--tag TAG] [--branch BRANCH] [--rev REV]");
    eprintln!("       soli add <name> --path <path>");
    eprintln!("       soli add <name> --version <version>");
    eprintln!("       soli remove <name>");
    eprintln!("       soli install");
    eprintln!("       soli update [name]");
    eprintln!("       soli login [--registry URL] [--token TOKEN]");
    eprintln!("       soli publish [--registry URL]");
    eprintln!("       soli generate scaffold <name> [fields...] [folder]");
    eprintln!("       soli generate auth [folder]");
    eprintln!("       soli generate component <name> [folder]");
    eprintln!("       soli serve <folder> [-d] [--dev] [--port PORT] [--workers N]");
    eprintln!("       soli test [paths...] [--jobs N] [--coverage] [--coverage=FORMAT] [--coverage-min N] [--show-uncovered] [--no-coverage] [--fail-on-n1]");
    eprintln!("       soli lint [paths...]");
    eprintln!("       soli check [paths...]");
    eprintln!("       soli lsp");
    eprintln!("  soli build <folder> [-o <file>] [--encrypt] [--protect] [--standalone] [--target PLATFORM]");
    eprintln!("  soli deploy [--folder <path>]");
    eprintln!("  soli db:migrate <up|down|status> [folder]");
    eprintln!("  soli db:migrate generate <name> [folder]");
    eprintln!("  soli db:seed [folder] [file.sl]");
    eprintln!("  soli db:seed generate <name> [folder]");
    eprintln!("  soli db:indexes [folder]");
    eprintln!("  soli routes [folder] [-g PATTERN] [--json]");
    eprintln!("  soli graph build [folder] [--no-embed] [--database NAME] [--dry-run] [--fresh]");
    eprintln!("  soli graph query \"<question>\" [folder] [--json] [--limit N] [--hops N] [--path PREFIX] [--kind KINDS]");
    eprintln!();
    eprintln!("Commands:");
    eprintln!("  new <app_name>       Create a new Soli MVC application");
    eprintln!("  new <app_name> --template <url>  Create from custom template URL");
    eprintln!("  init                 Create soli.toml in current directory");
    eprintln!("  add <name> --git <url>  Add a git dependency");
    eprintln!("  add <name> --path <path>  Add a local path dependency");
    eprintln!("  add <name> --version <ver>  Add a registry dependency");
    eprintln!("  remove <name>        Remove a dependency");
    eprintln!("  login                Login to the package registry");
    eprintln!("  publish              Publish the current package to the registry");
    eprintln!("  install              Install all dependencies from soli.toml");
    eprintln!(
        "  update [name]      Update a dependency (soli update = self-update to latest release)"
    );
    eprintln!("  generate scaffold    Generate model, controller, and views for a resource");
    eprintln!("                       Fields: name:string email:email text:description");
    eprintln!("  generate auth        Scaffold session auth (User + login/signup) and policies");
    eprintln!(
        "  generate component   Scaffold a view component (app/views/components/<name>.html.slv)"
    );
    eprintln!("  build <folder>       Bundle app into a single .soli file");
    eprintln!("                       --output, -o <file>  Custom output path");
    eprintln!("                       --encrypt      Encrypt the bundle (AES-256-GCM; key from");
    eprintln!("                                      SOLI_BUNDLE_KEY or SOLI_BUNDLE_AUTH_URL)");
    eprintln!("                       --protect      Ship binary ASTs instead of .sl sources");
    eprintln!("                                      (implies --encrypt)");
    eprintln!("                       --standalone   Emit a self-contained executable (embeds the");
    eprintln!(
        "                                      soli runtime; composes with --encrypt/--protect)"
    );
    eprintln!("                       --target T     Platform for --standalone: linux-amd64,");
    eprintln!("                                      linux-arm64, darwin-amd64, darwin-arm64");
    eprintln!("                                      (default: this machine)");
    eprintln!("  serve <folder>       Start MVC server from a project folder");
    eprintln!("                       Supports .soli bundle files");
    eprintln!("  test [paths...]      Run tests (default: tests/ directory)");
    eprintln!("  lint [paths...]      Lint .sl files for style issues and code smells");
    eprintln!("  check [paths...]     Static type-check .sl files without running them");
    eprintln!("  lsp                  Start the Soli LSP server on stdio (for editor plugins)");
    eprintln!(
        "  fmt [paths...]       Format .sl files in place (--check to dry-run, --stdin to filter)"
    );
    eprintln!("  deploy [--folder <path>]  Deploy application to servers via deploy.toml");
    eprintln!("  db:migrate           Database migration commands");
    eprintln!("  db:seed              Run database seed scripts (db/seeds.sl, db/seeds/*.sl, or a given file)");
    eprintln!("  routes [folder]      Print the app's route table (-g PATTERN to filter, --json for tooling)");
    eprintln!("  graph build [folder] Build a code graph in SolidB for agents (graph RAG); --dry-run for JSON");
    eprintln!("  graph query <q>      Retrieve the code most relevant to a task (semantic + graph); --json for agents, --path PREFIX / --kind KINDS to filter");
    eprintln!("  engine               Engine commands (create, db:migrate, db:rollback)");
    eprintln!("  -e <code>            Evaluate code and print result");
    eprintln!();
    eprintln!("Options:");
    eprintln!("  --no-type-check Skip type checking");
    eprintln!("  -d              Daemonize server (creates soli.pid and soli.log)");
    eprintln!("  --dev           Enable development mode (hot reload, no caching)");
    eprintln!("  --port PORT     Port for serve command (default: 5011)");
    eprintln!("  --workers N     Number of worker threads (default: CPU cores)");
    eprintln!("  --jobs N        Number of parallel test workers (default: 3 for apps with app/controllers/, 1 otherwise)");
    eprintln!("  --coverage           Generate coverage report (console)");
    eprintln!("  --coverage=FORMAT    Also generate FORMAT reports: html, json, xml (comma-sep)");
    eprintln!("  --coverage-min N     Fail if coverage is below N% (default: 80)");
    eprintln!("  --show-uncovered     List every uncovered line in the console report");
    eprintln!("  --no-coverage        Skip coverage collection");
    eprintln!("  --help, -h      Show this help message");
    eprintln!();
    eprintln!("Examples:");
    eprintln!("  soli                          Start interactive REPL");
    eprintln!("  soli script.sl                Run a script file");
    eprintln!("  soli new my_app               Create a new MVC application");
    eprintln!("  soli new my_app --template https://github.com/user/template/archive/main.tar.gz  Create from custom template");
    eprintln!("  soli init                     Create soli.toml in current directory");
    eprintln!("  soli add math --git https://github.com/user/soli-math --tag v1.0.0");
    eprintln!("  soli add utils --path ../shared/utils");
    eprintln!("  soli remove math              Remove dependency");
    eprintln!("  soli install                  Install all dependencies");
    eprintln!("  soli update                    Update soli CLI to latest release");
    eprintln!("  soli update math               Update a specific dependency");
    eprintln!("  soli generate scaffold users  Generate users model, controller, views");
    eprintln!("  soli generate scaffold users name:string email:email  Generate with fields");
    eprintln!("  soli generate auth            Scaffold authentication + policy layer");
    eprintln!("  soli build my_app             Bundle app into my_app.soli");
    eprintln!("  soli build my_app -o release.soli  Custom bundle output path");
    eprintln!("  soli build my_app --standalone     Self-contained executable ./my_app");
    eprintln!("  soli build my_app --standalone --protect --target linux-arm64");
    eprintln!(
        "                                Cross-build ./my_app-linux-arm64 (encrypted, no source)"
    );
    eprintln!("  soli serve my_app.soli       Serve app from bundle (no source files needed)");
    eprintln!("  soli serve my_app             Start production server (no hot reload)");
    eprintln!("  soli serve my_app -d          Start as daemon (background process)");
    eprintln!("  soli serve my_app --dev       Start development server (with hot reload)");
    eprintln!("  soli serve my_app --port 8080 Start on custom port");
    eprintln!("  soli serve my_app --workers 16 Start server with 16 workers");
    eprintln!("  soli test                     Run all tests in tests/");
    eprintln!("  soli test spec.sl             Run specific test file");
    eprintln!("  soli test --coverage          Run tests with coverage");
    eprintln!("  soli test --jobs=4            Run tests with 4 workers");
    eprintln!("  soli test --fail-on-n1        Fail any request spec that triggers an N+1");
    eprintln!("  soli db:migrate up            Run pending migrations");
    eprintln!("  soli db:migrate down          Rollback last migration");
    eprintln!("  soli db:migrate status        Show migration status");
    eprintln!("  soli db:migrate generate create_users  Generate new migration");
    eprintln!("  soli db:seed                  Run db/seeds.sl and db/seeds/*.sl");
    eprintln!("  soli db:seed db/seeds/demo.sl  Run a single seed file");
    eprintln!("  soli db:seed generate demo_users  Generate new seed file");
    eprintln!("  soli routes                   Print the route table of the app in .");
    eprintln!("  soli routes -g posts          Only routes matching 'posts'");
    eprintln!("  soli engine create shop       Create a new engine named 'shop'");
    eprintln!("  soli engine db:migrate        Run all engine migrations");
    eprintln!("  soli engine db:migrate shop   Run migrations for 'shop' engine only");
    eprintln!("  soli engine db:rollback shop  Rollback last migration of 'shop'");
    eprintln!("  soli -e 'print(1 + 1)'        Evaluate code directly");
}

pub fn parse_args() -> Options {
    let args: Vec<String> = env::args().skip(1).collect();
    let mut options = Options {
        command: Command::Repl,
        no_type_check: false,
        use_vm: false,
    };

    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        match arg.as_str() {
            "new" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("new command requires an app name");
                    print_usage();
                    process::exit(64);
                }
                let name = args[i].clone();
                i += 1;
                let mut template = None;
                while i < args.len() {
                    match args[i].as_str() {
                        "--template" => {
                            i += 1;
                            if i >= args.len() {
                                eprintln!("--template requires a URL");
                                print_usage();
                                process::exit(64);
                            }
                            template = Some(args[i].clone());
                        }
                        value if value.starts_with('-') => {
                            eprintln!("Unknown option for new command: {}", value);
                            print_usage();
                            process::exit(64);
                        }
                        _ => {
                            eprintln!("Unexpected argument: {}", args[i]);
                            print_usage();
                            process::exit(64);
                        }
                    }
                    i += 1;
                }
                options.command = Command::New { name, template };
                return options;
            }
            "generate" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("generate command requires a subcommand (scaffold)");
                    print_usage();
                    process::exit(64);
                }
                let subcommand = args[i].clone();
                match subcommand.as_str() {
                    "scaffold" => {
                        i += 1;
                        if i >= args.len() {
                            eprintln!("generate scaffold requires a resource name");
                            print_usage();
                            process::exit(64);
                        }
                        let scaffold_name = args[i].clone();
                        let mut fields = Vec::new();
                        i += 1;
                        while i < args.len() && !args[i].starts_with('-') {
                            let value = &args[i];
                            if value.contains(':') {
                                fields.push(value.clone());
                            } else if value == "." || value == "/" || !value.is_empty() {
                                break;
                            }
                            i += 1;
                        }
                        let folder = if i < args.len() && !args[i].starts_with('-') {
                            args[i].clone()
                        } else {
                            ".".to_string()
                        };
                        options.command = Command::Generate {
                            scaffold_name,
                            fields,
                            folder,
                        };
                        return options;
                    }
                    "auth" => {
                        i += 1;
                        let folder = if i < args.len() && !args[i].starts_with('-') {
                            args[i].clone()
                        } else {
                            ".".to_string()
                        };
                        options.command = Command::GenerateAuth { folder };
                        return options;
                    }
                    "mailer" => {
                        i += 1;
                        if i >= args.len() {
                            eprintln!("generate mailer requires a name (e.g. User)");
                            print_usage();
                            process::exit(64);
                        }
                        let name = args[i].clone();
                        i += 1;
                        // Remaining bare args are action names; a trailing path
                        // (".", "/", or containing a separator) is the folder.
                        let mut actions = Vec::new();
                        let mut folder = ".".to_string();
                        for value in &args[i..] {
                            if value.starts_with('-') {
                                break;
                            }
                            if value == "." || value == "/" || value.contains('/') {
                                folder = value.clone();
                                break;
                            }
                            actions.push(value.clone());
                        }
                        options.command = Command::GenerateMailer {
                            name,
                            actions,
                            folder,
                        };
                        return options;
                    }
                    "component" => {
                        i += 1;
                        if i >= args.len() {
                            eprintln!("generate component requires a name (e.g. stats_card)");
                            print_usage();
                            process::exit(64);
                        }
                        let name = args[i].clone();
                        i += 1;
                        // Optional trailing folder (name itself may contain `/`
                        // for a subdirectory component, so it is not the folder).
                        let folder = if i < args.len() && !args[i].starts_with('-') {
                            args[i].clone()
                        } else {
                            ".".to_string()
                        };
                        options.command = Command::GenerateComponent { name, folder };
                        return options;
                    }
                    _ => {
                        eprintln!(
                            "Unknown generate subcommand: {} (try: scaffold, auth, mailer, component)",
                            subcommand
                        );
                        print_usage();
                        process::exit(64);
                    }
                }
            }
            "db:migrate" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("db:migrate command requires an action (up, down, status, generate)");
                    print_usage();
                    process::exit(64);
                }
                let action_str = args[i].clone();
                let action = match action_str.as_str() {
                    "up" => DbMigrateAction::Up,
                    "down" => DbMigrateAction::Down,
                    "status" => DbMigrateAction::Status,
                    "generate" => {
                        i += 1;
                        if i >= args.len() {
                            eprintln!("db:migrate generate requires a migration name");
                            print_usage();
                            process::exit(64);
                        }
                        DbMigrateAction::Generate {
                            name: args[i].clone(),
                        }
                    }
                    _ => {
                        eprintln!(
                            "Unknown db:migrate action: {} (valid: up, down, status, generate)",
                            action_str
                        );
                        print_usage();
                        process::exit(64);
                    }
                };
                i += 1;
                let folder = if i < args.len() && !args[i].starts_with('-') {
                    args[i].clone()
                } else {
                    ".".to_string()
                };
                options.command = Command::DbMigrate { action, folder };
                return options;
            }
            "db:indexes" => {
                i += 1;
                let folder = if i < args.len() && !args[i].starts_with('-') {
                    args[i].clone()
                } else {
                    ".".to_string()
                };
                options.command = Command::DbIndexes { folder };
                return options;
            }
            "routes" => {
                i += 1;
                let mut folder = ".".to_string();
                let mut folder_set = false;
                let mut grep: Option<String> = None;
                let mut json = false;
                while i < args.len() {
                    match args[i].as_str() {
                        "-g" | "--grep" => {
                            i += 1;
                            if i >= args.len() {
                                eprintln!("routes: {} requires a pattern", args[i - 1]);
                                process::exit(64);
                            }
                            grep = Some(args[i].clone());
                        }
                        arg if arg.starts_with("--grep=") => {
                            grep = Some(arg["--grep=".len()..].to_string());
                        }
                        "--json" => json = true,
                        arg if !arg.starts_with('-') && !folder_set => {
                            folder = arg.to_string();
                            folder_set = true;
                        }
                        other => {
                            eprintln!("Unknown option for routes: {}", other);
                            print_usage();
                            process::exit(64);
                        }
                    }
                    i += 1;
                }
                options.command = Command::Routes { folder, grep, json };
                return options;
            }
            "graph" => {
                i += 1;
                let action = args.get(i).cloned().unwrap_or_default();
                match action.as_str() {
                    "build" => {
                        i += 1; // consume "build"
                        let mut folder = ".".to_string();
                        let mut folder_set = false;
                        let mut no_embed = false;
                        let mut database: Option<String> = None;
                        let mut dry_run = false;
                        let mut fresh = false;
                        let mut ext: Option<String> = None;
                        let mut exclude: Option<String> = None;
                        let mut config: Option<String> = None;
                        while i < args.len() {
                            match args[i].as_str() {
                                "--no-embed" => no_embed = true,
                                "--dry-run" => dry_run = true,
                                "--fresh" => fresh = true,
                                "--database" => {
                                    i += 1;
                                    if i >= args.len() {
                                        eprintln!("--database requires a name");
                                        process::exit(64);
                                    }
                                    database = Some(args[i].clone());
                                }
                                arg if arg.starts_with("--database=") => {
                                    database = Some(arg["--database=".len()..].to_string());
                                }
                                "--ext" => {
                                    i += 1;
                                    if i >= args.len() {
                                        eprintln!("--ext requires a comma-separated list");
                                        process::exit(64);
                                    }
                                    ext = Some(args[i].clone());
                                }
                                arg if arg.starts_with("--ext=") => {
                                    ext = Some(arg["--ext=".len()..].to_string());
                                }
                                "--exclude" => {
                                    i += 1;
                                    if i >= args.len() {
                                        eprintln!("--exclude requires a comma-separated list");
                                        process::exit(64);
                                    }
                                    exclude = Some(args[i].clone());
                                }
                                arg if arg.starts_with("--exclude=") => {
                                    exclude = Some(arg["--exclude=".len()..].to_string());
                                }
                                "--config" => {
                                    i += 1;
                                    if i >= args.len() {
                                        eprintln!("--config requires a path");
                                        process::exit(64);
                                    }
                                    config = Some(args[i].clone());
                                }
                                arg if arg.starts_with("--config=") => {
                                    config = Some(arg["--config=".len()..].to_string());
                                }
                                arg if !arg.starts_with('-') && !folder_set => {
                                    folder = arg.to_string();
                                    folder_set = true;
                                }
                                other => {
                                    eprintln!("Unknown option for graph build: {}", other);
                                    print_usage();
                                    process::exit(64);
                                }
                            }
                            i += 1;
                        }
                        options.command = Command::Graph {
                            folder,
                            no_embed,
                            database,
                            dry_run,
                            fresh,
                            ext,
                            exclude,
                            config,
                        };
                        return options;
                    }
                    "query" => {
                        i += 1; // consume "query"
                        let mut positionals: Vec<String> = Vec::new();
                        let mut database: Option<String> = None;
                        let mut limit = 6usize;
                        let mut hops = 1usize;
                        let mut path: Option<String> = None;
                        let mut kind: Option<String> = None;
                        let mut json = false;
                        while i < args.len() {
                            match args[i].as_str() {
                                "--json" => json = true,
                                "--database" => {
                                    i += 1;
                                    if i >= args.len() {
                                        eprintln!("--database requires a name");
                                        process::exit(64);
                                    }
                                    database = Some(args[i].clone());
                                }
                                arg if arg.starts_with("--database=") => {
                                    database = Some(arg["--database=".len()..].to_string());
                                }
                                "--limit" => {
                                    i += 1;
                                    if i >= args.len() {
                                        eprintln!("--limit requires a number");
                                        process::exit(64);
                                    }
                                    limit = args[i].parse().unwrap_or_else(|_| {
                                        eprintln!("Invalid --limit: {}", args[i]);
                                        process::exit(64);
                                    });
                                }
                                "--hops" => {
                                    i += 1;
                                    if i >= args.len() {
                                        eprintln!("--hops requires a number");
                                        process::exit(64);
                                    }
                                    hops = args[i].parse().unwrap_or_else(|_| {
                                        eprintln!("Invalid --hops: {}", args[i]);
                                        process::exit(64);
                                    });
                                }
                                "--path" => {
                                    i += 1;
                                    if i >= args.len() {
                                        eprintln!("--path requires a path prefix");
                                        process::exit(64);
                                    }
                                    path = Some(args[i].clone());
                                }
                                arg if arg.starts_with("--path=") => {
                                    path = Some(arg["--path=".len()..].to_string());
                                }
                                "--kind" => {
                                    i += 1;
                                    if i >= args.len() {
                                        eprintln!(
                                            "--kind requires a comma-separated list (e.g. method,controller)"
                                        );
                                        process::exit(64);
                                    }
                                    kind = Some(args[i].clone());
                                }
                                arg if arg.starts_with("--kind=") => {
                                    kind = Some(arg["--kind=".len()..].to_string());
                                }
                                arg if arg.starts_with('-') => {
                                    eprintln!("Unknown option for graph query: {}", arg);
                                    print_usage();
                                    process::exit(64);
                                }
                                _ => positionals.push(args[i].clone()),
                            }
                            i += 1;
                        }
                        if positionals.is_empty() {
                            eprintln!("graph query requires a question: soli graph query \"<question>\" [folder]");
                            print_usage();
                            process::exit(64);
                        }
                        let question = positionals[0].clone();
                        let folder = positionals
                            .get(1)
                            .cloned()
                            .unwrap_or_else(|| ".".to_string());
                        options.command = Command::GraphQuery {
                            question,
                            folder,
                            database,
                            limit,
                            hops,
                            path,
                            kind,
                            json,
                        };
                        return options;
                    }
                    other => {
                        eprintln!(
                            "graph requires a subcommand: build or query (got '{}')",
                            other
                        );
                        print_usage();
                        process::exit(64);
                    }
                }
            }
            "db:seed" => {
                // Unlike `db:migrate`, a bare `soli db:seed` is valid and
                // means "run the seeds". Only `generate` is a sub-action.
                i += 1;
                let gen_name = if i < args.len() && args[i] == "generate" {
                    i += 1;
                    if i >= args.len() {
                        eprintln!("db:seed generate requires a seed name");
                        print_usage();
                        process::exit(64);
                    }
                    let name = args[i].clone();
                    i += 1;
                    Some(name)
                } else {
                    None
                };
                // Remaining positionals (any order): a `.sl` arg selects a
                // single seed file to run; any other positional is the app
                // folder (defaults to ".").
                let mut folder = ".".to_string();
                let mut file: Option<String> = None;
                while i < args.len() && !args[i].starts_with('-') {
                    if args[i].ends_with(".sl") {
                        file = Some(args[i].clone());
                    } else {
                        folder = args[i].clone();
                    }
                    i += 1;
                }
                let action = match gen_name {
                    Some(name) => DbSeedAction::Generate { name },
                    None => DbSeedAction::Run { file },
                };
                options.command = Command::DbSeed { action, folder };
                return options;
            }
            "serve" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("serve command requires a folder argument");
                    print_usage();
                    process::exit(64);
                }
                let folder = args[i].clone();
                let mut port = 5011u16;
                let mut dev_mode = false;
                let mut daemonize = false;
                // Worker count: `SOLI_WORKERS` env (the documented baseline-RSS
                // lever) if set, else the CPU core count. An explicit
                // `--workers N` below overrides either.
                let mut workers = std::env::var("SOLI_WORKERS")
                    .ok()
                    .and_then(|s| s.parse::<usize>().ok())
                    .filter(|&n| n > 0)
                    .unwrap_or_else(|| {
                        std::thread::available_parallelism()
                            .map(|p| p.get())
                            .unwrap_or(4)
                    });
                i += 1;
                while i < args.len() {
                    if args[i] == "--port" {
                        i += 1;
                        if i >= args.len() {
                            eprintln!("--port requires a port number");
                            print_usage();
                            process::exit(64);
                        }
                        port = args[i].parse().unwrap_or_else(|_| {
                            eprintln!("Invalid port number: {}", args[i]);
                            process::exit(64);
                        });
                    } else if args[i] == "--workers" {
                        i += 1;
                        if i >= args.len() {
                            eprintln!("--workers requires a number");
                            print_usage();
                            process::exit(64);
                        }
                        workers = args[i].parse().unwrap_or_else(|_| {
                            eprintln!("Invalid workers number: {}", args[i]);
                            process::exit(64);
                        });
                    } else if args[i] == "-d" {
                        daemonize = true;
                    } else if args[i] == "--dev" {
                        dev_mode = true;
                    } else if args[i].starts_with('-') {
                        eprintln!("Unknown option for serve: {}", args[i]);
                        print_usage();
                        process::exit(64);
                    }
                    i += 1;
                }
                options.command = Command::Serve {
                    folder,
                    port,
                    dev_mode,
                    workers,
                    daemonize,
                };
                return options;
            }
            "init" => {
                options.command = Command::Init;
                return options;
            }
            "add" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("add command requires a package name");
                    print_usage();
                    process::exit(64);
                }
                let name = args[i].clone();
                i += 1;
                let mut git = None;
                let mut path = None;
                let mut tag = None;
                let mut branch = None;
                let mut rev = None;
                let mut version = None;
                while i < args.len() {
                    match args[i].as_str() {
                        "--git" => {
                            i += 1;
                            if i >= args.len() {
                                eprintln!("--git requires a URL");
                                process::exit(64);
                            }
                            git = Some(args[i].clone());
                        }
                        "--path" => {
                            i += 1;
                            if i >= args.len() {
                                eprintln!("--path requires a path");
                                process::exit(64);
                            }
                            path = Some(args[i].clone());
                        }
                        "--tag" => {
                            i += 1;
                            if i >= args.len() {
                                eprintln!("--tag requires a value");
                                process::exit(64);
                            }
                            tag = Some(args[i].clone());
                        }
                        "--branch" => {
                            i += 1;
                            if i >= args.len() {
                                eprintln!("--branch requires a value");
                                process::exit(64);
                            }
                            branch = Some(args[i].clone());
                        }
                        "--rev" => {
                            i += 1;
                            if i >= args.len() {
                                eprintln!("--rev requires a value");
                                process::exit(64);
                            }
                            rev = Some(args[i].clone());
                        }
                        "--version" => {
                            i += 1;
                            if i >= args.len() {
                                eprintln!("--version requires a value");
                                process::exit(64);
                            }
                            version = Some(args[i].clone());
                        }
                        _ => {
                            eprintln!("Unknown option for add: {}", args[i]);
                            print_usage();
                            process::exit(64);
                        }
                    }
                    i += 1;
                }
                options.command = Command::Add {
                    name,
                    git,
                    path,
                    tag,
                    branch,
                    rev,
                    version,
                };
                return options;
            }
            "remove" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("remove command requires a package name");
                    print_usage();
                    process::exit(64);
                }
                options.command = Command::Remove {
                    name: args[i].clone(),
                };
                return options;
            }
            "install" => {
                options.command = Command::Install;
                return options;
            }
            "login" => {
                i += 1;
                let mut registry = None;
                let mut token = None;
                while i < args.len() {
                    match args[i].as_str() {
                        "--registry" => {
                            i += 1;
                            if i >= args.len() {
                                eprintln!("--registry requires a URL");
                                process::exit(64);
                            }
                            registry = Some(args[i].clone());
                        }
                        "--token" => {
                            i += 1;
                            if i >= args.len() {
                                eprintln!("--token requires a value");
                                process::exit(64);
                            }
                            token = Some(args[i].clone());
                        }
                        _ => {
                            eprintln!("Unknown option for login: {}", args[i]);
                            print_usage();
                            process::exit(64);
                        }
                    }
                    i += 1;
                }
                options.command = Command::Login { registry, token };
                return options;
            }
            "publish" => {
                i += 1;
                let mut registry = None;
                while i < args.len() {
                    match args[i].as_str() {
                        "--registry" => {
                            i += 1;
                            if i >= args.len() {
                                eprintln!("--registry requires a URL");
                                process::exit(64);
                            }
                            registry = Some(args[i].clone());
                        }
                        _ => {
                            eprintln!("Unknown option for publish: {}", args[i]);
                            print_usage();
                            process::exit(64);
                        }
                    }
                    i += 1;
                }
                options.command = Command::Publish { registry };
                return options;
            }
            "update" => {
                i += 1;
                let name = if i < args.len() && !args[i].starts_with('-') {
                    Some(args[i].clone())
                } else {
                    None
                };
                options.command = if name.is_none() {
                    Command::SelfUpdate
                } else {
                    Command::Update { name }
                };
                return options;
            }
            "--no-type-check" => options.no_type_check = true,
            "--vm" => options.use_vm = true,
            "lsp" => {
                options.command = Command::Lsp;
                return options;
            }
            "lint" => {
                i += 1;
                let mut paths: Vec<String> = Vec::new();
                while i < args.len() {
                    if !args[i].starts_with('-') {
                        paths.push(args[i].clone());
                    } else {
                        eprintln!("Unknown option for lint: {}", args[i]);
                        print_usage();
                        process::exit(64);
                    }
                    i += 1;
                }
                options.command = Command::Lint { paths };
                return options;
            }
            "check" => {
                i += 1;
                let mut paths: Vec<String> = Vec::new();
                while i < args.len() {
                    if !args[i].starts_with('-') {
                        paths.push(args[i].clone());
                    } else {
                        eprintln!("Unknown option for check: {}", args[i]);
                        print_usage();
                        process::exit(64);
                    }
                    i += 1;
                }
                options.command = Command::Check { paths };
                return options;
            }
            "fmt" => {
                i += 1;
                let mut paths: Vec<String> = Vec::new();
                let mut check = false;
                let mut stdin = false;
                while i < args.len() {
                    match args[i].as_str() {
                        "--check" => check = true,
                        "--stdin" => stdin = true,
                        s if !s.starts_with('-') => paths.push(args[i].clone()),
                        other => {
                            eprintln!("Unknown option for fmt: {}", other);
                            print_usage();
                            process::exit(64);
                        }
                    }
                    i += 1;
                }
                options.command = Command::Fmt {
                    paths,
                    check,
                    stdin,
                };
                return options;
            }
            "deploy" => {
                i += 1;
                let mut folder: Option<String> = None;
                while i < args.len() {
                    if args[i] == "--folder" || args[i] == "-f" {
                        i += 1;
                        if i >= args.len() {
                            eprintln!("--folder requires a path");
                            print_usage();
                            process::exit(64);
                        }
                        folder = Some(args[i].clone());
                    } else {
                        eprintln!("Unknown option for deploy: {}", args[i]);
                        print_usage();
                        process::exit(64);
                    }
                    i += 1;
                }
                options.command = Command::Deploy { folder };
                return options;
            }
            "engine" => {
                i += 1;
                if i >= args.len() {
                    eprintln!(
                        "engine command requires an action (create, db:migrate, db:rollback)"
                    );
                    print_usage();
                    process::exit(64);
                }
                let action = match args[i].as_str() {
                    "create" => {
                        i += 1;
                        if i >= args.len() {
                            eprintln!("engine create requires an engine name");
                            print_usage();
                            process::exit(64);
                        }
                        EngineAction::Create {
                            name: args[i].clone(),
                        }
                    }
                    "db:migrate" => {
                        i += 1;
                        let engine_name = if i < args.len() && !args[i].starts_with('-') {
                            Some(args[i].clone())
                        } else {
                            None
                        };
                        EngineAction::DbMigrate { engine_name }
                    }
                    "db:rollback" => {
                        i += 1;
                        let engine_name = if i < args.len() && !args[i].starts_with('-') {
                            Some(args[i].clone())
                        } else {
                            None
                        };
                        EngineAction::DbRollback { engine_name }
                    }
                    _ => {
                        eprintln!(
                            "Unknown engine action: {} (valid: create, db:migrate, db:rollback)",
                            args[i]
                        );
                        print_usage();
                        process::exit(64);
                    }
                };
                options.command = Command::Engine { action };
                return options;
            }
            "test" => {
                i += 1;
                let mut paths: Vec<String> = Vec::new();
                let mut jobs: Option<usize> = None;
                let mut coverage_formats: Vec<String> = vec!["console".to_string()];
                let mut coverage_min: Option<f64> = None;
                let mut no_coverage = false;
                let mut show_uncovered = false;
                let mut fail_on_n1 = false;
                while i < args.len() {
                    if args[i].starts_with('-') {
                        // Support `--coverage=html`, `--coverage=json,xml`,
                        // `--coverage=html --coverage=json`, etc. Any non-empty
                        // value implies --coverage.
                        if let Some(rest) = args[i].strip_prefix("--coverage=") {
                            no_coverage = false;
                            for fmt in rest.split(',') {
                                let fmt = fmt.trim();
                                if fmt.is_empty() {
                                    continue;
                                }
                                let normalized = fmt.to_ascii_lowercase();
                                if !matches!(
                                    normalized.as_str(),
                                    "console" | "html" | "json" | "xml"
                                ) {
                                    eprintln!(
                                        "Unknown --coverage format '{}'. Valid: console, html, json, xml",
                                        fmt
                                    );
                                    process::exit(64);
                                }
                                if !coverage_formats.iter().any(|f| f == &normalized) {
                                    coverage_formats.push(normalized);
                                }
                            }
                            i += 1;
                            continue;
                        }
                        if let Some(rest) = args[i].strip_prefix("--jobs=") {
                            jobs = Some(rest.parse().unwrap_or_else(|_| {
                                eprintln!("Invalid jobs number: {}", rest);
                                process::exit(64);
                            }));
                            i += 1;
                            continue;
                        }
                        if let Some(rest) = args[i].strip_prefix("--coverage-min=") {
                            coverage_min = Some(rest.parse().unwrap_or_else(|_| {
                                eprintln!("Invalid coverage percentage: {}", rest);
                                process::exit(64);
                            }));
                            i += 1;
                            continue;
                        }
                        match args[i].as_str() {
                            "--jobs" => {
                                i += 1;
                                if i >= args.len() {
                                    eprintln!("--jobs requires a number");
                                    print_usage();
                                    process::exit(64);
                                }
                                jobs = Some(args[i].parse().unwrap_or_else(|_| {
                                    eprintln!("Invalid jobs number: {}", args[i]);
                                    process::exit(64);
                                }));
                            }
                            "--coverage" => {
                                if !coverage_formats.iter().any(|f| f == "console") {
                                    coverage_formats.push("console".to_string());
                                }
                            }
                            "--no-coverage" => {
                                no_coverage = true;
                            }
                            "--show-uncovered" => {
                                show_uncovered = true;
                            }
                            "--fail-on-n1" => {
                                fail_on_n1 = true;
                            }
                            "--coverage-min" => {
                                i += 1;
                                if i >= args.len() {
                                    eprintln!("--coverage-min requires a percentage");
                                    print_usage();
                                    process::exit(64);
                                }
                                coverage_min = Some(args[i].parse().unwrap_or_else(|_| {
                                    eprintln!("Invalid coverage percentage: {}", args[i]);
                                    process::exit(64);
                                }));
                            }
                            _ => {
                                eprintln!("Unknown option for test: {}", args[i]);
                                print_usage();
                                process::exit(64);
                            }
                        }
                    } else {
                        paths.push(args[i].clone());
                    }
                    i += 1;
                }
                options.command = Command::Test {
                    paths,
                    jobs,
                    coverage_formats: if no_coverage {
                        vec![]
                    } else {
                        coverage_formats
                    },
                    coverage_min: if no_coverage { None } else { coverage_min },
                    no_coverage,
                    show_uncovered,
                    fail_on_n1,
                };
                return options;
            }
            "desktop" => {
                i += 1;
                // Only `desktop build` exists today; anything else is a typo,
                // and guessing would be worse than saying so.
                match args.get(i).map(|s| s.as_str()) {
                    Some("build") => i += 1,
                    Some(other) => {
                        eprintln!("Unknown desktop subcommand '{}' — expected 'build'", other);
                        process::exit(64);
                    }
                    None => {
                        eprintln!(
                            "Usage: soli desktop build <folder> --app-id <id> --solidb <path>"
                        );
                        process::exit(64);
                    }
                }

                let mut folder: Option<String> = None;
                let mut app_id: Option<String> = None;
                let mut app_name: Option<String> = None;
                let mut output: Option<String> = None;
                let mut db_binary: Option<String> = None;
                let mut seed: Option<String> = None;
                let mut protect = false;
                let mut target: Option<String> = None;

                // Same positional-anywhere convention as `soli build`.
                while i < args.len() {
                    let take_value = |i: &mut usize, flag: &str| -> String {
                        *i += 1;
                        args.get(*i).cloned().unwrap_or_else(|| {
                            eprintln!("{} requires a value", flag);
                            process::exit(64);
                        })
                    };
                    match args[i].as_str() {
                        "--app-id" => app_id = Some(take_value(&mut i, "--app-id")),
                        "--name" => app_name = Some(take_value(&mut i, "--name")),
                        "--output" | "-o" => output = Some(take_value(&mut i, "--output")),
                        "--solidb" => db_binary = Some(take_value(&mut i, "--solidb")),
                        "--seed" => seed = Some(take_value(&mut i, "--seed")),
                        "--target" => target = Some(take_value(&mut i, "--target")),
                        "--protect" => protect = true,
                        other if other.starts_with('-') => {
                            eprintln!("Unknown option '{}' for desktop build", other);
                            process::exit(64);
                        }
                        other => {
                            if folder.is_none() {
                                folder = Some(other.to_string());
                            } else {
                                eprintln!("Unexpected argument '{}'", other);
                                process::exit(64);
                            }
                        }
                    }
                    i += 1;
                }

                let Some(app_id) = app_id else {
                    eprintln!("desktop build requires --app-id <reverse.dns.id>");
                    process::exit(64);
                };
                let Some(db_binary) = db_binary else {
                    eprintln!("desktop build requires --solidb <path-to-database-binary>");
                    process::exit(64);
                };

                options.command = Command::DesktopBuild {
                    folder: folder.unwrap_or_else(|| ".".to_string()),
                    app_id,
                    app_name,
                    output,
                    db_binary,
                    seed,
                    protect,
                    target,
                };
                return options;
            }
            "build" => {
                i += 1;
                // The folder is positional but may appear in any position
                // relative to the flags (`soli build --protect app` and
                // `soli build app --protect` both work). Collect the first
                // non-flag token as the folder.
                let mut folder: Option<String> = None;
                let mut output = None;
                let mut standalone = false;
                let mut encrypt = false;
                let mut protect = false;
                let mut target: Option<String> = None;
                while i < args.len() {
                    match args[i].as_str() {
                        "--output" | "-o" => {
                            i += 1;
                            if i >= args.len() {
                                eprintln!("--output requires a path");
                                print_usage();
                                process::exit(64);
                            }
                            output = Some(args[i].clone());
                        }
                        "--standalone" => {
                            standalone = true;
                        }
                        "--target" => {
                            i += 1;
                            if i >= args.len() {
                                eprintln!(
                                    "--target requires a platform (linux-amd64, linux-arm64, darwin-amd64, darwin-arm64)"
                                );
                                print_usage();
                                process::exit(64);
                            }
                            target = Some(args[i].clone());
                        }
                        "--encrypt" => {
                            encrypt = true;
                        }
                        "--protect" => {
                            protect = true;
                            encrypt = true;
                        }
                        other if other.starts_with('-') => {
                            eprintln!("Unknown option for build: {}", other);
                            print_usage();
                            process::exit(64);
                        }
                        other => {
                            if folder.is_some() {
                                eprintln!(
                                    "build takes a single folder argument (got an extra '{}')",
                                    other
                                );
                                print_usage();
                                process::exit(64);
                            }
                            folder = Some(other.to_string());
                        }
                    }
                    i += 1;
                }
                let folder = folder.unwrap_or_else(|| {
                    eprintln!("build command requires a folder argument");
                    print_usage();
                    process::exit(64);
                });
                if target.is_some() && !standalone {
                    eprintln!("--target only applies to --standalone builds");
                    print_usage();
                    process::exit(64);
                }
                options.command = Command::Build {
                    folder,
                    output,
                    standalone,
                    encrypt,
                    protect,
                    target,
                };
                return options;
            }
            "--help" | "-h" => {
                print_usage();
                process::exit(0);
            }
            "--version" | "-v" => {
                println!("Soli {}", VERSION);
                process::exit(0);
            }
            "-e" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("-e requires a code argument");
                    print_usage();
                    process::exit(64);
                }
                options.command = Command::Eval {
                    code: args[i].clone(),
                };
            }
            _ if arg.starts_with('-') => {
                eprintln!("Unknown option: {}", arg);
                print_usage();
                process::exit(64);
            }
            _ => {
                if let Command::Run { .. } = options.command {
                    eprintln!("Only one script file can be specified");
                    print_usage();
                    process::exit(64);
                }
                options.command = Command::Run { file: arg.clone() };
            }
        }
        i += 1;
    }

    options
}
