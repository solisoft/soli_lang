//! Soli CLI: Execute files or run the REPL.

use mimalloc::MiMalloc;

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

use std::env;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::Path;
use std::process;

const VERSION: &str = env!("CARGO_PKG_VERSION", "0.2.0");

#[cfg(unix)]
use daemonize::Daemonize;
#[cfg(unix)]
use nix::sys::signal::{kill, Signal};
#[cfg(unix)]
use nix::unistd::Pid;

/// CLI command to execute.
enum Command {
    /// Run a script file
    Run { file: String },
    /// Evaluate a string
    Eval { code: String },
    /// Start the REPL
    Repl,
    /// Create a new MVC application
    New {
        name: String,
        template: Option<String>,
    },
    /// Generate scaffold (model, controller, views)
    Generate {
        scaffold_name: String,
        fields: Vec<String>,
        folder: String,
    },
    /// Serve an MVC application
    Serve {
        folder: String,
        port: u16,
        dev_mode: bool,
        workers: usize,
        daemonize: bool,
    },
    /// Run tests
    Test {
        path: Option<String>,
        jobs: usize,
        coverage: bool,
        coverage_min: Option<f64>,
        no_coverage: bool,
    },
    /// Database migration commands
    DbMigrate {
        action: DbMigrateAction,
        folder: String,
    },
    /// Lint source files
    Lint { path: Option<String> },
    /// Initialize a new soli.toml
    Init,
    /// Add a dependency
    Add {
        name: String,
        git: Option<String>,
        path: Option<String>,
        tag: Option<String>,
        branch: Option<String>,
        rev: Option<String>,
        version: Option<String>,
    },
    /// Remove a dependency
    Remove { name: String },
    /// Install all dependencies
    Install,
    /// Update soli CLI to latest release (self-update)
    SelfUpdate,
    /// Update dependencies
    Update { name: Option<String> },
    /// Login to package registry
    Login {
        registry: Option<String>,
        token: Option<String>,
    },
    /// Publish package to registry
    Publish { registry: Option<String> },
    /// Deploy application to servers
    Deploy { folder: Option<String> },
}

/// Database migration action
enum DbMigrateAction {
    Up,
    Down,
    Status,
    Generate { name: String },
}

/// CLI options parsed from arguments.
struct Options {
    command: Command,
    no_type_check: bool,
    use_vm: bool,
}

fn print_usage() {
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
    eprintln!("       soli test [path] [--jobs N] [--coverage] [--coverage-min N] [--no-coverage]");
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
    eprintln!("  -e <code>            Evaluate code and print result");
    eprintln!();
    eprintln!("Options:");
    eprintln!("  --no-type-check Skip type checking");
    eprintln!("  -d              Daemonize server (creates soli.pid and soli.log)");
    eprintln!("  --dev           Enable development mode (hot reload, no caching)");
    eprintln!("  --port PORT     Port for serve command (default: 5011)");
    eprintln!("  --workers N     Number of worker threads (default: CPU cores)");
    eprintln!("  --jobs N        Number of parallel test workers (default: CPU cores)");
    eprintln!("  --coverage      Generate coverage report");
    eprintln!("  --coverage-min N  Fail if coverage is below N% (default: 80)");
    eprintln!("  --no-coverage   Skip coverage collection");
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
    eprintln!("  soli -e 'print(1 + 1)'        Evaluate code directly");
}

