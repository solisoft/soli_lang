//! Block structure, scalars and flow collections, read line by line.
//!
//! A node is read from a position (line, byte column) with the indentation of
//! its parent: a mapping or a sequence continues on the lines indented exactly
//! like its first entry; a scalar continues on the lines indented more than
//! its parent. Every structural character is ASCII, so byte columns are safe
//! slice bounds.

use std::cell::Cell;

use serde_json::{Map, Value};

use crate::{scalar, Error};

/// How deeply collections may nest — `serde_yaml`'s limit. Every level is a
/// stack frame here, so a document of a hundred thousand `[` would otherwise
/// overflow the stack and abort the process.
const MAX_DEPTH: usize = 128;
const TOO_DEEP: &str = "recursion limit exceeded";

pub(crate) fn parse(text: &str) -> Result<Value, Error> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let parser = Parser {
        lines: document_lines(text)?,
        depth: Cell::new(0),
    };
    let Some(start) = parser.next_significant(0) else {
        return Ok(Value::Null);
    };
    let col = parser.indent(start)?;
    let (value, next) = parser.block(start, col, -1)?;
    if let Some(extra) = parser.next_significant(next) {
        let col = parser.indent(extra)?;
        return Err(parser.err("did not find expected <document end>", extra, col));
    }
    Ok(value)
}

/// The document's lines, with a leading `---` and a trailing `...` turned into
/// blank lines (`--- value` keeps its value, at its column). A second
/// document, or a directive, is refused.
fn document_lines(text: &str) -> Result<Vec<String>, Error> {
    let mut lines = Vec::new();
    let mut seen_content = false;
    let mut ended = false;
    for (index, raw) in text.split('\n').enumerate() {
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        let number = index + 1;
        if is_marker(line, "---") {
            if seen_content || ended {
                return Err(Error::new(
                    "deserializing from YAML containing more than one document is not supported",
                    number,
                    1,
                ));
            }
            let rest = &line[3..];
            if !blank_or_comment(rest) {
                seen_content = true;
            }
            lines.push(format!("   {rest}"));
            continue;
        }
        if is_marker(line, "...") {
            ended = true;
            lines.push(String::new());
            continue;
        }
        if ended {
            if !blank_or_comment(line) {
                return Err(Error::new(
                    "deserializing from YAML containing more than one document is not supported",
                    number,
                    1,
                ));
            }
            lines.push(String::new());
            continue;
        }
        if !seen_content && line.starts_with('%') {
            return Err(Error::new(
                "directives (%YAML, %TAG) are not supported",
                number,
                1,
            ));
        }
        if !blank_or_comment(line) {
            seen_content = true;
        }
        lines.push(line.to_string());
    }
    Ok(lines)
}

fn is_marker(line: &str, marker: &str) -> bool {
    line == marker
        || (line.starts_with(marker) && matches!(line.as_bytes().get(3), Some(b' ' | b'\t')))
}

fn blank_or_comment(s: &str) -> bool {
    let t = s.trim_start_matches([' ', '\t']);
    t.is_empty() || t.starts_with('#')
}

/// `-` alone, or followed by a space: a block sequence entry.
fn is_seq_entry(s: &str) -> bool {
    s == "-" || s.starts_with("- ") || s.starts_with("-\t")
}

/// A plain scalar's text on one line: up to a comment (`#` after a space),
/// without trailing spaces.
fn plain_part(s: &str) -> &str {
    let bytes = s.as_bytes();
    let mut end = bytes.len();
    for i in 0..bytes.len() {
        if bytes[i] == b'#' && i > 0 && (bytes[i - 1] == b' ' || bytes[i - 1] == b'\t') {
            end = i;
            break;
        }
    }
    s[..end].trim_end_matches([' ', '\t'])
}

