//! Reading an IMAP `BODYSTRUCTURE`, so a message can be opened without
//! downloading it.
//!
//! A server sends the shape of a message with its headers, for free: what
//! parts it has, what each one is, what it is called and how big it is.
//! Until this module that shape was *counted* rather than read — the number
//! of times `"attachment"` appeared in the text was the number of paper
//! clips — which answers a list row and nothing else.
//!
//! Read properly it answers the question that matters: **which parts are
//! the letter and which are the freight**. A mail with seventeen
//! photographs on it is two megabytes, of which the text is four thousand
//! bytes; `UID FETCH <uid> (BODY.PEEK[])` brings down all of it to show
//! four thousand bytes, and then `p` brings down the same two megabytes
//! again to write the photographs. With the structure in hand the first
//! fetch asks for the text parts by number and the second asks for the
//! attachments by number, and nothing crosses the wire twice.
//!
//! The grammar is RFC 3501 §7.4.2: a nested list, `(` … `)`, whose leaves
//! are quoted strings, `NIL`, numbers and parenthesised parameter lists.
//! This parser is tolerant on purpose — a server that says something this
//! does not expect yields `None` and the caller falls back to fetching the
//! whole message, which is what every build before this one did anyway.

/// One leaf of a `BODYSTRUCTURE`: a part that carries bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Part {
    /// The IMAP part number, as `BODY[…]` takes it: `1`, `2`, `1.2`.
    pub number: String,
    /// Lowercase MIME type, e.g. `text`.
    pub kind: String,
    /// Lowercase MIME subtype, e.g. `plain`.
    pub sub: String,
    /// The filename, from the disposition or from the type's `name`.
    pub name: String,
    /// Bytes as the server counts them, before decoding.
    pub bytes: i64,
    /// Whether the part says it is an attachment.
    pub attached: bool,
}

impl Part {
    /// Whether this part is one of the letter's faces rather than freight.
    ///
    /// Text that carries a filename and says it is attached is freight —
    /// a `.txt` or a `.md` someone attached — and text that says nothing is
    /// the letter. The distinction is the disposition, not the type.
    pub fn is_text(&self) -> bool {
        self.kind == "text" && !self.attached
    }
}

/// Tokens of the structure grammar.
#[derive(Debug, PartialEq)]
enum Tok {
    Open,
    Close,
    Str(String),
    Atom(String),
}

fn lex(s: &str) -> Option<Vec<Tok>> {
    let mut out = Vec::new();
    let b: Vec<char> = s.chars().collect();
    let mut i = 0usize;
    while i < b.len() {
        let c = *b.get(i)?;
        match c {
            ' ' | '\t' | '\r' | '\n' => i += 1,
            '(' => {
                out.push(Tok::Open);
                i += 1;
            }
            ')' => {
                out.push(Tok::Close);
                i += 1;
            }
            '"' => {
                // A quoted string, with `\"` and `\\` inside it.
                let mut said = String::new();
                i += 1;
                loop {
                    let c = *b.get(i)?;
                    if c == '\\' {
                        said.push(*b.get(i + 1)?);
                        i += 2;
                        continue;
                    }
                    if c == '"' {
                        i += 1;
                        break;
                    }
                    said.push(c);
                    i += 1;
                }
                out.push(Tok::Str(said));
            }
            '{' => {
                // A literal, `{12}\r\n` then twelve bytes. Servers send
                // these for anything that will not quote; taking the bytes
                // as a string is the same answer as a quoted one.
                let close = s.get(i..)?.find('}')? + i;
                let n: usize = s.get(i + 1..close)?.parse().ok()?;
                let mut at = close + 1;
                while matches!(b.get(at), Some('\r') | Some('\n')) {
                    at += 1;
                }
                let said: String = b.get(at..at + n)?.iter().collect();
                out.push(Tok::Str(said));
                i = at + n;
            }
            _ => {
                let mut said = String::new();
                while let Some(&c) = b.get(i) {
                    if c == '(' || c == ')' || c == ' ' || c == '"' {
                        break;
                    }
                    said.push(c);
                    i += 1;
                }
                out.push(Tok::Atom(said));
            }
        }
    }
    Some(out)
}

/// A parsed node: either a leaf's fields, or a multipart's children.
#[derive(Debug)]
enum Node {
    Leaf(Vec<Item>),
    /// The children, then the multipart's own trailing fields — subtype
    /// parameters, disposition, language. Only the children are read today;
    /// the rest is kept so the parse stays lossless and a caller that later
    /// wants, say, the boundary does not send anyone back through the
    /// grammar for it.
    #[allow(dead_code)]
    Multi(Vec<Node>, Vec<Item>),
}

/// One field of a leaf: a string, a number, `NIL`, or a nested list.
#[derive(Debug, Clone)]
enum Item {
    Str(String),
    Nil,
    Num(i64),
    List(Vec<Item>),
}

