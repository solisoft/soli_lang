use std::io::Write;
use std::path::PathBuf;

use rustyline::error::ReadlineError;
use rustyline::DefaultEditor;

use crate::error::SolilangError;
use crate::interpreter::Interpreter;
use crate::lexer::Scanner;
use crate::parser::Parser;

const HISTORY_FILE: &str = ".soli_history";

pub struct EnhancedRepl {
    interpreter: Interpreter,
    history: Vec<String>,
    history_file: PathBuf,
}

impl EnhancedRepl {
    pub fn new() -> Self {
        let history_file = Self::get_history_path();
        let mut repl = Self {
            interpreter: Interpreter::new(),
            history: Vec::new(),
            history_file,
        };
        repl.load_history();
        repl
    }

    fn get_history_path() -> PathBuf {
        if let Some(home) = dirs::home_dir() {
            home.join(HISTORY_FILE)
        } else {
            PathBuf::from(HISTORY_FILE)
        }
    }

    fn load_history(&mut self) {
        if let Ok(content) = std::fs::read_to_string(&self.history_file) {
            for line in content.lines() {
                if !line.trim().is_empty() {
                    self.history.push(line.to_string());
                }
            }
        }
    }

    fn save_history(&self) {
        if let Some(parent) = self.history_file.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let content = self.history.join("\n");
        let _ = std::fs::write(&self.history_file, content);
    }

    pub fn run(&mut self) {
        println!("Soli - Enhanced REPL");
        println!("Type \".help\" for available commands.\n");

        let mut rl = match DefaultEditor::new() {
            Ok(editor) => editor,
            Err(_) => {
                println!("Warning: Using basic input (no history or completion)");
                self.run_basic();
                return;
            }
        };

        loop {
            match rl.readline(">>> ") {
                Ok(line) => {
                    let line = line.trim();
                    if line.is_empty() {
                        continue;
                    }
                    if line == "exit" || line == ".exit" || line == "quit" || line == ".quit" {
                        self.save_history();
                        println!("Goodbye!");
                        break;
                    }
                    let _ = rl.add_history_entry(line);
                    self.history.push(line.to_string());
                    if self.is_magic_command(line) {
                        self.handle_magic_command(line);
                    } else {
                        self.execute_single(line);
                    }
                }
                Err(ReadlineError::Interrupted) => {
                    println!("^C");
                    continue;
                }
                Err(ReadlineError::Eof) => {
                    self.save_history();
                    println!("\nGoodbye!");
                    break;
                }
                Err(e) => {
                    println!("Error: {}", e);
                    self.save_history();
                    break;
                }
            }
        }
    }

