use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::time::Duration;

use crossterm::{
    cursor::{self, Show},
    event::{self, DisableBracketedPaste, EnableBracketedPaste, Event, KeyCode, KeyModifiers},
    terminal::{disable_raw_mode, enable_raw_mode, size, Clear, ClearType},
    QueueableCommand,
};

use gag::BufferRedirect;

use crate::interpreter::{Interpreter, Value};
use crate::lexer::token::TokenKind;
use crate::lexer::Scanner;
use crate::parser::Parser;
use crate::repl_common;

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
        repl_common::detect_multiline_needed(&self.line.as_string())
    }

    fn enter_multiline(&mut self) {
        self.is_multiline = true;
        self.multiline_buffer = self.line.as_string();
        self.multiline_indent = repl_common::calculate_indent(&self.multiline_buffer);
        self.brace_balance = repl_common::count_block_balance(&self.multiline_buffer);
        self.line = LineBuffer::new();
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

struct CompletionState {
    candidates: Vec<String>,
    index: usize,
    replacement_start: usize,
    active: bool,
}

impl CompletionState {
    fn new() -> Self {
        Self {
            candidates: Vec::new(),
            index: 0,
            replacement_start: 0,
            active: false,
        }
    }

    fn reset(&mut self) {
        self.candidates.clear();
        self.index = 0;
        self.replacement_start = 0;
        self.active = false;
    }
}

fn methods_for_type(type_name: &str) -> Vec<&'static str> {
    crate::interpreter::executor::calls::method_registry::known_methods(type_name)
        .iter()
        .map(|m| m.name)
        .collect()
}

struct TuiRepl {
    interpreter: Interpreter,
    history_file: PathBuf,
    input: InputState,
    highlighting_enabled: bool,
    completion: CompletionState,
}

impl TuiRepl {
    fn new() -> Self {
        colored::control::set_override(true);
        let history_file = if let Some(home) = dirs::home_dir() {
            home.join(HISTORY_FILE)
        } else {
            PathBuf::from(HISTORY_FILE)
        };

        let mut interpreter = Interpreter::new();

        // Auto-load models if running inside an MVC app directory
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let models_dir = cwd.join("app").join("models");
        if models_dir.is_dir() {
            // Load .env files for DB configuration
            crate::serve::env_loader::load_env_files(&cwd);
            crate::interpreter::builtins::model::init_db_config();

            if let Err(e) = crate::serve::app_loader::load_models(&mut interpreter, &models_dir) {
                eprintln!("Warning: Failed to load models: {}", e);
            }
        }

        let mut repl = Self {
            interpreter,
            history_file,
            input: InputState::new(),
            highlighting_enabled: true,
            completion: CompletionState::new(),
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

        // Enable bracketed paste so multi-line pastes arrive as a single event
        stdout.queue(EnableBracketedPaste)?;

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
                match event::read()? {
                    Event::Key(key) => {
                        if self.handle_key(key) {
                            break;
                        }
                        self.draw()?;
                    }
                    Event::Paste(text) => {
                        self.handle_paste(&text);
                        self.draw()?;
                    }
                    _ => {}
                }
            }
        }

        // Cleanup
        disable_raw_mode()?;
        let mut stdout = io::stdout();
        stdout.queue(DisableBracketedPaste)?;
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
                    // Result in green: => prefix only on first result line
                    let is_first = idx == 0
                        || !matches!(self.input.output_lines[idx - 1], OutputLine::Result(_));
                    if is_first {
                        format!("\x1b[92m=> {}\x1b[0m", text)
                    } else {
                        format!("\x1b[92m   {}\x1b[0m", text)
                    }
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

        let (final_row, final_col) = if self.input.is_multiline {
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

            let cursor_col = cont_prompt.len() + cursor_pos;
            (current_row, cursor_col as u16)
        } else {
            // Single line mode
            stdout.queue(cursor::MoveTo(0, input_start_row))?;
            let prompt = ">>> ";
            stdout.write_all(prompt.as_bytes())?;

            let highlighted = self.highlight_code(&input_text);
            stdout.write_all(highlighted.as_bytes())?;

            let cursor_col = prompt.len() + cursor_pos;
            (input_start_row, cursor_col as u16)
        };

