use std::io::{self, Write};
use std::path::PathBuf;

use crate::interpreter::Interpreter;
use crate::lexer::Scanner;
use crate::parser::Parser;
use crate::repl_highlight::SyntaxHighlighter;

const HISTORY_FILE: &str = ".soli_history";

pub struct SimpleRepl {
    interpreter: Interpreter,
    history: Vec<String>,
    history_file: PathBuf,
    multiline_buffer: String,
    multiline_indent: usize,
    is_multiline: bool,
    brace_balance: i32,
    highlighter: SyntaxHighlighter,
    highlighting_enabled: bool,
}

impl SimpleRepl {
    pub fn new() -> Self {
        colored::control::set_override(true);
        let history_file = Self::get_history_path();
        let mut repl = Self {
            interpreter: Interpreter::new(),
            history: Vec::new(),
            history_file,
            multiline_buffer: String::new(),
            multiline_indent: 0,
            is_multiline: false,
            brace_balance: 0,
            highlighter: SyntaxHighlighter::new(),
            highlighting_enabled: true,
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
        println!("Soli - REPL");
        println!("Type .help for available commands.\n");

        let stdin = io::stdin();

        loop {
            let prompt = self.get_prompt();
            print!("{}", prompt);
            io::stdout().flush().unwrap();

            let mut line = String::new();
            match stdin.read_line(&mut line) {
                Ok(0) => {
                    println!("Goodbye!");
                    break;
                }
                Ok(_) => {
                    let line = line.trim_end();
                    if line.is_empty() && !self.is_multiline {
                        continue;
                    }

                    if line == "exit" || line == ".exit" || line == "quit" || line == ".quit" {
                        self.save_history();
                        println!("Goodbye!");
                        break;
                    }

                    if self.is_multiline {
                        self.handle_multiline_input(line);
                    } else {
                        self.history.push(line.to_string());

                        if self.is_magic_command(line) {
                            self.handle_magic_command(line);
                        } else if self.detect_multiline_needed(line) {
                            self.enter_multiline(line);
                            continue;
                        } else {
                            self.execute_single(line);
                        }
                    }
                }
                Err(_) => {
                    self.save_history();
                    println!("\nGoodbye!");
                    break;
                }
            }
        }
    }

    fn get_prompt(&self) -> String {
        if self.is_multiline {
            format!("{:indent$}... ", "", indent = self.multiline_indent)
        } else {
            ">>> ".to_string()
        }
    }

    fn detect_multiline_needed(&self, line: &str) -> bool {
        let trimmed = line.trim();
        trimmed.ends_with('{')
            || (trimmed.starts_with("class ") && !trimmed.ends_with('}'))
            || trimmed.starts_with("fn ")
            || trimmed.starts_with("if ")
            || trimmed.starts_with("while ")
            || trimmed.starts_with("for ")
            || trimmed.starts_with("match ")
            || trimmed == "do"
            || trimmed.starts_with("try")
    }

    fn enter_multiline(&mut self, line: &str) {
        self.is_multiline = true;
        self.multiline_buffer = line.to_string();
        self.multiline_indent = Self::calculate_indent(line);
        self.brace_balance = Self::count_braces(line);
        println!("      (enter .break to cancel)");
    }

    fn handle_multiline_input(&mut self, line: &str) {
        if line == ".break" || line == ".cancel" {
            self.cancel_multiline();
            return;
        }

        self.multiline_buffer.push('\n');
        self.multiline_buffer.push_str(line);
        self.multiline_indent = Self::calculate_indent(line);

        let line_balance = Self::count_braces(line);
        self.brace_balance += line_balance;

        if self.brace_balance <= 0 && !line.trim().is_empty() {
            self.execute_multiline();
        } else {
            print!("{}", self.get_prompt());
            io::stdout().flush().unwrap();
        }
    }

    fn execute_multiline(&mut self) {
        self.is_multiline = false;
        self.multiline_indent = 0;
        let code = self.multiline_buffer.clone();
        self.multiline_buffer.clear();

        if self.highlighting_enabled {
            let highlighted = self.highlighter.highlight(&code);
            println!("{}", highlighted);
        } else {
            println!("{}", code);
        }

        self.execute_code(&code).ok();
    }

    fn cancel_multiline(&mut self) {
        self.is_multiline = false;
        self.multiline_buffer.clear();
        self.multiline_indent = 0;
        self.brace_balance = 0;
        println!("(cancelled)");
    }

    fn calculate_indent(line: &str) -> usize {
        let trimmed = line.trim_start();
        let leading_spaces = line.len() - trimmed.len();
        let extra_indent = if trimmed.ends_with('{')
            || trimmed.ends_with("then")
            || trimmed.ends_with("do")
            || trimmed.ends_with("catch")
            || trimmed.ends_with("finally")
            || trimmed.ends_with("try")
        {
            4
        } else if trimmed.ends_with("else") || trimmed.ends_with("elsif") {
            if trimmed.starts_with("els") {
                4
            } else {
                0
            }
        } else {
            0
        };
        leading_spaces + extra_indent
    }

    fn count_braces(s: &str) -> i32 {
        let mut balance = 0;
        let mut in_string = false;
        let mut escaped = false;

        for c in s.chars() {
            if in_string {
                if escaped {
                    escaped = false;
                } else if c == '\\' {
                    escaped = true;
                } else if c == '"' {
                    in_string = false;
                }
            } else if c == '"' {
                in_string = true;
                escaped = false;
            } else if c == '{' {
                balance += 1;
            } else if c == '}' {
                balance -= 1;
            }
        }
        balance
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
            ".break" | ".cancel" => {
                if self.is_multiline {
                    self.cancel_multiline();
                } else {
                    println!("Not in multi-line mode.");
                }
            }
            ".last" => self.cmd_last(),
            ".theme" => self.cmd_theme(),
            ".highlight" | ".highlight on" => {
                self.highlighting_enabled = true;
                println!("Syntax highlighting enabled.");
            }
            ".highlight off" => {
                self.highlighting_enabled = false;
                println!("Syntax highlighting disabled.");
            }
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
        println!(".break         - Cancel multi-line input");
        println!(".load <file>   - Load and execute a file");
        println!(".save <file>   - Save session to a file");
        println!("? <expr>       - Inspect an expression");
        println!(".theme         - Cycle through syntax highlighting themes");
        println!(".highlight on/off - Enable/disable syntax highlighting");
        println!("exit / Ctrl+D  - Exit the REPL");
        println!();
    }

    fn cmd_theme(&mut self) {
        self.highlighter.toggle_theme();
        println!(
            "Theme: {} (use .theme to cycle)",
            self.highlighter.current_theme_name()
        );
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
        match env.borrow().get(expr) {
            Some(value) => {
                println!("{}: {:?}", expr, value);
            }
            _ => {
                println!("Error: '{}' not found in scope", expr);
            }
        }
    }

    fn execute_single(&mut self, line: &str) {
        if self.highlighting_enabled {
            let highlighted = self.highlighter.highlight(line);
            println!("{}", highlighted);
        } else {
            println!("{}", line);
        }
        if let Err(e) = self.execute(line) {
            println!("Error: {}", e);
        }
    }

    fn execute(&mut self, source: &str) -> Result<(), crate::error::SolilangError> {
        self.execute_code(source)
    }

    fn execute_code(&mut self, code: &str) -> Result<(), crate::error::SolilangError> {
        let should_print = Self::should_print_result(code);
        let trimmed = code.trim();

        let source = if should_print && !trimmed.ends_with('}') && !trimmed.ends_with(';') {
            format!("print({});", trimmed)
        } else if !trimmed.ends_with(';')
            && !trimmed.ends_with('}')
            && !trimmed.starts_with("let ")
            && !trimmed.starts_with("fn ")
            && !trimmed.starts_with("class ")
            && !trimmed.starts_with("const ")
        {
            format!("{};", trimmed)
        } else {
            code.to_string()
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

impl Default for SimpleRepl {
    fn default() -> Self {
        Self::new()
    }
}

pub fn run_simple_repl() {
    let mut repl = SimpleRepl::new();
    repl.run();
}
