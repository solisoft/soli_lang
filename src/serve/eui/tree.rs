//! From the view's hash to nodes, with the session's tables.
//!
//! The view returns plain data:
//!
//! ```text
//! { "k": "box", "s": { "display": "column", "gap": 4, "bg": "surface.base" },
//!   "key": "row-12", "t": "text content", "on": { "click": "increment" },
//!   "p": { "item_height": 20 }, "c": [ ...children... ] }
//! ```
//!
//! Style values are the spec's own vocabulary — role names, scale indices,
//! px — so a view author never sees a `StyleRecord`. Every string that will
//! repeat is interned into the session's atom table; every distinct style is
//! interned once; both tables only ever grow, exactly as the wire format
//! requires.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use crate::interpreter::value::{HashKey, Value};

use eui_proto::{
    AlignItems, AlignSelf, Batch, ColorRef, Cursor, Dim, Display, EventKind, FlatNode, FontFamily, FontWeight,
    Handler, Justify, NodeKind, Op, Overflow, Position, StyleRecord, Subtree, TextAlign, TextRef, Value as WireValue,
    Wrap, Writer,
};
use serde_json::Value as Json;

/// A node with children, before ids are settled.
#[derive(Debug, Clone, PartialEq)]
pub struct TNode {
    /// Server id; `0` until the diff assigns one.
    pub id: u32,
    /// Kind.
    pub kind: NodeKind,
    /// Style table id, `0` for the default record.
    pub style: u32,
    /// Reconciliation key, if given.
    pub key: Option<String>,
    /// The key as an atom, sent on the wire so a local handler can name the
    /// node; `0` when unkeyed.
    pub key_atom: u32,
    /// Text content.
    pub text: Option<TextRef>,
    /// `(name atom, value)`.
    pub props: Vec<(u32, WireValue)>,
    /// `(event, handler)`.
    pub handlers: Vec<(EventKind, Handler)>,
    /// Children in order.
    pub children: Vec<Child>,
    /// The address of the view value this keyed node came from, `0` when
    /// unkeyed or converted from JSON. What the memo is keyed on.
    pub identity: usize,
    /// Nodes in this subtree, itself included; settled when the node is
    /// frozen, `0` before — so counting a tree costs its fresh part only.
    pub size: u32,
}

/// A child of a tree node: freshly converted this render, or kept from the
/// last one because the view returned the very same object for it. A kept
/// child already carries its ids; a diff of a kept child against itself is
/// a no-op, which is what makes a like on a feed of ten thousand cards cost
/// one card.
#[derive(Debug, Clone, PartialEq)]
pub enum Child {
    Fresh(TNode),
    Kept(Arc<TNode>),
}

impl Child {
    /// The node, whichever way it is held.
    pub fn node(&self) -> &TNode {
        match self {
            Child::Fresh(n) => n,
            Child::Kept(n) => n,
        }
    }

    /// The node for mutation: a kept one is copied first, its ids intact.
    pub fn node_mut(&mut self) -> &mut TNode {
        if let Child::Kept(rc) = self {
            *self = Child::Fresh((**rc).clone());
        }
        match self {
            Child::Fresh(n) => n,
            Child::Kept(_) => unreachable!("just made fresh"),
        }
    }

    /// Both hold the same kept node.
    pub fn same_kept(&self, other: &Child) -> bool {
        matches!((self, other), (Child::Kept(a), Child::Kept(b)) if Arc::ptr_eq(a, b))
    }
}

impl std::ops::Deref for Child {
    type Target = TNode;
    fn deref(&self) -> &TNode {
        self.node()
    }
}

impl From<TNode> for Child {
    fn from(n: TNode) -> Self {
        Child::Fresh(n)
    }
}

/// One session's tables and previous tree.
#[derive(Debug, Default)]
pub struct Encoder {
    atoms: HashMap<String, u32>,
    styles: HashMap<[u8; 64], u32>,
    colors: HashMap<u32, u32>,
    chunks: HashMap<Vec<u8>, u32>,
    pending: Vec<Op>,
    next_node: u32,
    seq: u64,
    prev: Option<TNode>,
    /// Which session this is, for the thread-local memo.
    session: String,
    generation: u32,
}

/// One session's kept subtrees, by the address of the view value each came
/// from — with that value pinned, so the address cannot be reused by
/// another object while the entry lives. Values are `Rc`, so this lives on
/// the thread that evaluates the view; a render on another thread simply
/// misses, which is safe.
#[derive(Default)]
struct Memo {
    entries: HashMap<usize, (Value, Arc<TNode>, u32)>,
}

thread_local! {
    static MEMOS: RefCell<HashMap<String, Memo>> = RefCell::new(HashMap::new());
}

fn with_memo<R>(session: &str, f: impl FnOnce(&mut Memo) -> R) -> R {
    MEMOS.with(|m| f(m.borrow_mut().entry(session.to_owned()).or_default()))
}

/// Drop a session's memo when the session goes.
pub fn forget_session(session: &str) {
    MEMOS.with(|m| {
        m.borrow_mut().remove(session);
    });
}