impl Item {
    fn text(&self) -> Option<&str> {
        match self {
            Self::Str(s) => Some(s.as_str()),
            _ => None,
        }
    }
}

struct P<'a> {
    toks: &'a [Tok],
    at: usize,
}

impl P<'_> {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.at)
    }

    /// One `( … )`. A list whose first token is `(` is a multipart: its
    /// children come first and its own fields follow them.
    fn node(&mut self) -> Option<Node> {
        if !matches!(self.peek(), Some(Tok::Open)) {
            return None;
        }
        self.at += 1;
        let mut kids = Vec::new();
        while matches!(self.peek(), Some(Tok::Open)) {
            kids.push(self.node()?);
        }
        let mut fields = Vec::new();
        loop {
            match self.peek() {
                Some(Tok::Close) => {
                    self.at += 1;
                    break;
                }
                None => return None,
                _ => fields.push(self.item()?),
            }
        }
        if kids.is_empty() {
            return Some(Node::Leaf(fields));
        }
        Some(Node::Multi(kids, fields))
    }

    fn item(&mut self) -> Option<Item> {
        match self.toks.get(self.at)? {
            Tok::Str(s) => {
                self.at += 1;
                Some(Item::Str(s.clone()))
            }
            Tok::Atom(a) => {
                self.at += 1;
                if a.eq_ignore_ascii_case("NIL") {
                    return Some(Item::Nil);
                }
                Some(
                    a.parse::<i64>()
                        .map_or_else(|_| Item::Str(a.clone()), Item::Num),
                )
            }
            Tok::Open => {
                self.at += 1;
                let mut out = Vec::new();
                loop {
                    match self.toks.get(self.at)? {
                        Tok::Close => {
                            self.at += 1;
                            break;
                        }
                        _ => out.push(self.item()?),
                    }
                }
                Some(Item::List(out))
            }
            Tok::Close => None,
        }
    }
}

/// Look up `key` in a parameter list, which is `("k" "v" "k" "v")`.
fn param(items: &[Item], key: &str) -> Option<String> {
    let mut i = 0;
    while i + 1 < items.len() {
        if items
            .get(i)?
            .text()
            .is_some_and(|k| k.eq_ignore_ascii_case(key))
        {
            return items.get(i + 1)?.text().map(str::to_owned);
        }
        i += 2;
    }
    None
}

/// Walk a node into the flat list of parts that carry bytes.
fn flatten(node: &Node, prefix: &str, out: &mut Vec<Part>) {
    match node {
        Node::Multi(kids, _) => {
            for (i, kid) in kids.iter().enumerate() {
                let number = if prefix.is_empty() {
                    format!("{}", i + 1)
                } else {
                    format!("{prefix}.{}", i + 1)
                };
                flatten(kid, &number, out);
            }
        }
        Node::Leaf(fields) => {
            // `("text" "plain" (params) id description encoding size …)`,
            // then, for any part, an optional disposition further along.
            let kind = fields
                .first()
                .and_then(Item::text)
                .unwrap_or("")
                .to_ascii_lowercase();
            let sub = fields
                .get(1)
                .and_then(Item::text)
                .unwrap_or("")
                .to_ascii_lowercase();
            let params = match fields.get(2) {
                Some(Item::List(p)) => p.clone(),
                _ => Vec::new(),
            };
            let bytes = fields.iter().find_map(|f| match f {
                Item::Num(n) => Some(*n),
                _ => None,
            });
            // The disposition is the last list in the leaf that reads
            // `("attachment" ("filename" "x"))` or `("inline" …)`.
            let mut attached = false;
            let mut name = param(&params, "name").unwrap_or_default();
            for f in fields {
                let Item::List(items) = f else { continue };
                let Some(said) = items.first().and_then(Item::text) else {
                    continue;
                };
                if said.eq_ignore_ascii_case("attachment") || said.eq_ignore_ascii_case("inline") {
                    attached = said.eq_ignore_ascii_case("attachment");
                    if let Some(Item::List(p)) = items.get(1) {
                        if let Some(f) = param(p, "filename") {
                            name = f;
                        }
                    }
                }
            }
            // A part of a message with no multipart around it is part 1.
            let number = if prefix.is_empty() {
                "1".to_owned()
            } else {
                prefix.to_owned()
            };
            out.push(Part {
                number,
                kind,
                sub,
                name,
                bytes: bytes.unwrap_or(0),
                attached,
            });
        }
    }
}

/// The parts of a message, from the `BODYSTRUCTURE` in a FETCH response.
///
/// `None` when there is no structure in it or it does not parse — the
/// caller then asks for the whole message, as it always did.
pub fn parts_of(response: &str) -> Option<Vec<Part>> {
    let at = find_key(response, "BODYSTRUCTURE")?;
    let rest = response.get(at..)?;
    let toks = lex(rest)?;
    let mut p = P { toks: &toks, at: 0 };
    let node = p.node()?;
    let mut out = Vec::new();
    flatten(&node, "", &mut out);
    (!out.is_empty()).then_some(out)
}