/// A `: ` (or a final `:`) inside a plain scalar in block context.
fn mapping_indicator(s: &str) -> Option<usize> {
    let bytes = s.as_bytes();
    (0..bytes.len()).find(|&i| {
        bytes[i] == b':' && (i + 1 == bytes.len() || bytes[i + 1] == b' ' || bytes[i + 1] == b'\t')
    })
}

fn leading_ws(s: &str) -> usize {
    s.len() - s.trim_start_matches([' ', '\t']).len()
}

struct Parser {
    lines: Vec<String>,
    /// How many block nodes are open around the one being read.
    depth: Cell<usize>,
}

type Node = Result<(Value, usize), Error>;

impl Parser {
    fn err(&self, message: impl Into<String>, line: usize, col: usize) -> Error {
        let text = self.lines.get(line).map(String::as_str).unwrap_or("");
        let col = text
            .get(..col.min(text.len()))
            .map_or(col, |s| s.chars().count());
        Error::new(message, line + 1, col + 1)
    }

    fn next_significant(&self, from: usize) -> Option<usize> {
        (from..self.lines.len()).find(|&i| !blank_or_comment(&self.lines[i]))
    }

    /// The line's indentation, in spaces; a tab there is refused.
    fn indent(&self, line: usize) -> Result<usize, Error> {
        let s = &self.lines[line];
        let n = s.bytes().take_while(|&b| b == b' ').count();
        if s.as_bytes().get(n) == Some(&b'\t') {
            return Err(self.err(
                "found a tab character where an indentation space is expected",
                line,
                n,
            ));
        }
        Ok(n)
    }

    /// A node that starts at (`line`, `col`), as the first thing on its line
    /// or after a sequence entry's `- `.
    fn block(&self, line: usize, col: usize, parent: isize) -> Node {
        let depth = self.depth.get();
        if depth >= MAX_DEPTH {
            return Err(self.err(TOO_DEEP, line, col));
        }
        self.depth.set(depth + 1);
        let node = self.block_at_depth(line, col, parent);
        self.depth.set(depth);
        node
    }

    fn block_at_depth(&self, line: usize, col: usize, parent: isize) -> Node {
        let rest = &self.lines[line][col..];
        if is_seq_entry(rest) {
            return self.sequence(line, col);
        }
        if matches!(rest.as_bytes().first(), Some(b'[' | b'{')) {
            return self.flow_node(line, col);
        }
        if self.key(line, col)?.is_some() {
            return self.mapping(line, col);
        }
        self.inline_value(line, col, parent)
    }

    /// The key of a mapping entry at (`line`, `col`), and the column after
    /// its `:` — `None` when the line is no mapping entry.
    fn key(&self, line: usize, col: usize) -> Result<Option<(String, usize)>, Error> {
        let text = &self.lines[line];
        let s = &text[col..];
        match s.as_bytes().first() {
            Some(&q) if q == b'"' || q == b'\'' => {
                let Some(close) = closing_quote(s, q) else {
                    return Ok(None);
                };
                let after = col + close + 1;
                let rest = &text[after..];
                let colon = after + leading_ws(rest);
                let follows_colon = text[colon..].strip_prefix(':');
                match follows_colon {
                    Some(r) if r.is_empty() || r.starts_with([' ', '\t']) => {
                        let (key, _, _) = self.quoted(line, col)?;
                        Ok(Some((key, colon + 1)))
                    }
                    _ => Ok(None),
                }
            }
            Some(b'?') if s.len() == 1 || matches!(s.as_bytes()[1], b' ' | b'\t') => {
                Err(self.err("complex keys (`? key`) are not supported", line, col))
            }
            Some(b'[' | b'{') | None => Ok(None),
            Some(_) => {
                let bytes = s.as_bytes();
                for i in 0..bytes.len() {
                    if bytes[i] == b'#' && i > 0 && matches!(bytes[i - 1], b' ' | b'\t') {
                        return Ok(None);
                    }
                    if bytes[i] == b':'
                        && (i + 1 == bytes.len() || matches!(bytes[i + 1], b' ' | b'\t'))
                    {
                        let key = s[..i].trim_end_matches([' ', '\t']);
                        if key.is_empty() {
                            return Ok(None);
                        }
                        self.check_start(key, line, col)?;
                        return Ok(Some((key.to_string(), col + i + 1)));
                    }
                }
                Ok(None)
            }
        }
    }