        // Draw completion popup if active
        if self.completion.active && !self.completion.candidates.is_empty() {
            let hint_row = final_row + 1;
            if (hint_row as usize) < rows {
                stdout.queue(cursor::MoveTo(0, hint_row))?;
                let mut hint = String::new();
                let mut total_len = 0;
                for (i, candidate) in self.completion.candidates.iter().enumerate() {
                    let entry = if i == self.completion.index {
                        format!("\x1b[1;7m {candidate} \x1b[0m")
                    } else {
                        format!("\x1b[90m {candidate} \x1b[0m")
                    };
                    // Check if adding this entry would overflow terminal width
                    // (candidate.len() + 2 for surrounding spaces)
                    let visible_len = candidate.len() + 2;
                    if total_len + visible_len > cols.saturating_sub(4) && i > 0 {
                        hint.push_str("\x1b[90m ...\x1b[0m");
                        break;
                    }
                    hint.push_str(&entry);
                    total_len += visible_len;
                }
                stdout.write_all(hint.as_bytes())?;
            }
        }

        // Restore cursor to input position
        stdout.queue(cursor::MoveTo(final_col, final_row))?;
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
            | NullishCoalescing | SafeNavigation | DoubleColon | Arrow | FatArrow | Spread
            | Range => {
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
        if key.code != KeyCode::Tab {
            self.completion.reset();
        }
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
                self.handle_tab();
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
            self.input.multiline_indent = repl_common::calculate_indent(&line);
            let line_balance = repl_common::count_block_balance(&line);
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
        self.input.line = LineBuffer::new();

        // Show the executed code as input
        self.input.add_input(&code);

        self.execute_code(&code);
    }

    fn handle_paste(&mut self, text: &str) {
        let lines: Vec<&str> = text.lines().collect();
        if lines.is_empty() {
            return;
        }

        // Single-line paste: just insert into the current line buffer
        if lines.len() == 1 {
            for c in lines[0].chars() {
                self.input.line.insert(c);
            }
            return;
        }

        // Multi-line paste: compute block balance over the whole pasted text
        // and execute as a single unit
        let full_code = if self.input.is_multiline {
            // Already in multiline mode — append pasted text to buffer
            let mut buf = self.input.multiline_buffer.clone();
            for line in &lines {
                buf.push('\n');
                buf.push_str(line);
            }
            // Recompute balance for the whole buffer
            let balance: i32 = buf.lines().map(repl_common::count_block_balance).sum();
            if balance <= 0 {
                self.input.multiline_buffer = buf;
                self.execute_multiline();
                return;
            }
            // Still unbalanced — keep in multiline mode
            self.input.multiline_buffer = buf;
            self.input.brace_balance = balance;
            self.input.line = LineBuffer::new();
            return;
        } else {
            // Not yet in multiline mode — check if paste is self-contained
            let balance: i32 = lines
                .iter()
                .map(|l| repl_common::count_block_balance(l))
                .sum();
            if balance <= 0 {
                // Self-contained block — execute directly
                text.to_string()
            } else {
                // Incomplete block — enter multiline mode with the pasted text
                self.input.is_multiline = true;
                self.input.multiline_buffer = text.to_string();
                self.input.brace_balance = balance;
                self.input.multiline_indent =
                    repl_common::calculate_indent(lines.last().unwrap_or(&""));
                self.input.line = LineBuffer::new();
                return;
            }
        };

        // Execute the complete pasted code
        self.input.add_to_history(&full_code);
        self.input.add_input(&full_code);
        self.execute_code(&full_code);
        self.input.line = LineBuffer::new();
    }

