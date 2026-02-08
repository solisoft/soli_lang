use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::time::Duration;

use crossterm::{
    cursor::{self, Show},
    event::{self, Event, KeyCode, KeyModifiers},
    terminal::{disable_raw_mode, enable_raw_mode, size, Clear, ClearType},
    QueueableCommand,
};

use gag::BufferRedirect;

use crate::interpreter::Interpreter;
use crate::lexer::token::TokenKind;
use crate::lexer::Scanner;
use crate::parser::Parser;

const HISTORY_FILE: &str = ".soli_history";

struct LineBuffer {
    chars: Vec<char>,
    cursor: usize,
}

impl LineBuffer {
    fn new() -> Self {
        Self {
            chars: Vec::new(),
            cursor: 0,
        }
    }

    fn is_empty(&self) -> bool {
        self.chars.is_empty()
    }

    fn as_string(&self) -> String {
        self.chars.iter().collect()
    }

    fn insert(&mut self, c: char) {
        self.chars.insert(self.cursor, c);
        self.cursor += 1;
    }

    fn delete(&mut self) {
        if self.cursor < self.chars.len() {
            self.chars.remove(self.cursor);
        }
    }

    fn backspace(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
            self.chars.remove(self.cursor);
        }
    }

    fn move_left(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
        }
    }

    fn move_right(&mut self) {
        if self.cursor < self.chars.len() {
            self.cursor += 1;
        }
    }

    fn move_to_start(&mut self) {
        self.cursor = 0;
    }

    fn move_to_end(&mut self) {
        self.cursor = self.chars.len();
    }
}

#[derive(Clone)]
enum OutputLine {
    Input(String),  // User input (with syntax highlighting)
    Result(String), // Execution result (green)
    Error(String),  // Error message (red)
    Info(String),   // Info/system message (dim)
}

struct InputState {
    line: LineBuffer,
    multiline_buffer: String,
    multiline_indent: usize,
    is_multiline: bool,
    brace_balance: i32,
    history: Vec<String>,
    history_index: usize,
    output_lines: Vec<OutputLine>,
}

impl InputState {
    fn new() -> Self {
        Self {
            line: LineBuffer::new(),
            multiline_buffer: String::new(),
            multiline_indent: 0,
            is_multiline: false,
            brace_balance: 0,
            history: Vec::new(),
            history_index: 0,
            output_lines: Vec::new(),
        }
    }

    fn add_input(&mut self, text: &str) {
        self.output_lines.push(OutputLine::Input(text.to_string()));
        self.trim_output();
    }

    fn add_result(&mut self, text: &str) {
        self.output_lines.push(OutputLine::Result(text.to_string()));
        self.trim_output();
    }

    fn add_error(&mut self, text: &str) {
        self.output_lines.push(OutputLine::Error(text.to_string()));
        self.trim_output();
    }

    fn add_info(&mut self, text: &str) {
        self.output_lines.push(OutputLine::Info(text.to_string()));
        self.trim_output();
    }

    fn trim_output(&mut self) {
        if self.output_lines.len() > 100 {
            self.output_lines.remove(0);
        }
    }

