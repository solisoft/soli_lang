//! `local("…")`: a small statement language compiled to `spec/07` bytecode.
//!
//! ```text
//! state.count += 1
//! value.text = str(state.count)
//! if state.count > 9 { badge.style = @hot } else { badge.style = @cool }
//! self.style = @hover
//! theme.toggle()
//! emit("increment")
//! ```
//!
//! Targets are node keys (`value`, `"my key"`, or `self` for the node that
//! carries the handler); `state.x` is a root prop; `@name` is a style the
//! handler declared in its `styles` map. Nothing here can do anything the
//! bytecode cannot, and the bytecode can do very little — that is the point.

use std::collections::HashMap;

use eui_proto::Writer;

/// What the compiler needs from the encoder.
pub trait Ctx {
    /// Intern a string.
    fn atom(&mut self, s: &str) -> u32;
    /// A declared style by name, as a style table id.
    fn style(&mut self, name: &str) -> Option<u32>;
    /// The key of the node carrying the handler, for `self`.
    fn self_key(&self) -> Option<String>;
}

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Ident(String),
    Int(i64),
    Str(String),
    At(String),
    Sym(&'static str),
}

fn lex(src: &str) -> Result<Vec<Tok>, String> {
    let b = src.as_bytes();
    let mut i = 0;
    let mut out = Vec::new();
    let sym2 = ["==", "!=", "<=", ">=", "&&", "||", "+=", "-="];
    let sym1 = [
        "+", "-", "*", "<", ">", "!", "=", "(", ")", "{", "}", ".", ";", ",",
    ];
    while i < b.len() {
        let c = b[i] as char;
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        if c == '#' {
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if c.is_ascii_digit() {
            let s = i;
            while i < b.len() && b[i].is_ascii_digit() {
                i += 1;
            }
            out.push(Tok::Int(
                src[s..i].parse().map_err(|_| "integer too large")?,
            ));
            continue;
        }
        if c.is_ascii_alphabetic() || c == '_' {
            let s = i;
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                i += 1;
            }
            out.push(Tok::Ident(src[s..i].to_string()));
            continue;
        }
        if c == '@' {
            i += 1;
            let s = i;
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                i += 1;
            }
            if s == i {
                return Err("expected a style name after @".into());
            }
            out.push(Tok::At(src[s..i].to_string()));
            continue;
        }
        if c == '"' {
            i += 1;
            let s = i;
            while i < b.len() && b[i] != b'"' {
                i += 1;
            }
            if i >= b.len() {
                return Err("unterminated string".into());
            }
            out.push(Tok::Str(src[s..i].to_string()));
            i += 1;
            continue;
        }
        if i + 1 < b.len() {
            if let Some(s) = sym2.iter().find(|s| s.as_bytes() == &b[i..i + 2]) {
                out.push(Tok::Sym(s));
                i += 2;
                continue;
            }
        }
        if let Some(s) = sym1.iter().find(|s| s.as_bytes()[0] == b[i]) {
            out.push(Tok::Sym(s));
            i += 1;
            continue;
        }
        return Err(format!("unexpected character '{c}'"));
    }
    Ok(out)
}

/// One emitted instruction, jumps still symbolic.
enum Emit {
    Bytes(Vec<u8>),
    Jump(u8, usize),
    Label(usize),
}

struct Compiler<'a, C: Ctx> {
    toks: Vec<Tok>,
    pos: usize,
    ctx: &'a mut C,
    code: Vec<Emit>,
    labels: usize,
}

