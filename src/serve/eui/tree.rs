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
    FontWeight, Gradient, GradientStop, Handler, Justify, Motion, NodeKind, Op, Overflow, Position,
    StyleRecord, Subtree, TextAlign, TextRef, Value as WireValue, Wrap, Writer,
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
    /// Where this node's view asked it to be scrolled, in pixels (EUI 04
    /// §7). It is not a prop: the client has no `scroll_to` to set, and a
    /// scroll is a thing done to a node once, not a state it carries. The
    /// diff turns a change of this into `Op::ScrollTo` and nothing else.
    pub scroll_to: Option<(i64, i64)>,
    /// Whether this node's view asked it to take focus (EUI 02 §5,
    /// `Op::Focus`). The same shape as `scroll_to`, and for the same
    /// reason: focusing is something done to a node once.
    ///
    /// `autofocus` cannot cover this. It is deliberately weak -- a client
    /// applies it "only when focus is not already where it belongs"
    /// (03 §3.1), so that a batch arriving mid-Tab does not yank someone
    /// back to a dialog's first field. That makes it useless for a view
    /// that *opens* a field: focus is already on whatever was there
    /// before, so the new field never gets it. `Op::Focus` is the op the
    /// protocol has for exactly this, and nothing was emitting it.
    pub focus_to: bool,
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
    /// The protocol version this session settled on -- `hello.version`
    /// capped at ours (01 §2). `0` means nobody has said yet, which the
    /// encoder reads as "ours", so a path with no handshake behind it (a
    /// test, a render outside a session) is not silently degraded.
    ///
    /// It exists so that a handler naming an event an older client cannot
    /// decode can be left out instead of ending its session. A kind it has
    /// never heard of is `UnknownTag` and the batch carrying it is fatal,
    /// which is a steep price for a meter that would simply not move.
    version: u32,
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
    /// Gradients by value (eui 02 §5.3): defined once per session, like the
    /// colours their stops may name, and never for a session below 6.
    gradients: HashMap<Gradient, u32>,
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
    /// Whether this encoder's renders may keep subtrees in the worker's memo.
    ///
    /// A one-shot render — `GET /_eui/view/<component>`, which has no socket
    /// and no second render to be warm for — must not. The memo is a
    /// thread-local holding interpreter values, so what it keeps for a
    /// session that is already over can only be freed by asking the right
    /// worker nicely, and a render that never asked is a render with nothing
    /// to free. Cheaper and more certain than posting a forget afterwards
    /// and hoping the queue takes it.
    ephemeral: bool,
    generation: u32,
    /// How many nodes the last tree had, for `eui_stats()`.
    last_nodes: usize,
    /// Font roles this session has already been told about, so a `DefFont`
    /// goes out once rather than once per style that names the family.
    fonts_sent: std::collections::HashSet<u8>,
    /// What the last render spent, phase by phase, in milliseconds:
    /// converting the view value, counting the nodes, diffing, freezing the
    /// keyed memo, and cutting the ops into batches. `EUI_TRACE=1` prints
    /// them; without it they are five stores nobody reads.
    phase_convert: f64,
    phase_count: f64,
    phase_diff: f64,
    phase_freeze: f64,
    phase_batch: f64,
}

/// Style ids by the *identity* of the value that produced them, for the
/// length of one conversion pass.
///
/// A fingerprint is a walk of the hash and a BLAKE3 over every field; this is
/// a pointer comparison. A style the view hoists out of its loop — what the
/// catalogue's builders do, and what a table of ten thousand rows wants — is
/// then fingerprinted once per render instead of once per node. A style built
/// fresh per node has a fresh pointer and falls through to the fingerprint,
/// so nothing is lost by trying.
///
/// It keeps the value, and that is not bookkeeping: an address is unique only
/// among things that are *alive*. A style dropped after its lookup frees an
/// allocation the next one can be handed, and a pointer that answered for the
/// old contents would then quietly dress a node in somebody else's style.
/// Holding an `Rc` clone means nothing the memo remembers can be freed while
/// it remembers it.
///
/// A pass local rather than a field on the encoder, for the same two reasons
/// `pins` is: an interpreter value cannot cross threads, and a hash is
/// mutable — within one pass it cannot move, since the view has already
/// returned the whole tree, but between two a handler may have written to the
/// very hash a module constant holds.
#[derive(Default)]
struct StyleMemo {
    by_identity: HashMap<usize, (Value, u32)>,
}

impl StyleMemo {
    fn get(&self, at: &usize) -> Option<u32> {
        self.by_identity.get(at).map(|(_, id)| *id)
    }

    fn remember(&mut self, at: usize, value: Value, id: u32) {
        self.by_identity.insert(at, (value, id));
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.by_identity.len()
    }
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
        // Thirty-two bytes a face, and a role may carry eight.
        Op::DefFont { faces, .. } => 8 + faces.len() * eui_proto::limits::HASH_BYTES,
        Op::SetText {
            text: TextRef::Inline(s),
            ..
        } => 8 + s.len(),
        Op::Notify { title, body, tag } => 16 + title.len() + body.len() + tag.len(),
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
        scroll_ops(tree, &mut ops);
        return ops;
    }
    let mut bare = tree.clone();
    let children = std::mem::take(&mut bare.children);
    ops.push(Op::Mount(flatten(&bare)));
    scroll_ops(&bare, &mut ops);
    for (index, child) in children.iter().enumerate() {
        insert_ops(tree.id, index as u32, child.node(), &mut ops);
    }
    ops
}

/// `Op::ScrollTo` for every node in a freshly sent subtree that asked to be
/// scrolled. A mount arrives at the top of its scroller, so a list that
/// opens at its foot has to say so on the way in as well as on every render
/// after — and the op must come *after* the subtree that it names.
pub fn scroll_ops(node: &TNode, ops: &mut Vec<Op>) {
    if let Some((x, y)) = node.scroll_to {
        ops.push(Op::ScrollTo {
            node: node.id,
            x,
            y,
        });
    }
    // A field that arrives already asking for the caret gets it here: the
    // subtree is new, so there is no previous answer to compare against.
    if node.focus_to {
        ops.push(Op::Focus { node: node.id });
    }
    for child in &node.children {
        scroll_ops(child.node(), ops);
    }
}