/// Where the structure starts: just past the key, at its opening paren.
fn find_key(said: &str, key: &str) -> Option<usize> {
    let upper = said.to_ascii_uppercase();
    let at = upper.find(key)? + key.len();
    let rest = said.get(at..)?;
    let open = rest.find('(')?;
    Some(at + open)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shape this exists for: a letter and a photograph beside it.
    #[test]
    fn a_letter_with_an_attachment_reads_as_two_parts_and_only_one_is_the_letter() {
        let said = "1 FETCH (UID 42 RFC822.SIZE 284213 BODYSTRUCTURE ((\"text\" \"plain\" (\"charset\" \"utf-8\") NIL NIL \"7bit\" 1234 20)(\"image\" \"jpeg\" (\"name\" \"tour.jpg\") NIL NIL \"base64\" 284000 NIL (\"attachment\" (\"filename\" \"tour.jpg\")) NIL) \"mixed\"))";
        let parts = parts_of(said).expect("a structure");
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0].number, "1");
        assert_eq!(parts[0].kind, "text");
        assert_eq!(parts[0].bytes, 1234);
        assert!(parts[0].is_text());
        assert_eq!(parts[1].number, "2");
        assert_eq!(parts[1].name, "tour.jpg");
        assert_eq!(parts[1].bytes, 284000);
        assert!(parts[1].attached);
        assert!(!parts[1].is_text());
    }

    /// The common newsletter: text and html inside an alternative, inside
    /// a mixed with the freight. The faces are `1.1` and `1.2`, and
    /// nothing but the numbers can say so.
    #[test]
    fn nested_parts_are_numbered_the_way_body_takes_them() {
        let said = "* 1 FETCH (BODYSTRUCTURE (((\"text\" \"plain\" NIL NIL NIL \"7bit\" 10 1)(\"text\" \"html\" NIL NIL NIL \"7bit\" 20 1) \"alternative\")(\"application\" \"pdf\" (\"name\" \"d.pdf\") NIL NIL \"base64\" 900 NIL (\"attachment\" (\"filename\" \"d.pdf\")) NIL) \"mixed\"))";
        let parts = parts_of(said).expect("a structure");
        let numbers: Vec<&str> = parts.iter().map(|p| p.number.as_str()).collect();
        assert_eq!(numbers, vec!["1.1", "1.2", "2"]);
        let text: Vec<&str> = parts
            .iter()
            .filter(|p| p.is_text())
            .map(|p| p.number.as_str())
            .collect();
        assert_eq!(text, vec!["1.1", "1.2"]);
    }

    /// A message with no multipart at all is one part, and it is part 1.
    #[test]
    fn a_plain_message_is_part_one() {
        let said = "1 FETCH (BODYSTRUCTURE (\"text\" \"plain\" (\"charset\" \"utf-8\") NIL NIL \"quoted-printable\" 3277 60))";
        let parts = parts_of(said).expect("a structure");
        assert_eq!(parts.len(), 1);
        assert_eq!(parts[0].number, "1");
        assert!(parts[0].is_text());
        assert_eq!(parts[0].bytes, 3277);
    }

    /// Text that says it is attached is freight, whatever its type.
    #[test]
    fn an_attached_text_file_is_not_the_letter() {
        let said = "1 FETCH (BODYSTRUCTURE ((\"text\" \"plain\" NIL NIL NIL \"7bit\" 10 1)(\"text\" \"csv\" (\"name\" \"rows.csv\") NIL NIL \"base64\" 900 NIL (\"attachment\" (\"filename\" \"rows.csv\")) NIL) \"mixed\"))";
        let parts = parts_of(said).expect("a structure");
        assert!(parts[0].is_text());
        assert!(!parts[1].is_text());
        assert_eq!(parts[1].name, "rows.csv");
    }

    /// Nonsense, and a response with no structure at all, both say so
    /// rather than guessing: the caller then fetches the whole message.
    #[test]
    fn what_does_not_parse_says_so() {
        assert!(parts_of("1 FETCH (UID 7 FLAGS ())").is_none());
        assert!(parts_of("1 FETCH (BODYSTRUCTURE ((((").is_none());
    }

    /// A filename sent as a literal rather than a quoted string.
    #[test]
    fn a_literal_filename_is_read_like_a_quoted_one() {
        let said = "1 FETCH (BODYSTRUCTURE ((\"text\" \"plain\" NIL NIL NIL \"7bit\" 10 1)(\"image\" \"jpeg\" (\"name\" {9}\r\nphoto.jpg) NIL NIL \"base64\" 900 NIL (\"attachment\" (\"filename\" {9}\r\nphoto.jpg)) NIL) \"mixed\"))";
        let parts = parts_of(said).expect("a structure");
        assert_eq!(parts[1].name, "photo.jpg");
    }
}