    fn execute_code(&mut self, code: &str) {
        let source = repl_common::prepare_source(code);

        // Capture stdout during execution
        match BufferRedirect::stdout() {
            Ok(mut buf) => {
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
            }
            _ => {
                // Fallback if stdout capture fails
                if let Err(e) = self.run_interpreter(&source) {
                    self.input.add_error(&e);
                }
            }
        }
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

    fn handle_tab(&mut self) {
        if self.completion.active {
            // Cycle to next candidate
            self.completion.index = (self.completion.index + 1) % self.completion.candidates.len();
            let candidate = self.completion.candidates[self.completion.index].clone();
            // Replace text from replacement_start to current cursor
            self.input
                .line
                .chars
                .truncate(self.completion.replacement_start);
            self.input.line.cursor = self.completion.replacement_start;
            for c in candidate.chars() {
                self.input.line.insert(c);
            }
            return;
        }

        let cursor = self.input.line.cursor;
        // At start of line or after whitespace → indent
        if cursor == 0
            || self
                .input
                .line
                .chars
                .get(cursor - 1)
                .is_none_or(|c| c.is_whitespace())
        {
            for _ in 0..4 {
                self.input.line.insert(' ');
            }
            return;
        }

        let (candidates, replacement_start) = self.compute_completions();
        match candidates.len() {
            0 => {
                for _ in 0..4 {
                    self.input.line.insert(' ');
                }
            }
            1 => {
                let candidate = candidates[0].clone();
                self.input.line.chars.truncate(replacement_start);
                self.input.line.cursor = replacement_start;
                for c in candidate.chars() {
                    self.input.line.insert(c);
                }
            }
            _ => {
                let candidate = candidates[0].clone();
                self.input.line.chars.truncate(replacement_start);
                self.input.line.cursor = replacement_start;
                for c in candidate.chars() {
                    self.input.line.insert(c);
                }
                self.completion.candidates = candidates;
                self.completion.index = 0;
                self.completion.replacement_start = replacement_start;
                self.completion.active = true;
            }
        }
    }

    fn compute_completions(&self) -> (Vec<String>, usize) {
        let text: String = self.input.line.chars[..self.input.line.cursor]
            .iter()
            .collect();

        // Find last dot that's not inside a string
        if let Some(dot_pos) = self.find_last_dot(&text) {
            let before_dot = text[..dot_pos].trim();
            let after_dot = &text[dot_pos + 1..];

            // Validate after_dot: empty or starts with alpha/underscore
            if !after_dot.is_empty()
                && !after_dot
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_alphabetic() || c == '_')
            {
                return (vec![], 0);
            }

            if let Some(type_name) = self.detect_type_of_expr(before_dot) {
                let methods = self.methods_for_resolved_type(&type_name, before_dot);
                let prefix = after_dot.to_lowercase();
                let filtered: Vec<String> = methods
                    .into_iter()
                    .filter(|m| m.starts_with(&prefix))
                    .map(|m| m.to_string())
                    .collect();
                // replacement_start is the char position after the dot
                let replacement_start_chars = dot_pos + 1;
                // Convert byte offset to char offset
                let char_offset = text[..replacement_start_chars].chars().count();
                return (filtered, char_offset);
            }
        }

        // No dot context → identifier completion
        self.get_identifier_completions(&text)
    }

    fn find_last_dot(&self, text: &str) -> Option<usize> {
        let mut in_single_quote = false;
        let mut in_double_quote = false;
        let mut last_dot = None;
        let mut prev = '\0';

        for (i, c) in text.char_indices() {
            match c {
                '\'' if !in_double_quote && prev != '\\' => in_single_quote = !in_single_quote,
                '"' if !in_single_quote && prev != '\\' => in_double_quote = !in_double_quote,
                '.' if !in_single_quote && !in_double_quote => last_dot = Some(i),
                _ => {}
            }
            prev = c;
        }
        last_dot
    }

