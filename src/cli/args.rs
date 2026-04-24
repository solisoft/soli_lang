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
    Serve {
        folder: String,
        port: u16,
        dev_mode: bool,
        workers: usize,
        daemonize: bool,
    },
    Test {
        path: Option<String>,
        jobs: usize,
        /// Additional output format(s) beyond the console summary. Accepted
        /// via `--coverage=html`, `--coverage=json`, `--coverage=xml`.
        /// Empty = console only.
        coverage_formats: Vec<String>,
        coverage_min: Option<f64>,
        no_coverage: bool,
    },
    DbMigrate {
        action: DbMigrateAction,
        folder: String,
    },
    Lint {
        path: Option<String>,
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
    eprintln!("       soli serve <folder> [-d] [--dev] [--port PORT] [--workers N]");
    eprintln!("       soli test [path] [--jobs N] [--coverage] [--coverage=FORMAT] [--coverage-min N] [--no-coverage]");
    eprintln!("       soli lint [path]");
    eprintln!("       soli deploy [--folder <path>]");
    eprintln!("       soli db:migrate <up|down|status> [folder]");
    eprintln!("       soli db:migrate generate <name> [folder]");
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
    eprintln!("  serve <folder>       Start MVC server from a project folder");
    eprintln!("  test [path]          Run tests (default: tests/ directory)");
    eprintln!("  lint [path]          Lint .sl files for style issues and code smells");
    eprintln!("  deploy [--folder <path>]  Deploy application to servers via deploy.toml");
    eprintln!("  db:migrate           Database migration commands");
    eprintln!("  engine               Engine commands (create, db:migrate, db:rollback)");
    eprintln!("  -e <code>            Evaluate code and print result");
    eprintln!();
    eprintln!("Options:");
    eprintln!("  --no-type-check Skip type checking");
    eprintln!("  -d              Daemonize server (creates soli.pid and soli.log)");
    eprintln!("  --dev           Enable development mode (hot reload, no caching)");
    eprintln!("  --port PORT     Port for serve command (default: 5011)");
    eprintln!("  --workers N     Number of worker threads (default: CPU cores)");
    eprintln!("  --jobs N        Number of parallel test workers (default: CPU cores)");
    eprintln!("  --coverage           Generate coverage report (console)");
    eprintln!("  --coverage=FORMAT    Also generate FORMAT reports: html, json, xml (comma-sep)");
    eprintln!("  --coverage-min N     Fail if coverage is below N% (default: 80)");
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
    eprintln!("  soli serve my_app             Start production server (no hot reload)");
    eprintln!("  soli serve my_app -d          Start as daemon (background process)");
    eprintln!("  soli serve my_app --dev       Start development server (with hot reload)");
    eprintln!("  soli serve my_app --port 8080 Start on custom port");
    eprintln!("  soli serve my_app --workers 16 Start server with 16 workers");
    eprintln!("  soli test                     Run all tests in tests/");
    eprintln!("  soli test spec.sl             Run specific test file");
    eprintln!("  soli test --coverage          Run tests with coverage");
    eprintln!("  soli test --jobs=4            Run tests with 4 workers");
    eprintln!("  soli db:migrate up            Run pending migrations");
    eprintln!("  soli db:migrate down          Rollback last migration");
    eprintln!("  soli db:migrate status        Show migration status");
    eprintln!("  soli db:migrate generate create_users  Generate new migration");
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
                    _ => {
                        eprintln!(
                            "Unknown generate subcommand: {} (try: scaffold)",
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
                let mut workers = std::thread::available_parallelism()
                    .map(|p| p.get())
                    .unwrap_or(4);
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
            "lint" => {
                i += 1;
                let mut path: Option<String> = None;
                while i < args.len() {
                    if !args[i].starts_with('-') && path.is_none() {
                        path = Some(args[i].clone());
                    } else {
                        eprintln!("Unknown option for lint: {}", args[i]);
                        print_usage();
                        process::exit(64);
                    }
                    i += 1;
                }
                options.command = Command::Lint { path };
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
                let mut path: Option<String> = None;
                let mut jobs: usize = 1;
                let mut coverage_formats: Vec<String> = vec!["console".to_string()];
                let mut coverage_min: Option<f64> = None;
                let mut no_coverage = false;
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
                        match args[i].as_str() {
                            "--jobs" => {
                                i += 1;
                                if i >= args.len() {
                                    eprintln!("--jobs requires a number");
                                    print_usage();
                                    process::exit(64);
                                }
                                jobs = args[i].parse().unwrap_or_else(|_| {
                                    eprintln!("Invalid jobs number: {}", args[i]);
                                    process::exit(64);
                                });
                            }
                            "--coverage" => {
                                if !coverage_formats.iter().any(|f| f == "console") {
                                    coverage_formats.push("console".to_string());
                                }
                            }
                            "--no-coverage" => {
                                no_coverage = true;
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
                    } else if path.is_none() {
                        path = Some(args[i].clone());
                    } else {
                        eprintln!("Only one test path can be specified");
                        print_usage();
                        process::exit(64);
                    }
                    i += 1;
                }
                options.command = Command::Test {
                    path,
                    jobs,
                    coverage_formats: if no_coverage {
                        vec![]
                    } else {
                        coverage_formats
                    },
                    coverage_min: if no_coverage { None } else { coverage_min },
                    no_coverage,
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