impl<C: Ctx> Compiler<'_, C> {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.pos)
    }
    fn next(&mut self) -> Option<Tok> {
        let t = self.toks.get(self.pos).cloned();
        self.pos += 1;
        t
    }
    fn eat(&mut self, s: &str) -> bool {
        if matches!(self.peek(), Some(Tok::Sym(x)) if *x == s) {
            self.pos += 1;
            true
        } else {
            false
        }
    }
    fn expect(&mut self, s: &str) -> Result<(), String> {
        if self.eat(s) {
            Ok(())
        } else {
            Err(format!("expected '{s}'"))
        }
    }
    fn op(&mut self, opcode: u8) {
        self.code.push(Emit::Bytes(vec![opcode]));
    }
    fn op_varint(&mut self, opcode: u8, v: u32) {
        let mut w = Writer::new();
        w.u8(opcode).varint32(v);
        self.code.push(Emit::Bytes(w.into_vec()));
    }
    fn label(&mut self) -> usize {
        self.labels += 1;
        self.labels
    }

    fn program(&mut self) -> Result<(), String> {
        while self.peek().is_some() {
            self.stmt()?;
            let _ = self.eat(";");
        }
        Ok(())
    }

    fn block(&mut self) -> Result<(), String> {
        self.expect("{")?;
        while !matches!(self.peek(), Some(Tok::Sym("}"))) {
            if self.peek().is_none() {
                return Err("unterminated block".into());
            }
            self.stmt()?;
            let _ = self.eat(";");
        }
        self.expect("}")
    }

    fn stmt(&mut self) -> Result<(), String> {
        match self.next() {
            Some(Tok::Ident(k)) if k == "if" => {
                self.expr()?;
                let l_else = self.label();
                let l_end = self.label();
                self.code.push(Emit::Jump(0x21, l_else));
                self.block()?;
                self.code.push(Emit::Jump(0x20, l_end));
                self.code.push(Emit::Label(l_else));
                if matches!(self.peek(), Some(Tok::Ident(e)) if e == "else") {
                    self.pos += 1;
                    self.block()?;
                }
                self.code.push(Emit::Label(l_end));
                Ok(())
            }
            Some(Tok::Ident(k)) if k == "emit" => {
                self.expect("(")?;
                let name = match self.next() {
                    Some(Tok::Str(s)) => s,
                    _ => return Err("emit takes a string".into()),
                };
                self.expect(")")?;
                let a = self.ctx.atom(&name);
                self.op_varint(0x32, a);
                Ok(())
            }
            Some(Tok::Ident(k)) if k == "theme" => {
                // theme.mode = expr | theme.toggle()
                self.expect(".")?;
                match self.next() {
                    Some(Tok::Ident(w)) if w == "mode" => {
                        self.expect("=")?;
                        self.expr()?;
                    }
                    Some(Tok::Ident(w)) if w == "toggle" => {
                        self.expect("(")?;
                        self.expect(")")?;
                        let a = self.ctx.atom("toggle");
                        self.op_varint(0x02, a);
                    }
                    _ => return Err("expected theme.mode = … or theme.toggle()".into()),
                }
                self.op(0x34);
                Ok(())
            }
            Some(Tok::Ident(k)) if k == "state" => {
                self.expect(".")?;
                let field = match self.next() {
                    Some(Tok::Ident(f)) => f,
                    _ => return Err("state.<field>".into()),
                };
                let atom = self.ctx.atom(&field);
                match self.next() {
                    Some(Tok::Sym("=")) => {
                        self.expr()?;
                    }
                    Some(Tok::Sym("+=")) => {
                        self.op_varint(0x04, atom);
                        self.expr()?;
                        self.op(0x10);
                    }
                    Some(Tok::Sym("-=")) => {
                        self.op_varint(0x04, atom);
                        self.expr()?;
                        self.op(0x11);
                    }
                    _ => return Err("expected =, += or -= after state field".into()),
                }
                self.op_varint(0x05, atom);
                Ok(())
            }
            Some(Tok::Ident(k)) | Some(Tok::Str(k)) => {
                // <key>.text = expr | <key>.style = @name
                //
                // `self` is atom 0 and not the node's own key. A key baked
                // into the source would make the compile depend on it, and
                // the compile is cached on (source, key, styles) — so a
                // keyed node with a local handler would be one interned
                // chunk *per key*, even with byte-identical source. A list
                // with a hover on its rows would then end the session after
                // four thousand of them ("interned more than 4095 chunks").
                // Atom 0 is free: an unkeyed node is never entered in the
                // client's key map, so nothing else can answer to it.
                let mut here = false;
                let key = if k == "self" {
                    here = true;
                    String::new()
                } else {
                    k
                };
                self.expect(".")?;
                let what = match self.next() {
                    Some(Tok::Ident(w)) => w,
                    _ => return Err("expected .text or .style".into()),
                };
                self.expect("=")?;
                let key_atom = if here { 0 } else { self.ctx.atom(&key) };
                match what.as_str() {
                    "text" => {
                        self.expr()?;
                        self.op_varint(0x30, key_atom);
                        Ok(())
                    }
                    "style" => {
                        let name = match self.next() {
                            Some(Tok::At(n)) => n,
                            _ => {
                                return Err(
                                    "a style is @name, declared in the handler's styles".into()
                                )
                            }
                        };
                        let id = self
                            .ctx
                            .style(&name)
                            .ok_or_else(|| format!("undeclared style @{name}"))?;
                        let mut w = Writer::new();
                        w.u8(0x33).varint32(key_atom).varint32(id);
                        self.code.push(Emit::Bytes(w.into_vec()));
                        Ok(())
                    }
                    other => Err(format!("cannot assign .{other}; only .text and .style")),
                }
            }
            other => Err(format!("unexpected {other:?}")),
        }
    }

    fn expr(&mut self) -> Result<(), String> {
        self.or()
    }
    fn or(&mut self) -> Result<(), String> {
        self.and()?;
        while self.eat("||") {
            self.and()?;
            self.op(0x19);
        }
        Ok(())
    }
    fn and(&mut self) -> Result<(), String> {
        self.eq()?;
        while self.eat("&&") {
            self.eq()?;
            self.op(0x18);
        }
        Ok(())
    }
    fn eq(&mut self) -> Result<(), String> {
        self.cmp()?;
        loop {
            if self.eat("==") {
                self.cmp()?;
                self.op(0x15);
            } else if self.eat("!=") {
                self.cmp()?;
                self.op(0x15);
                self.op(0x14);
            } else {
                return Ok(());
            }
        }
    }
    fn cmp(&mut self) -> Result<(), String> {
        self.add()?;
        loop {
            if self.eat("<") {
                self.add()?;
                self.op(0x16);
            } else if self.eat(">") {
                self.add()?;
                self.op(0x17);
            } else if self.eat("<=") {
                self.add()?;
                self.op(0x17);
                self.op(0x14);
            } else if self.eat(">=") {
                self.add()?;
                self.op(0x16);
                self.op(0x14);
            } else {
                return Ok(());
            }
        }
    }
    fn add(&mut self) -> Result<(), String> {
        self.mul()?;
        loop {
            if self.eat("+") {
                self.mul()?;
                self.op(0x10);
            } else if self.eat("-") {
                self.mul()?;
                self.op(0x11);
            } else {
                return Ok(());
            }
        }
    }
    fn mul(&mut self) -> Result<(), String> {
        self.unary()?;
        while self.eat("*") {
            self.unary()?;
            self.op(0x12);
        }
        Ok(())
    }
    fn unary(&mut self) -> Result<(), String> {
        if self.eat("!") {
            self.unary()?;
            self.op(0x14);
            return Ok(());
        }
        if self.eat("-") {
            self.unary()?;
            self.op(0x13);
            return Ok(());
        }
        self.primary()
    }
    fn primary(&mut self) -> Result<(), String> {
        match self.next() {
            Some(Tok::Int(n)) => {
                let mut w = Writer::new();
                w.u8(0x01).svarint(n);
                self.code.push(Emit::Bytes(w.into_vec()));
                Ok(())
            }
            Some(Tok::Str(s)) => {
                let a = self.ctx.atom(&s);
                self.op_varint(0x02, a);
                Ok(())
            }
            Some(Tok::Ident(i)) if i == "true" || i == "false" => {
                self.code
                    .push(Emit::Bytes(vec![0x03, u8::from(i == "true")]));
                Ok(())
            }
            Some(Tok::Ident(i)) if i == "state" => {
                self.expect(".")?;
                let field = match self.next() {
                    Some(Tok::Ident(f)) => f,
                    _ => return Err("state.<field>".into()),
                };
                let a = self.ctx.atom(&field);
                self.op_varint(0x04, a);
                Ok(())
            }
            Some(Tok::Ident(i)) if i == "str" => {
                self.expect("(")?;
                self.expr()?;
                self.expect(")")?;
                self.op(0x1A);
                Ok(())
            }
            Some(Tok::Sym("(")) => {
                self.expr()?;
                self.expect(")")
            }
            other => Err(format!("unexpected {other:?} in expression")),
        }
    }

    /// Resolve labels and produce the chunk.
    fn finish(self) -> Result<Vec<u8>, String> {
        // Sizes: labels are zero bytes, jumps three.
        let mut offsets = Vec::with_capacity(self.code.len());
        let mut label_at: HashMap<usize, usize> = HashMap::new();
        let mut off = 0usize;
        for e in &self.code {
            offsets.push(off);
            match e {
                Emit::Bytes(b) => off += b.len(),
                Emit::Jump(..) => off += 3,
                Emit::Label(l) => {
                    label_at.insert(*l, off);
                }
            }
        }
        let mut out = b"EUIC".to_vec();
        out.push(1);
        out.push(16);
        for (i, e) in self.code.iter().enumerate() {
            match e {
                Emit::Bytes(b) => out.extend_from_slice(b),
                Emit::Jump(op, l) => {
                    let target = *label_at.get(l).ok_or("internal: unresolved label")? as i64;
                    let next = offsets[i] as i64 + 3;
                    let rel = i16::try_from(target - next).map_err(|_| "jump too far")?;
                    out.push(*op);
                    out.extend_from_slice(&rel.to_le_bytes());
                }
                Emit::Label(_) => {}
            }
        }
        // Spec 07 §4: a chunk must end in `return` or a jump. What decides
        // that is the last *instruction*, not the last byte — a `set_style`
        // whose style id is 32 or 64 ends in a 0x20 or 0x40 operand, and
        // reading that as the opcode left the chunk without its `return`
        // and the client rejecting the whole handler.
        let terminated = match self.code.last() {
            Some(Emit::Bytes(b)) => b.first() == Some(&0x40),
            Some(Emit::Jump(op, _)) => *op == 0x20,
            _ => false,
        };
        if !terminated {
            out.push(0x40);
        }
        Ok(out)
    }
}