    fn detect_type_of_expr(&self, expr: &str) -> Option<String> {
        let expr = expr.trim();
        if expr.is_empty() {
            return None;
        }

        // String literals
        if (expr.starts_with('"') && expr.ends_with('"'))
            || (expr.starts_with('\'') && expr.ends_with('\''))
        {
            return Some("string".to_string());
        }

        // Bool
        if expr == "true" || expr == "false" {
            return Some("bool".to_string());
        }

        // Null
        if expr == "null" {
            return Some("null".to_string());
        }

        // Array literal
        if expr.starts_with('[') && expr.ends_with(']') {
            return Some("array".to_string());
        }

        // Hash literal
        if expr.starts_with('{') && expr.ends_with('}') {
            return Some("hash".to_string());
        }

        // Numeric: all digits → int, digits with one inner dot → float
        if expr.chars().all(|c| c.is_ascii_digit() || c == '_') && !expr.is_empty() {
            return Some("int".to_string());
        }
        if expr
            .chars()
            .all(|c| c.is_ascii_digit() || c == '.' || c == '_')
            && expr.contains('.')
            && expr.matches('.').count() == 1
        {
            return Some("float".to_string());
        }

        // Negative numbers
        if let Some(rest) = expr.strip_prefix('-') {
            if rest.chars().all(|c| c.is_ascii_digit() || c == '_') && !rest.is_empty() {
                return Some("int".to_string());
            }
            if rest
                .chars()
                .all(|c| c.is_ascii_digit() || c == '.' || c == '_')
                && rest.contains('.')
                && rest.matches('.').count() == 1
            {
                return Some("float".to_string());
            }
        }

        // Identifier → look up in environment
        if expr
            .chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == '?')
            && expr
                .chars()
                .next()
                .is_some_and(|c| c.is_alphabetic() || c == '_')
        {
            let env = self.interpreter.global_env();
            if let Some(val) = env.borrow().get(expr) {
                return Some(Self::normalize_type_name(&val.type_name()));
            }
        }

        // Chained method call: expr contains a dot → split on last dot,
        // resolve base type, then infer return type of the method
        if let Some(dot_pos) = self.find_last_dot(expr) {
            let base = expr[..dot_pos].trim();
            let method = expr[dot_pos + 1..].trim();
            if let Some(base_type) = self.detect_type_of_expr(base) {
                if let Some(ret) = self.method_return_type(&base_type, method) {
                    return Some(ret);
                }
            }
        }