fn parse_args() -> Options {
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
                // Parse new command
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
                        arg if arg.starts_with("-") => {
                            eprintln!("Unknown option for new command: {}", arg);
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
                // Parse generate command
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

                        // Collect field arguments (name:type format)
                        let mut fields = Vec::new();
                        i += 1;
                        while i < args.len() && !args[i].starts_with('-') {
                            let arg = &args[i];
                            if arg.contains(':') {
                                fields.push(arg.clone());
                            } else if arg == "." || arg == "/" {
                                // It's a folder path, not a field
                                break;
                            } else if !arg.is_empty() {
                                // Assume it's a folder if it doesn't contain ':'
                                break;
                            }
                            i += 1;
                        }

                        // Check for optional folder argument
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
                // Parse db:migrate command
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

                // Check for optional folder argument
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
                // Parse serve command
                i += 1;
                if i >= args.len() {
                    eprintln!("serve command requires a folder argument");
                    print_usage();
                    process::exit(64);
                }
                let folder = args[i].clone();

                // Check for options
                let mut port = 5011u16;
                let mut dev_mode = false; // Production by default
                let mut daemonize = false;
                // Default to number of CPU cores for optimal parallelism
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
                        daemonize = true; // Enable daemon mode
                    } else if args[i] == "--dev" {
                        dev_mode = true; // Enable development mode
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
                if name.is_none() {
                    options.command = Command::SelfUpdate;
                } else {
                    options.command = Command::Update { name };
                }
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
            "test" => {
                // Parse test command
                i += 1;
                let mut path: Option<String> = None;
                let mut jobs: usize = 1;
                let mut coverage = false;
                let mut coverage_min: Option<f64> = None;
                let mut no_coverage = false;

                while i < args.len() {
                    if args[i].starts_with('-') {
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
                                coverage = true;
                            }
                            "--no-coverage" => {
                                no_coverage = true;
                                coverage = false;
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
                    coverage: !no_coverage && coverage,
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

fn main() {
    let options = parse_args();

    match &options.command {
        Command::Repl => run_repl(),
        Command::Run { file } => run_file(file, &options),
        Command::Eval { code } => run_eval(code, &options),
        Command::New { name, template } => run_new(name, template.as_deref()),
        Command::Generate {
            scaffold_name,
            fields,
            folder,
        } => run_generate(scaffold_name, fields, folder),
        Command::DbMigrate { action, folder } => run_db_migrate(action, folder),
        Command::Serve {
            folder,
            port,
            dev_mode,
            workers,
            daemonize,
        } => run_serve(folder, *port, *dev_mode, *workers, *daemonize),
        Command::Lint { path } => run_lint(path.as_deref()),
        Command::Deploy { folder } => run_deploy(folder.as_deref()),
        Command::Init => run_init(),
        Command::Add {
            name,
            git,
            path,
            tag,
            branch,
            rev,
            version,
        } => run_add(name, git, path, tag, branch, rev, version),
        Command::Remove { name } => run_remove(name),
        Command::Install => run_install(),
        Command::Update { name } => run_update(name.as_deref()),
        Command::SelfUpdate => run_self_update()
            .map_err(|e| e.to_string())
            .expect("Update failed"),
        Command::Login { registry, token } => run_login(registry.as_deref(), token.as_deref()),
        Command::Publish { registry } => run_publish(registry.as_deref()),
        Command::Test {
            path,
            jobs,
            coverage,
            coverage_min,
            no_coverage,
        } => run_test(
            path.as_deref(),
            *jobs,
            *coverage,
            *coverage_min,
            *no_coverage,
        ),
    }
}

fn run_serve(folder: &str, port: u16, dev_mode: bool, workers: usize, daemonize: bool) {
    let path = Path::new(folder);

    if !path.exists() {
        eprintln!("Error: Folder '{}' does not exist", folder);
        process::exit(1);
    }

    if !path.is_dir() {
        eprintln!("Error: '{}' is not a directory", folder);
        process::exit(1);
    }

    // Handle daemonization (Unix only)
    #[cfg(unix)]
    if daemonize {
        let pid_file = path.join("soli.pid");
        let log_file = path.join("soli.log");

        // Kill previous process if pid file exists
        kill_previous_process(&pid_file);

        // Create/truncate log file
        let log = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_file)
            .unwrap_or_else(|e| {
                eprintln!("Error: Cannot create log file: {}", e);
                process::exit(1);
            });

        let daemon = Daemonize::new()
            .pid_file(&pid_file)
            .chown_pid_file(true)
            .working_directory(path)
            .stdout(log.try_clone().unwrap())
            .stderr(log);

        println!("Starting soli daemon...");
        println!("  PID file: {}", pid_file.display());
        println!("  Log file: {}", log_file.display());

        match daemon.start() {
            Ok(_) => {
                // We're now in the daemon process
                println!("Daemon started successfully");
            }
            Err(e) => {
                eprintln!("Error: Failed to daemonize: {}", e);
                process::exit(1);
            }
        }
    }

    #[cfg(not(unix))]
    if daemonize {
        eprintln!("Error: Daemonization is only supported on Unix systems");
        process::exit(1);
    }

    if let Err(e) = solilang::serve::serve_folder_with_options(path, port, dev_mode, workers) {
        eprintln!("Error: {}", e);
        process::exit(70);
    }
}

fn run_new(name: &str, template: Option<&str>) {
    use solilang::scaffold::app_generator::print_success_message;
    match solilang::scaffold::create_app(name, template) {
        Ok(()) => {
            print_success_message(name);
        }
        Err(e) => {
            eprintln!("Error: {}", e);
            process::exit(1);
        }
    }
}

fn run_generate(scaffold_name: &str, fields: &[String], folder: &str) {
    match solilang::scaffold::create_scaffold_with_fields(folder, scaffold_name, fields) {
        Ok(()) => {
            solilang::scaffold::print_scaffold_success_message(scaffold_name);
        }
        Err(e) => {
            eprintln!("Error: {}", e);
            process::exit(1);
        }
    }
}

/// Kill the previous soli process if the PID file exists.
#[cfg(unix)]
fn kill_previous_process(pid_file: &Path) {
    if !pid_file.exists() {
        return;
    }

    // Read the PID from the file
    let mut file = match File::open(pid_file) {
        Ok(f) => f,
        Err(_) => return,
    };

    let mut pid_str = String::new();
    if file.read_to_string(&mut pid_str).is_err() {
        return;
    }

    let pid: i32 = match pid_str.trim().parse() {
        Ok(p) => p,
        Err(_) => {
            // Invalid PID, remove the stale file
            let _ = fs::remove_file(pid_file);
            return;
        }
    };

    // Check if the process exists and is named "soli"
    let cmdline_path = format!("/proc/{}/cmdline", pid);
    if let Ok(mut cmdline_file) = File::open(&cmdline_path) {
        let mut cmdline = String::new();
        if cmdline_file.read_to_string(&mut cmdline).is_ok() {
            // cmdline contains null-separated arguments, check if it's a soli process
            // Use Path to extract the filename for more robust detection
            let is_soli = cmdline.split('\0').any(|arg| {
                if arg.is_empty() {
                    return false;
                }
                // Check exact match first
                if arg == "soli" {
                    return true;
                }
                // Extract filename from path and check
                std::path::Path::new(arg)
                    .file_name()
                    .map(|name| name == "soli")
                    .unwrap_or(false)
            });

            if is_soli {
                println!("Killing previous soli process (PID: {})", pid);
                if let Err(e) = kill(Pid::from_raw(pid), Signal::SIGTERM) {
                    eprintln!("Warning: Failed to kill process {}: {}", pid, e);
                } else {
                    // Wait a bit for the process to terminate
                    std::thread::sleep(std::time::Duration::from_millis(500));

                    // Check if process is still running, force kill if necessary
                    if Path::new(&cmdline_path).exists() {
                        println!("Process still running, sending SIGKILL...");
                        let _ = kill(Pid::from_raw(pid), Signal::SIGKILL);
                        std::thread::sleep(std::time::Duration::from_millis(200));
                    }
                }
            }
        }
    }

    // Remove the old PID file
    let _ = fs::remove_file(pid_file);
}

fn run_file(path: &str, options: &Options) {
    let path = std::path::Path::new(path);

    let result = if options.use_vm {
        solilang::run_file_vm(path, !options.no_type_check)
    } else {
        solilang::run_file(path, !options.no_type_check)
    };

    if let Err(e) = result {
        eprintln!("Error: {}", e);
        process::exit(70);
    }
}

fn run_eval(code: &str, options: &Options) {
    let result = solilang::run_with_type_check(code, !options.no_type_check);

    if let Err(e) = result {
        eprintln!("Error: {}", e);
        process::exit(70);
    }
}

fn run_repl() {
    solilang::repl_tui::run_tui_repl().unwrap();
}

fn run_lint(path: Option<&str>) {
    let lint_path = match path {
        Some(p) => std::path::PathBuf::from(p),
        None => std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from(".")),
    };

    if !lint_path.exists() {
        eprintln!("Error: Path '{}' does not exist", lint_path.display());
        process::exit(1);
    }

    let files: Vec<std::path::PathBuf> = if lint_path.is_file() {
        vec![lint_path.clone()]
    } else {
        collect_test_files(&lint_path)
    };

    if files.is_empty() {
        println!("No .sl files found.");
        return;
    }

    let mut total_issues = 0;
    let mut files_with_issues = 0;

    for file in &files {
        let source = match fs::read_to_string(file) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("{}: error reading file: {}", file.display(), e);
                continue;
            }
        };

        let diagnostics = match solilang::lint(&source) {
            Ok(d) => d,
            Err(e) => {
                eprintln!("{}: parse error: {}", file.display(), e);
                continue;
            }
        };

        if !diagnostics.is_empty() {
            files_with_issues += 1;
            for d in &diagnostics {
                println!(
                    "{}:{}:{} - [{}] {}",
                    file.display(),
                    d.span.line,
                    d.span.column,
                    d.rule,
                    d.message
                );
            }
            total_issues += diagnostics.len();
        }
    }

    if total_issues > 0 {
        println!();
        println!(
            "{} issue(s) found in {} file(s)",
            total_issues, files_with_issues
        );
        process::exit(1);
    }
}

fn run_deploy(folder: Option<&str>) {
    let path = if let Some(f) = folder {
        Path::new(f).to_path_buf()
    } else {
        std::env::current_dir().expect("Failed to get current directory")
    };

    println!("Deploying from {}...", path.display());

    let config = match solilang::module::deploy::load_deploy_config(&path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Error: {}", e);
            std::process::exit(1);
        }
    };

    if config.servers.is_empty() {
        eprintln!("Error: No servers configured in deploy.toml");
        std::process::exit(1);
    }

    println!(
        "Deploying to {} server(s) in parallel...",
        config.servers.len()
    );
    println!();

    let rt = tokio::runtime::Runtime::new().expect("Failed to create Tokio runtime");
    let results = rt.block_on(solilang::module::deploy::deploy(config));

    solilang::module::deploy::print_summary(&results);

    let all_success = results.iter().all(|r| r.success);
    if !all_success {
        std::process::exit(1);
    }
}

