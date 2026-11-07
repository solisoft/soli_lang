//! What the expression before a `.` (or an `@`) is, read backwards from the
//! cursor without parsing: while the user types `post.` the buffer usually
//! does not parse, so a parse-based answer would come exactly when it is not
//! available.
//!
//! The resolver only answers when the receiver is unambiguous — a class name,
//! a local assigned from `X.new(…)` / `X.find(…)`, a typed parameter, `@` or
//! `this` inside a class, a literal — and says [`Receiver::Unknown`]
//! otherwise. It never guesses a type.

/// The receiver of a member access.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Receiver {
    /// `Post.` — the class itself: its static methods and scopes.
    Class(String),
    /// An instance of the class: `post.` after `post = Post.find(id)`.
    Instance(String),
    /// `@` or `this.` inside `class X`: the instance of X. `None` outside a
    /// class.
    SelfInstance(Option<String>),
    /// A built-in value: `"string"`, `"array"`, `"hash"`, `"int"`, `"float"`
    /// (the keys of `method_registry::known_methods`).
    Builtin(&'static str),
    /// Something the resolver cannot pin down.
    Unknown,
}

/// A member access being typed: the receiver, and the part of the member
/// name already typed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemberAccess {
    pub receiver: Receiver,
    pub prefix: String,
    /// `@name` (an instance member of the enclosing class, written without a
    /// dot) rather than `recv.name`.
    pub at_sign: bool,
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// A member name may end in `?` or `!` (`blank?`, `save!`).
fn is_member_char(c: char) -> bool {
    is_word(c) || c == '?' || c == '!'
}

/// The member access whose name ends at byte `offset` of `source` (the
/// cursor), or `None` when the cursor is not after a `.` or an `@`.
pub fn member_access(source: &str, offset: usize) -> Option<MemberAccess> {
    let offset = offset.min(source.len());
    if !source.is_char_boundary(offset) {
        return None;
    }
    let before = &source[..offset];
    let prefix_start = before
        .char_indices()
        .rev()
        .take_while(|&(_, c)| is_member_char(c))
        .last()
        .map_or(offset, |(i, _)| i);
    let prefix = before[prefix_start..].to_string();
    let head = &before[..prefix_start];
    let last = head.chars().next_back()?;
    match last {
        '@' => {
            // `@@x` is not a thing in Soli; a lone `@` opens an instance member.
            Some(MemberAccess {
                receiver: Receiver::SelfInstance(enclosing_class(head)),
                prefix,
                at_sign: true,
            })
        }
        '.' => {
            let receiver_text = &head[..head.len() - 1];
            // `1..5` or `...rest`: not a member access.
            if receiver_text.ends_with('.') {
                return None;
            }
            Some(MemberAccess {
                receiver: resolve(receiver_text, receiver_text),
                prefix,
                at_sign: false,
            })
        }
        _ => None,
    }
}

/// The receiver that ends at the end of `text`; `context` is the source
/// before it, searched for the assignment of a local.
fn resolve(text: &str, context: &str) -> Receiver {
    let trimmed = text.trim_end();
    let Some(last) = trimmed.chars().next_back() else {
        return Receiver::Unknown;
    };
    match last {
        '"' | '\'' => {
            if string_literal_start(trimmed).is_some() {
                Receiver::Builtin("string")
            } else {
                Receiver::Unknown
            }
        }
        ']' => match matching_open(trimmed, '[', ']') {
            Some(open) if starts_a_value(&trimmed[..open]) => Receiver::Builtin("array"),
            _ => Receiver::Unknown,
        },
        '}' => match matching_open(trimmed, '{', '}') {
            Some(open) if starts_a_value(&trimmed[..open]) => Receiver::Builtin("hash"),
            _ => Receiver::Unknown,
        },
        ')' => {
            let Some(open) = matching_open(trimmed, '(', ')') else {
                return Receiver::Unknown;
            };
            call_result(&trimmed[..open]).unwrap_or(Receiver::Unknown)
        }
        c if c.is_ascii_digit() => {
            let number: String = trimmed
                .chars()
                .rev()
                .take_while(|c| c.is_ascii_digit() || *c == '.' || *c == '_')
                .collect();
            let start = trimmed.len() - number.len();
            if trimmed[..start].chars().next_back().is_some_and(is_word) {
                // `x2.`: the tail of an identifier, not a number.
                return resolve_identifier(trimmed, context);
            }
            if number.contains('.') {
                Receiver::Builtin("float")
            } else {
                Receiver::Builtin("int")
            }
        }
        c if is_word(c) => resolve_identifier(trimmed, context),
        _ => Receiver::Unknown,
    }
}