fn count_nodes(t: &TNode) -> usize {
    1 + t
        .children
        .iter()
        .map(|c| match c {
            Child::Kept(k) if k.size != 0 => k.size as usize,
            c => count_nodes(c),
        })
        .sum::<usize>()
}

/// Ops per batch before an update is streamed as several. A slice of a
/// thousand ops is a few hundred kilobytes of cards and a few milliseconds
/// to apply — one frame's worth.
pub const MAX_OPS_PER_BATCH: usize = 1000;

impl Encoder {
    /// Intern a string, emitting its definition on first use.
    pub fn atom(&mut self, s: &str) -> u32 {
        if let Some(id) = self.atoms.get(s) {
            return *id;
        }
        let id = self.atoms.len() as u32 + 1;
        self.atoms.insert(s.to_string(), id);
        self.pending.push(Op::DefAtom { id, value: s.to_string() });
        id
    }

    fn atom_name(&self, id: u32) -> Option<&str> {
        self.atoms.iter().find(|(_, v)| **v == id).map(|(k, _)| k.as_str())
    }

    fn color_literal(&mut self, rgba: u32) -> ColorRef {
        if let Some(id) = self.colors.get(&rgba) {
            return ColorRef::literal(*id as u16);
        }
        let id = self.colors.len() as u32 + 1;
        self.colors.insert(rgba, id);
        self.pending.push(Op::DefColor { id, rgba });
        ColorRef::literal(id as u16)
    }

    /// Intern a chunk by its bytes, delivering it inline on first use.
    fn chunk(&mut self, bytes: Vec<u8>) -> u32 {
        if let Some(id) = self.chunks.get(&bytes) {
            return *id;
        }
        let id = self.chunks.len() as u32 + 1;
        self.chunks.insert(bytes.clone(), id);
        self.pending.push(Op::DefChunkBytes { id, bytes });
        id
    }

    /// Assemble a local handler written as data (`spec/07-bytecode.md`):
    ///
    /// ```text
    /// [["load", "count"], ["push", 1], ["add"], ["dup"], ["store", "count"],
    ///  ["to_str"], ["set_text", "value"], ["emit", "increment"]]
    /// ```
    ///
    /// Node targets are keys, sent as atoms; the client resolves them, so a
    /// chunk does not depend on any one render's ids and is interned once.
    /// Branches take a label: `["label", "else"]`, `["jump", "else"]`,
    /// `["jump_if_false", "else"]`.
    fn assemble(&mut self, program: &[Json]) -> Result<Vec<u8>, String> {
        let bad = |what: &str| format!("EUI: local handler: {what}");
        // Pass 1: sizes and labels, so jumps can be resolved.
        let mut labels: HashMap<String, usize> = HashMap::new();
        let mut sized: Vec<(Vec<u8>, Option<(u8, String)>)> = Vec::new(); // (bytes, pending jump)
        let mut offset = 0usize;
        for instr in program {
            let parts = instr.as_array().ok_or_else(|| bad("each instruction is a list"))?;
            let name = parts.first().and_then(Json::as_str).ok_or_else(|| bad("instruction name"))?;
            let arg = parts.get(1);
            let atom_arg = |enc: &mut Self| -> Result<u32, String> {
                let s = arg.and_then(Json::as_str).ok_or_else(|| bad("expected a name"))?;
                Ok(enc.atom(s))
            };
            let node_arg = |enc: &mut Self| -> Result<u32, String> {
                let key = arg.ok_or_else(|| bad("expected a node key"))?;
                let key = match key { Json::String(s) => s.clone(), other => other.to_string() };
                Ok(enc.atom(&key))
            };
            let mut w = Writer::new();
            let mut jump = None;
            match name {
                "push" => match arg {
                    Some(Json::Number(n)) => { w.u8(0x01).svarint(n.as_i64().ok_or_else(|| bad("integer"))?); }
                    Some(Json::Bool(b)) => { w.u8(0x03).u8(u8::from(*b)); }
                    Some(Json::String(s)) => { let a = self.atom(s); w.u8(0x02).varint32(a); }
                    _ => return Err(bad("push takes a number, bool or string")),
                },
                "load" => { let a = atom_arg(self)?; w.u8(0x04).varint32(a); }
                "store" => { let a = atom_arg(self)?; w.u8(0x05).varint32(a); }
                "dup" => { w.u8(0x06); }
                "pop" => { w.u8(0x07); }
                "add" => { w.u8(0x10); }
                "sub" => { w.u8(0x11); }
                "mul" => { w.u8(0x12); }
                "neg" => { w.u8(0x13); }
                "not" => { w.u8(0x14); }
                "eq" => { w.u8(0x15); }
                "lt" => { w.u8(0x16); }
                "gt" => { w.u8(0x17); }
                "and" => { w.u8(0x18); }
                "or" => { w.u8(0x19); }
                "to_str" => { w.u8(0x1A); }
                "concat" => { w.u8(0x1B); }
                "label" => {
                    let l = arg.and_then(Json::as_str).ok_or_else(|| bad("label name"))?;
                    labels.insert(l.to_string(), offset);
                    continue;
                }
                "jump" | "jump_if_false" => {
                    let l = arg.and_then(Json::as_str).ok_or_else(|| bad("jump label"))?;
                    w.u8(if name == "jump" { 0x20 } else { 0x21 }).u16(0);
                    jump = Some((if name == "jump" { 0x20 } else { 0x21 }, l.to_string()));
                }
                "set_text" => { let n = node_arg(self)?; w.u8(0x30).varint32(n); }
                "set_prop" => {
                    let n = node_arg(self)?;
                    let p = parts.get(2).and_then(Json::as_str).ok_or_else(|| bad("set_prop needs a prop name"))?;
                    let a = self.atom(p);
                    w.u8(0x31).varint32(n).varint32(a);
                }
                "emit" => { let a = atom_arg(self)?; w.u8(0x32).varint32(a); }
                "set_style" => {
                    let n = node_arg(self)?;
                    let st = parts.get(2).ok_or_else(|| bad("set_style needs a style hash"))?;
                    let record = self.style_record(st)?;
                    let id = self.style(record);
                    w.u8(0x33).varint32(n).varint32(id);
                }
                "return" => { w.u8(0x40); }
                other => return Err(bad(&format!("unknown instruction '{other}'"))),
            }
            offset += w.len();
            sized.push((w.into_vec(), jump));
        }
        // Pass 2: resolve jumps relative to the next instruction.
        let mut code = Vec::with_capacity(offset);
        for (bytes, jump) in sized {
            let mut bytes = bytes;
            if let Some((_, label)) = jump {
                let target = *labels.get(&label).ok_or_else(|| bad(&format!("unknown label '{label}'")))? as i64;
                let next = code.len() as i64 + bytes.len() as i64;
                let rel = i16::try_from(target - next).map_err(|_| bad("jump too far"))?;
                bytes[1..3].copy_from_slice(&rel.to_le_bytes());
            }
            code.extend_from_slice(&bytes);
        }
        if !matches!(code.last(), Some(0x40 | 0x20)) {
            code.push(0x40);
        }
        let mut out = b"EUIC".to_vec();
        out.push(1);
        out.push(16); // max stack; the client verifier proves the real depth fits
        out.extend_from_slice(&code);
        Ok(out)
    }