fn format_duration(duration: std::time::Duration) -> String {
    let micros = duration.as_micros();
    if micros < 1000 {
        format!("{}µs", micros)
    } else if micros < 1_000_000 {
        format!("{}ms", (micros + 500) / 1000)
    } else {
        format!("{}.{}s", micros / 1_000_000, (micros % 1_000_000) / 10000)
    }
}

fn run_test(
    path: Option<&str>,
    jobs: usize,
    _coverage: bool,
    _coverage_min: Option<f64>,
    _no_coverage: bool,
) {
    use std::sync::mpsc;

    let test_path = match path {
        Some(p) => std::path::PathBuf::from(p),
        None => std::env::current_dir()
            .unwrap_or_else(|_| std::path::PathBuf::from("."))
            .join("tests"),
    };

    if !test_path.exists() {
        eprintln!("Error: Test path '{}' does not exist", test_path.display());
        process::exit(1);
    }

    // Set APP_ENV=test and load .env + .env.test
    std::env::set_var("APP_ENV", "test");
    let app_dir = if test_path.is_file() {
        test_path
            .parent()
            .and_then(|p| p.parent())
            .unwrap_or_else(|| std::path::Path::new("."))
    } else {
        test_path
            .parent()
            .unwrap_or_else(|| std::path::Path::new("."))
    };
    let env_test_path = app_dir.join(".env.test");
    if !env_test_path.exists() {
        eprintln!(
            "Error: .env.test file not found at '{}'. Create one with test database configuration.",
            env_test_path.display()
        );
        process::exit(1);
    }
    solilang::serve::env_loader::load_env_files(app_dir);

    // Ensure the test database exists in SoliDB
    ensure_test_database();

    solilang::interpreter::builtins::model::init_db_config();

    let test_files: Vec<std::path::PathBuf> = if test_path.is_file() {
        vec![test_path.clone()]
    } else {
        collect_test_files(&test_path)
    };

    if test_files.is_empty() {
        println!("No test files found.");
        return;
    }

    // Pre-load model sources if app/models/ exists (MVC app)
    let models_dir = app_dir.join("app").join("models");
    let model_preamble = if models_dir.is_dir() {
        let mut preamble = String::new();
        if let Ok(entries) = fs::read_dir(&models_dir) {
            let mut model_files: Vec<_> = entries
                .flatten()
                .filter(|e| e.path().extension().is_some_and(|ext| ext == "sl"))
                .collect();
            model_files.sort_by_key(|e| e.path());
            for entry in model_files {
                if let Ok(content) = fs::read_to_string(entry.path()) {
                    preamble.push_str(&content);
                    preamble.push('\n');
                }
            }
        }
        preamble
    } else {
        String::new()
    };

    let num_workers = jobs.max(1);
    println!(
        "Running {} test(s) with {} worker(s)...",
        test_files.len(),
        num_workers
    );
    println!();

    let test_dir = if test_path.is_file() {
        test_path.parent().unwrap_or(&test_path).to_path_buf()
    } else {
        test_path.clone()
    };

    // Only start test server if integration tests are present
    let needs_test_server = test_files.iter().any(|f| {
        f.file_name()
            .map(|n| n.to_string_lossy().contains("integration"))
            .unwrap_or(false)
    });

    if needs_test_server {
        println!("Starting test server...");

        // Find a random available port
        let port = {
            let listener =
                std::net::TcpListener::bind("127.0.0.1:0").expect("Failed to bind to random port");
            listener.local_addr().unwrap().port()
        };
        // Listener is dropped here, freeing the port for the server

        // Store port in test_server atomics so request helpers (get, post, etc.) can use it
        solilang::interpreter::builtins::test_server::start_test_server_on_port(port);

        // Start the actual app server in a background thread
        let app_dir_owned = app_dir.to_path_buf();
        std::thread::spawn(move || {
            if let Err(e) =
                solilang::serve::serve_folder_with_options(&app_dir_owned, port, false, 1)
            {
                eprintln!("Test server error: {}", e);
            }
        });

        // Wait for the server to be ready by polling the health endpoint
        let base_url = format!("http://127.0.0.1:{}", port);
        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(2))
            .build()
            .unwrap();
        let max_attempts = 50;
        let mut ready = false;
        for _ in 0..max_attempts {
            std::thread::sleep(std::time::Duration::from_millis(100));
            if client.get(format!("{}/health", base_url)).send().is_ok() {
                ready = true;
                break;
            }
        }
        if !ready {
            eprintln!("Error: Test server failed to start on port {}", port);
            process::exit(1);
        }

        println!("Test server running on port {}", port);
        println!();
        std::io::stdout().flush().unwrap();
    }

    let (tx, rx) = mpsc::channel();

    std::thread::scope(|s| {
        let mut handles = Vec::new();

        let chunk_size = test_files.len().div_ceil(num_workers);
        for chunk in test_files.chunks(chunk_size) {
            let tx = tx.clone();
            let chunk: Vec<std::path::PathBuf> = chunk.to_vec();
            let preamble = model_preamble.clone();

            handles.push(s.spawn(move || {
                let mut results: Vec<(std::path::PathBuf, bool, String, std::time::Duration, i64)> =
                    Vec::new();

                for file in chunk {
                    let start = std::time::Instant::now();
                    let result = std::fs::read_to_string(&file).map_err(|e| e.to_string());

                    let (passed, error, assertions) = match result {
                        Ok(source) => {
                            // Prepend model definitions for non-integration tests
                            let full_source = if !preamble.is_empty()
                                && !file
                                    .file_name()
                                    .map(|n| n.to_string_lossy().contains("integration"))
                                    .unwrap_or(false)
                            {
                                format!("{}\n{}", preamble, source)
                            } else {
                                source
                            };
                            // Catch panics from async operations (e.g., WebSocket tests)
                            let panic_result = std::panic::catch_unwind(|| {
                                solilang::run_with_path_and_coverage(
                                    &full_source,
                                    Some(&file),
                                    false,
                                    None,
                                    Some(&file),
                                )
                            });
                            match panic_result {
                                Ok(Ok(count)) => (true, String::new(), count),
                                Ok(Err(e)) => (false, e.to_string(), 0),
                                Err(_) => (
                                    false,
                                    "Test panicked (may require async runtime)".to_string(),
                                    0,
                                ),
                            }
                        }
                        Err(e) => (false, e, 0),
                    };

                    let duration = start.elapsed();
                    results.push((file, passed, error, duration, assertions));
                }

                let _ = tx.send(results);
            }));
        }

        drop(tx);

        for handle in handles {
            handle.join().unwrap();
        }
    });

    let mut all_results: Vec<(std::path::PathBuf, bool, String, std::time::Duration, i64)> =
        Vec::new();
    for received in rx {
        all_results.extend(received);
    }

    let mut passed = 0;
    let mut failed = 0;
    let mut total_assertions = 0;

    let mut current_dir: Option<std::path::PathBuf> = None;
    for (path, passed_test, error, duration, assertions) in &all_results {
        let parent = path.parent().unwrap_or(path).to_path_buf();
        let relative_to_test_dir = path.strip_prefix(&test_dir).unwrap_or(path);
        let parent_str = relative_to_test_dir
            .parent()
            .and_then(|p| p.to_str())
            .unwrap_or(".");

        if current_dir.as_ref().map(|d| d != &parent).unwrap_or(true) {
            if current_dir.is_some() {
                println!();
            }
            current_dir = Some(parent.clone());
            if parent_str != "." {
                println!("{}", parent_str);
            }
        }

        let file_name = path.file_name().unwrap_or_default().to_string_lossy();
        let display_path = if parent_str == "." {
            file_name.to_string()
        } else {
            format!("{}/{}", parent_str, file_name)
        };

        let duration_str = format_duration(*duration);
        if *passed_test {
            passed += 1;
            total_assertions += *assertions;
            println!(
                "  {:40} {:>8} {:>6} ✓",
                display_path, duration_str, assertions
            );
        } else {
            failed += 1;
            println!(
                "  {:40} {:>8} {:>6} ✗ {}",
                display_path, duration_str, assertions, error
            );
        }
        std::io::stdout().flush().unwrap();
    }

    println!();
    println!("{}", if failed > 0 { "❌ " } else { "✓ " });
    println!(
        "  {} passed, {} failed ({} total)",
        passed,
        failed,
        passed + failed
    );
    println!("  {} assertions", total_assertions);

    if failed > 0 {
        process::exit(1);
    }

    println!();
}