    /// Refuse what YAML gives a meaning this crate does not read, at the start
    /// of a plain scalar or a key.
    fn check_start(&self, s: &str, line: usize, col: usize) -> Result<(), Error> {
        match s.as_bytes().first() {
            Some(b'&') => Err(self.err("anchors (`&name`) are not supported", line, col)),
            Some(b'*') => Err(self.err("aliases (`*name`) are not supported", line, col)),
            Some(b'!') => Err(self.err("tags (`!tag`) are not supported", line, col)),
            Some(b'@' | b'`') => {
                Err(self.err("found character that cannot start any token", line, col))
            }
            _ => Ok(()),
        }
    }

    fn mapping(&self, line: usize, col: usize) -> Node {
        let mut map = Map::new();
        let mut current = line;
        loop {
            let (key, after) = self
                .key(current, col)?
                .ok_or_else(|| self.err("could not find expected ':'", current, col))?;
            let (value, next) = self.mapping_value(current, after, col)?;
            // A key given twice keeps its last value (and its first place).
            map.insert(key, value);
            let Some(following) = self.next_significant(next) else {
                return Ok((Value::Object(map), next));
            };
            let indent = self.indent(following)?;
            if indent < col {
                return Ok((Value::Object(map), following));
            }
            if indent > col {
                return Err(self.err(
                    "mapping values are not allowed in this context",
                    following,
                    indent,
                ));
            }
            if is_seq_entry(&self.lines[following][col..]) {
                return Err(self.err("did not find expected key", following, col));
            }
            current = following;
        }
    }

    fn mapping_value(&self, line: usize, after: usize, col: usize) -> Node {
        let text = &self.lines[line];
        let start = after + leading_ws(&text[after..]);
        let rest = &text[start..];
        if rest.is_empty() || rest.starts_with('#') {
            if let Some(next) = self.next_significant(line + 1) {
                let indent = self.indent(next)?;
                if indent > col {
                    return self.block(next, indent, col as isize);
                }
                if indent == col && is_seq_entry(&self.lines[next][col..]) {
                    return self.sequence(next, col);
                }
            }
            return Ok((Value::Null, line + 1));
        }
        self.inline_value(line, start, col as isize)
    }

    fn sequence(&self, line: usize, col: usize) -> Node {
        let mut items = Vec::new();
        let mut current = line;
        loop {
            let text = &self.lines[current];
            let content = col + 1 + leading_ws(&text[col + 1..]);
            let rest = &text[content..];
            let (value, next) = if rest.is_empty() || rest.starts_with('#') {
                match self.next_significant(current + 1) {
                    Some(following) if self.indent(following)? > col => {
                        let indent = self.indent(following)?;
                        self.block(following, indent, col as isize)?
                    }
                    _ => (Value::Null, current + 1),
                }
            } else {
                self.block(current, content, col as isize)?
            };
            items.push(value);
            let Some(following) = self.next_significant(next) else {
                return Ok((Value::Array(items), next));
            };
            let indent = self.indent(following)?;
            if indent < col {
                return Ok((Value::Array(items), following));
            }
            if indent > col {
                return Err(self.err("did not find expected '-' indicator", following, indent));
            }
            // A key at the sequence's own indentation ends a sequence that
            // was the value of the key above it (`key:` / `- a` / `next: 1`).
            if !is_seq_entry(&self.lines[following][col..]) {
                return Ok((Value::Array(items), following));
            }
            current = following;
        }
    }

