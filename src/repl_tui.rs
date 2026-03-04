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
    flushed_count: usize, // how many output_lines have been flushed to scrollback
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
            flushed_count: 0,
        }
    }

    fn add_input(&mut self, text: &str) {
        for line in text.lines() {
            self.output_lines.push(OutputLine::Input(line.to_string()));
        }
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
        if self.output_lines.len() > 500 {
            self.output_lines.remove(0);
            if self.flushed_count > 0 {
                self.flushed_count -= 1;
            }
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
            self.history_index -= 1;
            let hist_line = self.history[self.history_index].clone();
            self.line = LineBuffer::new();
            for c in hist_line.chars() {
                self.line.insert(c);
            }
        }
    }

    fn history_down(&mut self) {
        if self.history_index < self.history.len().saturating_sub(1) {
            self.history_index += 1;
            let hist_line = self.history[self.history_index].clone();
            self.line = LineBuffer::new();
            for c in hist_line.chars() {
                self.line.insert(c);
            }
        } else {
            // Past the end — clear input
            self.history_index = self.history.len();
            self.line = LineBuffer::new();
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

        stdout.flush()?;

        // Print welcome message as normal scrolling output
        disable_raw_mode()?;
        println!("\x1b[90mSoli - TUI REPL\x1b[0m");
        println!("\x1b[90mType .help for available commands.\x1b[0m");
        io::stdout().flush()?;
        enable_raw_mode()?;

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
                    Event::Resize(_, _) => {
                        self.draw()?;
                    }
                    _ => {}
                }
            }
        }

        // Flush any remaining output
        let _ = self.flush_output();

        // Cleanup
        disable_raw_mode()?;
        let mut stdout = io::stdout();
        stdout.queue(DisableBracketedPaste)?;
        stdout.queue(Show)?;
        stdout.flush()?;

        println!("\nGoodbye!");
        self.save_history();
        Ok(())
    }

    /// Print output lines to terminal as normal scrolling text, then clear them.
    fn flush_output(&mut self) -> io::Result<()> {
        if self.input.output_lines.is_empty() {
            return Ok(());
        }

        let (cols, _) = size()?;
        let cols = cols as usize;
        let mut stdout = io::stdout();

        // Clear the prompt line first
        stdout.queue(cursor::Hide)?;
        stdout.write_all(b"\r")?;
        stdout.queue(Clear(ClearType::CurrentLine))?;
        stdout.flush()?;

        // Temporarily leave raw mode to print output correctly
        disable_raw_mode()?;

        for idx in 0..self.input.output_lines.len() {
            let formatted = self.format_output_line(idx, cols);
            let wrapped = Self::wrap_ansi(&formatted, cols);
            for line in wrapped {
                println!("{}", line);
            }
        }
        stdout.flush()?;
        enable_raw_mode()?;

        self.input.output_lines.clear();
        self.input.flushed_count = 0;

        Ok(())
    }

    fn draw(&mut self) -> io::Result<()> {
        let mut stdout = io::stdout();

        let (cols, rows) = size()?;
        let cols = cols as usize;
        let rows = rows as usize;

        // Get current cursor row
        let (_, current_row) = cursor::position()?;

        let input_text = self.input.line.as_string();
        let cursor_pos = self.input.line.cursor;

        // Clear from current position down
        stdout.queue(cursor::Hide)?;
        stdout.write_all(b"\r")?;
        stdout.queue(Clear(ClearType::FromCursorDown))?;

        let (final_row, final_col) = if self.input.is_multiline {
            let multiline_lines: Vec<&str> = self.input.multiline_buffer.lines().collect();
            let mut row = current_row;

            for (i, line) in multiline_lines.iter().enumerate() {
                let prompt = if i == 0 { ">>> " } else { "... " };
                stdout.queue(cursor::MoveTo(0, row))?;
                stdout.write_all(prompt.as_bytes())?;
                let highlighted = self.highlight_code(line);
                stdout.write_all(highlighted.as_bytes())?;
                row += 1;
                if row as usize >= rows {
                    break;
                }
            }

            stdout.queue(cursor::MoveTo(0, row))?;
            let cont_prompt = "... ";
            stdout.write_all(cont_prompt.as_bytes())?;
            let highlighted = self.highlight_code(&input_text);
            stdout.write_all(highlighted.as_bytes())?;

            let cursor_col = cont_prompt.len() + cursor_pos;
            (row, cursor_col as u16)
        } else {
            stdout.queue(cursor::MoveTo(0, current_row))?;
            let prompt = "\x1b[90m>>>\x1b[0m ";
            stdout.write_all(prompt.as_bytes())?;

            let highlighted = self.highlight_code(&input_text);
            stdout.write_all(highlighted.as_bytes())?;

            let cursor_col = 4 + cursor_pos; // ">>> " is 4 visible chars
            (current_row, cursor_col as u16)
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

    /// Colorize a REPL result string based on value types.
    /// Parses the inspect output and applies ANSI colors:
    /// - Strings (quoted): green
    /// - Numbers: blue
    /// - Booleans: magenta
    /// - null: cyan
    /// - Hash keys: yellow
    /// - Brackets/braces: white bold
    fn format_output_line(&self, idx: usize, _cols: usize) -> String {
        let output_line = &self.input.output_lines[idx];
        match output_line {
            OutputLine::Input(text) => {
                let highlighted = self.highlight_code(text);
                let is_first = idx == 0
                    || !matches!(self.input.output_lines[idx - 1], OutputLine::Input(_));
                if is_first {
                    format!("\x1b[90m>>>\x1b[0m {}", highlighted)
                } else {
                    format!("\x1b[90m...\x1b[0m {}", highlighted)
                }
            }
            OutputLine::Result(text) => {
                let colored = Self::colorize_result(text);
                let is_first = idx == 0
                    || !matches!(self.input.output_lines[idx - 1], OutputLine::Result(_));
                if is_first {
                    format!("\x1b[90m=>\x1b[0m {}", colored)
                } else {
                    format!("   {}", colored)
                }
            }
            OutputLine::Error(text) => {
                format!("\x1b[91m{}\x1b[0m", text)
            }
            OutputLine::Info(text) => {
                format!("\x1b[90m{}\x1b[0m", text)
            }
        }
    }

    fn colorize_result(text: &str) -> String {
        let mut result = String::new();
        let chars: Vec<char> = text.chars().collect();
        let len = chars.len();
        let mut i = 0;

        while i < len {
            match chars[i] {
                '"' => {
                    // String literal — scan to closing quote
                    let start = i;
                    i += 1;
                    while i < len && chars[i] != '"' {
                        if chars[i] == '\\' {
                            i += 1; // skip escaped char
                        }
                        i += 1;
                    }
                    if i < len {
                        i += 1; // consume closing "
                    }
                    let s: String = chars[start..i].iter().collect();
                    result.push_str(&format!("\x1b[92m{}\x1b[0m", s)); // green
                }
                c if c.is_ascii_digit() || (c == '-' && i + 1 < len && chars[i + 1].is_ascii_digit()) => {
                    // Number
                    let start = i;
                    if c == '-' {
                        i += 1;
                    }
                    while i < len && (chars[i].is_ascii_digit() || chars[i] == '.' || chars[i] == 'e' || chars[i] == 'E') {
                        i += 1;
                    }
                    let s: String = chars[start..i].iter().collect();
                    result.push_str(&format!("\x1b[94m{}\x1b[0m", s)); // blue
                }
                't' if text[i..].starts_with("true") && (i + 4 >= len || !chars[i + 4].is_alphanumeric()) => {
                    result.push_str("\x1b[95mtrue\x1b[0m"); // magenta
                    i += 4;
                }
                'f' if text[i..].starts_with("false") && (i + 5 >= len || !chars[i + 5].is_alphanumeric()) => {
                    result.push_str("\x1b[95mfalse\x1b[0m"); // magenta
                    i += 5;
                }
                'n' if text[i..].starts_with("null") && (i + 4 >= len || !chars[i + 4].is_alphanumeric()) => {
                    result.push_str("\x1b[96mnull\x1b[0m"); // cyan
                    i += 4;
                }
                '[' | ']' | '{' | '}' => {
                    result.push_str(&format!("\x1b[1;97m{}\x1b[0m", chars[i])); // white bold
                    i += 1;
                }
                ':' => {
                    result.push_str(&format!("\x1b[37m{}\x1b[0m", chars[i])); // dim white
                    i += 1;
                }
                ',' => {
                    result.push_str(&format!("\x1b[37m,\x1b[0m"));
                    i += 1;
                }
                _ => {
                    result.push(chars[i]);
                    i += 1;
                }
            }
        }

        result
    }

    /// Wrap a string containing ANSI escape codes into multiple lines of `max_cols` visible width.
    /// Continuation lines are indented to match the leading whitespace of the original line.
    /// Active ANSI color is preserved across line breaks.
    fn wrap_ansi(s: &str, max_cols: usize) -> Vec<String> {
        if max_cols == 0 {
            return vec![s.to_string()];
        }

        // Measure visible width first — if it fits, return as-is
        let visible_width = Self::visible_len(s);
        if visible_width <= max_cols {
            return vec![s.to_string()];
        }

        // Determine continuation indent: align under the value start
        // For hash entries like `  "key": "value..."`, align under the opening quote of value
        // Otherwise fall back to leading whitespace + 2
        let cont_indent_len = Self::find_value_start_pos(s)
            .unwrap_or_else(|| Self::count_leading_visible_spaces(s) + 2);
        let cont_indent = " ".repeat(cont_indent_len);
        let cont_cols = max_cols.saturating_sub(cont_indent.len());
        if cont_cols == 0 {
            return vec![s.to_string()];
        }

        let mut lines = Vec::new();
        let mut current_line = String::new();
        let mut visible = 0;
        let mut current_color = String::new(); // track last active ANSI code
        let mut is_first_line = true;
        let mut chars = s.chars().peekable();

        while let Some(c) = chars.next() {
            if c == '\x1b' {
                // Capture full ANSI sequence
                let mut seq = String::from(c);
                if let Some(&'[') = chars.peek() {
                    seq.push(chars.next().unwrap());
                    while let Some(&next) = chars.peek() {
                        seq.push(chars.next().unwrap());
                        if next.is_ascii_alphabetic() {
                            break;
                        }
                    }
                }
                // Track color state (reset clears it)
                if seq == "\x1b[0m" {
                    current_color.clear();
                } else {
                    current_color = seq.clone();
                }
                current_line.push_str(&seq);
            } else {
                let line_max = if is_first_line { max_cols } else { cont_cols };
                if visible >= line_max {
                    // Close color on current line and start a new one
                    current_line.push_str("\x1b[0m");
                    lines.push(current_line);
                    current_line = String::new();
                    // Add continuation indent and restore color
                    current_line.push_str(&cont_indent);
                    if !current_color.is_empty() {
                        current_line.push_str(&current_color);
                    }
                    visible = 0;
                    is_first_line = false;
                }
                current_line.push(c);
                visible += 1;
            }
        }

        if !current_line.is_empty() {
            lines.push(current_line);
        }

        if lines.is_empty() {
            vec![s.to_string()]
        } else {
            lines
        }
    }

    /// Count visible characters in a string with ANSI codes.
    fn visible_len(s: &str) -> usize {
        let mut count = 0;
        let mut chars = s.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '\x1b' {
                if let Some(&'[') = chars.peek() {
                    chars.next();
                    while let Some(&next) = chars.peek() {
                        chars.next();
                        if next.is_ascii_alphabetic() {
                            break;
                        }
                    }
                }
            } else {
                count += 1;
            }
        }
        count
    }

    /// Count leading visible spaces (skipping ANSI codes at the start).
    fn count_leading_visible_spaces(s: &str) -> usize {
        let mut count = 0;
        let mut chars = s.chars().peekable();
        while let Some(&c) = chars.peek() {
            if c == '\x1b' {
                chars.next();
                if let Some(&'[') = chars.peek() {
                    chars.next();
                    while let Some(&next) = chars.peek() {
                        chars.next();
                        if next.is_ascii_alphabetic() {
                            break;
                        }
                    }
                }
            } else if c == ' ' {
                count += 1;
                chars.next();
            } else {
                break;
            }
        }
        count
    }

    /// Find the visible position where a hash value starts.
    /// For lines like `  "key": "value"`, returns the position of the opening quote of the value.
    /// Returns None if the line doesn't look like a hash entry.
    fn find_value_start_pos(s: &str) -> Option<usize> {
        let mut visible_pos = 0;
        let mut chars = s.chars().peekable();
        let mut found_colon = false;

        while let Some(c) = chars.next() {
            if c == '\x1b' {
                // Skip ANSI sequence
                if let Some(&'[') = chars.peek() {
                    chars.next();
                    while let Some(&next) = chars.peek() {
                        chars.next();
                        if next.is_ascii_alphabetic() {
                            break;
                        }
                    }
                }
                continue;
            }

            if !found_colon {
                if c == ':' {
                    found_colon = true;
                }
                visible_pos += 1;
            } else {
                // After colon, skip spaces then return position of first non-space
                if c == ' ' {
                    visible_pos += 1;
                } else {
                    return Some(visible_pos);
                }
            }
        }
        None
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
                // Flush previous output before new execution
                let _ = self.flush_output();
                // Execute and show input + result
                self.input.add_input(&line);
                self.execute_code(&line);
                // Flush results to scrollback immediately
                let _ = self.flush_output();
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

        // Flush previous output before new execution
        let _ = self.flush_output();

        // Show the executed code as input
        self.input.add_input(&code);

        self.execute_code(&code);
        // Flush results to scrollback immediately
        let _ = self.flush_output();
    }

    fn handle_paste(&mut self, text: &str) {
        // Normalize line endings
        let text = text.replace("\r\n", "\n").replace('\r', "\n");
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

        // Multi-line paste: feed each line as if the user typed it + pressed Enter
        for line in &lines {
            // Set the current line buffer to this line
            self.input.line = LineBuffer::new();
            for c in line.chars() {
                self.input.line.insert(c);
            }
            // Execute as if Enter was pressed
            self.execute_current_line();
        }
    }

    fn execute_code(&mut self, code: &str) {
        let source = repl_common::prepare_source(code);

        let start = std::time::Instant::now();

        // Capture stdout during execution
        match BufferRedirect::stdout() {
            Ok(mut buf) => {
                let exec_result = self.run_interpreter(&source);
                let elapsed = start.elapsed();

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

                // Add elapsed time after result
                self.input.add_info(&Self::format_elapsed(elapsed));
            }
            _ => {
                // Fallback if stdout capture fails
                let exec_result = self.run_interpreter(&source);
                let elapsed = start.elapsed();
                if let Err(e) = exec_result {
                    self.input.add_error(&e);
                }
                self.input.add_info(&Self::format_elapsed(elapsed));
            }
        }
    }

    fn format_elapsed(elapsed: std::time::Duration) -> String {
        let micros = elapsed.as_micros();
        if micros < 1000 {
            format!("{}µs", micros)
        } else if micros < 1_000_000 {
            format!("{:.1}ms", micros as f64 / 1000.0)
        } else {
            format!("{:.2}s", elapsed.as_secs_f64())
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
