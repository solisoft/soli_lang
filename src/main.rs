//! Soli CLI: Execute files or run the REPL.

use std::env;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::Path;
use std::process;

use rustyline::error::ReadlineError;
use rustyline::DefaultEditor;

use solilang::ExecutionMode;

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
    /// Start the REPL
    Repl,
    /// Serve an MVC application
    Serve {
        folder: String,
        port: u16,
        dev_mode: bool,
        mode: ExecutionMode,
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
}

/// CLI options parsed from arguments.
struct Options {
    mode: ExecutionMode,
    disassemble: bool,
    command: Command,
    no_type_check: bool,
}

fn print_usage() {
    eprintln!("Soli v0.1.0 - Solilang Interpreter");
    eprintln!();
    eprintln!("Usage: soli [options] [script.soli]");
    eprintln!("       soli serve <folder> [-d] [--dev] [--port PORT] [--workers N] [--mode MODE]");
    eprintln!("       soli test [path] [--jobs N] [--coverage] [--coverage-min N] [--no-coverage]");
    eprintln!();
    eprintln!("Commands:");
    eprintln!("  serve <folder>  Start MVC server from a project folder");
    eprintln!("  test [path]     Run tests (default: tests/ directory)");
    eprintln!();
    eprintln!("Options:");
    eprintln!("  --tree-walk     Use tree-walking interpreter (default)");
    eprintln!("  --bytecode      Use bytecode VM (faster)");
    eprintln!("  --jit           Use JIT compilation (fastest)");
    eprintln!("  --disassemble   Print bytecode disassembly before execution");
    eprintln!("  --no-type-check Skip type checking");
    eprintln!("  -d              Daemonize server (creates soli.pid and soli.log)");
    eprintln!("  --dev           Enable development mode (hot reload, no caching)");
    eprintln!("  --port PORT     Port for serve command (default: 3000)");
    eprintln!("  --workers N     Number of worker threads (default: CPU cores)");
    eprintln!("  --mode MODE     Execution mode for serve: tree-walk, bytecode (default), jit");
    eprintln!("  --jobs N        Number of parallel test workers (default: CPU cores)");
    eprintln!("  --coverage      Generate coverage report");
    eprintln!("  --coverage-min N  Fail if coverage is below N% (default: 80)");
    eprintln!("  --no-coverage   Skip coverage collection");
    eprintln!("  --help, -h      Show this help message");
    eprintln!();
    eprintln!("Examples:");
    eprintln!("  soli                          Start interactive REPL");
    eprintln!("  soli script.soli              Run a script file");
    eprintln!("  soli --bytecode script.soli   Run with bytecode VM");
    eprintln!("  soli --disassemble fib.soli   Show bytecode and run");
    eprintln!("  soli serve my_app             Start production server (no hot reload)");
    eprintln!("  soli serve my_app -d          Start as daemon (background process)");
    eprintln!("  soli serve my_app --dev       Start development server (with hot reload)");
    eprintln!("  soli serve my_app --port 8080 Start on custom port");
    eprintln!("  soli serve my_app --workers 16 Start server with 16 workers");
    eprintln!("  soli serve my_app --mode bytecode  Use bytecode VM for MVC server");
    eprintln!("  soli test                     Run all tests in tests/");
    eprintln!("  soli test spec.soli           Run specific test file");
    eprintln!("  soli test --coverage          Run tests with coverage");
    eprintln!("  soli test --jobs=4            Run tests with 4 workers");
}