        // Fallback: evaluate the expression in the live interpreter to get its type
        self.try_eval_type(expr)
    }

    /// Try to evaluate an expression and return its type name.
    /// Suppresses stdout so side-effect output doesn't leak into the TUI.
    fn try_eval_type(&self, expr: &str) -> Option<String> {
        let var = "__tab_completion_tmp__";
        let source = format!("let {} = {};", var, expr);
        let tokens = Scanner::new(&source).scan_tokens().ok()?;
        let program = Parser::new(tokens).parse().ok()?;
        let mut tmp = Interpreter::with_environment(self.interpreter.global_env().clone());
        // Suppress any stdout (e.g. println side effects)
        let _guard = gag::BufferRedirect::stdout().ok();
        tmp.interpret(&program).ok()?;
        let env = tmp.global_env();
        let val = env.borrow().get(var)?;
        let ty = Self::normalize_type_name(&val.type_name());
        // Only return types we can complete on
        if !ty.is_empty() && ty != "function" && ty != "method" {
            Some(ty)
        } else {
            None
        }
    }

    /// Normalize a type name to match registry keys (lowercase, snake_case).
    fn normalize_type_name(name: &str) -> String {
        match name {
            "QueryBuilder" => "query_builder".to_string(),
            "Class" => "class".to_string(),
            _ => name.to_lowercase(),
        }
    }

    /// Get methods for a resolved type. Tries the static registry first,
    /// then falls back to extracting methods from actual class/instance values.
    fn methods_for_resolved_type(&self, type_name: &str, expr: &str) -> Vec<String> {
        // Try static registry first
        let registry = methods_for_type(type_name);
        if !registry.is_empty() {
            return registry.into_iter().map(|s| s.to_string()).collect();
        }

        // For "class" type, look up the actual class and extract static methods
        if type_name == "class" {
            return self.class_static_methods(expr);
        }

        // For instance types (type name is the class name), look up the class
        self.class_instance_methods(type_name)
    }

    /// Extract static method names from a Class value looked up by expression.
    fn class_static_methods(&self, expr: &str) -> Vec<String> {
        let env = self.interpreter.global_env();
        let val = env.borrow().get(expr.trim());
        if let Some(Value::Class(class)) = val {
            self.collect_class_static_methods(&class)
        } else {
            Vec::new()
        }
    }

    /// Extract instance method names from a class looked up by name.
    fn class_instance_methods(&self, class_name: &str) -> Vec<String> {
        let env = self.interpreter.global_env();
        // Try PascalCase first (e.g. "user" → look up "User")
        let pascal = Self::to_pascal_case(class_name);
        let val = env
            .borrow()
            .get(&pascal)
            .or_else(|| env.borrow().get(class_name));
        if let Some(Value::Class(class)) = val {
            let mut names: Vec<String> = Vec::new();
            // User-defined instance methods
            for name in class.methods.keys() {
                names.push(name.to_string());
            }
            // Native instance methods
            for name in class.native_methods.keys() {
                names.push(name.to_string());
            }
            // Universal methods
            for m in &["class", "inspect", "is_a?", "nil?", "to_s", "to_string"] {
                if !names.iter().any(|n| n == m) {
                    names.push(m.to_string());
                }
            }
            names.sort();
            names.dedup();
            names
        } else {
            Vec::new()
        }
    }

    fn collect_class_static_methods(
        &self,
        class: &crate::interpreter::value::Class,
    ) -> Vec<String> {
        let mut names: Vec<String> = Vec::new();
        // User-defined static methods
        for name in class.static_methods.keys() {
            names.push(name.to_string());
        }
        // Native static methods (e.g. model methods: all, count, create, etc.)
        for name in class.native_static_methods.keys() {
            names.push(name.to_string());
        }
        // Universal methods
        for m in &["class", "inspect", "is_a?", "nil?"] {
            if !names.iter().any(|n| n == m) {
                names.push(m.to_string());
            }
        }
        names.sort();
        names.dedup();
        names
    }

    fn to_pascal_case(s: &str) -> String {
        let mut result = String::new();
        let mut capitalize_next = true;
        for c in s.chars() {
            if c == '_' {
                capitalize_next = true;
            } else if capitalize_next {
                result.push(c.to_ascii_uppercase());
                capitalize_next = false;
            } else {
                result.push(c);
            }
        }
        result
    }

    fn method_return_type(&self, type_name: &str, method: &str) -> Option<String> {
        crate::interpreter::executor::calls::method_registry::method_return_type(type_name, method)
            .map(|s| s.to_string())
    }

    fn get_identifier_completions(&self, text: &str) -> (Vec<String>, usize) {
        // Find word boundary (scan backwards for identifier chars)
        let chars: Vec<char> = text.chars().collect();
        let mut start = chars.len();
        while start > 0
            && (chars[start - 1].is_alphanumeric()
                || chars[start - 1] == '_'
                || chars[start - 1] == '?')
        {
            start -= 1;
        }

        let prefix: String = chars[start..].iter().collect();
        if prefix.is_empty() {
            return (vec![], 0);
        }

        let keywords = [
            "as",
            "async",
            "await",
            "break",
            "case",
            "catch",
            "class",
            "const",
            "else",
            "elsif",
            "end",
            "export",
            "extends",
            "false",
            "finally",
            "fn",
            "for",
            "from",
            "if",
            "implements",
            "import",
            "in",
            "interface",
            "let",
            "match",
            "new",
            "not",
            "null",
            "print",
            "println",
            "private",
            "protected",
            "public",
            "return",
            "static",
            "super",
            "this",
            "throw",
            "true",
            "try",
            "unless",
            "when",
            "while",
        ];

        let mut candidates: Vec<String> = Vec::new();

        // Keywords
        for kw in &keywords {
            if kw.starts_with(&prefix) && *kw != prefix {
                candidates.push(kw.to_string());
            }
        }

        // Variable names from environment (skip __ internals)
        let env = self.interpreter.global_env();
        let var_names = env.borrow().get_var_names();
        for name in &var_names {
            if name.starts_with("__") {
                continue;
            }
            if name.starts_with(&prefix) && *name != prefix {
                candidates.push(name.clone());
            }
        }

        candidates.sort();
        candidates.dedup();
        (candidates, start)
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