    /// A scalar or a flow collection: a mapping's value after `key: `, or a
    /// node on its own line that is neither a mapping nor a sequence.
    fn inline_value(&self, line: usize, col: usize, parent: isize) -> Node {
        let s = &self.lines[line][col..];
        match s.as_bytes()[0] {
            b'|' | b'>' => self.block_scalar(line, col, parent),
            b'"' | b'\'' => {
                let (text, end_line, end_col) = self.quoted(line, col)?;
                self.expect_line_end(end_line, end_col)?;
                Ok((Value::String(text), end_line + 1))
            }
            b'[' | b'{' => self.flow_node(line, col),
            b'-' if is_seq_entry(s) => Err(self.err(
                "block sequence entries are not allowed in this context",
                line,
                col,
            )),
            b'?' if s.len() == 1 || matches!(s.as_bytes()[1], b' ' | b'\t') => {
                Err(self.err("complex keys (`? key`) are not supported", line, col))
            }
            _ => {
                self.check_start(s, line, col)?;
                self.plain(line, col, parent)
            }
        }
    }

    /// Nothing but a comment may follow a quoted scalar or a flow collection.
    fn expect_line_end(&self, line: usize, col: usize) -> Result<(), Error> {
        let rest = &self.lines[line][col..];
        if blank_or_comment(rest) && (rest.is_empty() || rest.starts_with([' ', '\t'])) {
            return Ok(());
        }
        Err(self.err("did not find expected key", line, col + leading_ws(rest)))
    }

    /// A plain scalar, folded over the lines indented more than `parent`.
    fn plain(&self, line: usize, col: usize, parent: isize) -> Node {
        let first_text = &self.lines[line][col..];
        let first = plain_part(first_text);
        if let Some(i) = mapping_indicator(first) {
            return Err(self.err(
                "mapping values are not allowed in this context",
                line,
                col + i,
            ));
        }
        let mut text = first.to_string();
        let mut next = line + 1;
        // A comment ends the scalar, and so does the end of its line.
        if first.len() < first_text.trim_end_matches([' ', '\t']).len() {
            return Ok((scalar::resolve(&text), next));
        }
        let mut breaks = 0;
        for current in line + 1..self.lines.len() {
            let raw = &self.lines[current];
            if raw.trim_matches([' ', '\t']).is_empty() {
                breaks += 1;
                continue;
            }
            let indent = self.indent(current)?;
            let content = &raw[indent..];
            if content.starts_with('#') || indent as isize <= parent {
                break;
            }
            let part = plain_part(content);
            if let Some(i) = mapping_indicator(part) {
                return Err(self.err(
                    "mapping values are not allowed in this context",
                    current,
                    indent + i,
                ));
            }
            if breaks > 0 {
                text.push_str(&"\n".repeat(breaks));
            } else {
                text.push(' ');
            }
            breaks = 0;
            text.push_str(part);
            next = current + 1;
            if part.len() < content.trim_end_matches([' ', '\t']).len() {
                break;
            }
        }
        Ok((scalar::resolve(&text), next))
    }