fn resolve_identifier(text: &str, context: &str) -> Receiver {
    let name_start = text
        .char_indices()
        .rev()
        .take_while(|&(_, c)| is_word(c))
        .last()
        .map_or(text.len(), |(i, _)| i);
    let name = &text[name_start..];
    let before = &text[..name_start];
    // `a.b.` — a chain this resolver does not follow; `X::Y.` — a nested
    // class, known by its own name.
    if before.ends_with('.') || before.ends_with("&.") {
        return Receiver::Unknown;
    }
    if name == "this" {
        return Receiver::SelfInstance(enclosing_class(before));
    }
    if name.starts_with(|c: char| c.is_ascii_uppercase()) {
        return Receiver::Class(name.to_string());
    }
    local_type(name, context)
}

/// What a local called `name` holds, from its last assignment or its typed
/// parameter before the cursor.
fn local_type(name: &str, context: &str) -> Receiver {
    let mut found = Receiver::Unknown;
    for line in context.lines() {
        let trimmed = line.trim_start();
        // `name = value` (not `==`, not `name.x = …`).
        if let Some(rest) = strip_word(trimmed, name) {
            let rest = rest.trim_start();
            if let Some(value) = rest.strip_prefix('=').filter(|v| !v.starts_with('=')) {
                found = value_type(value.trim());
                continue;
            }
            if let Some(value) = rest.strip_prefix("||=") {
                found = value_type(value.trim());
                continue;
            }
        }
        // A parameter `name: Type` in a `def` line.
        if trimmed.starts_with("def ") || trimmed.starts_with("static def ") {
            if let Some(ty) = parameter_type(trimmed, name) {
                found = type_receiver(&ty);
            }
        }
    }
    found
}

/// `rest` after `word` when `text` starts with `word` as a whole word.
fn strip_word<'a>(text: &'a str, word: &str) -> Option<&'a str> {
    let rest = text.strip_prefix(word)?;
    if rest.chars().next().is_some_and(is_word) {
        return None;
    }
    Some(rest)
}

/// The declared type of parameter `name` in a `def` line.
fn parameter_type(line: &str, name: &str) -> Option<String> {
    let open = line.find('(')?;
    let close = line.rfind(')').unwrap_or(line.len());
    for param in line.get(open + 1..close)?.split(',') {
        let param = param.trim().trim_start_matches('&');
        let Some(rest) = strip_word(param, name) else {
            continue;
        };
        let ty = rest.trim_start().strip_prefix(':')?.trim();
        let ty: String = ty
            .chars()
            .take_while(|c| is_word(*c) || *c == ':')
            .collect();
        return (!ty.is_empty()).then_some(ty);
    }
    None
}

fn type_receiver(ty: &str) -> Receiver {
    match ty {
        "String" => Receiver::Builtin("string"),
        "Int" => Receiver::Builtin("int"),
        "Float" => Receiver::Builtin("float"),
        "Array" => Receiver::Builtin("array"),
        "Hash" => Receiver::Builtin("hash"),
        "Any" | "" => Receiver::Unknown,
        other if other.starts_with(|c: char| c.is_ascii_uppercase()) => {
            Receiver::Instance(other.rsplit("::").next().unwrap_or(other).to_string())
        }
        _ => Receiver::Unknown,
    }
}