    fn style(&mut self, record: StyleRecord) -> u32 {
        if record == StyleRecord::default() {
            return 0;
        }
        let mut w = Writer::new();
        record.encode(&mut w);
        let mut key = [0u8; 64];
        key.copy_from_slice(w.as_slice());
        if let Some(id) = self.styles.get(&key) {
            return *id;
        }
        let id = self.styles.len() as u32 + 1;
        self.styles.insert(key, id);
        self.pending.push(Op::DefStyle { id, record });
        id
    }

    /// A fresh node id.
    pub fn fresh_id(&mut self) -> u32 {
        self.next_node += 1;
        self.next_node
    }

    /// Turn a view result into the next batch: definitions first, then a
    /// `Mount` (first render, or resync) or the diff against the previous tree.
    pub fn render(&mut self, json: &Json, resync: bool) -> Result<Vec<Batch>, String> {
        self.generation = self.generation.wrapping_add(1);
        let tree = self.convert(json)?;
        self.finish(tree, resync, HashMap::new())
    }

    /// [`Encoder::render`] straight from the view's value: no JSON in
    /// between, and a keyed child that is the same object as last render
    /// is kept — not converted, not diffed.
    pub fn render_value(&mut self, session: &str, value: &Value, resync: bool) -> Result<Vec<Batch>, String> {
        self.session = session.to_owned();
        self.generation = self.generation.wrapping_add(1);
        // The view values behind this render's keyed nodes, by identity,
        // until `finish` pins them in the memo. Held here, not on the
        // encoder: an interpreter value cannot cross threads.
        let mut pins: HashMap<usize, Value> = HashMap::new();
        let tree = match self.convert_value(value, &mut pins)? {
            Some(Child::Fresh(n)) => n,
            Some(Child::Kept(rc)) => (*rc).clone(),
            None => return Err("EUI: the view returned nothing".into()),
        };
        self.finish(tree, resync, pins)
    }