    /// A quoted scalar starting at (`line`, `col`), over as many lines as it
    /// takes: its text, and the line and column after the closing quote.
    fn quoted(&self, line: usize, col: usize) -> Result<(String, usize, usize), Error> {
        let quote = self.lines[line].as_bytes()[col];
        let double = quote == b'"';
        let mut out = String::new();
        let mut current = line;
        let mut pos = col + 1;
        loop {
            let text = &self.lines[current];
            let mut chars = text[pos..].char_indices().peekable();
            // Set when the line ends in an escaped line break (`\`): the next
            // line joins with no space.
            let mut joined = false;
            while let Some((offset, c)) = chars.next() {
                let at = pos + offset;
                match c {
                    '\'' if !double => {
                        if matches!(chars.peek(), Some((_, '\''))) {
                            chars.next();
                            out.push('\'');
                        } else {
                            return Ok((out, current, at + 1));
                        }
                    }
                    '"' if double => return Ok((out, current, at + 1)),
                    '\\' if double => match chars.next() {
                        None => joined = true,
                        Some((_, e)) => out.push(self.escape(e, &mut chars, current, at)?),
                    },
                    _ => out.push(c),
                }
            }
            // The line ends inside the scalar: fold the break.
            if !joined {
                let trimmed = out.trim_end_matches([' ', '\t']).len();
                out.truncate(trimmed);
            }
            current += 1;
            let mut breaks = 0;
            while current < self.lines.len() && self.lines[current].trim().is_empty() {
                breaks += 1;
                current += 1;
            }
            if current >= self.lines.len() {
                return Err(self.err(
                    "found unexpected end of stream while scanning a quoted scalar",
                    line,
                    col,
                ));
            }
            if breaks > 0 {
                out.push_str(&"\n".repeat(breaks));
            } else if !joined {
                out.push(' ');
            }
            pos = leading_ws(&self.lines[current]);
        }
    }