/// The value an assignment's right-hand side starts with.
fn value_type(value: &str) -> Receiver {
    let value = value.trim_end();
    match value.chars().next() {
        Some('"') | Some('\'') => return Receiver::Builtin("string"),
        Some('[') => return Receiver::Builtin("array"),
        Some('{') => return Receiver::Builtin("hash"),
        Some(c) if c.is_ascii_digit() => {
            let number: String = value
                .chars()
                .take_while(|c| c.is_ascii_digit() || *c == '.' || *c == '_')
                .collect();
            return if number.contains('.') {
                Receiver::Builtin("float")
            } else {
                Receiver::Builtin("int")
            };
        }
        _ => {}
    }
    // `Post.find(…)`, `Post.new(…)`, `Post.first` …: the whole value is one
    // call on a class.
    let class: String = value.chars().take_while(|c| is_word(*c)).collect();
    if !class.starts_with(|c: char| c.is_ascii_uppercase()) {
        return Receiver::Unknown;
    }
    let rest = &value[class.len()..];
    let Some(rest) = rest.strip_prefix('.') else {
        return Receiver::Unknown;
    };
    let method: String = rest.chars().take_while(|c| is_member_char(*c)).collect();
    let after = &rest[method.len()..];
    // Nothing chained after the call: `Post.find(id).title` is not a Post.
    let tail = if let Some(args) = after.strip_prefix('(') {
        match matching_close(args) {
            Some(close) => &args[close + 1..],
            None => return Receiver::Unknown,
        }
    } else {
        after
    };
    if !tail.trim().is_empty() && !tail.trim_start().starts_with('#') {
        return Receiver::Unknown;
    }
    if returns_instance(&method) {
        Receiver::Instance(class)
    } else {
        Receiver::Unknown
    }
}

/// Class methods that answer one instance of the class they are called on.
fn returns_instance(method: &str) -> bool {
    matches!(
        method,
        "new"
            | "find"
            | "find_by"
            | "first_by"
            | "first"
            | "last"
            | "create"
            | "create!"
            | "find_or_create_by"
            | "find_or_initialize_by"
    )
}

/// `X.method(` before a `(`: the result of a class-method call.
fn call_result(callee: &str) -> Option<Receiver> {
    let method_start = callee
        .char_indices()
        .rev()
        .take_while(|&(_, c)| is_member_char(c))
        .last()
        .map(|(i, _)| i)?;
    let method = &callee[method_start..];
    let class_part = callee[..method_start].strip_suffix('.')?;
    let class_start = class_part
        .char_indices()
        .rev()
        .take_while(|&(_, c)| is_word(c))
        .last()
        .map(|(i, _)| i)?;
    let class = &class_part[class_start..];
    if class_part[..class_start].ends_with('.') {
        return None;
    }
    (class.starts_with(|c: char| c.is_ascii_uppercase()) && returns_instance(method))
        .then(|| Receiver::Instance(class.to_string()))
}

/// Whether the text before an opening bracket leaves it starting a value
/// (`= [`, `([`, `, {`, a line start) rather than indexing or a block.
fn starts_a_value(before: &str) -> bool {
    match before.trim_end().chars().next_back() {
        None => true,
        Some(c) => matches!(
            c,
            '=' | '(' | ',' | '[' | '{' | ':' | '\n' | '>' | '|' | '&'
        ),
    }
}

/// The byte index of the bracket that opens the one `text` ends with.
fn matching_open(text: &str, open: char, close: char) -> Option<usize> {
    let mut depth = 0usize;
    for (i, c) in text.char_indices().rev() {
        if c == close {
            depth += 1;
        } else if c == open {
            depth -= 1;
            if depth == 0 {
                return Some(i);
            }
        }
    }
    None
}