    fn finish(&mut self, mut tree: TNode, resync: bool, mut pins: HashMap<usize, Value>) -> Result<Vec<Batch>, String> {
        // The client refuses a tree past its node limit and there is no way
        // to send it anyway; say so here, where the application can act.
        let nodes = count_nodes(&tree);
        if nodes > eui_proto::limits::MAX_NODES as usize {
            eprintln!("[EUI] the view returned {nodes} nodes; a client accepts at most {} — virtualise, paginate or trim", eui_proto::limits::MAX_NODES);
        }
        let ops = match (self.prev.take(), resync) {
            (Some(prev), false) => {
                let mut ops = Vec::new();
                super::diff::diff(self, &prev, &mut tree, &mut ops);
                ops
            }
            _ => {
                assign_fresh_ids(self, &mut tree);
                vec![Op::Mount(flatten(&tree))]
            }
        };
        // Keyed nodes converted this render are frozen behind an Arc, so the
        // next render can keep them by the identity of their view value.
        let generation = self.generation;
        if !self.session.is_empty() {
            let session = self.session.clone();
            with_memo(&session, |memo| {
                freeze(&mut tree, memo, &mut pins, generation);
                memo.entries.retain(|_, (_, _, seen)| *seen == generation);
            });
        }
        self.prev = Some(tree);
        let mut all = std::mem::take(&mut self.pending);
        all.extend(ops);
        // A big update streams: every prefix of the op list is valid on its
        // own (definitions come first, ops apply in order), so a batch of
        // thousands of inserts goes out in slices the client paints between.
        // The first slice is on screen long before the last is encoded.
        let mut batches = Vec::new();
        if all.len() <= MAX_OPS_PER_BATCH {
            self.seq += 1;
            batches.push(Batch { seq: self.seq, ops: all });
        } else {
            let mut rest = all;
            while !rest.is_empty() {
                let take = rest.len().min(MAX_OPS_PER_BATCH);
                let tail = rest.split_off(take);
                self.seq += 1;
                batches.push(Batch { seq: self.seq, ops: rest });
                rest = tail;
            }
        }
        Ok(batches)
    }

    /// The server-side name of the handler for `(node, event)` in the tree
    /// the client last received — `None` if the client is making it up.
    pub fn handler_name(&self, node: u32, event: EventKind) -> Option<String> {
        let tree = self.prev.as_ref()?;
        let found = find(tree, node)?;
        let atom = found.handlers.iter().find(|(e, _)| *e == event).and_then(|(_, h)| match h {
            Handler::Server(a) | Handler::LocalThenServer { name: a, .. } => Some(*a),
            Handler::Local(_) => None,
        })?;
        self.atom_name(atom).map(str::to_owned)
    }

    /// A node's props with atom names resolved, as JSON, for the handler's
    /// `params["props"]`. This is how a row in a list says which row it is.
    pub fn props_of(&self, node: u32) -> serde_json::Map<String, Json> {
        let mut out = serde_json::Map::new();
        let Some(tree) = self.prev.as_ref() else { return out };
        let Some(found) = find(tree, node) else { return out };
        for (atom, value) in &found.props {
            if let Some(name) = self.atom_name(*atom) {
                out.insert(name.to_string(), wire_to_json(self, value));
            }
        }
        out
    }

    /// A view value to a child: `None` for a `nil`/`false` in a child list
    /// (an `x if cond` that read as nothing), a kept child when the keyed
    /// value is the very object seen last render, else a fresh conversion.
    fn convert_value(&mut self, v: &Value, pins: &mut HashMap<usize, Value>) -> Result<Option<Child>, String> {
        let hash = match v {
            Value::Hash(h) => h,
            Value::Null | Value::Bool(false) => return Ok(None),
            other => return Err(format!("EUI: a node must be a hash, got {}", other.type_name())),
        };
        let identity = Rc::as_ptr(hash) as *const u8 as usize;
        let keyed = hash.borrow().contains_key(&HashKey::String("key".into()));
        if keyed && !self.session.is_empty() {
            let generation = self.generation;
            let kept = with_memo(&self.session, |memo| {
                memo.entries.get_mut(&identity).map(|(_, arc, seen)| {
                    *seen = generation;
                    Arc::clone(arc)
                })
            });
            if let Some(arc) = kept {
                return Ok(Some(Child::Kept(arc)));
            }
        }
        // The node's own fields go through the JSON path, which knows every
        // style key and handler shape; only the walk down `c` stays on values.
        let mut own = serde_json::Map::new();
        let mut kids: Vec<Value> = Vec::new();
        {
            let borrow = hash.borrow();
            for (k, val) in borrow.iter() {
                let HashKey::String(name) = k else { continue };
                if name.as_str() == "c" {
                    if let Value::Array(items) = val {
                        kids = items.borrow().clone();
                    }
                } else {
                    own.insert(name.to_string(), crate::interpreter::value::value_to_json(val)?);
                }
            }
        }
        let mut node = self.convert_shallow(&own)?;
        for child in &kids {
            if let Some(c) = self.convert_value(child, pins)? {
                node.children.push(c);
            }
        }
        if node.kind.is_leaf() && !node.children.is_empty() {
            return Err(format!("EUI: a {:?} node cannot have children", node.kind));
        }
        if keyed && !self.session.is_empty() {
            node.identity = identity;
            pins.insert(identity, v.clone());
        }
        Ok(Some(Child::Fresh(node)))
    }

    fn convert(&mut self, j: &Json) -> Result<TNode, String> {
        let obj = j.as_object().ok_or("EUI: a node must be a hash")?;
        let mut node = self.convert_shallow(obj)?;
        if let Some(c) = obj.get("c").and_then(Json::as_array) {
            for child in c {
                if child.is_null() || child == &Json::Bool(false) {
                    continue; // `x if cond` in a list reads as null: skip, like a template would.
                }
                node.children.push(Child::Fresh(self.convert(child)?));
            }
        }
        if node.kind.is_leaf() && !node.children.is_empty() {
            return Err(format!("EUI: a {:?} node cannot have children", node.kind));
        }
        Ok(node)
    }

