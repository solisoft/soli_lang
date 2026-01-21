//! Soli CLI: Execute files or run the REPL.

use std::env;
use std::io::{self, Write};
use std::path::Path;
use std::process;

use rustyline::error::ReadlineError;
use rustyline::DefaultEditor;

use solilang::ExecutionMode;

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
        live_reload: bool,
        mode: ExecutionMode,
        workers: usize,
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
    eprintln!(
        "       soli serve <folder> [--port PORT] [--workers N] [--no-live-reload] [--mode MODE]"
    );
    eprintln!();
    eprintln!("Commands:");
    eprintln!("  serve <folder>  Start MVC server from a project folder");
    eprintln!();
    eprintln!("Options:");
    eprintln!("  --tree-walk     Use tree-walking interpreter (default)");
    eprintln!("  --bytecode      Use bytecode VM (faster)");
    eprintln!("  --jit           Use JIT compilation (fastest)");
    eprintln!("  --disassemble   Print bytecode disassembly before execution");
    eprintln!("  --no-type-check Skip type checking");
    eprintln!("  --port PORT     Port for serve command (default: 3000)");
    eprintln!("  --workers N     Number of worker threads (default: CPU cores)");
    eprintln!("  --no-live-reload  Disable browser auto-refresh on file changes");
    eprintln!("  --mode MODE     Execution mode for serve: tree-walk, bytecode (default), jit");
    eprintln!("  --help, -h      Show this help message");
    eprintln!();
    eprintln!("Examples:");
    eprintln!("  soli                          Start interactive REPL");
    eprintln!("  soli script.soli              Run a script file");
    eprintln!("  soli --bytecode script.soli   Run with bytecode VM");
    eprintln!("  soli --disassemble fib.soli   Show bytecode and run");
    eprintln!("  soli serve my_app             Start MVC server");
    eprintln!("  soli serve my_app --port 8080 Start MVC server on port 8080");
    eprintln!("  soli serve my_app --workers 16 Start MVC server with 16 workers");
    eprintln!("  soli serve my_app --no-live-reload  Disable browser auto-refresh");
    eprintln!("  soli serve my_app --mode bytecode  Use bytecode VM for MVC server");
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
                let mut live_reload = true;
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
                    } else if args[i] == "--no-live-reload" {
                        live_reload = false;
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
                    live_reload,
                    mode: serve_mode,
                    workers,
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
            live_reload,
            mode,
            workers,
        } => run_serve(folder, *port, *live_reload, *mode, *workers),
    }
}

fn run_serve(folder: &str, port: u16, live_reload: bool, mode: ExecutionMode, workers: usize) {
    let path = Path::new(folder);

    if !path.exists() {
        eprintln!("Error: Folder '{}' does not exist", folder);
        process::exit(1);
    }

    if !path.is_dir() {
        eprintln!("Error: '{}' is not a directory", folder);
        process::exit(1);
    }

    if let Err(e) =
        solilang::serve::serve_folder_with_options_and_mode(path, port, live_reload, mode, workers)
    {
        eprintln!("Error: {}", e);
        process::exit(70);
    }
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
        // Check if input looks like an expression (no semicolon at end)
        // If so, wrap it to print the result
        let source = if !source.ends_with(';')
            && !source.ends_with('}')
            && !source.starts_with("let ")
            && !source.starts_with("fn ")
            && !source.starts_with("class ")
            && !source.starts_with("interface ")
            && !source.starts_with("if ")
            && !source.starts_with("while ")
            && !source.starts_with("for ")
            && !source.starts_with("return ")
        {
            // Wrap as print statement for expression evaluation
            format!("print({});", source)
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