    fn detect_multiline_needed(&self) -> bool {
        let line = self.line.as_string();
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

    fn enter_multiline(&mut self) {
        self.is_multiline = true;
        self.multiline_buffer = self.line.as_string();
        self.multiline_indent = Self::calculate_indent(&self.multiline_buffer);
        self.brace_balance = Self::count_braces(&self.multiline_buffer);
        self.line = LineBuffer::new();
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

    fn add_to_history(&mut self, line: &str) {
        if !line.trim().is_empty() && self.history.last().map(|l| l != line).unwrap_or(true) {
            self.history.push(line.to_string());
        }
        self.history_index = self.history.len();
    }

    fn history_up(&mut self) {
        if !self.history.is_empty() && self.history_index > 0 {
            if self.history_index == self.history.len() {
                self.history.push(self.line.as_string());
            }
            self.history_index -= 1;
            let hist_line = &self.history[self.history_index];
            self.line = LineBuffer::new();
            for c in hist_line.chars() {
                self.line.insert(c);
            }
        }
    }

    fn history_down(&mut self) {
        if !self.history.is_empty() && self.history_index < self.history.len() - 1 {
            self.history_index += 1;
            let hist_line = &self.history[self.history_index];
            self.line = LineBuffer::new();
            for c in hist_line.chars() {
                self.line.insert(c);
            }
        }
    }
}

struct TuiRepl {
    interpreter: Interpreter,
    history_file: PathBuf,
    input: InputState,
    highlighting_enabled: bool,
}

impl TuiRepl {
    fn new() -> Self {
        colored::control::set_override(true);
        let history_file = if let Some(home) = dirs::home_dir() {
            home.join(HISTORY_FILE)
        } else {
            PathBuf::from(HISTORY_FILE)
        };

        let mut repl = Self {
            interpreter: Interpreter::new(),
            history_file,
            input: InputState::new(),
            highlighting_enabled: true,
        };
        repl.load_history();
        repl
    }

    fn load_history(&mut self) {
        if let Ok(content) = std::fs::read_to_string(&self.history_file) {
            for line in content.lines() {
                if !line.trim().is_empty() {
                    self.input.history.push(line.to_string());
                }
            }
            self.input.history_index = self.input.history.len();
        }
    }

    fn save_history(&self) {
        if let Some(parent) = self.history_file.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let content = self.input.history.join("\n");
        let _ = std::fs::write(&self.history_file, content);
    }

    pub fn run(&mut self) -> io::Result<()> {
        // Setup terminal
        enable_raw_mode()?;
        let mut stdout = io::stdout();

        // Hide cursor during drawing
        stdout.queue(cursor::Hide)?;
        stdout.queue(Clear(ClearType::All))?;
        stdout.queue(cursor::MoveTo(0, 0))?;
        stdout.flush()?;

        self.input.add_info("Soli - TUI REPL");
        self.input.add_info("Type .help for available commands.");
        self.input.add_info("");

        self.draw()?;

        loop {
            if event::poll(Duration::from_millis(50))? {
                if let Event::Key(key) = event::read()? {
                    if self.handle_key(key) {
                        break;
                    }
                    self.draw()?;
                }
            }
        }

        // Cleanup
        disable_raw_mode()?;
        let mut stdout = io::stdout();
        stdout.queue(Clear(ClearType::All))?;
        stdout.queue(cursor::MoveTo(0, 0))?;
        stdout.queue(Show)?;
        stdout.flush()?;

        println!("Goodbye!");
        self.save_history();
        Ok(())
    }

    fn draw(&self) -> io::Result<()> {
        let mut stdout = io::stdout();

        // Hide cursor before redrawing
        stdout.queue(cursor::Hide)?;
        stdout.flush()?;

        // Get terminal size first
        let (cols, rows) = size()?;
        let cols = cols as usize;
        let rows = rows as usize;

        // Reserve space for input at bottom (3 lines minimum)
        let input_area_height = if self.input.is_multiline { 10 } else { 3 };
        let output_height = rows.saturating_sub(input_area_height + 1); // +1 for separator

        // Move to top-left and clear from there
        stdout.queue(cursor::MoveTo(0, 0))?;
        stdout.queue(Clear(ClearType::FromCursorDown))?;
        stdout.flush()?;

        // Draw output area (stick to bottom, just above separator)
        let num_lines = self.input.output_lines.len().min(output_height);
        let start_idx = self.input.output_lines.len().saturating_sub(output_height);
        let start_row = output_height.saturating_sub(num_lines);

        for (i, idx) in (start_idx..self.input.output_lines.len()).enumerate() {
            let output_line = &self.input.output_lines[idx];
            let row = (start_row + i) as u16;
            stdout.queue(cursor::MoveTo(0, row))?;

            let formatted = match output_line {
                OutputLine::Input(text) => {
                    // Input with syntax highlighting and >>> prefix
                    let highlighted = self.highlight_code(text);
                    format!("\x1b[90m>>>\x1b[0m {}", highlighted)
                }
                OutputLine::Result(text) => {
                    // Result in green with => prefix
                    format!("\x1b[92m=> {}\x1b[0m", text)
                }
                OutputLine::Error(text) => {
                    // Error in red
                    format!("\x1b[91m{}\x1b[0m", text)
                }
                OutputLine::Info(text) => {
                    // Info in dim gray
                    format!("\x1b[90m{}\x1b[0m", text)
                }
            };

            // Truncate if needed (accounting for ANSI codes is complex, so we just write it)
            stdout.write_all(formatted.as_bytes())?;
        }

        // Calculate separator row
        let separator_row = output_height as u16;

        // Draw separator line
        stdout.queue(cursor::MoveTo(0, separator_row))?;
        let sep = "─".repeat(cols);
        stdout.write_all(sep.as_bytes())?;

        let input_text = self.input.line.as_string();
        let cursor_pos = self.input.line.cursor;

        // Track the row where input starts
        let input_start_row = (output_height + 1) as u16;

        if self.input.is_multiline {
            // Show multiline buffer with prompts
            let multiline_lines: Vec<&str> = self.input.multiline_buffer.lines().collect();
            let mut current_row = input_start_row;

            // Draw the multiline buffer lines with prompts
            for (i, line) in multiline_lines.iter().enumerate() {
                let prompt = if i == 0 { ">>> " } else { "... " };
                stdout.queue(cursor::MoveTo(0, current_row))?;
                stdout.write_all(prompt.as_bytes())?;
                let highlighted = self.highlight_code(line);
                stdout.write_all(highlighted.as_bytes())?;
                current_row += 1;
            }

            // Draw current input line with continuation prompt
            stdout.queue(cursor::MoveTo(0, current_row))?;
            let cont_prompt = "... ";
            stdout.write_all(cont_prompt.as_bytes())?;
            let highlighted = self.highlight_code(&input_text);
            stdout.write_all(highlighted.as_bytes())?;

            // Position cursor on the current input line
            let cursor_col = cont_prompt.len() + cursor_pos;
            stdout.queue(cursor::MoveTo(cursor_col as u16, current_row))?;
        } else {
            // Single line mode
            stdout.queue(cursor::MoveTo(0, input_start_row))?;
            let prompt = ">>> ";
            stdout.write_all(prompt.as_bytes())?;

            let highlighted = self.highlight_code(&input_text);
            stdout.write_all(highlighted.as_bytes())?;

            // Position cursor
            let cursor_col = prompt.len() + cursor_pos;
            stdout.queue(cursor::MoveTo(cursor_col as u16, input_start_row))?;
        }

        // Show the cursor at final position
        stdout.queue(cursor::Show)?;
        stdout.flush()?;
        Ok(())
    }

    fn highlight_code(&self, code: &str) -> String {
        if !self.highlighting_enabled {
            return code.to_string();
        }

        let mut scanner = Scanner::new(code);
        match scanner.scan_tokens() {
            Ok(tokens) => {
                let mut result = String::new();
                let mut last_end = 0;

                for token in tokens {
                    if token.kind == TokenKind::Eof {
                        break;
                    }

                    // Add whitespace/content between tokens
                    if token.span.start > last_end {
                        result.push_str(&code[last_end..token.span.start]);
                    }

                    // Get token text and apply color
                    let token_text = &code[token.span.start..token.span.end];
                    let colored = self.colorize_token(&token.kind, token_text);
                    result.push_str(&colored);

                    last_end = token.span.end;
                }

                // Add remaining text
                if last_end < code.len() {
                    result.push_str(&code[last_end..]);
                }

                result
            }
            Err(_) => code.to_string(),
        }
    }

    fn colorize_token(&self, kind: &TokenKind, text: &str) -> String {
        use TokenKind::*;

        match kind {
            IntLiteral(_) | FloatLiteral(_) => {
                format!("\x1b[94m{}\x1b[0m", text) // Bright blue
            }
            StringLiteral(_) | InterpolatedString(_) => {
                format!("\x1b[92m{}\x1b[0m", text) // Bright green
            }
            BoolLiteral(_) => {
                format!("\x1b[95m{}\x1b[0m", text) // Bright magenta
            }
            Null => {
                format!("\x1b[96m{}\x1b[0m", text) // Cyan
            }
            Let | Const | Fn | Return | If | Else | Elsif | While | For | In | Class | Extends
            | Implements | Interface | New | This | Super | Public | Private | Protected
            | Static | Try | Catch | Finally | Throw | Not | Async | Await | Match | Case
            | When | End | Unless | Import | Export | From | As | Int | Float | Bool | String
            | Void => {
                format!("\x1b[1;93m{}\x1b[0m", text) // Bright yellow bold
            }
            Plus | Minus | Star | Slash | Percent | Equal | EqualEqual | BangEqual | Less
            | LessEqual | Greater | GreaterEqual | Bang | And | Or | Pipeline | Pipe
            | NullishCoalescing | DoubleColon | Arrow | FatArrow | Spread | Range => {
                format!("\x1b[91m{}\x1b[0m", text) // Bright red
            }
            LeftParen | RightParen | LeftBrace | RightBrace | LeftBracket | RightBracket
            | Comma | Dot | Colon | Semicolon | Question => {
                format!("\x1b[1;97m{}\x1b[0m", text) // Bright white bold
            }
            Identifier(_) => {
                format!("\x1b[37m{}\x1b[0m", text) // White
            }
            _ => text.to_string(),
        }
    }

    fn handle_key(&mut self, key: event::KeyEvent) -> bool {
        match key.code {
            KeyCode::Char(c) => {
                if key.modifiers.contains(KeyModifiers::CONTROL) {
                    match c {
                        'd' => {
                            if self.input.line.is_empty() {
                                self.save_history();
                                return true;
                            }
                        }
                        'c' => {
                            self.input.line = LineBuffer::new();
                            self.input.add_info("^C");
                        }
                        _ => {}
                    }
                } else {
                    self.input.line.insert(c);
                }
            }
            KeyCode::Backspace => {
                self.input.line.backspace();
            }
            KeyCode::Delete => {
                self.input.line.delete();
            }
            KeyCode::Enter => {
                if key.modifiers.contains(KeyModifiers::SHIFT) {
                    self.input.line.insert('\n');
                } else {
                    self.execute_current_line();
                }
            }
            KeyCode::Left => {
                self.input.line.move_left();
            }
            KeyCode::Right => {
                self.input.line.move_right();
            }
            KeyCode::Up => {
                if !self.input.is_multiline {
                    self.input.history_up();
                }
            }
            KeyCode::Down => {
                if !self.input.is_multiline {
                    self.input.history_down();
                }
            }
            KeyCode::Home => {
                self.input.line.move_to_start();
            }
            KeyCode::End => {
                self.input.line.move_to_end();
            }
            KeyCode::Tab => {
                self.input.line.insert(' ');
                self.input.line.insert(' ');
                self.input.line.insert(' ');
                self.input.line.insert(' ');
            }
            KeyCode::Esc => {
                self.save_history();
                return true;
            }
            _ => {}
        }
        false
    }

    fn execute_current_line(&mut self) {
        let line = self.input.line.as_string();
        let trimmed = line.trim();

        if trimmed.is_empty() && !self.input.is_multiline {
            self.input.line = LineBuffer::new();
            return;
        }

        if self.input.is_multiline {
            self.input.multiline_buffer.push('\n');
            self.input.multiline_buffer.push_str(&line);
            self.input.multiline_indent = InputState::calculate_indent(&line);
            let line_balance = InputState::count_braces(&line);
            self.input.brace_balance += line_balance;

            if self.input.brace_balance <= 0 && !trimmed.is_empty() {
                self.execute_multiline();
            } else {
                self.input.line = LineBuffer::new();
            }
        } else {
            self.input.add_to_history(&line);

            if trimmed == "exit" || trimmed == ".exit" || trimmed == "quit" || trimmed == ".quit" {
                self.save_history();
                return;
            }

            if trimmed == ".help" || trimmed == "?" {
                self.show_help();
            } else if trimmed.starts_with(".theme") {
                self.input
                    .add_info("Theme: default (themes not implemented in TUI)");
            } else if trimmed == ".highlight" || trimmed == ".highlight on" {
                self.highlighting_enabled = true;
                self.input.add_info("Syntax highlighting enabled.");
            } else if trimmed == ".highlight off" {
                self.highlighting_enabled = false;
                self.input.add_info("Syntax highlighting disabled.");
            } else if trimmed == ".vars" || trimmed == ".variables" {
                self.show_vars();
            } else if trimmed == ".funcs" || trimmed == ".functions" {
                self.show_funcs();
            } else if trimmed == ".classes" {
                self.show_classes();
            } else if trimmed == ".history" || trimmed == ".hist" {
                self.show_history();
            } else if trimmed == ".clear" || trimmed == ".reset" {
                self.clear_environment();
            } else if trimmed == ".break" || trimmed == ".cancel" {
                self.input.add_info("Not in multi-line mode.");
            } else if self.input.detect_multiline_needed() {
                self.input.enter_multiline();
                self.input.add_info("      (enter .break to cancel)");
            } else {
                // Execute and show input + result
                self.input.add_input(&line);
                self.execute_code(&line);
            }

            self.input.line = LineBuffer::new();
        }
    }

    fn execute_multiline(&mut self) {
        let code = self.input.multiline_buffer.clone();
        self.input.multiline_buffer.clear();
        self.input.is_multiline = false;
        self.input.multiline_indent = 0;
        self.input.brace_balance = 0;

        // Show the executed code as input
        self.input.add_input(&code);

        self.execute_code(&code);
    }

    fn execute_code(&mut self, code: &str) {
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

        // Capture stdout during execution
        match BufferRedirect::stdout() { Ok(mut buf) => {
            let exec_result = self.run_interpreter(&source);

            // Read captured output
            let mut output = String::new();
            buf.read_to_string(&mut output).ok();
            drop(buf);

            // Add captured output to display as results
            for line in output.lines() {
                self.input.add_result(line);
            }

            if let Err(e) = exec_result {
                self.input.add_error(&e);
            }
        } _ => {
            // Fallback if stdout capture fails
            if let Err(e) = self.run_interpreter(&source) {
                self.input.add_error(&e);
            }
        }}
    }

    fn run_interpreter(&mut self, source: &str) -> Result<(), String> {
        match Scanner::new(source).scan_tokens() {
            Ok(tokens) => match Parser::new(tokens).parse() {
                Ok(program) => self
                    .interpreter
                    .interpret(&program)
                    .map_err(|e| e.to_string()),
                Err(e) => Err(format!("Parse Error: {}", e)),
            },
            Err(e) => Err(format!("Lex Error: {}", e)),
        }
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

    fn show_help(&mut self) {
        self.input.add_info("");
        self.input.add_info("Soli REPL Commands");
        self.input
            .add_info(".help          - Show this help message");
        self.input
            .add_info(".vars          - List all variables in current scope");
        self.input
            .add_info(".funcs         - List all defined functions");
        self.input
            .add_info(".classes       - List all defined classes");
        self.input.add_info(".history       - Show command history");
        self.input
            .add_info(".clear         - Reset the REPL environment");
        self.input
            .add_info(".highlight on/off - Enable/disable syntax highlighting");
        self.input.add_info("exit / Ctrl+D  - Exit the REPL");
        self.input.add_info("Esc            - Exit the REPL");
        self.input.add_info("Arrow Up/Down  - History navigation");
        self.input.add_info("Ctrl+C         - Cancel input");
        self.input
            .add_info("Shift+Enter    - New line in multi-line mode");
        self.input.add_info("");
    }

    fn show_vars(&mut self) {
        let env = self.interpreter.global_env();
        let vars = env.borrow().get_var_names();
        if vars.is_empty() {
            self.input.add_info("No variables defined.");
        } else {
            self.input.add_info("Variables:");
            for name in vars {
                self.input.add_info(&format!("  {}", name));
            }
        }
    }

    fn show_funcs(&mut self) {
        let env = self.interpreter.global_env();
        let var_names = env.borrow().get_var_names();
        let funcs: Vec<_> = var_names
            .iter()
            .filter(|k| k.starts_with("__func_"))
            .collect();
        if funcs.is_empty() {
            self.input.add_info("No functions defined.");
        } else {
            self.input.add_info("Functions:");
            for name in funcs {
                let clean_name = name.strip_prefix("__func_").unwrap_or(name);
                self.input.add_info(&format!("  {}", clean_name));
            }
        }
    }

    fn show_classes(&mut self) {
        let env = self.interpreter.global_env();
        let var_names = env.borrow().get_var_names();
        let classes: Vec<_> = var_names
            .iter()
            .filter(|k| k.starts_with("__class_"))
            .collect();
        if classes.is_empty() {
            self.input.add_info("No classes defined.");
        } else {
            self.input.add_info("Classes:");
            for name in classes {
                let clean_name = name.strip_prefix("__class_").unwrap_or(name);
                self.input.add_info(&format!("  {}", clean_name));
            }
        }
    }

    fn show_history(&mut self) {
        self.input.add_info("History:");
        let history: Vec<String> = self
            .input
            .history
            .iter()
            .enumerate()
            .map(|(i, entry)| format!("{:4}  {}", i + 1, entry))
            .collect();
        for entry in history {
            self.input.add_info(&entry);
        }
    }

    fn clear_environment(&mut self) {
        self.interpreter = Interpreter::new();
        self.input.add_info("Environment reset.");
    }
}

pub fn run_tui_repl() -> io::Result<()> {
    let mut repl = TuiRepl::new();
    repl.run()
}