    /// A node from its own fields — everything but `c`.
    fn convert_shallow(&mut self, obj: &serde_json::Map<String, Json>) -> Result<TNode, String> {
        let kind = match obj.get("k").and_then(Json::as_str).unwrap_or("box") {
            "box" => NodeKind::Box,
            "text" => NodeKind::Text,
            "image" => NodeKind::Image,
            "icon" => NodeKind::Icon,
            "input" => NodeKind::Input,
            "textarea" => NodeKind::TextArea,
            "scroll" => NodeKind::Scroll,
            "list" => NodeKind::List,
            "canvas" => NodeKind::Canvas,
            "spacer" => NodeKind::Spacer,
            "divider" => NodeKind::Divider,
            "overlay" => NodeKind::Overlay,
            "slot" => NodeKind::Slot,
            "sizer" => NodeKind::Sizer,
            other => return Err(format!("EUI: unknown node kind '{other}'")),
        };
        let style = match obj.get("s") {
            Some(s) => {
                let record = self.style_record(s)?;
                self.style(record)
            }
            None => 0,
        };
        let key = obj.get("key").map(|k| match k {
            Json::String(s) => s.clone(),
            other => other.to_string(),
        });
        let key_atom = match &key {
            Some(k) => self.atom(k),
            None => 0,
        };
        let text = obj.get("t").map(|t| {
            let s = match t {
                Json::String(s) => s.clone(),
                other => other.to_string(),
            };
            // Short strings are worth interning only if they repeat; the
            // atom table is append-only, so a unique cell value would be a
            // permanent entry. Inline anything a row is likely to own.
            if s.len() <= 24 && obj.get("intern").and_then(Json::as_bool).unwrap_or(false) {
                TextRef::Atom(self.atom(&s))
            } else {
                TextRef::Inline(s)
            }
        });
        let mut props = Vec::new();
        if let Some(p) = obj.get("p").and_then(Json::as_object) {
            for (name, v) in p {
                let atom = self.atom(name);
                // An image's `src` is a file in the application; it goes on
                // the wire as the hash of its bytes, served from /_eui/asset.
                let value = if kind == NodeKind::Image && name == "src" {
                    match v {
                        Json::String(path) => WireValue::Asset(super::assets::from_file(path)?),
                        other => return Err(format!("EUI: image src must be a path, got {other}")),
                    }
                } else if kind == NodeKind::Canvas && name == "paths" {
                    self.paths_value(v)?
                } else {
                    self.wire_value(v)?
                };
                props.push((atom, value));
            }
        }
        let mut handlers = Vec::new();
        if let Some(on) = obj.get("on").and_then(Json::as_object) {
            for (event, target) in on {
                let kind = event_kind(event).ok_or_else(|| format!("EUI: unknown event '{event}'"))?;
                let handler = match target {
                    Json::String(name) => Handler::Server(self.atom(name)),
                    Json::Object(spec) => {
                        // {"local": "source" | [instructions], "styles": {name: style}?, "then": "server_event"?}
                        let mut declared: HashMap<String, u32> = HashMap::new();
                        if let Some(styles) = spec.get("styles").and_then(Json::as_object) {
                            for (name, st) in styles {
                                let record = self.style_record(st)?;
                                let id = self.style(record);
                                declared.insert(name.clone(), id);
                            }
                        }
                        let bytes = match spec.get("local") {
                            Some(Json::String(src)) => {
                                let mut ctx = LocalCtx { enc: self, declared: &declared, self_key: key.clone() };
                                super::local::compile(src, &mut ctx)?
                            }
                            Some(Json::Array(program)) => self.assemble(program)?,
                            _ => return Err("EUI: a local handler needs \"local\": a source string or an instruction list".into()),
                        };
                        let chunk = self.chunk(bytes);
                        match spec.get("then").and_then(Json::as_str) {
                            Some(name) => Handler::LocalThenServer { chunk, name: self.atom(name) },
                            None => Handler::Local(chunk),
                        }
                    }
                    _ => return Err("EUI: a handler is a server event name or {\"local\": [...]}".into()),
                };
                handlers.push((kind, handler));
            }
        }
        Ok(TNode { id: 0, kind, style, key, key_atom, text, props, handlers, children: Vec::new(), identity: 0, size: 0 })
    }

    /// Spec 03 §1.1: a canvas path is `[kind, colour, numbers…]`. The colour
    /// is written as a role name or `#RRGGBB` and goes on the wire resolved,
    /// so the client never parses a string while painting.
    fn paths_value(&mut self, v: &Json) -> Result<WireValue, String> {
        let paths = v.as_array().ok_or("EUI: paths must be a list of paths")?;
        let mut out = Vec::with_capacity(paths.len());
        for path in paths {
            let parts = path.as_array().ok_or("EUI: a path is a list: kind, colour, numbers")?;
            let mut items = Vec::with_capacity(parts.len());
            for (i, part) in parts.iter().enumerate() {
                items.push(if i == 1 { WireValue::Color(self.color(part)?) } else { self.wire_value(part)? });
            }
            out.push(WireValue::List(items));
        }
        Ok(WireValue::List(out))
    }