/// Ensure the test database exists in SoliDB with a clean slate.
/// Drops and recreates the database so tests start fresh.
fn ensure_test_database() {
    let host = std::env::var("SOLIDB_HOST").unwrap_or_else(|_| "http://localhost:6745".to_string());
    let database = std::env::var("SOLIDB_DATABASE").unwrap_or_else(|_| "default".to_string());

    // Don't drop the "default" database
    if database == "default" {
        return;
    }

    // Build basic auth header if credentials are configured
    let auth_header = match (
        std::env::var("SOLIDB_USERNAME"),
        std::env::var("SOLIDB_PASSWORD"),
    ) {
        (Ok(user), Ok(pass)) => {
            use base64::Engine;
            let encoded =
                base64::engine::general_purpose::STANDARD.encode(format!("{}:{}", user, pass));
            Some(format!("Basic {}", encoded))
        }
        _ => None,
    };

    // Drop existing test database (ignore errors - may not exist)
    let drop_url = format!("{}/_api/database/{}", host, database);
    let mut drop_req = ureq::delete(&drop_url);
    if let Some(ref auth) = auth_header {
        drop_req = drop_req.set("Authorization", auth);
    }
    let _ = drop_req.call();

    // Create fresh test database
    let create_url = format!("{}/_api/database", host);
    let payload = format!(r#"{{"name":"{}"}}"#, database);
    let mut create_req = ureq::post(&create_url).set("Content-Type", "application/json");
    if let Some(ref auth) = auth_header {
        create_req = create_req.set("Authorization", auth);
    }
    let _ = create_req.send_string(&payload);
}

fn collect_test_files(dir: &Path) -> Vec<std::path::PathBuf> {
    let mut files = Vec::new();

    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() && path.extension().map(|e| e == "sl").unwrap_or(false) {
                files.push(path);
            } else if path.is_dir() {
                files.extend(collect_test_files(&path));
            }
        }
    }

    files
}