fn parse_args() -> Options {
    let args: Vec<String> = env::args().skip(1).collect();
    let mut options = Options {
        mode: ExecutionMode::Bytecode,
        disassemble: false,
        command: Command::Repl,
        no_type_check: false,
    };

    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        match arg.as_str() {
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
                let mut port = 3000u16;
                let mut dev_mode = false; // Production by default
                let mut daemonize = false;
                let mut serve_mode = ExecutionMode::Bytecode;
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
                    } else if args[i] == "--mode" {
                        i += 1;
                        if i >= args.len() {
                            eprintln!("--mode requires a mode argument");
                            print_usage();
                            process::exit(64);
                        }
                        serve_mode = match args[i].as_str() {
                            "tree-walk" => ExecutionMode::TreeWalk,
                            "bytecode" => ExecutionMode::Bytecode,
                            "jit" => {
                                #[cfg(feature = "jit")]
                                {
                                    ExecutionMode::Jit
                                }
                                #[cfg(not(feature = "jit"))]
                                {
                                    eprintln!(
                                        "JIT mode not available - recompile with --features jit"
                                    );
                                    process::exit(64);
                                }
                            }
                            _ => {
                                eprintln!(
                                    "Unknown mode: {} (valid: tree-walk, bytecode, jit)",
                                    args[i]
                                );
                                print_usage();
                                process::exit(64);
                            }
                        };
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
                    mode: serve_mode,
                    workers,
                    daemonize,
                };
                return options;
            }
            "--tree-walk" => options.mode = ExecutionMode::TreeWalk,
            "--bytecode" => options.mode = ExecutionMode::Bytecode,
            #[cfg(feature = "jit")]
            "--jit" => options.mode = ExecutionMode::Jit,
            "--disassemble" => {
                options.disassemble = true;
                // Disassemble implies bytecode mode if not already set
                if options.mode == ExecutionMode::TreeWalk {
                    options.mode = ExecutionMode::Bytecode;
                }
            }
            "--no-type-check" => options.no_type_check = true,
            "test" => {
                // Parse test command
                i += 1;
                let mut path: Option<String> = None;
                let mut jobs = std::thread::available_parallelism()
                    .map(|p| p.get())
                    .unwrap_or(4);
                let mut coverage = true;
                let mut coverage_min: Option<f64> = Some(80.0);
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
        Command::Repl => run_repl(options.mode),
        Command::Run { file } => run_file(file, &options),
        Command::Serve {
            folder,
            port,
            dev_mode,
            mode,
            workers,
            daemonize,
        } => run_serve(folder, *port, *dev_mode, *mode, *workers, *daemonize),
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

fn run_serve(
    folder: &str,
    port: u16,
    dev_mode: bool,
    mode: ExecutionMode,
    workers: usize,
    daemonize: bool,
) {
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

    if let Err(e) =
        solilang::serve::serve_folder_with_options_and_mode(path, port, dev_mode, mode, workers)
    {
        eprintln!("Error: {}", e);
        process::exit(70);
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
            let is_soli = cmdline
                .split('\0')
                .any(|arg| arg.ends_with("/soli") || arg == "soli");

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

    let result = solilang::run_file(
        path,
        options.mode,
        !options.no_type_check,
        options.disassemble,
    );

    if let Err(e) = result {
        eprintln!("Error: {}", e);
        process::exit(70);
    }
}

fn run_repl(mode: ExecutionMode) {
    let mode_name = match mode {
        ExecutionMode::TreeWalk => "tree-walk",
        ExecutionMode::Bytecode => "bytecode",
        #[cfg(feature = "jit")]
        ExecutionMode::Jit => "jit",
    };
    println!("Soli v0.1.0 - Solilang Interpreter ({})", mode_name);
    println!("Type 'exit' or Ctrl+D to quit.\n");

    let mut rl = match DefaultEditor::new() {
        Ok(editor) => editor,
        Err(_) => {
            // Fallback to simple stdin reading
            run_simple_repl(mode);
            return;
        }
    };

    let mut repl_state = ReplState::new(mode);

    loop {
        match rl.readline(">>> ") {
            Ok(line) => {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                if line == "exit" || line == "quit" {
                    break;
                }

                let _ = rl.add_history_entry(line);

                // Try to execute the line
                if let Err(e) = repl_state.execute_line(line) {
                    eprintln!("Error: {}", e);
                }
            }
            Err(ReadlineError::Interrupted) => {
                println!("^C");
                continue;
            }
            Err(ReadlineError::Eof) => {
                println!("Goodbye!");
                break;
            }
            Err(e) => {
                eprintln!("Error: {}", e);
                break;
            }
        }
    }
}

fn run_simple_repl(mode: ExecutionMode) {
    let stdin = io::stdin();
    let mut repl_state = ReplState::new(mode);

    loop {
        print!(">>> ");
        io::stdout().flush().unwrap();

        let mut line = String::new();
        match stdin.read_line(&mut line) {
            Ok(0) => {
                println!("Goodbye!");
                break;
            }
            Ok(_) => {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                if line == "exit" || line == "quit" {
                    break;
                }

                if let Err(e) = repl_state.execute_line(line) {
                    eprintln!("Error: {}", e);
                }
            }
            Err(e) => {
                eprintln!("Error reading input: {}", e);
                break;
            }
        }
    }
}

/// REPL state that can work with different execution modes.
enum ReplState {
    TreeWalk(solilang::interpreter::Interpreter),
    Bytecode(solilang::bytecode::VM),
    #[cfg(feature = "jit")]
    Jit(solilang::jit::JitVM),
}

impl ReplState {
    fn new(mode: ExecutionMode) -> Self {
        match mode {
            ExecutionMode::TreeWalk => {
                ReplState::TreeWalk(solilang::interpreter::Interpreter::new())
            }
            ExecutionMode::Bytecode => ReplState::Bytecode(solilang::bytecode::VM::new()),
            #[cfg(feature = "jit")]
            ExecutionMode::Jit => ReplState::Jit(solilang::jit::JitVM::new()),
        }
    }

    fn execute_line(&mut self, source: &str) -> Result<(), solilang::error::SolilangError> {
        // Check if input looks like an expression that should print its result
        // Strip trailing semicolon for the check
        let trimmed = source.trim_end_matches(';').trim();

        let source = if !trimmed.ends_with('}')
            && !trimmed.starts_with("let ")
            && !trimmed.starts_with("fn ")
            && !trimmed.starts_with("class ")
            && !trimmed.starts_with("interface ")
            && !trimmed.starts_with("if ")
            && !trimmed.starts_with("while ")
            && !trimmed.starts_with("for ")
            && !trimmed.starts_with("return ")
            && !trimmed.starts_with("print(")
            && !trimmed.starts_with("println(")
        {
            // Wrap as print statement for expression evaluation
            format!("print({});", trimmed)
        } else if !source.ends_with(';') && !source.ends_with('}') {
            format!("{};", source)
        } else {
            source.to_string()
        };

        // Lex
        let tokens = solilang::lexer::Scanner::new(&source).scan_tokens()?;

        // Parse
        let program = solilang::parser::Parser::new(tokens).parse()?;

        // Skip type checking in REPL for flexibility

        // Execute based on mode
        match self {
            ReplState::TreeWalk(interpreter) => {
                interpreter.interpret(&program)?;
            }
            ReplState::Bytecode(vm) => {
                let mut compiler = solilang::bytecode::Compiler::new();
                let function = compiler.compile(&program)?;
                vm.run(function)?;
            }
            #[cfg(feature = "jit")]
            ReplState::Jit(vm) => {
                let mut compiler = solilang::bytecode::Compiler::new();
                let function = compiler.compile(&program)?;
                vm.run(function)?;
            }
        }

        Ok(())
    }
}

fn run_test(
    path: Option<&str>,
    jobs: usize,
    coverage: bool,
    coverage_min: Option<f64>,
    _no_coverage: bool,
) {
    use solilang::coverage::{CoverageConfig, CoverageReporter, CoverageTracker, OutputFormat};
    use std::cell::RefCell;
    use std::rc::Rc;

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

    let test_files: Vec<std::path::PathBuf> = if test_path.is_file() {
        vec![test_path]
    } else {
        collect_test_files(&test_path)
    };

    if test_files.is_empty() {
        println!("No test files found.");
        return;
    }

    println!("Found {} test file(s)", test_files.len());

    let mut tracker: Option<Rc<RefCell<CoverageTracker>>> = None;
    if coverage {
        let mut config = CoverageConfig::new();
        config.formats = vec![OutputFormat::Console];
        if let Some(min) = coverage_min {
            config.threshold = Some(min);
        }
        tracker = Some(Rc::new(RefCell::new(CoverageTracker::new(config))));
    }

    let mut passed = 0;
    let mut failed = 0;
    let mut pending = 0;

    for test_file in &test_files {
        match std::fs::read_to_string(test_file) {
            Ok(source) => {
                println!("\nRunning: {}", test_file.display());

                let test_name = test_file
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| "unknown".to_string());

                if let Some(ref tr) = tracker {
                    tr.borrow_mut().start_test(&test_name);
                }

                let mut interpreter = solilang::interpreter::Interpreter::new();
                if let Some(ref tr) = tracker {
                    interpreter.set_coverage_tracker(tr.clone());
                    interpreter.set_source_path(test_file.clone());
                }

                match solilang::run_with_path(
                    &source,
                    Some(test_file),
                    solilang::ExecutionMode::TreeWalk,
                    false,
                    false,
                ) {
                    Ok(_) => {
                        passed += 1;
                        println!("  ✓ Passed");
                    }
                    Err(e) => {
                        failed += 1;
                        println!("  ✗ Failed: {}", e);
                    }
                }

                if let Some(ref mut tr) = tracker {
                    tr.borrow_mut().end_test();
                }
            }
            Err(e) => {
                eprintln!("Error reading {}: {}", test_file.display(), e);
                failed += 1;
            }
        }
    }

    println!();
    println!("Test Results:");
    println!("  Passed:  {}", passed);
    println!("  Failed:  {}", failed);
    println!("  Pending: {}", pending);
    println!("  Total:   {}", passed + failed + pending);

    if let Some(ref tr) = tracker {
        let coverage_data = tr.borrow().get_aggregated_coverage();
        let reporter = CoverageReporter::new(CoverageConfig::new());
        reporter.generate_reports(&coverage_data);
    }

    if failed > 0 {
        process::exit(1);
    }
}

fn collect_test_files(dir: &std::path::PathBuf) -> Vec<std::path::PathBuf> {
    let mut files = Vec::new();

    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() {
                if let Some(ext) = path.extension() {
                    if ext == "soli" {
                        files.push(path);
                    }
                }
            } else if path.is_dir() {
                files.extend(collect_test_files(&path));
            }
        }
    }

    files
}