    fn wire_value(&mut self, v: &Json) -> Result<WireValue, String> {
        Ok(match v {
            Json::Null => WireValue::Null,
            Json::Bool(b) => WireValue::Bool(*b),
            Json::Number(n) => match (n.as_i64(), n.as_f64()) {
                (Some(i), _) => WireValue::Int(i),
                (None, Some(f)) if f.is_finite() => WireValue::Float(f),
                _ => return Err("EUI: non-finite number".into()),
            },
            Json::String(s) => WireValue::Str(s.clone()),
            Json::Array(items) => {
                let mut out = Vec::with_capacity(items.len());
                for i in items {
                    out.push(self.wire_value(i)?);
                }
                WireValue::List(out)
            }
            Json::Object(_) => return Err("EUI: a prop cannot be a hash".into()),
        })
    }

    fn color(&mut self, v: &Json) -> Result<ColorRef, String> {
        let s = v.as_str().ok_or("EUI: a colour is a role name or #RRGGBB")?;
        if let Some(hex) = s.strip_prefix('#') {
            let rgba = match hex.len() {
                6 => u32::from_str_radix(hex, 16).map_err(|_| "EUI: bad hex colour")? << 8 | 0xFF,
                8 => u32::from_str_radix(hex, 16).map_err(|_| "EUI: bad hex colour")?,
                _ => return Err("EUI: a hex colour is #RRGGBB or #RRGGBBAA".into()),
            };
            return Ok(self.color_literal(rgba));
        }
        if s == "none" {
            return Ok(ColorRef::NONE);
        }
        role_id(s).map(ColorRef::role).ok_or_else(|| format!("EUI: unknown colour role '{s}'"))
    }

    fn style_record(&mut self, s: &Json) -> Result<StyleRecord, String> {
        let obj = s.as_object().ok_or("EUI: style must be a hash")?;
        let mut r = StyleRecord::default();
        for (k, v) in obj {
            match k.as_str() {
                "display" => r.display = enum_of(v, &[("row", Display::Row), ("column", Display::Column), ("stack", Display::Stack), ("grid", Display::Grid), ("none", Display::None)])?,
                "wrap" => r.wrap = enum_of(v, &[("nowrap", Wrap::NoWrap), ("wrap", Wrap::Wrap), ("wrap_reverse", Wrap::WrapReverse)])?,
                "justify" => r.justify = enum_of(v, &[("start", Justify::Start), ("center", Justify::Center), ("end", Justify::End), ("between", Justify::Between), ("around", Justify::Around), ("evenly", Justify::Evenly)])?,
                "align" => r.align_items = enum_of(v, &[("start", AlignItems::Start), ("center", AlignItems::Center), ("end", AlignItems::End), ("stretch", AlignItems::Stretch), ("baseline", AlignItems::Baseline)])?,
                "self" => r.align_self = enum_of(v, &[("start", AlignSelf::Start), ("center", AlignSelf::Center), ("end", AlignSelf::End), ("stretch", AlignSelf::Stretch), ("baseline", AlignSelf::Baseline), ("auto", AlignSelf::Auto)])?,
                "grow" => r.grow = u8_of(v)?,
                "shrink" => r.shrink = u8_of(v)?,
                "gap" => r.gap = u8_of(v)?,
                "basis" => r.basis = dim_of(v)?,
                "width" => r.width = dim_of(v)?,
                "height" => r.height = dim_of(v)?,
                "min_width" => r.min_width = dim_of(v)?,
                "min_height" => r.min_height = dim_of(v)?,
                "max_width" => r.max_width = dim_of(v)?,
                "max_height" => r.max_height = dim_of(v)?,
                "pad" => r.padding = edges_of(v)?,
                "margin" => r.margin = edges_of(v)?,
                "bg" => r.bg = self.color(v)?,
                "fg" => r.fg = self.color(v)?,
                "border_color" => r.border_color = self.color(v)?,
                "border" => r.border_width = edges_of(v)?,
                "radius" => r.radius = u8_of(v)?,
                "shadow" => r.shadow = u8_of(v)?,
                "opacity" => r.opacity = u8_of(v)?,
                "font" => r.font_family = enum_of(v, &[("sans", FontFamily::Sans), ("mono", FontFamily::Mono)])?,
                "size" => r.font_size = u8_of(v)?,
                "weight" => r.font_weight = enum_of(v, &[("regular", FontWeight::Regular), ("medium", FontWeight::Medium), ("semibold", FontWeight::Semibold), ("bold", FontWeight::Bold)])?,
                "text_align" => r.text_align = enum_of(v, &[("start", TextAlign::Start), ("center", TextAlign::Center), ("end", TextAlign::End), ("justify", TextAlign::Justify)])?,
                "clamp" => r.line_clamp = u8_of(v)?,
                "underline" => r.text_decoration |= u8::from(v.as_bool().unwrap_or(false)),
                "strike" => r.text_decoration |= u8::from(v.as_bool().unwrap_or(false)) << 1,
                "overflow" => r.overflow = enum_of(v, &[("visible", Overflow::Visible), ("clip", Overflow::Clip), ("scroll", Overflow::Scroll)])?,
                "transition" => r.transition = enum_of(v, &[("none", 0), ("fast", 1), ("base", 2), ("slow", 3)])?,
                "animation" => r.animation = enum_of(v, &[("none", 0), ("spin", 1)])?,
                "position" => r.position = enum_of(v, &[("flow", Position::Flow), ("absolute", Position::Absolute)])?,
                "z" => r.z = u8_of(v)?,
                "cursor" => r.cursor = enum_of(v, &[("default", Cursor::Default), ("pointer", Cursor::Pointer), ("text", Cursor::Text), ("grab", Cursor::Grab), ("grabbing", Cursor::Grabbing), ("resize_h", Cursor::ResizeH), ("resize_v", Cursor::ResizeV), ("wait", Cursor::Wait), ("not_allowed", Cursor::NotAllowed)])?,
                other => return Err(format!("EUI: unknown style key '{other}'")),
            }
        }
        Ok(r)
    }
}