fn run_init() {
    use solilang::module::Package;

    let toml_path = Path::new("soli.toml");
    if toml_path.exists() {
        eprintln!("soli.toml already exists in this directory");
        process::exit(1);
    }

    // Use current directory name as package name
    let name = env::current_dir()
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
        .unwrap_or_else(|| "my-package".to_string());

    let pkg = Package::new(&name);
    let content = pkg.to_toml();

    fs::write(toml_path, content).unwrap_or_else(|e| {
        eprintln!("Error: Failed to write soli.toml: {}", e);
        process::exit(1);
    });

    println!();
    println!("  \x1b[32m\x1b[1m✓\x1b[0m Created soli.toml");
    println!();
}

fn run_add(
    name: &str,
    git: &Option<String>,
    path: &Option<String>,
    tag: &Option<String>,
    branch: &Option<String>,
    rev: &Option<String>,
    version: &Option<String>,
) {
    use solilang::module::{installer, lockfile::LockFile, Dependency, Package};

    // Find or create soli.toml
    let toml_path = match Package::find(Path::new(".")) {
        Some(p) => p,
        None => {
            eprintln!("No soli.toml found. Run 'soli init' first.");
            process::exit(1);
        }
    };

    let mut pkg = Package::load(&toml_path).unwrap_or_else(|e| {
        eprintln!("Error: Failed to load soli.toml: {}", e);
        process::exit(1);
    });

    let dep = if let Some(git_url) = git {
        Dependency::Git {
            url: git_url.clone(),
            tag: tag.clone(),
            branch: branch.clone(),
            rev: rev.clone(),
        }
    } else if let Some(dep_path) = path {
        Dependency::Path(dep_path.clone())
    } else if let Some(ver) = version {
        Dependency::Version(ver.clone())
    } else {
        eprintln!("Error: Must specify --git, --path, or --version");
        print_usage();
        process::exit(64);
    };

    installer::add_dependency(&mut pkg, name, dep.clone());

    // Write updated soli.toml
    fs::write(&toml_path, pkg.to_toml()).unwrap_or_else(|e| {
        eprintln!("Error: Failed to write soli.toml: {}", e);
        process::exit(1);
    });

    println!();
    println!("  \x1b[32m\x1b[1m✓\x1b[0m Added dependency '{}'", name);

    // Install if it's a remote dependency
    if matches!(dep, Dependency::Git { .. } | Dependency::Version(_)) {
        let lock_path = toml_path.with_file_name("soli.lock");
        let mut lock = LockFile::load(&lock_path).unwrap_or_default();

        println!();
        println!("  Installing...");
        if let Err(e) = installer::install_all(&pkg, &mut lock, &lock_path) {
            eprintln!("  \x1b[31mError:\x1b[0m {}", e);
            process::exit(1);
        }
    }

    println!();
}

