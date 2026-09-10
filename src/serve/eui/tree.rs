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
    AlignItems, AlignSelf, Batch, ColorRef, Cursor, Dim, Display, EventKind, FlatNode, FontFamily,
    FontWeight, Handler, Justify, NodeKind, Op, Overflow, Position, StyleRecord, Subtree,
    TextAlign, TextRef, Value as WireValue, Wrap, Writer,
};
use serde_json::Value as Json;

/// One instruction on its way through the two-pass assembler: the bytes it
/// encodes to, and the jump it still owes — the opcode and the label it targets
/// — for the branches whose offset is only known once pass 1 has sized
/// everything ahead of them.
type SizedInstr = (Vec<u8>, Option<(u8, String)>);

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
#[derive(Debug, Default, Clone)]
pub struct Encoder {
    atoms: HashMap<String, u32>,
    /// The atoms again, by id — ids are dense and 1-based, so `atoms_by_id[id]`
    /// is the string, and the reverse lookup every event needs is a index
    /// rather than a walk of the whole table.
    atoms_by_id: Vec<String>,
    styles: HashMap<[u8; 64], u32>,
    /// Style ids by a fingerprint of the view value that produced them, so
    /// a style hash the view writes the same way every render is looked up
    /// by hashing its fields, not by converting them to JSON, parsing that
    /// into a record and encoding the record — which was most of what a
    /// cold render spent per node.
    style_fingerprints: HashMap<[u8; 16], u32>,
    colors: HashMap<u32, u32>,
    chunks: HashMap<Vec<u8>, u32>,
    /// Chunk ids by local-handler source (and what the compile depended
    /// on), so a `local("…")` seen once is a lookup, not a lex, a parse and
    /// an assemble per node per render.
    local_cache: HashMap<String, u32>,
    /// The first table to pass a limit the client enforces; the render that
    /// did it fails, since every later one would too.
    overflow: Option<String>,
    pending: Vec<Op>,
    next_node: u32,
    seq: u64,
    prev: Option<TNode>,
    /// Which session this is, for the thread-local memo.
    session: String,
    generation: u32,
    /// How many nodes the last tree had, for `eui_stats()`.
    last_nodes: usize,
}

impl Encoder {
    /// What this session has interned, and how big its last tree was:
    /// `[atoms, styles, colours, chunks, nodes]`. A dev bar's second line.
    pub fn tables(&self) -> [usize; 5] {
        [
            self.atoms.len(),
            self.styles.len(),
            self.colors.len(),
            self.chunks.len(),
            self.last_nodes,
        ]
    }
}

/// One kept subtree: the frozen nodes, the view value they came from — pinned,
/// so its address cannot be reused by another object while the entry lives —
/// the render that last saw it, and the keyed subtrees directly inside it.
struct MemoEntry {
    /// Held, never read: it keeps the view object alive so its address —
    /// the entry's key — cannot be handed to another object.
    _pin: Value,
    tree: Arc<TNode>,
    seen: u32,
    /// Identities of the keyed nodes nested in this one. A kept subtree is
    /// never walked, so its inner entries are not seen by the render that
    /// keeps it; they stay alive through this list instead, and are warm
    /// when the outer node finally changes.
    children: Vec<usize>,
}

/// One session's kept subtrees, by the address of the view value each came
/// from. Values are `Rc`, so this lives on the thread that evaluates the
/// view — which is the same thread every render, the session being pinned
/// to one realtime worker (`serve::lv_sender_for`). A render on another
/// thread simply misses, which is safe.
#[derive(Default)]
struct Memo {
    entries: HashMap<usize, MemoEntry>,
}