fn enum_of<T: Copy>(v: &Json, table: &[(&str, T)]) -> Result<T, String> {
    let s = v.as_str().ok_or("EUI: expected a name")?;
    table.iter().find(|(n, _)| *n == s).map(|(_, t)| *t).ok_or_else(|| format!("EUI: unknown value '{s}'"))
}

fn u8_of(v: &Json) -> Result<u8, String> {
    v.as_u64().and_then(|n| u8::try_from(n).ok()).ok_or_else(|| format!("EUI: expected 0–255, got {v}"))
}

/// `"auto"`, `12` (px), `"50%"`, `"sp:4"` (space index), `"1fr"`.
fn dim_of(v: &Json) -> Result<Dim, String> {
    match v {
        Json::Number(n) => n.as_u64().and_then(|n| u16::try_from(n).ok()).map(Dim::Px).ok_or_else(|| "EUI: px must be 0–65535".to_string()),
        Json::String(s) if s == "auto" => Ok(Dim::Auto),
        Json::String(s) if s.ends_with('%') => s.trim_end_matches('%').parse::<f64>().ok().map(|p| Dim::Percent((p * 100.0).round() as u16)).ok_or_else(|| "EUI: bad percent".into()),
        Json::String(s) if s.ends_with("fr") => s.trim_end_matches("fr").parse::<f64>().ok().map(|f| Dim::Fr((f * 100.0).round() as u16)).ok_or_else(|| "EUI: bad fr".into()),
        Json::String(s) if s.starts_with("sp:") => s[3..].parse::<u8>().map(Dim::Space).map_err(|_| "EUI: bad space index".into()),
        other => Err(format!("EUI: cannot read a length from {other}")),
    }
}

/// One index for all sides, or `[t, r, b, l]`.
fn edges_of(v: &Json) -> Result<[u8; 4], String> {
    match v {
        Json::Number(_) => {
            let n = u8_of(v)?;
            Ok([n; 4])
        }
        Json::Array(a) if a.len() == 4 => Ok([u8_of(&a[0])?, u8_of(&a[1])?, u8_of(&a[2])?, u8_of(&a[3])?]),
        Json::Array(a) if a.len() == 2 => {
            let (y, x) = (u8_of(&a[0])?, u8_of(&a[1])?);
            Ok([y, x, y, x])
        }
        other => Err(format!("EUI: edges are one index or [t, r, b, l], got {other}")),
    }
}

fn event_kind(name: &str) -> Option<EventKind> {
    Some(match name {
        "click" => EventKind::Click,
        "double_click" => EventKind::DoubleClick,
        "pointer_down" => EventKind::PointerDown,
        "pointer_up" => EventKind::PointerUp,
        "pointer_move" => EventKind::PointerMove,
        "pointer_enter" => EventKind::PointerEnter,
        "pointer_leave" => EventKind::PointerLeave,
        "key_down" => EventKind::KeyDown,
        "key_up" => EventKind::KeyUp,
        "text_input" => EventKind::TextInput,
        "focus" => EventKind::Focus,
        "blur" => EventKind::Blur,
        "change" => EventKind::Change,
        "submit" => EventKind::Submit,
        "scroll" => EventKind::Scroll,
        "resize" => EventKind::Resize,
        "context_menu" => EventKind::ContextMenu,
        "drag_start" => EventKind::DragStart,
        "drag_over" => EventKind::DragOver,
        "drop" => EventKind::Drop,
        "long_press" => EventKind::LongPress,
        _ => return None,
    })
}