fn run_remove(name: &str) {
    use solilang::module::{installer, lockfile::LockFile, Package};

    let toml_path = match Package::find(Path::new(".")) {
        Some(p) => p,
        None => {
            eprintln!("No soli.toml found.");
            process::exit(1);
        }
    };

    let mut pkg = Package::load(&toml_path).unwrap_or_else(|e| {
        eprintln!("Error: Failed to load soli.toml: {}", e);
        process::exit(1);
    });

    if !pkg.dependencies.contains_key(name) {
        eprintln!("Error: Dependency '{}' not found in soli.toml", name);
        process::exit(1);
    }

    let lock_path = toml_path.with_file_name("soli.lock");
    let mut lock = LockFile::load(&lock_path).unwrap_or_default();

    installer::remove_dependency(&mut pkg, name, &mut lock);

    // Write updated soli.toml
    fs::write(&toml_path, pkg.to_toml()).unwrap_or_else(|e| {
        eprintln!("Error: Failed to write soli.toml: {}", e);
        process::exit(1);
    });

    // Write updated lock file
    if let Err(e) = lock.save(&lock_path) {
        eprintln!("Warning: Failed to update lock file: {}", e);
    }

    println!();
    println!("  \x1b[32m\x1b[1m✓\x1b[0m Removed dependency '{}'", name);
    println!();
}

fn run_install() {
    use solilang::module::{installer, lockfile::LockFile, Package};

    let toml_path = match Package::find(Path::new(".")) {
        Some(p) => p,
        None => {
            eprintln!("No soli.toml found. Run 'soli init' first.");
            process::exit(1);
        }
    };

    let pkg = Package::load(&toml_path).unwrap_or_else(|e| {
        eprintln!("Error: Failed to load soli.toml: {}", e);
        process::exit(1);
    });

    let lock_path = toml_path.with_file_name("soli.lock");
    let mut lock = LockFile::load(&lock_path).unwrap_or_default();

    let remote_deps: Vec<_> = pkg
        .dependencies
        .iter()
        .filter(|(_, d)| {
            matches!(
                d,
                solilang::module::Dependency::Git { .. } | solilang::module::Dependency::Version(_)
            )
        })
        .collect();

    if remote_deps.is_empty() {
        println!();
        println!("  No remote dependencies to install.");
        println!();
        return;
    }

    println!();
    println!("  \x1b[1mInstalling dependencies...\x1b[0m");
    println!();

    if let Err(e) = installer::install_all(&pkg, &mut lock, &lock_path) {
        eprintln!("  \x1b[31mError:\x1b[0m {}", e);
        process::exit(1);
    }

    // Print summary
    let summary = installer::installed_summary(&lock);
    if !summary.is_empty() {
        println!();
        println!(
            "  \x1b[32m\x1b[1m✓\x1b[0m {} package(s) installed",
            summary.len()
        );
        for (name, rev, _) in &summary {
            println!("    {} @ {}", name, rev);
        }
    }
    println!();
}

fn run_update(name: Option<&str>) {
    use solilang::module::{installer, lockfile::LockFile, Package};

    let toml_path = match Package::find(Path::new(".")) {
        Some(p) => p,
        None => {
            eprintln!("No soli.toml found. Run 'soli init' first.");
            process::exit(1);
        }
    };

    let pkg = Package::load(&toml_path).unwrap_or_else(|e| {
        eprintln!("Error: Failed to load soli.toml: {}", e);
        process::exit(1);
    });

    let lock_path = toml_path.with_file_name("soli.lock");
    let mut lock = LockFile::load(&lock_path).unwrap_or_default();

    println!();
    println!("  \x1b[1mUpdating dependencies...\x1b[0m");
    println!();

    let result = if let Some(pkg_name) = name {
        installer::update_package(pkg_name, &pkg, &mut lock, &lock_path)
    } else {
        installer::update_all(&pkg, &mut lock, &lock_path)
    };

    if let Err(e) = result {
        eprintln!("  \x1b[31mError:\x1b[0m {}", e);
        process::exit(1);
    }

    let summary = installer::installed_summary(&lock);
    if !summary.is_empty() {
        println!();
        println!(
            "  \x1b[32m\x1b[1m✓\x1b[0m {} package(s) up to date",
            summary.len()
        );
        for (name, rev, _) in &summary {
            println!("    {} @ {}", name, rev);
        }
    }
    println!();
}