/// Compile a `local("…")` program.
pub fn compile<C: Ctx>(src: &str, ctx: &mut C) -> Result<Vec<u8>, String> {
    let toks = lex(src).map_err(|e| format!("EUI: local handler: {e}"))?;
    let mut c = Compiler {
        toks,
        pos: 0,
        ctx,
        code: Vec::new(),
        labels: 0,
    };
    c.program()
        .map_err(|e| format!("EUI: local handler: {e}"))?;
    c.finish().map_err(|e| format!("EUI: local handler: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestCtx {
        atoms: Vec<String>,
        styles: HashMap<String, u32>,
    }
    impl Ctx for TestCtx {
        fn atom(&mut self, s: &str) -> u32 {
            if let Some(i) = self.atoms.iter().position(|a| a == s) {
                return i as u32 + 1;
            }
            self.atoms.push(s.to_string());
            self.atoms.len() as u32
        }
        fn style(&mut self, name: &str) -> Option<u32> {
            self.styles.get(name).copied()
        }
        fn self_key(&self) -> Option<String> {
            Some("me".into())
        }
    }

    #[test]
    fn the_counter_compiles_to_the_expected_bytes() {
        let mut ctx = TestCtx {
            atoms: vec![],
            styles: HashMap::new(),
        };
        let bytes = compile("state.count += 1\nvalue.text = str(state.count)", &mut ctx).unwrap();
        // count=1, value=2
        assert_eq!(&bytes[..6], b"EUIC\x01\x10");
        assert_eq!(
            &bytes[6..],
            &[0x04, 1, 0x01, 2, 0x10, 0x05, 1, 0x04, 1, 0x1A, 0x30, 2, 0x40]
        );
    }

    #[test]
    fn if_else_and_styles_and_self() {
        let mut ctx = TestCtx {
            atoms: vec![],
            styles: HashMap::from([("hot".into(), 7u32), ("cool".into(), 8)]),
        };
        let bytes = compile(
            "if state.n > 9 { self.style = @hot } else { self.style = @cool }; emit(\"ping\")",
            &mut ctx,
        )
        .unwrap();
        // Ends with a return, contains both set_style ops and one emit.
        assert_eq!(*bytes.last().unwrap(), 0x40);
        assert_eq!(bytes.iter().filter(|b| **b == 0x33).count(), 2);
        // `self` interns nothing now — it compiles to atom 0, resolved by
        // the client against the node the handler is on — so `ping` is the
        // second atom this chunk asks for, not the third.
        assert!(
            bytes.windows(2).any(|w| w == [0x32, 2]),
            "emit atom 2 = ping (n=1); `self` is atom 0 and interns nothing"
        );
        // And every `set_style` names node 0, which is what makes the chunk
        // independent of the key it sits on.
        assert!(
            bytes.windows(2).filter(|w| w[0] == 0x33).all(|w| w[1] == 0),
            "self is node 0"
        );
    }

    /// Spec 07 §4: the client checks that a chunk ends in `return` or a
    /// jump, and a `set_style` whose style id is 64 ends in the byte 0x40.
    /// Deciding on the byte rather than the instruction dropped the
    /// `return`, and the client threw the handler away.
    #[test]
    fn a_style_id_that_looks_like_return_still_gets_one() {
        for id in [0x20u32, 0x40] {
            let mut ctx = TestCtx {
                atoms: vec![],
                styles: HashMap::from([("lit".into(), id)]),
            };
            let bytes = compile("band.style = @lit", &mut ctx).unwrap();
            assert_eq!(
                &bytes[6..],
                &[0x33, 1, id as u8, 0x40],
                "style {id:#x}: set_style, then the return the verifier wants"
            );
        }
    }

    #[test]
    fn errors_are_named() {
        let mut ctx = TestCtx {
            atoms: vec![],
            styles: HashMap::new(),
        };
        assert!(compile("value.style = @nope", &mut ctx)
            .unwrap_err()
            .contains("undeclared style"));
        assert!(compile("value.colour = 1", &mut ctx)
            .unwrap_err()
            .contains("only .text and .style"));
        assert!(compile("state.x = \"open", &mut ctx)
            .unwrap_err()
            .contains("unterminated"));
        assert!(compile("state.x = 1 +", &mut ctx)
            .unwrap_err()
            .contains("unexpected"));
    }
}
