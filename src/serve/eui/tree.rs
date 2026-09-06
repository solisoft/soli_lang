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

use std::collections::HashMap;

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
    pub children: Vec<TNode>,
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
}

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
    pub fn render(&mut self, json: &Json, resync: bool) -> Result<Batch, String> {
        let mut tree = self.convert(json)?;
        let ops = match (&self.prev, resync) {
            (Some(prev), false) => {
                let mut ops = Vec::new();
                let prev = prev.clone();
                super::diff::diff(self, &prev, &mut tree, &mut ops);
                ops
            }
            _ => {
                assign_fresh_ids(self, &mut tree);
                vec![Op::Mount(flatten(&tree))]
            }
        };
        self.prev = Some(tree);
        self.seq += 1;
        let mut all = std::mem::take(&mut self.pending);
        all.extend(ops);
        Ok(Batch { seq: self.seq, ops: all })
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

    fn convert(&mut self, j: &Json) -> Result<TNode, String> {
        let obj = j.as_object().ok_or("EUI: a node must be a hash")?;
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
                props.push((atom, self.wire_value(v)?));
            }
        }
        let mut handlers = Vec::new();
        if let Some(on) = obj.get("on").and_then(Json::as_object) {
            for (event, target) in on {
                let kind = event_kind(event).ok_or_else(|| format!("EUI: unknown event '{event}'"))?;
                let handler = match target {
                    Json::String(name) => Handler::Server(self.atom(name)),
                    Json::Object(spec) => {
                        // {"local": [instructions], "then": "server_event"?}
                        let program = spec.get("local").and_then(Json::as_array).ok_or("EUI: a local handler needs a \"local\" list")?;
                        let bytes = self.assemble(program)?;
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
        let mut children = Vec::new();
        if let Some(c) = obj.get("c").and_then(Json::as_array) {
            for child in c {
                if child.is_null() || child == &Json::Bool(false) {
                    continue; // `x if cond` in a list reads as null: skip, like a template would.
                }
                children.push(self.convert(child)?);
            }
        }
        if kind.is_leaf() && !children.is_empty() {
            return Err(format!("EUI: a {kind:?} node cannot have children"));
        }
        Ok(TNode { id: 0, kind, style, key, key_atom, text, props, handlers, children })
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

/// Give every node in a subtree a fresh id.
pub fn assign_fresh_ids(enc: &mut Encoder, node: &mut TNode) {
    node.id = enc.fresh_id();
    for c in &mut node.children {
        assign_fresh_ids(enc, c);
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
            walk(c, out);
        }
    }
    walk(root, &mut out);
    out
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
    node.children.iter().find_map(|c| find(c, id))
}