    fn run_basic(&mut self) {
        let stdin = std::io::stdin();
        loop {
            print!(">>> ");
            std::io::stdout().flush().unwrap();
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
                    self.history.push(line.to_string());
                    if self.is_magic_command(line) {
                        self.handle_magic_command(line);
                    } else {
                        self.execute_single(line);
                    }
                }
                Err(e) => {
                    println!("Error: {}", e);
                    break;
                }
            }
        }
    }

    fn is_magic_command(&self, line: &str) -> bool {
        line.starts_with('.') || line == "?" || line.starts_with("? ")
    }

    fn handle_magic_command(&mut self, line: &str) {
        match line {
            ".help" | "?" => self.cmd_help(),
            ".vars" | ".variables" => self.cmd_vars(),
            ".funcs" | ".functions" => self.cmd_funcs(),
            ".classes" => self.cmd_classes(),
            ".history" | ".hist" => self.cmd_history(),
            ".clear" | ".reset" => self.cmd_clear(),
            ".load" => println!("Usage: .load <filename>"),
            ".save" => println!("Usage: .save <filename>"),
            ".last" => self.cmd_last(),
            _ if line.starts_with(".load ") => {
                let parts: Vec<&str> = line.splitn(2, ' ').collect();
                if parts.len() == 2 {
                    self.cmd_load(parts[1]);
                }
            }
            _ if line.starts_with(".save ") => {
                let parts: Vec<&str> = line.splitn(2, ' ').collect();
                if parts.len() == 2 {
                    self.cmd_save(parts[1]);
                }
            }
            _ if line.starts_with("? ") => {
                let parts: Vec<&str> = line.splitn(2, ' ').collect();
                if parts.len() == 2 {
                    self.cmd_inspect(parts[1]);
                }
            }
            _ => println!(
                "Unknown command: {}. Type .help for available commands.",
                line
            ),
        }
    }

    fn cmd_help(&self) {
        println!();
        println!("Soli REPL Commands");
        println!();
        println!(".help          - Show this help message");
        println!(".vars          - List all variables in current scope");
        println!(".funcs         - List all defined functions");
        println!(".classes       - List all defined classes");
        println!(".history       - Show command history");
        println!(".clear         - Reset the REPL environment");
        println!(".last          - Show last result");
        println!(".load <file>   - Load and execute a file");
        println!(".save <file>   - Save session to a file");
        println!("? <expr>       - Inspect an expression");
        println!("_              - Reference last result");
        println!("exit / Ctrl+D  - Exit the REPL");
    }

    fn cmd_vars(&self) {
        let env = self.interpreter.global_env();
        let vars = env.borrow().get_var_names();
        if vars.is_empty() {
            println!("No variables defined.");
        } else {
            println!("Variables:");
            for name in vars {
                println!("  {}", name);
            }
        }
    }

    fn cmd_funcs(&self) {
        let env = self.interpreter.global_env();
        let var_names = env.borrow().get_var_names();
        let funcs: Vec<_> = var_names
            .iter()
            .filter(|k| k.starts_with("__func_"))
            .collect();
        if funcs.is_empty() {
            println!("No functions defined.");
        } else {
            println!("Functions:");
            for name in funcs {
                let clean_name = name.strip_prefix("__func_").unwrap_or(name);
                println!("  {}", clean_name);
            }
        }
    }

    fn cmd_classes(&self) {
        let env = self.interpreter.global_env();
        let var_names = env.borrow().get_var_names();
        let classes: Vec<_> = var_names
            .iter()
            .filter(|k| k.starts_with("__class_"))
            .collect();
        if classes.is_empty() {
            println!("No classes defined.");
        } else {
            println!("Classes:");
            for name in classes {
                let clean_name = name.strip_prefix("__class_").unwrap_or(name);
                println!("  {}", clean_name);
            }
        }
    }

    fn cmd_history(&self) {
        println!("History:");
        for (i, entry) in self.history.iter().enumerate() {
            println!("{:4}  {}", i + 1, entry);
        }
    }

    fn cmd_clear(&mut self) {
        self.interpreter = Interpreter::new();
        println!("Environment reset.");
    }

    fn cmd_load(&mut self, filename: &str) {
        let path = PathBuf::from(filename);
        if !path.exists() {
            println!("Error: File not found: {}", filename);
            return;
        }
        if let Ok(content) = std::fs::read_to_string(&path) {
            if let Err(e) = self.execute(&content) {
                println!("Error: {}", e);
            }
        } else {
            println!("Error: Could not read file");
        }
    }

    fn cmd_save(&self, filename: &str) {
        let content = self.history.join("\n");
        if let Err(e) = std::fs::write(filename, content) {
            println!("Error: {}", e);
        } else {
            println!("History saved.");
        }
    }

    fn cmd_last(&self) {
        println!("Use _ to reference the last result in expressions.");
    }

    fn cmd_inspect(&self, expr: &str) {
        let env = self.interpreter.global_env();
        if let Some(value) = env.borrow().get(expr) {
            println!("{}: {:?}", expr, value);
        } else {
            println!("Error: '{}' not found in scope", expr);
        }
    }

    fn execute_single(&mut self, line: &str) {
        if let Err(e) = self.execute(line) {
            println!("Error: {}", e);
        }
    }

    fn execute(&mut self, source: &str) -> Result<(), SolilangError> {
        let should_print = Self::should_print_result(source);
        let source = if should_print && !source.trim_end().ends_with('}') {
            format!("print({});", source.trim())
        } else if !source.ends_with(';')
            && !source.ends_with('}')
            && !source.trim_start().starts_with("let ")
            && !source.trim_start().starts_with("fn ")
            && !source.trim_start().starts_with("class ")
            && !source.trim_start().starts_with("const ")
        {
            format!("{};", source)
        } else {
            source.to_string()
        };
        let tokens = Scanner::new(&source).scan_tokens()?;
        let program = Parser::new(tokens).parse()?;
        self.interpreter.interpret(&program)?;
        Ok(())
    }

    fn should_print_result(source: &str) -> bool {
        let trimmed = source.trim_end_matches(';').trim();
        !trimmed.starts_with("let ")
            && !trimmed.starts_with("const ")
            && !trimmed.starts_with("fn ")
            && !trimmed.starts_with("class ")
            && !trimmed.starts_with("interface ")
            && !trimmed.starts_with("if ")
            && !trimmed.starts_with("while ")
            && !trimmed.starts_with("for ")
            && !trimmed.starts_with("return ")
            && !trimmed.starts_with("print(")
            && !trimmed.starts_with("println(")
            && !trimmed.starts_with(".")
            && !trimmed.starts_with("try")
            && !trimmed.starts_with("import ")
    }
}

impl Default for EnhancedRepl {
    fn default() -> Self {
        Self::new()
    }
}