fn run_self_update() -> Result<(), Box<dyn std::error::Error>> {
    let repo = "solisoft/soli_lang";
    let current_version = VERSION;

    println!();
    println!("  \x1b[1mChecking for updates...\x1b[0m");
    println!();

    let os = match std::env::consts::OS {
        "linux" => "linux",
        "macos" => "darwin",
        _ => {
            eprintln!("  \x1b[31mError:\x1b[0m Unsupported operating system");
            process::exit(1);
        }
    };

    let arch = match std::env::consts::ARCH {
        "x86_64" => "amd64",
        "aarch64" | "arm64" => "arm64",
        _ => {
            eprintln!(
                "  \x1b[31mError:\x1b[0m Unsupported architecture: {}",
                std::env::consts::ARCH
            );
            process::exit(1);
        }
    };

    let tag_url = format!("https://api.github.com/repos/{}/releases/latest", repo);
    let client = reqwest::blocking::Client::builder()
        .user_agent("soli-lang-cli")
        .build()
        .expect("Failed to create HTTP client");

    let response = client
        .get(&tag_url)
        .send()
        .map_err(|e| format!("Failed to fetch latest release: {}", e))
        .and_then(|resp| {
            if resp.status() == reqwest::StatusCode::NOT_FOUND {
                Err("Release not found".into())
            } else {
                resp.error_for_status()
                    .map_err(|e| format!("GitHub API error: {}", e))
            }
        });

    let response = match response {
        Ok(r) => r,
        Err(e) => {
            eprintln!("  \x1b[31mError:\x1b[0m {}", e);
            process::exit(1);
        }
    };

    let json: serde_json::Value = response
        .json()
        .map_err(|e| format!("Failed to parse GitHub response: {}", e))
        .expect("Failed to parse release info");

    let latest_tag = json["tag_name"]
        .as_str()
        .unwrap_or("v0.0.0")
        .trim_start_matches('v');

    if latest_tag == current_version {
        println!(
            "  You are already running the latest version: v{}",
            current_version
        );
        println!();
        return Ok(());
    }

    println!("  Current version: v{}", current_version);
    println!("  Latest version: v{}", latest_tag);
    println!();

    let tarball = format!("soli-{}-{}.tar.gz", os, arch);
    let download_url = format!(
        "https://github.com/{}/releases/download/v{}/{}",
        repo, latest_tag, tarball
    );

    println!("  Downloading {}...", tarball);

    let temp_dir = std::env::temp_dir();
    let tarball_path = temp_dir.join(&tarball);
    let binary_path = temp_dir.join("soli");

    let response = client
        .get(&download_url)
        .send()
        .map_err(|e| format!("Failed to download: {}", e))
        .and_then(|resp| {
            if resp.status() == reqwest::StatusCode::NOT_FOUND {
                Err(format!(
                    "Release asset not found: {} - may not be available for {}-{}",
                    latest_tag, os, arch
                ))
            } else {
                resp.error_for_status()
                    .map_err(|e| format!("Download error: {}", e))
            }
        });

    let mut response = match response {
        Ok(r) => r,
        Err(e) => {
            eprintln!("  \x1b[31mError:\x1b[0m {}", e);
            process::exit(1);
        }
    };

    let mut file = std::fs::File::create(&tarball_path)
        .map_err(|e| format!("Failed to create temp file: {}", e))
        .expect("Failed to create temp file");

    response
        .copy_to(&mut file)
        .map_err(|e| format!("Failed to write download: {}", e))
        .expect("Failed to write download");

    println!("  Extracting...");
    let tf = std::fs::File::open(&tarball_path).expect("Failed to open tarball");
    let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(tf));
    archive
        .unpack(&temp_dir)
        .expect("Failed to extract tarball");

    let current_exe = std::env::current_exe().expect("Failed to get current executable path");
    let backup_path = current_exe.parent().unwrap().join("soli.backup");

    if std::fs::rename(&current_exe, &backup_path).is_err() {
        std::fs::copy(&current_exe, &backup_path)
            .map_err(|e| format!("Failed to backup current binary: {}", e))?;
        std::fs::remove_file(&current_exe)
            .map_err(|e| format!("Failed to remove old binary: {}", e))?;
    }
    if std::fs::rename(&binary_path, &current_exe).is_err() {
        std::fs::copy(&binary_path, &current_exe)
            .map_err(|e| format!("Failed to install new binary: {}", e))?;
        std::fs::remove_file(&binary_path).ok();
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&current_exe)
            .expect("Failed to get permissions")
            .permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&current_exe, perms)
            .expect("Failed to set executable permissions");
    }

    std::fs::remove_file(&tarball_path).ok();
    std::fs::remove_file(&backup_path).ok();

    println!();
    println!("  \x1b[32m\x1b[1m✓\x1b[0m Updated to v{}", latest_tag);
    println!();
    Ok(())
}

fn run_login(registry: Option<&str>, token: Option<&str>) {
    use solilang::module::credentials::{save_credentials, Credentials};
    use solilang::module::registry::DEFAULT_REGISTRY;

    let registry_url = registry.unwrap_or(DEFAULT_REGISTRY);

    let token_value = if let Some(t) = token {
        t.to_string()
    } else {
        // Prompt interactively
        eprint!("  Enter API token: ");
        let mut input = String::new();
        std::io::stdin().read_line(&mut input).unwrap_or_else(|e| {
            eprintln!("Error reading input: {}", e);
            process::exit(1);
        });
        let input = input.trim().to_string();
        if input.is_empty() {
            eprintln!("Error: Token cannot be empty");
            process::exit(1);
        }
        input
    };

    let creds = Credentials {
        url: registry_url.to_string(),
        token: token_value,
    };

    save_credentials(&creds).unwrap_or_else(|e| {
        eprintln!("Error: {}", e);
        process::exit(1);
    });

    println!();
    println!("  \x1b[32m\x1b[1m✓\x1b[0m Logged in to {}", registry_url);
    println!();
}