/// The index of the `)` that closes an argument list starting right after
/// its `(`.
fn matching_close(text: &str) -> Option<usize> {
    let mut depth = 1usize;
    for (i, c) in text.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

/// Where the string literal `text` ends with starts, if its quotes balance on
/// that line.
fn string_literal_start(text: &str) -> Option<usize> {
    let quote = text.chars().next_back()?;
    let line_start = text.rfind('\n').map_or(0, |i| i + 1);
    let line = &text[line_start..text.len() - quote.len_utf8()];
    let open = line.rfind(quote)?;
    Some(line_start + open)
}

/// The class whose body the end of `text` is in: the nearest `class Name`
/// line above that is still open, judged by indentation.
pub fn enclosing_class(text: &str) -> Option<String> {
    let current_indent = text
        .rsplit('\n')
        .next()
        .map(|line| line.len() - line.trim_start().len());
    let mut min_indent = current_indent.unwrap_or(usize::MAX);
    let above = &text[..text.rfind('\n').unwrap_or(0)];
    for line in above.lines().rev() {
        let trimmed = line.trim_start();
        if trimmed.is_empty() {
            continue;
        }
        let indent = line.len() - trimmed.len();
        if indent >= min_indent && min_indent != 0 {
            continue;
        }
        min_indent = indent;
        let declaration = trimmed
            .strip_prefix("private ")
            .unwrap_or(trimmed)
            .strip_prefix("class ");
        if let Some(rest) = declaration {
            let name: String = rest.chars().take_while(|c| is_word(*c)).collect();
            return (!name.is_empty()).then_some(name);
        }
        if indent == 0 {
            return None;
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at_end(source: &str) -> MemberAccess {
        member_access(source, source.len()).unwrap()
    }

    #[test]
    fn a_class_name_is_the_class() {
        let access = at_end("def index\n  Post.pub");
        assert_eq!(access.receiver, Receiver::Class("Post".into()));
        assert_eq!(access.prefix, "pub");
        assert_eq!(
            at_end("x = Billing::Invoice.").receiver,
            Receiver::Class("Invoice".into())
        );
    }

    #[test]
    fn a_local_assigned_from_a_finder_is_an_instance() {
        let source = "def show(id)\n  post = Post.find(id)\n  post.";
        assert_eq!(at_end(source).receiver, Receiver::Instance("Post".into()));
        let source = "post = Post.new({\"title\": \"x\"})\npost.ti";
        assert_eq!(at_end(source).receiver, Receiver::Instance("Post".into()));
        // A chained call is not the class's instance.
        let source = "title = Post.find(id).title\ntitle.";
        assert_eq!(at_end(source).receiver, Receiver::Unknown);
        assert_eq!(
            at_end("Post.find(1).").receiver,
            Receiver::Instance("Post".into())
        );
        // A where returns a query, not a post.
        assert_eq!(at_end("x = Post.where(a)\nx.").receiver, Receiver::Unknown);
    }

    #[test]
    fn a_typed_parameter_is_its_type() {
        let source = "def render_card(card: Card, title: String)\n  card.";
        assert_eq!(at_end(source).receiver, Receiver::Instance("Card".into()));
        let source = "def render_card(card: Card, title: String)\n  title.";
        assert_eq!(at_end(source).receiver, Receiver::Builtin("string"));
    }

    #[test]
    fn literals_and_locals_holding_them_are_builtins() {
        assert_eq!(at_end("\"x\".up").receiver, Receiver::Builtin("string"));
        assert_eq!(at_end("x = [1, 2].").receiver, Receiver::Builtin("array"));
        assert_eq!(at_end("{\"a\": 1}.").receiver, Receiver::Builtin("hash"));
        assert_eq!(at_end("42.").receiver, Receiver::Builtin("int"));
        assert_eq!(
            at_end("names = [\"a\"]\nnames.").receiver,
            Receiver::Builtin("array")
        );
        // Indexing is not an array literal.
        assert_eq!(at_end("rows[0].").receiver, Receiver::Unknown);
    }

    #[test]
    fn at_and_this_are_the_enclosing_class() {
        let source = "class Post < Model\n  title: String\n\n  def summary\n    @ti";
        let access = at_end(source);
        assert!(access.at_sign);
        assert_eq!(access.receiver, Receiver::SelfInstance(Some("Post".into())));
        assert_eq!(access.prefix, "ti");
        let source = "class Post < Model\n  def summary\n    this.";
        assert_eq!(
            at_end(source).receiver,
            Receiver::SelfInstance(Some("Post".into()))
        );
        let source = "class Post\nend\n\ndef loose\n  @";
        assert_eq!(at_end(source).receiver, Receiver::SelfInstance(None));
    }

    #[test]
    fn anything_else_is_unknown_and_ranges_are_not_access() {
        assert_eq!(at_end("a.b.").receiver, Receiver::Unknown);
        assert_eq!(at_end("thing.").receiver, Receiver::Unknown);
        assert!(member_access("(1..", 4).is_none());
        assert!(member_access("x = ab", 6).is_none());
    }
}