fn insert_ops(parent: u32, index: u32, node: &TNode, ops: &mut Vec<Op>) {
    if tree_weight(node) <= MAX_BATCH_WEIGHT {
        ops.push(Op::InsertChild {
            parent,
            index,
            subtree: flatten(node),
        });
        scroll_ops(node, ops);
        return;
    }
    let mut bare = node.clone();
    let children = std::mem::take(&mut bare.children);
    ops.push(Op::InsertChild {
        parent,
        index,
        subtree: flatten(&bare),
    });
    scroll_ops(&bare, ops);
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

    /// A `bg`: a colour as [`Encoder::color`] reads one, or a hash naming a
    /// linear gradient (eui 02 §5.3):
    ///
    /// ```text
    /// {"gradient": {"to": "right", "stops": ["accent.base", ["#ff80b5", 255]]}}
    /// {"gradient": {"angle": 45, "stops": [["accent.base", 26], "info.base"]}}
    /// ```
    ///
    /// `to` is a side or a corner as CSS writes it (`top right`), `angle`
    /// whole degrees; neither means `bottom`, as in CSS. A stop is a colour,
    /// or `[colour, at]` with `at` in 255ths of the way along; a stop given
    /// no position is spread evenly, as CSS spreads them. Two or three.
    ///
    /// Interned per session by value. A session below version 6 cannot
    /// decode one, so it is sent the first stop as a solid `bg` instead —
    /// the colour the gradient starts from, rather than a refused batch.
    fn bg(&mut self, v: &Json) -> Result<ColorRef, String> {
        let Some(obj) = v.as_object() else {
            return self.color(v);
        };
        let spec = match (obj.len(), obj.get("gradient")) {
            (1, Some(Json::Object(g))) => g,
            _ => {
                return Err(
                    "EUI: a bg hash is {\"gradient\": {\"to\": ..., \"stops\": [...]}}".into(),
                )
            }
        };
        let angle = match (spec.get("to"), spec.get("angle")) {
            (Some(_), Some(_)) => {
                return Err("EUI: a gradient takes `to` or `angle`, not both".into())
            }
            (Some(to), None) => gradient_to(to)?,
            (None, Some(a)) => a
                .as_u64()
                .filter(|d| *d < 360)
                .map(|d| d as u16)
                .ok_or("EUI: a gradient's angle is whole degrees, 0 to 359")?,
            (None, None) => 180,
        };
        for key in spec.keys() {
            if !["to", "angle", "stops"].contains(&key.as_str()) {
                return Err(format!(
                    "EUI: a gradient has `to` or `angle`, and `stops`; not `{key}`"
                ));
            }
        }
        let raw = spec
            .get("stops")
            .and_then(Json::as_array)
            .ok_or("EUI: a gradient needs `stops`, a list of two or three colours")?;
        if !(2..=eui_proto::limits::MAX_GRADIENT_STOPS).contains(&raw.len()) {
            return Err(format!(
                "EUI: a gradient has two or three stops, not {}",
                raw.len()
            ));
        }
        let last = raw.len() - 1;
        let mut stops = Vec::with_capacity(raw.len());
        for (i, stop) in raw.iter().enumerate() {
            let (colour, at) = match stop {
                Json::Array(pair) if pair.len() == 2 => {
                    let at = pair[1]
                        .as_u64()
                        .and_then(|n| u8::try_from(n).ok())
                        .ok_or("EUI: a gradient stop's position is 0 to 255")?;
                    (&pair[0], at)
                }
                Json::Array(_) => {
                    return Err("EUI: a gradient stop is a colour or [colour, position]".into())
                }
                other => (other, ((i * 255 + last / 2) / last) as u8),
            };
            let color = self.color(colour)?;
            if color.is_none() {
                return Err("EUI: a gradient stop is a colour, not none".into());
            }
            stops.push(GradientStop { color, at });
        }
        if stops.windows(2).any(|w| w[1].at < w[0].at) {
            return Err("EUI: a gradient's stops go forwards along it".into());
        }
        let gradient =
            Gradient::new(angle, &stops).ok_or("EUI: a gradient has two or three stops")?;
        if self.protocol() < 6 {
            return Ok(gradient.first());
        }
        if let Some(id) = self.gradients.get(&gradient) {
            return Ok(ColorRef::gradient(*id as u16));
        }
        let id = self.gradients.len() as u32 + 1;
        if id > eui_proto::limits::MAX_GRADIENTS {
            self.overflowed("gradients", eui_proto::limits::MAX_GRADIENTS);
        }
        self.gradients.insert(gradient, id);
        self.pending.push(Op::DefGradient { id, gradient });
        Ok(ColorRef::gradient(id as u16))
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

    /// `"sans"`, `"mono"`, or the name of a font this application declared
    /// with `eui_font` (02 §5.1).
    ///
    /// The role's `DefFont` is emitted the first time a style names it, and
    /// once per session: the faces are the same hashes every render, and a
    /// client that has been told cannot be untold. A name nothing declared
    /// is an error here, where the view's author is, rather than a role the
    /// client would draw in sans without ever saying why.
    fn font_family(&mut self, v: &Json) -> Result<FontFamily, String> {
        let name = v.as_str().ok_or("EUI: expected a name")?;
        match name {
            "sans" if super::fonts::role_of("sans").is_none() => return Ok(FontFamily::Sans),
            "mono" if super::fonts::role_of("mono").is_none() => return Ok(FontFamily::Mono),
            _ => {}
        }
        // A font role and the `DefFont` that binds it are both EUI 4, and a
        // client below it meets either as a decode error and ends the
        // session. The manifest keeps such a client away from an
        // application that declared a font at boot — but an application may
        // declare one from a view, with sessions already open, and a
        // typeface is not worth a window. So an older session is drawn in
        // the client's own face, the way `since` leaves a `level` handler
        // out rather than sending it: the application works and the
        // typography does not, which is the right way round.
        if self.protocol() < 4 {
            return Ok(match name {
                "mono" => FontFamily::Mono,
                _ => FontFamily::Sans,
            });
        }
        let role = super::fonts::role_of(name).ok_or_else(|| {
            let mut known = vec!["sans".to_owned(), "mono".to_owned()];
            // A declared `sans` or `mono` is a *rebinding* of the client's
            // own role, so it is already in the list above.
            known.extend(
                super::fonts::names()
                    .into_iter()
                    .filter(|n| n != "sans" && n != "mono"),
            );
            format!(
                "EUI: unknown font '{name}'; declare it with eui_font(\"{name}\", [...]) — known: {}",
                known.join(", ")
            )
        })?;
        if self.fonts_sent.insert(role) {
            let faces = super::fonts::faces_of(role)
                .ok_or_else(|| format!("EUI: font '{name}' names no face"))?;
            self.pending.push(Op::DefFont { role, faces });
        }
        FontFamily::from_u8(role).map_err(|e| format!("EUI: font role {role}: {e}"))
    }

    fn style(&mut self, record: StyleRecord) -> u32 {
        // 05 §2: `space` indices 13-17 are version 6. A session that
        // settled lower is sent the older step each falls back to -- its
        // client would refuse the batch otherwise -- and every `DefStyle`
        // passes through here, so this is the one place it is done. The
        // same call takes off the version-6 `animation` bits, pulse and
        // bounce (03 §5); a gradient `bg` was already replaced by its first
        // stop where it was read (`bg`).
        let record = record.for_protocol(self.protocol());
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
        // An ephemeral encoder names no session to the memo, which is how it
        // leaves nothing behind on the worker that rendered it: `finish`
        // freezes keyed subtrees only when this is non-empty.
        self.session = if self.ephemeral {
            String::new()
        } else {
            session.to_owned()
        };
        self.generation = self.generation.wrapping_add(1);
        // The view values behind this render's keyed nodes, by identity,
        // until `finish` pins them in the memo. Held here, not on the
        // encoder: an interpreter value cannot cross threads.
        let mut pins: HashMap<usize, Value> = HashMap::new();
        let mut styles = StyleMemo::default();
        let t_convert = std::time::Instant::now();
        let tree = match self.convert_value(value, &mut pins, &mut styles, 1)? {
            Some(Child::Fresh(n)) => n,
            Some(Child::Kept(rc)) => (*rc).clone(),
            None => return Err("EUI: the view returned nothing".into()),
        };
        self.phase_convert = t_convert.elapsed().as_secs_f64() * 1e3;
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
        let t_count = std::time::Instant::now();
        let nodes = count_nodes(&tree);
        self.phase_count = t_count.elapsed().as_secs_f64() * 1e3;
        self.last_nodes = nodes;
        if nodes > eui_proto::limits::MAX_NODES as usize {
            return Err(format!(
                "EUI: the view returned {nodes} nodes; a client accepts at most {} — virtualise, paginate or trim",
                eui_proto::limits::MAX_NODES
            ));
        }
        let t_diff = std::time::Instant::now();
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
        self.phase_diff = t_diff.elapsed().as_secs_f64() * 1e3;
        // Keyed nodes converted this render are frozen behind an Arc, so the
        // next render can keep them by the identity of their view value.
        let generation = self.generation;
        let t_freeze = std::time::Instant::now();
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
        self.phase_freeze = t_freeze.elapsed().as_secs_f64() * 1e3;
        self.prev = Some(tree);
        let t_batch = std::time::Instant::now();
        let mut all = std::mem::take(&mut self.pending);
        all.extend(ops);
        // What this pass asked to say to the person (02 §5.2). After the
        // tree, so the window is showing what the notification is about by
        // the time it arrives -- the same reason a `ScrollTo` follows the
        // subtree it names.
        all.extend(super::notify::take());
        // A big update streams: every prefix of the op list is valid on its
        // own (definitions come first, ops apply in order), so a batch of
        // thousands of inserts goes out in slices the client paints between.
        // The first slice is on screen long before the last is encoded. A
        // slice is bounded by ops and by weight — an insert of a whole
        // subtree is one op and most of a frame.
        let mut batches = Vec::new();
        let mut ops = Vec::new();
        let mut weight = 0usize;
        // The third bound on a slice, and the only one that is not about
        // size: a client refuses a batch carrying more than four
        // notifications (02 §5.2), so a handler that said five things
        // sends them in two batches rather than ending its own session.
        let mut notes = 0u32;
        for op in all {
            let w = op_weight(&op);
            let note = matches!(op, Op::Notify { .. });
            if !ops.is_empty()
                && (ops.len() >= MAX_OPS_PER_BATCH
                    || weight + w > MAX_BATCH_WEIGHT
                    || (note && notes >= eui_proto::limits::MAX_NOTIFY_PER_BATCH))
            {
                self.seq += 1;
                batches.push(Batch {
                    seq: self.seq,
                    ops: std::mem::take(&mut ops),
                });
                weight = 0;
                notes = 0;
            }
            weight += w;
            notes += u32::from(note);
            ops.push(op);
        }
        if !ops.is_empty() {
            self.seq += 1;
            batches.push(Batch { seq: self.seq, ops });
        }
        // A render that changed nothing sends nothing.
        //
        // An empty batch used to go out anyway, and it is not free on either
        // end: the client applies it, and applying *any* batch puts back
        // every style a local handler had previewed and relights whatever
        // the pointer is over (06 §6). On a page with no clock nobody
        // noticed. On a page that carries a `wake` — the clock in this very
        // demo, a progress bar, a messenger — it meant the whole screen was
        // restyled once a tick forever, which reads as a flicker and is one.
        self.phase_batch = t_batch.elapsed().as_secs_f64() * 1e3;
        if super::trace() {
            eprintln!(
                "[EUI trace] phases: convert {:.2} count {:.2} diff {:.2} freeze {:.2} batch {:.2} ms",
                self.phase_convert,
                self.phase_count,
                self.phase_diff,
                self.phase_freeze,
                self.phase_batch,
            );
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
    /// The version this session speaks; ours when nobody has said.
    fn protocol(&self) -> u32 {
        if self.version == 0 {
            eui_proto::PROTOCOL_VERSION
        } else {
            self.version
        }
    }

    /// Say that this encoder renders once and is thrown away, so nothing it
    /// converts is kept in the worker's keyed memo.
    pub fn set_ephemeral(&mut self, yes: bool) {
        self.ephemeral = yes;
    }

    /// Remember what the handshake settled on (01 §2).
    pub fn set_protocol(&mut self, version: u32) {
        self.version = version;
    }

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
        styles: &mut StyleMemo,
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
            let node = self.convert_shallow_value(&borrow, styles)?;
            let kids = match borrow.get(&HashKey::String("c".into())) {
                Some(Value::Array(items)) => Some(Rc::clone(items)),
                _ => None,
            };
            (node, kids)
        };
        if let Some(kids) = kids {
            for child in kids.borrow().iter() {
                if let Some(c) = self.convert_value(child, pins, styles, depth + 1)? {
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
        let (props, scroll_to, focus_to) = match obj.get("p").and_then(Json::as_object) {
            Some(p) => self.props_from(kind, p)?,
            None => (Vec::new(), None, false),
        };
        let handlers = match obj.get("on").and_then(Json::as_object) {
            Some(on) => self.handlers_from(on, &key)?,
            None => Vec::new(),
        };
        Ok(self.node(kind, style, key, text, props, scroll_to, focus_to, handlers))
    }

    /// [`convert_shallow`] straight off the view's hash: the scalar fields
    /// are read as they are, the style is looked up by fingerprint before
    /// anything is converted, and only `p` and `on` — small, and shaped by
    /// the JSON parsers — go through JSON.
    fn convert_shallow_value(
        &mut self,
        fields: &crate::interpreter::value::HashPairs,
        styles: &mut StyleMemo,
    ) -> Result<TNode, String> {
        let field = |name: &str| fields.get(&HashKey::String(name.into()));
        let kind = match field("k") {
            None => NodeKind::Box,
            Some(Value::String(name)) => kind_of(name)?,
            Some(other) => return Err(format!("EUI: a node kind is a string, got {other}")),
        };
        let style = match field("s") {
            None => 0,
            Some(style) => self.style_of_value(style, styles)?,
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
        let (props, scroll_to, focus_to) = match field("p") {
            None => (Vec::new(), None, false),
            Some(p) => {
                let json = crate::interpreter::value::value_to_json(p)?;
                match json.as_object() {
                    Some(p) => self.props_from(kind, p)?,
                    None => (Vec::new(), None, false),
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
        Ok(self.node(kind, style, key, text, props, scroll_to, focus_to, handlers))
    }

    #[allow(clippy::too_many_arguments)]
    fn node(
        &mut self,
        kind: NodeKind,
        style: u32,
        key: Option<String>,
        text: Option<TextRef>,
        props: Vec<(u32, WireValue)>,
        scroll_to: Option<(i64, i64)>,
        focus_to: bool,
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
            scroll_to,
            focus_to,
            handlers,
            children: Vec::new(),
            identity: 0,
            size: 0,
        }
    }

    /// A style id for a view value: by fingerprint when the value is a hash
    /// seen before, else the JSON way, remembered under its fingerprint.
    fn style_of_value(&mut self, style: &Value, seen: &mut StyleMemo) -> Result<u32, String> {
        let identity = match style {
            Value::Hash(hash) => Some(Rc::as_ptr(hash) as *const u8 as usize),
            _ => None,
        };
        if let Some(id) = identity.and_then(|at| seen.get(&at)) {
            return Ok(id);
        }

        let fingerprint = match style {
            Value::Hash(_) => fingerprint_of(style),
            _ => None,
        };
        let id = if let Some(id) = fingerprint.and_then(|fp| self.style_fingerprints.get(&fp)) {
            *id
        } else {
            let json = crate::interpreter::value::value_to_json(style)?;
            let record = self.style_record(&json)?;
            let id = self.style(record);
            if let Some(fp) = fingerprint {
                self.style_fingerprints.insert(fp, id);
            }
            id
        };
        if let Some(at) = identity {
            seen.remember(at, style.clone(), id);
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

    /// The props on the wire, and — separately — where the view asked this
    /// node to be scrolled.
    ///
    /// `scroll_to` is the one name in `p` that never reaches the client as a
    /// prop. It is an instruction, not a state: `Op::ScrollTo` (04 §7) moves
    /// the node once, and a client that was handed it as a prop would have
    /// nothing to do with it. Taking it out here is what lets a view say
    /// where a list should sit without the diff learning a special case
    /// about prop names.
    fn props_from(
        &mut self,
        kind: NodeKind,
        p: &serde_json::Map<String, Json>,
    ) -> Result<NodeProps, String> {
        let mut props = Vec::with_capacity(p.len());
        let mut scroll_to = None;
        let mut focus_to = false;
        for (name, v) in p {
            if name == "scroll_to" {
                scroll_to = Some(scroll_offset(kind, v)?);
                continue;
            }
            // Like `scroll_to`: an instruction, never a prop. The client
            // has no `focus_to` to set, and handing it one would leave it
            // nothing to do with it.
            if name == "focus_to" {
                focus_to = matches!(v, Json::Bool(true));
                continue;
            }
            let atom = self.atom(name);
            // An image's `src` is a file in the application; it goes on
            // the wire as the hash of its bytes, served from /_eui/asset.
            //
            // Or it is already in the store and names itself:
            // `{"asset": "<64 hex>"}`, what `eui_asset` answers, which is how
            // bytes that have no file — an attachment in SoliDB or S3 — reach
            // a window. An object and not an `"asset:<hex>"` string because a
            // path is only rejected after it is resolved, so `asset:foo.png`
            // under `public/` is a name a file may legally have and a prefix
            // would be ambiguous with it.
            // A scene names two assets rather than one, and they travel the
            // same verified path a picture's `src` does: a hash on the wire,
            // fetched from the origin, checked against its own name before
            // anything decodes it. The client then validates both in its
            // worker -- the module against the shader verifier, the mesh
            // against its vertex count -- so nothing here has to.
            let asset_prop = matches!(kind, NodeKind::Image | NodeKind::Audio | NodeKind::Video)
                && name == "src"
                || kind == NodeKind::Scene && matches!(name.as_str(), "shader" | "mesh");
            let value = if asset_prop {
                match v {
                    Json::String(path) => WireValue::Asset(super::assets::from_file(path)?),
                    Json::Object(o) if o.len() == 1 => {
                        let hex = o.get("asset").and_then(Json::as_str).ok_or_else(|| {
                            format!("EUI: a src object must be {{\"asset\": \"<hash>\"}}, got {v}")
                        })?;
                        WireValue::Asset(super::assets::parse_hex(hex).ok_or_else(|| {
                            format!("EUI: a src asset must be 64 hex characters, got '{hex}'")
                        })?)
                    }
                    other => return Err(format!("EUI: a {name} must be a path, got {other}")),
                }
            } else if kind == NodeKind::Scene && name == "uniforms" {
                self.uniforms_value(v)?
            } else if kind == NodeKind::Canvas && name == "paths" {
                self.paths_value(v)?
            } else {
                self.wire_value(v)?
            };
            props.push((atom, value));
        }
        Ok((props, scroll_to, focus_to))
    }

    fn handlers_from(
        &mut self,
        on: &serde_json::Map<String, Json>,
        key: &Option<String>,
    ) -> Result<Vec<(EventKind, Handler)>, String> {
        let mut handlers = Vec::with_capacity(on.len());
        for (event, target) in on {
            let kind = event_kind(event).ok_or_else(|| format!("EUI: unknown event '{event}'"))?;
            // An event the other end cannot decode is left out rather than
            // sent: the view still renders, the widget just never hears
            // from it. `level` arrived in version 3 (03 §7).
            if since(kind) > self.protocol() {
                continue;
            }
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
                            // source and the ids the declared styles
                            // resolved to.
                            //
                            // The node's own key used to be in here too,
                            // because `self` was compiled to it. It is atom
                            // 0 now and resolved by the client, so two nodes
                            // with the same source and the same styles share
                            // one chunk however they are keyed — which is
                            // what lets a list of four thousand rows carry a
                            // hover at all. The table holds 4 095.
                            let mut declared_ids: Vec<(&String, &u32)> = declared.iter().collect();
                            declared_ids.sort();
                            let cache_key = format!("{src}\u{0}{declared_ids:?}");
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
    /// A scene's eight floats: the author's half of the uniform block.
    ///
    /// Eight and not more, because the other twenty-four are the client's --
    /// the matrix, the clock and the size. A server therefore never sends a
    /// camera, and so can never send a degenerate one. Short lists are
    /// padded rather than refused: an author who wants one number should not
    /// have to write seven zeroes after it.
    fn uniforms_value(&mut self, v: &Json) -> Result<WireValue, String> {
        let given = v
            .as_array()
            .ok_or("EUI: a scene's uniforms must be a list of numbers")?;
        if given.len() > 8 {
            return Err(format!(
                "EUI: a scene has eight uniforms, got {}",
                given.len()
            ));
        }
        let mut out = Vec::with_capacity(8);
        for n in given {
            let f = n
                .as_f64()
                .ok_or_else(|| format!("EUI: a scene's uniforms must be numbers, got {n}"))?;
            if !f.is_finite() {
                return Err("EUI: a scene's uniforms must be finite".to_owned());
            }
            out.push(WireValue::Float(f));
        }
        out.resize(8, WireValue::Float(0.0));
        Ok(WireValue::List(out))
    }

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
                "bg" => r.bg = self.bg(v)?,
                "fg" => r.fg = self.color(v)?,
                "border_color" => r.border_color = self.color(v)?,
                "border" => r.border_width = edges_of(v)?,
                "radius" => r.radius = u8_of(v)?,
                "shadow" => r.shadow = u8_of(v)?,
                "opacity" => r.opacity = u8_of(v)?,
                "blur" => r.blur = u8_of(v)?,
                "font" => r.font_family = self.font_family(v)?,
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
                    r.transition = enum_of(
                        v,
                        &[
                            ("none", 0),
                            ("fast", 1),
                            ("base", 2),
                            ("slow", 3),
                            ("slower", 4),
                            ("slowest", 5),
                        ],
                    )?
                }
                "animation" => r.animation = animation_of(v)?,
                "motion" => {
                    r.motion = enum_of(
                        v,
                        &[
                            ("fade", Motion::Fade),
                            ("leading", Motion::Leading),
                            ("trailing", Motion::Trailing),
                            ("top", Motion::Top),
                            ("bottom", Motion::Bottom),
                            ("scale", Motion::Scale),
                            ("paired", Motion::Paired),
                        ],
                    )?
                }
                "position" => {
                    r.position = enum_of(
                        v,
                        &[
                            ("flow", Position::Flow),
                            ("absolute", Position::Absolute),
                            ("pointer", Position::Pointer),
                        ],
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
        "scene" => NodeKind::Scene,
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
    // Serialised first, hashed once.
    //
    // This used to feed BLAKE3 directly, a dozen `update` calls of a few
    // bytes each per style — and BLAKE3's per-call buffering, not its
    // compression, was most of what a style lookup cost: 895 ns for a
    // three-key style, of which the walk was 76 and hashing all 83 bytes in
    // one go was 195. The bytes fed are exactly the same, so the digest is
    // exactly the same; only the number of calls changed.
    fn feed(v: &Value, out: &mut Vec<u8>) -> bool {
        match v {
            Value::Null => out.push(b'n'),
            Value::Bool(b) => out.push(if *b { b't' } else { b'f' }),
            Value::Int(i) => {
                out.push(b'i');
                out.extend_from_slice(&i.to_le_bytes());
            }
            Value::Float(f) => {
                out.push(b'd');
                out.extend_from_slice(&f.to_bits().to_le_bytes());
            }
            Value::String(s) | Value::Symbol(s) => {
                out.push(b's');
                out.extend_from_slice(&(s.len() as u64).to_le_bytes());
                out.extend_from_slice(s.as_bytes());
            }
            Value::Array(items) => {
                let items = items.borrow();
                out.push(b'a');
                out.extend_from_slice(&(items.len() as u64).to_le_bytes());
                for item in items.iter() {
                    if !feed(item, out) {
                        return false;
                    }
                }
            }
            Value::Hash(pairs) => {
                let pairs = pairs.borrow();
                out.push(b'h');
                out.extend_from_slice(&(pairs.len() as u64).to_le_bytes());
                for (k, val) in pairs.iter() {
                    let HashKey::String(name) = k else {
                        return false;
                    };
                    out.extend_from_slice(&(name.len() as u64).to_le_bytes());
                    out.extend_from_slice(name.as_bytes());
                    if !feed(val, out) {
                        return false;
                    }
                }
            }
            _ => return false,
        };
        true
    }

    thread_local! {
        /// One buffer per thread, kept between styles: a style is tens of
        /// bytes and there are thousands of them per render, so the
        /// allocation would otherwise be the next thing to show up.
        static SCRATCH: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
    }

    SCRATCH.with(|scratch| {
        let mut buf = scratch.borrow_mut();
        buf.clear();
        if !feed(v, &mut buf) {
            return None;
        }
        let mut out = [0u8; 16];
        out.copy_from_slice(&blake3::hash(&buf).as_bytes()[..16]);
        Some(out)
    })
}

/// A gradient's `to`, as CSS writes it (eui 02 §5.3): a side is its angle,
/// and a corner is one of the four codes whose angle the box decides.
fn gradient_to(v: &Json) -> Result<u16, String> {
    const SIDES: &[(&str, u16)] = &[
        ("top", 0),
        ("right", 90),
        ("bottom", 180),
        ("left", 270),
        ("top right", Gradient::TO_TOP_RIGHT),
        ("right top", Gradient::TO_TOP_RIGHT),
        ("bottom right", Gradient::TO_BOTTOM_RIGHT),
        ("right bottom", Gradient::TO_BOTTOM_RIGHT),
        ("bottom left", Gradient::TO_BOTTOM_LEFT),
        ("left bottom", Gradient::TO_BOTTOM_LEFT),
        ("top left", Gradient::TO_TOP_LEFT),
        ("left top", Gradient::TO_TOP_LEFT),
    ];
    let s = v
        .as_str()
        .ok_or("EUI: a gradient's `to` is a side or a corner")?;
    SIDES.iter().find(|(n, _)| *n == s).map(|(_, a)| *a).ok_or_else(|| {
        format!("EUI: a gradient goes to top, right, bottom, left or a corner (top right); not '{s}'")
    })
}

/// `animation`, which is a bit set rather than one name (03 §5).
///
/// One name still works, and every existing view keeps meaning what it did:
/// `"spin"`, `"enter"`, `"none"`. A page needs two — it has to say how it
/// arrives *and* how it leaves while it is still there to say it, because the
/// op that removes a node is the only op there is — and two is written as a
/// list: `["enter", "exit"]`.
fn animation_of(v: &Json) -> Result<u8, String> {
    // `pulse` and `bounce` are version 6; `Encoder::style` takes them off a
    // record for a session below it (`StyleRecord::for_protocol`).
    const NAMES: &[(&str, u8)] = &[
        ("none", 0),
        ("spin", 1),
        ("enter", 2),
        ("exit", 4),
        ("pulse", 8),
        ("bounce", 16),
    ];
    match v.as_array() {
        Some(items) => items
            .iter()
            .try_fold(0u8, |acc, i| Ok(acc | enum_of(i, NAMES)?)),
        None => enum_of(v, NAMES),
    }
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

/// What a node's `p` hash amounts to: the props that go on the wire, and
/// the one that does not.
type NodeProps = (Vec<(u32, WireValue)>, Option<(i64, i64)>, bool);

/// `scroll_to: [x, y]`, in pixels, on a node that can be scrolled.
///
/// Only a scroller and a windowed list have an offset to set; the client
/// answers `Op::ScrollTo` on anything else with `NotScrollable`, which ends
/// the session (01 §4). Saying so here costs the view a clear error instead
/// of a window that closes.
///
/// The offsets are absolute, because that is what the op carries. There is
/// deliberately no "end" here: a view that wants the foot of a list knows
/// its own row heights and the height it was given, so it can say where the
/// foot is, and a sentinel would only move that arithmetic somewhere it has
/// less to work with.
fn scroll_offset(kind: NodeKind, v: &Json) -> Result<(i64, i64), String> {
    if !matches!(kind, NodeKind::Scroll | NodeKind::List) {
        return Err(format!(
            "EUI: scroll_to is for a scroll or a list, not a {kind:?}"
        ));
    }
    let pair = v
        .as_array()
        .filter(|a| a.len() == 2)
        .ok_or_else(|| format!("EUI: a scroll_to is [x, y] in pixels, got {v}"))?;
    let axis = |i: usize| -> Result<i64, String> {
        let n = pair
            .get(i)
            .and_then(Json::as_f64)
            .ok_or_else(|| format!("EUI: a scroll_to is [x, y] in pixels, got {v}"))?;
        // A view that computes an offset from row heights can land on a
        // fraction or, on an empty list, on something nonsensical. Rounding
        // and clamping here keeps `as i32` from being a silent wrap.
        Ok(n.round().clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i64)
    };
    Ok((axis(0)?, axis(1)?))
}

/// The protocol version an event kind arrived in. Everything older than
/// the first bump is `1`; this exists so `handlers` can skip what an older
/// client would choke on rather than guess from a list of names.
fn since(kind: EventKind) -> u32 {
    match kind {
        EventKind::Level => 3,
        _ => 1,
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
        "level" => EventKind::Level,
        "wake" => EventKind::Wake,
        "file_pick" => EventKind::FilePick,
        "file_save" => EventKind::FileSave,
        "file_drag" => EventKind::FileDrag,
        "back" => EventKind::Back,
        "location" => EventKind::Location,
        "nfc_tag" => EventKind::NfcTag,
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
        EventKind::Level => "level",
        EventKind::Wake => "wake",
        // EUI 03 §3.2. The client sends these for a node carrying `pick` or
        // `save`, and `event_kind` above deliberately does not take their
        // names yet: a view cannot declare the handler, so nothing can open
        // a dialog and neither of these can arrive. They are named here
        // because the wire has them, not because this server answers them.
        EventKind::FilePick => "file_pick",
        EventKind::FileSave => "file_save",
        EventKind::FileDrag => "file_drag",
        EventKind::Back => "back",
        EventKind::Location => "location",
        EventKind::NfcTag => "nfc_tag",
    }
}

/// `spec/05-theme.md` §1, by name.
const ROLES: [&str; 33] = [
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
    "series.1",
    "series.2",
    "series.3",
    "series.4",
    "series.5",
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
                        scroll_to: None,
                        focus_to: false,
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

    /// The premise the one-shot render's ETag and the adopt offer both rest
    /// on: the same tree, encoded twice by two fresh encoders, is the same
    /// bytes. Ids are allocated `len() + 1` in traversal order and every
    /// definition is pushed into `pending` at allocation, so nothing here
    /// depends on the iteration order of a `HashMap` — but that is a
    /// property of this code rather than a law, and it is load-bearing
    /// enough to be checked rather than reasoned about.
    #[test]
    fn two_fresh_encoders_render_the_same_tree_to_the_same_bytes() {
        let tree = json!({"k": "box", "s": {"display": "column", "gap": 8}, "c": [
            {"k": "text", "t": "one", "s": {"fg": "text.default"}},
            {"k": "text", "t": "two", "s": {"fg": "text.default"}},
            {"k": "box", "s": {"display": "row", "gap": 4}, "c": [
                {"k": "text", "t": "three"},
                {"k": "box", "on": {"click": "go"}, "c": [{"k": "text", "t": "go"}]},
            ]},
        ]});

        let encode = || {
            let mut enc = Encoder::default();
            let batches = enc.render(&tree, false).unwrap();
            batches
                .iter()
                .flat_map(|b| eui_proto::Frame::Batch(b.clone()).encode())
                .collect::<Vec<u8>>()
        };
        assert_eq!(encode(), encode());
    }

    /// An ephemeral encoder leaves nothing on the worker that drew it.
    ///
    /// The memo is a thread-local holding interpreter values, so anything it
    /// keeps for a render that is already over can only be freed by asking
    /// that same worker. A one-shot render never asks, because it never puts
    /// anything there.
    #[test]
    fn an_ephemeral_encoder_keeps_nothing_in_the_memo() {
        let tree = json!({"k": "box", "c": [
            {"k": "box", "key": "a", "c": [{"k": "text", "t": "one"}]},
            {"k": "box", "key": "b", "c": [{"k": "text", "t": "two"}]},
        ]});

        let value = crate::serve::json_to_value(&tree);

        let warm = "memo-test-warm";
        let mut kept = Encoder::default();
        kept.render_value(warm, &value, false).unwrap();
        assert!(
            memo_entries(warm) > 0,
            "an ordinary session memoizes its keyed subtrees"
        );

        let cold = "memo-test-cold";
        let mut once = Encoder::default();
        once.set_ephemeral(true);
        once.render_value(cold, &value, false).unwrap();
        assert_eq!(
            memo_entries(cold),
            0,
            "a one-shot render leaves the worker nothing to free"
        );

        forget_session(warm);
        forget_session(cold);
    }

    /// 03 §1.2: a `scene` names two assets, and its uniforms are the
    /// author's eight floats.
    ///
    /// Both hashes travel the way a picture's `src` does -- fetched from the
    /// origin, checked against their own name before anything decodes them
    /// -- and the client validates each in its worker afterwards. Nothing
    /// here inspects a module or a mesh, and nothing here should.
    #[test]
    fn a_scene_names_two_assets_and_carries_eight_uniforms() {
        let hash =
            super::super::assets::put(b"EUIS\x01@fragment fn fs_main() {}".to_vec()).unwrap();
        let hex: String = hash.iter().map(|b| format!("{b:02x}")).collect();

        let mut enc = Encoder::default();
        let tree = json!({"k": "box", "c": [
            {"k": "scene", "p": {
                "shader": {"asset": hex.clone()},
                "uniforms": [0.5, 2],
                "playing": true,
            }}
        ]});
        enc.render(&tree, false).unwrap();

        // The atom ids first: looking one up interns it, which wants the
        // encoder mutably, and the node below borrows it.
        let (shader_atom, uniforms_atom) = (enc.atom("shader"), enc.atom("uniforms"));
        let node = &enc.prev.as_ref().unwrap().children[0];
        assert_eq!(node.node().kind, NodeKind::Scene);
        let prop = |atom: u32| {
            node.node()
                .props
                .iter()
                .find(|(a, _)| *a == atom)
                .map(|(_, v)| v.clone())
        };
        assert!(matches!(prop(shader_atom), Some(WireValue::Asset(h)) if h == hash));
        // Short lists are padded, not refused: an author who wants one
        // number should not have to write seven zeroes after it.
        match prop(uniforms_atom) {
            Some(WireValue::List(vs)) => {
                assert_eq!(vs.len(), 8, "the block is eight floats wide");
                assert!(matches!(vs[0], WireValue::Float(f) if (f - 0.5).abs() < 1e-9));
                assert!(
                    matches!(vs[1], WireValue::Float(f) if (f - 2.0).abs() < 1e-9),
                    "an integer is a number too"
                );
                assert!(matches!(vs[7], WireValue::Float(f) if f == 0.0));
            }
            other => panic!("uniforms should be a list, got {other:?}"),
        }
    }

    /// What the uniform block refuses, so a bad view is a server-side error
    /// with a line number rather than a window drawing nothing.
    #[test]
    fn a_scenes_uniforms_must_be_eight_finite_numbers() {
        let mut enc = Encoder::default();
        let bad = |u: serde_json::Value| {
            let mut enc2 = Encoder::default();
            enc2.render(&json!({"k": "scene", "p": {"uniforms": u}}), false)
                .unwrap_err()
        };
        assert!(bad(json!([1, 2, 3, 4, 5, 6, 7, 8, 9])).contains("eight uniforms"));
        assert!(bad(json!("nope")).contains("list of numbers"));
        assert!(bad(json!(["a"])).contains("must be numbers"));
        // And the ordinary case still works, so the refusals above are not
        // refusing everything.
        enc.render(
            &json!({"k": "scene", "p": {"uniforms": [1, 2, 3, 4, 5, 6, 7, 8]}}),
            false,
        )
        .unwrap();
    }

    #[test]
    fn a_src_names_bytes_in_the_store_as_well_as_a_file() {
        // Bytes that were never a file — an attachment out of SoliDB or S3 —
        // reach a window by the name the store gave them. Anything else in a
        // `src` has to say so plainly, because the alternative is a window
        // drawing a hole with nothing logged.
        let hash = super::super::assets::put(b"not a file, just bytes".to_vec()).unwrap();
        let hex: String = hash.iter().map(|b| format!("{b:02x}")).collect();

        let mut enc = Encoder::default();
        let tree = json!({"k": "box", "c": [
            {"k": "image", "key": "kept", "p": {"src": {"asset": hex.clone()}}}
        ]});
        enc.render(&tree, false).unwrap();

        let src = enc.atom("src");
        let node = &enc.prev.as_ref().unwrap().children[0];
        let drawn = node
            .node()
            .props
            .iter()
            .find(|(a, _)| *a == src)
            .unwrap()
            .1
            .clone();
        assert_eq!(
            wire_to_json(&enc, &drawn),
            json!(hex),
            "the src is the asset it was given, not a path lookup"
        );

        let bad = |p: Json| {
            Encoder::default()
                .render(
                    &json!({"k": "box", "c": [{"k": "image", "key": "x", "p": {"src": p}}]}),
                    false,
                )
                .unwrap_err()
        };
        assert!(
            bad(json!({"asset": &hex[..63]})).contains("64 hex"),
            "a short hash is refused, not padded"
        );
        assert!(bad(json!({"asset": "z".repeat(64)})).contains("64 hex"));
        assert!(bad(json!({"path": &hex})).contains("asset"));
        assert!(
            bad(json!(7)).contains("must be a path"),
            "everything else still answers as before"
        );
    }

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
        let seen = &mut StyleMemo::default();
        let a = enc.style_of_value(&style(), seen).unwrap();
        assert_eq!(enc.style_fingerprints.len(), 1);
        let b = enc.style_of_value(&style(), seen).unwrap();
        assert_eq!(a, b, "the same fields are the same style");
        assert_eq!(enc.styles.len(), 1);
        let c = enc
            .style_of_value(&h(vec![("display", s("column"))]), seen)
            .unwrap();
        assert_ne!(a, c);
        assert_eq!(enc.style_fingerprints.len(), 2);
    }

    /// The point of the identity memo: a style a view hoists out of its loop
    /// is one allocation, so it is fingerprinted once however many nodes wear
    /// it. The fingerprint is a walk of the hash and a BLAKE3 over its
    /// fields; this is a pointer.
    #[test]
    fn a_style_shared_by_many_nodes_is_fingerprinted_once() {
        let shared = h(vec![("display", s("row")), ("gap", Value::Int(4))]);
        let mut enc = Encoder::default();
        let seen = &mut StyleMemo::default();

        let first = enc.style_of_value(&shared, seen).unwrap();
        for _ in 0..50 {
            assert_eq!(enc.style_of_value(&shared, seen).unwrap(), first);
        }
        assert_eq!(seen.len(), 1);
        assert_eq!(enc.style_fingerprints.len(), 1, "one walk, not fifty-one");
        assert_eq!(enc.styles.len(), 1);
    }

    /// And the reason it is cleared every pass: a hash is mutable. Inside one
    /// conversion nothing can move — the view has already returned the whole
    /// tree — but a handler between two renders may write to the very hash a
    /// module constant holds, and a pointer that still answers the old id
    /// would paint last render's style for ever.
    #[test]
    fn a_style_mutated_between_renders_is_not_the_one_the_pointer_remembers() {
        let shared = h(vec![("display", s("row"))]);
        let view = |style: &Value| {
            h(vec![
                ("k", s("box")),
                ("s", style.clone()),
                ("c", list(vec![h(vec![("k", s("text")), ("t", s("hi"))])])),
            ])
        };

        let mut enc = Encoder::default();
        enc.render_value("session", &view(&shared), false).unwrap();
        let before = enc
            .style_of_value(&shared, &mut StyleMemo::default())
            .unwrap();

        // The same allocation, different contents.
        let Value::Hash(pairs) = &shared else {
            unreachable!("a hash")
        };
        pairs
            .borrow_mut()
            .insert(HashKey::String("gap".into()), Value::Int(9));

        enc.render_value("session", &view(&shared), false).unwrap();
        let after = enc
            .style_of_value(&shared, &mut StyleMemo::default())
            .unwrap();
        assert_ne!(before, after, "the pointer outlived its contents");
    }

    /// The trap a mail reader's cursor was blamed on (eui memory, September
    /// 2026): a row whose only change is its style was said to receive no
    /// `SetStyle`. It does, keyed or not, through the path a Soli view
    /// takes — two ops, one per row that changed, and nothing else. What
    /// was actually seen was the snapshot tool photographing the first
    /// frame of the row's `transition`; the pin is here so the claim cannot
    /// come back as folklore.
    #[test]
    fn a_style_only_change_is_one_set_style_per_row_keyed_or_not() {
        for keyed in [false, true] {
            let row = |i: i64, sel: i64| {
                let bg = if i == sel {
                    "danger.base"
                } else {
                    "surface.raised"
                };
                let mut pairs = vec![
                    ("k", s("box")),
                    (
                        "s",
                        h(vec![
                            ("height", Value::Int(40)),
                            ("bg", s(bg)),
                            ("transition", s("fast")),
                        ]),
                    ),
                    (
                        "c",
                        list(vec![h(vec![
                            ("k", s("text")),
                            ("t", s(&format!("Row {i}"))),
                        ])]),
                    ),
                ];
                if keyed {
                    pairs.push(("key", s(&format!("r{i}"))));
                }
                h(pairs)
            };
            let view = |sel: i64| {
                h(vec![
                    ("k", s("box")),
                    ("c", list((0..3).map(|i| row(i, sel)).collect())),
                ])
            };
            let session = if keyed {
                "style-only-keyed"
            } else {
                "style-only"
            };
            let mut enc = Encoder::default();
            enc.render_value(session, &view(0), false).unwrap();
            let rows: Vec<u32> = enc
                .prev
                .as_ref()
                .unwrap()
                .children
                .iter()
                .map(|c| c.id)
                .collect();
            let lit = enc.prev.as_ref().unwrap().children[0].style;
            let ops: Vec<Op> = enc
                .render_value(session, &view(1), false)
                .unwrap()
                .into_iter()
                .flat_map(|b| b.ops)
                .collect();
            let restyled: Vec<(u32, u32)> = ops
                .iter()
                .filter_map(|o| match o {
                    Op::SetStyle { node, style } => Some((*node, *style)),
                    _ => None,
                })
                .collect();
            assert_eq!(
                restyled.len(),
                2,
                "keyed={keyed}: one per row that changed, got {ops:?}"
            );
            assert_eq!(ops.len(), 2, "keyed={keyed}: and nothing else, got {ops:?}");
            assert!(
                restyled.contains(&(rows[1], lit)),
                "keyed={keyed}: the lit record moved to row 1"
            );
            assert!(
                restyled.iter().any(|(n, st)| *n == rows[0] && *st != lit),
                "keyed={keyed}: and left row 0"
            );
        }
    }

    #[test]
    fn a_local_handler_is_compiled_once_per_source_however_it_is_keyed() {
        let mut enc = Encoder::default();
        let tree = || {
            json!({"k": "box", "c": [
                {"k": "box", "key": "a", "on": {"click": {"local": "state.n += 1"}}},
                {"k": "box", "key": "b", "on": {"click": {"local": "state.n += 1"}}}
            ]})
        };
        enc.render(&tree(), false).unwrap();
        // One compile, not one per key. This is the whole reason a list can
        // carry a hover: the table holds 4 095 chunks, and a source compiled
        // per key would spend one on every row.
        assert_eq!(enc.local_cache.len(), 1, "keyed by source and styles alone");
        assert_eq!(enc.chunks.len(), 1, "the bytes are the same chunk");
        enc.render(&tree(), false).unwrap();
        assert_eq!(enc.local_cache.len(), 1);
    }

    #[test]
    fn a_hover_on_four_thousand_rows_is_one_chunk() {
        let mut enc = Encoder::default();
        let rows: Vec<Json> = (0..4000)
            .map(|i| {
                json!({
                    "k": "box",
                    "key": format!("row-{i}"),
                    "on": {"pointer_enter": {"local": "self.style = @lit", "styles": {"lit": {"bg": "surface.raised"}}}}
                })
            })
            .collect();
        enc.render(&json!({"k": "box", "c": rows}), false).unwrap();
        assert_eq!(enc.chunks.len(), 1, "one chunk for the whole list");
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
    fn a_render_that_changed_nothing_sends_nothing() {
        let mut enc = Encoder::default();
        let tree = json!({"k": "box", "c": [{"k": "text", "t": "steady"}]});
        let first = enc.render(&tree, false).unwrap();
        assert!(!first.is_empty(), "the first render mounts the tree");

        // The same tree again. A page that carries a `wake` renders on every
        // tick whether or not anything moved; a batch for one of those is
        // bytes on the wire and, on the client, a full restore of every
        // locally previewed style.
        let again = enc.render(&tree, false).unwrap();
        assert!(again.is_empty(), "{again:?}");

        // A change still goes out.
        let moved = json!({"k": "box", "c": [{"k": "text", "t": "moved"}]});
        assert!(!enc.render(&moved, false).unwrap().is_empty());
    }

    /// EUI 02 §5.2: what `eui_notify` queued rides out with the render
    /// that follows it, after the tree, and four to a batch.
    #[test]
    fn a_queued_notification_leaves_with_the_next_render() {
        super::super::notify::clear();
        let mut enc = Encoder::default();
        let tree = json!({"k": "box", "c": [{"k": "text", "t": "steady"}]});
        enc.render(&tree, false).unwrap();

        super::super::notify::queue("Nouveau message", "Ana : on déjeune ?", "thread-7");
        // The same tree: a render that changed nothing still carries what
        // the handler asked to say.
        let batches = enc.render(&tree, false).unwrap();
        let ops: Vec<&Op> = batches.iter().flat_map(|b| b.ops.iter()).collect();
        assert_eq!(ops.len(), 1, "{ops:?}");
        assert!(
            matches!(ops[0], Op::Notify { title, body, tag }
                if title == "Nouveau message" && body == "Ana : on déjeune ?" && tag == "thread-7"),
            "{ops:?}"
        );
        // Taken, not repeated: the next render says nothing.
        assert!(enc.render(&tree, false).unwrap().is_empty());
    }

    /// A client refuses a batch carrying a fifth notification, so five
    /// leave in two batches rather than ending the session.
    #[test]
    fn five_notifications_go_out_in_two_batches() {
        super::super::notify::clear();
        let mut enc = Encoder::default();
        let tree = json!({"k": "box"});
        enc.render(&tree, false).unwrap();
        for i in 0..5 {
            super::super::notify::queue(&format!("ping {i}"), "", "");
        }
        let batches = enc.render(&tree, false).unwrap();
        assert_eq!(batches.len(), 2, "{batches:?}");
        assert_eq!(batches[0].ops.len(), 4);
        assert_eq!(batches[1].ops.len(), 1);
        assert!(
            batches[1].seq > batches[0].seq,
            "a second batch is a second sequence number"
        );
    }

    #[test]
    fn scroll_to_is_not_a_prop_and_is_refused_on_what_cannot_scroll() {
        let mut enc = Encoder::default();
        // On a list it is an instruction, and the client is never handed a
        // prop by that name.
        let batches = enc
            .render(
                &json!({"k": "list", "p": {"scroll_to": [0, 640], "count": 3}}),
                false,
            )
            .unwrap();
        let ops: Vec<&Op> = batches.iter().flat_map(|b| b.ops.iter()).collect();
        assert!(
            ops.iter()
                .any(|op| matches!(op, Op::ScrollTo { x: 0, y: 640, .. })),
            "{ops:?}"
        );
        assert!(
            !ops.iter().any(|op| matches!(
                op,
                Op::DefAtom { value, .. } if value == "scroll_to"
            )),
            "scroll_to reached the wire as a prop: {ops:?}"
        );

        // On a box it would end the session with NotScrollable, so it is
        // refused here, where there is somewhere to say why.
        let mut enc = Encoder::default();
        let err = enc
            .render(&json!({"k": "box", "p": {"scroll_to": [0, 10]}}), false)
            .unwrap_err();
        assert!(err.contains("scroll_to"), "{err}");
    }

    #[test]
    fn the_two_file_events_are_names_a_view_may_use() {
        // Without these the client opens no dialog at all: it wants the
        // `pick` prop *and* a server handler for the matching event.
        assert!(event_kind("file_pick").is_some());
        assert!(event_kind("file_save").is_some());
    }

    /// The atom names of every server handler in a batch's `Mount`.
    fn handler_names(enc: &Encoder, ops: &[Op]) -> Vec<String> {
        let mut out = Vec::new();
        for op in ops {
            if let Op::Mount(t) = op {
                for (_, h) in &t.handlers {
                    if let Handler::Server(atom) = h {
                        out.push(
                            enc.atoms_by_id
                                .get(*atom as usize)
                                .cloned()
                                .unwrap_or_default(),
                        );
                    }
                }
            }
        }
        out
    }

    #[test]
    fn a_handler_an_older_client_cannot_decode_is_left_out() {
        // `level` is version 3 (03 §7). A client that settled on 2 has
        // never heard of event kind 0x20, and a batch naming it would be
        // a decode error that ends the session -- an expensive way to
        // find out a meter would not have moved.
        // A box rather than an `audio`: the gate is on the handler, not
        // on the kind, and a sound would want an asset that exists.
        let view = json!({"k": "box", "on": {"level": "vu", "click": "done"}});

        let mut old = Encoder::default();
        old.set_protocol(2);
        let ops: Vec<Op> = old
            .render(&view, false)
            .unwrap()
            .into_iter()
            .flat_map(|b| b.ops)
            .collect();
        let names = handler_names(&old, &ops);
        assert!(
            !names.contains(&"vu".to_string()),
            "no level for a version-2 client: {names:?}"
        );
        assert!(
            names.contains(&"done".to_string()),
            "everything else still goes: {names:?}"
        );

        // And a client that speaks 3 gets it.
        let mut new = Encoder::default();
        new.set_protocol(3);
        let ops: Vec<Op> = new
            .render(&view, false)
            .unwrap()
            .into_iter()
            .flat_map(|b| b.ops)
            .collect();
        let names = handler_names(&new, &ops);
        assert!(names.contains(&"vu".to_string()), "{names:?}");
    }

    /// Every `DefStyle` record in a render, in the order defined.
    fn defined_styles(enc: &mut Encoder, view: &serde_json::Value) -> Vec<StyleRecord> {
        enc.render(view, false)
            .unwrap()
            .into_iter()
            .flat_map(|b| b.ops)
            .filter_map(|op| match op {
                Op::DefStyle { record, .. } => Some(record),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_space_step_an_older_client_lacks_falls_back() {
        // 05 §2: `space` 13-17 (Tailwind's 1.5 2.5 3.5 20 32) are version
        // 6. A session at 5 gets the nearest older step, ties down; one at
        // 6 gets the step the view wrote.
        let view = json!({"k": "box", "s": {"gap": 13, "pad": [14, 15, 16, 17], "margin": 4, "width": "sp:13"}});

        let mut old = Encoder::default();
        old.set_protocol(5);
        let got = defined_styles(&mut old, &view);
        assert_eq!(got.len(), 1, "{got:?}");
        assert_eq!(got[0].gap, 2, "6 px is 4 px below version 6");
        assert_eq!(got[0].padding, [3, 4, 11, 12]);
        assert_eq!(got[0].margin, [4; 4], "an older step is left alone");
        assert_eq!(got[0].width, eui_proto::Dim::Space(2));

        let mut new = Encoder::default();
        new.set_protocol(6);
        let got = defined_styles(&mut new, &view);
        assert_eq!(got[0].gap, 13);
        assert_eq!(got[0].padding, [14, 15, 16, 17]);
        assert_eq!(got[0].width, eui_proto::Dim::Space(13));

        // And a render with no handshake behind it is at ours.
        let got = defined_styles(&mut Encoder::default(), &view);
        assert_eq!(got[0].gap, 13);
    }

    fn all_ops(enc: &mut Encoder, view: &serde_json::Value) -> Vec<Op> {
        enc.render(view, false)
            .unwrap()
            .into_iter()
            .flat_map(|b| b.ops)
            .collect()
    }

    #[test]
    fn a_gradient_background_is_defined_once_per_session() {
        // eui 02 §5.3: a `bg` hash naming a gradient becomes one
        // `DefGradient`, before the `DefStyle` that names it, and the same
        // gradient written again -- here by a second node, and by a second
        // render -- is the same id and no second definition.
        let card = json!({"bg": {"gradient": {"to": "right", "stops": ["accent.base", ["#ff80b5", 255]]}}, "pad": 5});
        let view = json!({"k": "box", "c": [{"k": "box", "s": card}, {"k": "box", "s": {"bg": {"gradient": {"angle": 90, "stops": [["accent.base", 0], ["#ff80b5", 255]]}}}}]});
        let mut enc = Encoder::default();
        let ops = all_ops(&mut enc, &view);
        let defs: Vec<(usize, u32, Gradient)> = ops
            .iter()
            .enumerate()
            .filter_map(|(i, op)| match op {
                Op::DefGradient { id, gradient } => Some((i, *id, *gradient)),
                _ => None,
            })
            .collect();
        assert_eq!(
            defs.len(),
            1,
            "`to right` and 90 degrees are one gradient: {ops:?}"
        );
        let (at, id, g) = defs[0];
        assert_eq!((id, g.angle, g.count), (1, 90, 2));
        assert_eq!(
            g.stops()[0].color,
            ColorRef::role(role_id("accent.base").unwrap())
        );
        assert!(g.stops()[1].color.is_literal() && g.stops()[1].at == 255);
        let colour = ops
            .iter()
            .position(|op| {
                matches!(
                    op,
                    Op::DefColor {
                        rgba: 0xFF80B5FF,
                        ..
                    }
                )
            })
            .unwrap();
        let style = ops
            .iter()
            .position(|op| matches!(op, Op::DefStyle { record, .. } if record.bg == ColorRef::gradient(1)))
            .unwrap();
        assert!(
            colour < at && at < style,
            "the colour, then the gradient, then the style"
        );
        let again = all_ops(&mut enc, &json!({"k": "box", "s": card}));
        assert!(
            !again.iter().any(|op| matches!(op, Op::DefGradient { .. })),
            "defined once"
        );

        // Positions spread evenly when none is given; a corner is its code.
        let mut enc = Encoder::default();
        let ops = all_ops(
            &mut enc,
            &json!({"k": "box", "s": {"bg": {"gradient": {"to": "top right", "stops": ["accent.base", "info.base", "danger.base"]}}}}),
        );
        let g = ops
            .iter()
            .find_map(|op| match op {
                Op::DefGradient { gradient, .. } => Some(*gradient),
                _ => None,
            })
            .unwrap();
        assert_eq!(g.angle, Gradient::TO_TOP_RIGHT);
        assert_eq!(
            g.stops().iter().map(|s| s.at).collect::<Vec<_>>(),
            [0, 128, 255]
        );
        // And the shapes it refuses, each saying why.
        for (bad, why) in [
            (
                json!({"gradient": {"stops": ["accent.base"]}}),
                "two or three stops",
            ),
            (
                json!({"gradient": {"to": "up", "stops": ["accent.base", "info.base"]}}),
                "goes to top",
            ),
            (
                json!({"gradient": {"angle": 360, "stops": ["accent.base", "info.base"]}}),
                "0 to 359",
            ),
            (
                json!({"gradient": {"stops": [["accent.base", 200], ["info.base", 100]]}}),
                "forwards",
            ),
            (
                json!({"gradient": {"stops": ["none", "info.base"]}}),
                "not none",
            ),
            (
                json!({"gradient": {"stops": ["accent.base", "info.base"], "via": "x"}}),
                "not `via`",
            ),
            (json!({"linear": 1}), "a bg hash"),
        ] {
            let err = Encoder::default()
                .render(&json!({"k": "box", "s": {"bg": bad}}), false)
                .unwrap_err();
            assert!(err.contains(why), "{why}: {err}");
        }
    }

    #[test]
    fn an_older_session_is_sent_a_gradients_first_stop_and_no_pulse_or_bounce() {
        // eui 02 §5.3 and 03 §5: `DefGradient`, the gradient range and the
        // `animation` bits 8 and 16 are version 6. A session at 5 gets the
        // first stop as a solid `bg` and the bits it knows.
        let view = json!({"k": "box", "s": {"bg": {"gradient": {"to": "bottom", "stops": ["#123456", "info.base"]}}, "animation": ["spin", "pulse", "bounce"]}});
        let mut old = Encoder::default();
        old.set_protocol(5);
        let ops = all_ops(&mut old, &view);
        assert!(
            !ops.iter().any(|op| matches!(op, Op::DefGradient { .. })),
            "{ops:?}"
        );
        let got = defined_styles(
            &mut Encoder {
                version: 5,
                ..Encoder::default()
            },
            &view,
        );
        assert!(
            got[0].bg.is_literal(),
            "the first stop, #123456: {:?}",
            got[0].bg
        );
        assert_eq!(
            got[0].animation, 1,
            "spin, and nothing a version-5 client refuses"
        );

        let mut new = Encoder::default();
        new.set_protocol(6);
        let got = defined_styles(&mut new, &view);
        assert_eq!(got[0].bg, ColorRef::gradient(1));
        assert_eq!(got[0].animation, 1 | 8 | 16);
        assert_eq!(animation_of(&json!("bounce")).unwrap(), 16);
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

#[cfg(test)]
mod fingerprint_cost {
    use super::*;

    /// Not an assertion — a measurement, printed with `--nocapture`. The
    /// question it answers: of the 737 ns a style lookup costs per node, how
    /// much is walking the hash and how much is BLAKE3 over what the walk
    /// produced?
    #[test]
    #[ignore]
    fn what_a_style_fingerprint_costs() {
        let mut map = crate::interpreter::value::HashPairs::default();
        map.insert(HashKey::String("width".into()), Value::Int(90));
        map.insert(HashKey::String("size".into()), Value::Int(1));
        map.insert(
            HashKey::String("fg".into()),
            Value::String("text.default".into()),
        );
        let style = Value::Hash(Rc::new(RefCell::new(map)));

        let rounds = 200_000;
        let t = std::time::Instant::now();
        let mut sink = 0u8;
        for _ in 0..rounds {
            sink ^= fingerprint_of(&style).unwrap()[0];
        }
        let full = t.elapsed().as_secs_f64() / (rounds as f64) * 1e9;

        // The same walk, feeding a counter instead of a hash.
        let t = std::time::Instant::now();
        let mut bytes = 0usize;
        for _ in 0..rounds {
            bytes += walked_bytes(&style);
        }
        let walk = t.elapsed().as_secs_f64() / (rounds as f64) * 1e9;

        // And the hash alone, over that many bytes.
        let payload = vec![0u8; bytes / rounds];
        let t = std::time::Instant::now();
        for _ in 0..rounds {
            sink ^= blake3::hash(&payload).as_bytes()[0];
        }
        let hash = t.elapsed().as_secs_f64() / (rounds as f64) * 1e9;

        println!(
            "fingerprint {full:.0} ns = walk {walk:.0} ns + blake3 over {} bytes {hash:.0} ns (sink {sink})",
            bytes / rounds
        );
    }

    fn walked_bytes(v: &Value) -> usize {
        match v {
            Value::Int(_) => 9,
            Value::String(s) | Value::Symbol(s) => 9 + s.len(),
            Value::Hash(pairs) => {
                let pairs = pairs.borrow();
                9 + pairs
                    .iter()
                    .map(|(k, val)| {
                        let HashKey::String(name) = k else { return 0 };
                        8 + name.len() + walked_bytes(val)
                    })
                    .sum::<usize>()
            }
            _ => 1,
        }
    }
}