fn run_publish(registry: Option<&str>) {
    use solilang::module::credentials::load_credentials;
    use solilang::module::registry::{self, DEFAULT_REGISTRY};
    use solilang::module::Package;

    let toml_path = match Package::find(Path::new(".")) {
        Some(p) => p,
        None => {
            eprintln!("No soli.toml found. Run 'soli init' first.");
            process::exit(1);
        }
    };

    let pkg = Package::load(&toml_path).unwrap_or_else(|e| {
        eprintln!("Error: Failed to load soli.toml: {}", e);
        process::exit(1);
    });

    if pkg.name.is_empty() {
        eprintln!("Error: package.name is required in soli.toml");
        process::exit(1);
    }
    if pkg.version.is_empty() {
        eprintln!("Error: package.version is required in soli.toml");
        process::exit(1);
    }

    let creds = load_credentials().unwrap_or_else(|| {
        eprintln!("Error: Not logged in. Run 'soli login' first.");
        process::exit(1);
    });

    let registry_url = match registry {
        Some(url) => url.to_string(),
        None if !creds.url.is_empty() => creds.url.clone(),
        None => DEFAULT_REGISTRY.to_string(),
    };

    let project_dir = toml_path.parent().unwrap_or(Path::new("."));
    let description = pkg.description.as_deref().unwrap_or("");

    // Create tarball
    println!();
    println!("  \x1b[1mPackaging {}@{}...\x1b[0m", pkg.name, pkg.version);

    let tarball_path = std::env::temp_dir().join(format!("{}-{}.tar.gz", pkg.name, pkg.version));
    create_tarball(project_dir, &tarball_path).unwrap_or_else(|e| {
        eprintln!("Error: Failed to create tarball: {}", e);
        process::exit(1);
    });

    println!("  \x1b[1mPublishing to {}...\x1b[0m", registry_url);

    registry::publish_package(
        &registry_url,
        &creds.token,
        &pkg.name,
        &pkg.version,
        description,
        &tarball_path,
    )
    .unwrap_or_else(|e| {
        // Clean up tarball on error
        let _ = fs::remove_file(&tarball_path);
        eprintln!("  \x1b[31mError:\x1b[0m {}", e);
        process::exit(1);
    });

    // Clean up tarball
    let _ = fs::remove_file(&tarball_path);

    println!();
    println!(
        "  \x1b[32m\x1b[1m✓\x1b[0m Published {}@{}",
        pkg.name, pkg.version
    );
    println!();
}

/// Create a tar.gz archive of the project directory.
fn create_tarball(project_dir: &Path, dest: &std::path::Path) -> Result<(), String> {
    use flate2::write::GzEncoder;
    use flate2::Compression;
    use walkdir::WalkDir;

    let file = fs::File::create(dest).map_err(|e| format!("Failed to create tarball: {}", e))?;
    let encoder = GzEncoder::new(file, Compression::default());
    let mut archive = tar::Builder::new(encoder);

    let skip_dirs = [".git", "node_modules", "target", ".soli"];

    for entry in WalkDir::new(project_dir).into_iter().filter_entry(|e| {
        let name = e.file_name().to_string_lossy();
        !skip_dirs.iter().any(|d| name == *d)
    }) {
        let entry = entry.map_err(|e| format!("Failed to walk directory: {}", e))?;
        let path = entry.path();
        let relative = path
            .strip_prefix(project_dir)
            .map_err(|e| format!("Failed to compute relative path: {}", e))?;

        if relative.as_os_str().is_empty() {
            continue;
        }

        if path.is_file() {
            archive
                .append_path_with_name(path, relative)
                .map_err(|e| format!("Failed to add file to tarball: {}", e))?;
        } else if path.is_dir() {
            archive
                .append_dir(relative, path)
                .map_err(|e| format!("Failed to add directory to tarball: {}", e))?;
        }
    }

    archive
        .finish()
        .map_err(|e| format!("Failed to finalize tarball: {}", e))?;

    Ok(())
}

fn run_db_migrate(action: &DbMigrateAction, folder: &str) {
    use solilang::migration::{DbConfig, MigrationRunner};

    let app_path = Path::new(folder);

    if !app_path.exists() {
        eprintln!("Error: Folder '{}' does not exist", folder);
        process::exit(1);
    }

    // Load database config from .env file and environment
    let config = DbConfig::from_env(app_path);

    match action {
        DbMigrateAction::Up => {
            println!();
            println!("  \x1b[1mRunning migrations...\x1b[0m");
            println!();

            let runner = MigrationRunner::new(config, app_path);
            match runner.migrate_up() {
                Ok(result) => {
                    println!();
                    println!("  \x1b[32m{}\x1b[0m", result.message);
                    println!();
                }
                Err(e) => {
                    eprintln!("  \x1b[31mError:\x1b[0m {}", e);
                    process::exit(1);
                }
            }
        }
        DbMigrateAction::Down => {
            println!();
            println!("  \x1b[1mRolling back migration...\x1b[0m");
            println!();

            let runner = MigrationRunner::new(config, app_path);
            match runner.migrate_down() {
                Ok(result) => {
                    println!();
                    println!("  \x1b[32m{}\x1b[0m", result.message);
                    println!();
                }
                Err(e) => {
                    eprintln!("  \x1b[31mError:\x1b[0m {}", e);
                    process::exit(1);
                }
            }
        }
        DbMigrateAction::Status => {
            let runner = MigrationRunner::new(config, app_path);
            match runner.status() {
                Ok(status) => {
                    solilang::migration::print_status(&status);
                }
                Err(e) => {
                    eprintln!("  \x1b[31mError:\x1b[0m {}", e);
                    process::exit(1);
                }
            }
        }
        DbMigrateAction::Generate { name } => {
            match solilang::migration::generate_migration(app_path, name) {
                Ok(path) => {
                    println!();
                    println!("  \x1b[32mCreated migration:\x1b[0m {}", path.display());
                    println!();
                }
                Err(e) => {
                    eprintln!("  \x1b[31mError:\x1b[0m {}", e);
                    process::exit(1);
                }
            }
        }
    }
}