thread_local! {
    static MEMOS: RefCell<HashMap<String, Memo>> = RefCell::new(HashMap::new());
    /// Renders since this thread last swept its memos for dead sessions.
    static SWEEP: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

/// Renders between two sweeps of the memos for sessions that have gone.
const SWEEP_EVERY: u32 = 256;

fn with_memo<R>(session: &str, f: impl FnOnce(&mut Memo) -> R) -> R {
    // The socket asks the worker to forget a session on close; this is the
    // backstop for the ask that did not arrive (a saturated queue, a worker
    // that restarted). The encoder is dropped on the socket's side, so an
    // entry with no encoder belongs to a session that is over.
    let due = SWEEP.with(|n| {
        let next = n.get().wrapping_add(1);
        n.set(next);
        next % SWEEP_EVERY == 0
    });
    MEMOS.with(|m| {
        let mut memos = m.borrow_mut();
        if due {
            memos.retain(|id, _| id == session || super::has_encoder(id));
        }
        if !memos.contains_key(session) {
            memos.insert(session.to_owned(), Memo::default());
        }
        f(memos.get_mut(session).expect("just inserted"))
    })
}

/// How many subtrees this thread keeps for a session.
#[cfg(test)]
fn memo_entries(session: &str) -> usize {
    MEMOS.with(|m| m.borrow().get(session).map_or(0, |memo| memo.entries.len()))
}

/// Drop a session's memo when the session goes. Runs on the session's
/// worker: the socket posts `FORGET_EVENT` there.
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

/// The weight of one batch, roughly its bytes on the wire: a node counts
/// [`NODE_WEIGHT`] plus its text. The client refuses a frame over 8 MiB,
/// and a first render or a resync used to be one op however big the tree;
/// a tree of a few hundred thousand cards could never be mounted at all.
pub const MAX_BATCH_WEIGHT: usize = 2 * 1024 * 1024;

/// What a node costs on the wire before its text: kind, ids, style, key,
/// the prop and handler slices, the child count, and the props themselves,
/// generously.
const NODE_WEIGHT: usize = 48;

fn subtree_weight(t: &Subtree) -> usize {
    t.nodes
        .iter()
        .map(|n| {
            NODE_WEIGHT
                + match &n.text {
                    Some(TextRef::Inline(s)) => s.len(),
                    _ => 0,
                }
        })
        .sum::<usize>()
        + t.props.len() * 16
}

fn op_weight(op: &Op) -> usize {
    match op {
        Op::Mount(t) | Op::Replace { subtree: t, .. } | Op::InsertChild { subtree: t, .. } => {
            subtree_weight(t)
        }
        Op::DefAtom { value, .. } => 8 + value.len(),
        Op::DefChunkBytes { bytes, .. } => 8 + bytes.len(),
        Op::SetText {
            text: TextRef::Inline(s),
            ..
        } => 8 + s.len(),
        _ => 16,
    }
}

/// The weight of a tree that has not been flattened: the same measure as
/// [`subtree_weight`], off the nodes.
fn tree_weight(n: &TNode) -> usize {
    NODE_WEIGHT
        + match &n.text {
            Some(TextRef::Inline(s)) => s.len(),
            _ => 0,
        }
        + n.props.len() * 16
        + n.children
            .iter()
            .map(|c| tree_weight(c.node()))
            .sum::<usize>()
}

/// A whole tree as ops: one `Mount` when it fits a batch, else the root
/// mounted bare and its children inserted one by one — and a child that is
/// itself too big goes the same way, so no single op outweighs a batch and
/// a tree of any size the client accepts can be sent.
fn mount_ops(tree: &TNode) -> Vec<Op> {
    let mut ops = Vec::new();
    if tree_weight(tree) <= MAX_BATCH_WEIGHT {
        ops.push(Op::Mount(flatten(tree)));
        return ops;
    }
    let mut bare = tree.clone();
    let children = std::mem::take(&mut bare.children);
    ops.push(Op::Mount(flatten(&bare)));
    for (index, child) in children.iter().enumerate() {
        insert_ops(tree.id, index as u32, child.node(), &mut ops);
    }
    ops
}

fn insert_ops(parent: u32, index: u32, node: &TNode, ops: &mut Vec<Op>) {
    if tree_weight(node) <= MAX_BATCH_WEIGHT {
        ops.push(Op::InsertChild {
            parent,
            index,
            subtree: flatten(node),
        });
        return;
    }
    let mut bare = node.clone();
    let children = std::mem::take(&mut bare.children);
    ops.push(Op::InsertChild {
        parent,
        index,
        subtree: flatten(&bare),
    });
    for (i, child) in children.iter().enumerate() {
        insert_ops(node.id, i as u32, child.node(), ops);
    }
}

impl Encoder {
    /// Intern a string, emitting its definition on first use.
    pub fn atom(&mut self, s: &str) -> u32 {
        if let Some(id) = self.atoms.get(s) {
            return *id;
        }
        let id = self.atoms.len() as u32 + 1;
        if id > eui_proto::limits::MAX_ATOMS {
            self.overflowed("atoms", eui_proto::limits::MAX_ATOMS);
        }
        self.atoms.insert(s.to_string(), id);
        if self.atoms_by_id.is_empty() {
            self.atoms_by_id.push(String::new()); // id 0 is "no atom"
        }
        self.atoms_by_id.push(s.to_string());
        self.pending.push(Op::DefAtom {
            id,
            value: s.to_string(),
        });
        id
    }

    fn atom_name(&self, id: u32) -> Option<&str> {
        if id == 0 {
            return None;
        }
        self.atoms_by_id.get(id as usize).map(String::as_str)
    }

    /// Note the first table to outgrow the client's limit. The tables are
    /// append-only — the wire format says so — so a session that has used
    /// its last atom on a row key has no way back, and the honest thing is
    /// to fail this render with the reason rather than send ids the client
    /// will refuse without one.
    fn overflowed(&mut self, table: &str, max: u32) {
        if self.overflow.is_none() {
            self.overflow = Some(format!(
                "EUI: this session has interned more than {max} {table}; a key or a style \
                 derived from data grows the table with every new value — intern less, or \
                 reconnect"
            ));
        }
    }

    fn color_literal(&mut self, rgba: u32) -> ColorRef {
        if let Some(id) = self.colors.get(&rgba) {
            return ColorRef::literal(*id as u16);
        }
        let id = self.colors.len() as u32 + 1;
        if id > eui_proto::limits::MAX_COLORS {
            self.overflowed("colours", eui_proto::limits::MAX_COLORS);
        }
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
        if id > eui_proto::limits::MAX_CHUNKS {
            self.overflowed("chunks", eui_proto::limits::MAX_CHUNKS);
        }
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
        let mut sized: Vec<SizedInstr> = Vec::new();
        let mut offset = 0usize;
        for instr in program {
            let parts = instr
                .as_array()
                .ok_or_else(|| bad("each instruction is a list"))?;
            let name = parts
                .first()
                .and_then(Json::as_str)
                .ok_or_else(|| bad("instruction name"))?;
            let arg = parts.get(1);
            let atom_arg = |enc: &mut Self| -> Result<u32, String> {
                let s = arg
                    .and_then(Json::as_str)
                    .ok_or_else(|| bad("expected a name"))?;
                Ok(enc.atom(s))
            };
            let node_arg = |enc: &mut Self| -> Result<u32, String> {
                let key = arg.ok_or_else(|| bad("expected a node key"))?;
                let key = match key {
                    Json::String(s) => s.clone(),
                    other => other.to_string(),
                };
                Ok(enc.atom(&key))
            };
            let mut w = Writer::new();
            let mut jump = None;
            match name {
                "push" => match arg {
                    Some(Json::Number(n)) => {
                        w.u8(0x01)
                            .svarint(n.as_i64().ok_or_else(|| bad("integer"))?);
                    }
                    Some(Json::Bool(b)) => {
                        w.u8(0x03).u8(u8::from(*b));
                    }
                    Some(Json::String(s)) => {
                        let a = self.atom(s);
                        w.u8(0x02).varint32(a);
                    }
                    _ => return Err(bad("push takes a number, bool or string")),
                },
                "load" => {
                    let a = atom_arg(self)?;
                    w.u8(0x04).varint32(a);
                }
                "store" => {
                    let a = atom_arg(self)?;
                    w.u8(0x05).varint32(a);
                }
                "dup" => {
                    w.u8(0x06);
                }
                "pop" => {
                    w.u8(0x07);
                }
                "add" => {
                    w.u8(0x10);
                }
                "sub" => {
                    w.u8(0x11);
                }
                "mul" => {
                    w.u8(0x12);
                }
                "neg" => {
                    w.u8(0x13);
                }
                "not" => {
                    w.u8(0x14);
                }
                "eq" => {
                    w.u8(0x15);
                }
                "lt" => {
                    w.u8(0x16);
                }
                "gt" => {
                    w.u8(0x17);
                }
                "and" => {
                    w.u8(0x18);
                }
                "or" => {
                    w.u8(0x19);
                }
                "to_str" => {
                    w.u8(0x1A);
                }
                "concat" => {
                    w.u8(0x1B);
                }
                "label" => {
                    let l = arg
                        .and_then(Json::as_str)
                        .ok_or_else(|| bad("label name"))?;
                    labels.insert(l.to_string(), offset);
                    continue;
                }
                "jump" | "jump_if_false" => {
                    let l = arg
                        .and_then(Json::as_str)
                        .ok_or_else(|| bad("jump label"))?;
                    w.u8(if name == "jump" { 0x20 } else { 0x21 }).u16(0);
                    jump = Some((if name == "jump" { 0x20 } else { 0x21 }, l.to_string()));
                }
                "set_text" => {
                    let n = node_arg(self)?;
                    w.u8(0x30).varint32(n);
                }
                "set_prop" => {
                    let n = node_arg(self)?;
                    let p = parts
                        .get(2)
                        .and_then(Json::as_str)
                        .ok_or_else(|| bad("set_prop needs a prop name"))?;
                    let a = self.atom(p);
                    w.u8(0x31).varint32(n).varint32(a);
                }
                "emit" => {
                    let a = atom_arg(self)?;
                    w.u8(0x32).varint32(a);
                }
                "set_style" => {
                    let n = node_arg(self)?;
                    let st = parts
                        .get(2)
                        .ok_or_else(|| bad("set_style needs a style hash"))?;
                    let record = self.style_record(st)?;
                    let id = self.style(record);
                    w.u8(0x33).varint32(n).varint32(id);
                }
                "return" => {
                    w.u8(0x40);
                }
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
                let target = *labels
                    .get(&label)
                    .ok_or_else(|| bad(&format!("unknown label '{label}'")))?
                    as i64;
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
        if id > eui_proto::limits::MAX_STYLES {
            self.overflowed("styles", eui_proto::limits::MAX_STYLES);
        }
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
        let tree = self.convert(json, 1)?;
        self.finish(tree, resync, HashMap::new())
    }

    /// [`Encoder::render`] straight from the view's value: no JSON in
    /// between, and a keyed child that is the same object as last render
    /// is kept — not converted, not diffed.
    pub fn render_value(
        &mut self,
        session: &str,
        value: &Value,
        resync: bool,
    ) -> Result<Vec<Batch>, String> {
        self.session = session.to_owned();
        self.generation = self.generation.wrapping_add(1);
        // The view values behind this render's keyed nodes, by identity,
        // until `finish` pins them in the memo. Held here, not on the
        // encoder: an interpreter value cannot cross threads.
        let mut pins: HashMap<usize, Value> = HashMap::new();
        let tree = match self.convert_value(value, &mut pins, 1)? {
            Some(Child::Fresh(n)) => n,
            Some(Child::Kept(rc)) => (*rc).clone(),
            None => return Err("EUI: the view returned nothing".into()),
        };
        self.finish(tree, resync, pins)
    }

    fn finish(
        &mut self,
        mut tree: TNode,
        resync: bool,
        mut pins: HashMap<usize, Value>,
    ) -> Result<Vec<Batch>, String> {
        // What the client would refuse is refused here, where the reason
        // reaches the view's author: a table past its limit, or a tree past
        // the node count — which there is no way to send anyway.
        if let Some(reason) = self.overflow.take() {
            return Err(reason);
        }
        let nodes = count_nodes(&tree);
        self.last_nodes = nodes;
        if nodes > eui_proto::limits::MAX_NODES as usize {
            return Err(format!(
                "EUI: the view returned {nodes} nodes; a client accepts at most {} — virtualise, paginate or trim",
                eui_proto::limits::MAX_NODES
            ));
        }
        let ops = match (self.prev.take(), resync) {
            (Some(prev), false) => {
                let mut ops = Vec::new();
                super::diff::diff(self, &prev, &mut tree, &mut ops);
                ops
            }
            _ => {
                assign_fresh_ids(self, &mut tree);
                mount_ops(&tree)
            }
        };
        // Keyed nodes converted this render are frozen behind an Arc, so the
        // next render can keep them by the identity of their view value.
        let generation = self.generation;
        if !self.session.is_empty() {
            let session = self.session.clone();
            with_memo(&session, |memo| {
                freeze(&mut tree, memo, &mut pins, generation);
                // Keep what this render saw, and everything nested in it:
                // a kept subtree's inner keyed nodes were not walked, so they
                // were not seen, but they are still on the screen.
                let mut keep: std::collections::HashSet<usize> = memo
                    .entries
                    .iter()
                    .filter(|(_, e)| e.seen == generation)
                    .map(|(id, _)| *id)
                    .collect();
                let mut stack: Vec<usize> = keep.iter().copied().collect();
                while let Some(id) = stack.pop() {
                    if let Some(entry) = memo.entries.get(&id) {
                        for child in &entry.children {
                            if keep.insert(*child) {
                                stack.push(*child);
                            }
                        }
                    }
                }
                memo.entries.retain(|id, _| keep.contains(id));
            });
        }
        self.prev = Some(tree);
        let mut all = std::mem::take(&mut self.pending);
        all.extend(ops);
        // A big update streams: every prefix of the op list is valid on its
        // own (definitions come first, ops apply in order), so a batch of
        // thousands of inserts goes out in slices the client paints between.
        // The first slice is on screen long before the last is encoded. A
        // slice is bounded by ops and by weight — an insert of a whole
        // subtree is one op and most of a frame.
        let mut batches = Vec::new();
        let mut ops = Vec::new();
        let mut weight = 0usize;
        for op in all {
            let w = op_weight(&op);
            if !ops.is_empty() && (ops.len() >= MAX_OPS_PER_BATCH || weight + w > MAX_BATCH_WEIGHT)
            {
                self.seq += 1;
                batches.push(Batch {
                    seq: self.seq,
                    ops: std::mem::take(&mut ops),
                });
                weight = 0;
            }
            weight += w;
            ops.push(op);
        }
        if !ops.is_empty() || batches.is_empty() {
            self.seq += 1;
            batches.push(Batch { seq: self.seq, ops });
        }
        Ok(batches)
    }

    /// The server-side name of the handler for `(node, event)` in the tree
    /// the client last received — `None` if the client is making it up.
    pub fn handler_name(&self, node: u32, event: EventKind) -> Option<String> {
        let found = find(self.prev.as_ref()?, node)?;
        self.handler_of(found, event)
    }

    /// A node's props with atom names resolved, as JSON, for the handler's
    /// `params["props"]`. This is how a row in a list says which row it is.
    pub fn props_of(&self, node: u32) -> serde_json::Map<String, Json> {
        match self.prev.as_ref().and_then(|tree| find(tree, node)) {
            Some(found) => self.props_json(found),
            None => serde_json::Map::new(),
        }
    }

    /// [`handler_name`] and [`props_of`] for one event, from one walk of
    /// the tree: what `validate` needs, at the cost of one lookup.
    pub fn event_target(
        &self,
        node: u32,
        event: EventKind,
    ) -> Option<(String, serde_json::Map<String, Json>)> {
        let found = find(self.prev.as_ref()?, node)?;
        let name = self.handler_of(found, event)?;
        Some((name, self.props_json(found)))
    }

    fn handler_of(&self, found: &TNode, event: EventKind) -> Option<String> {
        let atom = found
            .handlers
            .iter()
            .find(|(e, _)| *e == event)
            .and_then(|(_, h)| match h {
                Handler::Server(a) | Handler::LocalThenServer { name: a, .. } => Some(*a),
                Handler::Local(_) => None,
            })?;
        self.atom_name(atom).map(str::to_owned)
    }

    fn props_json(&self, found: &TNode) -> serde_json::Map<String, Json> {
        let mut out = serde_json::Map::new();
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
    fn convert_value(
        &mut self,
        v: &Value,
        pins: &mut HashMap<usize, Value>,
        depth: u32,
    ) -> Result<Option<Child>, String> {
        too_deep(depth)?;
        let hash = match v {
            Value::Hash(h) => h,
            Value::Null | Value::Bool(false) => return Ok(None),
            other => {
                return Err(format!(
                    "EUI: a node must be a hash, got {}",
                    other.type_name()
                ))
            }
        };
        let identity = Rc::as_ptr(hash) as *const u8 as usize;
        let keyed = hash.borrow().contains_key(&HashKey::String("key".into()));
        if keyed && !self.session.is_empty() {
            let generation = self.generation;
            let kept = with_memo(&self.session, |memo| {
                memo.entries.get_mut(&identity).map(|entry| {
                    entry.seen = generation;
                    Arc::clone(&entry.tree)
                })
            });
            if let Some(arc) = kept {
                return Ok(Some(Child::Kept(arc)));
            }
        }
        // The node's own fields are read off the value; `c` is walked.
        let (mut node, kids) = {
            let borrow = hash.borrow();
            let node = self.convert_shallow_value(&borrow)?;
            let kids = match borrow.get(&HashKey::String("c".into())) {
                Some(Value::Array(items)) => Some(Rc::clone(items)),
                _ => None,
            };
            (node, kids)
        };
        if let Some(kids) = kids {
            for child in kids.borrow().iter() {
                if let Some(c) = self.convert_value(child, pins, depth + 1)? {
                    node.children.push(c);
                }
            }
        }
        if node.kind.is_leaf() && !node.children.is_empty() {
            return Err(format!("EUI: a {:?} node cannot have children", node.kind));
        }
        refuse_repeated_keys(&node)?;
        if keyed && !self.session.is_empty() {
            node.identity = identity;
            pins.insert(identity, v.clone());
        }
        Ok(Some(Child::Fresh(node)))
    }

    fn convert(&mut self, j: &Json, depth: u32) -> Result<TNode, String> {
        too_deep(depth)?;
        let obj = j.as_object().ok_or("EUI: a node must be a hash")?;
        let mut node = self.convert_shallow(obj)?;
        if let Some(c) = obj.get("c").and_then(Json::as_array) {
            for child in c {
                if child.is_null() || child == &Json::Bool(false) {
                    continue; // `x if cond` in a list reads as null: skip, like a template would.
                }
                node.children
                    .push(Child::Fresh(self.convert(child, depth + 1)?));
            }
        }
        if node.kind.is_leaf() && !node.children.is_empty() {
            return Err(format!("EUI: a {:?} node cannot have children", node.kind));
        }
        refuse_repeated_keys(&node)?;
        Ok(node)
    }

    /// A node from its own fields — everything but `c`.
    fn convert_shallow(&mut self, obj: &serde_json::Map<String, Json>) -> Result<TNode, String> {
        let kind = kind_of(obj.get("k").and_then(Json::as_str).unwrap_or("box"))?;
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
        let intern = obj.get("intern").and_then(Json::as_bool).unwrap_or(false);
        let text = match obj.get("t") {
            Some(Json::String(s)) => Some(self.text_of(s.clone(), intern)?),
            Some(other) => Some(self.text_of(other.to_string(), intern)?),
            None => None,
        };
        let props = match obj.get("p").and_then(Json::as_object) {
            Some(p) => self.props_from(kind, p)?,
            None => Vec::new(),
        };
        let handlers = match obj.get("on").and_then(Json::as_object) {
            Some(on) => self.handlers_from(on, &key)?,
            None => Vec::new(),
        };
        Ok(self.node(kind, style, key, text, props, handlers))
    }

    /// [`convert_shallow`] straight off the view's hash: the scalar fields
    /// are read as they are, the style is looked up by fingerprint before
    /// anything is converted, and only `p` and `on` — small, and shaped by
    /// the JSON parsers — go through JSON.
    fn convert_shallow_value(
        &mut self,
        fields: &crate::interpreter::value::HashPairs,
    ) -> Result<TNode, String> {
        let field = |name: &str| fields.get(&HashKey::String(name.into()));
        let kind = match field("k") {
            None => NodeKind::Box,
            Some(Value::String(name)) => kind_of(name)?,
            Some(other) => return Err(format!("EUI: a node kind is a string, got {other}")),
        };
        let style = match field("s") {
            None => 0,
            Some(style) => self.style_of_value(style)?,
        };
        let key = field("key").map(|k| match k {
            Value::String(s) => s.to_string(),
            other => other.to_string(),
        });
        let intern = matches!(field("intern"), Some(Value::Bool(true)));
        let text = match field("t") {
            None => None,
            Some(Value::String(s)) => Some(self.text_of(s.to_string(), intern)?),
            Some(other) => Some(self.text_of(other.to_string(), intern)?),
        };
        let props = match field("p") {
            None => Vec::new(),
            Some(p) => {
                let json = crate::interpreter::value::value_to_json(p)?;
                match json.as_object() {
                    Some(p) => self.props_from(kind, p)?,
                    None => Vec::new(),
                }
            }
        };
        let handlers = match field("on") {
            None => Vec::new(),
            Some(on) => {
                let json = crate::interpreter::value::value_to_json(on)?;
                match json.as_object() {
                    Some(on) => self.handlers_from(on, &key)?,
                    None => Vec::new(),
                }
            }
        };
        Ok(self.node(kind, style, key, text, props, handlers))
    }

    fn node(
        &mut self,
        kind: NodeKind,
        style: u32,
        key: Option<String>,
        text: Option<TextRef>,
        props: Vec<(u32, WireValue)>,
        handlers: Vec<(EventKind, Handler)>,
    ) -> TNode {
        let key_atom = match &key {
            Some(k) => self.atom(k),
            None => 0,
        };
        TNode {
            id: 0,
            kind,
            style,
            key,
            key_atom,
            text,
            props,
            handlers,
            children: Vec::new(),
            identity: 0,
            size: 0,
        }
    }

    /// A style id for a view value: by fingerprint when the value is a hash
    /// seen before, else the JSON way, remembered under its fingerprint.
    fn style_of_value(&mut self, style: &Value) -> Result<u32, String> {
        let fingerprint = match style {
            Value::Hash(_) => fingerprint_of(style),
            _ => None,
        };
        if let Some(fp) = fingerprint {
            if let Some(id) = self.style_fingerprints.get(&fp) {
                return Ok(*id);
            }
        }
        let json = crate::interpreter::value::value_to_json(style)?;
        let record = self.style_record(&json)?;
        let id = self.style(record);
        if let Some(fp) = fingerprint {
            self.style_fingerprints.insert(fp, id);
        }
        Ok(id)
    }

    /// A text as it goes on the wire. Short strings are worth interning only
    /// if they repeat; the atom table is append-only, so a unique cell value
    /// would be a permanent entry — inline anything a row is likely to own,
    /// and intern only what the view asked to (`intern: true`).
    fn text_of(&mut self, s: String, intern: bool) -> Result<TextRef, String> {
        if s.len() > eui_proto::limits::MAX_INLINE_STR {
            return Err(format!(
                "EUI: a text of {} bytes; the client accepts at most {} — split it into nodes",
                s.len(),
                eui_proto::limits::MAX_INLINE_STR
            ));
        }
        Ok(if s.len() <= 24 && intern {
            TextRef::Atom(self.atom(&s))
        } else {
            TextRef::Inline(s)
        })
    }

    fn props_from(
        &mut self,
        kind: NodeKind,
        p: &serde_json::Map<String, Json>,
    ) -> Result<Vec<(u32, WireValue)>, String> {
        let mut props = Vec::with_capacity(p.len());
        for (name, v) in p {
            let atom = self.atom(name);
            // An image's `src` is a file in the application; it goes on
            // the wire as the hash of its bytes, served from /_eui/asset.
            let value = if matches!(kind, NodeKind::Image | NodeKind::Audio | NodeKind::Video)
                && name == "src"
            {
                match v {
                    Json::String(path) => WireValue::Asset(super::assets::from_file(path)?),
                    other => return Err(format!("EUI: a src must be a path, got {other}")),
                }
            } else if kind == NodeKind::Canvas && name == "paths" {
                self.paths_value(v)?
            } else {
                self.wire_value(v)?
            };
            props.push((atom, value));
        }
        Ok(props)
    }

    fn handlers_from(
        &mut self,
        on: &serde_json::Map<String, Json>,
        key: &Option<String>,
    ) -> Result<Vec<(EventKind, Handler)>, String> {
        let mut handlers = Vec::with_capacity(on.len());
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
                    let chunk = match spec.get("local") {
                        Some(Json::String(src)) => {
                            // What the compile depends on, all of it: the
                            // source, the node's own key (`self`), and the
                            // ids the declared styles resolved to.
                            let mut declared_ids: Vec<(&String, &u32)> = declared.iter().collect();
                            declared_ids.sort();
                            let cache_key = format!("{src}\u{0}{key:?}\u{0}{declared_ids:?}");
                            match self.local_cache.get(&cache_key) {
                                Some(id) => *id,
                                None => {
                                    let bytes = {
                                        let mut ctx = LocalCtx {
                                            enc: self,
                                            declared: &declared,
                                            self_key: key.clone(),
                                        };
                                        super::local::compile(src, &mut ctx)?
                                    };
                                    let id = self.chunk(bytes);
                                    self.local_cache.insert(cache_key, id);
                                    id
                                }
                            }
                        }
                        Some(Json::Array(program)) => {
                            let bytes = self.assemble(program)?;
                            self.chunk(bytes)
                        }
                        _ => return Err("EUI: a local handler needs \"local\": a source string or an instruction list".into()),
                    };
                    match spec.get("then").and_then(Json::as_str) {
                        Some(name) => Handler::LocalThenServer {
                            chunk,
                            name: self.atom(name),
                        },
                        None => Handler::Local(chunk),
                    }
                }
                _ => {
                    return Err(
                        "EUI: a handler is a server event name or {\"local\": [...]}".into(),
                    )
                }
            };
            handlers.push((kind, handler));
        }
        Ok(handlers)
    }

    /// Spec 03 §1.1: a canvas path is `[kind, colour, numbers…]`. The colour
    /// is written as a role name or `#RRGGBB` and goes on the wire resolved,
    /// so the client never parses a string while painting.
    fn paths_value(&mut self, v: &Json) -> Result<WireValue, String> {
        let paths = v.as_array().ok_or("EUI: paths must be a list of paths")?;
        let mut out = Vec::with_capacity(paths.len());
        for path in paths {
            let parts = path
                .as_array()
                .ok_or("EUI: a path is a list: kind, colour, numbers")?;
            let mut items = Vec::with_capacity(parts.len());
            for (i, part) in parts.iter().enumerate() {
                items.push(if i == 1 {
                    WireValue::Color(self.color(part)?)
                } else {
                    self.wire_value(part)?
                });
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
        let s = v
            .as_str()
            .ok_or("EUI: a colour is a role name or #RRGGBB")?;
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
        role_id(s)
            .map(ColorRef::role)
            .ok_or_else(|| format!("EUI: unknown colour role '{s}'"))
    }

    fn style_record(&mut self, s: &Json) -> Result<StyleRecord, String> {
        let obj = s.as_object().ok_or("EUI: style must be a hash")?;
        let mut r = StyleRecord::default();
        for (k, v) in obj {
            match k.as_str() {
                "display" => {
                    r.display = enum_of(
                        v,
                        &[
                            ("row", Display::Row),
                            ("column", Display::Column),
                            ("stack", Display::Stack),
                            ("grid", Display::Grid),
                            ("none", Display::None),
                        ],
                    )?
                }
                "wrap" => {
                    r.wrap = enum_of(
                        v,
                        &[
                            ("nowrap", Wrap::NoWrap),
                            ("wrap", Wrap::Wrap),
                            ("wrap_reverse", Wrap::WrapReverse),
                        ],
                    )?
                }
                "justify" => {
                    r.justify = enum_of(
                        v,
                        &[
                            ("start", Justify::Start),
                            ("center", Justify::Center),
                            ("end", Justify::End),
                            ("between", Justify::Between),
                            ("around", Justify::Around),
                            ("evenly", Justify::Evenly),
                        ],
                    )?
                }
                "align" => {
                    r.align_items = enum_of(
                        v,
                        &[
                            ("start", AlignItems::Start),
                            ("center", AlignItems::Center),
                            ("end", AlignItems::End),
                            ("stretch", AlignItems::Stretch),
                            ("baseline", AlignItems::Baseline),
                        ],
                    )?
                }
                "self" => {
                    r.align_self = enum_of(
                        v,
                        &[
                            ("start", AlignSelf::Start),
                            ("center", AlignSelf::Center),
                            ("end", AlignSelf::End),
                            ("stretch", AlignSelf::Stretch),
                            ("baseline", AlignSelf::Baseline),
                            ("auto", AlignSelf::Auto),
                        ],
                    )?
                }
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
                "blur" => r.blur = u8_of(v)?,
                "font" => {
                    r.font_family =
                        enum_of(v, &[("sans", FontFamily::Sans), ("mono", FontFamily::Mono)])?
                }
                "size" => r.font_size = u8_of(v)?,
                "weight" => {
                    r.font_weight = enum_of(
                        v,
                        &[
                            ("regular", FontWeight::Regular),
                            ("medium", FontWeight::Medium),
                            ("semibold", FontWeight::Semibold),
                            ("bold", FontWeight::Bold),
                        ],
                    )?
                }
                "text_align" => {
                    r.text_align = enum_of(
                        v,
                        &[
                            ("start", TextAlign::Start),
                            ("center", TextAlign::Center),
                            ("end", TextAlign::End),
                            ("justify", TextAlign::Justify),
                        ],
                    )?
                }
                "clamp" => r.line_clamp = u8_of(v)?,
                "underline" => r.text_decoration |= u8::from(v.as_bool().unwrap_or(false)),
                "strike" => r.text_decoration |= u8::from(v.as_bool().unwrap_or(false)) << 1,
                "overflow" => {
                    r.overflow = enum_of(
                        v,
                        &[
                            ("visible", Overflow::Visible),
                            ("clip", Overflow::Clip),
                            ("scroll", Overflow::Scroll),
                        ],
                    )?
                }
                "transition" => {
                    r.transition =
                        enum_of(v, &[("none", 0), ("fast", 1), ("base", 2), ("slow", 3)])?
                }
                "animation" => r.animation = enum_of(v, &[("none", 0), ("spin", 1), ("enter", 2)])?,
                "position" => {
                    r.position = enum_of(
                        v,
                        &[("flow", Position::Flow), ("absolute", Position::Absolute)],
                    )?
                }
                "z" => r.z = u8_of(v)?,
                "cursor" => {
                    r.cursor = enum_of(
                        v,
                        &[
                            ("default", Cursor::Default),
                            ("pointer", Cursor::Pointer),
                            ("text", Cursor::Text),
                            ("grab", Cursor::Grab),
                            ("grabbing", Cursor::Grabbing),
                            ("resize_h", Cursor::ResizeH),
                            ("resize_v", Cursor::ResizeV),
                            ("wait", Cursor::Wait),
                            ("not_allowed", Cursor::NotAllowed),
                        ],
                    )?
                }
                other => return Err(format!("EUI: unknown style key '{other}'")),
            }
        }
        Ok(r)
    }
}

fn kind_of(name: &str) -> Result<NodeKind, String> {
    Ok(match name {
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
        "audio" => NodeKind::Audio,
        "video" => NodeKind::Video,
        other => return Err(format!("EUI: unknown node kind '{other}'")),
    })
}

/// The client refuses a tree nested past `MAX_TREE_DEPTH`; and every walk
/// of a tree here recurses on the native stack, which a view over nested
/// data could otherwise run off — and that aborts the process, not the
/// event.
fn too_deep(depth: u32) -> Result<(), String> {
    if depth > eui_proto::limits::MAX_TREE_DEPTH {
        return Err(format!(
            "EUI: the tree is nested more than {} deep — the client accepts no more; flatten it",
            eui_proto::limits::MAX_TREE_DEPTH
        ));
    }
    Ok(())
}

/// A 128-bit fingerprint of a plain-data view value — strings, numbers,
/// booleans, null, arrays and hashes of those — `None` for anything else.
/// The keyed style cache lives on this: two style hashes with the same
/// fields fingerprint the same, whatever object they are.
fn fingerprint_of(v: &Value) -> Option<[u8; 16]> {
    fn feed(v: &Value, h: &mut blake3::Hasher) -> bool {
        match v {
            Value::Null => h.update(b"n"),
            Value::Bool(b) => h.update(if *b { b"t" } else { b"f" }),
            Value::Int(i) => h.update(b"i").update(&i.to_le_bytes()),
            Value::Float(f) => h.update(b"d").update(&f.to_bits().to_le_bytes()),
            Value::String(s) | Value::Symbol(s) => h
                .update(b"s")
                .update(&(s.len() as u64).to_le_bytes())
                .update(s.as_bytes()),
            Value::Array(items) => {
                let items = items.borrow();
                h.update(b"a").update(&(items.len() as u64).to_le_bytes());
                for item in items.iter() {
                    if !feed(item, h) {
                        return false;
                    }
                }
                h
            }
            Value::Hash(pairs) => {
                let pairs = pairs.borrow();
                h.update(b"h").update(&(pairs.len() as u64).to_le_bytes());
                for (k, val) in pairs.iter() {
                    let HashKey::String(name) = k else {
                        return false;
                    };
                    h.update(&(name.len() as u64).to_le_bytes())
                        .update(name.as_bytes());
                    if !feed(val, h) {
                        return false;
                    }
                }
                h
            }
            _ => return false,
        };
        true
    }
    let mut h = blake3::Hasher::new();
    if !feed(v, &mut h) {
        return None;
    }
    let mut out = [0u8; 16];
    out.copy_from_slice(&h.finalize().as_bytes()[..16]);
    Some(out)
}

fn enum_of<T: Copy>(v: &Json, table: &[(&str, T)]) -> Result<T, String> {
    let s = v.as_str().ok_or("EUI: expected a name")?;
    table
        .iter()
        .find(|(n, _)| *n == s)
        .map(|(_, t)| *t)
        .ok_or_else(|| format!("EUI: unknown value '{s}'"))
}

fn u8_of(v: &Json) -> Result<u8, String> {
    v.as_u64()
        .and_then(|n| u8::try_from(n).ok())
        .ok_or_else(|| format!("EUI: expected 0–255, got {v}"))
}

/// `"auto"`, `12` (px), `"50%"`, `"sp:4"` (space index), `"1fr"`.
fn dim_of(v: &Json) -> Result<Dim, String> {
    match v {
        Json::Number(n) => n
            .as_u64()
            .and_then(|n| u16::try_from(n).ok())
            .map(Dim::Px)
            .ok_or_else(|| "EUI: px must be 0–65535".to_string()),
        Json::String(s) if s == "auto" => Ok(Dim::Auto),
        Json::String(s) if s.ends_with('%') => s
            .trim_end_matches('%')
            .parse::<f64>()
            .ok()
            .map(|p| Dim::Percent((p * 100.0).round() as u16))
            .ok_or_else(|| "EUI: bad percent".into()),
        Json::String(s) if s.ends_with("fr") => s
            .trim_end_matches("fr")
            .parse::<f64>()
            .ok()
            .map(|f| Dim::Fr((f * 100.0).round() as u16))
            .ok_or_else(|| "EUI: bad fr".into()),
        Json::String(s) if s.starts_with("sp:") => s[3..]
            .parse::<u8>()
            .map(Dim::Space)
            .map_err(|_| "EUI: bad space index".into()),
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
        Json::Array(a) if a.len() == 4 => {
            Ok([u8_of(&a[0])?, u8_of(&a[1])?, u8_of(&a[2])?, u8_of(&a[3])?])
        }
        Json::Array(a) if a.len() == 2 => {
            let (y, x) = (u8_of(&a[0])?, u8_of(&a[1])?);
            Ok([y, x, y, x])
        }
        other => Err(format!(
            "EUI: edges are one index or [t, r, b, l], got {other}"
        )),
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
        "window" => EventKind::Window,
        "ended" => EventKind::Ended,
        "time_update" => EventKind::TimeUpdate,
        "wake" => EventKind::Wake,
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
        EventKind::Window => "window",
        EventKind::Ended => "ended",
        EventKind::TimeUpdate => "time_update",
        EventKind::Wake => "wake",
    }
}

/// `spec/05-theme.md` §1, by name.
const ROLES: [&str; 28] = [
    "surface.base",
    "surface.raised",
    "surface.sunken",
    "surface.overlay",
    "text.default",
    "text.muted",
    "text.inverted",
    "text.disabled",
    "accent.base",
    "accent.hover",
    "accent.active",
    "accent.on",
    "success.base",
    "success.subtle",
    "success.on",
    "warning.base",
    "warning.subtle",
    "warning.on",
    "danger.base",
    "danger.subtle",
    "danger.on",
    "info.base",
    "info.subtle",
    "info.on",
    "border.subtle",
    "border.default",
    "border.strong",
    "focus.ring",
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
                let Some(pin) = pins.remove(&n.identity) else {
                    continue;
                };
                n.size = count_nodes(n) as u32;
                let arc = Arc::new(std::mem::replace(
                    n,
                    TNode {
                        id: 0,
                        kind: NodeKind::Box,
                        style: 0,
                        key: None,
                        key_atom: 0,
                        text: None,
                        props: Vec::new(),
                        handlers: Vec::new(),
                        children: Vec::new(),
                        identity: 0,
                        size: 0,
                    },
                ));
                let children = arc
                    .children
                    .iter()
                    .filter_map(|c| match c {
                        Child::Kept(k) if k.identity != 0 => Some(k.identity),
                        _ => None,
                    })
                    .collect();
                memo.entries.insert(
                    arc.identity,
                    MemoEntry {
                        _pin: pin,
                        tree: Arc::clone(&arc),
                        seen: generation,
                        children,
                    },
                );
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
        WireValue::Atom(a) => enc
            .atom_name(*a)
            .map(|s| Json::String(s.to_string()))
            .unwrap_or(Json::Null),
        WireValue::Str(s) => Json::String(s.clone()),
        WireValue::Asset(h) => Json::String(h.iter().map(|b| format!("{b:02x}")).collect()),
        WireValue::Color(c) => Json::from(c.0),
        WireValue::List(items) => Json::Array(items.iter().map(|i| wire_to_json(enc, i)).collect()),
    }
}

/// A key names one child among its siblings; two children sharing one would
/// both match the same old node in the diff, and the diff would then move a
/// child to an index its parent does not have. Said here, where the view
/// author can act on it, as the error every other malformed tree gets.
fn refuse_repeated_keys(node: &TNode) -> Result<(), String> {
    let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
    for child in &node.children {
        if let Some(key) = child.key.as_deref() {
            if !seen.insert(key) {
                return Err(format!(
                    "EUI: key '{key}' is used by two children of one {:?} node",
                    node.kind
                ));
            }
        }
    }
    Ok(())
}

fn find(node: &TNode, id: u32) -> Option<&TNode> {
    if node.id == id {
        return Some(node);
    }
    node.children.iter().find_map(|c| find(c.node(), id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn an_atom_is_found_by_its_id() {
        let mut enc = Encoder::default();
        let a = enc.atom("alpha");
        let b = enc.atom("beta");
        assert_eq!(enc.atom("alpha"), a, "interned once");
        assert_eq!(enc.atom_name(a), Some("alpha"));
        assert_eq!(enc.atom_name(b), Some("beta"));
        assert_eq!(enc.atom_name(0), None);
        assert_eq!(enc.atom_name(99), None);
    }

    #[test]
    fn an_event_resolves_against_the_last_tree_in_one_lookup() {
        let mut enc = Encoder::default();
        let tree = json!({"k": "box", "c": [
            {"k": "box", "key": "row-7", "p": {"id": 7}, "on": {"click": "pick"}}
        ]});
        enc.render(&tree, false).unwrap();
        let row = enc.prev.as_ref().unwrap().children[0].id;
        let (name, props) = enc.event_target(row, EventKind::Click).unwrap();
        assert_eq!(name, "pick");
        assert_eq!(props.get("id"), Some(&json!(7)));
        assert!(
            enc.event_target(row, EventKind::Change).is_none(),
            "no such handler"
        );
        assert!(
            enc.event_target(9999, EventKind::Click).is_none(),
            "no such node"
        );
    }

    #[test]
    fn a_style_written_twice_is_one_record_and_one_fingerprint_lookup() {
        let style = || {
            h(vec![
                ("display", s("row")),
                ("gap", Value::Int(4)),
                ("bg", s("surface.base")),
            ])
        };
        let mut enc = Encoder::default();
        let a = enc.style_of_value(&style()).unwrap();
        assert_eq!(enc.style_fingerprints.len(), 1);
        let b = enc.style_of_value(&style()).unwrap();
        assert_eq!(a, b, "the same fields are the same style");
        assert_eq!(enc.styles.len(), 1);
        let c = enc
            .style_of_value(&h(vec![("display", s("column"))]))
            .unwrap();
        assert_ne!(a, c);
        assert_eq!(enc.style_fingerprints.len(), 2);
    }

    #[test]
    fn a_local_handler_is_compiled_once_per_source() {
        let mut enc = Encoder::default();
        let tree = || {
            json!({"k": "box", "c": [
                {"k": "box", "key": "a", "on": {"click": {"local": "state.n += 1"}}},
                {"k": "box", "key": "b", "on": {"click": {"local": "state.n += 1"}}}
            ]})
        };
        enc.render(&tree(), false).unwrap();
        assert_eq!(enc.local_cache.len(), 2, "keyed by source and by self key");
        assert_eq!(enc.chunks.len(), 1, "the bytes are the same chunk");
        enc.render(&tree(), false).unwrap();
        assert_eq!(enc.local_cache.len(), 2);
    }

    #[test]
    fn the_client_limits_are_the_server_limits() {
        let mut enc = Encoder::default();
        let long = "x".repeat(eui_proto::limits::MAX_INLINE_STR + 1);
        let err = enc
            .render(&json!({"k": "text", "t": long}), false)
            .unwrap_err();
        assert!(err.contains("bytes"), "{err}");

        let mut deep = json!({"k": "box"});
        for _ in 0..=eui_proto::limits::MAX_TREE_DEPTH {
            deep = json!({"k": "box", "c": [deep]});
        }
        let err = enc.render(&deep, false).unwrap_err();
        assert!(err.contains("nested"), "{err}");

        let mut enc = Encoder::default();
        for i in 0..=eui_proto::limits::MAX_COLORS {
            enc.color_literal(i);
        }
        let err = enc.render(&json!({"k": "box"}), false).unwrap_err();
        assert!(err.contains("colours"), "{err}");
    }

    #[test]
    fn a_tree_too_big_for_one_frame_is_mounted_in_pieces() {
        // Rows of 4 KiB of text: a few hundred outweigh a batch.
        let rows: Vec<Json> = (0..1200)
            .map(|i| json!({"k": "text", "key": format!("r{i}"), "t": "x".repeat(4000)}))
            .collect();
        let tree = json!({"k": "scroll", "c": [{"k": "box", "c": rows}]});
        let mut enc = Encoder::default();
        let batches = enc.render(&tree, false).unwrap();
        assert!(batches.len() > 1, "{} batch(es)", batches.len());
        let mut nodes = 0;
        for b in &batches {
            let w: usize = b.ops.iter().map(op_weight).sum();
            assert!(
                w <= MAX_BATCH_WEIGHT + 4096 + NODE_WEIGHT,
                "batch weighs {w}"
            );
            for op in &b.ops {
                if let Op::Mount(t) | Op::InsertChild { subtree: t, .. } = op {
                    nodes += t.nodes.len();
                }
            }
        }
        assert_eq!(nodes, 1202, "every node is sent exactly once");
        // One Mount, and it comes before any insert — the atoms the rows
        // interned come first of all, a thousand of them to a batch.
        let flat: Vec<&Op> = batches.iter().flat_map(|b| b.ops.iter()).collect();
        let mount_at = flat
            .iter()
            .position(|op| matches!(op, Op::Mount(_)))
            .unwrap();
        assert_eq!(
            flat.iter().filter(|op| matches!(op, Op::Mount(_))).count(),
            1
        );
        let first_insert = flat
            .iter()
            .position(|op| matches!(op, Op::InsertChild { .. }))
            .unwrap();
        assert!(mount_at < first_insert);
        // The seqs are consecutive.
        for (i, b) in batches.iter().enumerate() {
            assert_eq!(b.seq, i as u64 + 1);
        }
    }

    #[test]
    fn a_key_repeated_among_siblings_is_refused() {
        let mut enc = Encoder::default();
        let tree = json!({"k": "box", "c": [
            {"k": "text", "key": "row", "t": "a"},
            {"k": "text", "key": "row", "t": "b"}
        ]});
        let err = enc.render(&tree, false).unwrap_err();
        assert!(err.contains("key 'row'"), "{err}");
    }

    #[test]
    fn the_same_key_under_different_parents_is_fine() {
        let mut enc = Encoder::default();
        let tree = json!({"k": "box", "c": [
            {"k": "box", "c": [{"k": "text", "key": "row", "t": "a"}]},
            {"k": "box", "c": [{"k": "text", "key": "row", "t": "b"}]}
        ]});
        assert!(enc.render(&tree, false).is_ok());
    }

    fn s(x: &str) -> Value {
        Value::String(x.into())
    }

    fn h(pairs: Vec<(&str, Value)>) -> Value {
        let mut map = crate::interpreter::value::HashPairs::default();
        for (k, v) in pairs {
            map.insert(HashKey::String(k.into()), v);
        }
        Value::Hash(Rc::new(RefCell::new(map)))
    }

    fn list(items: Vec<Value>) -> Value {
        Value::Array(Rc::new(RefCell::new(items)))
    }

    #[test]
    fn keyed_rows_inside_a_kept_card_stay_warm() {
        let row_a = h(vec![("k", s("text")), ("key", s("a")), ("t", s("a"))]);
        let row_b = h(vec![("k", s("text")), ("key", s("b")), ("t", s("b"))]);
        let card = h(vec![
            ("k", s("box")),
            ("key", s("card")),
            ("c", list(vec![row_a.clone(), row_b.clone()])),
        ]);
        let root = |card: &Value| h(vec![("k", s("box")), ("c", list(vec![card.clone()]))]);
        let mut enc = Encoder::default();
        enc.render_value("warm", &root(&card), false).unwrap();
        assert_eq!(memo_entries("warm"), 3, "card and both rows are kept");
        // The card is the same object: kept whole, its rows never walked —
        // and still kept, because the card keeps them.
        enc.render_value("warm", &root(&card), false).unwrap();
        assert_eq!(memo_entries("warm"), 3);
        // A new card object with the same rows: the rows are hits.
        let card2 = h(vec![
            ("k", s("box")),
            ("key", s("card")),
            ("c", list(vec![row_a.clone(), row_b.clone()])),
        ]);
        let batches = enc.render_value("warm", &root(&card2), false).unwrap();
        assert_eq!(memo_entries("warm"), 3);
        let ops: usize = batches.iter().map(|b| b.ops.len()).sum();
        assert_eq!(ops, 0, "nothing changed on the screen");
        forget_session("warm");
        assert_eq!(memo_entries("warm"), 0);
    }

    #[test]
    fn a_session_with_no_encoder_is_swept() {
        // The root is never kept — only children are — so the keyed node is
        // one level down.
        let node = || {
            h(vec![
                ("k", s("box")),
                (
                    "c",
                    list(vec![h(vec![("k", s("box")), ("key", s("only"))])]),
                ),
            ])
        };
        let mut gone = Encoder::default();
        gone.render_value("gone", &node(), false).unwrap();
        assert_eq!(memo_entries("gone"), 1);
        // Neither session has an encoder in the registry; the sweep keeps
        // the one it is rendering for and drops the other.
        let mut live = Encoder::default();
        for _ in 0..SWEEP_EVERY {
            live.render_value("live", &node(), false).unwrap();
        }
        assert_eq!(memo_entries("gone"), 0);
        assert_eq!(memo_entries("live"), 1);
    }
}