    fn escape(
        &self,
        e: char,
        chars: &mut std::iter::Peekable<std::str::CharIndices<'_>>,
        line: usize,
        col: usize,
    ) -> Result<char, Error> {
        let hex = |chars: &mut std::iter::Peekable<std::str::CharIndices<'_>>, n: usize| {
            let digits: String = (0..n)
                .filter_map(|_| chars.next().map(|(_, c)| c))
                .collect();
            u32::from_str_radix(&digits, 16)
                .ok()
                .filter(|_| digits.len() == n)
                .and_then(char::from_u32)
        };
        let decoded = match e {
            '0' => Some('\0'),
            'a' => Some('\u{7}'),
            'b' => Some('\u{8}'),
            't' | '\t' => Some('\t'),
            'n' => Some('\n'),
            'v' => Some('\u{b}'),
            'f' => Some('\u{c}'),
            'r' => Some('\r'),
            'e' => Some('\u{1b}'),
            ' ' => Some(' '),
            '"' => Some('"'),
            '/' => Some('/'),
            '\\' => Some('\\'),
            'N' => Some('\u{85}'),
            '_' => Some('\u{a0}'),
            'L' => Some('\u{2028}'),
            'P' => Some('\u{2029}'),
            'x' => hex(chars, 2),
            'u' => hex(chars, 4),
            'U' => hex(chars, 8),
            _ => None,
        };
        decoded.ok_or_else(|| {
            self.err(
                "found unknown escape character while parsing a quoted scalar",
                line,
                col,
            )
        })
    }

    /// A literal (`|`) or folded (`>`) block scalar whose header is at
    /// (`line`, `col`).
    fn block_scalar(&self, line: usize, col: usize, parent: isize) -> Node {
        let header = &self.lines[line][col..];
        let folded = header.starts_with('>');
        let mut chomp = ' ';
        let mut explicit: Option<usize> = None;
        let mut rest = &header[1..];
        while let Some(c) = rest.chars().next() {
            match c {
                '-' | '+' if chomp == ' ' => chomp = c,
                '1'..='9' if explicit.is_none() => explicit = c.to_digit(10).map(|d| d as usize),
                _ => break,
            }
            rest = &rest[1..];
        }
        if !blank_or_comment(rest) || !(rest.is_empty() || rest.starts_with([' ', '\t'])) {
            return Err(self.err(
                "did not find expected comment or line break",
                line,
                col + header.len() - rest.len(),
            ));
        }
        let content_indent = match explicit {
            Some(d) => (parent + d as isize).max(0) as usize,
            None => {
                let first = (line + 1..self.lines.len())
                    .find(|&i| !self.lines[i].trim_matches([' ', '\t']).is_empty());
                match first {
                    Some(i) if self.indent(i)? as isize > parent => self.indent(i)?,
                    _ => usize::MAX,
                }
            }
        };
        let mut body: Vec<String> = Vec::new();
        let mut next = line + 1;
        for (current, raw) in self.lines.iter().enumerate().skip(line + 1) {
            if raw.trim_matches([' ', '\t']).is_empty() {
                body.push(raw.get(content_indent..).unwrap_or("").to_string());
                next = current + 1;
                continue;
            }
            let indent = raw.bytes().take_while(|&b| b == b' ').count();
            if indent < content_indent {
                break;
            }
            body.push(raw[content_indent..].to_string());
            next = current + 1;
        }
        let trailing = body
            .iter()
            .rev()
            .take_while(|l| l.trim().is_empty())
            .count();
        // Blank lines after the scalar belong to whatever follows it.
        let kept = body.len() - trailing;
        let lines = &body[..kept];
        let mut text = if folded {
            fold(lines)
        } else {
            lines.join("\n")
        };
        match chomp {
            '-' => {}
            '+' => {
                if !lines.is_empty() {
                    text.push('\n');
                }
                text.push_str(&"\n".repeat(trailing));
            }
            _ => {
                if !lines.is_empty() {
                    text.push('\n');
                }
            }
        }
        Ok((Value::String(text), next))
    }

    /// A flow collection starting at (`line`, `col`), over as many lines as it
    /// takes.
    fn flow_node(&self, line: usize, col: usize) -> Node {
        let (text, end_line, end_col) = self.gather_flow(line, col)?;
        let mut flow = Flow {
            chars: text.chars().collect(),
            pos: 0,
            depth: self.depth.get(),
            parser: self,
            line,
            col,
        };
        let value = flow.value()?;
        flow.skip_ws();
        if flow.pos < flow.chars.len() {
            return Err(self.err("did not find expected <document end>", end_line, end_col));
        }
        self.expect_line_end(end_line, end_col)?;
        Ok((value, end_line + 1))
    }

    /// The text from the opening bracket to its closing one, lines joined by
    /// `\n` and comments dropped; and where it ends.
    fn gather_flow(&self, line: usize, col: usize) -> Result<(String, usize, usize), Error> {
        let mut out = String::new();
        let mut depth = 0usize;
        let mut quote: Option<char> = None;
        let mut current = line;
        let mut pos = col;
        loop {
            let text = &self.lines[current];
            let mut chars = text[pos..].char_indices().peekable();
            let mut prev = if pos == 0 {
                ' '
            } else {
                text[..pos].chars().last().unwrap_or(' ')
            };
            while let Some((offset, c)) = chars.next() {
                match quote {
                    Some('"') => {
                        out.push(c);
                        if c == '\\' {
                            if let Some((_, e)) = chars.next() {
                                out.push(e);
                            }
                        } else if c == '"' {
                            quote = None;
                        }
                    }
                    Some(_) => {
                        out.push(c);
                        if c == '\'' {
                            if matches!(chars.peek(), Some((_, '\''))) {
                                chars.next();
                                out.push('\'');
                            } else {
                                quote = None;
                            }
                        }
                    }
                    None => match c {
                        '#' if prev == ' ' || prev == '\t' => break,
                        '"' | '\'' => {
                            quote = Some(c);
                            out.push(c);
                        }
                        '[' | '{' => {
                            depth += 1;
                            out.push(c);
                        }
                        ']' | '}' => {
                            depth = depth.saturating_sub(1);
                            out.push(c);
                            if depth == 0 {
                                return Ok((out, current, pos + offset + 1));
                            }
                        }
                        _ => out.push(c),
                    },
                }
                prev = c;
            }
            current += 1;
            if current >= self.lines.len() {
                return Err(self.err(
                    "did not find expected ',' or ']' (unclosed flow collection)",
                    line,
                    col,
                ));
            }
            out.push('\n');
            pos = 0;
        }
    }
}