/// Wire event kind back to its view name, for the handler's `event` field.
pub fn event_name(kind: EventKind) -> &'static str {
    match kind {
        EventKind::Click => "click",
        EventKind::DoubleClick => "double_click",
        EventKind::PointerDown => "pointer_down",
        EventKind::PointerUp => "pointer_up",
        EventKind::PointerMove => "pointer_move",
        EventKind::PointerEnter => "pointer_enter",
        EventKind::PointerLeave => "pointer_leave",
        EventKind::KeyDown => "key_down",
        EventKind::KeyUp => "key_up",
        EventKind::TextInput => "text_input",
        EventKind::Focus => "focus",
        EventKind::Blur => "blur",
        EventKind::Change => "change",
        EventKind::Submit => "submit",
        EventKind::Scroll => "scroll",
        EventKind::Resize => "resize",
        EventKind::ContextMenu => "context_menu",
        EventKind::DragStart => "drag_start",
        EventKind::DragOver => "drag_over",
        EventKind::Drop => "drop",
        EventKind::LongPress => "long_press",
    }
}

/// `spec/05-theme.md` §1, by name.
const ROLES: [&str; 28] = [
    "surface.base", "surface.raised", "surface.sunken", "surface.overlay",
    "text.default", "text.muted", "text.inverted", "text.disabled",
    "accent.base", "accent.hover", "accent.active", "accent.on",
    "success.base", "success.subtle", "success.on",
    "warning.base", "warning.subtle", "warning.on",
    "danger.base", "danger.subtle", "danger.on",
    "info.base", "info.subtle", "info.on",
    "border.subtle", "border.default", "border.strong", "focus.ring",
];

fn role_id(name: &str) -> Option<u16> {
    ROLES.iter().position(|r| *r == name).map(|i| i as u16 + 1)
}

/// Give every node in a subtree a fresh id. A kept child inside an inserted
/// subtree is copied first: it is entering the tree anew, under new ids.
pub fn assign_fresh_ids(enc: &mut Encoder, node: &mut TNode) {
    node.id = enc.fresh_id();
    for c in &mut node.children {
        assign_fresh_ids(enc, c.node_mut());
    }
}

/// Freeze this render's keyed, freshly converted nodes behind an `Rc` and
/// remember them by the identity of the value they came from.
fn freeze(node: &mut TNode, memo: &mut Memo, pins: &mut HashMap<usize, Value>, generation: u32) {
    for child in &mut node.children {
        if let Child::Fresh(n) = child {
            freeze(n, memo, pins, generation);
            if n.identity != 0 {
                let Some(pin) = pins.remove(&n.identity) else { continue };
                n.size = count_nodes(n) as u32;
                let arc = Arc::new(std::mem::replace(n, TNode { id: 0, kind: NodeKind::Box, style: 0, key: None, key_atom: 0, text: None, props: Vec::new(), handlers: Vec::new(), children: Vec::new(), identity: 0, size: 0 }));
                memo.entries.insert(arc.identity, (pin, Arc::clone(&arc), generation));
                *child = Child::Kept(arc);
            }
        }
    }
}

/// Pre-order flatten into the wire shape.
pub fn flatten(root: &TNode) -> Subtree {
    let mut out = Subtree::default();
    fn walk(n: &TNode, out: &mut Subtree) {
        let props_start = out.props.len() as u32;
        out.props.extend(n.props.iter().cloned());
        let handlers_start = out.handlers.len() as u32;
        out.handlers.extend(n.handlers.iter().cloned());
        out.nodes.push(FlatNode {
            kind: n.kind,
            id: n.id,
            style: n.style,
            key: n.key_atom,
            text: n.text.clone(),
            props: (props_start, n.props.len() as u32),
            handlers: (handlers_start, n.handlers.len() as u32),
            child_count: n.children.len() as u32,
        });
        for c in &n.children {
            walk(c.node(), out);
        }
    }
    walk(root, &mut out);
    out
}

/// The compiler's window onto the encoder for one handler.
struct LocalCtx<'a> {
    enc: &'a mut Encoder,
    declared: &'a HashMap<String, u32>,
    self_key: Option<String>,
}

impl super::local::Ctx for LocalCtx<'_> {
    fn atom(&mut self, s: &str) -> u32 {
        self.enc.atom(s)
    }
    fn style(&mut self, name: &str) -> Option<u32> {
        self.declared.get(name).copied()
    }
    fn self_key(&self) -> Option<String> {
        self.self_key.clone()
    }
}

/// Wire value to JSON, atoms resolved through the encoder.
pub fn wire_to_json(enc: &Encoder, v: &WireValue) -> Json {
    match v {
        WireValue::Null => Json::Null,
        WireValue::Bool(b) => Json::Bool(*b),
        WireValue::Int(n) => Json::from(*n),
        WireValue::Float(f) => serde_json::json!(f),
        WireValue::Atom(a) => enc.atom_name(*a).map(|s| Json::String(s.to_string())).unwrap_or(Json::Null),
        WireValue::Str(s) => Json::String(s.clone()),
        WireValue::Asset(h) => Json::String(h.iter().map(|b| format!("{b:02x}")).collect()),
        WireValue::Color(c) => Json::from(c.0),
        WireValue::List(items) => Json::Array(items.iter().map(|i| wire_to_json(enc, i)).collect()),
    }
}

fn find(node: &TNode, id: u32) -> Option<&TNode> {
    if node.id == id {
        return Some(node);
    }
    node.children.iter().find_map(|c| find(c.node(), id))
}