/// Fold a folded block scalar's lines: lines of text join with a space, a
/// blank line is a line break, and more-indented lines keep their breaks.
fn fold(lines: &[String]) -> String {
    let mut out = String::new();
    let mut previous: Option<&str> = None;
    let mut breaks = 0;
    for line in lines {
        if line.trim().is_empty() {
            breaks += 1;
            continue;
        }
        let more = line.starts_with([' ', '\t']);
        if let Some(prev) = previous {
            let prev_more = prev.starts_with([' ', '\t']);
            if more || prev_more {
                out.push('\n');
                out.push_str(&"\n".repeat(breaks));
            } else if breaks > 0 {
                out.push_str(&"\n".repeat(breaks));
            } else {
                out.push(' ');
            }
        } else {
            out.push_str(&"\n".repeat(breaks));
        }
        breaks = 0;
        out.push_str(line);
        previous = Some(line);
    }
    out
}

/// The byte index of the quote closing the scalar that opens `s`, on this
/// line.
fn closing_quote(s: &str, quote: u8) -> Option<usize> {
    let bytes = s.as_bytes();
    let mut i = 1;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' if quote == b'"' => i += 2,
            b'\'' if quote == b'\'' => {
                if bytes.get(i + 1) == Some(&b'\'') {
                    i += 2;
                } else {
                    return Some(i);
                }
            }
            b if b == quote => return Some(i),
            _ => i += 1,
        }
    }
    None
}

/// A flow collection's text, read by recursive descent.
struct Flow<'a> {
    chars: Vec<char>,
    pos: usize,
    /// Open collections, the block ones around this flow node included.
    depth: usize,
    parser: &'a Parser,
    line: usize,
    col: usize,
}

impl Flow<'_> {
    fn err(&self, message: &str) -> Error {
        // The collection's line: flow content is short, and a column inside
        // the gathered text would not match the file's.
        self.parser.err(message, self.line, self.col)
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn skip_ws(&mut self) {
        while matches!(self.peek(), Some(' ' | '\t' | '\n' | '\r')) {
            self.pos += 1;
        }
    }

    fn value(&mut self) -> Result<Value, Error> {
        self.skip_ws();
        match self.peek() {
            Some('[' | '{') if self.depth >= MAX_DEPTH => Err(self.err(TOO_DEEP)),
            Some('[') => self.nested(Self::sequence),
            Some('{') => self.nested(Self::mapping),
            Some(q @ ('"' | '\'')) => self.quoted(q).map(Value::String),
            Some('&') => Err(self.err("anchors (`&name`) are not supported")),
            Some('*') => Err(self.err("aliases (`*name`) are not supported")),
            Some('!') => Err(self.err("tags (`!tag`) are not supported")),
            Some('@' | '`') => Err(self.err("found character that cannot start any token")),
            Some(_) => Ok(scalar::resolve(&self.plain())),
            None => Err(self.err("did not find expected node content")),
        }
    }

    /// A plain scalar in flow context: up to `,`, a bracket, or `: `.
    fn plain(&mut self) -> String {
        let start = self.pos;
        while let Some(c) = self.peek() {
            if matches!(c, ',' | '[' | ']' | '{' | '}') {
                break;
            }
            if c == ':'
                && matches!(
                    self.chars.get(self.pos + 1),
                    None | Some(' ' | '\t' | '\n' | ',' | ']' | '}')
                )
            {
                break;
            }
            self.pos += 1;
        }
        let raw: String = self.chars[start..self.pos].iter().collect();
        // A plain scalar over several lines folds its breaks into spaces.
        raw.split('\n')
            .map(|l| l.trim_matches([' ', '\t']))
            .filter(|l| !l.is_empty())
            .collect::<Vec<_>>()
            .join(" ")
    }

    fn quoted(&mut self, quote: char) -> Result<String, Error> {
        self.pos += 1;
        let mut out = String::new();
        loop {
            let Some(c) = self.peek() else {
                return Err(
                    self.err("found unexpected end of stream while scanning a quoted scalar")
                );
            };
            self.pos += 1;
            match c {
                '\'' if quote == '\'' => {
                    if self.peek() == Some('\'') {
                        self.pos += 1;
                        out.push('\'');
                    } else {
                        return Ok(out);
                    }
                }
                '"' if quote == '"' => return Ok(out),
                '\\' if quote == '"' => {
                    let rest: String = self.chars[self.pos..].iter().collect();
                    let mut chars = rest.char_indices().peekable();
                    let Some((_, e)) = chars.next() else {
                        return Err(self.err("found unexpected end of stream"));
                    };
                    if e == '\n' {
                        self.pos += 1;
                        while matches!(self.peek(), Some(' ' | '\t')) {
                            self.pos += 1;
                        }
                        continue;
                    }
                    let decoded = self.parser.escape(e, &mut chars, self.line, self.col)?;
                    let used = chars.peek().map_or(rest.len(), |(i, _)| *i);
                    self.pos += rest[..used].chars().count();
                    out.push(decoded);
                }
                '\n' => {
                    let trimmed = out.trim_end_matches([' ', '\t']).len();
                    out.truncate(trimmed);
                    let mut breaks = 0;
                    loop {
                        while matches!(self.peek(), Some(' ' | '\t')) {
                            self.pos += 1;
                        }
                        if self.peek() == Some('\n') {
                            breaks += 1;
                            self.pos += 1;
                        } else {
                            break;
                        }
                    }
                    if breaks > 0 {
                        out.push_str(&"\n".repeat(breaks));
                    } else {
                        out.push(' ');
                    }
                }
                _ => out.push(c),
            }
        }
    }

    fn nested(&mut self, read: fn(&mut Self) -> Result<Value, Error>) -> Result<Value, Error> {
        self.depth += 1;
        let value = read(self);
        self.depth -= 1;
        value
    }

    fn sequence(&mut self) -> Result<Value, Error> {
        self.pos += 1;
        let mut items = Vec::new();
        loop {
            self.skip_ws();
            if self.peek() == Some(']') {
                self.pos += 1;
                return Ok(Value::Array(items));
            }
            let item = self.value()?;
            self.skip_ws();
            // `[key: value]`: a single-pair mapping.
            if self.peek() == Some(':') {
                self.pos += 1;
                self.skip_ws();
                let value = if matches!(self.peek(), Some(',' | ']')) {
                    Value::Null
                } else {
                    self.value()?
                };
                let mut pair = Map::new();
                pair.insert(key_string(item), value);
                items.push(Value::Object(pair));
            } else {
                items.push(item);
            }
            self.skip_ws();
            match self.peek() {
                Some(',') => self.pos += 1,
                Some(']') => {}
                _ => return Err(self.err("did not find expected ',' or ']'")),
            }
        }
    }

    fn mapping(&mut self) -> Result<Value, Error> {
        self.pos += 1;
        let mut map = Map::new();
        loop {
            self.skip_ws();
            if self.peek() == Some('}') {
                self.pos += 1;
                return Ok(Value::Object(map));
            }
            let key = key_string(self.value()?);
            self.skip_ws();
            let value = if self.peek() == Some(':') {
                self.pos += 1;
                self.skip_ws();
                if matches!(self.peek(), Some(',' | '}')) {
                    Value::Null
                } else {
                    self.value()?
                }
            } else {
                Value::Null
            };
            map.insert(key, value);
            self.skip_ws();
            match self.peek() {
                Some(',') => self.pos += 1,
                Some('}') => {}
                _ => return Err(self.err("did not find expected ',' or '}'")),
            }
        }
    }
}

/// A flow mapping key as a string, as keys are once read into JSON.
fn key_string(value: Value) -> String {
    match value {
        Value::String(s) => s,
        other => other.to_string(),
    }
}
